# Changelog

## 0.4.0

### Changed
- Builder generation now reserves room for the SBE header, keeps ownership of its `Vec<u8>`, and returns borrowed `&[u8]` slices from `finish()`/`finish_with_header()`, so low-latency callers can reuse the builder via the new `clear()`/`reserve()` helpers.
- Constant fields (types with `presence="constant"`) are no longer exposed as writable setters, matching the wire layout and preventing downstream writes from clobbering those slots.
- Documentation/examples note the borrowed finish API, and downstream tests/examples now take the borrowed slice (calling `.to_vec()` only when a mutable owned copy is required).

## Unreleased

### Added
- Generated message modules now include a zero-allocation borrowed encoder (`<Message>Encoder<'a>`) that writes fixed fields and var-data directly into caller-provided `&mut [u8]`.
- Borrowed encoding now includes generated group entry encoders (`<Group>GroupEncoder<'a>` / `<Group>EntryEncoder<'a>`) so nested groups can be encoded without heap allocation.
- Added `encode_body_into` and `encode_with_header_into` helpers on each generated message for allocation-free body and framed encoding.
- Added `EncodeIntoError` with explicit buffer-capacity and var-data length-overflow reporting for borrowed encoding paths.

### Changed
- Owned builders remain the convenience path and keep existing `Vec<u8>`-returning `finish()` / `finish_with_header()` behavior.
- Constant fields (`presence="constant"`) continue to be excluded from writable setters, including the new borrowed encoder API.
