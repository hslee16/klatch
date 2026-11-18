//! The member model (#181, DESIGN section 24, R41 and R42): a stable id with the repository and directory klatch
//! observed. The repository id comes from the git common directory read from files, never typed, and it is
//! an observation, not authority.

use super::*;
use std::path::{Path, PathBuf};

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir()
        .join(format!("klatch-attrs-{}-{tag}", std::process::id()))
        .join(format!(
            "{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
    std::fs::create_dir_all(&d).unwrap();
    std::fs::canonicalize(&d).unwrap()
}

fn store(root: &Path) -> SqliteStore {
    SqliteStore::open(&root.join("t.db"), &klatch::store::insecure_test_key()).unwrap()
}

/// A repository laid out by hand (no git binary): `<root>/<name>/.git` with a source subdirectory.
fn fake_repo(root: &Path, name: &str) -> PathBuf {
    let repo = root.join(name);
    std::fs::create_dir_all(repo.join(".git/worktrees")).unwrap();
    std::fs::create_dir_all(repo.join("src/deep")).unwrap();
    repo
}

/// A linked worktree of `repo` as `git worktree add` lays it out: the worktree's `.git` is a file naming
/// `<repo>/.git/worktrees/<name>`, which has a `commondir` file pointing back at `<repo>/.git`.
fn fake_worktree(root: &Path, repo: &Path, name: &str, absolute: bool) -> PathBuf {
    let wt = root.join(name);
    std::fs::create_dir_all(wt.join("src")).unwrap();
    let admin = repo.join(".git/worktrees").join(name);
    std::fs::create_dir_all(&admin).unwrap();
    std::fs::write(admin.join("commondir"), "../..\n").unwrap();
    let target = if absolute {
        admin.to_string_lossy().into_owned()
    } else {
        // relative to the directory holding the `.git` file
        format!(
            "../{}/.git/worktrees/{name}",
            repo.file_name().unwrap().to_string_lossy()
        )
    };
    std::fs::write(wt.join(".git"), format!("gitdir: {target}\n")).unwrap();
    wt
}

fn dir(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

#[test]
fn a_repository_its_subdirectories_and_its_worktrees_share_one_repository_id() {
    let root = tmp("layout");
    let a = fake_repo(&root, "proj");
    let wt_abs = fake_worktree(&root, &a, "wt-abs", true);
    let wt_rel = fake_worktree(&root, &a, "wt-rel", false);
    let b = fake_repo(&root, "other");
    let id = |p: &Path| repository_for_dir(&dir(p));
    let top = id(&a).expect("a repository has an id");
    assert!(top.starts_with("git:proj-"), "{top}");
    assert_eq!(id(&a.join("src/deep")), Some(top.clone()), "a subdirectory");
    assert_eq!(
        id(&wt_abs),
        Some(top.clone()),
        "a worktree, absolute gitdir"
    );
    assert_eq!(
        id(&wt_abs.join("src")),
        Some(top.clone()),
        "inside a worktree"
    );
    assert_eq!(
        id(&wt_rel),
        Some(top.clone()),
        "a worktree, relative gitdir"
    );
    let other = id(&b).unwrap();
    assert_ne!(other, top, "another repository has its own id");
    assert!(klatch::domain::valid_scope(&top));
}

#[test]
fn a_directory_in_no_repository_has_no_repository_id() {
    let root = tmp("plain");
    assert_eq!(repository_for_dir(&dir(&root)), None);
    assert_eq!(
        repository_for_dir("/definitely/not/a/directory/klatch"),
        None
    );
}

#[test]
fn a_dot_git_file_that_is_not_a_gitdir_line_is_not_a_repository() {
    let root = tmp("junk");
    let d = root.join("junk");
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join(".git"), "hello\n").unwrap();
    assert_eq!(repository_for_dir(&dir(&d)), None);
}

#[test]
fn the_repository_id_agrees_with_the_scope_derived_for_the_same_directory() {
    let root = tmp("agree");
    let a = fake_repo(&root, "proj");
    let wt = fake_worktree(&root, &a, "wt", true);
    assert_eq!(
        repository_for_dir(&dir(&wt)),
        scope_for_dir(&dir(&wt)),
        "one derivation behind both"
    );
}

#[test]
fn two_sessions_in_one_repository_are_two_members_that_share_a_repository_attribute() {
    let root = tmp("two");
    let s = store(&root);
    let a = fake_repo(&root, "proj");
    let wt = fake_worktree(&root, &a, "wt", true);
    // same name from the harness, as two sessions of one project have
    let m1 = crate::identity::ensure_session_member(&s, "sid-aaaa1111", Some("proj"), 1).unwrap();
    let m2 = crate::identity::ensure_session_member(&s, "sid-bbbb2222", Some("proj"), 2).unwrap();
    assert_ne!(m1, m2, "same-named sessions stay distinct");
    ensure_member_attrs(&s, &m1, Some("sid-aaaa1111"), Some(&dir(&a)), 3).unwrap();
    ensure_member_attrs(&s, &m2, Some("sid-bbbb2222"), Some(&dir(&wt)), 4).unwrap();
    let (x, y) = (
        s.member_attrs(&m1).unwrap().unwrap(),
        s.member_attrs(&m2).unwrap().unwrap(),
    );
    assert_eq!(x.repository, y.repository, "one repository, two members");
    assert!(x.repository.starts_with("git:proj-"));
    assert_ne!(x.member_id, y.member_id, "two members, two ids");
    assert_eq!(x.directory, dir(&a));
    assert_eq!(
        y.directory,
        dir(&wt),
        "the worktree is the second member's directory"
    );
    let mut both = vec![m1.clone(), m2.clone()];
    both.sort();
    assert_eq!(s.members_in_repository(&x.repository).unwrap(), both);
}

#[test]
fn a_rename_keeps_one_member_and_its_id_and_a_move_updates_only_the_attributes() {
    let root = tmp("rename");
    let s = store(&root);
    let a = fake_repo(&root, "proj");
    let b = fake_repo(&root, "elsewhere");
    let m = crate::identity::ensure_session_member(&s, "sid-aaaa1111", Some("proj"), 1).unwrap();
    ensure_member_attrs(&s, &m, Some("sid-aaaa1111"), Some(&dir(&a)), 2).unwrap();
    let first = s.member_attrs(&m).unwrap().unwrap();
    // the harness renames the session: still the same member
    let again =
        crate::identity::ensure_session_member(&s, "sid-aaaa1111", Some("renamed"), 3).unwrap();
    assert_eq!(again, m);
    ensure_member_attrs(&s, &again, Some("sid-aaaa1111"), Some(&dir(&a)), 4).unwrap();
    assert_eq!(
        s.member_attrs(&m).unwrap().unwrap(),
        first,
        "nothing changed"
    );
    // the session moves to another repository: the id stays, the observation follows
    ensure_member_attrs(&s, &m, Some("sid-aaaa1111"), Some(&dir(&b)), 5).unwrap();
    let moved = s.member_attrs(&m).unwrap().unwrap();
    assert_eq!(moved.member_id, first.member_id);
    assert_ne!(moved.repository, first.repository);
    assert_eq!(moved.directory, dir(&b));
    assert!(
        s.members_in_repository(&first.repository)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_member_with_no_directory_is_left_alone_and_one_outside_a_repository_has_an_empty_repository() {
    let root = tmp("none");
    let s = store(&root);
    ensure_member_attrs(&s, "bot", None, None, 1).unwrap();
    assert_eq!(
        s.member_attrs("bot").unwrap(),
        None,
        "nothing observed, nothing recorded"
    );
    ensure_member_attrs(&s, "bot", Some("sid-1"), Some(&dir(&root)), 2).unwrap();
    let a = s.member_attrs("bot").unwrap().unwrap();
    assert_eq!(a.repository, "");
    assert_eq!(a.directory, dir(&root));
    assert!(
        s.members_in_repository("").unwrap().is_empty(),
        "no repository is not a group"
    );
}

#[test]
fn the_member_id_is_the_same_for_a_session_on_every_run_and_differs_between_sessions() {
    assert_eq!(
        member_id_for("m", Some("sid-1")),
        member_id_for("other", Some("sid-1"))
    );
    assert_ne!(
        member_id_for("m", Some("sid-1")),
        member_id_for("m", Some("sid-2"))
    );
    assert_ne!(member_id_for("a", None), member_id_for("b", None));
    assert!(member_id_for("m", None).starts_with("m-"));
}
