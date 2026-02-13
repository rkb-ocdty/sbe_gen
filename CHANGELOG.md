# Changelog

## 0.4.0

### Changed
- Builder generation now reserves room for the SBE header, keeps ownership of its `Vec<u8>`, and returns borrowed `&[u8]` slices from `finish()`/`finish_with_header()`, so low-latency callers can reuse the builder via the new `clear()`/`reserve()` helpers.
- Constant fields (types with `presence="constant"`) are no longer exposed as writable setters, matching the wire layout and preventing downstream writes from clobbering those slots.
- Documentation/examples note the borrowed finish API, and downstream tests/examples now take the borrowed slice (calling `.to_vec()` only when a mutable owned copy is required).

## Unreleased
