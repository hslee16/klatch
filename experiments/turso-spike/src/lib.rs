//! Spike for #620: a Turso Database (the Rust rewrite of SQLite) backend behind klatch's `Store` trait. It learns, it
//! does not ship: nothing in the main workspace depends on this crate. The backend (`turso_store`) keeps klatch's
//! SQLite schema (only the tables it uses) and ports the required methods plus the message, wait-position, mute and
//! plain-claim areas. The conformance suite is included from the klatch source by path, so it is never a copy.

pub mod turso_store;
pub use turso_store::{Encryption, TursoStore};

/// The names `conformance` expects at `crate::domain`, re-exported from klatch.
#[cfg(test)]
mod domain {
    pub use klatch::domain::*;
}

#[cfg(test)]
mod store;

#[cfg(test)]
mod run;
