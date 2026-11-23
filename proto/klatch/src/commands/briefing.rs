//! The late-joiner briefing at a session start (#1409): gathers the facts through the guarded reads the board and the
//! digest use (a room the member may not read refuses them, and then nothing about it is printed), and cuts the digest
//! to what is addressed to the member, because the briefing counts the rest. Nothing here moves a read cursor.

use super::*;
use klatch::domain::{
    BRIEFING_RECENT_MESSAGES, BriefingInput, Importance, Standing, StartSource, render_briefing,
};

/// The source Claude Code names in a `SessionStart` hook input. Any other word is ignored, never printed.
pub(super) fn start_source(input: Option<&serde_json::Value>) -> Option<StartSource> {
    input?
        .get("source")
        .and_then(|s| s.as_str())
        .and_then(StartSource::parse)
}

/// The briefing of one room, or `None` when the member may not read it or there is nothing to say. A failure to
/// gather is never a failure of the session start.
pub(super) fn room_briefing(
    ctx: &Ctx,
    room: &str,
    standing: Standing,
    source: Option<StartSource>,
    cap: usize,
) -> Option<String> {
    let t = ctx.t;
    let board = t.board(room).ok()?;
    let claims = t.claims(room).ok()?;
    let recent = t
        .history(
            room,
            klatch::gateway::HistoryRange {
                since: None,
                last: Some(BRIEFING_RECENT_MESSAGES),
            },
        )
        .ok()?
        .messages;
    let text = render_briefing(
        &BriefingInput {
            room,
            me: ctx.me,
            now: ctx.now,
            board: &board,
            claims: &claims,
            recent: &recent,
            standing,
            source,
        },
        cap,
    );
    (!text.is_empty()).then_some(text)
}

/// How the member stands to `room`: how long its history is for a new member or a visitor, how much is unread past
/// its read cursor for a returning one. Moves no cursor.
pub(super) fn standing_of(t: &dyn Transport, room: &str, kind: Kind) -> Standing {
    let head = || {
        t.history(
            room,
            klatch::gateway::HistoryRange {
                since: None,
                last: Some(1),
            },
        )
        .ok()
        .and_then(|h| h.messages.last().map(|m| m.seq))
        .unwrap_or(0)
    };
    match kind {
        Kind::New => Standing::New { messages: head() },
        Kind::Visitor => Standing::Visitor { messages: head() },
        Kind::Returning => t
            .read_ranked(room)
            .map(|r| Standing::returning(&r))
            .unwrap_or(Standing::Returning {
                unread: 0,
                for_it: 0,
            }),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    New,
    Returning,
    Visitor,
}

/// What a session start does to the digest so the briefing can replace the backlog: the messages that are not
/// addressed to the member are dropped from the digest (the briefing counts them; they stay unread, and the read
/// cursor is not touched), and the open asks and unowned PRs the briefing lists are not listed twice. Returns, per
/// room that is not briefed in full, `(room, unread, for the member)`.
pub(super) fn cut_digest(
    d: &mut klatch::gateway::Digest,
    briefed: &[String],
) -> Vec<(String, usize, usize)> {
    let mut others = Vec::new();
    for r in &mut d.rooms {
        let total = r.messages.len();
        let class_of = |seq: i64| {
            r.ranks
                .iter()
                .find(|(s, _)| *s == seq)
                .map_or(Importance::Rest, |(_, c)| *c)
        };
        let keep: Vec<bool> = r
            .messages
            .iter()
            .map(|m| r.addressed.contains(&m.seq) || class_of(m.seq) != Importance::Rest)
            .collect();
        let kept = keep.iter().filter(|k| **k).count();
        let mut i = 0;
        r.messages.retain(|_| {
            i += 1;
            keep[i - 1]
        });
        if briefed.contains(&r.room) {
            r.backlog.clear();
            r.unowned_prs.clear();
        } else if total > kept {
            others.push((r.room.clone(), total, kept));
        }
    }
    others
}

#[cfg(test)]
mod briefing_tests {
    use super::*;
    use klatch::domain::Principal;
    use klatch::gateway::areas::*;
    use klatch::gateway::{AllowAll, ClaimSpec, Clock, Direct, Gateway};
    use klatch::store::SqliteStore;
    use std::rc::Rc;

    struct Fixed;
    impl Clock for Fixed {
        fn now(&self) -> i64 {
            1000
        }
    }

    type Gw = Rc<Gateway<SqliteStore>>;

    fn me(g: &Gw, name: &str) -> Direct<SqliteStore> {
        Direct::new(g.clone(), Principal::unattributed(name))
    }

    /// A lobby with a public chat and a vault above the clearance of `ann`, who joined it before it was raised.
    fn vault_world() -> Gw {
        let g = Rc::new(Gateway::new(
            SqliteStore::open_in_memory().unwrap(),
            Fixed,
            AllowAll,
        ));
        let admin = Principal::unattributed("admin");
        me(&g, "ann").join("lobby").unwrap();
        me(&g, "ann").join("vault").unwrap();
        g.admin_set_room_label(&admin, "vault", "private").unwrap();
        g.admin_set_clearance(&admin, "bob", "private").unwrap();
        me(&g, "bob").join("vault").unwrap();
        let bob = me(&g, "bob");
        bob.post("vault", "seeking: review: SECRETWORD plan for the merger")
            .unwrap();
        bob.post("vault", "working: SECRETTASK nightly").unwrap();
        bob.post("vault", "handoff: SECRETTASK to carol").unwrap();
        bob.claim("vault", &ClaimSpec::new("secret/SECRETPATH.rs", 600, ""))
            .unwrap();
        me(&g, "bob").join("lobby").unwrap();
        me(&g, "bob")
            .post("lobby", "seeking: review: PUBLICWORD in the lobby")
            .unwrap();
        g
    }

    fn start_text(g: &Gw, who: &str, board: Option<&str>) -> String {
        let t = me(g, who);
        let ctx = Ctx {
            t: &t,
            me: who,
            now: 1000,
            stdin_session: None,
            discovered: false,
            stdin: None,
        };
        let args = HookArgs {
            event: Some("SessionStart".into()),
            board: board.map(String::from),
            no_join: false,
            checkpoint: false,
            machine_status: false,
        };
        session_start(&ctx, &args).unwrap().0
    }

    #[test]
    fn a_room_above_the_members_clearance_contributes_nothing_to_the_briefing() {
        let g = vault_world();
        for board in [None, Some("vault")] {
            let text = start_text(&g, "ann", board);
            for secret in ["SECRETWORD", "SECRETTASK", "SECRETPATH", "merger"] {
                assert!(
                    !text.contains(secret),
                    "{secret} leaked ({board:?}): {text}"
                );
            }
            assert!(
                !text.contains("[briefing vault"),
                "a briefing of a room it cannot read: {text}"
            );
            assert!(text.contains("PUBLICWORD"), "the lobby is briefed: {text}");
        }
    }

    #[test]
    fn a_member_cleared_for_the_room_is_briefed_on_it_so_the_test_above_is_not_vacuous() {
        let g = vault_world();
        let text = start_text(&g, "bob", Some("vault"));
        assert!(text.contains("[briefing vault"), "{text}");
        assert!(text.contains("SECRETWORD"), "{text}");
        assert!(text.contains("claims: 1 live (1 file)"), "{text}");
    }
}
