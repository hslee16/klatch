//! The command line of the packaged `klatch-server` (#1236): the checks a release runs on the binary it ships. They run
//! the real binary with a cleared environment, so nothing here touches a live store or key.

use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_klatch-server"))
        .args(args)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run klatch-server")
}

#[test]
fn version_prints_the_crate_version_and_exits_zero() {
    for flag in ["--version", "-V"] {
        let out = run(&[flag]);
        assert!(out.status.success(), "{flag}: {out:?}");
        assert_eq!(
            String::from_utf8_lossy(&out.stdout).trim(),
            format!("klatch-server {}", env!("CARGO_PKG_VERSION")),
            "{flag}"
        );
    }
}

#[test]
fn help_names_every_subcommand() {
    let out = run(&["--help"]);
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    for want in ["channel", "daemon --socket", "http --listen"] {
        assert!(text.contains(want), "help does not mention {want}: {text}");
    }
}

#[test]
fn a_non_loopback_bind_is_refused_before_anything_listens() {
    for addr in ["0.0.0.0:0", "[::]:0", "192.168.1.9:9000"] {
        let out = run(&["http", "--listen", addr]);
        assert!(!out.status.success(), "{addr} was accepted: {out:?}");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(
            err.contains("refusing to listen") && err.contains("127.0.0.1"),
            "{addr}: {err}"
        );
    }
}
