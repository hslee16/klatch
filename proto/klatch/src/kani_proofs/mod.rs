//! Kani bounded proofs of the pure security functions (#404, DESIGN section 32 item 66). Compiled only under
//! `cargo kani` (`cfg(kani)`); not part of any build, test, CI or gate run. Run them with `proto/scripts/kani`.
//!
//! Every harness states its bound. A result is a bounded proof: "no counterexample up to N", never "proven". What a
//! harness leaves out is listed with it, and RESEARCH-NOTES section 52 collects them.

mod common;
mod labels;
mod larger_bounds;
mod names;
mod paths;

#[cfg(feature = "cli")]
mod key_proofs;
#[cfg(feature = "cli")]
mod session_proofs;
