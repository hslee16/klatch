use super::*;
use crate::domain::Attribution;

fn msg(seq: i64, sender: &str, body: &str, ts: i64) -> Message {
    Message {
        id: format!("id{seq}"),
        seq,
        sender: sender.into(),
        attribution: Attribution::Unattributed,
        label: "public".into(),
        body: body.into(),
        ts,
        kind: None,
        payload: None,
        advisory: false,
        scope: None,
        refs: Vec::new(),
    }
}

fn claim(path: &str) -> Claim {
    Claim {
        path: path.into(),
        owner: "helper".into(),
        reason: String::new(),
        expires: 1000,
        scope: String::new(),
    }
}

mod deadlines;
mod moves;
mod open;
mod outcomes;
mod words;
