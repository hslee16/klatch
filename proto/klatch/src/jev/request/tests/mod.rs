use super::*;
use crate::domain::LabelChain;
use serde_json::{Value, json};

const MODEL: &str = "jev-1.14.0";
const TEXT: &str = "hello, can we schedule the weekly sync for tuesday?";

pub(super) fn gate() -> Gate {
    Gate::new(LabelChain::default(), Some("internal"))
}

pub(super) fn limits() -> Limits {
    Limits::default()
}

pub(super) fn build(state: &Entry, qs: &[Question]) -> Result<Prepared, RequestError> {
    build_request(MODEL, state, qs, &gate(), &limits())
}

pub(super) fn body(p: &Prepared) -> Value {
    serde_json::from_str(p.body()).unwrap()
}

pub(super) fn text(s: &str) -> Entry {
    Entry::text("public", s)
}

/// A PNG header of the given size and a unique tail, so two of them differ.
pub(super) fn png(seed: u8) -> Image {
    let mut v = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    v.extend(13u32.to_be_bytes());
    v.extend(b"IHDR");
    v.extend(300u32.to_be_bytes());
    v.extend(300u32.to_be_bytes());
    v.extend([8, 2, 0, 0, 0, 0, 0, 0, 0]);
    v.extend(4u32.to_be_bytes());
    v.extend(b"IDAT");
    v.extend([seed; 4]);
    v.extend([0, 0, 0, 0]);
    Image::from_bytes(v).unwrap()
}

pub(super) fn noul() -> Question {
    Question::noul("q", "Is it about billing?")
}

pub(super) fn nest(depth: usize) -> Entry {
    let mut e = text("leaf");
    for _ in 0..depth {
        e = Entry::list(vec![e]);
    }
    e
}

mod body;
mod gate;
mod images;
mod questions;
mod rules;
