# SBE Generator for Rust

This crate provides a small, pragmatic compiler for the
[Simple Binary Encoding (SBE)](https://github.com/aeron‑io/simple-binary-encoding) protocol.  It reads
SBE XML schemas and produces zero‑copy Rust message types that are
ready for use in performance‑sensitive applications.  Generated
structures rely on the [`zerocopy`](https://docs.rs/zerocopy/latest/zerocopy/) crate to provide
safe, alignment‑aware views over raw byte buffers.

Unlike the reference SBE toolchain, this project focuses solely on
Rust and keeps the API minimal and easy to use.  It does not support
code generation for other languages.

## Features

* **Zero‑copy decoding:** Generated message structs derive
  `FromBytes`, `IntoBytes`, `KnownLayout`, `Immutable` and `Unaligned` so
  they can be safely cast from network buffers without copying.
* **Spec‑aware encoding builders:** Each message gets a `FooBuilder`
  companion that writes the fixed block, groups and variable data with
  the correct offsets, byte order and length prefixes. Builders can also
  emit the standard SBE message header.
* **Byte‑order aware fields:** Multi‑byte integer and floating‑point
  fields use the `zerocopy::byteorder` types (e.g.
  `little_endian::U32`, `little_endian::F64`) so that endianness is
  explicit and efficient.
* **Field offsets and padding:** Explicit `offset` attributes are
  honoured, with padding inserted to keep layout in sync with the SBE
  block length.
* **Acting-version aware decoding:** Helpers accept the standard SBE
  message header, apply the advertised block length/version at runtime,
  and expose presence checks so older payloads still parse safely.
* **Declarative parsing helpers:** Each generated message implements a
  `parse_prefix` helper that leverages `zerocopy::Ref` to split a slice
  into a typed prefix and a remainder.
* **Groups and variable data included:** Nested repeating groups are
  emitted with iterable views and entry structs, and `data` fields
  become `VarData` slices with an ergonomic `as_str()` helper.
* **Optional fields:** `presence="optional"` fields stay zero‑copy but
  gain `<field>_opt()` accessors that return `Option` based on the SBE
  null value for that primitive.
* **Constant field correctness:** `presence="constant"` fields are
  treated as non-encoded wire data. Generated code exposes associated
  constants plus constant accessors, and builders/encoders do not emit
  writes for those fields.
* **Schema reflection:** Generated code surfaces `SINCE_VERSION`,
  `SEMANTIC_TYPE`, field offsets and constraint constants so you can
  reason about compatibility at the call site.

## Usage

Add the `sbe_gen` crate to your `Cargo.toml` and build a small driver
program which reads your XML schema and writes the generated code to
disk:

```rust
use std::{fs, path::Path};
use sbe_gen::{generate_to, GeneratorOptions};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let schema_xml = fs::read_to_string("./my-schema.xml")?;
    let out_dir = Path::new("./src/sbe");
    generate_to(&schema_xml, out_dir, &GeneratorOptions::default())?;
    Ok(())
}
```

Alternatively, install the CLI binary with `cargo install --path .` and
run it directly:

```shell
cargo run --bin sbe_gen -- -i path/to/my-schema.xml -o src/sbe
```

For a more complete guide with end-to-end examples (fixed-size and
variable-size messages), see [docs/USAGE.md](docs/USAGE.md).

This will create a module in `src/sbe` containing one Rust file per
message defined in the schema.  Each file starts with common
imports and includes a `parse_prefix` helper on each message type.  For example, given a
message header like:

```xml
<message name="PacketHdr" id="0" blockLength="12">
  <field name="seq" id="1" type="uint32" />
  <field name="sending_time" id="2" type="uint64" />
</message>
```

the generator produces the following Rust code:

```rust
use zerocopy::{Ref, FromBytes, IntoBytes, KnownLayout, Immutable, Unaligned};
use zerocopy::byteorder::little_endian::{U32, U64};

#[repr(C)]
#[derive(Debug, FromBytes, IntoBytes, KnownLayout, Immutable, Unaligned, Clone, Copy)]
pub struct PacketHdr {
    pub seq: U32,
    pub sending_time: U64,
}

impl PacketHdr {
    #[inline]
    pub fn parse_prefix(body: &[u8]) -> Option<(&Self, &[u8])> {
        Ref::<_, Self>::from_prefix(body)
            .ok()
            .map(|(r, b)| (Ref::into_ref(r), b))
    }
}
```

## Build-time integration (build.rs)

If you want generated modules to live inside your crate (like
`examples/cme_mdp3_pcap_dump`), use a build script that runs the
generator at compile time.

1) Add the build dependency:

```toml
[build-dependencies]
sbe_gen = "0.7.1"
```

2) Organize schemas under `schemas/<schema_name>/`:

```
schemas/
  my_schema/
    templates_FixBinary.xml   # preferred name
```

If `templates_FixBinary.xml` is not present, the build script below
expects exactly one `.xml` file in that directory.

3) Add `build.rs` that emits code into `src/generated/<schema_name>` and
writes `src/generated/mod.rs`:

```rust
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use sbe_gen::{generate_to, GeneratorOptions};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let schemas_dir = manifest_dir.join("schemas");
    let generated_root = manifest_dir.join("src/generated");

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", schemas_dir.display());

    fs::create_dir_all(&generated_root)?;

    let mut schema_dirs: Vec<PathBuf> = fs::read_dir(&schemas_dir)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    schema_dirs.sort();

    let opts = GeneratorOptions::default();
    let mut modules = Vec::new();
    for schema_dir in schema_dirs {
        let schema_name = schema_dir
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or("invalid schema directory name")?
            .to_string();

        let schema_xml = pick_schema_xml(&schema_dir)?;
        println!("cargo:rerun-if-changed={}", schema_xml.display());
        let schema_contents = fs::read_to_string(&schema_xml)?;

        let output_dir = generated_root.join(&schema_name);
        fs::create_dir_all(&output_dir)?;
        generate_to(&schema_contents, &output_dir, &opts)?;

        modules.push(schema_name);
    }

    write_generated_mod(&generated_root, &modules)?;
    Ok(())
}

fn pick_schema_xml(dir: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let preferred = dir.join("templates_FixBinary.xml");
    if preferred.is_file() {
        return Ok(preferred);
    }

    let mut xml_files: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().map(|ext| ext == "xml").unwrap_or(false))
        .collect();
    xml_files.sort();

    match xml_files.len() {
        0 => Err(format!("no XML schema file found in {}", dir.display()).into()),
        1 => Ok(xml_files.remove(0)),
        _ => Err(format!(
            "multiple XML files found in {}, pick one via templates_FixBinary.xml",
            dir.display()
        )
        .into()),
    }
}

fn write_generated_mod(root: &Path, modules: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let mut buf = String::from("// @generated by build.rs; do not edit\n");
    for module in modules {
        buf.push_str(&format!("pub mod {};\n", module));
    }
    fs::write(root.join("mod.rs"), buf)?;
    Ok(())
}
```

4) Expose and use the generated modules:

```rust
pub mod generated;
```

```rust
use crate::generated::my_schema::packet_hdr::PacketHdr;
```

If you only have one schema, you can simplify the build script to read a
single XML file instead of scanning subdirectories.

### Groups and variable data

Groups are emitted as iterators layered on top of the raw buffer, and
variable‑length `data` fields come back as lightweight `VarData<'a>`
wrappers you can inspect as bytes or as UTF‑8 strings.

```xml
<message name="Book" id="1" blockLength="4">
  <field name="seq" id="1" type="uint32" />
  <group name="Levels" id="2" blockLength="16" dimensionType="groupSize">
    <field name="price" id="1" type="int64" />
    <field name="qty" id="2" type="int64" />
    <data name="note" id="3" type="varStringEncoding" />
  </group>
  <data name="raw" id="4" type="varStringEncoding" />
</message>
```

The generated module exposes clear, chainable helpers:

```rust
use sbe::book::*;

let (book, rest) = Book::parse_prefix(bytes).expect("prefix");

// Parse the Levels group
let levels = parse_levels(rest).expect("levels header");
for level in levels.iter() {
    let price = level.price().map(|v| v.get());
    let qty = level.qty().map(|v| v.get());
    let note = level.note.as_str();
}
let after_levels = levels.iter().remainder();

// Parse trailing variable data
let (raw, tail) = book.parse_raw(after_levels).expect("raw data");
let raw_str = raw.as_str();
```

### Fast path for fixed schema/version

Default decode should use `has_*` + accessor methods because that path is
safe across schema evolution (shorter `acting_block_length`, older
`acting_version`, constant fields, etc.).

If your producer schema/version is pinned and fixed, you can guard once and
then read `view.body` directly to avoid per-field presence checks:

```rust
let (hdr, body) = MessageHeader::parse_prefix(frame).expect("header");
let (view, rest) = sbe::book::parse_with_header(body, &hdr).expect("book");
assert!(rest.is_empty());

// Safe-by-default, schema-evolution path.
let seq = view.seq().map(|v| v.get());

// Optional fast path for fixed layout streams.
if view.is_fixed_layout() {
    let msg = &*view.body;
    let seq_fast = msg.seq.get();
    let _ = seq_fast;
}
```

Group entries follow the same pattern: check
`entry.acting_block_length >= size_of::<Entry>()` before using
`&*entry.body` directly.

### Encoding with builders

Every message module includes a builder that writes the fixed block,
groups and variable data with the correct padding, offsets and length
prefixes. Builders accept native Rust numeric types and take care of the
endianness for you.
Variable-length field setters return `Result` if the payload exceeds the
length prefix type.

```rust
use sbe::book::*;

let mut builder = BookBuilder::new();
builder.seq(123);
builder.levels(|levels| {
    levels.entry(|entry| {
        entry.price(101_500);
        entry.qty(10);
        entry.note(b"resting").expect("note");
    });
});
builder.raw(b"payload").expect("raw");

// Emit the message framed with the standard SBE header
let framed = builder.finish_with_header(); // Vec<u8>
// or if you only need the body:
// let body = builder.finish(); // Vec<u8>

// Zero-allocation path for hot loops:
let mut dst = [0u8; 256];
let written = Book::encode_body_into(&mut dst, |enc| {
    enc.seq(123);
    enc.raw(b"payload")?;
    Ok(())
})?;
```

## CME MDP3 pcap dump example

This repository includes a standalone example crate that generates CME
MDP3 decoders from `examples/cme_mdp3_pcap_dump/schemas/cme_mdp3/templates_FixBinary.xml`
and dumps Market-by-Order packets from a pcap file:

```shell
cargo run --manifest-path examples/cme_mdp3_pcap_dump/Cargo.toml -- \
  --pcap /path/to/file.pcap
```

The example supports filters (`--src-port`, `--dst-port`, `--udp-port`,
`--src`, `--dst`) and `--limit`. The `pcap` crate requires libpcap
headers to be installed on your system.

## Status and limitations

This project is a work‑in‑progress.  The generator covers the core SBE
types (primitives, enums, sets, composites, groups and variable data)
along with the standard message header and byte‑order rules.  Notable
spec features that are still missing:
* **Optional composites** are treated as required; optional handling is
  only emitted for primitives, enums and sets.
* **Group-entry versioning** currently uses block-length based presence
  checks for fixed fields in entry views; explicit entry-level
  `sinceVersion` gating is not emitted yet.
* **Value constraints** for builders/encoders are enforced via `assert!`
  checks and therefore panic on violation (rather than returning
  recoverable validation errors).
* **Constant fields** remain `const` definitions rather than struct
  members.

Contributions to extend the generator are welcome.  See the
`src/parser.rs` and `src/codegen.rs` modules for the implementation.
