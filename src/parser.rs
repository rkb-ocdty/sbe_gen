//! Parsing of SBE XML schemas.
//!
//! This module builds a lightweight in-memory representation of the
//! subset of SBE XML needed by the generator. It preserves primitive,
//! enum, set, and composite type definitions, message fields, nested
//! groups, and variable-length data members, along with the metadata
//! the generator currently consumes.
//!
//! The parser is intentionally permissive: it records structure and
//! attributes but does not try to perform full semantic validation of
//! the schema beyond required attributes and integer parsing.

use derive_generic_visitor::Drive;
use heck::{ToShoutySnakeCase, ToSnakeCase};
use proc_macro2::{Ident, TokenStream};
use quote::{ToTokens, format_ident, quote};
use std::collections::BTreeMap;
use strum::IntoEnumIterator;
use syn::{Attribute, parse_quote};
use thiserror::Error;

const RUST_KEYWORDS: &[&str] = &[
    "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn", "for",
    "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return",
    "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where",
    "while", "async", "await", "dyn", "abstract", "become", "box", "do", "final", "macro",
    "override", "priv", "typeof", "unsized", "virtual", "yield", "try", "union", "_",
];

pub fn sanitize(name: &str) -> String {
    let mut out = String::with_capacity(name.len().max(1));
    let mut prev_underscore = false;
    for (idx, ch) in name.chars().enumerate() {
        let valid = if idx == 0 {
            ch == '_' || ch.is_ascii_alphabetic()
        } else {
            ch == '_' || ch.is_ascii_alphanumeric()
        };
        if valid {
            out.push(ch);
            prev_underscore = false;
        } else if !prev_underscore {
            out.push('_');
            prev_underscore = true;
        }
    }
    if out.is_empty() {
        out.push('_');
    }
    if out
        .as_bytes()
        .first()
        .map(|b| b.is_ascii_digit())
        .unwrap_or(false)
    {
        out.insert(0, '_');
    }
    if RUST_KEYWORDS.contains(&out.as_str()) {
        out.push('_');
    }
    if out == "_" {
        out.push('_');
    }
    out
}

/// Whether a field carries a value on the wire.
///
/// `Inherited` is not "unknown" — it is what the schema means by omitting the attribute: take
/// the presence of the type being referenced. Saying `required` explicitly is different, because
/// that overrides a `constant` declared on that type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Drive)]
pub enum Presence {
    #[default]
    Inherited,
    Required,
    Optional,
    Constant,
}

impl Presence {
    fn parse(raw: Option<&str>) -> Self {
        match raw {
            Some("required") => Self::Required,
            Some("optional") => Self::Optional,
            Some("constant") => Self::Constant,
            _ => Self::Inherited,
        }
    }
}

/// A name as written in the schema. Every declaration has one, and the Rust spellings it
/// can take are all derived here rather than at each use.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Drive)]
pub struct Name(String);

impl Name {
    pub fn type_ident(&self) -> Ident {
        format_ident!("{}", sanitize(&self.0))
    }
    pub fn snake_ident(&self) -> Ident {
        format_ident!("{}", sanitize(&self.0.to_snake_case()))
    }
    pub fn const_ident(&self) -> Ident {
        format_ident!("{}", sanitize(&self.0.to_shouty_snake_case()))
    }
}

impl<T: Into<String>> From<T> for Name {
    fn from(s: T) -> Self {
        Self(s.into())
    }
}

impl std::ops::Deref for Name {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

impl std::borrow::Borrow<str> for Name {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Name {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A parsed SBE schema.
#[derive(Debug, Clone, Drive)]
pub struct Schema {
    /// The optional package name declared on the root element.
    #[allow(dead_code)]
    pub package: Option<String>,
    /// Optional schema id from the root messageSchema element.
    pub schema_id: Option<u32>,
    /// Version declared on the schema.
    pub version: Option<u32>,
    /// A mapping of named types from `<types>`, including primitive aliases,
    /// enums, sets, and composites.
    pub types: BTreeMap<Name, TypeDef>,
    /// All messages declared in the schema.
    pub messages: Vec<Message>,
}

/// A schema description, kept as the doc attributes it is emitted as — one per line, so a
/// multi-line description stays readable in the generated source.
#[derive(Debug, Clone, Drive)]
#[drive(skip)]
pub struct Docs {
    pub text: String,
    pub attrs: Vec<Attribute>,
}

impl Docs {
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let attrs = text
            .lines()
            .map(|line| {
                let line = format!(" {}", line.trim_end());
                parse_quote!(#[doc = #line])
            })
            .collect();
        Self { text, attrs }
    }
}

impl ToTokens for Docs {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let attrs = &self.attrs;
        tokens.extend(quote!(#(#attrs)*));
    }
}

/// A user defined type from the `<types>` section of the schema.
#[derive(Debug, Clone, Drive, derive_more::IsVariant, derive_more::TryUnwrap)]
#[try_unwrap(ref)]
pub enum TypeDef {
    Primitive(PrimitiveDef),
    Enum(EnumDef),
    Set(SetDef),
    Composite(CompositeDef),
}

/// A named primitive definition from `<types>`, including optional fixed-length array,
/// presence/null metadata, and constant content.
#[derive(Debug, Clone, Drive)]
pub struct PrimitiveDef {
    pub name: Name,
    pub primitive: Primitive,
    pub length: Option<usize>,
    pub presence: Presence,
    pub null_value: Option<String>,
    pub constant: Option<String>,
    pub description: Option<Docs>,
}

/// An enumeration with a specific underlying type and a set of valid values.
#[derive(Debug, Clone, Drive)]
pub struct EnumDef {
    pub name: Name,
    pub encoding: Name,
    pub values: Vec<NamedValue>,
    pub description: Option<Docs>,
}

/// A bit set where each choice corresponds to a bit index.
#[derive(Debug, Clone, Drive)]
pub struct SetDef {
    pub name: Name,
    pub encoding: Name,
    pub choices: Vec<NamedValue>,
    pub description: Option<Docs>,
}

/// A composite type made up of nested `type` and `ref` elements.
#[derive(Debug, Clone, Drive)]
pub struct CompositeDef {
    pub name: Name,
    pub fields: Vec<CompositeField>,
    pub description: Option<Docs>,
}

/// One entry of an enum or a bit set. `value` is the literal as written in the schema: a
/// variant's encoded value for an enum, a bit index for a set.
#[derive(Debug, Clone, Drive)]
pub struct NamedValue {
    pub name: Name,
    pub value: String,
    pub description: Option<Docs>,
}

/// A field inside a composite definition.
#[derive(Debug, Clone, Drive)]
pub struct CompositeField {
    pub name: Name,
    pub offset: Option<u32>,
    pub kind: CompositeKind,
}

/// What a composite member actually encodes. `name` and `offset` are common to both and
/// live on [`CompositeField`], because layout only ever needs those two.
#[derive(Debug, Clone, Drive)]
pub enum CompositeKind {
    /// An inline primitive member, including optional array length, presence/null
    /// metadata, and constant content.
    Type {
        primitive: Primitive,
        length: Option<usize>,
        presence: Presence,
        null_value: Option<String>,
        constant: Option<String>,
        #[allow(dead_code)]
        description: Option<Docs>,
    },
    /// A reference to a previously defined type.
    Ref { ty: Name },
}

impl CompositeField {
    /// Constant members carry their value in the schema, so they occupy no wire bytes.
    pub fn is_constant(&self) -> bool {
        matches!(
            self.kind,
            CompositeKind::Type {
                presence: Presence::Constant,
                ..
            }
        )
    }
}

/// A message defined in the schema.
#[derive(Debug, Clone, Drive)]
pub struct Message {
    /// The message name.
    pub name: Name,
    /// The numeric identifier of the message.
    #[allow(dead_code)]
    pub id: u32,
    /// The optional block length attribute.
    #[allow(dead_code)]
    pub block_length: Option<u32>,
    /// sinceVersion of the message.
    pub since_version: Option<u32>,
    /// semanticType if provided.
    pub semantic_type: Option<String>,
    /// All message members (fields, groups, variable data) in order.
    pub members: Vec<MessageMember>,
}

/// A member of a message body.
#[derive(Debug, Clone, Drive)]
pub enum MessageMember {
    Field(Field),
    Group(Group),
    Data(VarDataField),
}

/// A repeating group with its own fields, nested groups, and variable-length
/// data members.
#[derive(Debug, Clone, Drive)]
pub struct Group {
    /// Group name.
    pub name: Name,
    /// Optional numeric identifier.
    #[allow(dead_code)]
    pub id: Option<u32>,
    /// The optional block length attribute for each entry.
    #[allow(dead_code)]
    pub block_length: Option<u32>,
    /// Name of the composite type that encodes the group dimensions.
    pub dimension_type: Name,
    /// Members inside the group (fields, nested groups and data) in order.
    pub members: Vec<GroupMember>,
    /// sinceVersion of the group.
    pub since_version: Option<u32>,
    /// semanticType of the group if provided.
    pub semantic_type: Option<String>,
    /// Optional description attribute.
    pub description: Option<Docs>,
}

/// Two groups are the same declaration when they hold the same members — same fields, same
/// types, same order, so the same layout. Everything else a group carries is bookkeeping: the
/// FIX tag codegen never reads, and a description that in CME's own schema says "Date and Time"
/// in one copy and "Date and time" in the next.
impl PartialEq for Group {
    fn eq(&self, other: &Self) -> bool {
        // blockLength too: two groups can hold the same fields and still sit a different
        // distance apart on the wire, and sharing one definition would use the wrong stride
        self.members == other.members && self.block_length == other.block_length
    }
}

impl Eq for Group {}

impl std::hash::Hash for Group {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        self.members.hash(h);
    }
}

impl PartialEq for Field {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.ty == other.ty
            && self.offset == other.offset
            && self.presence == other.presence
            && self.min_value == other.min_value
            && self.max_value == other.max_value
            && self.null_value == other.null_value
            && self.initial_value == other.initial_value
            && self.value_ref == other.value_ref
            && self.constant == other.constant
    }
}

impl Eq for Field {}

impl std::hash::Hash for Field {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        self.name.hash(h);
        self.ty.hash(h);
        self.offset.hash(h);
        self.presence.hash(h);
        self.min_value.hash(h);
        self.max_value.hash(h);
        self.null_value.hash(h);
        self.initial_value.hash(h);
        self.value_ref.hash(h);
        self.constant.hash(h);
    }
}

impl PartialEq for VarDataField {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && self.ty == other.ty
    }
}

impl Eq for VarDataField {}

impl std::hash::Hash for VarDataField {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        self.name.hash(h);
        self.ty.hash(h);
    }
}

// one row per SBE primitive. everything the generator knows about a primitive is derived
// from this table, so adding one can't leave a match arm behind somewhere else.
macro_rules! primitives {
    ($($variant:ident = $sbe:literal, $rust:ident, $host:ident, $size:literal, $null:expr;)+) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumIter)]
        pub enum Primitive { $($variant),+ }

        impl Primitive {
            pub fn rust(self) -> Ident {
                match self { $(Self::$variant => format_ident!(stringify!($rust))),+ }
            }
            pub fn size(self) -> usize {
                match self { $(Self::$variant => $size),+ }
            }
            pub fn sbe_name(self) -> &'static str {
                match self { $(Self::$variant => $sbe),+ }
            }
            pub fn host(self) -> Ident {
                match self { $(Self::$variant => format_ident!(stringify!($host))),+ }
            }
            pub fn null_name(self) -> &'static str {
                match self { $(Self::$variant => stringify!($null)),+ }
            }
        }
    };
}

primitives! {
    Char    = "char",    u8,  u8,  1, u8::MAX;
    Boolean = "boolean", u8,  u8,  1, u8::MAX;
    Uint8   = "uint8",   u8,  u8,  1, u8::MAX;
    Int8    = "int8",    i8,  i8,  1, i8::MIN;
    Uint16  = "uint16",  U16, u16, 2, u16::MAX;
    Int16   = "int16",   I16, i16, 2, i16::MIN;
    Uint32  = "uint32",  U32, u32, 4, u32::MAX;
    Int32   = "int32",   I32, i32, 4, i32::MIN;
    Uint64  = "uint64",  U64, u64, 8, u64::MAX;
    Int64   = "int64",   I64, i64, 8, i64::MIN;
    Float   = "float",   F32, f32, 4, f32::NAN;
    Double  = "double",  F64, f64, 8, f64::NAN;
}

impl std::fmt::Display for Primitive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.sbe_name())
    }
}

impl Primitive {
    pub fn parse(name: &str) -> Option<Self> {
        Self::iter().find(|p| p.sbe_name() == name)
    }

    pub fn is_float(self) -> bool {
        matches!(self, Self::Float | Self::Double)
    }
}

/// Members allowed within a group.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Drive)]
pub enum GroupMember {
    Field(Field),
    Group(Group),
    Data(VarDataField),
}

macro_rules! members {
    ($($owner:ty => $member:ident),+ $(,)?) => { $(
        impl $owner {
            members!(@of $member, fields, Field, Field);
            members!(@of $member, groups, Group, Group);
            members!(@of $member, data, Data, VarDataField);
        }
    )+ };
    (@of $member:ident, $method:ident, $variant:ident, $ty:ty) => {
        pub fn $method(&self) -> Vec<&$ty> {
            self.members
                .iter()
                .filter_map(|m| match m {
                    $member::$variant(x) => Some(x),
                    _ => None,
                })
                .collect()
        }
    };
}

members!(Message => MessageMember, Group => GroupMember);

/// A fixed-layout field on a message or group.
///
/// Groups and variable-length data are represented separately via
/// [`MessageMember`] and [`GroupMember`].
#[derive(Debug, Clone, Drive)]
pub struct Field {
    /// The field name.
    pub name: Name,
    /// The field id attribute.
    #[allow(dead_code)]
    pub id: Option<u32>,
    /// The name of the type used by this field (primitive or user defined).
    pub ty: Name,
    /// Optional fixed offset for the field within the block.
    pub offset: Option<u32>,
    /// minValue attribute if provided.
    pub min_value: Option<String>,
    /// maxValue attribute if provided.
    pub max_value: Option<String>,
    /// nullValue attribute if provided.
    pub null_value: Option<String>,
    /// initialValue attribute if provided.
    pub initial_value: Option<String>,
    /// The presence attribute if specified.
    pub presence: Presence,
    /// The sinceVersion attribute if specified.
    pub since_version: Option<u32>,
    /// The semanticType attribute if specified.
    pub semantic_type: Option<String>,
    /// The valueRef attribute if present (constants).
    pub value_ref: Option<String>,
    /// Constant literal from field text content when present.
    pub constant: Option<String>,
    /// Description attribute if present.
    pub description: Option<Docs>,
}

/// A variable length data field.
#[derive(Debug, Clone, Drive)]
pub struct VarDataField {
    /// The field name.
    pub name: Name,
    /// The optional field id.
    #[allow(dead_code)]
    pub id: Option<u32>,
    /// sinceVersion of the data field.
    #[allow(dead_code)]
    pub since_version: Option<u32>,
    /// semanticType if provided.
    #[allow(dead_code)]
    pub semantic_type: Option<String>,
    /// The referenced encoding type for the data field. In practice this is
    /// often a composite that describes the length prefix and trailing bytes.
    pub ty: Name,
}

/// Errors that can occur while parsing a schema.
#[derive(Debug, Error)]
pub enum ParseError {
    #[error("XML parse error: {0}")]
    Xml(#[from] roxmltree::Error),
    #[error("expected <types> or <message> section in schema")]
    MissingSections,
    #[error("missing required attribute '{0}' on element '{1}'")]
    MissingAttribute(String, String),
    #[error("invalid integer value for attribute '{0}' on element '{1}'")]
    InvalidInt(String, String),
    #[error("unsupported primitiveType '{0}' on element '{1}'")]
    UnsupportedPrimitive(String, String),
}

/// `primitiveType` names one of the SBE primitives; resolving it here means nothing downstream
/// re-parses the string.
fn primitive_req(node: &roxmltree::Node, owner: &str) -> Result<Primitive, ParseError> {
    let raw = attr_req(node, "primitiveType", owner)?.to_string();
    Primitive::parse(&raw)
        .ok_or_else(|| ParseError::UnsupportedPrimitive(raw.to_string(), owner.to_string()))
}

/// Parse an SBE schema from an XML string.
///
/// Accepts either a root `<messageSchema>` element or a document that
/// contains a nested or namespace-qualified `messageSchema` element.
pub fn parse_schema(xml: &str) -> Result<Schema, ParseError> {
    let doc = roxmltree::Document::parse(xml)?;
    let root = doc.root_element();
    if root.tag_name().name() != "messageSchema" && root.tag_name().name() != "sbe:messageSchema" {
        // try to find nested messageSchema
        let found = doc.descendants().find(|node| {
            let name = node.tag_name().name();
            name == "messageSchema" || name.ends_with(":messageSchema")
        });
        if let Some(node) = found {
            return parse_schema_from_node(node);
        }
        return Err(ParseError::MissingSections);
    }
    parse_schema_from_node(root)
}

/// Internal helper to parse from a specific `messageSchema` node once it has
/// been located in the XML document.
fn parse_schema_from_node(node: roxmltree::Node) -> Result<Schema, ParseError> {
    // package attribute is optional
    let package = node.attribute("package").map(|s| s.to_string());
    let schema_id = attr_opt_u32(&node, "schemaId", "messageSchema")?;
    let version = attr_opt_u32(&node, "version", "messageSchema")?;
    // build type map
    let mut types = BTreeMap::new();
    for child in node.children() {
        if child.tag_name().name() == "types" {
            for ty_node in child.children() {
                if ty_node.is_element() {
                    match ty_node.tag_name().name() {
                        "type" => {
                            // <type name="foo" primitiveType="uint8" ...>
                            let name = attr_req(&ty_node, "name", "type")?;
                            let primitive = primitive_req(&ty_node, &format!("type {name}"))?;
                            let length = ty_node
                                .attribute("length")
                                .and_then(|s| s.parse::<usize>().ok());
                            let presence = Presence::parse(ty_node.attribute("presence"));
                            let null_value = ty_node.attribute("nullValue").map(|s| s.to_string());
                            let constant = ty_node.text().map(|s| s.trim().to_string());
                            types.insert(
                                Name::from(name.clone()),
                                TypeDef::Primitive(PrimitiveDef {
                                    name: name.into(),
                                    primitive,
                                    length,
                                    presence,
                                    null_value,
                                    constant,
                                    description: ty_node.attribute("description").map(Docs::new),
                                }),
                            );
                        }
                        "enum" => {
                            let name = attr_req(&ty_node, "name", "enum")?;
                            let encoding =
                                attr_req(&ty_node, "encodingType", &format!("enum {}", name))?;
                            let description = ty_node.attribute("description").map(Docs::new);
                            let values = ty_node
                                .children()
                                .filter(|val_node| {
                                    val_node.is_element()
                                        && val_node.tag_name().name() == "validValue"
                                })
                                .map(|val_node| {
                                    let vname = attr_req(&val_node, "name", "validValue")?;
                                    let val = val_node.text().unwrap_or("").trim().to_string();
                                    Ok(NamedValue {
                                        name: vname.into(),
                                        value: val,
                                        description: val_node
                                            .attribute("description")
                                            .map(Docs::new),
                                    })
                                })
                                .collect::<Result<Vec<_>, ParseError>>()?;
                            types.insert(
                                Name::from(name.clone()),
                                TypeDef::Enum(EnumDef {
                                    name: name.into(),
                                    encoding: encoding.into(),
                                    values,
                                    description,
                                }),
                            );
                        }
                        "set" => {
                            let name = attr_req(&ty_node, "name", "set")?;
                            let encoding =
                                attr_req(&ty_node, "encodingType", &format!("set {}", name))?;
                            let description = ty_node.attribute("description").map(Docs::new);
                            let choices = ty_node
                                .children()
                                .filter(|ch_node| {
                                    ch_node.is_element() && ch_node.tag_name().name() == "choice"
                                })
                                .map(|ch_node| {
                                    let cname = attr_req(&ch_node, "name", "choice")?;
                                    let bit = ch_node.text().unwrap_or("").trim().to_string();
                                    Ok(NamedValue {
                                        name: cname.into(),
                                        value: bit,
                                        description: ch_node
                                            .attribute("description")
                                            .map(Docs::new),
                                    })
                                })
                                .collect::<Result<Vec<_>, ParseError>>()?;
                            types.insert(
                                Name::from(name.clone()),
                                TypeDef::Set(SetDef {
                                    name: name.into(),
                                    encoding: encoding.into(),
                                    choices,
                                    description,
                                }),
                            );
                        }
                        "composite" => {
                            let name = attr_req(&ty_node, "name", "composite")?;
                            let description = ty_node.attribute("description").map(Docs::new);
                            let mut fields = Vec::new();
                            for f_node in ty_node.children() {
                                if !f_node.is_element() {
                                    continue;
                                }
                                match f_node.tag_name().name() {
                                    "type" => {
                                        let fname = attr_req(
                                            &f_node,
                                            "name",
                                            &format!("composite field in {}", name),
                                        )?;
                                        let primitive = primitive_req(&f_node, &fname)?;
                                        let length = f_node
                                            .attribute("length")
                                            .and_then(|s| s.parse::<usize>().ok());
                                        let offset = attr_opt_u32(&f_node, "offset", &fname)?;
                                        let presence =
                                            Presence::parse(f_node.attribute("presence"));
                                        let null_value =
                                            f_node.attribute("nullValue").map(|s| s.to_string());
                                        let constant = f_node.text().map(|s| s.trim().to_string());
                                        fields.push(CompositeField {
                                            name: fname.into(),
                                            offset,
                                            kind: CompositeKind::Type {
                                                primitive,
                                                length,
                                                presence,
                                                null_value,
                                                constant,
                                                description: f_node
                                                    .attribute("description")
                                                    .map(Docs::new),
                                            },
                                        });
                                    }
                                    "ref" => {
                                        let fname = attr_req(
                                            &f_node,
                                            "name",
                                            &format!("composite ref in {}", name),
                                        )?;
                                        let ty = attr_req(&f_node, "type", &fname)?;
                                        let offset = attr_opt_u32(&f_node, "offset", &fname)?;
                                        fields.push(CompositeField {
                                            name: fname.into(),
                                            offset,
                                            kind: CompositeKind::Ref { ty: ty.into() },
                                        });
                                    }
                                    _ => {}
                                }
                            }
                            types.insert(
                                Name::from(name.clone()),
                                TypeDef::Composite(CompositeDef {
                                    name: name.into(),
                                    fields,
                                    description,
                                }),
                            );
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    // parse messages
    let mut messages = Vec::new();
    for child in node.children() {
        if child.is_element() {
            let name = child.tag_name().name();
            if name == "message" || name.ends_with(":message") {
                // message element
                let mname = attr_req(&child, "name", "message")?;
                let mid = attr_opt_u32(&child, "id", &mname)?;
                let block_length = attr_opt_u32(&child, "blockLength", &mname)?;
                let since_version = attr_opt_u32(&child, "sinceVersion", &mname)?;
                let semantic_type = child.attribute("semanticType").map(|s| s.to_string());
                let mut members = Vec::new();
                for f in child.children() {
                    if !f.is_element() {
                        continue;
                    }
                    match f.tag_name().name() {
                        "field" => {
                            let fname =
                                attr_req(&f, "name", &format!("field in message {}", mname))?;
                            let fid = attr_opt_u32(&f, "id", &fname)?;
                            let ftype = attr_req(&f, "type", &fname)?;
                            let offset = attr_opt_u32(&f, "offset", &fname)?;
                            let min_value = f.attribute("minValue").map(|s| s.to_string());
                            let max_value = f.attribute("maxValue").map(|s| s.to_string());
                            let null_value = f.attribute("nullValue").map(|s| s.to_string());
                            let initial_value = f.attribute("initialValue").map(|s| s.to_string());
                            let presence = Presence::parse(f.attribute("presence"));
                            let since_version = attr_opt_u32(&f, "sinceVersion", &fname)?;
                            let semantic_type = f.attribute("semanticType").map(|s| s.to_string());
                            let value_ref = f.attribute("valueRef").map(|s| s.to_string());
                            let constant = f
                                .text()
                                .map(|s| s.trim().to_string())
                                .filter(|s| !s.is_empty());
                            members.push(MessageMember::Field(Field {
                                name: fname.into(),
                                id: fid,
                                ty: ftype.into(),
                                offset,
                                min_value,
                                max_value,
                                null_value,
                                initial_value,
                                presence,
                                since_version,
                                semantic_type,
                                value_ref,
                                constant,
                                description: f.attribute("description").map(Docs::new),
                            }));
                        }
                        "group" => {
                            let group = parse_group(&f, &mname)?;
                            members.push(MessageMember::Group(group));
                        }
                        "data" => {
                            let name = attr_req(&f, "name", &format!("data in message {}", mname))?;
                            let id = attr_opt_u32(&f, "id", &name)?;
                            let ty = attr_req(&f, "type", &name)?;
                            members.push(MessageMember::Data(VarDataField {
                                name: name.clone().into(),
                                id,
                                since_version: attr_opt_u32(&f, "sinceVersion", &name)?,
                                semantic_type: f.attribute("semanticType").map(|s| s.to_string()),
                                ty: ty.into(),
                            }));
                        }
                        _ => {}
                    }
                }
                messages.push(Message {
                    name: mname.into(),
                    id: mid.unwrap_or(0),
                    block_length,
                    since_version,
                    semantic_type,
                    members,
                });
            }
        }
    }
    if messages.is_empty() {
        return Err(ParseError::MissingSections);
    }
    Ok(Schema {
        package,
        schema_id,
        version,
        types,
        messages,
    })
}

fn attr_req(node: &roxmltree::Node, attr: &str, elem_desc: &str) -> Result<String, ParseError> {
    match node.attribute(attr) {
        Some(v) => Ok(v.to_string()),
        None => Err(ParseError::MissingAttribute(attr.into(), elem_desc.into())),
    }
}

fn attr_opt_u32(
    node: &roxmltree::Node,
    attr: &str,
    elem_desc: &str,
) -> Result<Option<u32>, ParseError> {
    match node.attribute(attr) {
        Some(v) => v
            .parse::<u32>()
            .map(Some)
            .map_err(|_| ParseError::InvalidInt(attr.into(), elem_desc.into())),
        None => Ok(None),
    }
}

fn parse_group(node: &roxmltree::Node, parent_name: &str) -> Result<Group, ParseError> {
    let name = attr_req(node, "name", &format!("group in {}", parent_name))?;
    let id = attr_opt_u32(node, "id", &name)?;
    let block_length = attr_opt_u32(node, "blockLength", &name)?;
    let dimension_type = attr_req(node, "dimensionType", &name)?;
    let since_version = attr_opt_u32(node, "sinceVersion", &name)?;
    let semantic_type = node.attribute("semanticType").map(|s| s.to_string());
    let description = node.attribute("description").map(Docs::new);
    let mut members = Vec::new();
    for child in node.children() {
        if !child.is_element() {
            continue;
        }
        match child.tag_name().name() {
            "field" => {
                let fname = attr_req(
                    &child,
                    "name",
                    &format!("field in group {} of {}", name, parent_name),
                )?;
                let fid = attr_opt_u32(&child, "id", &fname)?;
                let ftype = attr_req(&child, "type", &fname)?;
                let offset = attr_opt_u32(&child, "offset", &fname)?;
                let presence = Presence::parse(child.attribute("presence"));
                let value_ref = child.attribute("valueRef").map(|s| s.to_string());
                let constant = child
                    .text()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty());
                members.push(GroupMember::Field(Field {
                    name: fname.clone().into(),
                    id: fid,
                    ty: ftype.into(),
                    offset,
                    min_value: child.attribute("minValue").map(|s| s.to_string()),
                    max_value: child.attribute("maxValue").map(|s| s.to_string()),
                    null_value: child.attribute("nullValue").map(|s| s.to_string()),
                    initial_value: child.attribute("initialValue").map(|s| s.to_string()),
                    presence,
                    since_version: attr_opt_u32(&child, "sinceVersion", &fname)?,
                    semantic_type: child.attribute("semanticType").map(|s| s.to_string()),
                    value_ref,
                    constant,
                    description: child.attribute("description").map(Docs::new),
                }));
            }
            "group" => {
                let nested = parse_group(&child, &name)?;
                members.push(GroupMember::Group(nested));
            }
            "data" => {
                let dname = attr_req(
                    &child,
                    "name",
                    &format!("data in group {} of {}", name, parent_name),
                )?;
                let did = attr_opt_u32(&child, "id", &dname)?;
                let ty = attr_req(&child, "type", &dname)?;
                members.push(GroupMember::Data(VarDataField {
                    name: dname.clone().into(),
                    id: did,
                    since_version: attr_opt_u32(&child, "sinceVersion", &dname)?,
                    semantic_type: child.attribute("semanticType").map(|s| s.to_string()),
                    ty: ty.into(),
                }));
            }
            _ => {}
        }
    }
    Ok(Group {
        name: name.into(),
        id,
        block_length,
        dimension_type: dimension_type.into(),
        members,
        since_version,
        semantic_type,
        description,
    })
}

#[cfg(test)]
mod dedupe_tests {
    use super::*;

    /// The four secdef groups are declared identically in Future54/Option55/Spread56, but with
    /// prose that drifts — one description drops a word, another says "Date and time" where its
    /// twin says "Date and Time". They are the same group, and equality has to say so.
    #[test]
    fn identical_groups_compare_equal_across_messages() {
        let xml = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/schemas/032_cme_mdp3.xml"
        ))
        .expect("cme fixture");
        let schema = parse_schema(&xml).expect("parse");

        let mut distinct: Vec<&Group> = Vec::new();
        let mut total = 0usize;
        for msg in &schema.messages {
            for g in msg.groups() {
                total += 1;
                if !distinct.contains(&g) {
                    distinct.push(g);
                }
            }
        }
        assert_eq!(total, 19, "top-level groups in the CME schema");
        assert_eq!(distinct.len(), 11, "distinct group definitions");

        for name in ["NoEvents", "NoMDFeedTypes", "NoInstAttrib", "NoLotTypeRules"] {
            let shared: Vec<_> = schema
                .messages
                .iter()
                .flat_map(|m| m.groups())
                .filter(|g| &*g.name == name)
                .collect();
            assert_eq!(shared.len(), 3, "{name} occurrences");
            assert!(
                shared.iter().all(|g| *g == shared[0]),
                "{name} should compare equal across all three instrument definitions"
            );
        }

        // and the ones that genuinely differ still do
        let md: Vec<_> = schema
            .messages
            .iter()
            .flat_map(|m| m.groups())
            .filter(|g| &*g.name == "NoMDEntries")
            .collect();
        assert_eq!(md.len(), 3);
        assert!(md[1] != md[0] && md[2] != md[0] && md[2] != md[1]);
    }
}
