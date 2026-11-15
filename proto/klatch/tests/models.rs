//! Model-based property tests (epic #396, #393): a random sequence of operations is run against the real gateway and
//! an in-memory store, and against a tiny model written separately and in the plainest way. After every step the two
//! must agree. They are evidence, not proof: a pass means no disagreement was found in the sequences tried.
//!
//! Run alone: `cargo test -p klatch --test models`; more sequences: `HEGEL_TEST_CASES=5000 cargo test -p klatch
//! --test models`. A failure prints the shortest failing sequence (each rule notes what it did) and a replay blob
//! (see CONTRIBUTING.md, "Property tests").

use hegel::TestCase;
use hegel::generators as gs;
use klatch::domain::{Error, Principal};
use klatch::gateway::{AllowAll, ClaimSpec, Clock, Gateway};
use klatch::store::SqliteStore;
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
const TTLS: [i64; 3] = [10, 60, 600];
const STEPS: [i64; 4] = [1, 9, 10, 60];

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
}

struct ClaimsRoom {
    gw: Gateway<SqliteStore>,
    now: Arc<AtomicI64>,
    /// Every claim row the store holds, expired or not: expired rows are purged only when a claim is attempted.
    model: Vec<Held>,
}

impl ClaimsRoom {
    fn new() -> Self {
        let (gw, now) = gateway();
        Self {
            gw,
            now,
            model: Vec::new(),
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
