//! The scope a member works in: derived from the directory it started in (a git repository, or the directory), or
//! declared, and remembered per member (DESIGN section 27).

use super::*;

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

/// The git directory every worktree of the repository containing `base` shares, if `base` is in one.
fn git_common_dir(base: &std::path::Path) -> Option<std::path::PathBuf> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(base)
        .args(["rev-parse", "--git-common-dir"])
        .stderr(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let printed = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if printed.is_empty() {
        return None;
    }
    let path = std::path::Path::new(&printed);
    std::fs::canonicalize(base.join(path)).ok()
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
