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
* **Declarative parsing helpers:** Each generated message implements a
  `parse_prefix` helper via a `zc_parse_prefix!` macro which
  leverages `zerocopy::Ref` to split a slice into a typed prefix and a
  remainder.
* **Groups and variable data included:** Nested repeating groups are
  emitted with iterable views and entry structs, and `data` fields
  become `VarData` slices with an ergonomic `as_str()` helper.
* **Optional fields:** `presence="optional"` fields stay zero‑copy but
  gain `<field>_opt()` accessors that return `Option` based on the SBE
  null value for that primitive.
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

This will create a module in `src/sbe` containing one Rust file per
message defined in the schema.  Each file starts with common
imports and the `zc_parse_prefix!` macro.  For example, given a
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

#[macro_export]
macro_rules! zc_parse_prefix {
    () => {
        #[inline]
        pub fn parse_prefix(body: &[u8]) -> Option<(&Self, &[u8])> {
            Ref::<_, Self>::from_prefix(body)
                .ok()
                .map(|(r, b)| (Ref::into_ref(r), b))
        }
    };
}

#[repr(C)]
#[derive(Debug, FromBytes, IntoBytes, KnownLayout, Immutable, Unaligned, Clone, Copy)]
pub struct PacketHdr {
    pub seq: U32,
    pub sending_time: U64,
}

impl PacketHdr {
    zc_parse_prefix!();
}
```

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
    let price = level.body.price;
    let qty = level.body.qty;
    let note = level.note.as_str();
}
let after_levels = levels.iter().remainder();

// Parse trailing variable data
let (raw, tail) = book.parse_raw(after_levels).expect("raw data");
let raw_str = raw.as_str();
```

### Encoding with builders

Every message module includes a builder that writes the fixed block,
groups and variable data with the correct padding, offsets and length
prefixes. Builders accept native Rust numeric types and take care of the
endianness for you.

```rust
use sbe::book::*;

let mut builder = BookBuilder::new();
builder.seq(123);
builder.levels(|levels| {
    levels.entry(|entry| {
        entry.price(101_500);
        entry.qty(10);
        entry.note(b"resting");
    });
});
builder.raw(b"payload");

// Emit the message framed with the standard SBE header
let framed = builder.finish_with_header();
// or if you only need the body:
// let body = builder.finish();
```

## Status and limitations

This project is a work‑in‑progress.  The generator covers the core SBE
types (primitives, enums, sets, composites, groups and variable data)
along with the standard message header and byte‑order rules.  Notable
spec features that are still missing:
* **Optional composites** are treated as required; optional handling is
  only emitted for primitives, enums and sets.
* **Acting version awareness** is not implemented; parsing assumes the
  current schema version and declared block lengths.
* **Value constraints** (min/max/null/initial) are emitted as constants
  but are not enforced at runtime.
* **Constant fields** remain `const` definitions rather than struct
  members.

Contributions to extend the generator are welcome.  See the
`src/parser.rs` and `src/codegen.rs` modules for the implementation.
