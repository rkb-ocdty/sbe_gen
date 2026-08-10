# SBE Generator Usage Guide

This guide shows how to run the generator, what it produces, and how to use
the generated code. It includes fixed-size and variable-size examples with
XML, generated Rust excerpts, and usage snippets.

## Running the generator

### CLI

```sh
cargo run --bin sbe_gen -- -i schemas/my_schema.xml -o src/sbe
# Use big endian defaults if your schema uses byteOrder="big":
cargo run --bin sbe_gen -- -i schemas/my_schema.xml -o src/sbe -e big
```

### Library API

```rust
use std::fs;
use sbe_gen::{generate_to, GeneratorOptions};

let xml = fs::read_to_string("schemas/my_schema.xml")?;
generate_to(&xml, "src/sbe", &GeneratorOptions::default())?;
```

### Build-time (build.rs)

If you want generated modules to live inside your crate, run the generator
from build.rs and write into your source tree.

```rust
use std::env;
use std::fs;
use std::path::PathBuf;

use sbe_gen::{generate_to, GeneratorOptions};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let schema = manifest_dir.join("schemas/templates_FixBinary.xml");
    let out_dir = manifest_dir.join("src/sbe");

    println!("cargo:rerun-if-changed={}", schema.display());
    let xml = fs::read_to_string(schema)?;
    generate_to(&xml, &out_dir, &GeneratorOptions::default())?;
    Ok(())
}
```

Add the generator as a build dependency:

```toml
[build-dependencies]
sbe_gen = "0.7.1"
```

## Generated output layout

The generator always writes:

```
src/sbe/
  mod.rs
  types.rs
  message_header.rs
  <message>.rs
```

`mod.rs` re-exports message types, builders, and the standard SBE message
header. In your crate:

```rust
pub mod sbe;

use crate::sbe::{MessageHeader, Heartbeat, HeartbeatBuilder};
```

Note: helper functions such as `parse_with_header` and `parse_<group>`
are defined in each message module (for example,
`crate::sbe::heartbeat::parse_with_header`).

Generated code names `::sbe_support` and `::zerocopy` absolutely, so both are
dependencies of your crate:

```toml
[dependencies]
zerocopy = { version = "0.8", features = ["derive"] }
sbe_support = "0.7"
```

`mod.rs` re-exports `sbe_support`, so the runtime traits come from one import:

```rust
use crate::sbe::sbe_support::{Dimension, MessageEncode};
```

Schema types are referenced as `crate::types::X`, so expose them at the root:

```rust
pub mod types {
    pub use crate::sbe::types::*;
}
```

## What a generated message looks like

The generator writes the block as an annotated struct. `#[sbe_gen]` expands it
into the builder, encoder, view, constants and layout assertions.

```rust
#[sbe_gen(
    message,
    size = 8usize,
    block_length = 8usize,
    template_id = 42u32,
    group(order = 2, name = bids, ty = Bids, dimension = crate::types::groupSize, stride = 12usize),
    data(order = 3, name = comment, len = U16),
)]
pub struct OrderBook {
    #[sbe_gen(offset = 0u32, value(ty = "u32", read = "get"), required)]
    pub seq: crate::types::uInt32,
    #[sbe_gen(offset = 4u32, value(ty = "u32", read = "get"), required)]
    pub source: crate::types::uInt32,
}
```

Everything the macro needs is on the attribute: where a field sits, how its
value reads, whether it may be absent, and where each group and var-data sits
among the block's members. `cargo expand` shows the result.

Each message module gets:

| item | what it is |
| --- | --- |
| `OrderBook` | the packed block, read straight off the wire |
| `OrderBookBuilder` | appends into a `Vec<u8>` |
| `OrderBookEncoder` | writes into a caller's buffer |
| `OrderBookView` | a parsed block plus the bytes after it |
| `OrderBookMessage<B, G..>` | the block and its groups together |
| `OrderBookRef<'a>` / `OrderBookOwned` | borrowed and owned aliases of the above |
| `parse_with_header`, `parse_<group>` | free functions |

`Ref` is the one to reach for. It takes header and body together, so a message
that parses is a whole one:

```rust
let msg = OrderBookRef::parse_message(bytes)?;
msg.seq.get();        // derefs to the block
msg.bids;             // &[BidsEntry], walked once at parse
```

`parse_message` rejects a block shorter than this build's layout, so there is no
per-field presence check. A group whose entries carry their own groups or
var-data has no fixed stride and is reached through `parse_<group>` and its
iterator instead.

## Extension steps

`Derivation` runs over the laid-out schema and says what it wants added. Steps
are passed to the generator, so an extension needs no change to the generator
itself.

```rust
use sbe_gen::{Derivation, PlacedField, generate_to_with};
use syn::{Field as SynField, parse_quote};

struct Docs;

impl Derivation for Docs {
    fn field(&self, placed: &PlacedField, field: &mut SynField) -> Result<(), CodegenError> {
        let note = format!("FIX tag {}", placed.field.id);
        field.attrs.push(parse_quote!(#[doc = #note]));
        Ok(())
    }
}

generate_to_with(&xml, "src/sbe", &opts, &[&Docs])?;
```

The hooks:

| hook | reaches |
| --- | --- |
| `field` | a field the schema declared |
| `padding` | the members it did not |
| `message_struct` | the block |
| `group_struct` | a group entry |
| `message` | items appended to the module |
| `type_items` | enums, sets, composites |
| `types` | items appended to `types.rs` |
| `modules` | whole modules of its own |

### serde

`DeriveSerialize` is the worked example. It adds the derives and the field
attributes that make the wire types readable, and takes per-type and
per-semantic-type overrides:

```rust
let serde = DeriveSerialize::default()
    .map_semantic("UTCTimestamp", "crate::json_types::utc_timestamp");

generate_to_with(&xml, "src/sbe", &opts, &[&serde])?;
```

```json
{"TransactTime": "2023-07-16T22:00:00Z", "MDEntryPx": "30500", "NoMDEntries": [...]}
```

A type that is laid out identically to the schema's own can replace it
outright, which is a substitution rather than a serialisation:

```rust
GeneratorOptions {
    type_map: [("PRICE9".into(), "crate::json_types::Price9".into())].into(),
    ..Default::default()
}
```

## Fixed-size message example

### XML schema

```xml
<messageSchema package="example" schemaId="1" version="1">
  <message name="Heartbeat" id="1" blockLength="12">
    <field name="seq" id="1" type="uint32" />
    <field name="sending_time" id="2" type="uint64" />
  </message>
</messageSchema>
```

### Generated Rust (excerpt)

```rust
#[repr(C)]
#[derive(Debug, ::zerocopy::FromBytes, ::zerocopy::IntoBytes, ::zerocopy::KnownLayout, ::zerocopy::Immutable, ::zerocopy::Unaligned, Clone, Copy)]
pub struct Heartbeat {
    pub seq: ::zerocopy::byteorder::little_endian::U32,
    pub sending_time: ::zerocopy::byteorder::little_endian::U64,
}

impl Heartbeat {
    #[inline]
    pub fn parse_prefix(body: &[u8]) -> Option<(&Self, &[u8])> { /* ... */ }
    pub const BLOCK_LENGTH: u16 = 12;
    pub const TEMPLATE_ID: u16 = 1;
    pub const SCHEMA_ID: u16 = 1;
    pub const SCHEMA_VERSION: u16 = 1;
}

pub struct HeartbeatBuilder {
    buf: Vec<u8>,
}

impl HeartbeatBuilder {
    pub fn new() -> Self { /* ... */ }
    pub fn seq(&mut self, value: u32) -> &mut Self { /* ... */ }
    pub fn sending_time(&mut self, value: u64) -> &mut Self { /* ... */ }
    pub fn finish(self) -> Vec<u8> { /* ... */ }
    pub fn finish_with_header(self) -> Vec<u8> { /* ... */ }
}
```

### Using the generated code

Decode a message body:

```rust
let (heartbeat, rest) = Heartbeat::parse_prefix(body).expect("heartbeat body");
assert!(rest.is_empty());
let seq = heartbeat.seq.get();
let ts = heartbeat.sending_time.get();
```

Decode a framed message (header + body):

```rust
let (hdr, body) = MessageHeader::parse_prefix(frame).expect("header");
let (view, rest) = crate::sbe::heartbeat::parse_with_header(body, &hdr).expect("heartbeat");
assert!(rest.is_empty());
let seq = view.seq().map(|v| v.get());
```

Encode a message:

```rust
let mut builder = HeartbeatBuilder::new();
builder.seq(42);
builder.sending_time(1_700_000_000);

let framed = builder.finish_with_header(); // Vec<u8>, header + body
// or if you only need the body:
let body = builder.finish(); // sbe_support::Vec<u8>, derefs to [u8]
```

The buffer is `sbe_support::Vec`, re-exported from `allocator_api2`: `Vec<u8, A>`
with `A` defaulting to `Global`. `new_in` takes the allocator instead, so a
message can be built in an arena and the bytes are the same either way:

```rust
let arena = bumpalo::Bump::new();
let mut builder = HeartbeatBuilder::new_in(&arena);
builder.seq(42);
let body = builder.finish(); // Vec<u8, &Bump>
```

Encode directly into caller-owned memory (no allocation in the hot path):

```rust
let mut dst = [0u8; 256];
let written = Heartbeat::encode_body_into(&mut dst, |enc| {
    enc.seq(42);
    enc.sending_time(1_700_000_000);
    Ok(())
})?;
let body = &dst[..written];
```

## Variable-size message example (groups + data)

### XML schema

```xml
<messageSchema package="example" schemaId="1" version="1">
  <types>
    <composite name="groupSize">
      <type name="blockLength" primitiveType="uint16" />
      <type name="numInGroup" primitiveType="uint16" />
    </composite>
    <type name="varStringEncoding" primitiveType="uint8" />
  </types>

  <message name="Book" id="2" blockLength="4">
    <field name="seq" id="1" type="uint32" />
    <group name="Levels" id="2" blockLength="16" dimensionType="groupSize">
      <field name="price" id="1" type="int64" />
      <field name="qty" id="2" type="int64" />
      <data name="note" id="3" type="varStringEncoding" />
    </group>
    <data name="raw" id="3" type="varStringEncoding" />
  </message>
</messageSchema>
```

### Generated Rust (excerpt)

```rust
#[derive(Debug, Clone, Copy)]
pub struct VarData<'a> {
    pub len: usize,
    pub bytes: &'a [u8],
}

impl<'a> VarData<'a> {
    pub fn as_str(&self) -> Option<&'a str> { /* ... */ }
}

#[repr(C)]
pub struct Book {
    pub seq: U32,
}

impl Book {
    pub fn parse_prefix(body: &[u8]) -> Option<(&Self, &[u8])> { /* ... */ }
    pub fn parse_raw<'a>(&self, buf: &'a [u8]) -> Option<(VarData<'a>, &'a [u8])> { /* ... */ }
}

pub fn parse_levels<'a>(buf: &'a [u8]) -> Option<LevelsGroup<'a>> { /* ... */ }

pub struct LevelsGroup<'a> { /* header, payload, block_length */ }
pub struct LevelsIter<'a> { /* iterator state */ }

#[repr(C)]
pub struct LevelsEntry {
    pub price: I64,
    pub qty: I64,
}

pub enum LevelsEntryBody<'a> {
    Borrowed(&'a LevelsEntry, &'a [u8]),
    Owned(Vec<u8>),
}

pub struct LevelsEntryView<'a> {
    pub body: LevelsEntryBody<'a>,
    pub acting_block_length: usize,
    pub note: VarData<'a>,
}

impl<'a> LevelsEntryView<'a> {
    pub fn has_price(&self) -> bool { /* ... */ }
    pub fn price(&self) -> Option<&I64> { /* ... */ }
}

pub struct BookBuilder { /* ... */ }
pub struct LevelsGroupBuilder<'a> { /* ... */ }
pub struct LevelsEntryBuilder<'a> { /* ... */ }
```

### Using the generated code

Decode a message with group entries and trailing variable data:

```rust
let (book, rest) = Book::parse_prefix(body).expect("book body");
let levels = crate::sbe::book::parse_levels(rest).expect("levels");

let mut iter = levels.iter();
while let Some(level) = iter.next() {
    let price = level.price().map(|p| p.get()); // blockLength-safe path
    let qty = level.qty().map(|q| q.get());
    let note = level.note.as_str();
}

let after_levels = iter.remainder();
let (raw, tail) = book.parse_raw(after_levels).expect("raw data");
assert!(tail.is_empty());
```

Encode the same message with the builder API. Variable-length setters
return `Result` when the payload does not fit the length prefix type:

```rust
let mut builder = BookBuilder::new();
builder.seq(7);
builder.levels(|levels| {
    levels.entry(|entry| {
        entry.price(101_500);
        entry.qty(10);
        entry.note(b"bid").expect("note");
    });
    levels.entry(|entry| {
        entry.price(101_600);
        entry.qty(12);
        entry.note(b"ask").expect("note");
    });
});
builder.raw(b"payload").expect("raw");

let framed = builder.finish_with_header(); // Vec<u8>, header + body
let body = builder.finish(); // sbe_support::Vec<u8>
```

## Header-aware decoding (acting version)

If you parse the standard SBE header, you can use `parse_with_header` to
respect the acting block length and version at runtime.

```rust
let (hdr, body) = MessageHeader::parse_prefix(frame).expect("header");
let (view, rest) = crate::sbe::book::parse_with_header(body, &hdr).expect("book");
if view.has_seq() {
    let seq = view.seq().map(|v| v.get());
}
```

## Fast path (fixed schema/version)

Use accessors by default. They are the schema-evolution-safe path and handle
runtime `acting_block_length` / `acting_version` correctly.

If your input stream is pinned to one schema version and fixed block length,
you can guard once and then read `view.body` directly:

```rust
let (hdr, body) = MessageHeader::parse_prefix(frame).expect("header");
let (view, rest) = crate::sbe::book::parse_with_header(body, &hdr).expect("book");
assert!(rest.is_empty());

if view.is_fixed_layout() {
    let msg = &*view.body;
    let seq_fast = msg.seq.get();
    let _ = seq_fast;
}
```

For repeating groups, use the same guard per entry before direct deref:

```rust
for entry in entries.iter() {
    if entry.acting_block_length >= core::mem::size_of::<NoMDEntriesEntry>() {
        let body = &*entry.body;
        // direct fixed-field reads here
    } else {
        let price = entry.price().map(|v| v.get());
        let _ = price;
    }
}
```

## Constant fields

SBE constant fields are not encoded on the wire. Generated code reflects
that by:

- excluding constant fields from `#[repr(C)]` message/group-entry structs,
- exposing associated constants and constant accessors,
- skipping constant setters in builders/encoders.

Example shape:

```rust
impl SomeMessage {
    pub const SECURITY_ID_SOURCE: SecurityIDSource = [b'8'];
    #[inline]
    pub fn security_id_source(&self) -> SecurityIDSource { Self::SECURITY_ID_SOURCE }
}
```
