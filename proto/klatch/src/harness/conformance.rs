//! The conformance suite every harness adapter must pass (#62, DESIGN section 20 rules 1, 3 and 6). An adapter is an
//! untrusted conveyor that only declares what its runtime can do, so what it declares has to be checkable: this
//! suite reads the declaration and reports every rule it breaks. It runs over each adapter in `adapters()`; adding an
//! adapter (Codex arrives for real in #64) is one line there, and `every_registered_adapter_passes` covers it.
//!
//! The rules, each with the way a bad adapter fails it:
//! 1. The id is a short stable word (`[a-z0-9-]`, 1 to 32 characters), the same whatever the probe said, and no two
//!    adapters share one.
//! 2. `level` is total (an answer for every capability) and gives the same answer when asked again.
//! 3. A `Native` level names its basis, a path from the repository root to the test or document that observed it,
//!    and the file exists and is not empty. A level below `Native` names none.
//! 4. Nothing is `Native` on a guess: with the probe unanswered, a capability that rides on the probe is not
//!    `Native`, and no capability is better than it is with the probe answered well.

use super::negotiate::{Capability, ClaudeCode, Codex, Generic, Harness, HookTrust, Level};
use super::{Support, Version};
use std::path::Path;

/// One adapter as the suite sees it: built once with its probe answered well and once with the probe unanswered.
pub struct Adapter {
    /// The name in failure messages.
    pub name: &'static str,
    /// The harness when whatever it depends on (a version probe, a hook trust check) answered well.
    pub probed: Box<dyn Harness>,
    /// The same harness when that probe could not tell.
    pub unprobed: Box<dyn Harness>,
    /// The capabilities whose level rides on the probe. With the probe unanswered, none may be `Native`.
    pub probe_dependent: &'static [Capability],
}

/// Every adapter in this build. A new adapter is one line here.
pub fn adapters() -> Vec<Adapter> {
    vec![
        Adapter {
            name: "claude-code",
            probed: Box::new(ClaudeCode::new(Support::Supported(Version(2, 1, 325)))),
            unprobed: Box::new(ClaudeCode::new(Support::Unknown("no claude".into()))),
            probe_dependent: &[Capability::Wake],
        },
        Adapter {
            name: "codex",
            probed: Box::new(Codex::paper(HookTrust::Trusted)),
            unprobed: Box::new(Codex::paper(HookTrust::Unknown)),
            probe_dependent: &[
                Capability::ContextInjection,
                Capability::SessionIdentity,
                Capability::PreActionEnforcement,
                Capability::PermissionMode,
            ],
        },
        Adapter {
            name: "generic",
            probed: Box::new(Generic),
            unprobed: Box::new(Generic),
            probe_dependent: &[],
        },
    ]
}

/// Where basis paths start: the repository root.
fn repo_root() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn rank(l: Level) -> u8 {
    match l {
        Level::Absent => 0,
        Level::Degraded => 1,
        Level::Native => 2,
    }
}

fn id_problem(id: &str) -> Option<String> {
    let ok = !id.is_empty()
        && id.len() <= 32
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    (!ok).then(|| format!("id {id:?} is not 1 to 32 characters of [a-z0-9-]"))
}

/// The rules one harness breaks on its own (1 to 3), as sentences. Empty when it passes.
pub fn check_harness(h: &dyn Harness) -> Vec<String> {
    let mut found = Vec::new();
    let id = h.id().to_string();
    found.extend(id_problem(&id));
    if h.id() != id {
        found.push(format!(
            "id changed between calls: {id:?} then {:?}",
            h.id()
        ));
    }
    for c in Capability::ALL {
        let level = h.level(c);
        if h.level(c) != level {
            found.push(format!("{id}: level of {} changed between calls", c.name()));
        }
        match (level, h.basis(c)) {
            (Level::Native, None) => found.push(format!(
                "{id}: {} is native but names no basis (a test or document that observed it)",
                c.name()
            )),
            (Level::Native, Some(path)) => {
                let file = repo_root().join(path);
                match std::fs::metadata(&file) {
                    Ok(m) if m.is_file() && m.len() > 0 => {}
                    _ => found.push(format!(
                        "{id}: {} is native on basis {path}, which is not a non-empty file in the repository",
                        c.name()
                    )),
                }
            }
            (_, Some(path)) => found.push(format!(
                "{id}: {} is not native but names basis {path}",
                c.name()
            )),
            (_, None) => {}
        }
    }
    found
}

/// Every rule an adapter breaks, including the ones that compare its two builds. Empty when it passes.
pub fn check_adapter(a: &Adapter) -> Vec<String> {
    let mut found = check_harness(a.probed.as_ref());
    found.extend(check_harness(a.unprobed.as_ref()));
    if a.probed.id() != a.unprobed.id() {
        found.push(format!(
            "{}: the id depends on the probe ({:?} and {:?})",
            a.name,
            a.probed.id(),
            a.unprobed.id()
        ));
    }
    for c in Capability::ALL {
        let (good, guess) = (a.probed.level(c), a.unprobed.level(c));
        if rank(guess) > rank(good) {
            found.push(format!(
                "{}: {} is better with the probe unanswered than answered well",
                a.name,
                c.name()
            ));
        }
        if a.probe_dependent.contains(&c) && guess == Level::Native {
            found.push(format!(
                "{}: {} rides on the probe and is native without an answer",
                a.name,
                c.name()
            ));
        }
    }
    found
}

/// Every rule broken across a set of adapters, including a shared id. Empty when all pass.
pub fn check_all(all: &[Adapter]) -> Vec<String> {
    let mut found: Vec<String> = all.iter().flat_map(check_adapter).collect();
    for (i, a) in all.iter().enumerate() {
        if all[..i].iter().any(|b| b.probed.id() == a.probed.id()) {
            found.push(format!("two adapters share the id {:?}", a.probed.id()));
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_registered_adapter_passes() {
        let all = adapters();
        assert!(
            all.len() >= 3,
            "claude-code, codex and generic are registered"
        );
        let found = check_all(&all);
        assert!(found.is_empty(), "{}", found.join("\n"));
    }

    /// A harness built to break one rule at a time.
    struct Bad {
        id: &'static str,
        native: Option<Capability>,
        basis: Option<&'static str>,
    }

    impl Bad {
        fn new() -> Bad {
            Bad {
                id: "bad",
                native: None,
                basis: None,
            }
        }
    }

    impl Harness for Bad {
        fn id(&self) -> &str {
            self.id
        }
        fn level(&self, c: Capability) -> Level {
            if self.native == Some(c) {
                Level::Native
            } else {
                Level::Absent
            }
        }
        fn basis(&self, _: Capability) -> Option<&'static str> {
            self.basis
        }
    }

    fn says(found: &[String], word: &str) -> bool {
        found.iter().any(|f| f.contains(word))
    }

    #[test]
    fn a_bad_id_is_caught() {
        for id in [
            "",
            "Claude",
            "has space",
            "a-very-long-id-that-goes-on-for-far-too-long",
        ] {
            let mut b = Bad::new();
            b.id = id;
            assert!(says(&check_harness(&b), "id "), "{id:?}");
        }
    }

    #[test]
    fn a_native_with_no_basis_is_caught() {
        let mut b = Bad::new();
        b.native = Some(Capability::Wake);
        assert!(says(&check_harness(&b), "names no basis"));
    }

    #[test]
    fn a_native_whose_basis_is_not_a_file_is_caught() {
        let mut b = Bad::new();
        b.native = Some(Capability::Wake);
        b.basis = Some("proto/klatch/tests/there-is-no-such-test.rs");
        assert!(says(&check_harness(&b), "not a non-empty file"));
        b.basis = Some("proto");
        assert!(says(&check_harness(&b), "not a non-empty file"));
    }

    #[test]
    fn a_basis_on_something_that_is_not_native_is_caught() {
        let mut b = Bad::new();
        b.basis = Some("RESEARCH-NOTES.md");
        assert!(says(&check_harness(&b), "is not native but names basis"));
    }

    #[test]
    fn a_level_that_changes_between_calls_is_caught() {
        struct Flaky(std::cell::Cell<bool>);
        impl Harness for Flaky {
            fn id(&self) -> &str {
                "flaky"
            }
            fn level(&self, _: Capability) -> Level {
                self.0.set(!self.0.get());
                if self.0.get() {
                    Level::Absent
                } else {
                    Level::Degraded
                }
            }
            fn basis(&self, _: Capability) -> Option<&'static str> {
                None
            }
        }
        assert!(says(
            &check_harness(&Flaky(std::cell::Cell::new(false))),
            "changed between calls"
        ));
    }

    fn pair(a: Bad, b: Bad, dep: &'static [Capability]) -> Adapter {
        Adapter {
            name: "bad",
            probed: Box::new(a),
            unprobed: Box::new(b),
            probe_dependent: dep,
        }
    }

    #[test]
    fn a_probe_dependent_capability_that_is_native_without_an_answer_is_caught() {
        let mut a = Bad::new();
        a.native = Some(Capability::PullPath);
        a.basis = Some("RESEARCH-NOTES.md");
        let mut b = Bad::new();
        b.native = Some(Capability::PullPath);
        b.basis = Some("RESEARCH-NOTES.md");
        let found = check_adapter(&pair(a, b, &[Capability::PullPath]));
        assert!(
            says(&found, "rides on the probe and is native"),
            "{found:?}"
        );
    }

    #[test]
    fn an_unanswered_probe_that_does_better_than_an_answered_one_is_caught() {
        let a = Bad::new();
        let mut b = Bad::new();
        b.native = Some(Capability::Wake);
        b.basis = Some("RESEARCH-NOTES.md");
        let found = check_adapter(&pair(a, b, &[]));
        assert!(
            says(&found, "better with the probe unanswered"),
            "{found:?}"
        );
    }

    #[test]
    fn an_id_that_depends_on_the_probe_or_is_shared_is_caught() {
        let a = Bad::new();
        let mut b = Bad::new();
        b.id = "other";
        assert!(says(
            &check_adapter(&pair(a, b, &[])),
            "id depends on the probe"
        ));
        let both = [
            pair(Bad::new(), Bad::new(), &[]),
            pair(Bad::new(), Bad::new(), &[]),
        ];
        assert!(says(&check_all(&both), "share the id"));
    }

    #[test]
    fn the_suite_is_not_vacuous_each_real_adapter_has_something_native_to_check() {
        for a in adapters() {
            assert!(
                Capability::ALL
                    .iter()
                    .any(|c| a.probed.level(*c) == Level::Native),
                "{} has no native capability, so rule 3 checked nothing",
                a.name
            );
        }
    }
}
