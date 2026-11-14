//! The Linux Secret Service, through the `secret-tool` command: the secret goes in on standard input and comes out on
//! standard output, never in an argument. The Rust Secret Service crates need system D-Bus libraries and a running
//! session bus at build time; the command needs neither, and its absence is a clean fallback to a key file.

use crate::source::check_name;
use crate::{Error, KEY_LEN, Key, KeySource};
use std::io::Write;
use std::process::{Command, Stdio};

pub struct SecretService {
    service: String,
}

impl SecretService {
    /// A Secret Service backend if `secret-tool` is installed and a session bus is reachable; `None` otherwise.
    pub fn detect(service: &str) -> Option<Self> {
        let installed = Command::new("secret-tool")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok();
        let bus = std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some();
        (installed && bus).then(|| Self {
            service: service.to_string(),
        })
    }

    fn attrs<'a>(&'a self, name: &'a str) -> [&'a str; 4] {
        ["service", &self.service, "name", name]
    }
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
        let out = Command::new("secret-tool")
            .arg("lookup")
            .args(self.attrs(name))
            .stderr(Stdio::null())
            .output()
            .map_err(|e| Error::Store(format!("secret-tool: {e}")))?;
        if !out.status.success() || out.stdout.is_empty() {
            return Ok(None);
        }
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        unhex(&text)
            .map(|b| Some(Key::from_bytes(b)))
            .ok_or_else(|| {
                Error::Bad(format!(
                    "secret service item {name} is not a {KEY_LEN}-byte key"
                ))
            })
    }

    fn put(&self, name: &str, key: &Key) -> Result<(), Error> {
        check_name(name)?;
        let mut child = Command::new("secret-tool")
            .args(["store", "--label", "klatch key"])
            .args(self.attrs(name))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| Error::Store(format!("secret-tool: {e}")))?;
        child
            .stdin
            .take()
            .ok_or_else(|| Error::Store("secret-tool: no input".into()))?
            .write_all(hex(key.expose()).as_bytes())
            .map_err(|e| Error::Store(format!("secret-tool: {e}")))?;
        let status = child
            .wait()
            .map_err(|e| Error::Store(format!("secret-tool: {e}")))?;
        if status.success() {
            Ok(())
        } else {
            Err(Error::Store("secret-tool could not store the key".into()))
        }
    }

    fn delete(&self, name: &str) -> Result<(), Error> {
        check_name(name)?;
        // clearing a missing item exits non-zero; that is not an error here
        Command::new("secret-tool")
            .arg("clear")
            .args(self.attrs(name))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|e| Error::Store(format!("secret-tool: {e}")))?;
        Ok(())
    }

    fn describe(&self) -> &'static str {
        "secret-service"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hex_form_round_trips_and_rejects_anything_else() {
        let b = [0xAB; KEY_LEN];
        assert_eq!(unhex(&hex(&b)), Some(b));
        assert_eq!(unhex(&format!("{}\n", hex(&b))), Some(b));
        assert_eq!(unhex("abcd"), None);
        assert_eq!(unhex(&"zz".repeat(KEY_LEN)), None);
    }
}
