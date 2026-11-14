//! Who is calling: the evidence the process has about its caller (a typed name, or the coding-agent session found
//! above it), the stable member name each session keeps, and the principal the gateway acts for.

use super::*;

pub(crate) type Finder<'a> = &'a dyn Fn() -> Result<Found, Miss>;

/// What the process knows about its caller. A name the user gave (flag or environment) wins and the registry
/// is not consulted; otherwise, when discovery was asked for, the session found above this process. A miss is
/// explained in the returned notice and falls back to no evidence, so the identity says how to give a name.
pub(crate) fn gather_evidence(
    declared: Option<String>,
    find: Option<Finder<'_>>,
) -> (PeerEvidence, Option<String>) {
    if let Some(name) = declared {
        return (PeerEvidence::DeclaredName(name), None);
    }
    let Some(find) = find else {
        return (PeerEvidence::None, None);
    };
    match find() {
        Ok(f) => (
            PeerEvidence::Session {
                session_id: f.session_id,
                name: f.name,
            },
            None,
        ),
        Err(Miss::NoSession) => (
            PeerEvidence::None,
            Some(
                "no running session was found above this process; falling back to a typed name"
                    .into(),
            ),
        ),
        Err(Miss::Stale(why)) => (
            PeerEvidence::None,
            Some(format!(
                "a registry record was found above this process but is stale ({why:?}); falling back to a typed name"
            )),
        ),
    }
}

/// The caller of a hook, from what Claude Code says it started the hook for. `stdin_session` is the session id on
/// the hook's standard input; `found` is what the walk up the process tree reached; `remembered` is the member
/// the store has for a session id. A record whose session id is not the one the hook was started for belongs to
/// another session (the parent that started this one, say) and is never used (ticket #284).
pub(crate) fn hook_evidence(
    stdin_session: Option<&str>,
    found: Result<Found, Miss>,
    remembered: &dyn Fn(&str) -> Option<String>,
) -> (PeerEvidence, Option<String>) {
    let Some(sid) = stdin_session else {
        return gather_evidence(None, Some(&move || found.clone()));
    };
    if let Ok(f) = found
        .as_ref()
        .map_err(|_| ())
        .and_then(|f| match f.session_id == sid {
            true => Ok(f),
            false => Err(()),
        })
    {
        let evidence = PeerEvidence::Session {
            session_id: f.session_id.clone(),
            name: f.name.clone(),
        };
        return (evidence, None);
    }
    match remembered(sid) {
        Some(member) => (PeerEvidence::DeclaredName(member), None),
        None => (PeerEvidence::None, Some(unknown_session(found.is_ok()))),
    }
}

/// Why a hook cannot say which session it was started for.
fn unknown_session(another_session_found: bool) -> String {
    if another_session_found {
        "the session above this process is not the session this hook was started for; not acting as it"
    } else {
        "no record of the session this hook was started for; falling back to a typed name"
    }
    .to_string()
}

/// The member a harness session is. The first name a session gets is kept for its whole life (a rename in the
/// harness does not make a new member), and two different sessions never share a member name: when the name is
/// already held by another session, a few characters of the session id are added (ticket #170).
pub(crate) fn ensure_session_member(
    store: &dyn Store,
    session_id: &str,
    name: Option<&str>,
    now: i64,
) -> Result<String> {
    if let Some(member) = store.member_of_session(session_id)? {
        return Ok(member);
    }
    let evidence = PeerEvidence::Session {
        session_id: session_id.into(),
        name: name.map(String::from),
    };
    let base = LocalIdentity.resolve(&evidence)?.member;
    let held_by_another = |m: &str| -> Result<bool> {
        Ok(store
            .session_of_member(m)?
            .is_some_and(|other| other != session_id))
    };
    let mut member = base.clone();
    if held_by_another(&member)? {
        let id: String = session_id
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .map(|c| c.to_ascii_lowercase())
            .collect();
        let stem: String = base.chars().take(35).collect();
        member = format!("{stem}-{}", id.chars().take(4).collect::<String>());
        let mut more = 4;
        while held_by_another(&member)? && more < id.len() {
            more += 1;
            let stem: String = base.chars().take(40 - 1 - more).collect();
            member = format!("{stem}-{}", id.chars().take(more).collect::<String>());
        }
    }
    store.note_session(session_id, &member, now)?;
    Ok(member)
}

/// The JSON Claude Code puts on a hook's standard input. Reads nothing when standard input is a terminal.
pub(crate) fn stdin_json() -> Option<serde_json::Value> {
    use std::io::{IsTerminal, Read};
    if std::io::stdin().is_terminal() {
        return None;
    }
    // Claude Code writes the JSON at once and closes the pipe. Anything else (a script that holds standard input
    // open, a pipe nobody writes to) must not make a hook wait for ever, so the read is time-bounded.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut text = String::new();
        let _ = std::io::stdin().take(1 << 20).read_to_string(&mut text);
        let _ = tx.send(text);
    });
    let text = rx
        .recv_timeout(std::time::Duration::from_millis(1500))
        .ok()?;
    serde_json::from_str(&text).ok()
}

/// The session id in a hook's input, when it is text.
fn session_id_of(v: &serde_json::Value) -> Option<String> {
    v.get("session_id")?.as_str().map(String::from)
}

/// Look for the session above this process, using the real process table and the harness registry
/// (`$HOME/.claude/sessions`). Reads nothing but the registry's .json files and never the environment.
pub(crate) fn discover_session() -> Result<Found, Miss> {
    let Some((registry, domain)) = registry_and_domain() else {
        return Err(Miss::NoSession);
    };
    find_session(std::process::id(), &PsTable, &registry, domain)
}

/// The harness registry under `$HOME` and the pid domain of this machine, if there is a home.
fn registry_and_domain() -> Option<(FsRegistry, &'static str)> {
    let home = std::env::var_os("HOME")?;
    let registry = FsRegistry {
        dir: PathBuf::from(home).join(".claude").join("sessions"),
    };
    let domain = if cfg!(target_os = "macos") {
        "darwin"
    } else {
        std::env::consts::OS
    };
    Some((registry, domain))
}

/// The session above `session`, with the real process table and registry.
fn discover_parent(session: &Found) -> Option<Found> {
    let (registry, domain) = registry_and_domain()?;
    find_parent(session, &PsTable, &registry, domain)
}

/// The member of a session, named and remembered if it had none (it may never have run klatch itself).
fn member_of(store: &SqliteStore, session: &Found) -> Option<String> {
    ensure_session_member(store, &session.session_id, session.name.as_deref(), now()).ok()
}

/// What to note about the caller's parent: `None` when it was already looked for, else `Some(parent)` where the
/// parent is the member of the session above the caller's, or `None` for a caller with no session above it. A walk
/// up the process tree is made once per member, and a top level session records that it has no parent.
pub(crate) fn pending_parent(store: &SqliteStore, caller: &Caller) -> Option<Option<String>> {
    let session = caller.session.as_ref()?;
    if store
        .parent_of(&caller.principal.member)
        .ok()
        .flatten()
        .is_some()
    {
        return None;
    }
    Some(discover_parent(session).and_then(|p| member_of(store, &p)))
}

/// The caller, decided by the identity from what the transport knows. The CLI knows only a typed name (a
/// flag or the environment), which proves nothing, so with `DeclaredName` the caller is unattributed.
pub(crate) fn resolve_principal(
    identity: &dyn Identity,
    evidence: &PeerEvidence,
) -> Result<Principal> {
    Ok(identity.resolve(evidence)?)
}

/// The caller of this process: who it acts as, where its session started and the session id a hook was given.
pub(crate) struct Caller {
    pub principal: Principal,
    /// The coding-agent session found above this process, when there is one.
    pub session: Option<Found>,
    pub session_dir: Option<String>,
    pub stdin_session: Option<String>,
    /// The whole input of a hook, for events that carry more than the session (a tool call).
    pub stdin: Option<serde_json::Value>,
}

/// Who is calling: the typed name if there is one, else the coding-agent session found above this process, kept to
/// one stable member name. `None` for a SessionEnd hook that cannot tell which session it ends: that does nothing,
/// quietly, because it must never fail a session's exit and never act as anyone else.
pub(crate) fn identify(who: &WhoFlags, cmd: &Cmd, store: &SqliteStore) -> Result<Option<Caller>> {
    // The only thing the CLI knows about its caller is a typed name, which proves nothing.
    let declared = who
        .as_member
        .clone()
        .or_else(|| std::env::var("KLATCH_NAME").ok());
    let discover = who.discover || std::env::var("KLATCH_DISCOVER").is_ok_and(|v| v == "1");
    // A worker holding a key bundle is who the bundle says, proven by its credential, and never anyone else.
    if let Some(path) = std::env::var_os("KLATCH_IDENTITY_FILE") {
        return identify_by_credential(
            std::path::Path::new(&path),
            declared.as_deref(),
            cmd,
            store,
        );
    }
    // A hook gets the session it was started for on standard input. It acts only as that session: a registry
    // record for another session (the parent that started this one) is never used (ticket #284).
    let is_hook = matches!(cmd, Cmd::Hook { .. } | Cmd::Wake { .. });
    let stdin = if is_hook { stdin_json() } else { None };
    let stdin_session = stdin.as_ref().and_then(session_id_of);
    // What the walk up the process tree found, once: it gives the identity and, for a coding session, its directory.
    let found = (declared.is_none() && discover).then(discover_session);
    let session = found.as_ref().and_then(|r| r.as_ref().ok()).cloned();
    let session_dir = session.as_ref().map(|f| f.cwd.clone());
    let probe = Probe {
        declared,
        found,
        is_hook,
        stdin_session: stdin_session.as_deref(),
    };
    let (evidence, notice) = probe.evidence(store);
    // A SessionEnd hook that cannot tell which session it ends does nothing, quietly.
    let session_end = matches!(cmd, Cmd::Hook { event: Some(e), .. } if e == "SessionEnd");
    if session_end && matches!(evidence, PeerEvidence::None) {
        return Ok(None);
    }
    if let Some(n) = notice {
        eprintln!("klatch: {n}");
    }
    let evidence = settle_name(evidence, store)?;
    Ok(Some(Caller {
        principal: resolve_principal(&LocalIdentity, &evidence)?,
        session,
        session_dir,
        stdin_session,
        stdin,
    }))
}

/// The caller is the member of a key bundle, if its credential is live in the registry. A typed name is accepted only
/// if it is the bundle's own; a failure is recorded without the token, and nothing falls back to a weaker identity.
fn identify_by_credential(
    path: &std::path::Path,
    declared: Option<&str>,
    cmd: &Cmd,
    store: &SqliteStore,
) -> Result<Option<Caller>> {
    let bundle = klatch_keys::Bundle::read(path)?;
    if let Some(name) = declared.filter(|n| *n != bundle.member) {
        let _ = name;
        bail!(
            "this process holds a credential for {}, so it cannot act as another member",
            bundle.member
        );
    }
    let at = now();
    if let Err(e) = klatch_gateway::check_credential(store, at, &bundle.member, &bundle.token) {
        let _ = klatch_store::Store::record_refusal(
            store,
            &klatch_store::NewRefusal::new(&bundle.member, "", "credential", "credential"),
            at,
        );
        return Err(e.into());
    }
    let is_hook = matches!(cmd, Cmd::Hook { .. } | Cmd::Wake { .. });
    let stdin = if is_hook { stdin_json() } else { None };
    let stdin_session = stdin.as_ref().and_then(session_id_of);
    let evidence = PeerEvidence::Credential {
        member: bundle.member,
    };
    Ok(Some(Caller {
        principal: resolve_principal(&LocalIdentity, &evidence)?,
        session: None,
        session_dir: None,
        stdin_session,
        stdin,
    }))
}

/// What `identify` found out about the caller before deciding.
struct Probe<'a> {
    declared: Option<String>,
    found: Option<Result<Found, Miss>>,
    is_hook: bool,
    stdin_session: Option<&'a str>,
}

impl Probe<'_> {
    /// The evidence and, when there is none, a notice saying how to give a name.
    fn evidence(self, store: &SqliteStore) -> (PeerEvidence, Option<String>) {
        match (self.declared, self.found, self.is_hook) {
            (None, Some(f), true) => hook_evidence(self.stdin_session, f, &|sid| {
                store.member_of_session(sid).ok().flatten()
            }),
            (None, Some(f), false) => gather_evidence(None, Some(&|| f.clone())),
            (declared, _, _) => gather_evidence(declared, None),
        }
    }
}

/// A session keeps one member name for its life and never shares it with another session (ticket #170).
fn settle_name(evidence: PeerEvidence, store: &SqliteStore) -> Result<PeerEvidence> {
    match evidence {
        PeerEvidence::Session { session_id, name } => {
            let member = ensure_session_member(store, &session_id, name.as_deref(), now())?;
            Ok(PeerEvidence::Session {
                name: Some(member),
                session_id,
            })
        }
        other => Ok(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use klatch_core::Attribution;
    use klatch_gateway::FixedIdentity;
    use klatch_harness::Liveness;

    fn declared(name: Option<&str>) -> PeerEvidence {
        name.map_or(PeerEvidence::None, |n| PeerEvidence::DeclaredName(n.into()))
    }

    fn found(name: Option<&str>) -> Found {
        Found {
            session_id: "sess-0123456789".into(),
            name: name.map(Into::into),
            cwd: "/w".into(),
            pid: 10,
        }
    }

    #[test]
    fn a_name_the_user_gave_wins_and_discovery_is_not_run() {
        let boom = || -> Result<Found, Miss> { panic!("the registry must not be consulted") };
        let (ev, notice) = gather_evidence(Some("alice".into()), Some(&boom));
        assert_eq!(ev, PeerEvidence::DeclaredName("alice".into()));
        assert_eq!(notice, None);
    }

    #[test]
    fn without_a_name_or_discovery_there_is_no_evidence_and_no_registry_read() {
        assert_eq!(gather_evidence(None, None), (PeerEvidence::None, None));
    }

    #[test]
    fn a_discovered_session_becomes_session_evidence() {
        let ok = || Ok(found(Some("klatch-1b")));
        let (ev, notice) = gather_evidence(None, Some(&ok));
        assert_eq!(
            ev,
            PeerEvidence::Session {
                session_id: "sess-0123456789".into(),
                name: Some("klatch-1b".into())
            }
        );
        assert_eq!(notice, None);
    }

    #[test]
    fn a_miss_says_so_and_falls_back_to_no_evidence() {
        let none = || Err(Miss::NoSession);
        let (ev, notice) = gather_evidence(None, Some(&none));
        assert_eq!(ev, PeerEvidence::None);
        assert!(notice.unwrap().contains("no running session"));
        let stale = || Err(Miss::Stale(Liveness::Reused));
        let (ev, notice) = gather_evidence(None, Some(&stale));
        assert_eq!(ev, PeerEvidence::None);
        assert!(notice.unwrap().contains("stale"));
    }

    #[test]
    fn a_discovered_session_is_a_never_verified_caller() {
        let ok = || Ok(found(None));
        let (ev, _) = gather_evidence(None, Some(&ok));
        let p = LocalIdentity.resolve(&ev).unwrap();
        assert_eq!(p.attribution, Attribution::Unattributed);
        assert_eq!(p.member, "session-sess-012");
    }

    #[test]
    fn the_cli_depends_on_the_identity_trait_so_a_fake_identity_decides_the_caller() {
        let fake = FixedIdentity(Principal::verified("carol"));
        let p = resolve_principal(&fake, &declared(Some("whatever"))).unwrap();
        assert_eq!(
            (p.member.as_str(), p.attribution),
            ("carol", Attribution::Verified)
        );
        let p = resolve_principal(&fake, &declared(None)).unwrap();
        assert_eq!(p.member, "carol");
    }

    #[test]
    fn with_the_declared_name_identity_a_typed_name_is_unattributed() {
        let p = resolve_principal(&LocalIdentity, &declared(Some("alice"))).unwrap();
        assert_eq!(
            (p.member.as_str(), p.attribution),
            ("alice", Attribution::Unattributed)
        );
    }

    #[test]
    fn a_missing_or_invalid_name_is_an_error_naming_the_variable() {
        for declared in [None, Some(String::new()), Some("bad name".into())] {
            let evidence = gather_evidence(declared, None).0;
            let err = resolve_principal(&LocalIdentity, &evidence)
                .unwrap_err()
                .to_string();
            assert!(err.contains("KLATCH_NAME"), "{err}");
        }
    }
}

#[cfg(test)]
mod hook_identity_tests {
    use super::*;

    fn found(sid: &str, name: &str) -> Result<Found, Miss> {
        Ok(Found {
            session_id: sid.into(),
            name: Some(name.into()),
            cwd: "/x".into(),
            pid: 1,
        })
    }

    fn none(_: &str) -> Option<String> {
        None
    }

    fn mapped(sid: &str) -> Option<String> {
        (sid == "child").then(|| "proj-child".to_string())
    }

    #[test]
    fn a_record_for_the_session_the_hook_was_started_for_is_used() {
        let (e, notice) = hook_evidence(Some("child"), found("child", "proj-child"), &none);
        assert!(matches!(e, PeerEvidence::Session { ref session_id, .. } if session_id == "child"));
        assert!(notice.is_none());
    }

    #[test]
    fn a_record_for_another_session_is_never_used_and_the_remembered_member_is() {
        let (e, _) = hook_evidence(Some("child"), found("parent", "klatch-1b"), &mapped);
        assert!(
            matches!(e, PeerEvidence::DeclaredName(ref n) if n == "proj-child"),
            "{e:?}"
        );
    }

    #[test]
    fn a_record_for_another_session_with_nothing_remembered_gives_no_identity() {
        let (e, notice) = hook_evidence(Some("stranger"), found("parent", "klatch-1b"), &mapped);
        assert!(
            matches!(e, PeerEvidence::None),
            "acted as the parent: {e:?}"
        );
        assert!(
            notice.is_some_and(|n| n.contains("not the session")),
            "no explanation"
        );
    }

    #[test]
    fn no_record_at_all_falls_back_to_the_remembered_member() {
        let (e, _) = hook_evidence(Some("child"), Err(Miss::NoSession), &mapped);
        assert!(
            matches!(e, PeerEvidence::DeclaredName(ref n) if n == "proj-child"),
            "{e:?}"
        );
        let (e, _) = hook_evidence(Some("child"), Err(Miss::NoSession), &none);
        assert!(matches!(e, PeerEvidence::None));
    }

    #[test]
    fn without_a_session_id_on_standard_input_the_walk_decides_as_before() {
        let (e, _) = hook_evidence(None, found("parent", "klatch-1b"), &mapped);
        assert!(
            matches!(e, PeerEvidence::Session { ref session_id, .. } if session_id == "parent")
        );
    }
}

#[cfg(test)]
mod member_name_tests {
    use super::*;

    fn store() -> SqliteStore {
        let dir = std::env::temp_dir().join(format!(
            "klatch-names-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(format!(
            "{}.db",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        SqliteStore::open(&p, &klatch_store::insecure_test_key()).unwrap()
    }

    #[test]
    fn a_new_session_with_a_free_name_keeps_the_name_and_is_remembered() {
        let s = store();
        assert_eq!(
            ensure_session_member(&s, "sid-aaaa1111", Some("proj-20"), 1).unwrap(),
            "proj-20"
        );
        assert_eq!(
            s.member_of_session("sid-aaaa1111").unwrap().as_deref(),
            Some("proj-20")
        );
    }

    #[test]
    fn a_renamed_session_keeps_its_first_name() {
        let s = store();
        ensure_session_member(&s, "sid-aaaa1111", Some("proj-20"), 1).unwrap();
        assert_eq!(
            ensure_session_member(&s, "sid-aaaa1111", Some("renamed-by-user"), 2).unwrap(),
            "proj-20"
        );
    }

    #[test]
    fn a_second_session_with_the_same_name_gets_a_suffix_from_its_session_id_and_keeps_it() {
        let s = store();
        ensure_session_member(&s, "sid-aaaa1111", Some("proj-20"), 1).unwrap();
        let second = ensure_session_member(&s, "sid-bbbb2222", Some("proj-20"), 2).unwrap();
        assert_ne!(second, "proj-20");
        assert!(
            second.starts_with("proj-20-") && second.len() <= 40,
            "{second}"
        );
        assert_eq!(
            ensure_session_member(&s, "sid-bbbb2222", Some("proj-20"), 3).unwrap(),
            second
        );
        assert_eq!(
            ensure_session_member(&s, "sid-aaaa1111", Some("proj-20"), 4).unwrap(),
            "proj-20",
            "the first is untouched"
        );
    }

    #[test]
    fn a_long_name_still_fits_with_a_suffix_and_a_session_with_no_name_is_named_from_its_id() {
        let s = store();
        let long = "n".repeat(40);
        ensure_session_member(&s, "sid-aaaa1111", Some(&long), 1).unwrap();
        let second = ensure_session_member(&s, "sid-bbbb2222", Some(&long), 2).unwrap();
        assert!(second.len() <= 40 && second != long, "{second}");
        assert_eq!(
            ensure_session_member(&s, "0123456789abcdef", None, 3).unwrap(),
            "session-01234567"
        );
    }
}
