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

use std::path::Path;
use thiserror::Error;

/// Options that influence how code is generated.
#[derive(Debug, Clone)]
pub struct GeneratorOptions {
    /// Endianness to use for multi‑byte fields.  Valid values are
    /// `"little"` or `"big"`.  Defaults to `"little"`.
    pub endian: String,
}

impl Default for GeneratorOptions {
    fn default() -> Self {
        Self {
            endian: "little".into(),
        }
    }
}

/// Errors produced by the generator.
#[derive(Debug, Error)]
pub enum GeneratorError {
    #[error("failed to parse schema: {0}")]
    Parse(#[from] parser::ParseError),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Parse an XML schema and emit a collection of Rust source files.
///
/// The returned vector contains `(filename, contents)` tuples for each
/// generated module.  The common macro and imports are included in
/// every file to make them standalone.
pub fn generate(
    schema_xml: &str,
    opts: &GeneratorOptions,
) -> Result<Vec<(String, String)>, GeneratorError> {
    let schema = parser::parse_schema(schema_xml)?;
    Ok(codegen::generate(&schema, opts))
}

/// Read an XML schema and write the generated modules into a target
/// directory.  This convenience function creates the directory if it
/// does not exist and writes one `.rs` file per message.
pub fn generate_to<P: AsRef<Path>>(
    schema_xml: &str,
    out_dir: P,
    opts: &GeneratorOptions,
) -> Result<(), GeneratorError> {
    use std::fs;
    use std::fs::File;
    use std::io::Write;
    let modules = generate(schema_xml, opts)?;
    let out_dir = out_dir.as_ref();
    if !out_dir.exists() {
        fs::create_dir_all(out_dir)?;
    }
    for (fname, contents) in modules {
        let path = out_dir.join(&fname);
        let mut f = File::create(&path)?;
        f.write_all(contents.as_bytes())?;
    }
    Ok(())
}
