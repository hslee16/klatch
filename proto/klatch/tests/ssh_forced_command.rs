//! #107: `klatch serve --stdio --ssh-key <label>` is what an `authorized_keys` forced command runs. The label is mapped
//! to ONE member by the human (`klatch admin ssh-key`, a terminal act, so the mapping is seeded here through the
//! library into a throwaway store), and the session cannot be anyone else. Two keys, two members, run as child
//! processes against a temporary home: each can post only as itself, and a frame naming the other member, or a forged
//! identity field, is refused. No ssh, no sshd and no network are involved: ssh only carries the bytes, which this
//! test writes to the child's standard input. The real sshd path is not exercised here (see the deployment guide).
//! Seeding sets process environment variables, so it takes a lock; the child processes get their own environment.

#[path = "common/mod.rs"]
mod common;

use common::{Home, request};
use klatch::domain::Principal;
use klatch::gateway::{AllowAll, Gateway};
use klatch::member::{CliClock, custody, db_path};
use klatch::protocol::{Attribution, ServerFrame, decode_server_line};
use std::io::Write as _;
use std::process::{Command, Stdio};

struct Ran {
    frames: Vec<ServerFrame>,
    ok: bool,
    stderr: String,
}

/// Run `klatch serve --stdio <extra>` as sshd would: the command line is the administrator's, the environment is
/// hostile (a name, a discover flag and an original command the remote might have influenced), and standard input is
/// the only thing the remote controls.
fn session(home: &Home, extra: &[&str], lines: &[String]) -> Ran {
    let mut child = Command::new(env!("CARGO_BIN_EXE_klatch"))
        .env_remove("KLATCH_IDENTITY_FILE")
        .env("KLATCH_HOME", &home.0)
        .env("KLATCH_KEY_DIR", home.0.join("keys"))
        .env("KLATCH_NAME", "member-b")
        .env("KLATCH_DISCOVER", "1")
        .env(
            "SSH_ORIGINAL_COMMAND",
            "klatch serve --stdio --as-member member-b",
        )
        .args(["serve", "--stdio"])
        .args(extra)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    {
        let stdin = child.stdin.as_mut().unwrap();
        for l in lines {
            writeln!(stdin, "{l}").unwrap();
        }
    }
    drop(child.stdin.take());
    let out = child.wait_with_output().unwrap();
    let frames = String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|l| decode_server_line(l).unwrap_or_else(|e| panic!("not a frame: {l}: {e:?}")))
        .collect();
    Ran {
        frames,
        ok: out.status.success(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

fn hello(r: &Ran) -> (String, Attribution) {
    match r.frames.first() {
        Some(ServerFrame::Hello {
            member,
            attribution,
            ..
        }) => (member.clone(), attribution.clone()),
        other => panic!("the first frame is hello, got {other:?}"),
    }
}

fn response_ok(r: &Ran, id: &str) -> bool {
    r.frames
        .iter()
        .find_map(|f| match f {
            ServerFrame::Response { id: i, ok, .. } if i == id => Some(*ok),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no response for {id}"))
}

fn refusal_rule(r: &Ran, id: &str) -> Option<String> {
    r.frames.iter().find_map(|f| match f {
        ServerFrame::Refusal {
            request: Some(i),
            rule,
            ..
        } if i == id => Some(rule.clone()),
        _ => None,
    })
}

/// Every message of `room` as `(sender, body)`, read by an ssh session of the given label.
fn room_log(home: &Home, label: &str, room: &str) -> Vec<(String, String)> {
    let r = session(
        home,
        &["--ssh-key", label],
        &[request("h", "history", &format!(r#"{{"room":"{room}"}}"#))],
    );
    let result = r
        .frames
        .iter()
        .find_map(|f| match f {
            ServerFrame::Response {
                result: Some(v), ..
            } => Some(v.clone()),
            _ => None,
        })
        .expect("a history result");
    result["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            (
                m["sender"].as_str().unwrap().to_string(),
                m["body"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

/// Map the labels as the human would at a terminal, into the temporary store.
fn seed(home: &Home, mappings: &[(&str, &str)]) {
    static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _held = ENV.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: every read and write of the process environment in this file is under the lock above (the tests spawn
    // children with an explicit environment and read nothing from their own).
    unsafe {
        std::env::set_var("HOME", &home.0);
        std::env::set_var("KLATCH_HOME", &home.0);
        std::env::set_var("KLATCH_KEY_DIR", home.0.join("keys"));
        std::env::remove_var("KLATCH_IDENTITY_FILE");
    }
    let store = custody::open_store(&db_path().unwrap()).unwrap();
    let gateway = Gateway::new(store, CliClock, AllowAll);
    let human = Principal::unattributed("admin");
    for (label, member) in mappings {
        gateway
            .admin_set_ssh_key(&human, label, member, true)
            .unwrap();
    }
}

#[test]
fn two_keys_map_to_two_members_and_cannot_impersonate_each_other() {
    let home = Home::new();
    seed(&home, &[("a", "member-a"), ("b", "member-b")]);
    let post =
        |id: &str, body: &str| request(id, "post", &format!(r#"{{"room":"r","body":"{body}"}}"#));

    // Each key posts as its own member, whatever the environment says (KLATCH_NAME is member-b for both children).
    let a = session(&home, &["--ssh-key", "a"], &[post("p", "from a")]);
    assert!(a.ok, "{}", a.stderr);
    assert_eq!(hello(&a), ("member-a".into(), Attribution::Verified));
    assert!(response_ok(&a, "p"));
    let b = session(&home, &["--ssh-key", "b"], &[post("p", "from b")]);
    assert!(b.ok, "{}", b.stderr);
    assert_eq!(hello(&b), ("member-b".into(), Attribution::Verified));
    assert!(response_ok(&b, "p"));

    // A frame naming the other member, or carrying a forged identity field, is refused and runs nothing.
    let forged = [
        r#"{"room":"r","body":"forged 1","member":"member-b"}"#,
        r#"{"room":"r","body":"forged 2","sender":"member-b"}"#,
        r#"{"room":"r","body":"forged 3","from":"member-b"}"#,
        r#"{"room":"r","body":"forged 4","as_member":"member-b"}"#,
        r#"{"room":"r","body":"forged 5","identity":"member-b"}"#,
        r#"{"room":"r","body":"forged 6","attribution":"verified"}"#,
    ];
    let lines: Vec<String> = forged
        .iter()
        .enumerate()
        .map(|(i, p)| request(&format!("f{i}"), "post", p))
        .collect();
    let attempt = session(&home, &["--ssh-key", "a"], &lines);
    assert!(attempt.ok, "{}", attempt.stderr);
    for i in 0..forged.len() {
        let id = format!("f{i}");
        assert!(!response_ok(&attempt, &id), "{id} must be refused");
        assert_eq!(
            refusal_rule(&attempt, &id).as_deref(),
            Some("member-fixed"),
            "{id}"
        );
    }
    // and the same the other way round
    let attempt = session(
        &home,
        &["--ssh-key", "b"],
        &[request(
            "x",
            "post",
            r#"{"room":"r","body":"b as a","member":"member-a"}"#,
        )],
    );
    assert_eq!(refusal_rule(&attempt, "x").as_deref(), Some("member-fixed"));

    // The log has exactly the two honest posts, each by the member its key is mapped to, and nothing forged.
    let log = room_log(&home, "a", "r");
    assert_eq!(
        log,
        vec![
            ("member-a".to_string(), "from a".to_string()),
            ("member-b".to_string(), "from b".to_string())
        ]
    );
}

#[test]
fn an_unmapped_label_does_not_start_and_says_which_rule_and_what_to_do() {
    let home = Home::new();
    seed(&home, &[("a", "member-a")]);
    let r = session(&home, &["--ssh-key", "stranger"], &[]);
    assert!(!r.ok);
    assert!(r.frames.is_empty(), "no hello for an unmapped key");
    assert!(r.stderr.contains("ssh-key-unmapped"), "{}", r.stderr);
    assert!(
        r.stderr.contains("klatch admin ssh-key stranger"),
        "{}",
        r.stderr
    );
    // a label that is not a bare word never reaches the store
    let r = session(&home, &["--ssh-key", "a b"], &[]);
    assert!(!r.ok);
    assert!(r.frames.is_empty());
}

#[test]
fn a_typed_name_beside_the_key_is_refused_at_start() {
    let home = Home::new();
    seed(&home, &[("a", "member-a")]);
    for extra in [
        vec!["--ssh-key", "a", "--as-member", "member-b"],
        vec!["--ssh-key", "a", "--discover"],
        vec!["--ssh-key", "a", "--codex"],
    ] {
        let r = session(&home, &extra, &[]);
        assert!(!r.ok, "{extra:?}");
        assert!(r.frames.is_empty(), "{extra:?}");
        assert!(r.stderr.contains("fixed by the ssh key"), "{}", r.stderr);
    }
}
