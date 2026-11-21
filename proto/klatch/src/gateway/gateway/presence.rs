//! Who a session is and when it was last seen: parks, hook runs, notes about the member and its scope.

use super::*;

impl<S: Store> Gateway<S> {
    /// A `parked:` post parks its sender; any other post is a sign of life and ends a park (DESIGN section 32 item
    /// 45). The post already happened, so a failure here is not the post's failure.
    pub(super) fn park_by_post(&self, who: &Principal, body: &str) {
        let now = self.now();
        let _ = match parse_parked(body, now) {
            Some(p) => self.store.park(&who.member, &p, now),
            None => self.store.unpark(&who.member, now),
        };
    }

    /// The Stop hook at the end of a turn: the turn was a sign of life, and background tasks or crons still
    /// running mean the member now waits on them (parked until it next speaks or the default horizon passes).
    pub fn record_stop(
        &self,
        who: &Principal,
        background_tasks: usize,
        crons: usize,
    ) -> Result<()> {
        check_member(who)?;
        self.check_own_record_live(who)?;
        let now = self.now();
        self.store.unpark(&who.member, now)?;
        if background_tasks + crons == 0 {
            return Ok(());
        }
        let plural = |n: usize, w: &str| format!("{n} {w}{}", if n == 1 { "" } else { "s" });
        let mut parts = Vec::new();
        if background_tasks > 0 {
            parts.push(plural(background_tasks, "background task"));
        }
        if crons > 0 {
            parts.push(plural(crons, "cron"));
        }
        let parked = Parked {
            why: parts.join(", "),
            back_by: now + PARKED_DEFAULT_SECS,
        };
        self.store.park(&who.member, &parked, now)
    }

    /// Any hook that runs is a sign of life: it ends a park.
    pub fn sign_of_life(&self, who: &Principal) -> Result<()> {
        check_member(who)?;
        self.check_own_record_live_and_renew(who)?;
        self.store.unpark(&who.member, self.now())
    }

    /// Remember that the caller is a child of `parent`, the member whose session started the caller's (#24); `None`
    /// records that a parent was looked for and there is none, so it is not looked for again. The first answer noted
    /// is kept.
    pub fn note_parent(&self, who: &Principal, parent: Option<&str>) -> Result<()> {
        check_member(who)?;
        self.check_own_record_live(who)?;
        let parent = parent.unwrap_or("");
        let acceptable = parent.is_empty() || (valid_name(parent) && parent != who.member);
        invalid(acceptable, "bad parent member".into())?;
        self.store.note_parent(&who.member, parent, self.now())
    }

    /// The rooms the caller has joined, without reading anything: a hook that only needs to act on each room (a
    /// session ending) must not use up the messages a digest would show the member's next session.
    pub fn rooms(&self, who: &Principal) -> Result<Vec<String>> {
        check_member(who)?;
        self.check_own_record_live(who)?;
        self.store.rooms_of(&who.member)
    }

    /// Set the caller's own status in a room (for example `done`). Short, one line, no control characters.
    pub fn set_status(&self, who: &Principal, room: &str, status: &str) -> Result<()> {
        check_member(who)?;
        check_room(room)?;
        check_status(status)?;
        self.refuse_text(who, (room, "status"), status)?;
        self.authorize(
            who,
            &Operation::Status {
                room: room.into(),
                status: status.into(),
            },
        )?;
        self.store
            .set_status(Seat::new(room, &who.member), status.trim(), self.now())
    }

    /// Record that one of the caller's hook events ran, and how many bytes it printed (never the content).
    pub fn record_hook_run(&self, who: &Principal, event: &str, bytes: usize) -> Result<()> {
        check_member(who)?;
        check_event_name(event)?;
        // the end of a session may record itself after its credential idled out; no other event may (#835)
        if event != "SessionEnd" {
            // every other hook event proves the session is alive, so it renews the credential (#1027)
            self.check_own_record_live_and_renew(who)?;
        }
        self.store.record_hook_run(
            &HookEvent::new(&who.member, event, bytes as i64),
            self.now(),
        )
    }

    /// The latest run of each hook event recorded for `member`: the caller's own freely, anyone else's through the guard.
    pub fn hook_runs(&self, who: &Principal, member: &str) -> Result<Vec<crate::domain::HookRun>> {
        check_member(who)?;
        if !valid_name(member) {
            return Err(Error::Invalid("bad member name".into()));
        }
        // a member reads its own record freely; another member's goes through the guard (#161)
        if member != who.member {
            self.check_guard(who, &Operation::admin("hook_runs", member))?;
        }
        self.store.hook_runs(member)
    }

    /// Remember the permission mode the caller's session reported (#553). A mode is a short word, so anything else
    /// is refused rather than stored.
    pub fn note_member_mode(&self, who: &Principal, mode: &str) -> Result<()> {
        check_member(who)?;
        self.check_own_record_live(who)?;
        invalid(identifier(mode, 40, ""), "bad permission mode".into())?;
        self.store.note_member_mode(&who.member, mode, self.now())
    }

    /// Remember that the caller is the member of harness session `session_id`.
    pub fn note_session(&self, who: &Principal, session_id: &str) -> Result<()> {
        check_member(who)?;
        self.check_own_record_live(who)?;
        check_session_id(session_id)?;
        self.store.note_session(session_id, &who.member, self.now())
    }

    /// Housekeeping at session start: remove members not seen for `older_than` seconds. Returns how many.
    pub fn prune_members(&self, who: &Principal, older_than: i64) -> Result<usize> {
        check_member(who)?;
        self.check_guard(who, &Operation::admin("prune", &older_than.to_string()))?;
        self.store.prune_members(older_than, self.now())
    }

    /// Declare the scope the caller works in (a repository, a bot's area, a project): claims then conflict only
    /// inside one scope (DESIGN section 27).
    pub fn set_scope(&self, who: &Principal, scope: &str) -> Result<()> {
        check_member(who)?;
        self.check_own_record_live(who)?;
        self.refuse_facilitator(who, "scope")?;
        if !crate::domain::valid_scope(scope) {
            return Err(Error::Invalid(
                "bad scope: 1 to 80 characters of letters, digits and . : _ - /".into(),
            ));
        }
        self.store.set_member_scope(&who.member, scope, self.now())
    }

    /// The scope the caller declared or was given, if any.
    pub fn scope_of(&self, who: &Principal) -> Result<Option<String>> {
        check_member(who)?;
        self.check_own_record_live(who)?;
        self.store.member_scope(&who.member)
    }
}
