//! The Linux Secret Service, through the `secret-tool` command: the secret goes in on standard input and comes out on
//! standard output, never in an argument. The Rust Secret Service crates need system D-Bus libraries and a running
//! session bus at build time; the command needs neither, and its absence is a clean fallback to a key file.
//!
//! Every call is bounded in time and has no terminal, so a keyring that wants a dialog (a locked collection on a box
//! with no display) fails instead of waiting for someone who is not there (#383).

use crate::keys::source::check_name;
use crate::keys::{Error, KEY_LEN, Key, KeySource};
use std::io::Write;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

/// How long one `secret-tool` call may take. A healthy call is a few milliseconds; this only ends a hang.
const CALL_LIMIT: Duration = Duration::from_secs(10);

/// The service name every klatch key is stored under.
const SERVICE: &str = "klatch";

/// The two programs this backend starts.
#[derive(Clone, Copy)]
enum Tool {
    SecretTool,
    Busctl,
}

impl Tool {
    fn name(self) -> &'static str {
        match self {
            Self::SecretTool => "secret-tool",
            Self::Busctl => "busctl",
        }
    }
}

/// What a read of one key name said.
type EarlyRead = (String, Result<Option<Key>, Error>);

pub struct SecretService {
    service: String,
    /// A `PATH` to run it under instead of the inherited one: how the unit tests put a stand-in first.
    path: Option<std::ffi::OsString>,
    limit: Duration,
    /// What the lookup made while detecting read, for the key name it was made for: the first `get` of that name
    /// takes it instead of asking the keyring a second time (#562). Used once, so a later `get` is never stale.
    prefetched: std::cell::RefCell<Option<EarlyRead>>,
}

impl SecretService {
    /// A Secret Service backend if `secret-tool` is installed and answers on the session bus; `None` otherwise. A
    /// bus address that points nowhere (an ssh login or a cron job can inherit one) counts as no service, so the
    /// caller falls back to a key file instead of failing on every command. The service is asked for the key
    /// `name` the caller is about to read, so the question that proves the service answers is also the first read
    /// and a command makes one keyring call, not two.
    pub fn detect(name: &str) -> Option<Self> {
        Self::detect_in(SERVICE, name)
    }

    /// `detect` under another service name, so the tests against a real keyring each get items of their own.
    #[doc(hidden)]
    pub fn detect_in(service: &str, name: &str) -> Option<Self> {
        std::env::var_os("DBUS_SESSION_BUS_ADDRESS")?;
        Self {
            service: service.to_string(),
            path: None,
            limit: CALL_LIMIT,
            prefetched: Default::default(),
        }
        .probe(name)
    }

    /// Read the key `name` once; keep the service if it answered, with that read held for the first `get`.
    fn probe(self, name: &str) -> Option<Self> {
        // A missing item, a locked keyring and a malformed item are all answers from a service that is there; only
        // a failure to reach it (a bus that is gone, a call that hangs, no `secret-tool`) is not.
        let first = self.get(name);
        let answers = !matches!(first, Err(Error::Store(_)));
        *self.prefetched.borrow_mut() = Some((name.to_string(), first));
        answers.then_some(self)
    }

    fn attrs<'a>(&'a self, name: &'a str) -> [&'a str; 4] {
        ["service", &self.service, "name", name]
    }

    /// Run `secret-tool` with `input` on standard input (never a terminal) and wait at most the limit for it.
    fn run(&self, args: &[&str], input: Option<&[u8]>) -> Result<Output, Error> {
        self.run_program(Tool::SecretTool, args, input)
    }

    /// What a silent failure of `secret-tool` hides. A lookup or a clear that finds nothing, and one that cannot
    /// read a locked collection or one that does not exist, all exit 1 and say nothing, so the answer comes from the
    /// default collection's `Locked` property over D-Bus (`busctl`, part of systemd). Asked only after a silent miss,
    /// never on a hit, so a key that is found costs no extra call (#562). When `busctl` is missing or says something
    /// unexpected the state is unknown and the miss stays a miss: custody still refuses to make a key for an
    /// existing encrypted store, so this costs a less specific message and nothing else.
    fn require_readable(&self) -> Result<(), Error> {
        const ARGS: [&str; 6] = [
            "--user",
            "get-property",
            "org.freedesktop.secrets",
            "/org/freedesktop/secrets/aliases/default",
            "org.freedesktop.Secret.Collection",
            "Locked",
        ];
        let Ok(out) = self.run_program(Tool::Busctl, &ARGS, None) else {
            return Ok(());
        };
        if !out.status.success() {
            // the text differs by daemon: gnome-keyring says "Object does not exist at path", others "Unknown object"
            let text = said(&out);
            return if text.contains("Unknown object") || text.contains("does not exist") {
                Err(Error::Locked(
                    "the keyring has no default collection (no login keyring was created or unlocked); \
                     unlock or create it, then try again. klatch made no new key"
                        .into(),
                ))
            } else {
                Ok(())
            };
        }
        match String::from_utf8_lossy(&out.stdout).trim() {
            "b true" => Err(Error::Locked(
                "the keyring is locked and there is nothing here to unlock it on; unlock it (log in on the \
                 desktop, or `secret-tool` after unlocking), then try again. klatch made no new key"
                    .into(),
            )),
            _ => Ok(()),
        }
    }

    /// Run `program` with `input` on standard input (never a terminal) and wait at most the limit for it.
    fn run_program(
        &self,
        tool: Tool,
        args: &[&str],
        input: Option<&[u8]>,
    ) -> Result<Output, Error> {
        let program = tool.name();
        let fail = |e: std::io::Error| Error::Store(format!("{program}: {e}"));
        // literal names, so the source check can see every program klatch starts (tests/no_telemetry.rs)
        let mut command = match tool {
            Tool::SecretTool => Command::new("secret-tool"),
            Tool::Busctl => Command::new("busctl"),
        };
        if let Some(path) = &self.path {
            command.env("PATH", path);
        }
        let mut child = command
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(fail)?;
        // closing standard input (the take drops it) is what tells `store` the secret has ended
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| Error::Store(format!("{program}: no input")))?;
        if let Some(bytes) = input {
            stdin.write_all(bytes).map_err(fail)?;
        }
        drop(stdin);
        let start = Instant::now();
        while child.try_wait().map_err(fail)?.is_none() {
            if start.elapsed() > self.limit {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Error::Store(format!(
                    "{program} did not answer within {} seconds; the keyring may be waiting for a dialog",
                    self.limit.as_secs().max(1)
                )));
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        child.wait_with_output().map_err(fail)
    }
}

/// What the command said on standard error, flattened to one line for a message (it never holds the secret).
fn said(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<[u8; KEY_LEN]> {
    let s = s.trim();
    if s.len() != KEY_LEN * 2 {
        return None;
    }
    let mut out = [0u8; KEY_LEN];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

impl KeySource for SecretService {
    fn get(&self, name: &str) -> Result<Option<Key>, Error> {
        // the read made while detecting answers the first ask for its name (#562)
        let early = self.prefetched.borrow_mut().take();
        if let Some((asked, result)) = early
            && asked == name
        {
            return result;
        }
        check_name(name)?;
        let mut args = vec!["lookup"];
        args.extend(self.attrs(name));
        let out = self.run(&args, None)?;
        if !out.status.success() {
            // not found exits non-zero and is silent, and so does a locked collection: ask which it was (#561);
            // anything said is a failure to reach the service
            return match said(&out) {
                m if m.is_empty() => self.require_readable().map(|()| None),
                m => Err(Error::Store(format!("secret-tool: {m}"))),
            };
        }
        if out.stdout.is_empty() {
            return Ok(None);
        }
        unhex(&String::from_utf8_lossy(&out.stdout))
            .map(|b| Some(Key::from_bytes(b)))
            .ok_or_else(|| {
                Error::Bad(format!(
                    "secret service item {name} is not a {KEY_LEN}-byte key"
                ))
            })
    }

    fn put(&self, name: &str, key: &Key) -> Result<(), Error> {
        self.prefetched.borrow_mut().take();
        check_name(name)?;
        let mut args = vec!["store", "--label", "klatch key"];
        args.extend(self.attrs(name));
        let out = self.run(&args, Some(hex(key.expose()).as_bytes()))?;
        if out.status.success() {
            Ok(())
        } else {
            Err(Error::Store(format!(
                "secret-tool could not store the key: {}",
                said(&out)
            )))
        }
    }

    fn delete(&self, name: &str) -> Result<(), Error> {
        self.prefetched.borrow_mut().take();
        check_name(name)?;
        let mut args = vec!["clear"];
        args.extend(self.attrs(name));
        let out = self.run(&args, None)?;
        // clearing a missing item exits non-zero and is silent; that is not an error here, but a locked collection
        // is just as silent and the key is still there, so a revoke must not report success (#561)
        match said(&out) {
            _ if out.status.success() => Ok(()),
            m if m.is_empty() => self.require_readable(),
            m => Err(Error::Store(format!("secret-tool: {m}"))),
        }
    }

    fn describe(&self) -> &'static str {
        "secret-service"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn the_hex_form_round_trips_and_rejects_anything_else() {
        let b = [0xAB; KEY_LEN];
        assert_eq!(unhex(&hex(&b)), Some(b));
        assert_eq!(unhex(&format!("{}\n", hex(&b))), Some(b));
        assert_eq!(unhex("abcd"), None);
        assert_eq!(unhex(&"zz".repeat(KEY_LEN)), None);
    }

    /// What every stand-in tool runs: it reads the body of the test that made it from the file beside the link it was
    /// started through (`$0` is that link), so one script serves every fake in every test.
    const TRAMPOLINE: &str = "#!/bin/sh\n[ -n \"$KLATCH_FAKE_PROBE\" ] && exit 0\n. \"$0.body\"\n";

    /// The one shared script the stand-in tools are links to, made once per machine and never changed afterwards. Why
    /// shared: a freshly written executable costs about 150 ms the first time it is started on macOS, and under load
    /// those starts queue for seconds, which made every test that wrote its own script take 3 to 8 s (#1353). It is
    /// found again by its content, and only trusted when it is ours and nobody else can write it; otherwise this
    /// process makes a private copy.
    fn trampoline(private: &std::path::Path) -> std::path::PathBuf {
        use std::hash::{Hash, Hasher};
        use std::os::unix::fs::MetadataExt;
        let mut h = std::collections::hash_map::DefaultHasher::new();
        TRAMPOLINE.hash(&mut h);
        let shared = std::env::temp_dir().join(format!("klatch-fake-tool-{:016x}", h.finish()));
        let make = |to: &std::path::Path| {
            std::fs::write(to, TRAMPOLINE).unwrap();
            std::fs::set_permissions(to, std::fs::Permissions::from_mode(0o755)).unwrap();
        };
        // The file is made under a private name and linked into place, so a half written script is never seen and the
        // first maker wins; `hard_link` fails when the name is taken, which is fine.
        let tmp = private.join("trampoline.tmp");
        make(&tmp);
        let mine = std::fs::metadata(&tmp).unwrap().uid();
        let _ = std::fs::hard_link(&tmp, &shared);
        let trusted = std::fs::metadata(&shared).is_ok_and(|m| {
            m.uid() == mine
                && m.mode() & 0o022 == 0
                && std::fs::read_to_string(&shared).is_ok_and(|c| c == TRAMPOLINE)
        });
        if trusted {
            let _ = std::fs::remove_file(&tmp);
            shared
        } else {
            tmp
        }
    }

    /// A stand-in tool `name` in `dir` that runs `body`: a link to the shared script, with the body in a file beside it.
    fn install_tool(dir: &std::path::Path, name: &str, body: &str) {
        let link = dir.join(name);
        std::fs::write(dir.join(format!("{name}.body")), format!("{body}\n")).unwrap();
        // a test may install the same tool again with another body, as writing the file again once did
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(trampoline(dir), &link).unwrap();
        // Another test thread may fork while the file is open for writing, and a child briefly holds that handle:
        // executing the script then fails with "Text file busy". Wait until it can be executed (CI flake, #570).
        // (sh execs the script and reports exit code 126 for "Text file busy".)
        for _ in 0..200 {
            let status = std::process::Command::new("sh")
                .args(["-c", "exec \"$0\""])
                .arg(&link)
                .env("KLATCH_FAKE_PROBE", "1")
                .stderr(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .status();
            match status {
                Ok(s) if s.code() == Some(126) => std::thread::sleep(Duration::from_millis(10)),
                _ => break,
            }
        }
    }

    /// A stand-in `secret-tool`: a shell script with the given body, in a directory of its own.
    fn fake(body: &str) -> (std::ffi::OsString, crate::store::test_dir::TestDir) {
        let dir = crate::store::test_dir::TestDir::new("fake");
        install_tool(&dir, "secret-tool", body);
        (format!("{}:/usr/bin:/bin", dir.display()).into(), dir)
    }

    /// A stand-in `busctl` in `dir`, with the given body: what the collection's `Locked` property reads as (#561).
    fn install_busctl(dir: &std::path::Path, body: &str) {
        install_tool(dir, "busctl", body);
    }

    /// A silent miss from `secret-tool` and a `busctl` that says `bus_body`.
    fn with_fake_bus(bus_body: &str) -> (SecretService, crate::store::test_dir::TestDir) {
        let (svc, dir) = with_fake("exit 1", CALL_LIMIT);
        install_busctl(&dir, bus_body);
        (svc, dir)
    }

    /// What gnome-keyring's own bus answers (seen in the container, #561).
    const GONE_GNOME: &str = "echo \"Failed to get property Locked on interface org.freedesktop.Secret.Collection: Object does not exist at path \u{201c}/org/freedesktop/secrets/collection/login\u{201d}\" >&2; exit 1";

    const GONE: &str = "echo \"Failed to get property Locked: Unknown object '/org/freedesktop/secrets/aliases/default'.\" >&2; exit 1";

    #[test]
    fn a_silent_miss_on_a_locked_keyring_is_a_locked_error_not_no_key() {
        let (svc, _dir) = with_fake_bus("echo 'b true'");
        let got = svc.get("store-a");
        assert!(
            matches!(&got, Err(Error::Locked(m)) if m.contains("locked") && m.contains("no new key")),
            "{got:?}"
        );
    }

    #[test]
    fn a_silent_miss_with_no_login_collection_is_a_locked_error_not_no_key() {
        let (svc, _dir) = with_fake_bus(GONE);
        let got = svc.get("store-a");
        assert!(
            matches!(&got, Err(Error::Locked(m)) if m.contains("no default collection")),
            "{got:?}"
        );
    }

    #[test]
    fn gnome_keyrings_wording_for_a_missing_collection_is_a_locked_error_too() {
        let (svc, _dir) = with_fake_bus(GONE_GNOME);
        let got = svc.get("store-a");
        assert!(
            matches!(&got, Err(Error::Locked(m)) if m.contains("no default collection")),
            "{got:?}"
        );
    }

    #[test]
    fn a_silent_miss_on_an_unlocked_keyring_is_still_no_key() {
        let (svc, _dir) = with_fake_bus("echo 'b false'");
        assert!(svc.get("store-a").unwrap().is_none());
    }

    #[test]
    fn a_silent_miss_when_the_lock_state_cannot_be_read_is_still_no_key() {
        // custody refuses to make a key for an existing encrypted store, so this only costs a less specific message
        for body in ["echo 'busctl: boom' >&2; exit 1", "echo 'something else'"] {
            let (svc, _dir) = with_fake_bus(body);
            assert!(svc.get("store-a").unwrap().is_none(), "{body}");
        }
    }

    #[test]
    fn clearing_on_a_locked_keyring_is_a_locked_error_not_a_quiet_success() {
        let (svc, _dir) = with_fake_bus("echo 'b true'");
        assert!(matches!(svc.delete("store-a"), Err(Error::Locked(_))));
        let (svc2, _dir2) = with_fake_bus("echo 'b false'");
        svc2.delete("store-a").unwrap();
    }

    #[test]
    fn a_key_found_is_returned_without_asking_whether_the_keyring_is_locked() {
        let body = format!("echo {}", hex(&[7; KEY_LEN]));
        let (svc, dir) = with_fake(&body, CALL_LIMIT);
        install_busctl(&dir, "touch \"$0.ran\"");
        assert_eq!(svc.get("store-a").unwrap().unwrap().expose(), &[7; KEY_LEN]);
        assert!(!dir.join("busctl.ran").exists());
    }

    /// Detect against the stand-ins in `path`.
    fn detect_fake(path: std::ffi::OsString) -> Option<SecretService> {
        SecretService {
            service: "klatch-unit".into(),
            path: Some(path),
            limit: CALL_LIMIT,
            prefetched: Default::default(),
        }
        .probe("store-a")
    }

    fn with_fake(body: &str, limit: Duration) -> (SecretService, crate::store::test_dir::TestDir) {
        let (path, dir) = fake(body);
        let svc = SecretService {
            service: "klatch-unit".into(),
            path: Some(path),
            limit,
            prefetched: Default::default(),
        };
        (svc, dir)
    }

    #[test]
    fn a_call_that_never_answers_is_ended_and_reported_not_waited_for() {
        let (svc, _dir) = with_fake("sleep 30", Duration::from_millis(300));
        let started = Instant::now();
        let got = svc.get("store-a");
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(
            matches!(&got, Err(Error::Store(m)) if m.contains("did not answer")),
            "{got:?}"
        );
        assert!(svc.put("store-a", &Key::from_bytes([1; KEY_LEN])).is_err());
    }

    #[test]
    fn a_service_that_complains_is_not_detected_and_a_silent_miss_is() {
        let (path, _d1) = fake("echo 'Could not connect' >&2; exit 1");
        assert!(detect_fake(path).is_none());
        let (path, d2) = fake("exit 1");
        install_busctl(&d2, "echo 'b false'");
        assert!(
            detect_fake(path).is_some(),
            "a lookup that finds nothing is a service that answers"
        );
    }

    /// A `secret-tool` that counts its calls in a file beside it and answers `body`.
    fn counting(body: &str) -> (std::ffi::OsString, crate::store::test_dir::TestDir) {
        let (path, dir) = fake(&format!("echo x >> \"$(dirname \"$0\")/calls\"\n{body}"));
        install_busctl(&dir, "echo 'b false'");
        (path, dir)
    }

    fn calls(dir: &std::path::Path) -> usize {
        std::fs::read_to_string(dir.join("calls")).map_or(0, |s| s.lines().count())
    }

    #[test]
    fn detecting_and_the_first_read_of_the_key_cost_one_call_not_two() {
        let (path, dir) = counting(&format!("echo {}", hex(&[5; KEY_LEN])));
        let svc = detect_fake(path).unwrap();
        assert_eq!(calls(&dir), 1, "detect alone");
        assert_eq!(svc.get("store-a").unwrap().unwrap().expose(), &[5; KEY_LEN]);
        assert_eq!(calls(&dir), 1, "the first get reuses what detect read");
        assert_eq!(svc.get("store-a").unwrap().unwrap().expose(), &[5; KEY_LEN]);
        assert_eq!(calls(&dir), 2, "a second get asks again: never a stale key");
    }

    #[test]
    fn a_put_or_delete_after_detecting_never_leaves_the_early_read_to_be_served_stale() {
        let (path, dir) = counting("exit 1");
        let svc = detect_fake(path).unwrap();
        let _ = svc.put("store-a", &Key::from_bytes([1; KEY_LEN]));
        svc.get("store-a").unwrap();
        assert_eq!(calls(&dir), 3, "detect, put, then a fresh get");
        let (path, dir2) = counting("exit 1");
        let svc = detect_fake(path).unwrap();
        svc.delete("store-a").unwrap();
        svc.get("store-a").unwrap();
        assert_eq!(calls(&dir2), 3, "detect, clear, then a fresh get");
    }

    #[test]
    fn the_read_made_while_detecting_serves_only_the_key_name_it_asked_for() {
        let (path, dir) = counting(&format!("echo {}", hex(&[5; KEY_LEN])));
        let svc = detect_fake(path).unwrap();
        svc.get("store-b").unwrap();
        assert_eq!(calls(&dir), 2);
    }

    #[test]
    fn a_miss_and_a_locked_keyring_seen_while_detecting_are_handed_on_as_they_were() {
        let (path, dir) = counting("exit 1");
        let svc = detect_fake(path).unwrap();
        assert!(svc.get("store-a").unwrap().is_none());
        assert_eq!(calls(&dir), 1);
        let (path, dir2) = counting("exit 1");
        install_busctl(&dir2, "echo 'b true'");
        let svc = detect_fake(path).unwrap();
        assert!(matches!(svc.get("store-a"), Err(Error::Locked(_))));
    }

    #[test]
    fn a_failure_to_reach_the_service_is_an_error_not_a_missing_key() {
        let (svc, _dir) = with_fake("echo 'Cannot connect' >&2; exit 1", CALL_LIMIT);
        assert!(matches!(svc.get("store-a"), Err(Error::Store(_))));
        assert!(matches!(svc.delete("store-a"), Err(Error::Store(_))));
    }

    #[test]
    fn a_missing_item_is_none_and_clearing_it_is_fine() {
        let (svc, _dir) = with_fake("exit 1", CALL_LIMIT);
        assert!(svc.get("store-a").unwrap().is_none());
        svc.delete("store-a").unwrap();
    }

    #[test]
    fn the_stand_in_tools_of_two_tests_are_one_executable_so_a_new_script_is_not_started_for_each()
    {
        // On macOS the first start of each new executable file costs about 150 ms, and under load those starts queue
        // for seconds, so every test that wrote its own script took 3 to 8 s (#1353). The stand-ins are links to one
        // shared script that reads its body from a file beside the link, so only that one script is ever new.
        let (_p1, d1) = fake("exit 1");
        install_busctl(&d1, "echo 'b false'");
        let (_p2, d2) = fake("exit 2");
        install_busctl(&d2, "echo 'b true'");
        let id = |p: std::path::PathBuf| std::fs::canonicalize(p).unwrap();
        assert_eq!(id(d1.join("secret-tool")), id(d2.join("secret-tool")));
        assert_eq!(id(d1.join("busctl")), id(d2.join("busctl")));
        assert_eq!(id(d1.join("busctl")), id(d1.join("secret-tool")));
        // each one still does what its own test said
        let run = |d: &std::path::Path, tool: &str| {
            let o = std::process::Command::new(d.join(tool)).output().unwrap();
            (
                o.status.code(),
                String::from_utf8_lossy(&o.stdout).into_owned(),
            )
        };
        assert_eq!(run(&d1, "secret-tool"), (Some(1), String::new()));
        assert_eq!(run(&d2, "secret-tool"), (Some(2), String::new()));
        assert_eq!(run(&d1, "busctl"), (Some(0), "b false\n".into()));
        assert_eq!(run(&d2, "busctl"), (Some(0), "b true\n".into()));
    }
}
