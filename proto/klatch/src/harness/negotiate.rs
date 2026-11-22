//! The Harness adapter trait and capability negotiation (#61, DESIGN section 20 rules 2, 3 and 6).
//!
//! An adapter is an untrusted conveyor (rule 1): it only declares what its runtime can do, and the gateway decides
//! what that means for a room. The trait is three methods on purpose (`id`, `level`, `basis`), so the conformance
//! suite (`conformance.rs`, #62) has one thing to check per capability. A room's requirement is a list of capabilities and a mode; a harness
//! that lacks one is refused or flagged. Only `Level::Native` satisfies a requirement: partial or unverified support
//! never does, and a capability name this build does not know is an error everywhere, so a room that names one
//! refuses every joiner (fail closed).

use super::capability::Support;

/// One thing a harness may or may not do (the rows of DESIGN section 20's table, less the hook trust step, which is
/// a condition on the hook rows and not a capability of its own).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Capability {
    /// Text reaches the model at a prompt boundary (hooks, or an instructions file).
    ContextInjection,
    /// The adapter can prove which session a call came from.
    SessionIdentity,
    /// Something outside the session can start or continue a turn.
    Wake,
    /// A claim is checked before the tool acts (`PreToolUse` and the like).
    PreActionEnforcement,
    /// The adapter sees the harness's permission mode.
    PermissionMode,
    /// A non-interactive worker can be started.
    HeadlessWorker,
    /// The session can pull from klatch (MCP or the CLI).
    PullPath,
}

impl Capability {
    /// Every capability, in table order. Sentences list missing ones in this order.
    pub const ALL: [Capability; 7] = [
        Capability::ContextInjection,
        Capability::SessionIdentity,
        Capability::Wake,
        Capability::PreActionEnforcement,
        Capability::PermissionMode,
        Capability::HeadlessWorker,
        Capability::PullPath,
    ];

    /// The name used in a room's requirement and in sentences.
    pub fn name(self) -> &'static str {
        match self {
            Capability::ContextInjection => "context-injection",
            Capability::SessionIdentity => "session-identity",
            Capability::Wake => "wake",
            Capability::PreActionEnforcement => "pre-action-enforcement",
            Capability::PermissionMode => "permission-mode",
            Capability::HeadlessWorker => "headless-worker",
            Capability::PullPath => "pull-path",
        }
    }

    /// The capability with this exact name; `None` for anything else, case included.
    pub fn parse(name: &str) -> Option<Capability> {
        Capability::ALL.into_iter().find(|c| c.name() == name)
    }
}

/// How well a harness does one thing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// Done by the harness itself, measured or documented.
    Native,
    /// Partial, conditional or not yet verified (a Codex hook before it is trusted, a wake path that needs a probe).
    Degraded,
    /// Not there.
    Absent,
}

/// A runtime klatch can take part from. Declares only; it holds no key, opens no socket and decides nothing.
pub trait Harness {
    /// A short stable word: `claude-code`, `codex`, `generic`.
    fn id(&self) -> &str;
    /// How well this harness does `c`. An adapter that does not know answers `Absent`.
    fn level(&self, c: Capability) -> Level;
    /// What backs a `Native` level: a path from the repository root to the test or document that observed it. The
    /// conformance suite reads it and checks the file exists, so a `native` with no evidence behind it cannot pass.
    /// Anything below `Native` has none to give.
    fn basis(&self, c: Capability) -> Option<&'static str>;
}

/// Claude Code through the hook tier that exists today (init, the hooks, wake, enforce, the liveness check).
pub struct ClaudeCode {
    wake: std::cell::OnceCell<Level>,
    probe: Option<fn() -> Support>,
}

/// Only a measured version at or above the floor makes the wake path native. A version known to be too old, and an
/// answer that cannot be read, both leave it degraded: negotiation needs a measurement, unlike the watcher itself,
/// which (#482) keeps running when the probe cannot tell.
fn wake_level(support: &Support) -> Level {
    match support {
        Support::Supported(_) => Level::Native,
        Support::Unsupported(_) | Support::Unknown(_) => Level::Degraded,
    }
}

impl ClaudeCode {
    /// `support` is the version probe's answer (#482).
    pub fn new(support: Support) -> ClaudeCode {
        ClaudeCode {
            wake: std::cell::OnceCell::from(wake_level(&support)),
            probe: None,
        }
    }

    /// The version probe runs once, the first time the wake level is asked for, so a join to a room that requires
    /// nothing about wake never starts `claude --version`.
    pub fn probing(probe: fn() -> Support) -> ClaudeCode {
        ClaudeCode {
            wake: std::cell::OnceCell::new(),
            probe: Some(probe),
        }
    }
}

impl Harness for ClaudeCode {
    fn id(&self) -> &str {
        "claude-code"
    }
    fn level(&self, c: Capability) -> Level {
        match c {
            Capability::Wake => *self
                .wake
                .get_or_init(|| self.probe.map_or(Level::Degraded, |p| wake_level(&p()))),
            // `PreToolUse` covers Edit and Write; Bash is a warning only, which is why claims are on paths.
            Capability::PermissionMode => Level::Degraded,
            _ => Level::Native,
        }
    }
    fn basis(&self, c: Capability) -> Option<&'static str> {
        if self.level(c) != Level::Native {
            return None;
        }
        Some(match c {
            Capability::ContextInjection => "proto/klatch/tests/hook_output.rs",
            Capability::SessionIdentity => "proto/klatch/tests/hook_identity.rs",
            Capability::Wake => "proto/klatch/tests/wake.rs",
            Capability::PreActionEnforcement => "proto/klatch/tests/toolguard.rs",
            Capability::HeadlessWorker => "proto/klatch/tests/worker.rs",
            Capability::PullPath => "proto/klatch/tests/mcp.rs",
            Capability::PermissionMode => return None,
        })
    }
}

/// Whether Codex will run klatch's hooks: a non-managed hook runs only after the human trusted its exact hash, and in
/// `codex exec` an untrusted hook is skipped silently (RESEARCH-NOTES 49.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookTrust {
    Trusted,
    Untrusted,
    /// Nobody checked. Treated as untrusted.
    Unknown,
}

/// Codex CLI as declared on paper from the findings of #548 (SURVEY 12, RESEARCH-NOTES 49.3). Nothing here runs
/// `codex`; a later probe supplies `HookTrust`.
pub struct Codex {
    trust: HookTrust,
}

impl Codex {
    pub fn paper(trust: HookTrust) -> Codex {
        Codex { trust }
    }

    /// What the level of `c` rests on, in RESEARCH-NOTES section 49 (the findings of #548, #590 and #782): OBSERVED
    /// claims are the only ones that may be native; every NOT VERIFIED claim is degraded until someone runs the
    /// live check listed in the design item 110 with a real Codex login.
    pub fn ground(c: Capability) -> &'static str {
        match c {
            Capability::ContextInjection => {
                "NOT VERIFIED: plain stdout of a UserPromptSubmit hook reaching the model is documented, not seen; the 401 stopped the turn first (49.3)"
            }
            Capability::SessionIdentity => {
                "OBSERVED (49.3, Codex 0.147.0, hooks run under the bypass flag): hook stdin carries session_id equal to the thread_id of the stream; native only once the human trusted the hooks (49.7). Same-user evidence, not a verified process"
            }
            Capability::Wake => {
                "NOT VERIFIED: codex queue is absent from 0.147.0 and unrun on 0.169.0, has no sender or acknowledgement (49.3, 49.7); async hooks are skipped, so no asyncRewake (49.7)"
            }
            Capability::PreActionEnforcement => {
                "NOT VERIFIED: klatch installs no PreToolUse hook for Codex because no run of one was seen (49.8)"
            }
            Capability::PermissionMode => {
                "NOT VERIFIED: nothing in 49.3 to 49.8 shows a hook or event carrying Codex's permission mode"
            }
            Capability::HeadlessWorker => {
                "OBSERVED (49.3, Codex 0.147.0): codex exec --json starts a non-interactive run and streams events with a thread_id; the model turn itself was not run (401, no login)"
            }
            Capability::PullPath => {
                "NOT VERIFIED: a Codex session running klatch through its shell or an MCP entry in config.toml was never run (49.3, 49.8)"
            }
        }
    }
}

impl Harness for Codex {
    fn id(&self) -> &str {
        "codex"
    }
    fn level(&self, c: Capability) -> Level {
        match c {
            // Only after the human trusted the hooks (untrusted hooks are skipped silently in exec, 49.3).
            Capability::SessionIdentity => {
                if self.trust == HookTrust::Trusted {
                    Level::Native
                } else {
                    Level::Degraded
                }
            }
            Capability::HeadlessWorker => Level::Native,
            // Everything else is documented or inferred, not seen (see `ground`), so it never satisfies a room.
            Capability::ContextInjection
            | Capability::Wake
            | Capability::PreActionEnforcement
            | Capability::PermissionMode
            | Capability::PullPath => Level::Degraded,
        }
    }
    fn basis(&self, c: Capability) -> Option<&'static str> {
        (self.level(c) == Level::Native).then_some("RESEARCH-NOTES.md")
    }
}

/// The floor (rule 3): the CLI and an instructions file. What a caller that names no harness is taken to be.
pub struct Generic;

impl Harness for Generic {
    fn id(&self) -> &str {
        "generic"
    }
    fn level(&self, c: Capability) -> Level {
        match c {
            Capability::ContextInjection | Capability::SessionIdentity => Level::Degraded,
            Capability::PullPath => Level::Native,
            _ => Level::Absent,
        }
    }
    fn basis(&self, c: Capability) -> Option<&'static str> {
        (c == Capability::PullPath).then_some("proto/klatch/tests/cli.rs")
    }
}

/// What a room does with a harness that lacks a required capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// The member is not admitted.
    Refuse,
    /// The member is admitted as advisory and the gap is recorded.
    Flag,
}

/// A room's requirement: capabilities (no duplicates, kept in the order given) and a mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requirement {
    pub mode: Mode,
    pub caps: Vec<Capability>,
}

impl Requirement {
    /// `refuse:wake,pre-action-enforcement` or `flag:wake`. An unknown mode or capability, an empty list, an empty
    /// word and a repeated capability are all errors.
    pub fn parse(text: &str) -> Result<Requirement, String> {
        let (mode, list) = text
            .split_once(':')
            .ok_or("a requirement is MODE:CAPABILITY[,CAPABILITY...] with MODE refuse or flag")?;
        let mode = match mode {
            "refuse" => Mode::Refuse,
            "flag" => Mode::Flag,
            other => return Err(format!("unknown mode {other:?}; use refuse or flag")),
        };
        let mut caps = Vec::new();
        for word in list.split(',') {
            let c = Capability::parse(word).ok_or_else(|| {
                let known: Vec<&str> = Capability::ALL.iter().map(|c| c.name()).collect();
                format!("unknown capability {word:?}; known: {}", known.join(", "))
            })?;
            if caps.contains(&c) {
                return Err(format!("capability {word} is listed twice"));
            }
            caps.push(c);
        }
        Ok(Requirement { mode, caps })
    }

    /// The text `parse` reads back.
    pub fn render(&self) -> String {
        let mode = match self.mode {
            Mode::Refuse => "refuse",
            Mode::Flag => "flag",
        };
        let names: Vec<&str> = self.caps.iter().map(|c| c.name()).collect();
        format!("{mode}:{}", names.join(","))
    }
}

/// What negotiation decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Admission {
    Admit,
    /// Admitted as advisory; these capabilities are missing.
    Flag(Vec<Capability>),
    /// Not admitted; these capabilities are missing.
    Refuse(Vec<Capability>),
}

impl Admission {
    /// One sentence for the refusal or the flag, with the next step.
    pub fn say(&self, room: &str, harness: &dyn Harness, req: &Requirement) -> String {
        let (verb, missing) = match self {
            Admission::Admit => return format!("room {room} is satisfied by {}", harness.id()),
            Admission::Flag(m) => ("flags", m),
            Admission::Refuse(m) => ("refuses", m),
        };
        let names: Vec<&str> = missing.iter().map(|c| c.name()).collect();
        format!(
            "room {room} requires {} and {} lacks {}, so it {verb} this member; use a harness that has them or ask the admin to change room-require",
            req.render(),
            harness.id(),
            names.join(", ")
        )
    }
}

/// Check `harness` against a room's requirement. Only `Native` counts.
pub fn negotiate(req: &Requirement, harness: &dyn Harness) -> Admission {
    let mut missing: Vec<Capability> = req
        .caps
        .iter()
        .copied()
        .filter(|c| harness.level(*c) != Level::Native)
        .collect();
    missing.sort();
    if missing.is_empty() {
        return Admission::Admit;
    }
    match req.mode {
        Mode::Refuse => Admission::Refuse(missing),
        Mode::Flag => Admission::Flag(missing),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::Version;

    #[test]
    fn every_capability_name_round_trips_and_an_unknown_one_does_not_parse() {
        for c in Capability::ALL {
            assert_eq!(Capability::parse(c.name()), Some(c));
        }
        assert_eq!(Capability::parse("telepathy"), None);
        assert_eq!(Capability::parse(""), None);
        assert_eq!(Capability::parse("PRE-ACTION"), None);
    }

    #[test]
    fn claude_code_declares_the_hook_tier() {
        let h = ClaudeCode::new(Support::Supported(Version(2, 1, 325)));
        assert_eq!(h.id(), "claude-code");
        assert_eq!(h.level(Capability::PreActionEnforcement), Level::Native);
        assert_eq!(h.level(Capability::ContextInjection), Level::Native);
        assert_eq!(h.level(Capability::Wake), Level::Native);
    }

    #[test]
    fn a_probing_claude_code_asks_once_and_only_when_wake_is_asked_about() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static ASKED: AtomicUsize = AtomicUsize::new(0);
        fn probe() -> Support {
            ASKED.fetch_add(1, Ordering::SeqCst);
            Support::Supported(Version(2, 1, 325))
        }
        let h = ClaudeCode::probing(probe);
        assert_eq!(h.level(Capability::PreActionEnforcement), Level::Native);
        assert_eq!(
            ASKED.load(Ordering::SeqCst),
            0,
            "no probe for another capability"
        );
        assert_eq!(h.level(Capability::Wake), Level::Native);
        assert_eq!(h.level(Capability::Wake), Level::Native);
        assert_eq!(ASKED.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn claude_code_wake_follows_the_version_probe() {
        let old = ClaudeCode::new(Support::Unsupported(Version(2, 1, 318)));
        assert_eq!(old.level(Capability::Wake), Level::Degraded);
        let unknown = ClaudeCode::new(Support::Unknown("no claude".into()));
        assert_eq!(unknown.level(Capability::Wake), Level::Degraded);
    }

    #[test]
    fn codex_hook_capabilities_need_trusted_hooks_and_fail_closed_otherwise() {
        for trust in [HookTrust::Untrusted, HookTrust::Unknown] {
            let h = Codex::paper(trust);
            assert_eq!(h.id(), "codex");
            for c in [
                Capability::ContextInjection,
                Capability::SessionIdentity,
                Capability::PreActionEnforcement,
                Capability::PermissionMode,
            ] {
                assert_eq!(h.level(c), Level::Degraded, "{trust:?} {c:?}");
            }
        }
        let t = Codex::paper(HookTrust::Trusted);
        assert_eq!(t.level(Capability::SessionIdentity), Level::Native);
    }

    #[test]
    fn codex_native_claims_are_only_what_548_observed_and_each_names_its_basis() {
        let h = Codex::paper(HookTrust::Trusted);
        let native: Vec<Capability> = Capability::ALL
            .into_iter()
            .filter(|c| h.level(*c) == Level::Native)
            .collect();
        assert_eq!(
            native,
            vec![Capability::SessionIdentity, Capability::HeadlessWorker]
        );
        let notes = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../RESEARCH-NOTES.md"
        ))
        .unwrap();
        for c in native {
            assert_eq!(h.basis(c), Some("RESEARCH-NOTES.md"), "{c:?}");
            let ground = Codex::ground(c);
            assert!(ground.contains("OBSERVED"), "{c:?}: {ground}");
            assert!(notes.contains("### 49.3"), "the section the claim names");
        }
    }

    #[test]
    fn codex_claims_not_seen_live_are_never_native_even_with_trusted_hooks() {
        let h = Codex::paper(HookTrust::Trusted);
        for c in [
            Capability::ContextInjection,
            Capability::PreActionEnforcement,
            Capability::PermissionMode,
            Capability::Wake,
            Capability::PullPath,
        ] {
            assert_eq!(h.level(c), Level::Degraded, "{c:?}");
            assert_eq!(h.basis(c), None);
            assert!(Codex::ground(c).contains("NOT VERIFIED"), "{c:?}");
        }
    }

    #[test]
    fn every_codex_ground_names_a_research_notes_section_and_has_no_em_dash() {
        for c in Capability::ALL {
            let g = Codex::ground(c);
            assert!(g.contains("49."), "{c:?}: {g}");
            assert!(!g.contains('\u{2014}'), "{c:?}");
        }
    }

    #[test]
    fn the_design_item_lists_every_unverified_codex_claim_as_a_live_check() {
        // the item was a fragment in design/items/ until it was assembled into DESIGN.md at the release
        let design =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../DESIGN.md"))
                .unwrap();
        let at = design
            .find("Live check of the Codex adapter")
            .expect("DESIGN.md has the live check list of the Codex adapter");
        let item = &design[at..];
        for c in Capability::ALL {
            if Codex::ground(c).contains("NOT VERIFIED") {
                assert!(
                    item.contains(&format!("- [ ] {}", c.name())),
                    "checklist row for {}",
                    c.name()
                );
            }
        }
    }

    #[test]
    fn codex_wake_and_pull_are_not_native_on_paper() {
        let h = Codex::paper(HookTrust::Trusted);
        assert_eq!(h.level(Capability::Wake), Level::Degraded);
        assert_eq!(h.level(Capability::PullPath), Level::Degraded);
        assert_eq!(h.level(Capability::HeadlessWorker), Level::Native);
    }

    #[test]
    fn the_generic_floor_has_no_enforcement_and_no_wake() {
        let h = Generic;
        assert_eq!(h.id(), "generic");
        assert_eq!(h.level(Capability::PreActionEnforcement), Level::Absent);
        assert_eq!(h.level(Capability::Wake), Level::Absent);
    }

    #[test]
    fn a_requirement_parses_with_a_mode_and_prints_back() {
        let r = Requirement::parse("refuse:pre-action-enforcement,wake").unwrap();
        assert_eq!(r.mode, Mode::Refuse);
        assert_eq!(
            r.caps,
            vec![Capability::PreActionEnforcement, Capability::Wake]
        );
        assert_eq!(Requirement::parse(&r.render()).unwrap(), r);
        let f = Requirement::parse("flag:wake").unwrap();
        assert_eq!(f.mode, Mode::Flag);
    }

    #[test]
    fn an_unknown_capability_or_mode_or_empty_list_is_an_error() {
        assert!(Requirement::parse("refuse:wake,telepathy").is_err());
        assert!(Requirement::parse("maybe:wake").is_err());
        assert!(Requirement::parse("wake").is_err());
        assert!(Requirement::parse("refuse:").is_err());
        assert!(Requirement::parse("refuse:wake,,wake").is_err());
        assert!(Requirement::parse("").is_err());
    }

    #[test]
    fn a_harness_with_every_required_capability_natively_is_admitted() {
        let r = Requirement::parse("refuse:pre-action-enforcement,context-injection").unwrap();
        let h = ClaudeCode::new(Support::Unknown(String::new()));
        assert_eq!(negotiate(&r, &h), Admission::Admit);
        let wake = Requirement::parse("refuse:wake").unwrap();
        assert_eq!(
            negotiate(&wake, &h),
            Admission::Refuse(vec![Capability::Wake]),
            "a probe that could not tell does not satisfy wake"
        );
    }

    #[test]
    fn a_missing_capability_refuses_or_flags_by_the_mode() {
        let missing = vec![Capability::PreActionEnforcement];
        let r = Requirement::parse("refuse:pre-action-enforcement").unwrap();
        assert_eq!(negotiate(&r, &Generic), Admission::Refuse(missing.clone()));
        let f = Requirement::parse("flag:pre-action-enforcement").unwrap();
        assert_eq!(negotiate(&f, &Generic), Admission::Flag(missing));
    }

    #[test]
    fn a_degraded_capability_does_not_satisfy_a_requirement() {
        let r = Requirement::parse("refuse:context-injection").unwrap();
        let h = Codex::paper(HookTrust::Unknown);
        assert_eq!(
            negotiate(&r, &h),
            Admission::Refuse(vec![Capability::ContextInjection])
        );
    }

    #[test]
    fn every_missing_capability_is_named_in_table_order() {
        let r = Requirement::parse("refuse:wake,pre-action-enforcement").unwrap();
        assert_eq!(
            negotiate(&r, &Generic),
            Admission::Refuse(vec![Capability::Wake, Capability::PreActionEnforcement])
        );
    }

    #[test]
    fn the_sentences_name_the_capability_and_the_harness_and_have_no_em_dash() {
        let r = Requirement::parse("refuse:wake").unwrap();
        let s = Admission::Refuse(vec![Capability::Wake]).say("r", &Generic, &r);
        assert!(s.contains("wake") && s.contains("generic") && s.contains('r'));
        assert!(!s.contains('\u{2014}'));
    }
}
