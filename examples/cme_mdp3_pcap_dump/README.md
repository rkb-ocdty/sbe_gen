# CME MDP3 pcap dump example

This example crate generates CME MDP3 SBE decoders from
`schemas/cme_mdp3/templates_FixBinary.xml` and dumps selected message
templates from a pcap file.

## Requirements

- Rust toolchain
- libpcap headers (for the `pcap` crate)

## Build and run

From the repo root:

```shell
cargo run --manifest-path examples/cme_mdp3_pcap_dump/Cargo.toml -- \
  --pcap /path/to/file.pcap
```

Optional filters:

- `--src-port`, `--dst-port`, `--udp-port`
- `--src`, `--dst`
- `--limit`

## Notes

- Generated decoders are written to `examples/cme_mdp3_pcap_dump/src/generated` at build time.
- The example currently handles templates 30, 47, 48, 51, 53, and 59 (plus 4 and 12).
- The example test suite also covers constant-field correctness and
  repeating-group decoding when runtime `blockLength` is shorter than the
  compiled entry struct size.
- Decode paths in `src/dump.rs` use accessors by default. For fixed-layout
  hot paths, direct `view.body` / `entry.body` reads are guarded by runtime
  checks before deref:

```rust
if view.is_fixed_layout() {
    let msg = &*view.body;
}

if entry.acting_block_length >= core::mem::size_of::<EntryType>() {
    let body = &*entry.body;
}
```
