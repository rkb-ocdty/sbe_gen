//! Code generation for parsed SBE schemas.
//!
//! The generator produces one Rust module per message along with a
//! `types.rs` module which contains definitions for user defined
//! enums, sets and composites.  A `mod.rs` file is also emitted to
//! re‑export the generated types for ergonomic use.
use heck::{ToShoutySnakeCase, ToUpperCamelCase};
use proc_macro2::{Ident, Literal, TokenStream};
use quote::{ToTokens, format_ident, quote};
use std::cell::RefCell;
use std::collections::HashSet;
use std::fmt::Display;
use syn::{Field as SynField, Item, ItemStruct, Path, Type, parse_quote};
use thiserror::Error;

mod derive;
pub(crate) mod emit;
pub(crate) mod ext;
pub use ext::DeriveSerialize;
pub(crate) mod groups;
mod layout;
pub(crate) mod lower;
mod model;
mod types;
mod validate;
mod walk;

pub use derive::Derivation;
#[allow(unused_imports)]
use derive::*;
pub(crate) use emit::Emit;
#[allow(unused_imports)]
use emit::*;
pub(crate) use groups::DedupedSchema;
#[allow(unused_imports)]
use groups::*;
pub(crate) use layout::LaidOutSchema;
use layout::*;
pub(crate) use lower::LoweredSchema;
use lower::*;
use model::*;
use types::*;
pub(crate) use validate::ValidatedSchema;
use validate::*;
use walk::*;

use crate::GeneratorOptions;
use crate::parser::{
    CompositeDef, CompositeField, CompositeKind, Docs, EnumDef, Field, Group, GroupMember, Message,
    MessageMember, Name, NamedValue, Presence, Primitive, PrimitiveDef, Schema, SetDef, TypeDef,
    VarDataField,
};
use derive_generic_visitor::{Drive, Visit};

#[derive(Debug, Error)]
pub enum CodegenError {
    /// The generator emitted something that is not valid Rust. Carries syn's error so the
    /// span survives, rather than being flattened into a message.
    #[error("generated invalid Rust: {0}")]
    InvalidRust(#[from] syn::Error),

    #[error(
        "identifier collision in {module} after Rust sanitization: two schema names both map \
         to '{ident}' in {scope}{context}"
    )]
    IdentifierCollision {
        module: String,
        ident: String,
        scope: String,
        context: String,
    },

    #[error("unsupported field type '{ty}' for {scope}")]
    UnsupportedFieldType { ty: Name, scope: String },

    #[error("unsupported var-data length type '{ty}' for {scope} data '{field}'")]
    UnsupportedVarDataLength {
        ty: Name,
        scope: String,
        field: Name,
    },

    #[error("cannot compute the encoded size of type '{ty}' for {scope}")]
    UncomputableSize { ty: Name, scope: String },

    #[error("unknown type '{ty}' referenced by {scope}")]
    UnknownType { ty: Name, scope: String },

    #[error("cyclic type reference in {scope}: {cycle}")]
    CyclicType { scope: String, cycle: String },

    #[error("unsupported encoding type '{encoding}' in '{ty}' used by {scope}")]
    UnsupportedEncoding {
        encoding: Name,
        ty: Name,
        scope: String,
    },

    #[error(
        "composite '{composite}' field '{field}' starts at {start} but previous layout ends \
         at {end}"
    )]
    OverlappingComposite {
        composite: Name,
        field: Name,
        start: usize,
        end: usize,
    },

    #[error("composite '{composite}' field '{field}' has unsupported layout")]
    UnsupportedCompositeLayout { composite: Name, field: Name },

    #[error("composite '{composite}' layout overflows while processing field '{field}'")]
    CompositeLayoutOverflow { composite: Name, field: Name },

    /// Validation reports everything it found, not just the first thing.
    #[error("{}", .0.iter().map(ToString::to_string).collect::<Vec<_>>().join("; "))]
    Several(Vec<CodegenError>),

    #[error("dimensionType '{ty}' in {scope} {problem}")]
    Dimension {
        ty: Name,
        scope: String,
        problem: DimensionProblem,
    },
}

impl CodegenError {
    /// Report everything a pass found, not just the first thing.
    pub(crate) fn all(errors: impl Iterator<Item = Self>) -> Result<(), Self> {
        match errors.collect::<Vec<_>>() {
            none if none.is_empty() => Ok(()),
            mut one if one.len() == 1 => Err(one.remove(0)),
            many => Err(Self::Several(many)),
        }
    }
}

/// What is wrong with a group's `dimensionType`. Split out so the dimension errors share one
/// variant instead of one apiece.
#[derive(Debug, Error)]
pub enum DimensionProblem {
    #[error("is not declared")]
    Unknown,
    #[error("must be a composite")]
    NotComposite,
    #[error("must expose at least two usable integer fields")]
    TooFewFields,
    #[error("does not expose a usable blockLength field")]
    NoBlockLength,
    #[error("does not expose a usable numInGroup/count field")]
    NoCount,
    #[error("field '{0}' must resolve to an integer type")]
    NotAnInteger(Name),
    #[error("field '{0}' has no resolvable offset")]
    UnresolvableOffset(Name),
    #[error("has no computable size")]
    NoSize,
}

impl<T: Display + ?Sized> SbeName for T {
    fn sanitized(&self) -> String {
        crate::parser::sanitize(&self.to_string())
    }
    fn type_ident(&self) -> Ident {
        format_ident!("{}", self.sanitized())
    }
    fn const_ident(&self) -> Ident {
        format_ident!("{}", self.to_string().to_shouty_snake_case().sanitized())
    }
    fn variant_ident(&self) -> Ident {
        format_ident!("{}", self.to_string().to_upper_camel_case().sanitized())
    }
    fn suffix_snake(&self, suffix: impl Display) -> Ident {
        let mut out = self.to_string();
        if !out.ends_with('_') {
            out.push('_');
        }
        out.push_str(suffix.to_string().trim_start_matches('_'));
        format_ident!("{}", out.sanitized())
    }
}

impl Schema {
    pub(crate) fn type_is_optional(&self, ty: &str) -> bool {
        matches!(
            self.types.get(ty),
            Some(TypeDef::Primitive(PrimitiveDef {
                presence: Presence::Optional,
                ..
            }))
        ) || matches!(
            self.types.get(ty),
            Some(TypeDef::Primitive(PrimitiveDef {
                null_value: Some(_),
                ..
            }))
        )
    }

    pub(crate) fn type_null_value(&self, ty: &str) -> Option<&str> {
        match self.types.get(ty) {
            Some(TypeDef::Primitive(PrimitiveDef { null_value, .. })) => null_value.as_deref(),
            _ => None,
        }
    }

    pub(crate) fn type_length(&self, ty: &str) -> Option<usize> {
        match self.types.get(ty) {
            Some(TypeDef::Primitive(PrimitiveDef { length, .. })) => *length,
            _ => None,
        }
    }

    pub(crate) fn type_primitive(&self, ty: &str) -> Option<Primitive> {
        match self.types.get(ty) {
            Some(TypeDef::Primitive(PrimitiveDef { primitive, .. })) => Some(*primitive),
            _ => None,
        }
    }

    /// A declared primitive is already resolved; an enum/set encoding and a bare field type
    /// are type references, so those still have to be looked up.
    pub(crate) fn field_primitive(&self, field: &Field) -> Option<Primitive> {
        match self.types.get(&field.ty) {
            Some(TypeDef::Primitive(PrimitiveDef { primitive, .. })) => Some(*primitive),
            Some(
                TypeDef::Enum(EnumDef { encoding, .. }) | TypeDef::Set(SetDef { encoding, .. }),
            ) => Primitive::parse(encoding),
            Some(TypeDef::Composite(CompositeDef { .. })) => None,
            None => Primitive::parse(&field.ty),
        }
    }

    pub(crate) fn field_is_constant(&self, field: &Field) -> bool {
        if field.presence == Presence::Constant {
            return true;
        }
        if field.value_ref.is_some() {
            return true;
        }
        if field.presence != Presence::Inherited {
            return false;
        }
        matches!(
            self.types.get(&field.ty),
            Some(TypeDef::Primitive(PrimitiveDef {
                presence: Presence::Constant,
                ..
            }))
        )
    }

    pub(crate) fn field_is_optional(&self, field: &Field) -> bool {
        match field.presence {
            Presence::Optional => true,
            Presence::Constant | Presence::Required => false,
            _ => field.null_value.is_some() || self.type_is_optional(&field.ty),
        }
    }

    pub(crate) fn field_null_value<'a>(&'a self, field: &'a Field) -> Option<&'a str> {
        field
            .null_value
            .as_deref()
            .or_else(|| self.type_null_value(&field.ty))
    }

    pub(crate) fn field_is_required(&self, field: &Field) -> bool {
        match field.presence {
            Presence::Required => true,
            Presence::Optional | Presence::Constant => false,
            Presence::Inherited => !self.field_is_optional(field),
        }
    }

    pub(crate) fn constant_type_alias_from_schema(&self, type_name: &Name) -> Option<Name> {
        let mut candidates: HashSet<Name> = HashSet::new();
        for field in SchemaFields::default().visit_by_val_infallible(self).0 {
            if &field.ty != type_name {
                continue;
            }
            if let Some(value_ref) = &field.value_ref
                && let Some((ty, _)) = value_ref.split_once('.')
                && ty != &**type_name
                && let Some(td) = self.types.get(ty)
                && !matches!(td, TypeDef::Composite(CompositeDef { .. }))
            {
                candidates.insert(ty.into());
            }
            if &field.name != type_name
                && let Some(td) = self.types.get(&field.name)
                && !matches!(td, TypeDef::Composite(CompositeDef { .. }))
            {
                candidates.insert(field.name.clone());
            }
        }
        if candidates.len() == 1 {
            candidates.into_iter().next()
        } else {
            None
        }
    }
}

// names the generated modules import unqualified, so a schema type sanitizing to one of
// these would shadow it
pub(crate) const IMPORTED_NAMES: &[&str] = &[
    "bool",
    "char",
    "str",
    "isize",
    "usize",
    "u8",
    "i8",
    "u16",
    "i16",
    "u32",
    "i32",
    "u64",
    "i64",
    "f32",
    "f64",
    "Ref",
    "FromBytes",
    "IntoBytes",
    "KnownLayout",
    "Immutable",
    "Unaligned",
    "U16",
    "I16",
    "U32",
    "I32",
    "U64",
    "I64",
    "F32",
    "F64",
];

/// Two schema names can sanitize to the same Rust identifier. Rather than predicting what
/// the emitters will name things, look at what they actually named: every namespace the
/// generated file declares is visible in the parsed AST, so this can't drift from the
/// emitters the way a parallel walk over the schema does.
/// A type without its lifetimes or parameters. Rust pools the items of every impl on a type
/// whatever the block wrote in its header, so the scope key has to ignore that too.
fn base_name(ty: &str) -> String {
    ty.split('<').next().unwrap_or(ty).trim().to_string()
}

/// The methods `#[sbe_gen]` will write for a block, which are not in the file this sees and
/// would otherwise only collide once the generated crate is compiled.
fn sbe_gen_names(s: &syn::ItemStruct) -> Vec<(String, String)> {
    let is_sbe_gen = |a: &&syn::Attribute| {
        a.path()
            .segments
            .last()
            .is_some_and(|s| s.ident == "sbe_gen")
    };
    let carries = |attrs: &[syn::Attribute], key: &str| {
        attrs
            .iter()
            .filter(is_sbe_gen)
            .any(|a| a.to_token_stream().to_string().contains(key))
    };
    if !s.attrs.iter().any(|a| is_sbe_gen(&a)) {
        return Vec::new();
    }
    let (view, block) = (format!("impl {}View", s.ident), format!("impl {}", s.ident));
    let mut names = vec![(block.clone(), "parse_prefix".to_string())];
    if carries(&s.attrs, "message") {
        names.push((view.clone(), "is_fixed_layout".to_string()));
    }
    for f in &s.fields {
        let Some(ident) = f.ident.as_ref().map(Ident::to_string) else {
            continue;
        };
        // padding is the only member with no place of its own, and it gets no methods
        if !carries(&f.attrs, "offset") && !carries(&f.attrs, "at") {
            continue;
        }
        names.push((view.clone(), format!("has_{ident}")));
        names.push((view.clone(), ident.clone()));
        if carries(&f.attrs, "optional") {
            names.push((block.clone(), format!("{ident}_opt")));
        }
        if carries(&f.attrs, "value") {
            names.push((view.clone(), format!("{ident}_value")));
        }
        if carries(&f.attrs, "required") {
            names.push((view.clone(), format!("{ident}_required")));
        }
        if carries(&f.attrs, "string") {
            for suffix in ["bytes", "str", "str_trimmed"] {
                names.push((view.clone(), format!("{ident}_{suffix}")));
            }
        }
    }
    names
}

pub(crate) fn check_collisions(
    module: &str,
    file: &syn::File,
    rendered: &str,
) -> Result<(), CodegenError> {
    use syn::{ImplItem, Item};

    let mut declared: Vec<(String, String)> = Vec::new();
    for item in &file.items {
        match item {
            Item::Struct(s) => {
                declared.push(("module scope".into(), s.ident.to_string()));
                let scope = format!("struct {}", s.ident);
                declared.extend(
                    s.fields
                        .iter()
                        .filter_map(|f| Some((scope.clone(), f.ident.as_ref()?.to_string()))),
                );
                declared.extend(sbe_gen_names(s));
            }
            Item::Enum(e) => {
                declared.push(("module scope".into(), e.ident.to_string()));
                let scope = format!("enum {}", e.ident);
                declared.extend(
                    e.variants
                        .iter()
                        .map(|v| (scope.clone(), v.ident.to_string())),
                );
            }
            Item::Type(t) => declared.push(("module scope".into(), t.ident.to_string())),
            // `const _` is the layout assertion; every one of them is called `_` and none of
            // them is a name anything can collide with
            Item::Const(c) if c.ident == "_" => {}
            Item::Const(c) => declared.push(("module scope".into(), c.ident.to_string())),
            Item::Fn(f) => declared.push(("module scope".into(), f.sig.ident.to_string())),
            Item::Mod(m) => declared.push(("module scope".into(), m.ident.to_string())),
            // several impl blocks can target one type and Rust pools their items, so the
            // scope key is the target rather than the block
            Item::Impl(i) => {
                let target = match &i.trait_ {
                    Some((path, _)) => format!(
                        "impl {} for {}",
                        path.to_token_stream(),
                        base_name(&i.self_ty.to_token_stream().to_string())
                    ),
                    None => format!(
                        "impl {}",
                        base_name(&i.self_ty.to_token_stream().to_string())
                    ),
                };
                declared.extend(i.items.iter().filter_map(|member| {
                    Some((
                        target.clone(),
                        match member {
                            ImplItem::Fn(f) => f.sig.ident.to_string(),
                            ImplItem::Const(c) => c.ident.to_string(),
                            ImplItem::Type(t) => t.ident.to_string(),
                            _ => return None,
                        },
                    ))
                }));
            }
            _ => {}
        }
    }

    let mut seen: HashSet<(String, String)> = IMPORTED_NAMES
        .iter()
        .map(|n| ("module scope".to_string(), n.to_string()))
        .collect();
    for (scope, name) in declared {
        if seen.insert((scope.clone(), name.clone())) {
            continue;
        }
        // point at the emitted line rather than describing it: that is the context
        let declares = |line: &&str| {
            ["fn ", "const ", "mod ", "struct ", "type ", "enum "]
                .iter()
                .filter_map(|kw| Some(line.find(&format!("{kw}{name}"))? + kw.len() + name.len()))
                .any(|end| !line[end..].starts_with(|c: char| c.is_alphanumeric() || c == '_'))
        };
        let context = rendered
            .lines()
            .rfind(declares)
            .or_else(|| rendered.lines().find(|l| l.contains(&name)))
            .map(|line| format!("\n  --> {}", line.trim()))
            .unwrap_or_default();
        return Err(CodegenError::IdentifierCollision {
            module: module.to_string(),
            ident: name,
            scope,
            context,
        });
    }
    Ok(())
}

// the banner is an inner doc attribute rather than a // comment so it survives the
// parse2/unparse round trip like everything else instead of being stitched on as text
/// A blank line after each top-level item. prettyplease emits none, and a file that is one
/// declaration hard against the next is difficult to read. Only a closing brace at column zero
/// ends an item — an attribute belongs to what follows it, and the imports are one block.
fn spaced(rendered: String) -> String {
    let mut out = String::with_capacity(rendered.len());
    let mut previous_closed = false;
    for line in rendered.lines() {
        if previous_closed && !line.is_empty() {
            out.push('\n');
        }
        out.push_str(line);
        out.push('\n');
        previous_closed = line == "}";
    }
    out
}

pub(crate) fn render_file(
    module: &str,
    allows: &[&str],
    body: TokenStream,
) -> Result<String, CodegenError> {
    let allows = allows.iter().copied().map(frag);
    let file = syn::parse2::<syn::File>(quote! {
        #(#allows)*
        #![doc = " Automatically generated by sbe_gen."]
        #body
    })?;
    let rendered = spaced(prettyplease::unparse(&file));
    check_collisions(module, &file, &rendered)?;
    Ok(rendered)
}
