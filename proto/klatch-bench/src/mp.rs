//! The multi-process fault tester (#625, DESIGN item 66). Several real klatch processes post, claim, archive,
//! restore, back up and rotate the key against one store while some are killed with SIGKILL at random moments;
//! afterwards the store is reopened and checked. Run by hand (`cargo run -q -p klatch-bench --bin klatch-faults`),
//! never in CI. The seed fixes the plan (which operations, which are killed and after how long), not how the
//! operating system interleaves the processes, so a failing seed is a strong hint to rerun and not a recording.

use std::collections::{BTreeMap, BTreeSet};

/// A small seeded generator (splitmix64), so a printed seed replays the same plan.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number below `n` (`n` must not be 0).
    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }
}

/// What one process does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    /// `post` of a message whose body is unique, so it can be looked for afterwards.
    Post(String),
    Claim(u64),
    Archive,
    RestoreArchive,
    RotateKey,
    Backup,
    Read,
}

/// One planned process: what it does and, for some, how long after its start it is killed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub op: Op,
    pub kill_after_ms: Option<u64>,
}

/// The plan of one round: `ops` processes, drawn from the seed.
pub fn plan(seed: u64, round: usize, ops: usize) -> Vec<Step> {
    let warm = warm_up(ops);
    let mut rng = Rng::new(seed ^ ((round as u64 + 1).wrapping_mul(0xA24B_AED4_963E_E407)));
    (0..ops)
        .map(|i| {
            let op = match rng.below(100) {
                65..=84 if i < warm => Op::Read,
                0..=49 => Op::Post(format!("p{seed}-{round}-{i}")),
                50..=64 => Op::Claim(rng.below(6)),
                65..=76 => Op::Archive,
                77..=84 => Op::RestoreArchive,
                85..=89 => Op::RotateKey,
                90..=93 => Op::Backup,
                _ => Op::Read,
            };
            // a third of the processes are killed, from before they have opened anything to well into the work
            let kill_after_ms = (rng.below(100) < 35).then(|| rng.below(60));
            Step { op, kill_after_ms }
        })
        .collect()
}

/// How many processes run before the first archive may start: the posts of that stretch are the old messages the
/// archives then move, and a barrier follows them.
pub fn warm_up(ops: usize) -> usize {
    ops / 3
}

/// What the harness knows it did, to compare with what the store holds.
#[derive(Default)]
pub struct Ledger {
    /// Bodies of posts that exited successfully: the store owes every one of them.
    pub acked: BTreeSet<String>,
    /// Bodies of posts that were killed or failed: the store may hold one of them, once, or not at all.
    pub maybe: BTreeSet<String>,
    /// Bodies that were in the store before the round (the migrated fixture): each must survive once.
    pub seeded: Vec<String>,
}

/// What the reopened store holds.
#[derive(Default)]
pub struct Facts {
    /// Lines of `PRAGMA integrity_check` and `cipher_integrity_check` that are not `ok`.
    pub integrity: Vec<String>,
    /// The seq of every live row, per room.
    pub rooms: BTreeMap<String, Vec<i64>>,
    /// Every message body.
    pub bodies: Vec<String>,
    /// Rows still standing in for an archived message after every archive was restored.
    pub tombstones: usize,
}

/// Every way the store breaks what klatch promises; empty when it holds.
pub fn judge(facts: &Facts, ledger: &Ledger) -> Vec<String> {
    let mut out = Vec::new();
    for line in &facts.integrity {
        out.push(format!("integrity check failed: {line}"));
    }
    for (room, seqs) in &facts.rooms {
        if !is_dense(seqs) {
            out.push(format!(
                "seq of room {room} is not dense from 1: {}",
                head(seqs)
            ));
        }
    }
    if facts.tombstones > 0 {
        out.push(format!(
            "{} tombstone(s) remain after every archive was restored",
            facts.tombstones
        ));
    }
    let mut count: BTreeMap<&str, usize> = BTreeMap::new();
    for b in &facts.bodies {
        *count.entry(b.as_str()).or_default() += 1;
    }
    for (body, n) in &count {
        let known = ledger.acked.contains(*body)
            || ledger.maybe.contains(*body)
            || ledger.seeded.iter().any(|s| s == body);
        if *n > 1 {
            out.push(format!("{body:?} is stored {n} times (twice or more)"));
        } else if !known {
            out.push(format!("{body:?} is in the store and nobody posted it"));
        }
    }
    for body in &ledger.acked {
        if !count.contains_key(body.as_str()) {
            out.push(format!(
                "lost: {body:?} was acknowledged and is not in the store"
            ));
        }
    }
    for body in &ledger.seeded {
        if !count.contains_key(body.as_str()) {
            out.push(format!(
                "lost: {body:?} was in the store before the run and is gone"
            ));
        }
    }
    out
}

/// Seqs of one room are 1, 2, 3 ... with nothing missing and nothing repeated (tombstones keep theirs).
fn is_dense(seqs: &[i64]) -> bool {
    let mut sorted = seqs.to_vec();
    sorted.sort_unstable();
    sorted.iter().enumerate().all(|(i, s)| *s == i as i64 + 1)
}

fn head(seqs: &[i64]) -> String {
    let mut sorted = seqs.to_vec();
    sorted.sort_unstable();
    format!(
        "{:?} ({} rows)",
        &sorted[..sorted.len().min(12)],
        sorted.len()
    )
}

/// Names in the store directory that no finished process may leave behind: lock files, half-written copies.
pub fn stray_files(names: &[String]) -> Vec<String> {
    let allowed = |n: &str| match n.split_once('/') {
        None => n == "rooms.db" || n == "keys" || n == "archive" || is_backup(n),
        Some(("keys", f)) => f.starts_with("store-") && f.ends_with(".key"),
        Some(("archive", f)) => (f.ends_with(".db") || f.ends_with(".partial")) && !f.contains('/'),
        Some(_) => false,
    };
    names.iter().filter(|n| !allowed(n)).cloned().collect()
}

/// A backup the harness itself wrote.
fn is_backup(name: &str) -> bool {
    name.starts_with("bk-") && (name.ends_with(".db") || name.contains(".db.partial"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(x: &str) -> String {
        x.to_string()
    }

    fn ledger(acked: &[&str], maybe: &[&str]) -> Ledger {
        Ledger {
            acked: acked.iter().map(|b| s(b)).collect(),
            maybe: maybe.iter().map(|b| s(b)).collect(),
            seeded: vec![],
        }
    }

    fn facts(seqs: &[i64], bodies: &[&str]) -> Facts {
        Facts {
            integrity: vec![],
            rooms: BTreeMap::from([(s("r"), seqs.to_vec())]),
            bodies: bodies.iter().map(|b| s(b)).collect(),
            tombstones: 0,
        }
    }

    #[test]
    fn the_same_seed_draws_the_same_plan_and_another_seed_a_different_one() {
        assert_eq!(plan(7, 0, 40), plan(7, 0, 40));
        assert_ne!(plan(7, 0, 40), plan(8, 0, 40));
        assert_ne!(plan(7, 0, 40), plan(7, 1, 40));
        assert_eq!(plan(7, 0, 40).len(), 40);
    }

    #[test]
    fn a_plan_has_every_kind_of_operation_and_some_kills() {
        let p = plan(1, 0, 400);
        for want in [
            "Archive",
            "RestoreArchive",
            "RotateKey",
            "Backup",
            "Claim",
            "Post",
            "Read",
        ] {
            assert!(
                p.iter().any(|st| format!("{:?}", st.op).starts_with(want)),
                "no {want} in the plan"
            );
        }
        let kills = p.iter().filter(|st| st.kill_after_ms.is_some()).count();
        assert!(kills > 40 && kills < 200, "{kills} kills in 400 steps");
    }

    #[test]
    fn nothing_is_archived_or_restored_while_the_first_posts_are_still_arriving() {
        // a message old enough to archive that lands between the copy and the tombstone is tombstoned without being
        // archived (#634); the tester only ever has such messages before the first archive starts
        let p = plan(9, 0, 300);
        let warm = warm_up(300);
        assert_eq!(warm, 100);
        for st in &p[..warm] {
            assert!(!matches!(st.op, Op::Archive | Op::RestoreArchive), "{st:?}");
        }
        assert!(p[warm..].iter().any(|st| st.op == Op::Archive));
    }

    #[test]
    fn post_bodies_are_unique_within_a_round_and_across_rounds() {
        let mut seen = BTreeSet::new();
        for round in 0..3 {
            for st in plan(5, round, 200) {
                if let Op::Post(b) = st.op {
                    assert!(seen.insert(b.clone()), "{b} drawn twice");
                }
            }
        }
    }

    #[test]
    fn a_store_that_holds_everything_once_is_clean() {
        let l = ledger(&["a", "b"], &["c"]);
        assert_eq!(
            judge(&facts(&[1, 2, 3], &["a", "b", "c"]), &l),
            Vec::<String>::new()
        );
        assert_eq!(
            judge(&facts(&[1, 2], &["a", "b"]), &l),
            Vec::<String>::new()
        );
    }

    #[test]
    fn an_acknowledged_post_that_is_gone_is_lost() {
        let v = judge(&facts(&[1], &["a"]), &ledger(&["a", "b"], &[]));
        assert!(
            v.iter().any(|m| m.contains("lost") && m.contains('b')),
            "{v:?}"
        );
    }

    #[test]
    fn a_body_stored_twice_is_a_duplicate_even_when_its_post_was_killed() {
        let v = judge(&facts(&[1, 2], &["c", "c"]), &ledger(&[], &["c"]));
        assert!(
            v.iter().any(|m| m.contains("twice") && m.contains('c')),
            "{v:?}"
        );
    }

    #[test]
    fn a_body_nobody_posted_is_unexpected() {
        let v = judge(&facts(&[1], &["ghost"]), &ledger(&[], &[]));
        assert!(v.iter().any(|m| m.contains("ghost")), "{v:?}");
    }

    #[test]
    fn a_gap_or_a_repeat_in_seq_is_reported() {
        let l = ledger(&[], &[]);
        let gap = judge(&facts(&[1, 2, 4], &[]), &l);
        assert!(gap.iter().any(|m| m.contains("dense")), "{gap:?}");
        let repeat = judge(&facts(&[1, 2, 2], &[]), &l);
        assert!(repeat.iter().any(|m| m.contains("dense")), "{repeat:?}");
        let late = judge(&facts(&[2, 3], &[]), &l);
        assert!(late.iter().any(|m| m.contains("dense")), "{late:?}");
    }

    #[test]
    fn a_failed_integrity_check_and_a_leftover_tombstone_are_reported() {
        let mut f = facts(&[1], &[]);
        f.integrity = vec![s("row 3 missing from index")];
        f.tombstones = 2;
        let v = judge(&f, &ledger(&[], &[]));
        assert!(v.iter().any(|m| m.contains("integrity")), "{v:?}");
        assert!(v.iter().any(|m| m.contains("tombstone")), "{v:?}");
    }

    #[test]
    fn a_seeded_body_must_survive_exactly_once() {
        let mut l = ledger(&[], &[]);
        l.seeded = vec![s("hello from v2")];
        assert!(judge(&facts(&[1], &["hello from v2"]), &l).is_empty());
        assert!(!judge(&facts(&[1], &[]), &l).is_empty());
    }

    #[test]
    fn only_the_store_keys_archives_and_the_harness_backups_may_stay() {
        let ok = [
            "rooms.db",
            "keys",
            "archive",
            "bk-3.db",
            "keys/store-19ab.key",
            "archive/r-1900000001.db",
            // an interrupted archive or backup leaves a partial file by design (`admin archives` calls it safe to
            // remove), so it is not a stale lock
            "archive/r-1900000001.db.4242.partial",
            "bk-3.db.partial",
            "bk-3.db.partial-journal",
        ]
        .map(s);
        assert!(stray_files(&ok).is_empty());
        let bad = [
            "rooms.db-wal",
            "rooms.db.lock",
            "keys/store-19ab.key.tmp",
            "rooms.db.enc",
            "archive/r-1.db.lock",
        ]
        .map(s);
        assert_eq!(stray_files(&bad).len(), bad.len());
    }
}
