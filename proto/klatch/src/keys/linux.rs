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

    /// Run the command with `input` on standard input (never a terminal) and wait at most the limit for it.
    fn run(&self, args: &[&str], input: Option<&[u8]>) -> Result<Output, Error> {
        let fail = |e: std::io::Error| Error::Store(format!("secret-tool: {e}"));
        let mut command = Command::new("secret-tool");
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
            .ok_or_else(|| Error::Store("secret-tool: no input".into()))?;
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
                    "secret-tool did not answer within {} seconds; the keyring may be waiting for a dialog",
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
            // not found exits non-zero and is silent; anything said is a failure to reach the service
            return match said(&out) {
                m if m.is_empty() => Ok(None),
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
        // clearing a missing item exits non-zero and is silent; that is not an error here
        match said(&out) {
            m if out.status.success() || m.is_empty() => Ok(()),
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
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        (format!("{}:/usr/bin:/bin", dir.display()).into(), dir)
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
