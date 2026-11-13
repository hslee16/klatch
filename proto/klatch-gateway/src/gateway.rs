use crate::{Clock, Guard, Operation};
use klatch_core::{
    Claim, Error, MemberInfo, Message, Posted, Principal, Result, RoomDigest, STALE_AFTER_SECS,
    valid_name,
};
use klatch_store::Store;

/// What a hook needs: unread messages and others' claims per room, and the rooms that were refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Digest {
    pub rooms: Vec<RoomDigest>,
    pub refused: Vec<Refused>,
    pub now: i64,
}

/// A room the guard would not let the caller read, with the reason. Shown, never silently dropped (R19).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused {
    pub room: String,
    pub reason: String,
}

/// The label every message is posted under until rooms have labels (DESIGN D3, the label tickets).
pub const DEFAULT_LABEL: &str = "public";

const MAX_PATH_BYTES: usize = 1024;
const MAX_REASON_BYTES: usize = 500;

#[cfg(test)]
pub(crate) trait AsSpy {
    fn spy_calls(&self) -> usize;
    fn spy_fail(&self, fail: bool);
}

pub struct Gateway<S: Store> {
    store: S,
    clock: Box<dyn Clock>,
    guard: Box<dyn Guard>,
}

impl<S: Store> Gateway<S> {
    pub fn new(store: S, clock: impl Clock + 'static, guard: impl Guard + 'static) -> Self {
        Self {
            store,
            clock: Box::new(clock),
            guard: Box::new(guard),
        }
    }

    pub fn now(&self) -> i64 {
        self.clock.now()
    }

    #[cfg(test)]
    pub(crate) fn store_calls(&self) -> usize
    where
        S: AsSpy,
    {
        self.store.spy_calls()
    }

    #[cfg(test)]
    pub(crate) fn set_store_failing(&self, fail: bool)
    where
        S: AsSpy,
    {
        self.store.spy_fail(fail)
    }

    pub fn join(&self, who: &Principal, room: &str) -> Result<()> {
        self.admit(who, room, Operation::Join { room: room.into() })?;
        self.store.join(room, &who.member, self.now())
    }

    pub fn post(&self, who: &Principal, room: &str, body: &str) -> Result<Posted> {
        check_member(who)?;
        check_room(room)?;
        if body.trim().is_empty() {
            return Err(Error::Invalid(
                "empty message: there is nothing to post".into(),
            ));
        }
        self.guard.check(
            who,
            &Operation::Post {
                room: room.into(),
                body: body.into(),
            },
        )?;
        self.store.post_as(
            room,
            &who.member,
            who.attribution,
            DEFAULT_LABEL,
            body,
            self.now(),
        )
    }

    pub fn read(&self, who: &Principal, room: &str, advance: bool) -> Result<Vec<Message>> {
        self.admit(
            who,
            room,
            Operation::Read {
                room: room.into(),
                advance,
            },
        )?;
        self.store.read(room, &who.member, advance, self.now())
    }

    /// What a `wait` should consider: unread messages it has not yet looked at. Moves no cursor; refreshes presence.
    pub fn wait_poll(&self, who: &Principal, room: &str) -> Result<Vec<Message>> {
        self.admit(
            who,
            room,
            Operation::Read {
                room: room.into(),
                advance: false,
            },
        )?;
        self.store.wait_poll(room, &who.member, self.now())
    }

    /// Record that a `wait` looked at the room up to `seq`; the messages stay unread.
    pub fn wait_seen(&self, who: &Principal, room: &str, seq: i64) -> Result<()> {
        self.admit(
            who,
            room,
            Operation::Read {
                room: room.into(),
                advance: false,
            },
        )?;
        self.store.wait_seen(room, &who.member, seq, self.now())
    }

    /// Set the caller's own status in a room (for example `done`). Short, one line, no control characters.
    pub fn set_status(&self, who: &Principal, room: &str, status: &str) -> Result<()> {
        check_member(who)?;
        check_room(room)?;
        if status.trim().is_empty() || status.chars().count() > 80 || has_control(status) {
            return Err(Error::Invalid(
                "bad status: one line of 1 to 80 characters, no control characters".into(),
            ));
        }
        self.guard.check(
            who,
            &Operation::Status {
                room: room.into(),
                status: status.into(),
            },
        )?;
        self.store
            .set_status(room, &who.member, status.trim(), self.now())
    }

    /// Record that one of the caller's hook events ran, and how many bytes it printed (never the content).
    pub fn record_hook_run(&self, who: &Principal, event: &str, bytes: usize) -> Result<()> {
        check_member(who)?;
        if event.is_empty() || event.len() > 40 || !event.chars().all(|c| c.is_ascii_alphanumeric())
        {
            return Err(Error::Invalid("bad hook event name".into()));
        }
        self.store
            .record_hook_run(&who.member, event, self.now(), bytes as i64)
    }

    /// The latest run of each hook event recorded for `member` (who may be the caller or anyone: metadata only).
    pub fn hook_runs(&self, who: &Principal, member: &str) -> Result<Vec<klatch_core::HookRun>> {
        check_member(who)?;
        if !valid_name(member) {
            return Err(Error::Invalid("bad member name".into()));
        }
        self.store.hook_runs(member)
    }

    /// The room's board: open asks, current offers and members with their last announced status.
    pub fn board(&self, who: &Principal, room: &str) -> Result<klatch_core::Board> {
        self.admit(
            who,
            room,
            Operation::Read {
                room: room.into(),
                advance: false,
            },
        )?;
        let now = self.now();
        let messages = self.store.tail(room, 0, 5000)?;
        let statuses = klatch_core::statuses(&messages);
        let all = self.store.members(room)?;
        let done: Vec<String> = all
            .iter()
            .filter(|m| m.status.as_deref().is_some_and(|s| s.starts_with("done")))
            .map(|m| m.name.clone())
            .collect();
        let (recent, gone): (Vec<_>, Vec<_>) = all
            .into_iter()
            .partition(|m| now - m.last_seen <= klatch_core::MEMBER_ACTIVE_SECS);
        // An offer follows its sender's presence: a member that has gone quiet is not available, and a member
        // that keeps waiting (which refreshes presence) keeps its offer with no renewal.
        let absent: Vec<String> = gone.iter().map(|m| m.name.clone()).collect();
        let members = recent
            .into_iter()
            .map(|m| klatch_core::BoardMember {
                status: m.status.clone().or_else(|| {
                    statuses
                        .iter()
                        .find(|s| s.member == m.name)
                        .map(|s| s.status.clone())
                }),
                name: m.name,
                last_seen: m.last_seen,
            })
            .collect();
        Ok(klatch_core::Board {
            room: room.into(),
            asks: self.store.open_asks(room, now)?,
            offers: klatch_core::current_offers(&messages, now, klatch_core::OFFER_TTL_SECS)
                .into_iter()
                .filter(|o| !done.contains(&o.sender) && !absent.contains(&o.sender))
                .collect(),
            members,
            stale_members: gone.len(),
        })
    }

    /// Asks in a room nobody has answered or closed.
    pub fn open_asks(&self, who: &Principal, room: &str) -> Result<Vec<klatch_core::OpenAsk>> {
        self.admit(
            who,
            room,
            Operation::Read {
                room: room.into(),
                advance: false,
            },
        )?;
        self.store.open_asks(room, self.now())
    }

    pub fn who(&self, who: &Principal, room: &str) -> Result<Vec<MemberInfo>> {
        self.admit(who, room, Operation::Who { room: room.into() })?;
        self.store.members(room)
    }

    pub fn claim(
        &self,
        who: &Principal,
        room: &str,
        path: &str,
        ttl: i64,
        reason: &str,
    ) -> Result<()> {
        check_member(who)?;
        check_room(room)?;
        check_path(path)?;
        check_reason(reason)?;
        if ttl <= 0 {
            return Err(Error::Invalid("ttl must be positive".into()));
        }
        self.guard.check(
            who,
            &Operation::Claim {
                room: room.into(),
                path: path.into(),
            },
        )?;
        self.store
            .claim(room, &who.member, path, ttl, reason, self.now())
    }

    pub fn release(&self, who: &Principal, room: &str, path: &str) -> Result<()> {
        check_member(who)?;
        check_room(room)?;
        check_path(path)?;
        self.guard.check(
            who,
            &Operation::Release {
                room: room.into(),
                path: path.into(),
            },
        )?;
        self.store.release(room, &who.member, path)
    }

    pub fn claims(&self, who: &Principal, room: &str) -> Result<Vec<Claim>> {
        self.admit(who, room, Operation::Claims { room: room.into() })?;
        self.store.claims(room, self.now())
    }

    /// Messages after `after_seq`, oldest first, at most `limit`, from every sender including the caller.
    /// No cursor or presence side effects; the guard sees a `Subscribe` operation.
    pub fn tail(
        &self,
        who: &Principal,
        room: &str,
        after_seq: i64,
        limit: usize,
    ) -> Result<Vec<Message>> {
        check_member(who)?;
        check_room(room)?;
        if after_seq < 0 || limit == 0 {
            return Err(Error::Invalid("bad tail range".into()));
        }
        self.guard
            .check(who, &Operation::Subscribe { room: room.into() })?;
        self.store.tail(room, after_seq, limit)
    }

    /// The room's highest `seq`, 0 when empty; the guard sees a `Subscribe` operation.
    pub fn head(&self, who: &Principal, room: &str) -> Result<i64> {
        self.admit(who, room, Operation::Subscribe { room: room.into() })?;
        self.store.head(room)
    }

    /// Acknowledge messages up to `seq`: moves the caller's read cursor forward, never past the head.
    pub fn ack(&self, who: &Principal, room: &str, seq: i64) -> Result<()> {
        check_member(who)?;
        check_room(room)?;
        if seq < 0 {
            return Err(Error::Invalid("bad acknowledgement".into()));
        }
        self.guard
            .check(who, &Operation::Ack { room: room.into() })?;
        self.store
            .advance_cursor(room, &who.member, seq, self.now())
    }

    /// Unread messages and others' claims for every room the caller has joined. A room the guard refuses
    /// is reported in `refused`, never dropped silently; store failures are errors.
    pub fn digest(&self, who: &Principal) -> Result<Digest> {
        self.digest_with(who, false)
    }

    /// `digest`, optionally with the backlog of open unclaimed asks from others (for a session start).
    pub fn digest_with(&self, who: &Principal, backlog: bool) -> Result<Digest> {
        check_member(who)?;
        let now = self.now();
        let mut rooms = Vec::new();
        let mut refused = Vec::new();
        for room in self.store.rooms_of(&who.member)? {
            let op = Operation::Read {
                room: room.clone(),
                advance: true,
            };
            if let Err(e) = self.guard.check(who, &op) {
                refused.push(Refused {
                    room,
                    reason: e.to_string(),
                });
                continue;
            }
            let messages = self.store.hook_read(&room, &who.member, now)?;
            let changes = self.store.claim_changes(&room, &who.member, now)?;
            let stale = self
                .store
                .stale_asks(&room, &who.member, now, STALE_AFTER_SECS)?;
            let escalated = self.store.escalated_asks(&room, &who.member, now)?;
            let backlog = if backlog {
                self.store
                    .open_asks(&room, now)?
                    .into_iter()
                    .filter(|a| a.sender != who.member && !a.claimed)
                    .collect()
            } else {
                Vec::new()
            };
            rooms.push(RoomDigest {
                room,
                messages,
                claims: changes.added,
                released: changes.released,
                stale,
                backlog,
                escalated,
            });
        }
        Ok(Digest {
            rooms,
            refused,
            now,
        })
    }

    /// Validate the caller and the room, then ask the guard. Runs before the store is touched.
    fn admit(&self, who: &Principal, room: &str, op: Operation) -> Result<()> {
        check_member(who)?;
        check_room(room)?;
        self.guard.check(who, &op)
    }
}

fn check_member(who: &Principal) -> Result<()> {
    if valid_name(&who.member) {
        Ok(())
    } else {
        Err(Error::Invalid("bad member name".into()))
    }
}

fn check_room(room: &str) -> Result<()> {
    if valid_name(room) {
        Ok(())
    } else {
        Err(Error::Invalid("bad room name".into()))
    }
}

fn has_control(s: &str) -> bool {
    s.chars().any(char::is_control)
}

fn check_path(path: &str) -> Result<()> {
    if path.is_empty() || path.len() > MAX_PATH_BYTES || has_control(path) {
        Err(Error::Invalid(format!(
            "bad claim path: 1 to {MAX_PATH_BYTES} bytes, no control characters"
        )))
    } else {
        Ok(())
    }
}

fn check_reason(reason: &str) -> Result<()> {
    if reason.len() > MAX_REASON_BYTES || has_control(reason) {
        Err(Error::Invalid(format!(
            "bad claim reason: at most {MAX_REASON_BYTES} bytes, no control characters"
        )))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{SpyGuard, SpyStore};
    use crate::{AllowAll, FixedClock, Operation};
    use klatch_core::Error;
    use std::rc::Rc;

    fn alice() -> Principal {
        Principal::unattributed("alice")
    }
    fn bob() -> Principal {
        Principal::unattributed("bob")
    }
    fn open() -> Gateway<SpyStore> {
        Gateway::new(SpyStore::new(), FixedClock(100), AllowAll)
    }

    #[test]
    fn post_then_read_round_trips_through_the_gateway() {
        let g = open();
        g.join(&alice(), "r").unwrap();
        let p = g.post(&bob(), "r", "hello").unwrap();
        let got = g.read(&alice(), "r", true).unwrap();
        assert_eq!(
            (got[0].sender.as_str(), got[0].body.as_str(), got[0].seq),
            ("bob", "hello", p.seq)
        );
        assert!(g.read(&alice(), "r", true).unwrap().is_empty());
    }

    #[test]
    fn the_injected_clock_is_the_only_source_of_time() {
        let g = open();
        g.claim(&alice(), "r", "src", 10, "").unwrap();
        assert_eq!(g.claims(&alice(), "r").unwrap()[0].expires, 110);
        assert_eq!(g.now(), 100);
    }

    #[test]
    fn every_operation_validates_the_room_name_before_anything_else() {
        let g = open();
        for room in ["bad room", "a\nb", "", "x/y"] {
            let who = alice();
            assert!(
                matches!(g.join(&who, room), Err(Error::Invalid(_))),
                "join {room:?}"
            );
            assert!(
                matches!(g.post(&who, room, "x"), Err(Error::Invalid(_))),
                "post {room:?}"
            );
            assert!(
                matches!(g.read(&who, room, true), Err(Error::Invalid(_))),
                "read {room:?}"
            );
            assert!(
                matches!(g.who(&who, room), Err(Error::Invalid(_))),
                "who {room:?}"
            );
            assert!(
                matches!(g.claim(&who, room, "p", 10, ""), Err(Error::Invalid(_))),
                "claim {room:?}"
            );
            assert!(
                matches!(g.release(&who, room, "p"), Err(Error::Invalid(_))),
                "release {room:?}"
            );
            assert!(
                matches!(g.claims(&who, room), Err(Error::Invalid(_))),
                "claims {room:?}"
            );
        }
        assert_eq!(
            g.store.calls.get(),
            0,
            "an invalid request must not reach the store"
        );
    }

    #[test]
    fn an_invalid_member_name_is_refused_everywhere() {
        let g = open();
        let bad = Principal::unattributed("bad name\nforged");
        assert!(matches!(g.join(&bad, "r"), Err(Error::Invalid(_))));
        assert!(matches!(g.post(&bad, "r", "x"), Err(Error::Invalid(_))));
        assert!(matches!(g.digest(&bad), Err(Error::Invalid(_))));
        assert_eq!(g.store.calls.get(), 0);
    }

    #[test]
    fn claim_paths_and_reasons_are_checked() {
        let g = open();
        let who = alice();
        for path in ["", "a\nb", "a\rb", "a\u{0}b", &"x".repeat(1025)] {
            assert!(
                matches!(g.claim(&who, "r", path, 10, ""), Err(Error::Invalid(_))),
                "path {path:?}"
            );
        }
        assert!(matches!(
            g.claim(&who, "r", "src", 10, "bad\nreason"),
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            g.claim(&who, "r", "src", 10, &"y".repeat(501)),
            Err(Error::Invalid(_))
        ));
        g.claim(&who, "r", "src/a b.rs", 10, "a fine reason")
            .unwrap();
        assert_eq!(g.store.calls.get(), 1);
    }

    #[test]
    fn a_claim_needs_a_positive_ttl() {
        let g = open();
        assert!(matches!(
            g.claim(&alice(), "r", "src", 0, ""),
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            g.claim(&alice(), "r", "src", -5, ""),
            Err(Error::Invalid(_))
        ));
    }

    #[test]
    fn the_guard_runs_once_per_operation_in_order_before_the_store() {
        let guard = Rc::new(SpyGuard::allowing());
        let g = Gateway::new(SpyStore::new(), FixedClock(1), guard.clone());
        let who = alice();
        g.join(&who, "r").unwrap();
        g.post(&who, "r", "hi").unwrap();
        g.read(&who, "r", false).unwrap();
        g.who(&who, "r").unwrap();
        g.claim(&who, "r", "src", 10, "").unwrap();
        g.claims(&who, "r").unwrap();
        g.release(&who, "r", "src").unwrap();
        let kinds: Vec<&str> = guard
            .seen
            .borrow()
            .iter()
            .map(|o| match o {
                Operation::Join { .. } => "join",
                Operation::Post { .. } => "post",
                Operation::Read { .. } => "read",
                Operation::Who { .. } => "who",
                Operation::Claim { .. } => "claim",
                Operation::Claims { .. } => "claims",
                Operation::Release { .. } => "release",
                Operation::Subscribe { .. } => "subscribe",
                Operation::Status { .. } => "status",
                Operation::Ack { .. } => "ack",
            })
            .collect();
        assert_eq!(
            kinds,
            vec!["join", "post", "read", "who", "claim", "claims", "release"]
        );
    }

    #[test]
    fn the_guard_sees_the_post_body_so_a_label_check_can_judge_it() {
        let guard = Rc::new(SpyGuard::allowing());
        let g = Gateway::new(SpyStore::new(), FixedClock(1), guard.clone());
        g.post(&alice(), "r", "the body").unwrap();
        assert_eq!(
            guard.seen.borrow()[0],
            Operation::Post {
                room: "r".into(),
                body: "the body".into()
            }
        );
    }

    #[test]
    fn a_denied_operation_never_reaches_the_store_and_returns_the_guards_error() {
        let g = Gateway::new(
            SpyStore::new(),
            FixedClock(1),
            Rc::new(SpyGuard::denying(&["locked"])),
        );
        let who = alice();
        let err = g.post(&who, "locked", "x").unwrap_err();
        assert!(
            matches!(err, Error::Refused { ref rule, .. } if rule == "test.deny_room"),
            "{err}"
        );
        assert!(g.read(&who, "locked", true).is_err());
        assert!(g.claim(&who, "locked", "p", 10, "").is_err());
        assert_eq!(g.store.calls.get(), 0);
        g.post(&who, "open", "x").unwrap();
    }

    #[test]
    fn store_failures_surface_as_errors_never_as_success() {
        let g = open();
        g.store.fail.set(true);
        let who = alice();
        assert!(matches!(g.post(&who, "r", "x"), Err(Error::Backend(_))));
        assert!(matches!(g.read(&who, "r", true), Err(Error::Backend(_))));
        assert!(matches!(
            g.claim(&who, "r", "p", 10, ""),
            Err(Error::Backend(_))
        ));
        assert!(matches!(g.claims(&who, "r"), Err(Error::Backend(_))));
        assert!(matches!(g.digest(&who), Err(Error::Backend(_))));
    }

    #[test]
    fn tail_head_and_ack_validate_before_the_store_and_go_through_the_guard() {
        let guard = Rc::new(SpyGuard::allowing());
        let g = Gateway::new(SpyStore::new(), FixedClock(1), guard.clone());
        let who = alice();
        for room in ["bad room", "a\nb", ""] {
            assert!(matches!(g.tail(&who, room, 0, 10), Err(Error::Invalid(_))));
            assert!(matches!(g.head(&who, room), Err(Error::Invalid(_))));
            assert!(matches!(g.ack(&who, room, 1), Err(Error::Invalid(_))));
        }
        assert!(matches!(g.tail(&who, "r", -1, 10), Err(Error::Invalid(_))));
        assert!(matches!(g.tail(&who, "r", 0, 0), Err(Error::Invalid(_))));
        assert!(matches!(g.ack(&who, "r", -1), Err(Error::Invalid(_))));
        assert_eq!(g.store.calls.get(), 0);
        assert!(
            guard.seen.borrow().is_empty(),
            "invalid requests never reach the guard"
        );
        g.tail(&who, "r", 0, 10).unwrap();
        g.head(&who, "r").unwrap();
        g.ack(&who, "r", 0).unwrap();
        assert_eq!(
            *guard.seen.borrow(),
            vec![
                Operation::Subscribe { room: "r".into() },
                Operation::Subscribe { room: "r".into() },
                Operation::Ack { room: "r".into() },
            ]
        );
    }

    #[test]
    fn tail_includes_everyones_messages_and_head_reports_the_top_seq() {
        let g = open();
        g.post(&alice(), "r", "one").unwrap();
        g.post(&bob(), "r", "two").unwrap();
        assert_eq!(g.tail(&alice(), "r", 0, 10).unwrap().len(), 2);
        assert_eq!(g.head(&alice(), "r").unwrap(), 2);
    }

    #[test]
    fn an_ack_moves_the_read_cursor_so_the_hook_path_does_not_redeliver() {
        let g = open();
        g.join(&alice(), "r").unwrap();
        for i in 0..3 {
            g.post(&bob(), "r", &format!("m{i}")).unwrap();
        }
        g.ack(&alice(), "r", 2).unwrap();
        let unread = g.read(&alice(), "r", false).unwrap();
        assert_eq!(unread.iter().map(|m| m.seq).collect::<Vec<_>>(), vec![3]);
    }

    #[test]
    fn a_denied_tail_never_reaches_the_store() {
        let g = Gateway::new(
            SpyStore::new(),
            FixedClock(1),
            Rc::new(SpyGuard::denying(&["locked"])),
        );
        assert!(matches!(
            g.tail(&alice(), "locked", 0, 10),
            Err(Error::Refused { .. })
        ));
        assert!(matches!(
            g.ack(&alice(), "locked", 1),
            Err(Error::Refused { .. })
        ));
        assert_eq!(g.store.calls.get(), 0);
    }

    #[test]
    fn a_verified_principal_posts_a_verified_message_and_an_unattributed_one_stays_unattributed() {
        let g = open();
        g.post(&Principal::verified("carol"), "r", "proven")
            .unwrap();
        g.post(&Principal::unattributed("eve"), "r", "unproven")
            .unwrap();
        let got = g.tail(&alice(), "r", 0, 10).unwrap();
        assert_eq!(got[0].attribution, klatch_core::Attribution::Verified);
        assert_eq!(got[1].attribution, klatch_core::Attribution::Unattributed);
    }

    #[test]
    fn messages_are_posted_under_the_default_label() {
        let g = open();
        g.post(&alice(), "r", "x").unwrap();
        assert_eq!(
            g.tail(&alice(), "r", 0, 10).unwrap()[0].label,
            DEFAULT_LABEL
        );
        assert_eq!(DEFAULT_LABEL, "public");
    }

    #[test]
    fn an_empty_or_whitespace_only_body_is_refused_before_the_guard_and_the_store() {
        let guard = Rc::new(SpyGuard::allowing());
        let g = Gateway::new(SpyStore::new(), FixedClock(1), guard.clone());
        for body in ["", " ", "   \t ", "\n", " \r\n "] {
            assert!(
                matches!(g.post(&alice(), "r", body), Err(Error::Invalid(_))),
                "body {body:?}"
            );
        }
        assert_eq!(g.store.calls.get(), 0);
        assert!(guard.seen.borrow().is_empty());
        g.post(&alice(), "r", " x ").unwrap();
    }

    #[test]
    fn a_digest_collects_unread_and_others_claims_and_advances_the_cursor() {
        let g = open();
        g.join(&alice(), "r").unwrap();
        g.post(&bob(), "r", "ping").unwrap();
        g.claim(&bob(), "r", "src", 50, "").unwrap();
        g.claim(&alice(), "r", "docs", 50, "").unwrap();
        let d = g.digest(&alice()).unwrap();
        assert_eq!(d.now, 100);
        assert_eq!(d.rooms.len(), 1);
        assert_eq!(d.rooms[0].messages[0].body, "ping");
        assert_eq!(
            d.rooms[0]
                .claims
                .iter()
                .map(|c| c.owner.as_str())
                .collect::<Vec<_>>(),
            vec!["bob"]
        );
        assert!(g.digest(&alice()).unwrap().rooms[0].messages.is_empty());
    }

    #[test]
    fn a_message_a_digest_showed_can_still_be_read_once() {
        let g = open();
        g.join(&alice(), "r").unwrap();
        g.post(&bob(), "r", "ask").unwrap();
        assert_eq!(g.digest(&alice()).unwrap().rooms[0].messages.len(), 1);
        assert!(g.digest(&alice()).unwrap().rooms[0].messages.is_empty());
        assert_eq!(g.read(&alice(), "r", true).unwrap().len(), 1);
        assert!(g.read(&alice(), "r", true).unwrap().is_empty());
    }

    #[test]
    fn a_message_a_wait_has_seen_is_still_unread_and_the_wait_does_not_wake_again() {
        let g = open();
        g.join(&alice(), "r").unwrap();
        g.post(&bob(), "r", "ask").unwrap();
        assert_eq!(g.wait_poll(&alice(), "r").unwrap().len(), 1);
        g.wait_seen(&alice(), "r", 1).unwrap();
        assert!(g.wait_poll(&alice(), "r").unwrap().is_empty());
        assert_eq!(g.read(&alice(), "r", false).unwrap().len(), 1);
    }

    #[test]
    fn open_asks_go_through_the_gateway() {
        let g = open();
        g.join(&alice(), "r").unwrap();
        g.post(&bob(), "r", "seeking: slugify").unwrap();
        let o = g.open_asks(&alice(), "r").unwrap();
        assert_eq!((o.len(), o[0].sender.as_str()), (1, "bob"));
    }

    #[test]
    fn a_digest_tells_the_asker_once_that_its_ask_has_no_claim() {
        let g = open();
        g.join(&alice(), "r").unwrap();
        g.store.post("r", "alice", "seeking: a lib", 10).unwrap();
        let d = g.digest(&alice()).unwrap();
        assert_eq!(d.rooms[0].stale.len(), 1);
        assert!(
            g.digest(&alice()).unwrap().rooms[0].stale.is_empty(),
            "reported once"
        );
    }

    #[test]
    fn a_young_or_claimed_ask_is_not_stale() {
        let g = open();
        g.join(&alice(), "r").unwrap();
        g.store.post("r", "alice", "seeking: young", 90).unwrap();
        let old = g
            .store
            .post("r", "alice", "seeking: claimed", 10)
            .unwrap()
            .seq;
        g.claim(&bob(), "r", &format!("ask:{old}"), 500, "")
            .unwrap();
        assert!(g.digest(&alice()).unwrap().rooms[0].stale.is_empty());
    }

    #[test]
    fn the_backlog_lists_others_open_unclaimed_asks_only_when_asked_for() {
        let g = open();
        g.join(&alice(), "r").unwrap();
        g.store.post("r", "bob", "seeking: open", 10).unwrap();
        let claimed = g.store.post("r", "bob", "seeking: taken", 10).unwrap().seq;
        g.store.post("r", "alice", "seeking: mine", 10).unwrap();
        g.claim(&bob(), "r", &format!("ask:{claimed}"), 500, "")
            .unwrap();
        assert!(
            g.digest(&alice())
                .unwrap()
                .rooms
                .iter()
                .all(|r| r.backlog.is_empty())
        );
        let d = g.digest_with(&alice(), true).unwrap();
        let b = &d.rooms[0].backlog;
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].text, "seeking: open");
    }

    #[test]
    fn the_board_combines_asks_offers_members_and_status() {
        let g = open();
        g.join(&alice(), "r").unwrap();
        g.store.post("r", "bob", "seeking: slugify", 90).unwrap();
        g.store.post("r", "alice", "offering: helpers", 95).unwrap();
        g.store.post("r", "bob", "status: busy", 96).unwrap();
        let b = g.board(&alice(), "r").unwrap();
        assert_eq!(b.room, "r");
        assert_eq!(b.asks.len(), 1);
        assert_eq!(b.offers.len(), 1);
        assert_eq!(b.offers[0].sender, "alice");
        let bob = b.members.iter().find(|m| m.name == "bob").unwrap();
        assert_eq!(bob.status.as_deref(), Some("busy"));
        let alice = b.members.iter().find(|m| m.name == "alice").unwrap();
        assert_eq!(alice.status, None);
    }

    #[test]
    fn a_board_is_refused_where_a_read_is_and_never_reaches_the_store() {
        let g = Gateway::new(
            SpyStore::new(),
            FixedClock(1),
            Rc::new(SpyGuard::denying(&["locked"])),
        );
        assert!(g.board(&alice(), "locked").is_err());
        assert_eq!(g.store.calls.get(), 0);
    }

    #[test]
    fn the_board_lists_recent_members_and_counts_the_long_gone() {
        let g = Gateway::new(SpyStore::new(), FixedClock(1000), AllowAll);
        g.join(&alice(), "r").unwrap();
        g.store.join("r", "ghost", 1).unwrap();
        g.store.join("r", "bob", 999).unwrap();
        let b = g.board(&alice(), "r").unwrap();
        let names: Vec<_> = b.members.iter().map(|m| m.name.as_str()).collect();
        assert!(
            names.contains(&"alice") && names.contains(&"bob"),
            "{names:?}"
        );
        assert!(
            !names.contains(&"ghost"),
            "listed a member not seen for ages: {names:?}"
        );
        assert_eq!(b.stale_members, 1);
    }

    #[test]
    fn reading_the_board_does_not_make_the_reader_a_member() {
        let g = open();
        g.join(&bob(), "r").unwrap();
        g.board(&alice(), "r").unwrap();
        let names: Vec<_> = g
            .who(&bob(), "r")
            .unwrap()
            .into_iter()
            .map(|m| m.name)
            .collect();
        assert!(!names.contains(&"alice".to_string()), "{names:?}");
    }

    #[test]
    fn a_status_goes_through_the_guard_and_shows_in_who() {
        let g = Gateway::new(
            SpyStore::new(),
            FixedClock(5),
            Rc::new(SpyGuard::allowing()),
        );
        g.join(&alice(), "r").unwrap();
        g.set_status(&alice(), "r", "done leaving").unwrap();
        let m = g.who(&alice(), "r").unwrap();
        assert_eq!(m[0].status.as_deref(), Some("done leaving"));
    }

    #[test]
    fn a_status_in_a_refused_room_never_reaches_the_store() {
        let g = Gateway::new(
            SpyStore::new(),
            FixedClock(1),
            Rc::new(SpyGuard::denying(&["locked"])),
        );
        assert!(g.set_status(&alice(), "locked", "done").is_err());
        assert_eq!(g.store.calls.get(), 0);
    }

    #[test]
    fn a_bad_status_is_rejected_before_the_store() {
        let g = open();
        for bad in ["", "   ", "a\nb", "x\u{7}y", &"z".repeat(81)] {
            let err = g.set_status(&alice(), "r", bad).unwrap_err();
            assert!(matches!(err, Error::Invalid(_)), "{bad:?}: {err}");
        }
        assert_eq!(g.store.calls.get(), 0);
    }

    #[test]
    fn the_board_prefers_the_status_fact_and_drops_the_offers_of_a_member_who_is_done() {
        let g = open();
        g.join(&alice(), "r").unwrap();
        g.store.post("r", "bob", "offering: helpers", 90).unwrap();
        g.set_status(&bob(), "r", "done leaving").unwrap();
        let b = g.board(&alice(), "r").unwrap();
        let bob = b.members.iter().find(|m| m.name == "bob").unwrap();
        assert_eq!(bob.status.as_deref(), Some("done leaving"));
        assert!(
            b.offers.is_empty(),
            "a done member's offer stayed on the board"
        );
    }

    #[test]
    fn an_offer_goes_when_its_sender_has_not_been_seen_for_a_while() {
        let g = Gateway::new(SpyStore::new(), FixedClock(1000), AllowAll);
        g.join(&alice(), "r").unwrap();
        g.store.post("r", "ghost", "offering: old help", 1).unwrap();
        g.store
            .post("r", "bob", "offering: fresh help", 999)
            .unwrap();
        let b = g.board(&alice(), "r").unwrap();
        let who: Vec<_> = b.offers.iter().map(|o| o.sender.as_str()).collect();
        assert_eq!(who, ["bob"], "an offer outlived its sender's presence");
    }

    #[test]
    fn a_present_member_keeps_its_offer_without_renewing_it() {
        let g = Gateway::new(SpyStore::new(), FixedClock(5000), AllowAll);
        g.join(&alice(), "r").unwrap();
        g.store.post("r", "bob", "offering: help", 100).unwrap();
        g.store.wait_poll("r", "bob", 4990).unwrap();
        let b = g.board(&alice(), "r").unwrap();
        assert_eq!(
            b.offers.len(),
            1,
            "a waiting member lost its offer to the clock"
        );
    }

    #[test]
    fn a_hook_run_is_recorded_for_the_caller_and_readable_by_anyone() {
        let g = open();
        g.record_hook_run(&alice(), "SessionStart", 0).unwrap();
        g.record_hook_run(&alice(), "SessionStart", 40).unwrap();
        let r = g.hook_runs(&bob(), "alice").unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(
            (
                r[0].event.as_str(),
                r[0].runs,
                r[0].last_bytes,
                r[0].last_ts
            ),
            ("SessionStart", 2, 40, 100)
        );
    }

    #[test]
    fn a_bad_event_or_member_name_is_rejected_before_the_store() {
        let g = open();
        for bad in ["", "a b", "x\ny", &"e".repeat(41)] {
            assert!(
                matches!(g.record_hook_run(&alice(), bad, 0), Err(Error::Invalid(_))),
                "{bad:?}"
            );
        }
        assert!(matches!(
            g.hook_runs(&alice(), "bad name"),
            Err(Error::Invalid(_))
        ));
        assert_eq!(g.store.calls.get(), 0);
    }

    #[test]
    fn a_digest_shows_a_claim_once_not_on_every_call() {
        let g = open();
        g.join(&alice(), "r").unwrap();
        g.claim(&bob(), "r", "src", 50, "").unwrap();
        assert_eq!(g.digest(&alice()).unwrap().rooms[0].claims.len(), 1);
        for _ in 0..3 {
            let d = g.digest(&alice()).unwrap();
            assert!(d.rooms[0].claims.is_empty(), "unchanged claim repeated");
            assert!(d.rooms[0].released.is_empty());
        }
    }

    #[test]
    fn a_digest_reports_a_release_once() {
        let g = open();
        g.join(&alice(), "r").unwrap();
        g.claim(&bob(), "r", "src", 50, "").unwrap();
        g.digest(&alice()).unwrap();
        g.release(&bob(), "r", "src").unwrap();
        let d = g.digest(&alice()).unwrap();
        assert_eq!(d.rooms[0].released.len(), 1);
        assert_eq!(d.rooms[0].released[0].owner, "bob");
        assert_eq!(d.rooms[0].released[0].path, "src");
        assert!(g.digest(&alice()).unwrap().rooms[0].released.is_empty());
    }

    #[test]
    fn a_digest_still_delivers_messages_when_claims_are_unchanged() {
        let g = open();
        g.join(&alice(), "r").unwrap();
        g.claim(&bob(), "r", "src", 50, "").unwrap();
        g.digest(&alice()).unwrap();
        g.post(&bob(), "r", "later").unwrap();
        let d = g.digest(&alice()).unwrap();
        assert_eq!(d.rooms[0].messages.len(), 1);
        assert!(d.rooms[0].claims.is_empty());
    }

    #[test]
    fn a_digest_reports_refused_rooms_and_still_returns_the_others() {
        let g = Gateway::new(
            SpyStore::new(),
            FixedClock(1),
            Rc::new(SpyGuard::denying(&["secretroom"])),
        );
        let who = alice();
        g.join(&who, "open").unwrap();
        g.store.inner_join_for_test("secretroom", "alice");
        g.post(&bob(), "open", "visible").unwrap();
        let d = g.digest(&who).unwrap();
        assert_eq!(d.rooms.len(), 1);
        assert_eq!(d.rooms[0].room, "open");
        assert_eq!(d.refused.len(), 1);
        assert_eq!(d.refused[0].room, "secretroom");
        assert!(d.refused[0].reason.contains("test.deny_room"));
    }
}
