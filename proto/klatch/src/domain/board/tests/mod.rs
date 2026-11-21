use super::members::*;
use super::offers::*;
use super::*;
use crate::domain::{Attribution, MemberInfo, MemberPlace, Message, OpenAsk, Parked};

mod notes;
mod offers;
mod render;
mod who;

/// A message with this seq, (sender, body) and timestamp.
fn msg(seq: i64, said: (&str, &str), ts: i64) -> Message {
    let (sender, body) = said;
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
    }
}

fn peer(name: &str, last_seen: i64, status: Option<&str>) -> MemberInfo {
    MemberInfo {
        status_ts: 0,
        name: name.into(),
        joined: 0,
        last_seen,
        status: status.map(Into::into),
        parent: None,
        parked: None,
        kind: None,
        place: None,
        mode: None,
        advisory: false,
    }
}

/// An unclaimed ask with this seq, (sender, text) and timestamp.
fn ask(seq: i64, said: (&str, &str), ts: i64) -> OpenAsk {
    let (sender, text) = said;
    OpenAsk {
        seq,
        sender: sender.into(),
        text: text.into(),
        ts,
        ..Default::default()
    }
}

fn claimed(ask: OpenAsk) -> OpenAsk {
    OpenAsk {
        claimed: true,
        ..ask
    }
}

fn sample() -> Board {
    Board {
        room: "lobby".into(),
        asks: vec![
            ask(1, ("api-1", "seeking: slugify"), 90),
            claimed(ask(2, ("docs-1", "seeking: the api"), 80)),
        ],
        offers: vec![Offer {
            sender: "lib-1".into(),
            seq: 3,
            text: "offering: text helpers".into(),
            ts: 50,
        }],
        members: vec![
            BoardMember {
                status_ts: 0,
                name: "lib-1".into(),
                last_seen: 98,
                status: None,
                parent: None,
                parked: None,
                kind: None,
                place: None,
                advisory: false,
            },
            BoardMember {
                status_ts: 0,
                name: "api-1".into(),
                last_seen: 40,
                status: Some("done leaving".into()),
                parent: None,
                parked: None,
                kind: None,
                place: None,
                advisory: false,
            },
        ],
        stale_members: 0,
        tasks: vec![],
        prs: vec![],
        queue: vec![],
        merge_holder: None,
    }
}

fn aged(name: &str, status: &str, set_at: i64) -> MemberInfo {
    MemberInfo {
        status_ts: set_at,
        ..peer(name, 990, Some(status))
    }
}

fn with_mode(name: &str, mode: Option<&str>) -> MemberInfo {
    MemberInfo {
        mode: mode.map(Into::into),
        ..peer(name, 1000, None)
    }
}
