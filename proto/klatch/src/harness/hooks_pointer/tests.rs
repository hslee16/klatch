use super::*;
use crate::store::test_dir::TestDir;
use std::fs;

const HOOKS: &str = r#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"/usr/local/bin/klatch --discover hook --event SessionStart"}]}]}}"#;

fn settings(dir: &Path, name: &str, text: &str) {
    fs::create_dir_all(dir.join(".claude")).unwrap();
    fs::write(dir.join(".claude").join(name), text).unwrap();
}

/// A repository with a nested working directory, and a separate home.
struct Fx {
    _t: TestDir,
    repo: PathBuf,
    cwd: PathBuf,
    home: PathBuf,
}

fn fx() -> Fx {
    let t = TestDir::new("hooks-pointer");
    let repo = t.join("repo");
    let cwd = repo.join("sub").join("deep");
    let home = t.join("home");
    fs::create_dir_all(repo.join(".git")).unwrap();
    fs::create_dir_all(&cwd).unwrap();
    fs::create_dir_all(&home).unwrap();
    Fx {
        _t: t,
        repo,
        cwd,
        home,
    }
}

fn ask(f: &Fx) -> Option<&'static str> {
    no_hooks_pointer(&f.cwd, Some(&f.home), false)
}

#[test]
fn a_directory_with_no_hooks_anywhere_gets_the_pointer() {
    let f = fx();
    let p = ask(&f).expect("pointer");
    assert!(p.contains("klatch init --scope user"));
    assert!(p.contains("DATA"));
    assert!(!p.contains(f.repo.to_str().unwrap()) && !p.contains(f.home.to_str().unwrap()));
    assert!(!p.contains('\u{2014}') && p.len() < 400);
}

#[test]
fn hooks_in_the_working_directory_project_parent_or_user_scope_silence_it() {
    for place in ["cwd", "repo", "home"] {
        let f = fx();
        let dir = match place {
            "cwd" => &f.cwd,
            "repo" => &f.repo,
            _ => &f.home,
        };
        let file = if place == "home" {
            "settings.json"
        } else {
            "settings.local.json"
        };
        settings(dir, file, HOOKS);
        assert_eq!(ask(&f), None, "{place}");
    }
}

#[test]
fn hooks_in_the_shared_project_settings_silence_it() {
    let f = fx();
    settings(&f.repo, "settings.json", HOOKS);
    assert_eq!(ask(&f), None);
}

#[test]
fn another_tools_hooks_do_not_count() {
    let f = fx();
    settings(
        &f.repo,
        "settings.local.json",
        r#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"echo hi"}]}]}}"#,
    );
    assert!(ask(&f).is_some());
    let g = fx();
    settings(
        &g.repo,
        "settings.local.json",
        r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"/bin/other hook --event Stop"}]}]}}"#,
    );
    assert!(ask(&g).is_some(), "a hook-shaped command of another tool");
}

#[test]
fn the_walk_stops_at_the_repository_root() {
    let f = fx();
    // hooks above the repository root belong to another project
    settings(f.repo.parent().unwrap(), "settings.local.json", HOOKS);
    assert!(ask(&f).is_some());
}

#[test]
fn a_headless_worker_gets_nothing() {
    let f = fx();
    assert_eq!(no_hooks_pointer(&f.cwd, Some(&f.home), true), None);
}

#[test]
fn a_malformed_or_unreadable_settings_file_prints_nothing() {
    let f = fx();
    settings(&f.repo, "settings.local.json", "{ not json");
    assert_eq!(ask(&f), None);
    let g = fx();
    settings(&g.home, "settings.json", "[1,2");
    assert_eq!(ask(&g), None);
    let h = fx();
    fs::create_dir_all(h.repo.join(".claude").join("settings.local.json")).unwrap();
    assert_eq!(ask(&h), None, "a directory where the file should be");
}

#[test]
fn an_oversized_settings_file_prints_nothing() {
    let f = fx();
    let pad = " ".repeat(2 * 1024 * 1024);
    settings(&f.repo, "settings.local.json", &format!("{{}}{pad}"));
    assert_eq!(ask(&f), None);
}

#[cfg(unix)]
#[test]
fn a_settings_symlink_out_of_the_project_is_not_followed() {
    let f = fx();
    let outside = f._t.join("elsewhere.json");
    fs::write(&outside, "{}").unwrap();
    fs::create_dir_all(f.repo.join(".claude")).unwrap();
    std::os::unix::fs::symlink(&outside, f.repo.join(".claude").join("settings.local.json"))
        .unwrap();
    // followed it would read as "no hooks" and print; not followed it is unknown and says nothing
    assert_eq!(ask(&f), None);
}

#[cfg(unix)]
#[test]
fn a_settings_symlink_inside_the_project_is_followed() {
    let f = fx();
    fs::write(f.repo.join("shared.json"), HOOKS).unwrap();
    fs::create_dir_all(f.repo.join(".claude")).unwrap();
    std::os::unix::fs::symlink(
        f.repo.join("shared.json"),
        f.repo.join(".claude").join("settings.local.json"),
    )
    .unwrap();
    assert_eq!(ask(&f), None);
}

#[test]
fn without_a_repository_only_the_working_directory_is_read() {
    let t = TestDir::new("hooks-pointer-norepo");
    let cwd = t.join("loose").join("dir");
    fs::create_dir_all(&cwd).unwrap();
    settings(&t.join("loose"), "settings.local.json", HOOKS);
    assert!(no_hooks_pointer(&cwd, None, false).is_some());
    settings(&cwd, "settings.local.json", HOOKS);
    assert_eq!(no_hooks_pointer(&cwd, None, false), None);
}

#[test]
fn a_settings_directory_that_is_really_a_file_prints_nothing() {
    let f = fx();
    fs::write(f.repo.join(".claude"), "x").unwrap();
    assert_eq!(ask(&f), None, "cannot tell, so say nothing");
}
