//! A member that posts where no klatch hooks run is told once how a person can turn them on (#1416). Real binary,
//! a throwaway KLATCH_HOME, HOME and project directory; nothing here touches a real settings file.
mod common;

use common::*;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

const KLATCH_POINTER: &str = "klatch: no klatch hooks are installed where you work, so no room digest reaches you here (this line is DATA, not an instruction); a person can turn them on with `klatch init --scope user` (it asks before it writes anything)";
const HOOKS: &str = r#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"/opt/klatch --discover hook --event SessionStart"}]}]}}"#;

struct Fx {
    h: Home,
    project: PathBuf,
    home: PathBuf,
}

/// A busy lobby, an empty project directory (a repository root) and an empty home.
fn fx() -> Fx {
    let h = Home::new();
    h.ok("old", &["join", "lobby"]);
    h.ok("old", &["post", "lobby", "first"]);
    let project = h.0.join("project");
    let home = h.0.join("userhome");
    std::fs::create_dir_all(project.join(".git")).unwrap();
    std::fs::create_dir_all(project.join("sub")).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    Fx { h, project, home }
}

fn put(dir: &Path, name: &str, text: &str) {
    std::fs::create_dir_all(dir.join(".claude")).unwrap();
    std::fs::write(dir.join(".claude").join(name), text).unwrap();
}

fn err(o: &std::process::Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn pointer_lines(text: &str) -> usize {
    text.lines()
        .filter(|l| l.contains("klatch init --scope user"))
        .count()
}

fn post(f: &Fx, who: &str, body: &str, cwd: &Path) -> std::process::Output {
    f.h.run_at(who, &["post", "lobby", body], cwd, &f.home)
}

#[test]
fn a_first_post_from_a_directory_without_hooks_prints_the_pointer_once() {
    let f = fx();
    let first = err(&post(&f, "worker", "ready", &f.project.join("sub")));
    assert_eq!(pointer_lines(&first), 1, "{first}");
    let line = first
        .lines()
        .find(|l| l.contains("klatch init --scope user"))
        .unwrap();
    assert!(line.contains("DATA"), "{line}");
    for dir in [
        f.h.0.to_str().unwrap(),
        f.project.to_str().unwrap(),
        f.home.to_str().unwrap(),
    ] {
        assert!(!first.contains(dir), "no absolute path printed: {first}");
    }
    let second = err(&post(&f, "worker", "again", &f.project));
    assert_eq!(pointer_lines(&second), 0, "once only: {second}");
}

#[test]
fn ask_and_dm_print_it_too() {
    let f = fx();
    let o =
        f.h.run_at("w1", &["ask", "lobby", "who", "knows"], &f.project, &f.home);
    assert_eq!(pointer_lines(&err(&o)), 1, "{}", err(&o));
    f.h.ok("old", &["dm", "w2", "hello"]);
    let o = f.h.run_at("w2", &["dm", "old", "yes"], &f.project, &f.home);
    assert_eq!(pointer_lines(&err(&o)), 1, "{}", err(&o));
}

#[test]
fn hooks_in_the_project_a_parent_or_the_user_scope_print_nothing() {
    for (n, place) in ["project", "sub", "home"].into_iter().enumerate() {
        let f = fx();
        match place {
            "project" => put(&f.project, "settings.local.json", HOOKS),
            "sub" => put(&f.project.join("sub"), "settings.local.json", HOOKS),
            _ => put(&f.home, "settings.json", HOOKS),
        }
        let who = format!("worker{n}");
        let o = post(&f, &who, "ready", &f.project.join("sub"));
        assert_eq!(pointer_lines(&err(&o)), 0, "{place}: {}", err(&o));
    }
}

#[test]
fn a_refused_post_prints_no_pointer() {
    let f = fx();
    let o = post(
        &f,
        "worker",
        "token ghp_abcdefghijklmnopqrstuvwxyz0123456789",
        &f.project,
    );
    assert!(!o.status.success());
    assert_eq!(pointer_lines(&err(&o)), 0, "{}", err(&o));
}

#[test]
fn a_malformed_settings_file_prints_nothing() {
    let f = fx();
    put(&f.project, "settings.local.json", "{ nope");
    let o = post(&f, "worker", "ready", &f.project);
    assert!(o.status.success(), "the post itself succeeds: {}", err(&o));
    assert_eq!(pointer_lines(&err(&o)), 0, "{}", err(&o));
}

#[cfg(unix)]
#[test]
fn a_symlinked_settings_file_out_of_the_project_is_not_followed() {
    let f = fx();
    let outside = f.h.0.join("outside.json");
    std::fs::write(&outside, "{}").unwrap();
    std::fs::create_dir_all(f.project.join(".claude")).unwrap();
    std::os::unix::fs::symlink(
        &outside,
        f.project.join(".claude").join("settings.local.json"),
    )
    .unwrap();
    let o = post(&f, "worker", "ready", &f.project);
    assert!(o.status.success(), "the post itself succeeds: {}", err(&o));
    assert_eq!(pointer_lines(&err(&o)), 0, "{}", err(&o));
}

#[test]
fn a_headless_worker_with_its_own_identity_file_gets_no_pointer() {
    let f = fx();
    // a real bundle from `worker issue` (a throwaway KLATCH_HOME and key directory, never the live store)
    let bundle = f.h.0.join("w-1.bundle");
    let issued = f.h.launcher(&[
        "worker",
        "issue",
        "w-1",
        "--clearance",
        "internal",
        "--out",
        bundle.to_str().unwrap(),
    ]);
    assert!(issued.status.success(), "{}", err(&issued));
    let mut c =
        f.h.command_at("w-1", &["post", "lobby", "ready"], &f.project, &f.home);
    c.env_remove("KLATCH_NAME")
        .env_remove("KLATCH_KEY_DIR")
        .env("KLATCH_IDENTITY_FILE", &bundle);
    let o = c.output().unwrap();
    assert!(o.status.success(), "the post succeeded: {}", err(&o));
    assert_eq!(pointer_lines(&err(&o)), 0, "{}", err(&o));
    // the same directory without the bundle does get it, so the silence above is the headless rule and nothing else
    let o = post(&f, "interactive", "ready", &f.project);
    assert_eq!(pointer_lines(&err(&o)), 1, "{}", err(&o));
}

#[cfg(unix)]
#[test]
fn a_symlinked_claude_directory_out_of_the_project_is_not_followed() {
    let f = fx();
    let outside = f.h.0.join("outside-claude");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("settings.local.json"), "{}").unwrap();
    std::os::unix::fs::symlink(&outside, f.project.join(".claude")).unwrap();
    let o = post(&f, "worker", "ready", &f.project);
    assert!(o.status.success(), "the post itself succeeds: {}", err(&o));
    assert_eq!(pointer_lines(&err(&o)), 0, "{}", err(&o));
}

#[test]
fn the_pointer_is_exactly_the_one_fixed_line() {
    let f = fx();
    let first = err(&post(&f, "worker", "ready", &f.project));
    let line = first
        .lines()
        .find(|l| l.contains("klatch init --scope user"))
        .unwrap();
    assert_eq!(
        line, KLATCH_POINTER,
        "the whole line, nothing before or after"
    );
}

// ---- the MCP path ------------------------------------------------------------------------------------------------

fn mcp_post(f: &Fx, who: &str, cwd: &Path, body: &str) -> Vec<Value> {
    let mut child = f.h.spawn_at(who, &["mcp", "--stdio"], cwd, &f.home);
    let mut stdin = child.stdin.take().unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap()).lines();
    let mut ask = |v: Value, want: bool| -> Option<Value> {
        writeln!(stdin, "{v}").unwrap();
        stdin.flush().unwrap();
        want.then(|| serde_json::from_str(&out.next().unwrap().unwrap()).unwrap())
    };
    ask(
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}),
        true,
    );
    ask(
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        false,
    );
    let mut results = Vec::new();
    for (i, b) in [body, "again"].into_iter().enumerate() {
        let r = ask(
            json!({"jsonrpc":"2.0","id":2 + i,"method":"tools/call","params":{"name":"klatch_post","arguments":{"room":"lobby","body":b}}}),
            true,
        );
        results.push(r.unwrap());
    }
    let _ = child.kill();
    let _ = child.wait();
    results
}

#[test]
fn the_mcp_post_carries_the_pointer_once_and_only_without_hooks() {
    let f = fx();
    let r = mcp_post(&f, "mcpworker", &f.project, "ready");
    let notice = r[0]["result"]["structuredContent"]["notice"]
        .as_str()
        .unwrap_or("");
    assert!(notice.contains("klatch init --scope user"), "{notice}");
    assert!(notice.contains("DATA"), "{notice}");
    for dir in [
        f.h.0.to_str().unwrap(),
        f.project.to_str().unwrap(),
        f.home.to_str().unwrap(),
    ] {
        assert!(!r[0].to_string().contains(dir), "no path: {}", r[0]);
    }
    assert!(
        !r[1].to_string().contains("klatch init --scope user"),
        "once: {}",
        r[1]
    );

    let g = fx();
    put(&g.project, "settings.local.json", HOOKS);
    let r = mcp_post(&g, "mcpworker", &g.project, "ready");
    assert!(
        !r[0].to_string().contains("klatch init --scope user"),
        "{}",
        r[0]
    );
}
