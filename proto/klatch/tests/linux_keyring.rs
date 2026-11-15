//! The Linux Secret Service backend against a real service (epic #387, #382). These tests need a session bus with an
//! unlocked login collection, so they are ignored by default and run by `proto/linux-keys/run.sh`, inside a container
//! that has dbus and gnome-keyring. Each uses its own service name so none can see another's items.
#![cfg(target_os = "linux")]

use klatch::keys::{Error, KEY_LEN, Key, KeySource, SecretService};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

static N: AtomicU32 = AtomicU32::new(0);

fn service_name() -> String {
    format!(
        "klatch-test-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    )
}

fn backend() -> (SecretService, String) {
    let service = service_name();
    let svc = SecretService::detect(&service)
        .expect("a session bus and secret-tool are needed: run proto/linux-keys/run.sh");
    (svc, service)
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Put a raw secret in the keyring the way another program could, to plant a value klatch did not write.
fn plant(service: &str, name: &str, secret: &str) {
    use std::io::Write;
    let mut c = Command::new("secret-tool")
        .args([
            "store", "--label", "planted", "service", service, "name", name,
        ])
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    c.stdin
        .take()
        .unwrap()
        .write_all(secret.as_bytes())
        .unwrap();
    assert!(c.wait().unwrap().success());
}

fn count_items(service: &str) -> usize {
    let out = Command::new("secret-tool")
        .args(["search", "--all", "service", service])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.starts_with("[/"))
        .count()
}

#[test]
#[ignore = "needs a real Secret Service; run proto/linux-keys/run.sh"]
fn a_key_round_trips_through_the_keyring() {
    let (svc, _) = backend();
    let key = Key::generate().unwrap();
    svc.put("store-a", &key).unwrap();
    assert_eq!(svc.get("store-a").unwrap().unwrap(), key);
    svc.delete("store-a").unwrap();
}

#[test]
#[ignore = "needs a real Secret Service; run proto/linux-keys/run.sh"]
fn a_second_put_replaces_the_key_and_leaves_one_item() {
    let (svc, service) = backend();
    let (old, new) = (Key::from_bytes([1; KEY_LEN]), Key::from_bytes([2; KEY_LEN]));
    svc.put("store-a", &old).unwrap();
    svc.put("store-a", &new).unwrap();
    assert_eq!(svc.get("store-a").unwrap().unwrap(), new);
    assert_eq!(count_items(&service), 1, "a rotation must replace, not add");
    svc.delete("store-a").unwrap();
}

#[test]
#[ignore = "needs a real Secret Service; run proto/linux-keys/run.sh"]
fn delete_removes_the_key_and_deleting_a_missing_one_is_not_an_error() {
    let (svc, _) = backend();
    svc.put("store-a", &Key::from_bytes([3; KEY_LEN])).unwrap();
    svc.delete("store-a").unwrap();
    assert!(svc.get("store-a").unwrap().is_none());
    svc.delete("store-a").unwrap();
    svc.delete("never-existed").unwrap();
}

#[test]
#[ignore = "needs a real Secret Service; run proto/linux-keys/run.sh"]
fn a_missing_key_is_none_and_names_do_not_collide() {
    let (svc, _) = backend();
    assert!(svc.get("store-a").unwrap().is_none());
    svc.put("store-a", &Key::from_bytes([4; KEY_LEN])).unwrap();
    assert!(svc.get("store-b").unwrap().is_none());
    svc.delete("store-a").unwrap();
}

#[test]
#[ignore = "needs a real Secret Service; run proto/linux-keys/run.sh"]
fn an_item_of_the_wrong_size_is_refused_not_used() {
    let (svc, service) = backend();
    plant(&service, "short", &hex(&[7; KEY_LEN - 1]));
    plant(&service, "long", &hex(&[7; KEY_LEN + 1]));
    plant(&service, "text", "not a key at all");
    for name in ["short", "long", "text"] {
        assert!(
            matches!(svc.get(name), Err(Error::Bad(_))),
            "{name} was accepted"
        );
    }
}

#[test]
#[ignore = "needs a real Secret Service; run proto/linux-keys/run.sh"]
fn names_outside_the_safe_alphabet_are_refused_by_every_call_and_store_nothing() {
    let (svc, service) = backend();
    let k = Key::from_bytes([5; KEY_LEN]);
    for bad in ["", "a b", "../x", "a/b", "x\n", "a\"b", &"n".repeat(65)] {
        assert!(
            svc.get(bad).is_err() && svc.put(bad, &k).is_err() && svc.delete(bad).is_err(),
            "{bad:?}"
        );
    }
    assert_eq!(count_items(&service), 0);
}

/// Every command line on the machine, polled as fast as possible while `work` runs.
fn argv_seen_while(work: impl FnOnce(), needle: &str) -> bool {
    use std::sync::atomic::AtomicBool;
    let stop = std::sync::Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let needle = needle.to_string();
    let watcher = std::thread::spawn(move || {
        let mut seen = false;
        while !flag.load(Ordering::SeqCst) {
            for e in std::fs::read_dir("/proc").into_iter().flatten().flatten() {
                if let Ok(raw) = std::fs::read(e.path().join("cmdline")) {
                    seen |= String::from_utf8_lossy(&raw).contains(&needle);
                }
            }
        }
        seen
    });
    work();
    stop.store(true, Ordering::SeqCst);
    watcher.join().unwrap()
}

#[test]
#[ignore = "needs a real Secret Service; run proto/linux-keys/run.sh"]
fn the_secret_never_appears_in_any_command_line() {
    let (svc, _) = backend();
    let key = Key::from_bytes([0xC3; KEY_LEN]);
    let secret_hex = hex(key.expose());

    // The watcher must be able to see a command line at all, or "never seen" proves nothing.
    let control = argv_seen_while(
        || {
            let mut c = Command::new("sh")
                .args(["-c", "sleep 5", &secret_hex])
                .spawn()
                .unwrap();
            std::thread::sleep(std::time::Duration::from_millis(300));
            c.kill().unwrap();
            c.wait().unwrap();
        },
        &secret_hex,
    );
    assert!(control, "the /proc watcher cannot see command lines");

    let seen = argv_seen_while(
        || {
            for _ in 0..25 {
                svc.put("store-a", &key).unwrap();
                svc.get("store-a").unwrap().unwrap();
            }
            svc.delete("store-a").unwrap();
        },
        &secret_hex,
    );
    assert!(!seen, "the key was on a command line");
}

#[test]
#[ignore = "needs a real Secret Service; run proto/linux-keys/run.sh"]
fn the_binary_keeps_its_key_in_the_keyring_and_writes_no_key_file() {
    let _ = backend(); // the bus must be there
    let dir = std::env::temp_dir().join(format!("klatch-ring-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let run = |who: &str, args: &[&str]| {
        let o = Command::new(env!("CARGO_BIN_EXE_klatch"))
            .env_remove("KLATCH_KEY_DIR")
            .env_remove("KLATCH_IDENTITY_FILE")
            .env("HOME", dir.join("home"))
            .env("XDG_STATE_HOME", dir.join("state"))
            .env("KLATCH_HOME", dir.join("rooms"))
            .env("KLATCH_NAME", who)
            .args(args)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(
            o.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        String::from_utf8_lossy(&o.stdout).into_owned()
    };
    std::fs::create_dir_all(dir.join("home")).unwrap();
    run("alice", &["join", "lobby"]);
    run("alice", &["post", "lobby", "under a keyring key"]);
    run("bob", &["join", "lobby"]);
    assert!(run("bob", &["read", "lobby", "--peek"]).contains("under a keyring key"));
    assert!(
        !dir.join("state/klatch/keys").exists(),
        "a key file was written beside the keyring"
    );
    let name = klatch::keys::store_key_name(&dir.join("rooms"));
    let found = Command::new("secret-tool")
        .args(["lookup", "service", "klatch", "name", &name])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&found.stdout).trim().len(), 64);
    // leave the keyring as found
    Command::new("secret-tool")
        .args(["clear", "service", "klatch", "name", &name])
        .status()
        .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "needs a real Secret Service; run proto/linux-keys/run.sh"]
fn the_default_source_is_the_secret_service_with_no_warning() {
    let (source, warning) = klatch::keys::default_source(std::path::Path::new("/nonexistent"));
    assert_eq!(source.describe(), "secret-service");
    assert!(warning.is_none());
}
