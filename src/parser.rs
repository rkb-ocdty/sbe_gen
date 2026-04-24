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

use std::collections::HashMap;
use thiserror::Error;

/// A parsed SBE schema.
#[derive(Debug, Clone)]
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
    pub types: HashMap<String, TypeDef>,
    /// All messages declared in the schema.
    pub messages: Vec<Message>,
}

/// A user defined type from the `<types>` section of the schema.
#[derive(Debug, Clone)]
pub enum TypeDef {
    /// A named primitive definition from `<types>`, including optional
    /// fixed-length array, presence/null metadata, and constant content.
    Primitive {
        name: String,
        primitive: String,
        length: Option<usize>,
        presence: Option<String>,
        null_value: Option<String>,
        constant: Option<String>,
        description: Option<String>,
    },
    /// An enumeration with a specific underlying type and a set of valid values.
    Enum {
        name: String,
        encoding: String,
        values: Vec<(String, String, Option<String>)>,
        description: Option<String>,
    },
    /// A bit set where each choice corresponds to a bit index.
    Set {
        name: String,
        encoding: String,
        choices: Vec<(String, String, Option<String>)>,
        description: Option<String>,
    },
    /// A composite type made up of nested `type` and `ref` elements.
    Composite {
        name: String,
        fields: Vec<CompositeField>,
        description: Option<String>,
    },
}

/// A field inside a composite definition.
#[derive(Debug, Clone)]
pub enum CompositeField {
    /// An inline primitive member of a composite, including optional array
    /// length, presence/null metadata, and constant content.
    Type {
        name: String,
        primitive: String,
        length: Option<usize>,
        offset: Option<u32>,
        #[allow(dead_code)]
        presence: Option<String>,
        #[allow(dead_code)]
        null_value: Option<String>,
        #[allow(dead_code)]
        constant: Option<String>,
        #[allow(dead_code)]
        description: Option<String>,
    },
    /// A reference to a previously defined type.
    Ref {
        name: String,
        ty: String,
        offset: Option<u32>,
    },
}

/// A message defined in the schema.
#[derive(Debug, Clone)]
pub struct Message {
    /// The message name.
    pub name: String,
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
#[derive(Debug, Clone)]
pub enum MessageMember {
    Field(Field),
    Group(Group),
    Data(VarDataField),
}

/// A repeating group with its own fields, nested groups, and variable-length
/// data members.
#[derive(Debug, Clone)]
pub struct Group {
    /// Group name.
    pub name: String,
    /// Optional numeric identifier.
    #[allow(dead_code)]
    pub id: Option<u32>,
    /// The optional block length attribute for each entry.
    #[allow(dead_code)]
    pub block_length: Option<u32>,
    /// Name of the composite type that encodes the group dimensions.
    pub dimension_type: String,
    /// Members inside the group (fields, nested groups and data) in order.
    pub members: Vec<GroupMember>,
    /// sinceVersion of the group.
    pub since_version: Option<u32>,
    /// semanticType of the group if provided.
    pub semantic_type: Option<String>,
    /// Optional description attribute.
    pub description: Option<String>,
}

/// Members allowed within a group.
#[derive(Debug, Clone)]
pub enum GroupMember {
    Field(Field),
    Group(Group),
    Data(VarDataField),
}

/// A fixed-layout field on a message or group.
///
/// Groups and variable-length data are represented separately via
/// [`MessageMember`] and [`GroupMember`].
#[derive(Debug, Clone)]
pub struct Field {
    /// The field name.
    pub name: String,
    /// The field id attribute.
    #[allow(dead_code)]
    pub id: Option<u32>,
    /// The name of the type used by this field (primitive or user defined).
    pub ty: String,
    /// Optional fixed offset for the field within the block.
    pub offset: Option<u32>,
    /// Per-field byte order override.
    pub byte_order: Option<String>,
    /// minValue attribute if provided.
    pub min_value: Option<String>,
    /// maxValue attribute if provided.
    pub max_value: Option<String>,
    /// nullValue attribute if provided.
    pub null_value: Option<String>,
    /// initialValue attribute if provided.
    pub initial_value: Option<String>,
    /// The presence attribute if specified.
    pub presence: Option<String>,
    /// The sinceVersion attribute if specified.
    pub since_version: Option<u32>,
    /// The semanticType attribute if specified.
    pub semantic_type: Option<String>,
    /// The valueRef attribute if present (constants).
    pub value_ref: Option<String>,
    /// Constant literal from field text content when present.
    pub constant: Option<String>,
    /// Description attribute if present.
    pub description: Option<String>,
}

/// A variable length data field.
#[derive(Debug, Clone)]
pub struct VarDataField {
    /// The field name.
    pub name: String,
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
    pub ty: String,
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
        let mut found = None;
        for node in doc.descendants() {
            let name = node.tag_name().name();
            if name == "messageSchema" || name.ends_with(":messageSchema") {
                found = Some(node);
                break;
            }
        }
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
    let mut types = HashMap::new();
    for child in node.children() {
        if child.tag_name().name() == "types" {
            for ty_node in child.children() {
                if ty_node.is_element() {
                    match ty_node.tag_name().name() {
                        "type" => {
                            // <type name="foo" primitiveType="uint8" ...>
                            let name = attr_req(&ty_node, "name", "type")?;
                            let primitive =
                                attr_req(&ty_node, "primitiveType", &format!("type {}", name))?;
                            let length = ty_node
                                .attribute("length")
                                .and_then(|s| s.parse::<usize>().ok());
                            let presence = ty_node.attribute("presence").map(|s| s.to_string());
                            let null_value = ty_node.attribute("nullValue").map(|s| s.to_string());
                            let constant = ty_node.text().map(|s| s.trim().to_string());
                            types.insert(
                                name.clone(),
                                TypeDef::Primitive {
                                    name,
                                    primitive: primitive.to_string(),
                                    length,
                                    presence,
                                    null_value,
                                    constant,
                                    description: ty_node
                                        .attribute("description")
                                        .map(|s| s.to_string()),
                                },
                            );
                        }
                        "enum" => {
                            let name = attr_req(&ty_node, "name", "enum")?;
                            let encoding =
                                attr_req(&ty_node, "encodingType", &format!("enum {}", name))?;
                            let description =
                                ty_node.attribute("description").map(|s| s.to_string());
                            let mut values = Vec::new();
                            for val_node in ty_node.children() {
                                if val_node.is_element()
                                    && val_node.tag_name().name() == "validValue"
                                {
                                    let vname = attr_req(&val_node, "name", "validValue")?;
                                    let val = val_node.text().unwrap_or("").trim().to_string();
                                    let desc =
                                        val_node.attribute("description").map(|s| s.to_string());
                                    values.push((vname.to_string(), val, desc));
                                }
                            }
                            types.insert(
                                name.clone(),
                                TypeDef::Enum {
                                    name,
                                    encoding: encoding.to_string(),
                                    values,
                                    description,
                                },
                            );
                        }
                        "set" => {
                            let name = attr_req(&ty_node, "name", "set")?;
                            let encoding =
                                attr_req(&ty_node, "encodingType", &format!("set {}", name))?;
                            let description =
                                ty_node.attribute("description").map(|s| s.to_string());
                            let mut choices = Vec::new();
                            for ch_node in ty_node.children() {
                                if ch_node.is_element() && ch_node.tag_name().name() == "choice" {
                                    let cname = attr_req(&ch_node, "name", "choice")?;
                                    let bit = ch_node.text().unwrap_or("").trim().to_string();
                                    let desc =
                                        ch_node.attribute("description").map(|s| s.to_string());
                                    choices.push((cname.to_string(), bit, desc));
                                }
                            }
                            types.insert(
                                name.clone(),
                                TypeDef::Set {
                                    name,
                                    encoding: encoding.to_string(),
                                    choices,
                                    description,
                                },
                            );
                        }
                        "composite" => {
                            let name = attr_req(&ty_node, "name", "composite")?;
                            let description =
                                ty_node.attribute("description").map(|s| s.to_string());
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
                                        let primitive = attr_req(&f_node, "primitiveType", &fname)?;
                                        let length = f_node
                                            .attribute("length")
                                            .and_then(|s| s.parse::<usize>().ok());
                                        let offset = attr_opt_u32(&f_node, "offset", &fname)?;
                                        let presence =
                                            f_node.attribute("presence").map(|s| s.to_string());
                                        let null_value =
                                            f_node.attribute("nullValue").map(|s| s.to_string());
                                        let constant = f_node.text().map(|s| s.trim().to_string());
                                        fields.push(CompositeField::Type {
                                            name: fname,
                                            primitive: primitive.to_string(),
                                            length,
                                            offset,
                                            presence,
                                            null_value,
                                            constant,
                                            description: f_node
                                                .attribute("description")
                                                .map(|s| s.to_string()),
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
                                        fields.push(CompositeField::Ref {
                                            name: fname,
                                            ty: ty.to_string(),
                                            offset,
                                        });
                                    }
                                    _ => {}
                                }
                            }
                            types.insert(
                                name.clone(),
                                TypeDef::Composite {
                                    name,
                                    fields,
                                    description,
                                },
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
                            let byte_order = f.attribute("byteOrder").map(|s| s.to_string());
                            let min_value = f.attribute("minValue").map(|s| s.to_string());
                            let max_value = f.attribute("maxValue").map(|s| s.to_string());
                            let null_value = f.attribute("nullValue").map(|s| s.to_string());
                            let initial_value = f.attribute("initialValue").map(|s| s.to_string());
                            let presence = f.attribute("presence").map(|s| s.to_string());
                            let since_version = attr_opt_u32(&f, "sinceVersion", &fname)?;
                            let semantic_type = f.attribute("semanticType").map(|s| s.to_string());
                            let value_ref = f.attribute("valueRef").map(|s| s.to_string());
                            let constant = f
                                .text()
                                .map(|s| s.trim().to_string())
                                .filter(|s| !s.is_empty());
                            members.push(MessageMember::Field(Field {
                                name: fname,
                                id: fid,
                                ty: ftype.to_string(),
                                offset,
                                byte_order,
                                min_value,
                                max_value,
                                null_value,
                                initial_value,
                                presence,
                                since_version,
                                semantic_type,
                                value_ref,
                                constant,
                                description: f.attribute("description").map(|s| s.to_string()),
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
                                name: name.clone(),
                                id,
                                since_version: attr_opt_u32(&f, "sinceVersion", &name)?,
                                semantic_type: f.attribute("semanticType").map(|s| s.to_string()),
                                ty: ty.to_string(),
                            }));
                        }
                        _ => {}
                    }
                }
                messages.push(Message {
                    name: mname,
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
    let description = node.attribute("description").map(|s| s.to_string());
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
                let byte_order = child.attribute("byteOrder").map(|s| s.to_string());
                let presence = child.attribute("presence").map(|s| s.to_string());
                let value_ref = child.attribute("valueRef").map(|s| s.to_string());
                let constant = child
                    .text()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty());
                members.push(GroupMember::Field(Field {
                    name: fname.clone(),
                    id: fid,
                    ty: ftype.to_string(),
                    offset,
                    byte_order,
                    min_value: child.attribute("minValue").map(|s| s.to_string()),
                    max_value: child.attribute("maxValue").map(|s| s.to_string()),
                    null_value: child.attribute("nullValue").map(|s| s.to_string()),
                    initial_value: child.attribute("initialValue").map(|s| s.to_string()),
                    presence,
                    since_version: attr_opt_u32(&child, "sinceVersion", &fname)?,
                    semantic_type: child.attribute("semanticType").map(|s| s.to_string()),
                    value_ref,
                    constant,
                    description: child.attribute("description").map(|s| s.to_string()),
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
                    name: dname.clone(),
                    id: did,
                    since_version: attr_opt_u32(&child, "sinceVersion", &dname)?,
                    semantic_type: child.attribute("semanticType").map(|s| s.to_string()),
                    ty: ty.to_string(),
                }));
            }
            _ => {}
        }
    }
    Ok(Group {
        name,
        id,
        block_length,
        dimension_type,
        members,
        since_version,
        semantic_type,
        description,
    })
}
