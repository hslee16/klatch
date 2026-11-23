//! Model-based property tests (epic #396, #393): a random sequence of operations is run against the real gateway and
//! an in-memory store, and against a tiny model written separately and in the plainest way. After every step the two
//! must agree. They are evidence, not proof: a pass means no disagreement was found in the sequences tried.
//!
//! Run alone: `cargo test -p klatch --test models`; more sequences: `HEGEL_TEST_CASES=5000 cargo test -p klatch
//! --test models`. A failure prints the shortest failing sequence (each rule notes what it did) and a replay blob
//! (see CONTRIBUTING.md, "Property tests").

#[path = "models/roles.rs"]
mod roles;
use hegel::TestCase;
use hegel::generators as gs;
use klatch::domain::{Error, Principal};
use klatch::gateway::{AllowAll, ClaimSpec, Clock, Gateway};
use klatch::store::SqliteStore;
use roles::{Held4, ROLES, Try, every_try, parked_expiry, role_spec};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

const ROOM: &str = "r";
const MEMBERS: [&str; 3] = ["alice", "bob", "carol"];

/// A clock the test moves by hand.
struct TestClock(Arc<AtomicI64>);
impl Clock for TestClock {
    fn now(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

fn principal(index: u8) -> Principal {
    Principal::unattributed(MEMBERS[index as usize])
}

fn member_index() -> impl hegel::generators::PrintableGenerator<u8> {
    gs::integers::<u8>().max_value(MEMBERS.len() as u8 - 1)
}

fn gateway() -> (Gateway<SqliteStore>, Arc<AtomicI64>) {
    let now = Arc::new(AtomicI64::new(1_000));
    let gw = Gateway::new(
        SqliteStore::open_in_memory().unwrap(),
        TestClock(now.clone()),
        AllowAll,
    );
    for i in 0..MEMBERS.len() as u8 {
        gw.join(&principal(i), ROOM).unwrap();
    }
    (gw, now)
}

// ---- claims ----------------------------------------------------------------------------------------------------

const PATHS: [&str; 6] = [
    "src",
    "src/a.rs",
    "src/a.rs/x",
    "src/b.rs",
    "docs",
    "docs/guide",
];
/// Lifetimes in seconds, and clock steps that land before, exactly on and after an expiry.
const TTLS: [i64; 4] = [10, 60, 600, klatch::domain::CLAIM_MAX_SECS];
const STEPS: [i64; 5] = [1, 9, 10, 60, 90_000];

/// Two paths overlap when one is the other or lies beneath it, by whole path components. Written here on its own
/// rather than calling klatch's `overlaps`, so a bug in that function cannot hide itself.
fn related(a: &str, b: &str) -> bool {
    a == b || a.starts_with(&format!("{b}/")) || b.starts_with(&format!("{a}/"))
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Held {
    path: String,
    owner: String,
    expires: i64,
    /// The lifetime the claim was made with: activity renews it to this, counted from the activity (#494).
    ttl: i64,
}

struct ClaimsRoom {
    gw: Gateway<SqliteStore>,
    now: Arc<AtomicI64>,
    /// Every claim row the store holds, expired or not: expired rows are purged only when a claim is attempted.
    model: Vec<Held>,
    posts: usize,
    /// Members whose last post said `parked:`: a park stretches their live claims to the return time, and the next
    /// ordinary post cuts them back to a normal lease (DESIGN 45).
    parked: [bool; 3],
    /// The return time of the park a member is in, 0 when not parked (what an earlier park still holds, #647).
    held_until: [i64; 3],
}

impl ClaimsRoom {
    fn new() -> Self {
        let (gw, now) = gateway();
        Self {
            gw,
            now,
            model: Vec::new(),
            posts: 0,
            parked: [false; 3],
            held_until: [0; 3],
        }
    }
    fn now(&self) -> i64 {
        self.now.load(Ordering::SeqCst)
    }
}

#[hegel::state_machine]
impl ClaimsRoom {
    #[rule]
    fn claim(&mut self, tc: TestCase) {
        let who = tc.draw(member_index());
        let path = PATHS[tc.draw(gs::integers::<u8>().max_value(PATHS.len() as u8 - 1)) as usize];
        let ttl = TTLS[tc.draw(gs::integers::<u8>().max_value(TTLS.len() as u8 - 1)) as usize];
        let (me, now) = (MEMBERS[who as usize], self.now());
        tc.note(&format!("t={now}: {me} claims {path} for {ttl}s"));
        // an attempt first purges every lapsed claim
        self.model.retain(|c| c.expires > now);
        let blockers: Vec<Held> = self
            .model
            .iter()
            .filter(|c| c.owner != me && related(&c.path, path))
            .cloned()
            .collect();
        match self
            .gw
            .claim(&principal(who), ROOM, &ClaimSpec::new(path, ttl, "model"))
        {
            Ok(()) => {
                assert!(blockers.is_empty(), "claim granted over {blockers:?}");
                self.model.retain(|c| !(c.owner == me && c.path == path)); // a repeat renews
                self.model.push(Held {
                    path: path.into(),
                    owner: me.into(),
                    expires: now + ttl,
                    ttl,
                });
            }
            Err(Error::Conflict {
                owner, secs_left, ..
            }) => {
                let hit = blockers
                    .iter()
                    .find(|c| c.owner == owner)
                    .unwrap_or_else(|| {
                        panic!(
                            "conflict names {owner}, who holds nothing in the way: {:?}",
                            self.model
                        )
                    });
                assert_eq!(secs_left, hit.expires - now, "wrong time left on {hit:?}");
            }
            Err(other) => panic!("unexpected error {other:?}"),
        }
    }

    #[rule]
    fn release(&mut self, tc: TestCase) {
        let who = tc.draw(member_index());
        let path = PATHS[tc.draw(gs::integers::<u8>().max_value(PATHS.len() as u8 - 1)) as usize];
        let me = MEMBERS[who as usize];
        tc.note(&format!("t={}: {me} releases {path}", self.now()));
        let at = self
            .model
            .iter()
            .position(|c| c.owner == me && c.path == path);
        match self.gw.release(&principal(who), ROOM, path) {
            Ok(()) => {
                assert!(
                    at.is_some(),
                    "released a claim the model does not have: {me} {path}"
                );
                self.model.remove(at.unwrap());
            }
            Err(Error::NotFound(_)) => assert!(
                at.is_none(),
                "refused to release {me} {path}, which is held"
            ),
            Err(other) => panic!("unexpected error {other:?}"),
        }
    }

    /// Activity renews: a member that posts keeps each live claim it holds for the lifetime that claim was made with,
    /// counted from now. Never shorter than what the claim has, never a claim that already lapsed (RESEARCH-NOTES
    /// 42.8, #494). A park stretched a lease, so the first ordinary post after it cuts the lease back to a normal
    /// one (but never below what renewal gave it). The model says it in a few lines; the real thing is SQL.
    #[rule]
    fn activity(&mut self, tc: TestCase) {
        let who = tc.draw(member_index());
        let (me, now) = (MEMBERS[who as usize], self.now());
        self.posts += 1;
        tc.note(&format!("t={now}: {me} posts (renews its live claims)"));
        self.gw
            .post(&principal(who), ROOM, &format!("note {}", self.posts))
            .unwrap();
        let was_parked = std::mem::replace(&mut self.parked[who as usize], false);
        self.held_until[who as usize] = 0;
        for c in self
            .model
            .iter_mut()
            .filter(|c| c.owner == me && c.expires > now)
        {
            c.expires = c.expires.max(now + c.ttl);
        }
        if was_parked {
            for c in self.model.iter_mut().filter(|c| c.owner == me) {
                // a normal lease, but never below the claim's own ttl from now (#649)
                let cut = now + klatch::domain::CLAIM_DEFAULT_SECS.max(c.ttl);
                c.expires = c.expires.min(cut);
            }
        }
    }

    /// A `parked:` post: the member renews as for any post, then every live claim it holds is held until it is due
    /// back (a claim already past it keeps its own time), and what an earlier park stretched past the new return time is
    /// cut back to it.
    #[rule]
    fn park(&mut self, tc: TestCase) {
        let who = tc.draw(member_index());
        let hours = [1, 3][tc.draw(gs::integers::<u8>().max_value(1)) as usize];
        let (me, now) = (MEMBERS[who as usize], self.now());
        self.posts += 1;
        tc.note(&format!("t={now}: {me} parks for {hours}h"));
        self.gw
            .post(
                &principal(who),
                ROOM,
                &format!("parked: waiting {}, back by {hours}h", self.posts),
            )
            .unwrap();
        self.parked[who as usize] = true;
        let back_by = now + hours * 3600;
        let held = std::mem::replace(&mut self.held_until[who as usize], back_by);
        for c in self
            .model
            .iter_mut()
            .filter(|c| c.owner == me && c.expires > now)
        {
            // renewal first; short of the return time: stretched to it; the stretch of an earlier park (beyond the
            // new return time or the claim's own ttl from now, within the earlier return time): cut back to that
            c.expires = parked_expiry(c.expires, c.ttl, (now, back_by, held));
        }
    }

    #[rule]
    fn time_passes(&mut self, tc: TestCase) {
        let by = STEPS[tc.draw(gs::integers::<u8>().max_value(STEPS.len() as u8 - 1)) as usize];
        tc.note(&format!("{by}s pass"));
        self.now.fetch_add(by, Ordering::SeqCst);
    }

    #[invariant]
    fn the_listing_is_exactly_the_live_claims_and_none_overlap(&mut self, _: TestCase) {
        let now = self.now();
        let listed = self.gw.claims(&principal(0), ROOM).unwrap();
        let mut got: Vec<(String, String, i64)> = listed
            .iter()
            .map(|c| (c.path.clone(), c.owner.clone(), c.expires))
            .collect();
        got.sort();
        let mut want: Vec<(String, String, i64)> = self
            .model
            .iter()
            .filter(|c| c.expires > now)
            .map(|c| (c.path.clone(), c.owner.clone(), c.expires))
            .collect();
        want.sort();
        assert_eq!(got, want, "at t={now} the listing differs from the model");
        // no lease is ever longer than the cap, whatever renewed it
        for c in &listed {
            assert!(
                c.expires - now <= klatch::domain::CLAIM_MAX_SECS,
                "{c:?} outlives the cap at t={now}"
            );
        }
        // the safety property, checked on what the gateway lists and not on the model: two members never hold
        // overlapping paths at the same time
        for a in &listed {
            for b in &listed {
                assert!(
                    a.owner == b.owner || !related(&a.path, &b.path),
                    "{a:?} and {b:?} overlap"
                );
            }
        }
    }
}

#[hegel::test]
fn claims_agree_with_the_model_under_any_sequence_of_claims_releases_and_time(tc: TestCase) {
    hegel::stateful::machine(ClaimsRoom::new()).run(tc);
}

// ---- read cursors ----------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
struct Said {
    seq: i64,
    sender: usize,
    body: String,
}

struct Cursors {
    gw: Gateway<SqliteStore>,
    messages: Vec<Said>,
    cursor: [i64; 3],
    delivered: HashSet<(usize, i64)>,
    posted: HashMap<usize, usize>,
}

impl Cursors {
    fn new() -> Self {
        let (gw, _) = gateway();
        Self {
            gw,
            messages: Vec::new(),
            cursor: [0; 3],
            delivered: HashSet::new(),
            posted: HashMap::new(),
        }
    }
    /// What a read by `who` must return: everyone else's messages beyond the cursor, oldest first.
    fn expected(&self, who: usize) -> Vec<(i64, String, String)> {
        self.messages
            .iter()
            .filter(|m| m.seq > self.cursor[who] && m.sender != who)
            .map(|m| (m.seq, MEMBERS[m.sender].to_string(), m.body.clone()))
            .collect()
    }
}

#[hegel::state_machine]
impl Cursors {
    #[rule]
    fn post(&mut self, tc: TestCase) {
        let who = tc.draw(member_index()) as usize;
        let n = self.posted.entry(who).or_insert(0);
        *n += 1;
        let body = format!("{}-{n}", MEMBERS[who]);
        tc.note(&format!("{} posts {body}", MEMBERS[who]));
        let posted = self.gw.post(&principal(who as u8), ROOM, &body).unwrap();
        let seq = self.messages.len() as i64 + 1;
        assert_eq!(posted.seq, seq, "the room numbers its messages 1, 2, 3 ...");
        self.messages.push(Said {
            seq,
            sender: who,
            body,
        });
    }

    #[rule]
    fn read(&mut self, tc: TestCase) {
        let who = tc.draw(member_index()) as usize;
        let advance = tc.draw(gs::booleans());
        tc.note(&format!("{} reads (advance: {advance})", MEMBERS[who]));
        let want = self.expected(who);
        let got: Vec<(i64, String, String)> = self
            .gw
            .read(&principal(who as u8), ROOM, advance)
            .unwrap()
            .into_iter()
            .map(|m| (m.seq, m.sender, m.body))
            .collect();
        assert_eq!(got, want, "{} read the wrong messages", MEMBERS[who]);
        if advance {
            for (seq, _, _) in &got {
                assert!(
                    self.delivered.insert((who, *seq)),
                    "message {seq} was delivered to {} twice",
                    MEMBERS[who]
                );
            }
            if let Some(last) = got.last() {
                self.cursor[who] = self.cursor[who].max(last.0);
            }
        }
    }

    #[rule]
    fn ack(&mut self, tc: TestCase) {
        let who = tc.draw(member_index()) as usize;
        // sometimes beyond the room's head, which must not skip messages that have not arrived yet
        let seq = tc.draw(
            gs::integers::<i64>()
                .min_value(0)
                .max_value(self.messages.len() as i64 + 3),
        );
        tc.note(&format!("{} acknowledges up to {seq}", MEMBERS[who]));
        self.gw.ack(&principal(who as u8), ROOM, seq).unwrap();
        let head = self.messages.len() as i64;
        self.cursor[who] = self.cursor[who].max(seq.min(head));
    }

    #[invariant]
    fn a_read_that_moves_nothing_shows_exactly_what_the_model_says_is_unread(
        &mut self,
        _: TestCase,
    ) {
        for (who, name) in MEMBERS.iter().enumerate() {
            let got: Vec<(i64, String, String)> = self
                .gw
                .read(&principal(who as u8), ROOM, false)
                .unwrap()
                .into_iter()
                .map(|m| (m.seq, m.sender, m.body))
                .collect();
            assert_eq!(
                got,
                self.expected(who),
                "{name} would see the wrong unread messages"
            );
        }
    }
}

#[hegel::test]
fn read_cursors_agree_with_the_model_under_any_sequence_of_posts_reads_and_acknowledgements(
    tc: TestCase,
) {
    hegel::stateful::machine(Cursors::new()).run(tc);
}

// ---- roles -----------------------------------------------------------------------------------------------------

const ROLE_RULES: [&str; 4] = ["reviewer", "read-only", "admin-scope", "floors"];

/// A members' gateway with a room, and `sub` holding `role`; the human (no role, on the terminal gateway) grants it.
fn role_gateway(role: Held4) -> (Gateway<SqliteStore>, Arc<AtomicI64>) {
    let now = Arc::new(AtomicI64::new(1_000));
    let gw = Gateway::new(
        SqliteStore::open_in_memory().unwrap(),
        TestClock(now.clone()),
        AllowAll,
    );
    let human = Principal::unattributed("human");
    gw.join(&Principal::unattributed("sub"), ROOM).unwrap();
    gw.join(&Principal::unattributed("other"), ROOM).unwrap();
    gw.post(&Principal::unattributed("other"), ROOM, "hello")
        .unwrap();
    match role {
        Held4::Nobody => {}
        Held4::Librarian => gw.admin_set_librarian(&human, "sub", true).unwrap(),
        Held4::Admin => gw.admin_set_admin_agent(&human, "sub", true).unwrap(),
        Held4::Reviewer => gw.admin_set_reviewer(&human, "sub", true).unwrap(),
    }
    (gw.for_members(), now)
}

/// Run one attempt as `sub`; the rule if the role layer refused it, None if it did not.
fn attempt_refused_by(gw: &Gateway<SqliteStore>, attempt: &Try) -> Option<String> {
    let sub = Principal::unattributed("sub");
    let outcome: Result<(), Error> = match attempt {
        Try::Join => gw.join(&sub, ROOM),
        Try::Post => gw.post(&sub, ROOM, "text").map(|_| ()),
        Try::ReadAdvance => gw.read(&sub, ROOM, true).map(|_| ()),
        Try::ReadPeek => gw.read(&sub, ROOM, false).map(|_| ()),
        Try::Who => gw.who(&sub, ROOM).map(|_| ()),
        Try::Claim => gw.claim(&sub, ROOM, &ClaimSpec::new("p", 10, "")),
        Try::Release => gw.release(&sub, ROOM, "p"),
        Try::Claims => gw.claims(&sub, ROOM).map(|_| ()),
        Try::Tail => gw.tail(&sub, ROOM, (0, 10)).map(|_| ()),
        Try::Head => gw.head(&sub, ROOM).map(|_| ()),
        Try::History => gw
            .history(&sub, ROOM, klatch::gateway::HistoryRange::default())
            .map(|_| ()),
        Try::Status => gw.set_status(&sub, ROOM, "busy"),
        Try::Ack => gw.ack(&sub, ROOM, 1),
        Try::Mute => gw.set_muted(&sub, ROOM, true),
        Try::ReviewNext => gw.review_next(&sub).map(|_| ()),
        Try::ReviewVerdict => gw.review_verdict(&sub, 1, &klatch::domain::Verdict::Allow),
        Try::Admin(action) => gw.admin_note(&sub, (action, "x"), "test"),
    };
    match outcome {
        Err(Error::Refused { rule, .. }) if ROLE_RULES.contains(&rule.as_str()) => Some(rule),
        _ => None,
    }
}

/// Exhaustive, not sampled: every role against every operation, on a fresh gateway each (a refusal feeds the
/// breaker, so attempts must not share one). The gateway's decision must equal the spec's.
#[test]
fn the_role_layer_decides_every_operation_for_every_role_exactly_as_the_spec_table_says() {
    for role in ROLES {
        for attempt in every_try() {
            let (gw, _) = role_gateway(role);
            let got = attempt_refused_by(&gw, &attempt);
            let want = role_spec(role, &attempt);
            assert_eq!(
                got.as_deref(),
                want,
                "{role:?} attempting {attempt:?}: gateway says {got:?}, spec says {want:?}"
            );
        }
    }
}

/// No operation, by anyone, turns a member into a role-holder or one role-holder into another: the only way to a
/// role is the human's terminal gateway. A random sequence of attempts, role-granting calls included, from every
/// kind of member; the member's role never changes.
#[hegel::test]
fn no_sequence_of_operations_by_a_member_ever_changes_its_own_or_another_members_role(
    tc: TestCase,
) {
    let role = ROLES[tc.draw(gs::integers::<u8>().max_value(3)) as usize];
    let (gw, now) = role_gateway(role);
    let sub = Principal::unattributed("sub");
    let others = Principal::unattributed("other");
    let tries = every_try();
    let steps = tc.draw(gs::integers::<u8>().max_value(12));
    for _ in 0..steps {
        now.fetch_add(400, Ordering::SeqCst); // past the breaker window, so a refusal never silences the next try
        let choice = tc.draw(gs::integers::<u8>().max_value(tries.len() as u8 + 5)) as usize;
        let target = ["sub", "other"][tc.draw(gs::integers::<u8>().max_value(1)) as usize];
        let on = tc.draw(gs::booleans());
        tc.note(&format!("choice {choice} target {target} on {on}"));
        let _ = match choice.checked_sub(tries.len()) {
            None => {
                let _ = attempt_refused_by(&gw, &tries[choice]);
                Ok(())
            }
            Some(0) => gw.admin_set_librarian(&sub, target, on),
            Some(1) => gw.admin_set_admin_agent(&sub, target, on),
            Some(2) => gw.admin_set_reviewer(&sub, target, on),
            Some(3) => gw.admin_set_librarian(&others, "sub", on),
            Some(_) => gw.admin_set_admin_agent(&others, target, on),
        };
        let held = |m: &str| format!("{:?}", gw.role_of(m).unwrap());
        let want_sub = match role {
            Held4::Nobody => "None",
            Held4::Librarian => "Some(Librarian)",
            Held4::Admin => "Some(Admin)",
            Held4::Reviewer => "Some(Reviewer)",
        };
        assert_eq!(held("sub"), want_sub, "sub's role changed from {role:?}");
        assert_eq!(held("other"), "None", "other gained a role");
    }
}
