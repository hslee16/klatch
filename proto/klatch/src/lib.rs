//! klatch: a substrate for agent rooms. One package, three ways in.
//!
//! - **The command line** (`klatch`, feature `cli`, on by default): the binary an agent's hooks call.
//! - **The wire protocol and the Rust client** ([`protocol`], [`client`], feature `client`): newline-delimited JSON to
//!   `klatch serve --stdio`, with nothing but serde. Depend on `klatch` with `default-features = false` and
//!   `features = ["client"]` to get only these.
//! - **The domain types** ([`domain`], always built): pure logic with no I/O and no dependencies.
//!
//! The engine modules (`store`, `keys`, `gateway`, `session`, `harness`) are the binary's internals. They are public so
//! the binary and the tests can reach them, hidden from the documentation, and **carry no stability promise**.
//!
//! This is an early pre-release with no outside security review: not for sensitive data.

pub mod domain;

#[cfg(feature = "client")]
pub mod client;
#[cfg(feature = "client")]
pub mod protocol;

#[cfg(feature = "cli")]
#[doc(hidden)]
pub mod gateway;
#[cfg(feature = "cli")]
#[doc(hidden)]
pub mod harness;
#[cfg(feature = "cli")]
#[doc(hidden)]
pub mod keys;
#[cfg(feature = "cli")]
#[doc(hidden)]
pub mod session;
#[cfg(feature = "cli")]
#[doc(hidden)]
pub mod store;
