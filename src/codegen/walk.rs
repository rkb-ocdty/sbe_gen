use super::*;
use derive_generic_visitor::{Continue, ControlFlow, Visit, Visitor};
use std::collections::BTreeMap;

/// Which schema types are actually referenced. A constant type nothing references is emitted
/// as a `const`; a referenced one needs an alias instead, so this decides which of the two
/// `generate_types` produces.
#[derive(Default, Visitor, Visit)]
#[visit(drive(
    Schema,
    Message,
    MessageMember,
    GroupMember,
    TypeDef,
    CompositeField,
    CompositeDef
))]
#[visit(enter(Field, VarDataField, Group, CompositeKind))]
#[visit(drive(for<T> Vec<T>, for<T> Option<T>))]
#[visit(skip(
    Name,
    Primitive,
    Presence,
    String,
    u32,
    usize,
    bool,
    PrimitiveDef,
    EnumDef,
    SetDef,
    NamedValue,
    Docs
))]
pub(crate) struct UsedTypes(pub(crate) HashSet<Name>);

impl UsedTypes {
    fn enter_field(&mut self, field: &Field) {
        self.0.insert(field.ty.clone());
    }

    fn enter_var_data_field(&mut self, data: &VarDataField) {
        self.0.insert(data.ty.clone());
    }

    fn enter_group(&mut self, group: &Group) {
        self.0.insert(group.dimension_type.clone());
    }

    fn enter_composite_kind(&mut self, kind: &CompositeKind) {
        if let CompositeKind::Ref { ty } = kind {
            self.0.insert(ty.clone());
        }
    }
}

// the crate drives Vec and Option but not maps, and the type table is keyed by name
impl<'a> Visit<'a, BTreeMap<Name, TypeDef>> for UsedTypes {
    fn visit(&mut self, types: &'a BTreeMap<Name, TypeDef>) -> ControlFlow<Self::Break> {
        for def in types.values() {
            self.visit(def)?;
        }
        Continue(())
    }
}

/// What alias resolution needs off each fixed field. Owned rather than borrowed so the
/// visitor carries no lifetime: three names is cheaper than fighting for the reference.
pub(crate) struct FieldKey {
    pub(crate) name: Name,
    pub(crate) ty: Name,
    pub(crate) value_ref: Option<String>,
}

/// Every fixed field the schema declares, in order. Composites are skipped: their members
/// are `CompositeField`, laid out separately.
#[derive(Default, Visitor, Visit)]
#[visit(drive(Schema, Message, MessageMember, Group, GroupMember))]
#[visit(skip(Docs))]
#[visit(enter(Field))]
#[visit(drive(for<T> Vec<T>, for<T> Option<T>))]
#[visit(skip(Name, Primitive, Presence, String, u32, usize, bool, VarDataField))]
#[visit(skip(BTreeMap<Name, TypeDef>))]
pub(crate) struct SchemaFields(pub(crate) Vec<FieldKey>);

impl SchemaFields {
    fn enter_field(&mut self, field: &Field) {
        self.0.push(FieldKey {
            name: field.name.clone(),
            ty: field.ty.clone(),
            value_ref: field.value_ref.clone(),
        });
    }
}
