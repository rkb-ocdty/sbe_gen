# Changelog

## Unreleased

### Changed
- Builder code now reserves room for the SBE header, keeps ownership of its buffer, and returns borrowed `&[u8]` slices from `finish()`/`finish_with_header()`, letting low-latency callers reuse the same builder instance via new `clear()`/`reserve()` helpers.
- Constant fields (types with `presence="constant"`) are no longer exposed as writable setters so generated builders respect the real wire layout and cannot clobber payload slots.
- Documentation and examples explain the borrowed finish API, and downstream tests/examples were updated to use `builder.finish()` consistently (cloning when a mutable owned copy is needed).
