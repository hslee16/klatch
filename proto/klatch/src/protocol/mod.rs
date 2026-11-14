//! The protocol module (the `klatch-protocol` crate before the crates became modules): the public wire contract (DESIGN section 20 rules 10 to 12). Language neutral; this
//! crate is the Rust implementation and the reference for the conformance tests.

mod codec;
mod frames;

pub use codec::{
    DecodeError, FrameReader, MAX_LINE_BYTES, decode_client_line, decode_server_line, encode_line,
};
pub use frames::{Attribution, ClientFrame, ErrorBody, ServerFrame};

/// The major version of the protocol this crate speaks. Changes within a major version are additive.
pub const PROTOCOL_VERSION: u32 = 1;
