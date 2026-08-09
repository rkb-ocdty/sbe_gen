# Changelog

## Unreleased

### Changed
- Code generation moved into a `#[sbe_gen]` attribute macro (`sbe_gen_derive`).
  The generator now writes annotated structs and the macro expands them into
  builders, encoders, views, constants and layout assertions. Generated output
  is roughly a fifth of its previous size.
- Runtime support moved to the `sbe_support` crate: `Group`, `GroupIter`,
  `EntryBody`, `GroupBuilder`, `GroupEncoder`, `Dimension`, `MessageEncode`,
  `ParsePrefix`. It was previously emitted identically into every module.
- Schema types are referenced as `crate::types::X`. Consumers must expose
  `types` at the crate root.
- `#[repr(C)]` and the zerocopy derives come from the macro rather than
  appearing in the generated source.
- `encode_body_into` / `encode_with_header_into` moved to the `MessageEncode`
  trait.
- Group entry views no longer carry accessors for constant fields; the value is
  on the entry struct as an associated constant.

### Added
- `<Message>Message<B, G..>` holding a block and its groups, with `<Message>Ref`
  (borrowed) and `<Message>Owned` aliases. `parse_message` takes the header and
  body together and rejects a block shorter than this build's layout, so every
  field is present without a per-field check.
- Groups declared by more than one message are emitted once into `groups.rs`.
- `Derivation` and the staged schema types are public, so extension steps can be
  written outside the crate and passed to `generate_with` / `generate_to_with`.
- `DeriveSerialize`: serde derives and field attributes, with per-type and
  per-semantic-type mapping.
- `GeneratorOptions::type_map` substitutes a byte-compatible Rust type for a
  schema type.

### Fixed
- Group slices returned wrong data. Entry structs were sized from their fields
  rather than the group's declared `blockLength`, so a `&[Entry]` strided by
  `size_of` while the wire strides by `blockLength`. Every entry after the first
  was read at the wrong offset and decoded to plausible but incorrect values;
  CME's `NoOrderIDEntries` is 16 bytes on the wire against 12 bytes of fields, so
  order ids past the first were garbage. Iteration was unaffected, which is why
  it went unnoticed. Entries now pad to the declared block.

## 0.7.3

### Changed
- Generated `char`-encoded enum variants, associated constants, and `as_enum`
  match arms now use Rust byte-literal syntax (e.g. `Bid = b'0'`) for printable
  ASCII values instead of the numeric `Nu8` form (e.g. `Bid = 48u8`). Generated
  code is more readable and stays aligned with the schema. Non-printable
  single-character values still fall back to the numeric form.

## 0.7.2

### Fixed
- Generated message and group modules now reference schema-defined helper types
  via `super::types::...` and `super::message_header::...` instead of
  `crate::types::...` / `crate::message_header::...`. The previous absolute
  paths only resolved when the consumer crate happened to expose `types` and
  `message_header` at the crate root, which broke multi-schema layouts where
  each schema sits in its own submodule (e.g.
  `crate::generated::<schema>::types`). Relative `super::` paths resolve
  correctly in both single-schema and multi-schema crates without changing the
  collision-avoidance properties introduced in 0.7.0.
- Closures over message accessors (e.g. `field().and_then(|v| v.mantissa_opt())`)
  no longer hit `E0282` type-inference errors caused by unresolvable
  `crate::types::T` paths in multi-schema consumers.

## 0.7.1

### Changed
- Fixed composite offset handling in repeating-group dimension types, including `groupSize8Byte` layouts with explicit padding offsets.

## 0.7.0

### Changed
- Generator validation is now fail-fast for unsupported fixed field types instead of silently skipping them.
- Repeating-group and var-data schema validation is stricter:
  - invalid `dimensionType` declarations now fail generation instead of producing broken Rust,
  - unsupported var-data length encodings now fail generation up front.
- Generated Rust identifiers are now sanitized consistently across modules, types, fields, constants, groups, and var-data helpers.
- Post-sanitization identifier collisions now fail generation with explicit schema errors instead of silently overwriting files or emitting duplicate items.
- `valueRef` constant fields now resolve against the generated enum/set associated constants correctly.
- Generated message and group modules now reference schema-defined types through `crate::types::...`, avoiding collisions with generated helper types such as `MessageHeader`.
- Group builders and encoders now report repeating-group count overflow as structured errors instead of panicking.

### Documentation
- Refreshed parser documentation to match the current AST and parsing behavior.

## 0.6.1

### Added
- Message `*View` APIs now generate comprehensive schema-evolution helpers:
  - full field coverage for fixed-block fields (including previously skipped enum/set fields such as `security_update_action`),
  - value-level accessors (`field_value()`),
  - required-field helpers (`field_required() -> Result<_, DecodeFieldError>`),
  - enum convenience accessors (`field_enum()`),
  - nullable-composite passthrough helpers (e.g. `field_mantissa_opt()`),
  - fixed-byte-string helpers (`field_bytes()`, `field_str()`, `field_str_trimmed()`),
  - `view.is_fixed_layout()` for one-shot fixed-layout fast-path checks.
- Generated message modules now include `DecodeFieldError` for required/nullable fallback decode flows.

### Changed
- Fixed type-size/layout resolution for enum/set encoding aliases, so view/accessor generation no longer drops affected fields.
- Added `#[inline]` on newly generated tiny helper methods and enum `as_enum()` methods.

### Documentation
- Updated top-level docs and CME example docs to use `view.is_fixed_layout()` in fixed-layout fast-path guidance.
- Updated CME example decode flow to prefer fixed-layout direct-body reads, with simplified view-helper fallback for schema evolution.

## 0.6.0

### Added
- Generated constant fields now expose both associated constants and `#[inline]` constant accessors on messages and group entries.
- Added field-level constant literal support (`<field presence="constant">...</field>`) in the parser and generator.
- Added group-entry view accessors (`has_*` / getter methods) that honor runtime entry `blockLength` bounds.
- Added regression coverage for:
  - constant-field wire-layout drift cases (including shifted offsets and group block-length mismatches),
  - dimension composites using `<ref>` members for `blockLength`/`numInGroup`,
  - CME MDP3 example cases with constants and short group `blockLength`.

### Changed
- Constant fields are now fully treated as non-encoded wire data:
  - excluded from `#[repr(C)]` wire struct layout,
  - excluded from builder/encoder writable setters,
  - excluded from encoded-size assumptions.
- Message view accessors now generate `#[inline]` `has_*` and getter methods, including constant-field version-aware access in `parse_with_header` views.
- Group iterators now parse entries with a block-length-safe borrowed/owned path instead of relying on direct `parse_prefix` over raw entry slices.
- Short-block decode fallbacks in `parse_with_header` and group entry parsing no longer allocate/copy padding buffers; generated views now keep borrowed raw slices and rely on accessor methods for schema-evolution-safe field reads.
- Generated message/group bodies now cache parsed references when the acting block is large enough, and field getters use that cached fast path before falling back to offset-based raw parsing.
- Generated fixed-field encoder setters now use an in-bounds write helper (after constructor-time capacity checks) instead of per-call checked writes with `.expect(...)`.
- Generated min/max/null validation checks in setters now emit direct typed literals/conditions instead of parsing numeric strings on each call.
- Dimension-type field detection for repeating groups now supports both `<type>` and `<ref>` members and uses stronger name normalization/fallbacks.

### Documentation
- Updated `README.md`, `docs/USAGE.md`, and `examples/cme_mdp3_pcap_dump/README.md` to match current constant-field and group-entry semantics.
- Added explicit guarded fast-path guidance (`acting_block_length` / `acting_version` checks before direct `view.body`/`entry.body` reads) and aligned the CME example dump code with that pattern.

## 0.5.0

### Added
- Generated message modules now include a zero-allocation borrowed encoder (`<Message>Encoder<'a>`) that writes fixed fields and var-data directly into caller-provided `&mut [u8]`.
- Borrowed encoding now includes generated group entry encoders (`<Group>GroupEncoder<'a>` / `<Group>EntryEncoder<'a>`) so nested groups can be encoded without heap allocation.
- Added `encode_body_into` and `encode_with_header_into` helpers on each generated message for allocation-free body and framed encoding.
- Added `EncodeIntoError` with explicit buffer-capacity and var-data length-overflow reporting for borrowed encoding paths.

### Changed
- Owned builders remain the convenience path and keep existing `Vec<u8>`-returning `finish()` / `finish_with_header()` behavior.
- Constant fields (`presence="constant"`) continue to be excluded from writable setters, including the new borrowed encoder API.

## 0.4.0

### Changed
- Builder generation now reserves room for the SBE header, keeps ownership of its `Vec<u8>`, and returns borrowed `&[u8]` slices from `finish()`/`finish_with_header()`, so low-latency callers can reuse the builder via the new `clear()`/`reserve()` helpers.
- Constant fields (types with `presence="constant"`) are no longer exposed as writable setters, matching the wire layout and preventing downstream writes from clobbering those slots.
- Documentation/examples note the borrowed finish API, and downstream tests/examples now take the borrowed slice (calling `.to_vec()` only when a mutable owned copy is required).
