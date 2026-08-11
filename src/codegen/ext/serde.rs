use super::*;
use std::collections::HashMap;

/// Serde derives for every block and entry, with the field attributes that make the wire types
/// writable.
///
/// A block's fields are zerocopy wrappers and byte arrays. Neither is `Serialize`, and both are
/// foreign, so each field points at a `serialize_with` in `sbe_support::ser` instead.
pub struct DeriveSerialize {
    /// write the block's fields alongside the groups rather than under a `block` key
    pub flatten: bool,
    /// `serialize_with` paths keyed by the schema's own name for a type, so a price can be
    /// written as a decimal rather than as the integer the wire carries.
    pub overrides: HashMap<String, String>,
    /// the same, keyed by FIX semantic type. A timestamp is a `uInt64` like any other, so it
    /// is only distinguishable by what the schema says it means.
    pub semantic: HashMap<String, String>,
}

impl Default for DeriveSerialize {
    fn default() -> Self {
        Self {
            flatten: true,
            overrides: HashMap::new(),
            semantic: HashMap::new(),
        }
    }
}

impl DeriveSerialize {
    pub fn map(mut self, schema_type: &str, serialize_with: &str) -> Self {
        self.overrides
            .insert(schema_type.to_string(), serialize_with.to_string());
        self
    }

    pub fn map_semantic(mut self, semantic_type: &str, serialize_with: &str) -> Self {
        self.semantic
            .insert(semantic_type.to_string(), serialize_with.to_string());
        self
    }

    /// what writes this field, or `None` where serde reaches it on its own
    fn writer(&self, f: &PlacedField) -> Option<String> {
        if let Some(path) = self.overrides.get(&*f.field.ty) {
            return Some(path.clone());
        }
        if let Some(path) = f
            .field
            .semantic_type
            .as_deref()
            .and_then(|s| self.semantic.get(s))
        {
            return Some(path.clone());
        }
        if f.view.fixed_string.is_some() {
            return Some("sbe_support::serde_ascii".to_string());
        }
        match (f.primitive, f.view.newtype) {
            (Some(_), false) => Some("sbe_support::serde_wire".to_string()),
            _ => None,
        }
    }
}

impl Derivation for DeriveSerialize {
    fn message_struct(
        &self,
        _msg: &PlacedMessage,
        item: &mut ItemStruct,
    ) -> Result<(), CodegenError> {
        item.attrs
            .push(parse_quote!(#[derive(serde::Serialize, serde::Deserialize)]));
        Ok(())
    }

    fn group_struct(
        &self,
        _group: &PlacedGroup,
        item: &mut ItemStruct,
    ) -> Result<(), CodegenError> {
        item.attrs
            .push(parse_quote!(#[derive(serde::Serialize, serde::Deserialize)]));
        Ok(())
    }

    fn padding(&self, field: &mut SynField) {
        // skipped both ways, so deserialising has to put the bytes back
        field
            .attrs
            .push(parse_quote!(#[serde(skip, default = "sbe_support::zeroed")]));
    }

    /// The whole message. Written out because the type is the macro's, and delegating to a
    /// local struct is what makes `flatten` reachable — a hand-rolled `serialize_struct`
    /// cannot splice another struct's fields into its own.
    fn message(&self, msg: &PlacedMessage) -> Result<TokenStream, CodegenError> {
        // no groups is still a message, it is just the block
        let sliceable = msg
            .groups
            .iter()
            .all(|g| g.data.is_empty() && g.groups.is_empty());
        if !sliceable {
            return Ok(TokenStream::new());
        }
        let ty = format_ident!("{}Message", msg.names.msg);
        let params: Vec<Ident> = (0..msg.groups.len())
            .map(|i| format_ident!("G{i}"))
            .collect();
        let names: Vec<Ident> = msg
            .groups
            .iter()
            .map(|g| g.group.name.snake_ident())
            .collect();
        let members = msg.groups.iter().zip(&params).map(|(g, p)| {
            let (name, reported) = (g.group.name.snake_ident(), g.group.name.to_string());
            quote! { #[serde(rename = #reported)] #name: &'a #p, }
        });
        let block = match self.flatten {
            true => quote! { #[serde(flatten)] block: &'a B, },
            false => quote! { block: &'a B, },
        };
        let owned_members = msg.groups.iter().zip(&params).map(|(g, p)| {
            let (name, reported) = (g.group.name.snake_ident(), g.group.name.to_string());
            quote! { #[serde(rename = #reported)] #name: #p, }
        });
        let owned_block = match self.flatten {
            true => quote! { #[serde(flatten)] block: B, },
            false => quote! { block: B, },
        };
        Ok(quote! {
            impl<B: serde::Serialize, #(#params: serde::Serialize),*> serde::Serialize
                for #ty<B, #(#params),*>
            {
                fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                    #[derive(serde::Serialize)]
                    struct Flat<'a, B, #(#params),*> {
                        #block
                        #(#members)*
                    }
                    Flat {
                        block: &self.block,
                        #(#names: &self.#names,)*
                    }
                    .serialize(s)
                }
            }

            impl<'de, B, #(#params),*> serde::Deserialize<'de> for #ty<B, #(#params),*>
            where
                B: serde::Deserialize<'de>,
                #(#params: serde::Deserialize<'de>,)*
            {
                fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                    #[derive(serde::Deserialize)]
                    struct Flat<B, #(#params),*> {
                        #owned_block
                        #(#owned_members)*
                    }
                    let flat = Flat::deserialize(d)?;
                    Ok(Self { block: flat.block, #(#names: flat.#names,)* })
                }
            }
        })
    }

    /// The union writes itself under the template's name, which is what makes a dump of mixed
    /// messages readable. The discriminant enum carries the same name for anything that wants it
    /// on its own.
    fn body_enum(
        &self,
        schema: &LaidOutSchema,
        item: &mut ItemEnum,
    ) -> Result<(), CodegenError> {
        // the name always writes, whatever the bodies can do
        item.attrs.push(parse_quote!(
            #[strum_discriminants(derive(serde::Serialize, serde::Deserialize))]
        ));

        // a message with var-data or a nested group is held as a walked view, and a view has no
        // serde of its own to defer to
        let sliced = schema.messages.iter().all(|msg| {
            msg.data.is_empty()
                && msg
                    .groups
                    .iter()
                    .all(|g| g.data.is_empty() && g.groups.is_empty())
        });
        if !sliced {
            return Ok(());
        }

        // nested under the template name: a stream of mixed messages is unreadable otherwise, and
        // the reader has to know which one it is holding before the fields mean anything
        item.attrs.push(parse_quote!(#[derive(serde::Serialize)]));
        Ok(())
    }

    /// A value enum arrives as a byte and the schema names some of those bytes. Write the name
    /// when it has one and the byte when it does not, rather than a number nobody can read or a
    /// name the wire did not say.
    fn types(&self, schema: &LaidOutSchema) -> Result<TokenStream, CodegenError> {
        let mut out = TokenStream::new();

        for def in schema.schema.types.values() {
            let TypeDef::Set(def) = def else { continue };
            if def.choices.is_empty() {
                continue;
            }

            let ty = def.name.type_ident();
            let choice_ty = format_ident!("{}Choice", ty);
            let primitive = encoding_primitive(&def.encoding, &schema.schema.types)
                .unwrap_or(Primitive::Uint8);
            let resolved = Resolved::Scalar(primitive);
            let host = primitive.host();
            let Some(read) = resolved.read(quote!(self.0)) else {
                continue;
            };

            let bits: Vec<u32> = def
                .choices
                .iter()
                .map(|c| c.value.parse().unwrap_or(0))
                .collect();
            let names: Vec<Ident> = def
                .choices
                .iter()
                .map(|c| c.name.variant_ident())
                .collect();
            let docs: Vec<_> = def.choices.iter().map(|c| c.description.clone()).collect();
            let variants = names.iter().zip(&bits).zip(&docs).map(|((name, bit), doc)| {
                let bit = *bit as u8;
                quote! { #doc #name = #bit, }
            });
            let named_mask: u64 = bits.iter().fold(0, |mask, bit| mask | (1u64 << bit));
            // host space: the wrapper has no bit ops of its own
            let named_mask = quote!(#named_mask as #host);
            let set = resolved.encode(quote!(raw));

            out.extend(quote! {
                /// One flag of the set, by the bit it occupies.
                #[repr(u8)]
                #[derive(
                    Debug, Clone, Copy, PartialEq, Eq, Hash,
                    serde::Serialize, serde::Deserialize,
                )]
                pub enum #choice_ty { #(#variants)* }

                impl #choice_ty {
                    pub const ALL: &'static [Self] = &[ #(Self::#names),* ];
                }

                impl #ty {
                    /// the flags that are set, in declaration order
                    pub fn choices(self) -> impl Iterator<Item = #choice_ty> {
                        let raw = #read;
                        #choice_ty::ALL
                            .iter()
                            .copied()
                            .filter(move |choice| raw & (1 << (*choice as u32)) != 0)
                    }
                }

                impl serde::Serialize for #ty {
                    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                        // a bit the schema never named would not survive the round trip, so the
                        // whole thing goes out as the number it is
                        if #read & !(#named_mask) != 0 {
                            return serde::Serialize::serialize(&#read, s);
                        }
                        s.collect_seq(self.choices())
                    }
                }

                impl<'de> serde::Deserialize<'de> for #ty {
                    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                        #[derive(serde::Deserialize)]
                        #[serde(untagged)]
                        enum Repr {
                            Choices(Vec<#choice_ty>),
                            Code(#host),
                        }

                        let raw = match Repr::deserialize(d)? {
                            Repr::Choices(choices) => choices
                                .into_iter()
                                .fold(0, |raw: #host, choice| raw | (1 << (choice as u32))),
                            Repr::Code(code) => code,
                        };
                        Ok(Self(#set))
                    }
                }
            });
        }

        for def in schema.schema.types.values() {
            let TypeDef::Enum(def) = def else { continue };
            if def.values.is_empty() {
                continue;
            }

            let ty = def.name.type_ident();
            let enum_ty = format_ident!("{}Enum", ty);
            let primitive = encoding_primitive(&def.encoding, &schema.schema.types)
                .unwrap_or(Primitive::Uint8);
            let resolved = Resolved::Scalar(primitive);
            let host = primitive.host();
            let Some(read) = resolved.read(quote!(self.0)) else {
                continue;
            };
            let known = resolved.encode(quote!(known as #host));
            let code = resolved.encode(quote!(code));

            out.extend(quote! {
                impl serde::Serialize for #ty {
                    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                        match self.as_enum() {
                            Some(known) => serde::Serialize::serialize(&known, s),
                            None => serde::Serialize::serialize(&#read, s),
                        }
                    }
                }

                impl<'de> serde::Deserialize<'de> for #ty {
                    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                        #[derive(serde::Deserialize)]
                        #[serde(untagged)]
                        enum Repr {
                            Known(#enum_ty),
                            Code(#host),
                        }

                        Ok(match Repr::deserialize(d)? {
                            Repr::Known(known) => Self(#known),
                            Repr::Code(code) => Self(#code),
                        })
                    }
                }
            });
        }

        Ok(out)
    }

    fn field(&self, placed: &PlacedField, field: &mut SynField) -> Result<(), CodegenError> {
        let name = placed.field.name.to_string();
        field.attrs.push(parse_quote!(#[serde(rename = #name)]));
        if let Some(path) = self.writer(placed) {
            field.attrs.push(parse_quote!(#[serde(with = #path)]))
        }
        Ok(())
    }

    /// A schema type's own members are wire types too, so they need the same attribute. Which
    /// one is decided from the declared type: this is the struct as written, not a field the
    /// layout pass placed.
    fn type_items(&self, def: &TypeDef, items: &mut [Item]) -> Result<(), CodegenError> {
        const WIRE: [&str; 8] = ["u8", "i8", "U16", "U32", "U64", "I16", "I32", "I64"];
        // a named enum's wire newtype writes itself as the name, from `types`, so the derive would
        // be a second impl of the same trait
        let named = match def {
            TypeDef::Enum(EnumDef { values, .. }) => !values.is_empty(),
            TypeDef::Set(SetDef { choices, .. }) => !choices.is_empty(),
            _ => false,
        };
        for item in items {
            // a value enum's variants are units, so the derive alone spells them by name
            if let Item::Enum(e) = item {
                e.attrs
                    .push(parse_quote!(#[derive(serde::Serialize, serde::Deserialize)]));
                continue;
            }
            let Item::Struct(s) = item else { continue };
            if named {
                continue;
            }
            s.attrs
                .push(parse_quote!(#[derive(serde::Serialize, serde::Deserialize)]));
            for field in &mut s.fields {
                let last = match &field.ty {
                    Type::Path(p) => p.path.segments.last().map(|s| s.ident.to_string()),
                    _ => None,
                };
                let with = match &field.ty {
                    _ if last.as_deref().is_some_and(|t| WIRE.contains(&t)) => {
                        Some("sbe_support::serde_wire")
                    }
                    Type::Array(_) => Some("sbe_support::serde_ascii"),
                    _ => None,
                };
                if let Some(path) = with {
                    field.attrs.push(parse_quote!(#[serde(with = #path)]));
                }
            }
        }
        Ok(())
    }
}
