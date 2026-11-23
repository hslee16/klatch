use super::*;

/// Asserts that the output holds the text; the failure prints the whole output.
macro_rules! shows {
    ($out:expr, $needle:expr $(,)?) => {
        assert!($out.contains($needle), "{}", $out)
    };
}

/// Asserts that the output does not hold the text; the failure prints the whole output.
macro_rules! hides {
    ($out:expr, $needle:expr $(,)?) => {
        assert!(!$out.contains($needle), "{}", $out)
    };
}

/// Asserts that the output shows messages `newest` down to `oldest` from bob and none older than `oldest`.
macro_rules! shows_newest_down_to {
    ($out:expr, $newest:expr, $oldest:expr) => {
        shows!($out, &format!("#{} from bob", $newest));
        shows!($out, &format!("#{} from bob", $oldest));
        hides!($out, &format!("#{} from bob", $oldest - 1));
    };
}

/// A message from bob; a test that needs another sender sets it with `Message { sender: ..., ..msg(..) }`.
fn msg(seq: i64, body: &str) -> Message {
    Message {
        id: format!("ID{seq}"),
        seq,
        sender: "bob".into(),
        attribution: crate::domain::Attribution::Unattributed,
        label: "public".into(),
        body: body.into(),
        ts: 0,
        kind: None,
        payload: None,
        advisory: false,
        scope: None,
        refs: Vec::new(),
    }
}

/// An open ask of the member; a test that needs another asker sets it with `OpenAsk { sender: ..., ..ask(..) }`.
fn ask(seq: i64, text: &str, ts: i64) -> OpenAsk {
    OpenAsk {
        seq,
        sender: "me".into(),
        text: text.into(),
        ts,
        claimed: false,
        ..Default::default()
    }
}

/// What the hook prints is the JSON form; that is what Claude Code measures against its cap (#549).
fn printed_bytes(out: &str) -> usize {
    hook_context_json("UserPromptSubmit", out).len()
}

fn thirty() -> Vec<Message> {
    (1..=30).map(|i| msg(i, &format!("m{i}"))).collect()
}

fn many(n: i64, len: usize) -> Vec<Message> {
    (1..=n).map(|i| msg(i, &"y".repeat(len))).collect()
}

mod budget;
mod digest_lines;
mod fences;
mod levels;
mod rooms;
