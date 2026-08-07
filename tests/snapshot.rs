//! Locks the generated output for every schema in `tests/schemas`.
//!
//! The other tests assert on fragments of the output; this one notices when *anything* moves.
//! It exists because refactors kept being "obviously" output-neutral and twice were not.
//!
//! When a change is meant to alter generated code, re-record with:
//!
//! ```text
//! UPDATE_SNAPSHOTS=1 cargo test --test snapshot
//! ```
//!
//! and read the resulting diff to `tests/snapshots.txt` as part of reviewing the change.

use std::fmt::Write as _;
use std::path::Path;

use sbe_gen::{GeneratorOptions, generate};

const SNAPSHOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/snapshots.txt");

fn record() -> String {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/schemas");
    let mut schemas: Vec<_> = std::fs::read_dir(&dir)
        .expect("tests/schemas")
        .map(|entry| entry.expect("dir entry").path())
        .filter(|path| path.extension().is_some_and(|e| e == "xml"))
        .collect();
    schemas.sort();

    let mut out = String::new();
    for path in schemas {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let xml = std::fs::read_to_string(&path).expect("read schema");
        match generate(&xml, &GeneratorOptions::default()) {
            Ok(generated) => {
                let mut modules: Vec<_> = generated
                    .modules()
                    .map(|m| (m.name.clone(), m.source.clone()))
                    .collect();
                modules.sort();
                for (file, code) in modules {
                    // length and a content hash: enough to catch any change, small enough that
                    // the snapshot stays reviewable
                    let hash = code
                        .bytes()
                        .fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64));
                    writeln!(out, "{name} {file} {} {hash:016x}", code.len()).unwrap();
                }
            }
            Err(err) => writeln!(out, "{name} ERR {err}").unwrap(),
        }
    }
    out
}

#[test]
fn generated_output_matches_snapshot() {
    let current = record();
    if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
        std::fs::write(SNAPSHOT, &current).expect("write snapshot");
        return;
    }
    let recorded = std::fs::read_to_string(SNAPSHOT).unwrap_or_default();
    if recorded == current {
        return;
    }
    let changed: Vec<_> = current
        .lines()
        .zip(recorded.lines())
        .filter(|(now, before)| now != before)
        .take(10)
        .collect();
    panic!(
        "generated output changed for {} of {} entries (first few below).\n\
         re-record with UPDATE_SNAPSHOTS=1 if this is intended.\n{}",
        current
            .lines()
            .zip(recorded.lines())
            .filter(|(a, b)| a != b)
            .count(),
        current.lines().count(),
        changed
            .iter()
            .map(|(now, before)| format!("  was {before}\n  now {now}\n"))
            .collect::<String>()
    );
}
