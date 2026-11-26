//! The wait-for graph a claim refusal is judged against: who waits for whom, whether a cycle closes, and the chain of
//! members and the requester's own path that name it (#1603).

use super::claims::{Attempt, Held};
use crate::domain::{overlaps, scopes_collide};

/// A member's refused claim attempt, remembered as a wait.
pub(super) struct Wait {
    pub(super) member: String,
    pub(super) path: String,
    pub(super) scope: String,
}

/// The members `node` is waiting for: holders of a claim that overlaps something `node` waits for, in a scope
/// that collides with the wait.
pub(super) fn blockers<'a>(node: &str, waits: &[Wait], held: &'a [Held]) -> Vec<&'a str> {
    let mut out = Vec::new();
    for w in waits.iter().filter(|w| w.member == node) {
        let blocks = |h: &&Held| {
            h.owner != w.member && overlaps(&h.path, &w.path) && scopes_collide(&h.scope, &w.scope)
        };
        out.extend(held.iter().filter(blocks).map(|h| h.owner.as_str()));
    }
    out
}

/// Whether `target` can be reached from `start` by following who waits for whom.
pub(super) fn reaches(start: &str, target: &str, waits: &[Wait], held: &[Held]) -> bool {
    chain(start, target, waits, held).is_some()
}

/// The members on a shortest wait chain from `start` to `target`, both included, in order (#1603).
pub(super) fn chain<'a>(
    start: &'a str,
    target: &str,
    waits: &[Wait],
    held: &'a [Held],
) -> Option<Vec<&'a str>> {
    let mut seen: Vec<&str> = vec![start];
    let mut paths: Vec<Vec<&str>> = vec![vec![start]];
    let mut next = 0;
    while next < paths.len() {
        let path = paths[next].clone();
        next += 1;
        let node = path[path.len() - 1];
        if node == target {
            return Some(path);
        }
        for b in blockers(node, waits, held) {
            if !seen.contains(&b) {
                seen.push(b);
                let mut p = path.clone();
                p.push(b);
                paths.push(p);
            }
        }
    }
    None
}

/// The members of the cycle a refusal closes, starting at a member that is in it (the named blocker when it is, else the
/// first holder in the cycle), and the requester's own path the last of them waits for.
pub(super) fn cycle_names(
    req: &Attempt,
    blocker: &str,
    cycle: &[String],
    waits: &[Wait],
    held: &[Held],
) -> (Vec<String>, String) {
    let start = cycle.iter().find(|m| *m == blocker).unwrap_or(&cycle[0]);
    let Some(c) = chain(start, req.owner, waits, held) else {
        return (Vec::new(), String::new());
    };
    let via = path_waited_for(c[c.len() - 2], req, waits, held).unwrap_or_default();
    (
        c[..c.len() - 1].iter().map(|m| m.to_string()).collect(),
        via,
    )
}

/// The refused member's own held path that `waiter` waits for: what closes the cycle on the requester's side.
pub(super) fn path_waited_for(
    waiter: &str,
    req: &Attempt,
    waits: &[Wait],
    held: &[Held],
) -> Option<String> {
    held.iter()
        .filter(|h| h.owner == req.owner)
        .find(|h| {
            waits.iter().any(|w| {
                w.member == waiter
                    && overlaps(&h.path, &w.path)
                    && scopes_collide(&h.scope, &w.scope)
            })
        })
        .map(|h| h.path.clone())
}
