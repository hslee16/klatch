//! #1083: an ssh session that is already open ends when the human unmaps its key. A child `klatch serve --stdio
//! --ssh-key a` is held open (its input stays open), the key is unmapped through the library as the human would at a
//! terminal, and the next request is refused and the child exits by itself. No ssh and no network are involved.

#[path = "common/mod.rs"]
mod common;

use common::{Home, request};
use klatch::domain::Principal;
use klatch::gateway::{AllowAll, Gateway};
use klatch::member::{CliClock, custody, db_path};
use klatch::protocol::{ServerFrame, decode_server_line};
use std::io::{BufRead as _, Write as _};
use std::process::{Child, ChildStdin, Command, Stdio};

fn with_store<T>(home: &Home, f: impl FnOnce(&Gateway<klatch::store::SqliteStore>) -> T) -> T {
    // SAFETY: the only test in this file, so nothing else touches the process environment.
    unsafe {
        std::env::set_var("HOME", &home.0);
        std::env::set_var("KLATCH_HOME", &home.0);
        std::env::set_var("KLATCH_KEY_DIR", home.0.join("keys"));
        std::env::remove_var("KLATCH_IDENTITY_FILE");
    }
    let store = custody::open_store(&db_path().unwrap()).unwrap();
    f(&Gateway::new(store, CliClock, AllowAll))
}

/// `klatch serve --stdio --ssh-key a` with a pipe on standard input and output, as an `authorized_keys` forced command.
fn spawn_session(home: &Home) -> Child {
    Command::new(env!("CARGO_BIN_EXE_klatch"))
        .env_remove("KLATCH_IDENTITY_FILE")
        .env("KLATCH_HOME", &home.0)
        .env("KLATCH_KEY_DIR", home.0.join("keys"))
        .args(["serve", "--stdio", "--ssh-key", "a"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

/// Send one post of `body` to room r as request `id`.
fn send_post(stdin: &mut ChildStdin, id: &str, body: &str) {
    let params = format!(r#"{{"room":"r","body":"{body}"}}"#);
    writeln!(stdin, "{}", request(id, "post", &params)).unwrap();
    stdin.flush().unwrap();
}

#[test]
fn an_open_session_ends_with_a_refusal_when_its_key_is_unmapped() {
    let home = Home::new();
    let human = Principal::unattributed("admin");
    with_store(&home, |g| {
        g.admin_set_ssh_key(&human, "a", "member-a", true).unwrap()
    });

    let mut child = spawn_session(&home);
    let mut stdin = child.stdin.take().unwrap();
    let mut lines = std::io::BufReader::new(child.stdout.take().unwrap()).lines();
    let mut until = |stop: &dyn Fn(&ServerFrame) -> bool| -> Vec<ServerFrame> {
        let mut seen = Vec::new();
        for line in lines.by_ref() {
            let f = decode_server_line(&line.unwrap()).unwrap();
            let done = stop(&f);
            seen.push(f);
            if done {
                break;
            }
        }
        seen
    };
    let answers = |id: &'static str| move |f: &ServerFrame| matches!(f, ServerFrame::Response { id: i, .. } if i == id);

    send_post(&mut stdin, "p1", "while mapped");
    let frames = until(&answers("p1"));
    assert!(
        matches!(frames.last(), Some(ServerFrame::Response { ok: true, .. })),
        "{frames:?}"
    );

    // the human unmaps the key while the session is open
    with_store(&home, |g| {
        g.admin_set_ssh_key(&human, "a", "member-a", false).unwrap()
    });

    send_post(&mut stdin, "p2", "after unmap");
    let frames = until(&answers("p2"));
    assert!(
        frames.iter().any(|f| matches!(
            f,
            ServerFrame::Refusal { request: Some(i), rule, .. } if i == "p2" && rule == "ssh-key-unmapped"
        )),
        "{frames:?}"
    );
    assert!(matches!(
        frames.last(),
        Some(ServerFrame::Response { ok: false, .. })
    ));

    // the session ends by itself with its input still open
    let status = child.wait().unwrap();
    assert!(status.success(), "{status:?}");
    drop(stdin);

    // and nothing was posted after the unmap
    with_store(&home, |g| {
        let other = Principal::unattributed("other");
        g.join(&other, "r").unwrap();
        let bodies: Vec<String> = g
            .read(&other, "r", true)
            .unwrap()
            .into_iter()
            .map(|m| m.body)
            .collect();
        assert_eq!(bodies, vec!["while mapped".to_string()]);
    });
}
