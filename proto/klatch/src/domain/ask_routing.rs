//! Ask routing (#1407): the pure rules that pick who an ask is addressed to. Only structured fields go in: the ask's
//! declared scope, its refs of kind `path` and `repo`, each member's declared scope, the paths it edited since its trail
//! was cleared (#1408) and when it was last seen. Never a word of a message (message text is data), so no keyword can
//! steer an ask. Whether a member may take the ask at all is `could_answer`, which the gateway asks first; this module
//! only orders the members that passed and cuts the list.

use crate::domain::AskRef;

/// The most capable members one ask is addressed to. A broadcast to everyone would be chatter; three is enough to have a
/// second and a third if the first is gone, and few enough that each of them still reads it as meant for them.
pub const ROUTE_MAX: usize = 3;

/// The outcomes a routed ask records, each at most once per ask. Counts only.
pub const ROUTED: &str = "routed";
pub const TAKEN: &str = "taken";
pub const LAPSED: &str = "lapsed";
pub const ESCALATED: &str = "escalated";

/// Why a member is addressed: it could answer, or the lease ran out and it coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteKind {
    Capable,
    Coordinator,
}

impl RouteKind {
    pub fn word(self) -> &'static str {
        match self {
            Self::Capable => "capable",
            Self::Coordinator => "coordinator",
        }
    }

    pub fn parse(word: &str) -> Self {
        if word == "coordinator" {
            Self::Coordinator
        } else {
            Self::Capable
        }
    }
}

/// One addressing: ask `seq` was routed to `member`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskRoute {
    pub seq: i64,
    pub member: String,
    pub kind: RouteKind,
    pub ts: i64,
}

/// A member that passed `could_answer`, with what the order is made of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub name: String,
    pub last_seen: i64,
    pub affinity: u32,
}

/// How closely a member's structured state matches the ask: 2 for the same declared scope, 2 for a `repo` ref equal to
/// its scope, 2 for a `path` ref it edited itself (the `:N` suffix is dropped), 1 for a `path` ref in a directory it
/// edited in. A member that declared nothing scores 0 and is still a candidate: scope is advisory in klatch.
pub fn affinity(
    (ask_scope, refs): (Option<&str>, &[AskRef]),
    (scope, trail): (Option<&str>, &[String]),
) -> u32 {
    let mut score = 0;
    if let (Some(a), Some(m)) = (ask_scope, scope)
        && a == m
    {
        score += 2;
    }
    for r in refs {
        match r.kind.as_str() {
            "repo" if scope == Some(r.value.as_str()) => score += 2,
            "path" => {
                let path = r.value.rsplit_once(':').map_or(&*r.value, |(p, n)| {
                    if n.chars().all(|c| c.is_ascii_digit() || c == '-') {
                        p
                    } else {
                        &r.value
                    }
                });
                let dir = path.rsplit_once('/').map(|(d, _)| d);
                if trail.iter().any(|f| f == path) {
                    score += 2;
                } else if let Some(d) = dir
                    && trail
                        .iter()
                        .any(|f| f.rsplit_once('/').map(|(fd, _)| fd) == Some(d))
                {
                    score += 1;
                }
            }
            _ => {}
        }
    }
    score
}

/// The members an ask is addressed to: highest affinity first, then the most recently seen, then by name, cut to
/// `ROUTE_MAX`. A fixed order, so the same state always gives the same answer.
pub fn pick(mut candidates: Vec<Candidate>) -> Vec<String> {
    candidates.sort_by(|a, b| {
        b.affinity
            .cmp(&a.affinity)
            .then(b.last_seen.cmp(&a.last_seen))
            .then(a.name.cmp(&b.name))
    });
    candidates
        .into_iter()
        .take(ROUTE_MAX)
        .map(|c| c.name)
        .collect()
}

/// The routing outcomes of a window, as counts. An ask routed to nobody is `unrouted`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RoutingCounts {
    pub routed: i64,
    pub unrouted: i64,
    pub taken: i64,
    pub lapsed: i64,
    pub escalated: i64,
    /// Escalations that found no coordinator to address.
    pub escalated_unreached: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(kind: &str, value: &str) -> AskRef {
        AskRef {
            kind: kind.into(),
            value: value.into(),
            ..Default::default()
        }
    }

    #[test]
    fn affinity_comes_from_scope_repo_and_paths_only() {
        let trail = vec!["src/a/x.rs".to_string()];
        assert_eq!(affinity((Some("web"), &[]), (Some("web"), &[])), 2);
        assert_eq!(affinity((Some("web"), &[]), (Some("db"), &[])), 0);
        assert_eq!(affinity((None, &[]), (Some("db"), &[])), 0);
        assert_eq!(affinity((None, &[r("repo", "db")]), (Some("db"), &[])), 2);
        assert_eq!(
            affinity((None, &[r("path", "src/a/x.rs:10-20")]), (None, &trail)),
            2
        );
        assert_eq!(
            affinity((None, &[r("path", "src/a/y.rs")]), (None, &trail)),
            1
        );
        assert_eq!(
            affinity((None, &[r("path", "src/b/y.rs")]), (None, &trail)),
            0
        );
        assert_eq!(affinity((None, &[r("ticket", "#5")]), (None, &trail)), 0);
    }

    #[test]
    fn pick_orders_by_affinity_then_recency_then_name_and_cuts() {
        let c = |n: &str, last_seen, affinity| Candidate {
            name: n.into(),
            last_seen,
            affinity,
        };
        let got = pick(vec![
            c("d", 50, 0),
            c("b", 10, 2),
            c("a", 10, 2),
            c("c", 99, 1),
        ]);
        assert_eq!(got, ["a", "b", "c"]);
        assert!(pick(Vec::new()).is_empty());
    }
}
