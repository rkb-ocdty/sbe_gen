use super::*;
use crate::GeneratedModule;

/// An extra derivation step over the laid-out schema.
///
/// Every field already has its type and its place by the time one of these runs, so a step only
/// has to say what it wants written. It is a trait object on purpose: the set of steps is a
/// run-time list, so a new one (serde, postgres, whatever) plugs in without touching a pass.
///
/// The staged tree derives `Drive`, so a step that wants to find its own nodes can walk it with
/// a visitor rather than re-matching the shape by hand.
pub trait Derivation {
    /// A block's padding member: bytes it needs to reach the next field or the end of the
    /// block, which the schema never declared and nothing may set.
    fn padding(&self, _field: &mut SynField) {}

    /// items appended to `types.rs`
    fn types(&self, _schema: &LaidOutSchema) -> Result<TokenStream, CodegenError> {
        Ok(TokenStream::new())
    }

    /// A field's member in the packed struct, before it is written. Mutable, so a step can put
    /// its own `#[serde(...)]` on it or swap the type the schema resolved to — the `PlacedField`
    /// carries what it was, down to the primitive.
    ///
    /// *note: a substituted type has to keep the same size or the packed layout stops matching
    /// the wire*
    fn field(&self, _placed: &PlacedField, _field: &mut SynField) -> Result<(), CodegenError> {
        Ok(())
    }

    /// Everything rendered for one schema type — the composite's struct, an enum and its impls,
    /// a set and its constants. Mutable, so a step can put its own attributes on the declaration
    /// a field's type resolves to, the same way it reaches the message struct.
    fn type_items(&self, _def: &TypeDef, _items: &mut [Item]) -> Result<(), CodegenError> {
        Ok(())
    }

    /// The message's packed struct, before it is written out. It arrives as an `ItemStruct` so a
    /// step can push a `#[derive(..)]` or a container attribute onto it, or hand it to something
    /// that wants a `DeriveInput`.
    fn message_struct(
        &self,
        _msg: &PlacedMessage,
        _item: &mut ItemStruct,
    ) -> Result<(), CodegenError> {
        Ok(())
    }

    /// the same, for a group's entry struct
    fn group_struct(
        &self,
        _group: &PlacedGroup,
        _item: &mut ItemStruct,
    ) -> Result<(), CodegenError> {
        Ok(())
    }

    /// items appended to a message's own module
    fn message(&self, _msg: &PlacedMessage) -> Result<TokenStream, CodegenError> {
        Ok(TokenStream::new())
    }

    /// whole modules the step contributes on its own
    fn modules(&self, _schema: &LaidOutSchema) -> Result<Vec<GeneratedModule>, CodegenError> {
        Ok(Vec::new())
    }
}

impl Emit<'_> {
    pub(crate) fn derive_padding(&self, field: &mut SynField) {
        self.derivations.iter().for_each(|d| d.padding(field));
    }

    pub(crate) fn derived_types(&self) -> Result<TokenStream, CodegenError> {
        self.derivations
            .iter()
            .map(|d| d.types(self.schema))
            .collect()
    }

    pub(crate) fn derive_field(
        &self,
        placed: &PlacedField,
        field: &mut SynField,
    ) -> Result<(), CodegenError> {
        self.derivations
            .iter()
            .try_for_each(|d| d.field(placed, field))
    }

    pub(crate) fn derive_message_struct(
        &self,
        msg: &PlacedMessage,
        item: &mut ItemStruct,
    ) -> Result<(), CodegenError> {
        self.derivations
            .iter()
            .try_for_each(|d| d.message_struct(msg, item))
    }

    pub(crate) fn derive_group_struct(
        &self,
        group: &PlacedGroup,
        item: &mut ItemStruct,
    ) -> Result<(), CodegenError> {
        self.derivations
            .iter()
            .try_for_each(|d| d.group_struct(group, item))
    }

    pub(crate) fn derived_message(&self, msg: &PlacedMessage) -> Result<TokenStream, CodegenError> {
        self.derivations.iter().map(|d| d.message(msg)).collect()
    }

    pub(crate) fn derived_modules(&self) -> Result<Vec<GeneratedModule>, CodegenError> {
        Ok(self
            .derivations
            .iter()
            .map(|d| d.modules(self.schema))
            .collect::<Result<Vec<_>, _>>()?
            .concat())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::ext::DeriveSerialize;

    #[test]
    fn derivation_ext() {
        let xml = r#"
            <messageSchema package="test" id="1" version="0">
              <types>
                <type name="Price" primitiveType="int64"/>
                <composite name="Qty">
                  <type name="mantissa" primitiveType="int64"/>
                  <type name="exponent" primitiveType="int8"/>
                </composite>
                <composite name="groupSize">
                  <type name="blockLength" primitiveType="uint16"/>
                  <type name="numInGroup" primitiveType="uint16"/>
                </composite>
              </types>
              <message name="Quote" id="7">
                <field name="price" id="1" type="Price"/>
                <field name="qty" id="2" type="Qty"/>
                <group name="Legs" id="3" dimensionType="groupSize">
                  <field name="leg" id="1" type="Price"/>
                </group>
                <group name="Events" id="4" dimensionType="groupSize">
                  <field name="stamp" id="1" type="uint32"/>
                </group>
              </message>
            </messageSchema>"#;
        let opts: &'static GeneratorOptions = Box::leak(Box::default());
        let schema = crate::parser::parse_schema(xml).unwrap();
        let laid_out = LaidOutSchema::new(
            LoweredSchema::new(ValidatedSchema::new(DedupedSchema::new(schema), opts).unwrap())
                .unwrap(),
        )
        .unwrap();

        let generated = Emit::new(&laid_out, &[&DeriveSerialize::default()])
            .generated()
            .unwrap();
        let _quote_rs = &generated.messages[0].source;

        let expected: ItemStruct = parse_quote! {
            #[derive(serde::Serialize, serde::Deserialize)]
            pub struct Quote {
                #[serde(rename = "price")]
                #[serde(with = "sbe_support::serde_wire")]
                pub price: super::types::Price,
                #[serde(rename = "qty")]
                pub qty: super::types::Qty,
            }
        };
        let mut actual = generated.structs[0].clone();
        let is_sbe_gen = |a: &syn::Attribute| {
            a.path()
                .segments
                .last()
                .is_some_and(|s| s.ident == "sbe_gen")
        };
        actual.attrs.retain(|a| !is_sbe_gen(a));
        if let syn::Fields::Named(fields) = &mut actual.fields {
            for field in &mut fields.named {
                field.attrs.retain(|a| !is_sbe_gen(a));
            }
        }
        assert_eq!(actual, expected);
    }
}
