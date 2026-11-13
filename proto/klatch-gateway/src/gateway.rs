use crate::{Clock, Guard, Operation};
use klatch_core::{
    Claim, Error, MemberInfo, Message, Posted, Principal, Result, RoomDigest, valid_name,
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
        self.admit(
            who,
            room,
            Operation::Post {
                room: room.into(),
                body: body.into(),
            },
        )?;
        self.store.post(room, &who.member, body, self.now())
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
            let messages = self.store.read(&room, &who.member, true, now)?;
            let claims = self
                .store
                .claims(&room, now)?
                .into_iter()
                .filter(|c| c.owner != who.member)
                .collect();
            rooms.push(RoomDigest {
                room,
                messages,
                claims,
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
