//! The scope a member works in: derived from the directory it started in (a git repository, or the directory), or
//! declared, and remembered per member (DESIGN section 27).

use super::*;

#[cfg(test)]
mod attr_tests;
#[cfg(test)]
mod edge_tests;

/// FNV-1a, a hash that is the same on every run and platform (a scope id must be).
pub(crate) fn fnv1a(text: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in text.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// The scope of a directory: `git:<name>-<hash>` for a git repository (a worktree shares its repository's scope,
/// because the git common directory is the same), else `dir:<name>-<hash>`.
pub(crate) fn scope_for_dir(dir: &str) -> Option<String> {
    let base = std::fs::canonicalize(dir).ok()?;
    let (kind, common, root) = match git_common_dir(&base) {
        Some(common) => ("git", common.clone(), repository_root(&common)),
        None => ("dir", base.clone(), base),
    };
    let name = scope_name(&root);
    let id = fnv1a(&common.to_string_lossy()) as u32;
    let scope = format!("{kind}:{name}-{id:08x}");
    klatch::domain::valid_scope(&scope).then_some(scope)
}

/// The git directory every worktree of the repository containing `base` shares, if `base` is in one. Read from the
/// files git lays out (no git process is run, so nothing in the directory can execute): walk up to the first `.git`;
/// a directory is the common directory itself; a file says `gitdir: <path>` (a linked worktree or a submodule), and
/// the `commondir` file there, if any, points back at the common directory.
fn git_common_dir(base: &std::path::Path) -> Option<std::path::PathBuf> {
    for dir in base.ancestors() {
        let dot_git = dir.join(".git");
        let Ok(meta) = std::fs::symlink_metadata(&dot_git) else {
            continue;
        };
        if meta.is_dir() {
            return std::fs::canonicalize(&dot_git).ok();
        }
        if !meta.is_file() {
            return None;
        }
        let text = std::fs::read_to_string(&dot_git).ok()?;
        let target = text.lines().next()?.trim().strip_prefix("gitdir:")?.trim();
        if target.is_empty() {
            return None;
        }
        let gitdir = std::fs::canonicalize(dir.join(target)).ok()?;
        return match std::fs::read_to_string(gitdir.join("commondir")) {
            Ok(rel) => std::fs::canonicalize(gitdir.join(rel.trim())).ok(),
            Err(_) => Some(gitdir),
        };
    }
    None
}

/// The repository id of a directory: its `git:<name>-<hash>` scope when it is in a git repository, else `None`. It is
/// derived here from the git common directory (so worktrees share it) and never taken from the caller (R42). It is an
/// observation of where a process started; a directory can name another repository's git directory, so the value
/// proves nothing and grants nothing.
pub(crate) fn repository_for_dir(dir: &str) -> Option<String> {
    scope_for_dir(dir).filter(|s| s.starts_with("git:"))
}

/// The id a member keeps for its whole life (R41): the same for one session on every run, and different for two
/// sessions. A caller with no session gets one from its member name. It is an identifier, not a secret or a credential.
pub(crate) fn member_id_for(member: &str, session_id: Option<&str>) -> String {
    let key = match session_id {
        Some(sid) => format!("session\0{sid}"),
        None => format!("member\0{member}"),
    };
    format!("m-{:016x}", fnv1a(&key))
}

/// Record what klatch observes about a member: its directory and the repository derived from it. The member id is
/// made the first time and kept; a later call only updates the repository and directory, and does nothing at all when
/// they are unchanged. With no directory there is nothing to observe. Nothing here is consulted for clearance or
/// access (the repository value is shown and grouped by, never trusted).
pub(crate) fn ensure_member_attrs(
    store: &dyn Store,
    member: &str,
    session_id: Option<&str>,
    dir: Option<&str>,
    now: i64,
) -> Result<()> {
    let Some(dir) = dir else {
        return Ok(());
    };
    let repository = repository_for_dir(dir).unwrap_or_default();
    let existing = store.member_attrs(member)?;
    if existing
        .as_ref()
        .is_some_and(|e| e.repository == repository && e.directory == dir)
    {
        return Ok(());
    }
    let id = existing
        .map(|e| e.member_id)
        .unwrap_or_else(|| member_id_for(member, session_id));
    Ok(store.note_member_attrs(
        member,
        &klatch::domain::MemberAttrs::new(id, repository, dir),
        now,
    )?)
}

/// The repository directory is the parent of `.git`; a bare repository is the directory itself.
fn repository_root(common: &std::path::Path) -> std::path::PathBuf {
    let is_dot_git = common.file_name().is_some_and(|n| n == ".git");
    match common.parent() {
        Some(parent) if is_dot_git => parent.to_path_buf(),
        _ => common.to_path_buf(),
    }
}

/// A directory's name as a scope label: lower case, anything but letters, digits and `. _ -` turned into `-`, at
/// most 40 characters, `repo` when it has none.
fn scope_name(root: &std::path::Path) -> String {
    let keep = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-');
    let name: String = root
        .file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
        .chars()
        .map(|c| if keep(c) { c } else { '-' })
        .take(40)
        .collect();
    if name.is_empty() { "repo".into() } else { name }
}

/// Settle a member's scope: a declared one wins and is remembered, an existing one is kept, and a coding session
/// with none gets the scope of its directory.
pub(crate) fn ensure_member_scope(
    store: &dyn Store,
    member: &str,
    (declared, dir): (Option<&str>, Option<&str>),
    now: i64,
) -> Result<()> {
    let stored = store.member_scope(member)?;
    match (declared, &stored) {
        (Some(d), Some(s)) if d == s => Ok(()),
        (Some(d), _) => {
            if !klatch::domain::valid_scope(d) {
                bail!("bad scope {d:?}: 1 to 80 characters of letters, digits and . : _ - /");
            }
            Ok(store.set_member_scope(member, d, now)?)
        }
        (None, Some(_)) => Ok(()),
        (None, None) => {
            if let Some(scope) = dir.and_then(scope_for_dir) {
                store.set_member_scope(member, &scope, now)?;
            }
            Ok(())
        }
    }
}

/// The kind a member declares: the flag, else `KLATCH_KIND` when it is not empty (#301).
pub(crate) fn declared_kind(flag: Option<&str>) -> Option<String> {
    flag.map(str::to_string)
        .or_else(|| std::env::var("KLATCH_KIND").ok().filter(|k| !k.is_empty()))
}

/// Declare the caller's kind from `--member-kind` or `KLATCH_KIND` (#301): `bot`, `service` or `human`, or `off` to
/// withdraw it. `session` is refused: klatch derives it from the session record. The kind is display only.
pub(crate) fn declare_member_kind(
    gateway: &Gateway<impl Store>,
    who: &Principal,
    word: &str,
) -> Result<()> {
    let kind =
        match word {
            "off" => None,
            w => Some(klatch::domain::MemberKind::parse(w).with_context(|| {
                format!("bad member kind {w:?}: use bot, service, human, or off")
            })?),
        };
    Ok(gateway.set_kind(who, kind)?)
}

#[cfg(test)]
mod scope_dir_tests {
    use super::*;

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir()
            .join(format!(
                "klatch-scope-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ))
            .join(name);
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::canonicalize(&d).unwrap()
    }

    fn git(dir: &std::path::Path, args: &[&str]) -> bool {
        std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .is_ok_and(|o| o.status.success())
    }

    #[test]
    fn a_git_repository_and_its_subdirectory_share_one_scope_and_another_repository_has_its_own() {
        let a = tmp("repo-a");
        let b = tmp("repo-b");
        if !git(&a, &["init", "-q"]) || !git(&b, &["init", "-q"]) {
            return; // no git here: the directory fallback is tested below
        }
        std::fs::create_dir_all(a.join("src/deep")).unwrap();
        let top = scope_for_dir(&a.to_string_lossy()).unwrap();
        let sub = scope_for_dir(&a.join("src/deep").to_string_lossy()).unwrap();
        let other = scope_for_dir(&b.to_string_lossy()).unwrap();
        assert!(top.starts_with("git:repo-a-"), "{top}");
        assert_eq!(top, sub, "a subdirectory is the same repository");
        assert_ne!(top, other);
        assert!(klatch::domain::valid_scope(&top) && top.len() <= 80);
    }

    #[test]
    fn a_worktree_made_by_git_shares_the_repository_id_of_the_one_it_was_made_from() {
        let a = tmp("real-main");
        if !git(&a, &["init", "-q"])
            || !git(
                &a,
                &[
                    "-c",
                    "user.name=t",
                    "-c",
                    "user.email=t@t",
                    "commit",
                    "-q",
                    "--allow-empty",
                    "-m",
                    "x",
                ],
            )
        {
            return;
        }
        let wt = a.parent().unwrap().join("real-linked");
        let _ = std::fs::remove_dir_all(&wt);
        if !git(&a, &["worktree", "add", "-q", &wt.to_string_lossy()]) {
            return;
        }
        assert_eq!(
            repository_for_dir(&a.to_string_lossy()),
            repository_for_dir(&wt.to_string_lossy())
        );
        assert!(repository_for_dir(&a.to_string_lossy()).is_some());
    }

    #[test]
    fn a_directory_that_is_not_a_repository_gets_a_dir_scope_and_a_missing_one_gets_none() {
        let d = tmp("plain");
        let scope = scope_for_dir(&d.to_string_lossy()).unwrap();
        assert!(
            scope.starts_with("dir:") || scope.starts_with("git:"),
            "{scope}"
        );
        assert_eq!(scope_for_dir("/definitely/not/a/directory/klatch"), None);
    }

    #[test]
    fn the_scope_of_a_directory_is_the_same_on_every_run() {
        let d = tmp("stable");
        assert_eq!(
            scope_for_dir(&d.to_string_lossy()),
            scope_for_dir(&d.to_string_lossy())
        );
    }

    #[test]
    fn an_explicit_scope_wins_and_is_kept_and_a_bad_one_is_refused() {
        let dir = tmp("store");
        let s = SqliteStore::open(&dir.join("t.db"), &klatch::store::insecure_test_key()).unwrap();
        ensure_member_scope(&s, "alice", (None, Some(&dir.to_string_lossy())), 1).unwrap();
        let derived = s.member_scope("alice").unwrap().unwrap();
        ensure_member_scope(&s, "alice", (None, Some("/other")), 2).unwrap();
        assert_eq!(
            s.member_scope("alice").unwrap().unwrap(),
            derived,
            "an existing scope is kept"
        );
        ensure_member_scope(&s, "alice", (Some("bot:grok"), None), 3).unwrap();
        assert_eq!(
            s.member_scope("alice").unwrap().as_deref(),
            Some("bot:grok")
        );
        assert!(ensure_member_scope(&s, "alice", (Some("has space"), None), 4).is_err());
    }
}
