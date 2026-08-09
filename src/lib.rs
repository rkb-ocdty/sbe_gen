//! # SBE code generator
//!
//! This crate exposes a small API for generating zero‑copy Rust
//! structures from an [SBE](https://github.com/aeron‑io/simple-binary-encoding)
//! XML schema.  At a high level the process is:
//!
//! 1. Parse the XML into an intermediate [`Schema`](parser::Schema) using
//!    [`parser::parse_schema`].
//! 2. Transform the schema into Rust code strings with
//!    [`codegen::generate`].
//! 3. Write the resulting files to disk via [`generate_to`].
//!
//! Generated structs derive the necessary traits from the
//! [`zerocopy`](https://docs.rs/zerocopy/latest/zerocopy/) crate and
//! include a `parse_prefix` helper so that a message can be viewed at
//! the front of a byte buffer without copying.

mod codegen;
mod parser;

use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

/// Options that influence how code is generated.
#[derive(Debug, Clone, Default)]
pub struct GeneratorOptions {
    /// Optional crate-level allow attribute line (e.g. `#![allow(...)]`).
    /// When set, it is inserted at the top of each generated file.
    pub allow_attr: Option<String>,
    /// Overrides for constant primitive type aliases. Keys are type
    /// names, values are type names to alias to.
    pub constant_type_aliases: HashMap<String, String>,
    /// Replacement types for schema types, keyed by the schema's own name. The replacement has
    /// to be laid out identically — it becomes the struct's field, so it is read straight off
    /// the wire, not converted.
    pub type_map: HashMap<String, String>,
}

/// Errors produced by the generator.
#[derive(Debug, Error)]
pub enum GeneratorError {
    #[error("failed to parse schema: {0}")]
    Parse(#[from] parser::ParseError),
    #[error("code generation failed: {0}")]
    Codegen(#[from] codegen::CodegenError),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Parse an XML schema and emit a collection of Rust source files.
///
/// The returned vector contains `(filename, contents)` tuples for each
/// generated module.  Common imports and helpers are included in every
/// file to make them standalone.
/// One emitted module: the file it belongs in and its source.
#[derive(Debug, Clone)]
pub struct GeneratedModule {
    pub name: String,
    pub source: String,
}

/// Everything a schema generates. Named rather than a bag of pairs, because the caller almost
/// always wants a particular one of these.
#[derive(Debug, Clone)]
pub struct Generated {
    pub types: GeneratedModule,
    /// a re-export of `sbe_support::MessageHeader`, kept because consumers import the module
    pub message_header: GeneratedModule,
    pub messages: Vec<GeneratedModule>,
    /// the groups more than one message declares, written once. `None` when no two messages in
    /// the schema hold the same group.
    pub groups: Option<GeneratedModule>,
    pub mod_rs: GeneratedModule,
    /// whole modules contributed by the extra derivation steps, if any ran
    pub derived: Vec<GeneratedModule>,
    /// every packed struct that was emitted, in the order it was written. Kept as syn rather
    /// than text so a caller can introspect the layout or dedup structurally identical groups.
    pub structs: Vec<syn::ItemStruct>,
}

impl Generated {
    pub fn modules(&self) -> impl Iterator<Item = &GeneratedModule> {
        [&self.types, &self.message_header]
            .into_iter()
            .chain(&self.groups)
            .chain(&self.messages)
            .chain([&self.mod_rs])
            .chain(&self.derived)
    }
}

/// A step that writes extra items over the laid-out schema. Implement it to add a
/// serialisation, a mapping, whatever the schema alone does not say.
pub use codegen::Derivation;
/// Serde impls for every block, entry and whole message, written from the schema's own layout.
pub use codegen::DeriveSerialize;

pub fn generate(schema_xml: &str, opts: &GeneratorOptions) -> Result<Generated, GeneratorError> {
    generate_with(schema_xml, opts, &[])
}

/// The same, with extra derivation steps.
pub fn generate_with(
    schema_xml: &str,
    opts: &GeneratorOptions,
    derivations: &[&dyn Derivation],
) -> Result<Generated, GeneratorError> {
    use codegen::{DedupedSchema, Emit, LaidOutSchema, LoweredSchema, ValidatedSchema};

    // the generator is a short-lived process and the options outlive every stage, so they are
    // leaked once here rather than threaded or cloned per stage
    let opts: &'static GeneratorOptions = Box::leak(Box::new(opts.clone()));

    let schema = parser::parse_schema(schema_xml)?;
    let deduped = DedupedSchema::new(schema);
    let validated = ValidatedSchema::new(deduped, opts)?;
    let lowered = LoweredSchema::new(validated)?;
    let laid_out = LaidOutSchema::new(lowered)?;
    Ok(Emit::new(&laid_out, derivations).generated()?)
}

/// Read an XML schema and write the generated modules into a target
/// directory.  This convenience function creates the directory if it
/// does not exist and writes one `.rs` file per message.
pub fn generate_to<P: AsRef<Path>>(
    schema_xml: &str,
    out_dir: P,
    opts: &GeneratorOptions,
) -> Result<(), GeneratorError> {
    generate_to_with(schema_xml, out_dir, opts, &[])
}

/// The same, with extra derivation steps.
pub fn generate_to_with<P: AsRef<Path>>(
    schema_xml: &str,
    out_dir: P,
    opts: &GeneratorOptions,
    derivations: &[&dyn Derivation],
) -> Result<(), GeneratorError> {
    use std::fs;
    use std::fs::File;
    use std::io::Write;
    let generated = generate_with(schema_xml, opts, derivations)?;
    let out_dir = out_dir.as_ref();
    if !out_dir.exists() {
        fs::create_dir_all(out_dir)?;
    }
    for module in generated.modules() {
        let mut f = File::create(out_dir.join(&module.name))?;
        f.write_all(module.source.as_bytes())?;
    }
    Ok(())
}
