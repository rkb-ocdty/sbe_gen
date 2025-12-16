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
* **Byte‑order aware fields:** Multi‑byte integer and floating‑point
  fields use the `zerocopy::byteorder` types (e.g.
  `little_endian::U32`, `little_endian::F64`) so that endianness is
  explicit and efficient.
* **Declarative parsing helpers:** Each generated message implements a
  `parse_prefix` helper via a `zc_parse_prefix!` macro which
  leverages `zerocopy::Ref` to split a slice into a typed prefix and a
  remainder.
* **Schema reflection:** Basic support for SBE primitives, enums,
  sets and composites.  Nested groups and variable‑length data are
  ignored by the generator at the moment.

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
<message name="CmePacketHdr" id="0" blockLength="12">
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
pub struct CmePacketHdr {
    pub seq: U32,
    pub sending_time: U64,
}

impl CmePacketHdr {
    zc_parse_prefix!();
}
```

## Status and limitations

This project is a work‑in‑progress.  It covers the core SBE types
(primitives, enums, sets and composites) but deliberately ignores
several advanced features:

* **Groups and variable‑length data** are not emitted by the
  generator.  These sections of a message appear as gaps in the
  emitted struct; you can still access the remainder of the buffer
  using the `parse_prefix` helper.
* **Constant fields** are not materialised as struct members.  They
  are represented by `const` definitions on the generated type.
* **Extra metadata** such as field IDs and descriptions are not
  surfaced at runtime.

Contributions to extend the generator are welcome.  See the
`src/parser.rs` and `src/codegen.rs` modules for the implementation.