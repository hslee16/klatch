//! Fuzzing the wire decoder (#165): random and mutated input must never panic, hang or grow without bound. This runs
//! in every CI run; `fuzz/` holds the same target for a coverage-guided `cargo fuzz` run.

use klatch::protocol::{FrameReader, decode_client_line, decode_server_line};
use std::io::Cursor;

/// A small deterministic generator, so a failure can be reproduced from its seed.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

fn corpus() -> Vec<String> {
    let mut v = Vec::new();
    for f in [
        include_str!("../fixtures/protocol/client_basic.ndjson"),
        include_str!("../fixtures/protocol/server_basic.ndjson"),
        include_str!("../fixtures/protocol/forward_compat.ndjson"),
    ] {
        v.extend(f.lines().filter(|l| !l.trim().is_empty()).map(String::from));
    }
    assert!(v.len() > 5, "the fixtures are missing");
    v
}

fn mutate(rng: &mut Rng, line: &str) -> Vec<u8> {
    let mut b = line.as_bytes().to_vec();
    for _ in 0..1 + rng.below(4) {
        match rng.below(7) {
            0 if !b.is_empty() => {
                let i = rng.below(b.len());
                b[i] ^= 1 << rng.below(8);
            }
            1 if !b.is_empty() => {
                let i = rng.below(b.len());
                b.truncate(i);
            }
            2 => {
                let i = rng.below(b.len() + 1);
                b.insert(i, (rng.next() & 0xFF) as u8);
            }
            3 if !b.is_empty() => {
                let i = rng.below(b.len());
                let j = (i + rng.below(32)).min(b.len());
                let dup: Vec<u8> = b[i..j].to_vec();
                b.splice(i..i, dup);
            }
            4 => b.extend(b"\"".repeat(rng.below(5))),
            5 => {
                let i = rng.below(b.len() + 1);
                b.splice(i..i, b"9999999999999999999999999999".iter().copied());
            }
            _ => {
                let i = rng.below(b.len() + 1);
                b.splice(i..i, [0u8, 0xFF, 0xFE, 0xC0, b'\r', 0x1B]);
            }
        }
    }
    b
}

#[test]
fn mutated_frames_never_panic_either_decoder() {
    let corpus = corpus();
    let mut rng = Rng(0x9E3779B97F4A7C15);
    let mut decoded = 0u32;
    for _ in 0..60_000 {
        let line = corpus[rng.below(corpus.len())].as_str();
        let bytes = mutate(&mut rng, line);
        let text = String::from_utf8_lossy(&bytes);
        decoded += u32::from(decode_server_line(&text).is_ok());
        decoded += u32::from(decode_client_line(&text).is_ok());
    }
    assert!(
        decoded > 0,
        "no mutation survived: the corpus is not exercising the decoder"
    );
}

#[test]
fn a_stream_of_garbage_ends_and_yields_no_more_frames_than_lines() {
    let corpus = corpus();
    let mut rng = Rng(0xDEADBEEFCAFEF00D);
    for round in 0..300 {
        let mut input = Vec::new();
        for _ in 0..rng.below(40) {
            let line = corpus[rng.below(corpus.len())].as_str();
            input.extend(mutate(&mut rng, line));
            input.push(b'\n');
        }
        if round % 7 == 0 {
            input.extend(std::iter::repeat_n(b'a', 2_000_000)); // an oversize line, then more input
            input.push(b'\n');
        }
        // a mutation can insert a newline, so count the lines the input really has
        let lines = input.iter().filter(|b| **b == b'\n').count() + 1;
        let mut r = FrameReader::new(Cursor::new(input));
        let mut seen = 0usize;
        while r.next_client().is_some() {
            seen += 1;
            assert!(
                seen <= lines + 1,
                "more frames than lines: it is not consuming input"
            );
        }
    }
}

#[test]
fn deeply_nested_and_huge_values_are_refused_without_exhausting_the_stack() {
    for depth in [10, 200, 5_000, 100_000] {
        let nested = format!("{}{}", "[".repeat(depth), "]".repeat(depth));
        let line =
            format!(r#"{{"v":1,"type":"request","id":"x","method":"post","params":{nested}}}"#);
        let _ = decode_client_line(&line);
        let _ = decode_server_line(&line);
    }
    let big = format!(
        r#"{{"v":1,"type":"request","id":"{}","method":"read"}}"#,
        "x".repeat(2_000_000)
    );
    let _ = decode_client_line(&big);
}

#[test]
fn every_prefix_of_every_fixture_line_decodes_or_errors_but_never_panics() {
    for line in corpus() {
        for end in 0..=line.len() {
            if line.is_char_boundary(end) {
                let _ = decode_client_line(&line[..end]);
                let _ = decode_server_line(&line[..end]);
            }
        }
    }
}
