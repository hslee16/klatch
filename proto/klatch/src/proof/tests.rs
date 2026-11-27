//! The proof flow against a real store: what is asked, what is charged, what is audited, and what is never kept.

use super::*;
use crate::domain::Attribution;
use crate::store::SqliteStore;
use crate::store::areas::PolicyStore;
use std::collections::VecDeque;

pub(super) const PASS: &str = "correct horse battery";
const CHEAP: Cost = Cost {
    m_kib: 64,
    t: 1,
    p: 1,
};

/// What a person would type, in order; running out is a read that stopped.
pub(super) struct Typed {
    lines: VecDeque<Result<String, ProofError>>,
    pub(super) asked: usize,
    tty: &'static str,
}

impl Typed {
    pub(super) fn new(lines: &[&str]) -> Self {
        Self {
            lines: lines.iter().map(|l| Ok(l.to_string())).collect(),
            asked: 0,
            tty: "ttys001/s1",
        }
    }
    pub(super) fn on(mut self, tty: &'static str) -> Self {
        self.tty = tty;
        self
    }
    fn nobody() -> Self {
        Self {
            lines: VecDeque::from([Err(ProofError::NoTerminal)]),
            asked: 0,
            tty: "ttys001/s1",
        }
    }
}

impl Prompt for Typed {
    fn terminal(&self) -> String {
        self.tty.to_string()
    }
    fn secret(&mut self, _label: &str) -> Result<Zeroizing<String>, ProofError> {
        self.asked += 1;
        self.lines
            .pop_front()
            .unwrap_or(Err(ProofError::Aborted))
            .map(Zeroizing::new)
    }
}

pub(super) fn store() -> SqliteStore {
    SqliteStore::open_in_memory().unwrap()
}

pub(super) fn with_pass(s: &SqliteStore) {
    store_verifier(s, &Verifier::create(PASS, CHEAP).unwrap(), 1).unwrap();
}

pub(super) fn at(unix: i64) -> At {
    At {
        unix,
        mono: Instant::now(),
    }
}

pub(super) fn actions(s: &SqliteStore) -> Vec<String> {
    s.audit_log(100)
        .unwrap()
        .into_iter()
        .map(|a| a.action)
        .collect()
}

/// Every byte the store holds about the passphrase flow: the audit rows (all fields) and every policy entry.
pub(super) fn everything_kept(s: &SqliteStore) -> String {
    let mut all = format!("{:?}", s.audit_log(1000).unwrap());
    for (k, v) in s.policy_list("").unwrap() {
        if k != VERIFIER_KEY {
            all.push_str(&format!("{k}={v};"));
        }
    }
    all
}

#[test]
fn nothing_is_asked_and_nothing_let_in_while_no_passphrase_is_set() {
    let s = store();
    let mut p = Typed::new(&[PASS]);
    let r = prove(&s, &mut p, at(10), &Unlock::new(), "a test");
    assert_eq!(r.unwrap_err(), ProofError::NotSet);
    assert_eq!(p.asked, 0);
}

#[test]
fn the_right_passphrase_builds_a_proof_that_is_the_verified_human_and_is_audited() {
    let s = store();
    store_verifier(&s, &Verifier::create(PASS, CURRENT_COST).unwrap(), 1).unwrap();
    let proof = prove(
        &s,
        &mut Typed::new(&[PASS]),
        at(10),
        &Unlock::new(),
        "a test",
    )
    .unwrap();
    let who = proof.principal();
    assert_eq!(who.member, "human");
    assert_eq!(who.attribution, Attribution::Verified);
    assert_eq!(actions(&s), vec!["proof-ok"]);
    let row = &s.audit_log(1).unwrap()[0];
    assert_eq!(row.detail, "window 60s");
}

#[test]
fn a_wrong_or_empty_passphrase_is_refused_charged_audited_and_never_kept() {
    let s = store();
    with_pass(&s);
    for typed in ["wrong passphrase", "", "correct horse batterY"] {
        let r = prove(
            &s,
            &mut Typed::new(&[typed]),
            at(10),
            &Unlock::new(),
            "a test",
        );
        assert_eq!(r.unwrap_err(), ProofError::Wrong, "{typed:?}");
    }
    assert_eq!(
        actions(&s),
        vec!["proof-failed", "proof-failed", "proof-failed"]
    );
    let kept = everything_kept(&s);
    assert!(kept.contains("failures=3"), "{kept}");
    for secret in ["wrong passphrase", "correct horse batterY", PASS] {
        assert!(!kept.contains(secret), "{secret} leaked into {kept}");
    }
}

#[test]
fn no_terminal_is_a_refusal_that_charges_nothing_and_is_audited_at_most_once_a_minute() {
    let s = store();
    with_pass(&s);
    for t in 0..20 {
        let r = prove(
            &s,
            &mut Typed::nobody(),
            at(1000 + t),
            &Unlock::new(),
            "a test",
        );
        assert_eq!(r.unwrap_err(), ProofError::NoTerminal);
    }
    assert_eq!(
        actions(&s),
        vec!["proof-refused"],
        "one row for twenty tries"
    );
    assert_eq!(s.policy_get(limit::KEY).unwrap(), None, "nothing charged");
    prove(&s, &mut Typed::nobody(), at(1100), &Unlock::new(), "a test").unwrap_err();
    assert_eq!(actions(&s).len(), 2, "a new minute, a new row");
}

#[test]
fn after_three_wrong_ones_even_the_right_passphrase_waits_and_nothing_is_asked() {
    let s = store();
    with_pass(&s);
    for _ in 0..limit::FREE {
        prove(
            &s,
            &mut Typed::new(&["nope nope nope"]),
            at(100),
            &Unlock::new(),
            "a test",
        )
        .unwrap_err();
    }
    let mut p = Typed::new(&[PASS]);
    let e = prove(&s, &mut p, at(101), &Unlock::new(), "a test").unwrap_err();
    assert!(
        matches!(e, ProofError::Locked(w) if w > 0 && w <= 30),
        "{e:?}"
    );
    assert_eq!(p.asked, 0, "not even asked while locked");
    assert!(actions(&s).contains(&"proof-locked".to_string()));
    // the wait over, the right passphrase is accepted and the count is forgotten
    prove(
        &s,
        &mut Typed::new(&[PASS]),
        at(131),
        &Unlock::new(),
        "a test",
    )
    .unwrap();
    assert_eq!(s.policy_get(limit::KEY).unwrap(), None);
}

#[test]
fn each_further_failure_waits_longer_and_the_wait_has_a_cap() {
    let s = store();
    with_pass(&s);
    let mut now = 0;
    let mut waits = Vec::new();
    for _ in 0..8 {
        prove(
            &s,
            &mut Typed::new(&["nope nope nope"]),
            at(now),
            &Unlock::new(),
            "a test",
        )
        .unwrap_err();
        let w = limit::wait_secs(&s, "ttys001/s1", now).unwrap();
        waits.push(w);
        now += w;
    }
    assert_eq!(waits, vec![0, 0, 30, 60, 120, 240, 480, 900]);
}

#[test]
fn a_damaged_failure_count_reads_as_locked_not_as_free() {
    let s = store();
    with_pass(&s);
    s.reserved_policy_set(&ProofCap::new(), limit::KEY, Some("garbage"), 1)
        .unwrap();
    let e = prove(
        &s,
        &mut Typed::new(&[PASS]),
        at(50),
        &Unlock::new(),
        "a test",
    )
    .unwrap_err();
    assert!(matches!(e, ProofError::Locked(_)), "{e:?}");
}

#[test]
fn a_damaged_verifier_lets_nobody_in() {
    let s = store();
    s.reserved_policy_set(
        &ProofCap::new(),
        VERIFIER_KEY,
        Some("argon2id$m=1,t=1,p=1$00$00"),
        1,
    )
    .unwrap();
    let mut p = Typed::new(&[PASS]);
    assert_eq!(
        prove(&s, &mut p, at(1), &Unlock::new(), "a test").unwrap_err(),
        ProofError::Corrupt
    );
    assert_eq!(p.asked, 0);
}

#[test]
fn a_second_proof_inside_the_window_does_not_ask_and_one_after_it_does() {
    let s = store();
    with_pass(&s);
    let unlock = Unlock::new();
    let t0 = Instant::now();
    let when = |secs: u64, unix| At {
        unix,
        mono: t0 + Duration::from_secs(secs),
    };
    prove(&s, &mut Typed::new(&[PASS]), when(0, 10), &unlock, "a test").unwrap();
    let mut quiet = Typed::new(&[]);
    prove(&s, &mut quiet, when(59, 69), &unlock, "a test").unwrap();
    assert_eq!(quiet.asked, 0, "inside the window: no prompt");
    let mut again = Typed::new(&[PASS]);
    prove(&s, &mut again, when(60, 70), &unlock, "a test").unwrap();
    assert_eq!(again.asked, 1, "after the window: asked");
}

#[test]
fn a_sleep_that_the_monotonic_clock_does_not_count_still_closes_the_window() {
    let s = store();
    with_pass(&s);
    let unlock = Unlock::new();
    let t0 = Instant::now();
    let when = |mono_secs: u64, unix| At {
        unix,
        mono: t0 + Duration::from_secs(mono_secs),
    };
    prove(&s, &mut Typed::new(&[PASS]), when(0, 10), &unlock, "a test").unwrap();
    // the machine slept: 10 s on the monotonic clock, 1000 s on the wall clock
    let mut after_sleep = Typed::new(&[PASS]);
    prove(&s, &mut after_sleep, when(10, 1010), &unlock, "a test").unwrap();
    assert_eq!(
        after_sleep.asked, 1,
        "the wall clock passed the window: asked"
    );
    // a wall clock set back does not hold the window open past the monotonic clock
    let mut back = Typed::new(&[PASS]);
    prove(&s, &mut back, when(310, 5), &unlock, "a test").unwrap();
    assert_eq!(
        back.asked, 1,
        "the monotonic clock passed the window: asked"
    );
}

#[test]
fn another_process_does_not_share_the_unlock() {
    let s = store();
    with_pass(&s);
    let mine = Unlock::new();
    prove(&s, &mut Typed::new(&[PASS]), at(10), &mine, "a test").unwrap();
    // a second process has a window of its own, which starts closed
    let theirs = Unlock::new();
    let mut p = Typed::nobody();
    assert_eq!(
        prove(&s, &mut p, at(11), &theirs, "a test").unwrap_err(),
        ProofError::NoTerminal
    );
    assert_eq!(p.asked, 1);
}

#[test]
fn an_unlock_earned_before_a_change_of_passphrase_is_void() {
    let s = store();
    with_pass(&s);
    let unlock = Unlock::new();
    prove(&s, &mut Typed::new(&[PASS]), at(10), &unlock, "a test").unwrap();
    // the passphrase is replaced behind the open window (by any route)
    store_verifier(
        &s,
        &Verifier::create("another long passphrase", CHEAP).unwrap(),
        11,
    )
    .unwrap();
    let mut p = Typed::nobody();
    assert_eq!(
        prove(&s, &mut p, at(12), &unlock, "a test").unwrap_err(),
        ProofError::NoTerminal,
        "the old unlock must not replay"
    );
}

#[test]
fn a_length_of_the_window_comes_from_policy_and_is_bounded() {
    let s = store();
    with_pass(&s);
    assert_eq!(window_len(&s), Duration::from_secs(60));
    assert_eq!(DEFAULT_WINDOW_SECS, 60);
    let proof = prove(
        &s,
        &mut Typed::new(&[PASS]),
        at(1),
        &Unlock::new(),
        "a test",
    )
    .unwrap();
    set_window(&s, &proof, Some(120), 2).unwrap();
    assert_eq!(window_len(&s), Duration::from_secs(120));
    assert!(set_window(&s, &proof, Some(5), 3).is_err());
    assert!(set_window(&s, &proof, Some(100_000), 3).is_err());
    s.reserved_policy_set(&ProofCap::new(), WINDOW_KEY, Some("junk"), 4)
        .unwrap();
    assert_eq!(window_len(&s), Duration::from_secs(60));
    set_window(&s, &proof, None, 5).unwrap();
    assert_eq!(s.policy_get(WINDOW_KEY).unwrap(), None);
}

#[test]
fn a_verifier_of_weaker_cost_is_rewritten_at_the_next_success() {
    let s = store();
    with_pass(&s);
    let before = s.policy_get(VERIFIER_KEY).unwrap().unwrap();
    prove(
        &s,
        &mut Typed::new(&[PASS]),
        at(10),
        &Unlock::new(),
        "a test",
    )
    .unwrap();
    let after = s.policy_get(VERIFIER_KEY).unwrap().unwrap();
    assert_ne!(before, after);
    assert!(after.starts_with("argon2id$m=19456,t=2,p=1$"), "{after}");
    assert!(actions(&s).contains(&"passphrase-rehashed".to_string()));
    prove(
        &s,
        &mut Typed::new(&[PASS]),
        at(11),
        &Unlock::new(),
        "a test",
    )
    .unwrap();
}

/// A stored row that parses to a stale verifier but is not byte-identical to its canonical encoding
/// (parameters in another order, a leading zero).
fn non_canonical_stale_row(s: &SqliteStore) -> String {
    let v = Verifier::create(PASS, CHEAP).unwrap();
    let canon = v.encode();
    let (cost, rest) = canon
        .strip_prefix("argon2id$")
        .unwrap()
        .split_once('$')
        .unwrap();
    assert_eq!(cost, "m=64,t=1,p=1");
    let row = format!("argon2id$p=1,t=01,m=64${rest}");
    s.reserved_policy_set(&ProofCap::new(), VERIFIER_KEY, Some(&row), 1)
        .unwrap();
    row
}

#[test]
fn a_stale_row_that_is_not_its_canonical_text_proves_but_is_left_as_it_was() {
    let s = store();
    let row = non_canonical_stale_row(&s);
    let parsed = Verifier::parse(&row).unwrap();
    assert!(parsed.is_stale());
    assert_ne!(parsed.encode(), row);
    prove(
        &s,
        &mut Typed::new(&[PASS]),
        at(10),
        &Unlock::new(),
        "a test",
    )
    .unwrap();
    assert_eq!(
        s.policy_get(VERIFIER_KEY).unwrap().as_deref(),
        Some(row.as_str()),
        "a row that is not the canonical encoding was rewritten"
    );
    assert!(!actions(&s).contains(&"passphrase-rehashed".to_string()));
}

#[test]
fn a_row_with_upper_case_hex_is_corrupt_and_never_proves() {
    let s = store();
    with_pass(&s);
    let row = s.policy_get(VERIFIER_KEY).unwrap().unwrap();
    let (head, tail) = row.rsplit_once('$').unwrap();
    s.reserved_policy_set(
        &ProofCap::new(),
        VERIFIER_KEY,
        Some(&format!("{head}${}", tail.to_uppercase())),
        2,
    )
    .unwrap();
    let e = prove(
        &s,
        &mut Typed::new(&[PASS]),
        at(10),
        &Unlock::new(),
        "a test",
    )
    .unwrap_err();
    assert_eq!(e, ProofError::Corrupt);
}

#[test]
fn init_sets_the_first_passphrase_twice_typed_and_keeps_only_a_verifier() {
    let s = store();
    let unlock = Unlock::new();
    // three too short, or three mismatched, change nothing
    assert!(matches!(
        init(
            &s,
            &mut Typed::new(&["short", "short", "short"]),
            at(1),
            &unlock
        ),
        Err(ProofError::Rejected(_))
    ));
    assert!(matches!(
        init(
            &s,
            &mut Typed::new(&[
                PASS,
                "something else entirely",
                PASS,
                "something else entirely",
                PASS,
                "something else entirely"
            ]),
            at(2),
            &unlock
        ),
        Err(ProofError::Rejected(_))
    ));
    assert!(!is_set(&s).unwrap());
    init(&s, &mut Typed::new(&[PASS, PASS]), at(3), &unlock).unwrap();
    assert!(is_set(&s).unwrap());
    let stored = s.policy_get(VERIFIER_KEY).unwrap().unwrap();
    assert!(!stored.contains("horse"), "{stored}");
    assert_eq!(actions(&s), vec!["passphrase-set"]);
    assert!(!everything_kept(&s).contains(PASS));
    // a second init is refused: it cannot replace a passphrase without the old one
    assert!(matches!(
        init(&s, &mut Typed::new(&[PASS, PASS]), at(4), &unlock),
        Err(ProofError::Rejected(_))
    ));
}

#[test]
fn a_change_needs_the_old_passphrase_even_inside_the_window() {
    let s = store();
    with_pass(&s);
    let unlock = Unlock::new();
    prove(&s, &mut Typed::new(&[PASS]), at(1), &unlock, "a test").unwrap();
    // wrong old: nothing changes
    let before = s.policy_get(VERIFIER_KEY).unwrap();
    let e = change(
        &s,
        &mut Typed::new(&[
            "not the old one",
            "brand new passphrase",
            "brand new passphrase",
        ]),
        at(2),
        &unlock,
    )
    .unwrap_err();
    assert_eq!(e, ProofError::Wrong);
    assert_eq!(s.policy_get(VERIFIER_KEY).unwrap(), before);
    // right old: changed, and only the new one proves from then on
    change(
        &s,
        &mut Typed::new(&[PASS, "brand new passphrase", "brand new passphrase"]),
        at(3),
        &unlock,
    )
    .unwrap();
    assert!(actions(&s).contains(&"passphrase-changed".to_string()));
    assert_eq!(
        prove(
            &s,
            &mut Typed::new(&[PASS]),
            at(4),
            &Unlock::new(),
            "a test"
        )
        .unwrap_err(),
        ProofError::Wrong
    );
    prove(
        &s,
        &mut Typed::new(&["brand new passphrase"]),
        at(5),
        &Unlock::new(),
        "a test",
    )
    .unwrap();
}

#[test]
fn the_recovery_code_resets_the_passphrase_and_a_wrong_code_is_charged_like_a_wrong_passphrase() {
    let s = store();
    with_pass(&s);
    let code = "KLATCH-ABCD EFGH";
    let unlock = Unlock::new();
    assert_eq!(
        reset(
            &s,
            code,
            &mut Typed::new(&["KLATCH-ZZZZ EFGH"]),
            at(1),
            &unlock
        )
        .unwrap_err(),
        ProofError::Wrong
    );
    assert_eq!(
        s.policy_get(limit::KEY).unwrap().as_deref(),
        Some("ttys001/s1=1,1")
    );
    // spacing and case do not matter; the new passphrase is asked twice
    reset(
        &s,
        code,
        &mut Typed::new(&["klatch-abcd   efgh", "after the reset", "after the reset"]),
        at(2),
        &unlock,
    )
    .unwrap();
    assert_eq!(s.policy_get(limit::KEY).unwrap(), None);
    prove(
        &s,
        &mut Typed::new(&["after the reset"]),
        at(3),
        &Unlock::new(),
        "a test",
    )
    .unwrap();
    assert!(actions(&s).contains(&"passphrase-reset".to_string()));
    assert!(!everything_kept(&s).contains("ABCD"));
}

#[test]
fn guesses_at_the_recovery_code_share_the_limit_with_the_passphrase() {
    let s = store();
    with_pass(&s);
    for _ in 0..limit::FREE {
        reset(
            &s,
            "KLATCH-A",
            &mut Typed::new(&["KLATCH-B"]),
            at(10),
            &Unlock::new(),
        )
        .unwrap_err();
    }
    let e = prove(
        &s,
        &mut Typed::new(&[PASS]),
        at(11),
        &Unlock::new(),
        "a test",
    )
    .unwrap_err();
    assert!(matches!(e, ProofError::Locked(_)), "{e:?}");
}

#[test]
fn attempts_made_at_once_from_many_processes_are_each_charged() {
    let dir = crate::store::test_dir::TestDir::new("proof-concurrent");
    let path = dir.join("rooms.db");
    let key = crate::store::insecure_test_key();
    {
        let s = SqliteStore::open(&path, &key).unwrap();
        with_pass(&s);
    }
    let charged: Vec<_> = (0..8)
        .map(|_| {
            let path = path.clone();
            std::thread::spawn(move || {
                let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
                limit::charge(&s, "ttys001/s1", 500).unwrap().is_ok()
            })
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|h| h.join().unwrap())
        .collect();
    assert_eq!(
        charged.iter().filter(|c| **c).count(),
        limit::FREE as usize,
        "eight at once, only the free three get through: {charged:?}"
    );
    let s = SqliteStore::open(&path, &key).unwrap();
    assert_eq!(
        s.policy_get(limit::KEY).unwrap().as_deref(),
        Some("ttys001/s1=3,500")
    );
}

/// Types the new passphrase twice, and in between another process sets one (the race `init` must not lose).
struct RacingPrompt<'a> {
    store: &'a SqliteStore,
    asked: usize,
}

impl Prompt for RacingPrompt<'_> {
    fn secret(&mut self, _label: &str) -> Result<Zeroizing<String>, ProofError> {
        self.asked += 1;
        if self.asked == 2 {
            store_verifier(
                self.store,
                &Verifier::create("the other process won", CHEAP).unwrap(),
                5,
            )
            .unwrap();
        }
        Ok(Zeroizing::new(PASS.to_string()))
    }
}

#[test]
fn init_does_not_overwrite_a_passphrase_set_while_it_was_asking() {
    let s = store();
    let mut p = RacingPrompt {
        store: &s,
        asked: 0,
    };
    let e = init(&s, &mut p, at(10), &Unlock::new()).unwrap_err();
    assert!(matches!(e, ProofError::Rejected(_)), "{e:?}");
    let kept = Verifier::parse(&s.policy_get(VERIFIER_KEY).unwrap().unwrap()).unwrap();
    assert!(
        kept.verify("the other process won"),
        "the first set was replaced"
    );
    assert!(!kept.verify(PASS));
    assert!(!actions(&s).contains(&"passphrase-set".to_string()));
}

/// Types `typed`, and while it is typing another process replaces the stored passphrase.
struct ChangingPrompt<'a> {
    store: &'a SqliteStore,
    typed: &'static str,
}

impl Prompt for ChangingPrompt<'_> {
    fn secret(&mut self, _label: &str) -> Result<Zeroizing<String>, ProofError> {
        store_verifier(
            self.store,
            &Verifier::create("changed meanwhile by someone", CHEAP).unwrap(),
            5,
        )
        .unwrap();
        Ok(Zeroizing::new(self.typed.to_string()))
    }
}

#[test]
fn a_passphrase_changed_while_asking_is_neither_overwritten_by_a_rehash_nor_proved_against_the_old_one()
 {
    let s = store();
    with_pass(&s); // a verifier of weaker cost, which a success would rewrite
    let mut p = ChangingPrompt {
        store: &s,
        typed: PASS,
    };
    let e = prove(&s, &mut p, at(10), &Unlock::new(), "a test").unwrap_err();
    assert_eq!(
        e,
        ProofError::Wrong,
        "the old passphrase proved against the new verifier"
    );
    let kept = Verifier::parse(&s.policy_get(VERIFIER_KEY).unwrap().unwrap()).unwrap();
    assert!(
        kept.verify("changed meanwhile by someone"),
        "the change was overwritten"
    );
    assert!(!actions(&s).contains(&"passphrase-rehashed".to_string()));
}

/// Records what the person is told between attempts.
struct Noted {
    typed: Typed,
    notes: Vec<String>,
}

impl Prompt for Noted {
    fn secret(&mut self, label: &str) -> Result<Zeroizing<String>, ProofError> {
        self.typed.secret(label)
    }
    fn notice(&mut self, text: &str) {
        self.notes.push(text.to_string());
    }
}

#[test]
fn init_asks_again_after_a_short_or_mismatched_entry_and_says_why() {
    let s = store();
    let mut p = Noted {
        typed: Typed::new(&["short", PASS, "not the same at all", PASS, PASS]),
        notes: vec![],
    };
    init(&s, &mut p, at(1), &Unlock::new()).unwrap();
    assert!(is_set(&s).unwrap());
    assert_eq!(p.notes.len(), 2, "{:?}", p.notes);
    assert!(p.notes[0].contains("at least"), "{:?}", p.notes);
    assert!(p.notes[1].contains("differ"), "{:?}", p.notes);
    assert!(p.notes.iter().all(|n| !n.contains(PASS)));
    assert_eq!(actions(&s), vec!["passphrase-set"]);
}

#[test]
fn two_short_entries_then_a_good_pair_succeeds_and_three_bad_attempts_stop_with_nothing_written() {
    let s = store();
    init(
        &s,
        &mut Typed::new(&["short", "tiny", PASS, PASS]),
        at(1),
        &Unlock::new(),
    )
    .unwrap();
    let s = store();
    let mut p = Typed::new(&["a", "b", "c", PASS, PASS]);
    let e = init(&s, &mut p, at(1), &Unlock::new()).unwrap_err();
    assert!(matches!(e, ProofError::Rejected(_)), "{e:?}");
    assert_eq!(p.asked, NEW_ATTEMPTS, "the cap is the constant");
    assert!(!is_set(&s).unwrap());
    assert!(actions(&s).is_empty());
}

#[test]
fn retries_for_a_new_passphrase_are_not_lockout_failures() {
    let s = store();
    with_pass(&s);
    let unlock = Unlock::new();
    // change: old proved once, then two bad new ones and a good one
    change(
        &s,
        &mut Typed::new(&[
            PASS,
            "short",
            "tiny",
            "brand new passphrase",
            "brand new passphrase",
        ]),
        at(1),
        &unlock,
    )
    .unwrap();
    assert_eq!(s.policy_get(limit::KEY).unwrap(), None);
    // three bad new ones: rejected, and the failure count is still clear
    let e = change(
        &s,
        &mut Typed::new(&["brand new passphrase", "x", "y", "z"]),
        at(2),
        &unlock,
    )
    .unwrap_err();
    assert!(matches!(e, ProofError::Rejected(_)), "{e:?}");
    assert_eq!(s.policy_get(limit::KEY).unwrap(), None);
    // reset: the code is charged once and cleared on success; retries add nothing
    reset(
        &s,
        "KLATCH-ABCD EFGH",
        &mut Typed::new(&[
            "KLATCH-ABCD EFGH",
            "a",
            "b",
            "after the reset",
            "after the reset",
        ]),
        at(3),
        &unlock,
    )
    .unwrap();
    assert_eq!(s.policy_get(limit::KEY).unwrap(), None);
}

#[test]
fn a_second_terminal_failing_three_times_does_not_lock_the_first() {
    let s = store();
    with_pass(&s);
    for _ in 0..limit::FREE {
        prove(
            &s,
            &mut Typed::new(&["nope nope nope"]).on("ttys009/s9"),
            at(100),
            &Unlock::new(),
            "a test",
        )
        .unwrap_err();
    }
    // the second terminal is locked
    let mut other = Typed::new(&[PASS]).on("ttys009/s9");
    let e = prove(&s, &mut other, at(101), &Unlock::new(), "a test").unwrap_err();
    assert!(matches!(e, ProofError::Locked(w) if w > 0), "{e:?}");
    assert_eq!(other.asked, 0);
    // the first terminal is still free, and its success clears only its own count
    prove(
        &s,
        &mut Typed::new(&["nope nope nope"]),
        at(102),
        &Unlock::new(),
        "a test",
    )
    .unwrap_err();
    prove(
        &s,
        &mut Typed::new(&[PASS]),
        at(103),
        &Unlock::new(),
        "a test",
    )
    .unwrap();
    let e = prove(
        &s,
        &mut Typed::new(&[PASS]).on("ttys009/s9"),
        at(104),
        &Unlock::new(),
        "a test",
    )
    .unwrap_err();
    assert!(
        matches!(e, ProofError::Locked(_)),
        "a success on one terminal must not clear another's count: {e:?}"
    );
    let kept = everything_kept(&s);
    assert!(kept.contains("failures=3"), "{kept}");
}

#[test]
fn old_terminals_are_pruned_and_the_table_stays_small() {
    let s = store();
    for i in 0..100 {
        limit::charge(&s, &format!("pts{i}/s{i}"), 1000 + i)
            .unwrap()
            .unwrap();
    }
    let row = s.policy_get(limit::KEY).unwrap().unwrap();
    assert_eq!(row.split(';').count(), 64, "capped, newest kept");
    assert!(row.contains("pts99/s99=1,1099") && !row.contains("pts0/s0="));
    // a terminal idle for a week is forgotten the next time anyone fails
    limit::charge(&s, "late/s1", 1099 + 8 * 24 * 3600)
        .unwrap()
        .unwrap();
    assert_eq!(
        s.policy_get(limit::KEY).unwrap().as_deref(),
        Some(format!("late/s1=1,{}", 1099 + 8 * 24 * 3600).as_str())
    );
}

#[test]
fn a_row_from_before_per_terminal_counting_is_still_honoured() {
    let s = store();
    s.reserved_policy_set(&ProofCap::new(), limit::KEY, Some("5,100"), 1)
        .unwrap();
    assert!(limit::wait_secs(&s, "ttys001/s1", 101).unwrap() > 0);
    limit::clear(&s, "ttys001/s1", 101).unwrap();
    assert_eq!(s.policy_get(limit::KEY).unwrap(), None);
}

#[test]
fn the_audit_names_the_terminal_and_never_an_attempt() {
    let s = store();
    with_pass(&s);
    prove(
        &s,
        &mut Typed::new(&["hunter2 hunter2"]).on("ttys042/s7"),
        at(5),
        &Unlock::new(),
        "a test",
    )
    .unwrap_err();
    let kept = everything_kept(&s);
    assert!(kept.contains("failures=1 terminal=ttys042/s7"), "{kept}");
    assert!(!kept.contains("hunter2"));
}

/// Every policy key the proof owns (#1275): the verifier, the failure counter, the unlock window, and anything
/// that starts with the reserved prefix.
const OWNED: [&str; 4] = [
    VERIFIER_KEY,
    limit::KEY,
    WINDOW_KEY,
    "admin-passphrase-future",
];

#[test]
fn the_generic_policy_writer_refuses_every_key_the_proof_owns() {
    for key in OWNED {
        let s = store();
        with_pass(&s);
        let before = s.policy_list("").unwrap();
        assert!(s.policy_set(key, Some("planted"), 5).is_err(), "{key} set");
        assert!(s.policy_set(key, None, 6).is_err(), "{key} cleared");
        assert_eq!(s.policy_list("").unwrap(), before, "{key} left unchanged");
    }
}

#[test]
fn a_refused_write_to_an_owned_key_is_audited_by_key_name_only() {
    let s = store();
    with_pass(&s);
    let _ = s.policy_set(VERIFIER_KEY, Some("PLANTED-SECRET-VALUE"), 5);
    let log = s.audit_log(10).unwrap();
    let row = log
        .iter()
        .find(|a| a.action == "policy-reserved-refused")
        .expect("an audit row");
    assert_eq!(row.target, VERIFIER_KEY);
    assert!(!format!("{log:?}").contains("PLANTED-SECRET-VALUE"));
}

#[test]
fn the_proof_commands_still_write_their_own_rows() {
    let s = store();
    with_pass(&s);
    assert!(s.policy_get(VERIFIER_KEY).unwrap().is_some());
    s.reserved_policy_set(&ProofCap::new(), WINDOW_KEY, Some("60"), 7)
        .unwrap();
    assert_eq!(s.policy_get(WINDOW_KEY).unwrap().as_deref(), Some("60"));
}

#[test]
fn an_ordinary_key_is_not_caught_by_the_reserved_check() {
    let s = store();
    s.policy_set("admin:note", Some("x"), 1).unwrap();
    s.policy_set("label:admin-passphrase-room", Some("x"), 1)
        .unwrap();
    assert!(
        s.reserved_policy_set(&ProofCap::new(), "label:r", Some("x"), 1)
            .is_err()
    );
}

#[test]
fn the_refusal_is_a_refused_error_with_its_rule_and_never_the_value() {
    let s = store();
    match s.policy_set(VERIFIER_KEY, Some("PLANTED-SECRET-VALUE"), 5) {
        Err(crate::domain::Error::Refused { rule, reason }) => {
            assert_eq!(rule, "admin-passphrase-rows");
            assert!(!reason.contains("PLANTED-SECRET-VALUE"));
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn a_refusal_inside_a_transaction_is_audited_after_it_rolls_back() {
    for key in OWNED {
        let s = store();
        with_pass(&s);
        let before = s.policy_list("").unwrap();
        let r = s.atomically(&mut || s.policy_set(key, Some("PLANTED-SECRET-VALUE"), 5));
        assert!(r.is_err());
        let log = s.audit_log(10).unwrap();
        let rows: Vec<_> = log
            .iter()
            .filter(|a| a.action == "policy-reserved-refused" && a.target == key)
            .collect();
        assert_eq!(rows.len(), 1, "{key}: {log:?}");
        assert!(!format!("{log:?}").contains("PLANTED-SECRET-VALUE"));
        assert_eq!(s.policy_list("").unwrap(), before);
    }
}

#[test]
fn a_refusal_in_a_transaction_that_commits_anyway_is_audited_once() {
    let s = store();
    let r = s.atomically(&mut || {
        let _ = s.policy_set(VERIFIER_KEY, Some("x"), 5);
        Ok(())
    });
    assert!(r.is_ok());
    let n = s
        .audit_log(10)
        .unwrap()
        .iter()
        .filter(|a| a.action == "policy-reserved-refused")
        .count();
    assert_eq!(n, 1);
}

#[test]
fn a_refusal_then_an_ordinary_nested_write_then_an_error_keeps_exactly_one_audit_row() {
    let s = store();
    let r = s.atomically(&mut || {
        let _ = s.policy_set(VERIFIER_KEY, Some("x"), 5);
        s.policy_set("label:r", Some("secret"), 6)?;
        Err(crate::domain::Error::Invalid("stop".into()))
    });
    assert!(r.is_err());
    let log = s.audit_log(10).unwrap();
    let n = log
        .iter()
        .filter(|a| a.action == "policy-reserved-refused")
        .count();
    assert_eq!(n, 1, "{log:?}");
    assert_eq!(s.policy_get("label:r").unwrap(), None, "rolled back");
}

#[test]
fn flushed_refusals_are_written_once_and_never_again() {
    let s = store();
    s.atomically(&mut || {
        let _ = s.policy_set(VERIFIER_KEY, Some("x"), 5);
        let _ = s.policy_set(limit::KEY, Some("x"), 6);
        Ok(())
    })
    .unwrap();
    s.policy_set("label:a", Some("1"), 7).unwrap();
    s.atomically(&mut || s.policy_set("label:b", Some("2"), 8))
        .unwrap();
    let n = s
        .audit_log(20)
        .unwrap()
        .iter()
        .filter(|a| a.action == "policy-reserved-refused")
        .count();
    assert_eq!(n, 2);
}

#[test]
fn a_flushed_refusal_carries_the_time_of_the_refusal() {
    let s = store();
    let _ = s.atomically(&mut || {
        let _ = s.policy_set(VERIFIER_KEY, Some("x"), 4242);
        Err(crate::domain::Error::Invalid("stop".into()))
    });
    let log = s.audit_log(10).unwrap();
    let row = log
        .iter()
        .find(|a| a.action == "policy-reserved-refused")
        .unwrap();
    assert_eq!(row.ts, 4242);
}
