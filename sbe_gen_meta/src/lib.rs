//! The `#[sbe_gen]` attribute.
//!
//! One definition, written by `sbe_gen` through `ToTokens` and read by `sbe_gen_derive` through
//! darling. Two hand-maintained copies of the same key list would drift, and a key spelled one
//! way on the way out and another on the way in fails silently: the macro sees a default and
//! writes plausible code for it.

use darling::{FromField, FromMeta};
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::{Expr, Ident, Path, Type};

/// What the generator hangs on the struct.
#[derive(Default, FromMeta)]
#[darling(default)]
pub struct Block {
    /// a message rather than a group entry: its builder owns its `Vec` and can stamp a header
    pub message: bool,
    /// what the struct has to measure, which is asserted rather than trusted
    pub size: Option<usize>,
    /// the block's width on the wire, which the schema can declare wider than the fields need
    pub block_length: Option<u16>,
    pub template_id: Option<u16>,
    pub schema_id: Option<u16>,
    pub schema_version: Option<u16>,
    #[darling(default)]
    pub since_version: Option<u32>,
    // a string, not a path like the fields': a message's semanticType is a FIX message-type
    // code, and "0" and "5" are not identifiers
    pub semantic_type: Option<String>,
    /// the composite holding this group's count and block length. Present only on a group
    /// entry, which is what tells one from a message.
    pub dimension: Option<Path>,
    /// the group's accessor name, sanitized by the generator, which a re-casing would not match
    pub name: Option<Ident>,
    /// the repeating groups the block declares
    #[darling(multiple)]
    pub group: Vec<GroupSpec>,
    /// the variable-length members trailing the block
    #[darling(multiple)]
    pub data: Vec<VarData>,
    /// the fields the schema gave a fixed value
    #[darling(multiple)]
    pub constant: Vec<Constant>,
}

/// What it hangs on each field. Padding carries only `skip` and `padding`; a field the schema
/// declared always knows where it goes.
#[derive(FromField)]
#[darling(attributes(sbe_gen))]
pub struct Meta {
    pub ident: Option<Ident>,
    pub ty: Type,
    #[darling(default)]
    /// the generator writes this field's setter itself
    pub skip: bool,
    /// bytes the block needs to reach the next field, not a value
    #[darling(default)]
    pub padding: bool,
    /// the schema's own `offset`, which becomes a constant as well as an assertion
    #[darling(default)]
    pub offset: Option<u32>,
    /// where the layout pass put it, when the schema did not say
    #[darling(default)]
    pub at: Option<usize>,
    pub since_version: Option<u32>,
    #[darling(default)]
    pub semantic_type: Option<Path>,
    /// what the field reads as, which the value helpers on the view hand back
    #[darling(default)]
    pub value: Option<Value>,
    /// present when the schema said the field may be absent
    #[darling(default)]
    pub optional: Option<Optional>,
    /// the schema marked it required, so it gets the helper that says so
    #[darling(default)]
    pub required: bool,
    /// length of the byte array, when the field is one
    #[darling(default)]
    pub string: Option<usize>,
    #[darling(default)]
    pub min: Option<String>,
    #[darling(default)]
    pub max: Option<String>,
    #[darling(default)]
    pub null: Option<String>,
    #[darling(default)]
    pub initial: Option<String>,
}

/// How a field reaches its raw integer. A schema type is a newtype over one, a byteorder
/// wrapper around one, or both.
#[derive(Clone, Copy, Default, PartialEq, Eq, FromMeta)]
#[darling(rename_all = "snake_case")]
pub enum Read {
    #[default]
    Plain,
    Get,
    Inner,
    InnerGet,
}

impl Read {
    pub fn key(self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::Get => "get",
            Self::Inner => "inner",
            Self::InnerGet => "inner_get",
        }
    }

    /// reaches the raw integer from the field
    pub fn tail(self) -> TokenStream {
        match self {
            Self::Plain => quote!(),
            Self::Get => quote!(.get()),
            Self::Inner => quote!(.0),
            Self::InnerGet => quote!(.0.get()),
        }
    }

    /// an enum or set hands back the wrapper it was read through, a primitive the raw value
    pub fn wrapper(self) -> bool {
        matches!(self, Self::Inner | Self::InnerGet)
    }
}

/// A repeating group the block carries.
#[derive(FromMeta)]
pub struct GroupSpec {
    /// where it sits among the block's members, which is the order it sits on the wire
    pub order: usize,
    /// the accessor's name, sanitized by the generator, which a re-casing here would not match
    pub name: Ident,
    /// the group's name, which its entry, view, iterator and builders are all named from
    pub ty: Ident,
    /// the composite holding its count and block length
    pub dimension: Path,
    /// bytes between entries, when every entry is the same width. An entry carrying its own
    /// groups or var-data has no stride and cannot be handed back as a slice.
    pub stride: Option<usize>,
}

/// A variable-length member: its name and the integer its length is written as.
#[derive(FromMeta)]
pub struct VarData {
    pub order: usize,
    pub name: Ident,
    pub len: Type,
}

/// A field whose value the schema fixed, so it takes no bytes and is not a member of the struct.
#[derive(FromMeta)]
pub struct Constant {
    /// the accessor's name
    pub name: Ident,
    /// the constant's own name
    pub ident: Ident,
    pub ty: Type,
    pub value: Expr,
    #[darling(default)]
    pub since_version: u32,
}

/// What the field hands back once it has been read through whatever wraps it.
#[derive(FromMeta)]
pub struct Value {
    // a string, because a field's type can be an array and darling parses those from one
    pub ty: Type,
    #[darling(default)]
    pub read: Read,
}

/// Present when the schema said the field may be absent.
#[derive(FromMeta)]
pub struct Optional {
    /// the sentinel the raw integer holds when the field is absent. An expression rather than
    /// a literal: a schema that names no null takes the primitive's, which reads `i64::MIN`.
    pub null: Option<Expr>,
    /// a float has no sentinel: absence is NaN
    #[darling(default)]
    pub nan: bool,
}

impl Optional {
    pub fn test(&self) -> TokenStream {
        let null = &self.null;
        match self.nan {
            true => quote!(raw.is_nan()),
            false => quote!(raw == #null),
        }
    }
}

/// One `key = value,` of the attribute, or nothing when there is nothing to say.
fn key(name: &str, value: Option<impl ToTokens>) -> TokenStream {
    let name = Ident::new(name, proc_macro2::Span::call_site());
    match value {
        Some(value) => quote!(#name = #value,),
        None => quote!(),
    }
}

fn flag(name: &str, set: bool) -> TokenStream {
    let name = Ident::new(name, proc_macro2::Span::call_site());
    match set {
        true => quote!(#name,),
        false => quote!(),
    }
}

impl ToTokens for Block {
    fn to_tokens(&self, out: &mut TokenStream) {
        let semantic = self.semantic_type.as_deref().filter(|s| !s.is_empty());
        let (group, data, constant) = (&self.group, &self.data, &self.constant);
        out.extend(quote! {
            #(#group)*
            #(#data)*
            #(#constant)*
        });
        out.extend(flag("message", self.message));
        out.extend(key("size", self.size));
        out.extend(key("block_length", self.block_length));
        out.extend(key("template_id", self.template_id));
        out.extend(key("schema_id", self.schema_id));
        out.extend(key("schema_version", self.schema_version));
        out.extend(key("since_version", self.since_version));
        out.extend(key("semantic_type", semantic));
        out.extend(key("dimension", self.dimension.as_ref()));
        out.extend(key("name", self.name.as_ref()));
    }
}

impl ToTokens for Meta {
    fn to_tokens(&self, out: &mut TokenStream) {
        out.extend(flag("skip", self.skip));
        out.extend(flag("padding", self.padding));
        out.extend(key("offset", self.offset));
        out.extend(key("at", self.at));
        out.extend(key("since_version", self.since_version.filter(|v| *v != 0)));
        out.extend(key("semantic_type", self.semantic_type.as_ref()));
        out.extend(key("string", self.string));
        out.extend(key("min", self.min.as_deref()));
        out.extend(key("max", self.max.as_deref()));
        out.extend(key("null", self.null.as_deref()));
        out.extend(key("initial", self.initial.as_deref()));
        if let Some(value) = &self.value {
            out.extend(quote!(#value,));
        }
        if let Some(optional) = &self.optional {
            out.extend(quote!(#optional,));
        }
        out.extend(flag("required", self.required));
    }
}

impl ToTokens for GroupSpec {
    fn to_tokens(&self, out: &mut TokenStream) {
        let (order, name, ty, dimension) = (self.order, &self.name, &self.ty, &self.dimension);
        let stride = key("stride", self.stride);
        out.extend(quote! {
            group(order = #order, name = #name, ty = #ty, dimension = #dimension, #stride),
        });
    }
}

impl ToTokens for VarData {
    fn to_tokens(&self, out: &mut TokenStream) {
        let (order, name, len) = (self.order, &self.name, &self.len);
        out.extend(quote!(data(order = #order, name = #name, len = #len),));
    }
}

impl ToTokens for Constant {
    fn to_tokens(&self, out: &mut TokenStream) {
        let (name, ident, ty, value) = (&self.name, &self.ident, &self.ty, &self.value);
        let ty = ty.to_token_stream().to_string();
        let value = value.to_token_stream().to_string();
        let since = self.since_version;
        out.extend(quote! {
            constant(
                name = #name, ident = #ident, ty = #ty, value = #value, since_version = #since
            ),
        });
    }
}

impl ToTokens for Value {
    fn to_tokens(&self, out: &mut TokenStream) {
        let ty = self.ty.to_token_stream().to_string();
        let read = self.read.key();
        out.extend(quote!(value(ty = #ty, read = #read)));
    }
}

impl ToTokens for Optional {
    fn to_tokens(&self, out: &mut TokenStream) {
        match (&self.null, self.nan) {
            (_, true) => out.extend(quote!(optional(nan))),
            (null, false) => out.extend(quote!(optional(null = #null))),
        }
    }
}
