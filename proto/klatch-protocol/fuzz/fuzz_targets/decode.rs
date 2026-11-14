#![no_main]
//! Coverage-guided fuzzing of the wire decoder: `cargo +nightly fuzz run decode` from `klatch-protocol/`.
use klatch_protocol::{FrameReader, decode_client_line, decode_server_line};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let _ = decode_client_line(&text);
    let _ = decode_server_line(&text);
    let mut r = FrameReader::new(std::io::Cursor::new(data));
    while r.next_client().is_some() {}
});
