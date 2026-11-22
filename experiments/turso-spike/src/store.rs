//! The names the klatch conformance suite expects at `crate::store`, re-exported from klatch, with the suite itself
//! included by path from the klatch source (never copied).
pub use klatch::store::*;

#[path = "../../../proto/klatch/src/store/conformance/mod.rs"]
pub mod conformance;
