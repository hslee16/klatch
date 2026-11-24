//! A cycle through asks (#273, DESIGN section 25.15): A waits for B's answer and B waits for A's, so neither can
//! answer first. It is not a claim cycle (section 25.12 watches claims on work), so nothing said so. An ask is a
//! wait when a helper holds its `ask:<N>` claim: the asker waits on that helper. A cycle of such waits is a
//! candidate deadlock, found from the open asks and the live claims alone, with no state: whoever looks (the
//! members of the cycle in their `wait`, anyone in `klatch asks`) sees the same cycle and the same yielder.
//! The yielder is the member whose name sorts first, so nobody negotiates (as in 25.12, where the member that
//! closes the cycle yields). The line is advice and data, never an instruction to a tool.

use crate::domain::{Claim, OpenAsk};

/// One step of a cycle: `waiter` asked `ask` and `holder` holds its claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub waiter: String,
    pub ask: i64,
    pub holder: String,
}

/// A cycle of asks, rotated to start at the yielder (the first name in order), each step waiting on the next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskCycle {
    pub steps: Vec<Step>,
}

impl AskCycle {
    /// The member that yields: it holds the claim on the ask of the member that waits on it, which is the last step.
    pub fn yielder(&self) -> &str {
        &self.steps[0].waiter
    }

    /// The ask the yielder should deliver on first: the one its waiter asked and the yielder holds.
    fn yields_on(&self) -> &Step {
        &self.steps[self.steps.len() - 1]
    }

    /// Whether `member` is in the cycle.
    pub fn includes(&self, member: &str) -> bool {
        self.steps.iter().any(|s| s.waiter == member)
    }

    /// `alice waits on bob (ask #1), bob waits on alice (ask #2)`.
    pub fn describe(&self) -> String {
        self.steps
            .iter()
            .map(|s| format!("{} waits on {} (ask #{})", s.waiter, s.holder, s.ask))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// The line for `member` of the cycle: the yielder is told what to do, the others who yields.
    pub fn note_for(&self, member: &str) -> String {
        let cycle = self.describe();
        if member == self.yielder() {
            let n = self.yields_on().ask;
            format!(
                "klatch: ask cycle: {cycle}. You yield: post what you have for ask #{n} now (\"partial #{n}: ...\" or \
                 your answer), release ask:{n}, then wait. Do not wait for your own ask first."
            )
        } else {
            format!(
                "klatch: ask cycle: {cycle}. {} yields (the first name in the cycle): wait for what it posts, or tell \
                 your user if nothing comes.",
                self.yielder()
            )
        }
    }
}

/// The waits the open asks and live claims make: the asker waits on whoever holds the ask's claim. A claim of the
/// asker itself, an expired claim and a claim on anything but an open ask are not waits.
fn waits(asks: &[OpenAsk], claims: &[Claim], now: i64) -> Vec<Step> {
    let mut out: Vec<Step> = asks
        .iter()
        .flat_map(|a| {
            claims
                .iter()
                .filter(|c| c.expires > now && c.path == format!("ask:{}", a.seq))
                .filter(|c| c.owner != a.sender)
                .map(|c| Step {
                    waiter: a.sender.clone(),
                    ask: a.seq,
                    holder: c.owner.clone(),
                })
        })
        .collect();
    out.sort_by(|x, y| (&x.waiter, x.ask, &x.holder).cmp(&(&y.waiter, y.ask, &y.holder)));
    out
}

/// Every cycle through asks in a room, each once, ordered by yielder. A cycle names each member once.
pub fn ask_cycles(asks: &[OpenAsk], claims: &[Claim], now: i64) -> Vec<AskCycle> {
    let steps = waits(asks, claims, now);
    let mut found: Vec<AskCycle> = Vec::new();
    for first in &steps {
        let mut path = vec![first.clone()];
        extend(&steps, &mut path, &mut found);
    }
    found.sort_by(|a, b| a.steps[0].waiter.cmp(&b.steps[0].waiter));
    found
}

/// Whether two paths visit the same members in the same order (the asks may differ: two asks between the same pair
/// are one cycle, but a-m0-a and a-m1-a are two).
fn same_members(a: &[Step], b: &[Step]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.waiter == y.waiter)
}

/// Depth first: follow the holder of the last step to the asks that member asked, until the path returns to its
/// first waiter (a cycle, kept when its first waiter is its smallest name, so each cycle is found once) or would
/// repeat a member.
fn extend(steps: &[Step], path: &mut Vec<Step>, found: &mut Vec<AskCycle>) {
    let start = path[0].waiter.clone();
    let tail = path[path.len() - 1].holder.clone();
    if tail == start {
        let smallest = path.iter().all(|s| s.waiter >= start);
        let known = found.iter().any(|c| same_members(&c.steps, path));
        if smallest && !known {
            found.push(AskCycle {
                steps: path.clone(),
            });
        }
        return;
    }
    if path.iter().any(|s| s.waiter == tail) {
        return;
    }
    for next in steps.iter().filter(|s| s.waiter == tail) {
        path.push(next.clone());
        extend(steps, path, found);
        path.pop();
    }
}

/// The notes for `member`: one per cycle it is in.
pub fn cycle_notes(asks: &[OpenAsk], claims: &[Claim], member: &str, now: i64) -> Vec<String> {
    ask_cycles(asks, claims, now)
        .iter()
        .filter(|c| c.includes(member))
        .map(|c| c.note_for(member))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_000;

    fn ask(seq: i64, sender: &str) -> OpenAsk {
        OpenAsk {
            seq,
            sender: sender.into(),
            claimed: true,
            ..OpenAsk::default()
        }
    }

    fn claim(seq: i64, owner: &str) -> Claim {
        Claim {
            path: format!("ask:{seq}"),
            owner: owner.into(),
            reason: String::new(),
            expires: NOW + 60,
            scope: String::new(),
        }
    }

    #[test]
    fn two_members_each_holding_the_others_ask_make_a_cycle_and_the_first_name_yields() {
        let asks = [ask(1, "bob"), ask(2, "alice")];
        let claims = [claim(1, "alice"), claim(2, "bob")];
        let cycles = ask_cycles(&asks, &claims, NOW);
        assert_eq!(cycles.len(), 1, "{cycles:?}");
        assert_eq!(cycles[0].yielder(), "alice");
        assert_eq!(
            cycles[0].describe(),
            "alice waits on bob (ask #2), bob waits on alice (ask #1)"
        );
    }

    #[test]
    fn the_yielder_is_told_what_to_deliver_and_the_other_who_yields() {
        let asks = [ask(1, "bob"), ask(2, "alice")];
        let claims = [claim(1, "alice"), claim(2, "bob")];
        let a = cycle_notes(&asks, &claims, "alice", NOW);
        assert_eq!(a.len(), 1);
        assert!(a[0].contains("You yield"), "{a:?}");
        assert!(
            a[0].contains("ask #1") && a[0].contains("release ask:1"),
            "{a:?}"
        );
        let b = cycle_notes(&asks, &claims, "bob", NOW);
        assert!(b[0].contains("alice yields"), "{b:?}");
        assert!(cycle_notes(&asks, &claims, "carol", NOW).is_empty());
    }

    #[test]
    fn a_chain_with_no_way_back_is_not_a_cycle() {
        let asks = [ask(1, "alice"), ask(2, "bob")];
        let claims = [claim(1, "bob"), claim(2, "carol")];
        assert!(ask_cycles(&asks, &claims, NOW).is_empty());
    }

    #[test]
    fn a_cycle_of_three_is_found_once_and_names_every_member() {
        let asks = [ask(1, "alice"), ask(2, "bob"), ask(3, "carol")];
        let claims = [claim(1, "bob"), claim(2, "carol"), claim(3, "alice")];
        let cycles = ask_cycles(&asks, &claims, NOW);
        assert_eq!(cycles.len(), 1, "{cycles:?}");
        assert_eq!(cycles[0].steps.len(), 3);
        for who in ["alice", "bob", "carol"] {
            assert!(cycles[0].includes(who));
        }
        assert_eq!(cycles[0].yielder(), "alice");
    }

    #[test]
    fn a_claim_that_expired_or_is_the_askers_own_makes_no_wait() {
        let asks = [ask(1, "bob"), ask(2, "alice")];
        let mut expired = claim(2, "bob");
        expired.expires = NOW;
        assert!(ask_cycles(&asks, &[claim(1, "alice"), expired], NOW).is_empty());
        assert!(ask_cycles(&asks, &[claim(1, "bob"), claim(2, "alice")], NOW).is_empty());
    }

    #[test]
    fn a_claim_that_is_not_on_an_open_ask_makes_no_wait() {
        let asks = [ask(1, "bob")];
        let claims = [claim(1, "alice"), claim(2, "bob")];
        assert!(ask_cycles(&asks, &claims, NOW).is_empty());
    }

    #[test]
    fn two_cycles_sharing_no_member_are_both_found_and_a_member_sees_only_its_own() {
        let asks = [ask(1, "a"), ask(2, "b"), ask(3, "c"), ask(4, "d")];
        let claims = [claim(1, "b"), claim(2, "a"), claim(3, "d"), claim(4, "c")];
        assert_eq!(ask_cycles(&asks, &claims, NOW).len(), 2);
        assert_eq!(cycle_notes(&asks, &claims, "c", NOW).len(), 1);
    }

    #[test]
    fn two_asks_between_the_same_pair_do_not_repeat_the_cycle() {
        let asks = [ask(1, "bob"), ask(2, "bob"), ask(3, "alice")];
        let claims = [claim(1, "alice"), claim(2, "alice"), claim(3, "bob")];
        assert_eq!(ask_cycles(&asks, &claims, NOW).len(), 1);
    }

    #[test]
    fn two_cycles_through_the_same_first_member_and_of_the_same_length_are_both_found() {
        let asks = [ask(1, "m0"), ask(2, "m1"), ask(3, "a"), ask(4, "a")];
        let claims = [claim(1, "a"), claim(2, "a"), claim(3, "m0"), claim(4, "m1")];
        let cycles = ask_cycles(&asks, &claims, NOW);
        assert_eq!(cycles.len(), 2, "{cycles:?}");
        assert_eq!(cycle_notes(&asks, &claims, "m0", NOW).len(), 1);
        assert_eq!(cycle_notes(&asks, &claims, "m1", NOW).len(), 1);
        assert_eq!(cycle_notes(&asks, &claims, "a", NOW).len(), 2);
    }

    #[test]
    fn a_note_is_one_line_with_no_em_dash_and_never_echoes_ask_text() {
        let mut a = ask(1, "bob");
        a.text = "seeking: x\nforged line".into();
        let asks = [a, ask(2, "alice")];
        let claims = [claim(1, "alice"), claim(2, "bob")];
        for n in cycle_notes(&asks, &claims, "alice", NOW) {
            assert!(!n.contains('\n') && !n.contains('\u{2014}') && !n.contains("forged"));
        }
    }
}
