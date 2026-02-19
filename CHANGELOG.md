# Changelog

## Unreleased

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
- Dimension-type field detection for repeating groups now supports both `<type>` and `<ref>` members and uses stronger name normalization/fallbacks.

### Documentation
- Updated `README.md`, `docs/USAGE.md`, and `examples/cme_mdp3_pcap_dump/README.md` to match current constant-field and group-entry semantics.

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
