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

pub struct SecretService {
    service: String,
    /// A `PATH` to run it under instead of the inherited one: how the unit tests put a stand-in first.
    path: Option<std::ffi::OsString>,
    limit: Duration,
}

impl SecretService {
    /// A Secret Service backend if `secret-tool` is installed and answers on the session bus; `None` otherwise. A
    /// bus address that points nowhere (an ssh login or a cron job can inherit one) counts as no service, so the
    /// caller falls back to a key file instead of failing on every command.
    pub fn detect(service: &str) -> Option<Self> {
        Self::detect_with(None, service, CALL_LIMIT)
    }

    fn detect_with(
        path: Option<std::ffi::OsString>,
        service: &str,
        limit: Duration,
    ) -> Option<Self> {
        std::env::var_os("DBUS_SESSION_BUS_ADDRESS")?;
        let svc = Self {
            service: service.to_string(),
            path,
            limit,
        };
        svc.answers().then_some(svc)
    }

    /// Whether the service answers a lookup for an item that does not exist. A missing item exits non-zero and says
    /// nothing; a bus that is gone, or a service that cannot be reached, says why on standard error.
    fn answers(&self) -> bool {
        self.run(&["lookup", "service", &self.service, "name", "probe"], None)
            .is_ok_and(|out| out.stderr.is_empty())
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
            return if said(&out).contains("Unknown object") {
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

    /// A stand-in `secret-tool`: a shell script with the given body, in a directory of its own.
    fn fake(tag: &str, body: &str) -> (std::ffi::OsString, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("klatch-fake-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("secret-tool");
        // The probe line lets the helper run the script once and return at once: see below.
        std::fs::write(
            &path,
            format!("#!/bin/sh\n[ -n \"$KLATCH_FAKE_PROBE\" ] && exit 0\n{body}\n"),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        // Another test thread may fork while the file is open for writing, and a child briefly holds that handle:
        // executing the script then fails with "Text file busy". Wait until it can be executed (CI flake, #570).
        // (sh execs the script and reports exit code 126 for "Text file busy".)
        for _ in 0..200 {
            let status = std::process::Command::new("sh")
                .args(["-c", "exec \"$0\""])
                .arg(&path)
                .env("KLATCH_FAKE_PROBE", "1")
                .stderr(std::process::Stdio::null())
                .status();
            match status {
                Ok(s) if s.code() == Some(126) => std::thread::sleep(Duration::from_millis(10)),
                _ => break,
            }
        }
        (format!("{}:/usr/bin:/bin", dir.display()).into(), dir)
    }

    /// A stand-in `busctl` in `dir`, with the given body: what the collection's `Locked` property reads as (#561).
    fn install_busctl(dir: &std::path::Path, body: &str) {
        let path = dir.join("busctl");
        std::fs::write(
            &path,
            format!("#!/bin/sh\n[ -n \"$KLATCH_FAKE_PROBE\" ] && exit 0\n{body}\n"),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        // the same "Text file busy" wait as `fake` (#570)
        for _ in 0..200 {
            let status = std::process::Command::new("sh")
                .args(["-c", "exec \"$0\""])
                .arg(&path)
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

    /// A silent miss from `secret-tool` and a `busctl` that says `bus_body`.
    fn with_fake_bus(tag: &str, bus_body: &str) -> (SecretService, std::path::PathBuf) {
        let (svc, dir) = with_fake(tag, "exit 1", CALL_LIMIT);
        install_busctl(&dir, bus_body);
        (svc, dir)
    }

    const GONE: &str = "echo \"Failed to get property Locked: Unknown object '/org/freedesktop/secrets/aliases/default'.\" >&2; exit 1";

    #[test]
    fn a_silent_miss_on_a_locked_keyring_is_a_locked_error_not_no_key() {
        let (svc, dir) = with_fake_bus("locked", "echo 'b true'");
        let got = svc.get("store-a");
        assert!(
            matches!(&got, Err(Error::Locked(m)) if m.contains("locked") && m.contains("no new key")),
            "{got:?}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_silent_miss_with_no_login_collection_is_a_locked_error_not_no_key() {
        let (svc, dir) = with_fake_bus("nocoll", GONE);
        let got = svc.get("store-a");
        assert!(
            matches!(&got, Err(Error::Locked(m)) if m.contains("no default collection")),
            "{got:?}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_silent_miss_on_an_unlocked_keyring_is_still_no_key() {
        let (svc, dir) = with_fake_bus("unlocked", "echo 'b false'");
        assert!(svc.get("store-a").unwrap().is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_silent_miss_when_the_lock_state_cannot_be_read_is_still_no_key() {
        // custody refuses to make a key for an existing encrypted store, so this only costs a less specific message
        for (tag, body) in [
            ("fail", "echo 'busctl: boom' >&2; exit 1"),
            ("odd", "echo 'something else'"),
        ] {
            let (svc, dir) = with_fake_bus(tag, body);
            assert!(svc.get("store-a").unwrap().is_none(), "{tag}");
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn clearing_on_a_locked_keyring_is_a_locked_error_not_a_quiet_success() {
        let (svc, dir) = with_fake_bus("dellocked", "echo 'b true'");
        assert!(matches!(svc.delete("store-a"), Err(Error::Locked(_))));
        let (svc2, dir2) = with_fake_bus("delunlocked", "echo 'b false'");
        svc2.delete("store-a").unwrap();
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(dir2);
    }

    #[test]
    fn a_key_found_is_returned_without_asking_whether_the_keyring_is_locked() {
        let body = format!("echo {}", hex(&[7; KEY_LEN]));
        let (svc, dir) = with_fake("found", &body, CALL_LIMIT);
        install_busctl(&dir, "touch \"$0.ran\"");
        assert_eq!(svc.get("store-a").unwrap().unwrap().expose(), &[7; KEY_LEN]);
        assert!(!dir.join("busctl.ran").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    fn with_fake(tag: &str, body: &str, limit: Duration) -> (SecretService, std::path::PathBuf) {
        let (path, dir) = fake(tag, body);
        let svc = SecretService {
            service: "klatch-unit".into(),
            path: Some(path),
            limit,
        };
        (svc, dir)
    }

    #[test]
    fn a_call_that_never_answers_is_ended_and_reported_not_waited_for() {
        let (svc, dir) = with_fake("hang", "sleep 30", Duration::from_millis(300));
        let started = Instant::now();
        let got = svc.get("store-a");
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(
            matches!(&got, Err(Error::Store(m)) if m.contains("did not answer")),
            "{got:?}"
        );
        assert!(svc.put("store-a", &Key::from_bytes([1; KEY_LEN])).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_service_that_complains_is_not_detected_and_a_silent_miss_is() {
        let (down, d1) = with_fake("down", "echo 'Could not connect' >&2; exit 1", CALL_LIMIT);
        assert!(!down.answers());
        let (up, d2) = with_fake("up", "exit 1", CALL_LIMIT);
        assert!(
            up.answers(),
            "a lookup that finds nothing is a service that answers"
        );
        let _ = std::fs::remove_dir_all(d1);
        let _ = std::fs::remove_dir_all(d2);
    }

    #[test]
    fn a_failure_to_reach_the_service_is_an_error_not_a_missing_key() {
        let (svc, dir) = with_fake("err", "echo 'Cannot connect' >&2; exit 1", CALL_LIMIT);
        assert!(matches!(svc.get("store-a"), Err(Error::Store(_))));
        assert!(matches!(svc.delete("store-a"), Err(Error::Store(_))));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_missing_item_is_none_and_clearing_it_is_fine() {
        let (svc, dir) = with_fake("miss", "exit 1", CALL_LIMIT);
        assert!(svc.get("store-a").unwrap().is_none());
        svc.delete("store-a").unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }
}
