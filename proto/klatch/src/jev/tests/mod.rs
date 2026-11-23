use super::fake::{FakeTransport, fixtures, status_with_headers};
use super::*;
use crate::domain::{HostedConfig, HostedJudge, Judge, LabelChain, ReviewItem, Verdict};
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

pub(super) const SECRET: &str = "sk-live-SECRET-1234";

pub(super) const MODEL: &str = "jev-1.14.0";
/// The only message text any fixture uses.
pub(super) const TEXT: &str = "hello, can we schedule the weekly sync for tuesday?";

pub(super) fn cfg() -> JevConfig {
    JevConfig::new(MODEL, 0.8).unwrap()
}

pub(super) fn gate() -> Gate {
    Gate::new(LabelChain::default(), Some("internal"))
}

pub(super) fn client(f: &FakeTransport) -> JevClient<&FakeTransport> {
    JevClient::new(f, cfg())
        .with_key_source(Box::new(|| Some(SECRET.to_string())))
        .with_gate(gate())
}

pub(super) fn envelope(answers: Value) -> Value {
    json!({"model": MODEL, "answers": answers, "usage": {"input_tokens": 120, "output_tokens": 4}})
}

pub(super) fn judge_reply(choice: &str, conf: f64) -> String {
    let mut p = json!({"allow": 0.0, "redact": 0.0, "deny": 0.0});
    p[choice] = json!(conf);
    let rest = (1.0 - conf) / 2.0;
    for k in ["allow", "redact", "deny"] {
        if k != choice {
            p[k] = json!(rest);
        }
    }
    envelope(json!({"verdict": {"type": "choice", "choice": choice, "confidence": conf, "probabilities": p}}))
        .to_string()
}

pub(super) fn item(label: &str, text: &str) -> ReviewItem {
    ReviewItem {
        id: 1,
        room: "r".into(),
        seq: 1,
        to_label: "public".into(),
        from_label: label.into(),
        reason: "crossing".into(),
        text: text.into(),
    }
}

pub(super) fn hosted(f: &FakeTransport) -> HostedJudge<JevJudge<&FakeTransport>> {
    HostedJudge::new(
        JevJudge::new(client(f)),
        LabelChain::default(),
        HostedConfig {
            enabled: true,
            max_label: Some("internal".into()),
        },
    )
}

pub(super) fn state() -> Entry {
    Entry::object(vec![(Key::Fixed("message"), Entry::text("public", TEXT))])
}

pub(super) fn ask_judge(f: &FakeTransport) -> Result<Answers, JevError> {
    client(f).ask(&state(), &[judge_question()])
}

pub(super) fn png() -> Image {
    let mut v = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    v.extend(13u32.to_be_bytes());
    v.extend(b"IHDR");
    v.extend(300u32.to_be_bytes());
    v.extend(300u32.to_be_bytes());
    v.extend([8, 2, 0, 0, 0, 0, 0, 0, 0]);
    v.extend(4u32.to_be_bytes());
    v.extend(b"IDAT");
    v.extend([7u8; 4]);
    v.extend([0, 0, 0, 0]);
    Image::from_bytes(v).unwrap()
}

mod config;
mod facilitator;
mod judge;
mod listing;
mod multipart;
mod retries;
