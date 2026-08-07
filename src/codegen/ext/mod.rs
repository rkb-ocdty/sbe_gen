//! Derivation steps: the things written on top of the wire format rather than by it.
//!
//! Each one is a `Derivation`, so it sees the laid-out schema and says what it wants added. The
//! set is a run-time list, which is what lets a consumer add its own without a pass knowing.
use super::*;

mod serde;

pub use serde::DeriveSerialize;
