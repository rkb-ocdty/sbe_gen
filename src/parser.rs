//! Parsing of SBE XML schemas.
//!
//! This module defines a very small subset of the SBE grammar that is
//! sufficient for generating zero‑copy Rust types.  The parser reads
//! primitive type definitions, enums, sets, composites and messages
//! with fields.  Nested groups and variable‑length data are recognised
//! but ignored, allowing the generator to skip over portions of the
//! message that it does not yet support.

use std::collections::HashMap;
use thiserror::Error;

/// A parsed SBE schema.
#[derive(Debug, Clone)]
pub struct Schema {
    /// The optional package name declared on the root element.
    #[allow(dead_code)]
    pub package: Option<String>,
    /// A mapping of user defined types (enums, sets, composites).
    pub types: HashMap<String, TypeDef>,
    /// All messages declared in the schema.
    pub messages: Vec<Message>,
}

/// A user defined type from the `<types>` section of the schema.
#[derive(Debug, Clone)]
pub enum TypeDef {
    /// A simple alias around a primitive type (e.g. `<type name="Foo" primitiveType="uint8"/>`).
    Primitive {
        name: String,
        primitive: String,
        length: Option<usize>,
        presence: Option<String>,
        constant: Option<String>,
    },
    /// An enumeration with a specific underlying type and a set of valid values.
    Enum {
        name: String,
        encoding: String,
        values: Vec<(String, String)>,
    },
    /// A bit set where each choice corresponds to a bit index.
    Set {
        name: String,
        encoding: String,
        choices: Vec<(String, String)>,
    },
    /// A composite type made up of nested `type` and `ref` elements.
    Composite {
        name: String,
        fields: Vec<CompositeField>,
    },
}

/// A field inside a composite definition.
#[derive(Debug, Clone)]
pub enum CompositeField {
    /// A primitive type with an optional constant value.
    Type {
        name: String,
        primitive: String,
        length: Option<usize>,
        #[allow(dead_code)]
        presence: Option<String>,
        #[allow(dead_code)]
        constant: Option<String>,
    },
    /// A reference to a previously defined type.
    Ref { name: String, ty: String },
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

/// A repeating group with its own fixed fields and optional variable data.
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
}

/// Members allowed within a group.
#[derive(Debug, Clone)]
pub enum GroupMember {
    Field(Field),
    Group(Group),
    Data(VarDataField),
}

/// A field on a message.  Only fields that map to fixed‑length types
/// are preserved; groups and variable‑length data are skipped.
#[derive(Debug, Clone)]
pub struct Field {
    /// The field name.
    pub name: String,
    /// The field id attribute.
    #[allow(dead_code)]
    pub id: Option<u32>,
    /// The name of the type used by this field (primitive or user defined).
    pub ty: String,
    /// The presence attribute if specified.
    pub presence: Option<String>,
    /// The valueRef attribute if present (constants).
    pub value_ref: Option<String>,
}

/// A variable length data field.
#[derive(Debug, Clone)]
pub struct VarDataField {
    /// The field name.
    pub name: String,
    /// The optional field id.
    #[allow(dead_code)]
    pub id: Option<u32>,
    /// The encoding type used for the length prefix.
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

/// Internal helper to parse from a particular `messageSchema` node.
fn parse_schema_from_node(node: roxmltree::Node) -> Result<Schema, ParseError> {
    // package attribute is optional
    let package = node.attribute("package").map(|s| s.to_string());
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
                            let constant = ty_node.text().map(|s| s.trim().to_string());
                            types.insert(
                                name.clone(),
                                TypeDef::Primitive {
                                    name,
                                    primitive: primitive.to_string(),
                                    length,
                                    presence,
                                    constant,
                                },
                            );
                        }
                        "enum" => {
                            let name = attr_req(&ty_node, "name", "enum")?;
                            let encoding =
                                attr_req(&ty_node, "encodingType", &format!("enum {}", name))?;
                            let mut values = Vec::new();
                            for val_node in ty_node.children() {
                                if val_node.is_element()
                                    && val_node.tag_name().name() == "validValue"
                                {
                                    let vname = attr_req(&val_node, "name", "validValue")?;
                                    let val = val_node.text().unwrap_or("").trim().to_string();
                                    values.push((vname.to_string(), val));
                                }
                            }
                            types.insert(
                                name.clone(),
                                TypeDef::Enum {
                                    name,
                                    encoding: encoding.to_string(),
                                    values,
                                },
                            );
                        }
                        "set" => {
                            let name = attr_req(&ty_node, "name", "set")?;
                            let encoding =
                                attr_req(&ty_node, "encodingType", &format!("set {}", name))?;
                            let mut choices = Vec::new();
                            for ch_node in ty_node.children() {
                                if ch_node.is_element() && ch_node.tag_name().name() == "choice" {
                                    let cname = attr_req(&ch_node, "name", "choice")?;
                                    let bit = ch_node.text().unwrap_or("").trim().to_string();
                                    choices.push((cname.to_string(), bit));
                                }
                            }
                            types.insert(
                                name.clone(),
                                TypeDef::Set {
                                    name,
                                    encoding: encoding.to_string(),
                                    choices,
                                },
                            );
                        }
                        "composite" => {
                            let name = attr_req(&ty_node, "name", "composite")?;
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
                                        let presence =
                                            f_node.attribute("presence").map(|s| s.to_string());
                                        let constant = f_node.text().map(|s| s.trim().to_string());
                                        fields.push(CompositeField::Type {
                                            name: fname,
                                            primitive: primitive.to_string(),
                                            length,
                                            presence,
                                            constant,
                                        });
                                    }
                                    "ref" => {
                                        let fname = attr_req(
                                            &f_node,
                                            "name",
                                            &format!("composite ref in {}", name),
                                        )?;
                                        let ty = attr_req(&f_node, "type", &fname)?;
                                        fields.push(CompositeField::Ref {
                                            name: fname,
                                            ty: ty.to_string(),
                                        });
                                    }
                                    _ => {}
                                }
                            }
                            types.insert(name.clone(), TypeDef::Composite { name, fields });
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
                            let presence = f.attribute("presence").map(|s| s.to_string());
                            let value_ref = f.attribute("valueRef").map(|s| s.to_string());
                            members.push(MessageMember::Field(Field {
                                name: fname,
                                id: fid,
                                ty: ftype.to_string(),
                                presence,
                                value_ref,
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
                                name,
                                id,
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
                let presence = child.attribute("presence").map(|s| s.to_string());
                let value_ref = child.attribute("valueRef").map(|s| s.to_string());
                members.push(GroupMember::Field(Field {
                    name: fname,
                    id: fid,
                    ty: ftype.to_string(),
                    presence,
                    value_ref,
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
                    name: dname,
                    id: did,
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
    })
}
