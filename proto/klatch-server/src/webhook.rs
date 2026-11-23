//! Loopback webhook intake for `gh webhook forward` (#1058, epic #1056; the integrator's event source, #863 and #866).
//!
//! `klatch-server channel --webhook 127.0.0.1:PORT (--webhook-secret-file PATH | --webhook-secret-env NAME)` listens on
//! loopback (module `loopback`) for POSTs to `/github`, verifies the `X-Hub-Signature-256` HMAC over the raw body in
//! constant time, and turns `pull_request`, `check_suite` and `workflow_run` events into channel notifications (the
//! same `notifications/claude/channel` path as the room events).
//!
//! The rules, each pinned by a test:
//! - there is no unauthenticated mode: no secret configuration is a startup error, and a missing or wrong signature is
//!   401 with nothing emitted;
//! - the secret is read from a file (not readable by group or others) or from an environment variable the flag NAMES; it
//!   is never a command-line value and never printed;
//! - only structured fields are used (action, number, state, merged, head sha, status, conclusion, mergeable_state),
//!   each checked against a fixed set or a strict shape, and `content` is a fixed template; `meta` has identifier keys
//!   (`event`, `pr`, `sha_short`, `state`, `conclusion`) and short values from fixed sets or hex. A title, body, branch
//!   name, workflow name or user name is never copied anywhere (a branch name is compared with the default branch to
//!   tell `main` from another branch, and that is all);
//! - a cancelled run for a head sha that is not the pull request's current head is `superseded`, not a failure.

use crate::channel::SharedOut;
use crate::loopback::{self, Handler, Head, Request, Response};
pub use crate::webhook_translate::*;
use anyhow::{Result, bail};
use hmac::{Hmac, Mac};
use serde_json::Value;
use sha2::Sha256;
use std::collections::VecDeque;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use subtle::ConstantTimeEq;

/// The one path the intake answers on (`gh webhook forward --url http://127.0.0.1:PORT/github`).
pub const PATH: &str = "/github";

/// How many notices wait for the channel before the oldest are dropped.
pub const QUEUE_CAP: usize = 64;

/// How many recent `X-GitHub-Delivery` ids are remembered to drop redeliveries.
pub const DELIVERY_IDS: usize = 1024;

// ---- HMAC-SHA256 ----

/// HMAC-SHA256 from the audited RustCrypto `hmac` crate (the repository does not hand-roll a MAC; RFC 4231 vectors are
/// unit tests).
pub fn signature_digest(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut mac =
        <Hmac<Sha256> as Mac>::new_from_slice(key).expect("HMAC takes a key of any length");
    mac.update(message);
    mac.finalize().into_bytes().into()
}

/// The shared secret of the webhook. Its debug form shows nothing.
pub struct Secret(Vec<u8>);

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(..)")
    }
}

impl Secret {
    /// A secret from bytes; an empty one is refused.
    pub fn new(bytes: Vec<u8>) -> Result<Secret> {
        if bytes.is_empty() {
            bail!("the webhook secret is empty");
        }
        Ok(Secret(bytes))
    }

    /// Read the secret from exactly one of a file or a named environment variable. `lookup` reads the environment (a
    /// parameter so a test needs no process-wide state).
    pub fn load(
        file: Option<&Path>,
        env: Option<&str>,
        lookup: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Secret> {
        match (file, env) {
            (None, None) => bail!(
                "the webhook needs a secret: give --webhook-secret-file PATH or --webhook-secret-env NAME (there is no unauthenticated mode)"
            ),
            (Some(_), Some(_)) => {
                bail!("give only one of --webhook-secret-file and --webhook-secret-env, not both")
            }
            (None, Some(name)) => {
                let Some(value) = lookup(name) else {
                    bail!("the environment variable {name} (--webhook-secret-env) is not set");
                };
                Secret::new(value.trim_end_matches(['\r', '\n']).as_bytes().to_vec())
                    .map_err(|_| anyhow::anyhow!("the environment variable {name} is empty"))
            }
            (Some(path), None) => {
                let meta = std::fs::metadata(path).map_err(|e| {
                    anyhow::anyhow!(
                        "cannot read the webhook secret file {}: {}",
                        path.display(),
                        e.kind()
                    )
                })?;
                if !meta.is_file() || meta.len() > 4096 {
                    bail!(
                        "the webhook secret file {} is not a small regular file",
                        path.display()
                    );
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if meta.permissions().mode() & 0o077 != 0 {
                        bail!(
                            "the webhook secret file {} is readable by group or others; run chmod 600 on it",
                            path.display()
                        );
                    }
                }
                let bytes = std::fs::read(path).map_err(|e| {
                    anyhow::anyhow!(
                        "cannot read the webhook secret file {}: {}",
                        path.display(),
                        e.kind()
                    )
                })?;
                let end = bytes
                    .iter()
                    .rposition(|b| *b != b'\n' && *b != b'\r')
                    .map_or(0, |i| i + 1);
                Secret::new(bytes[..end].to_vec()).map_err(|_| {
                    anyhow::anyhow!("the webhook secret file {} is empty", path.display())
                })
            }
        }
    }
}

/// Decode 64 hex digits into 32 bytes.
fn decode_hex32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, pair) in s.as_bytes().chunks(2).enumerate() {
        let hi = (pair[0] as char).to_digit(16)?;
        let lo = (pair[1] as char).to_digit(16)?;
        out[i] = u8::try_from(hi * 16 + lo).ok()?;
    }
    Some(out)
}

/// Is `header` (the `X-Hub-Signature-256` value, `sha256=<64 hex digits>`) the HMAC of `body` under the secret? The two
/// 32-byte digests are compared in constant time; a missing or malformed header is simply false.
pub fn verify_signature(secret: &Secret, body: &[u8], header: Option<&str>) -> bool {
    let Some(given) = header
        .and_then(|h| h.strip_prefix("sha256="))
        .and_then(decode_hex32)
    else {
        return false;
    };
    let want = signature_digest(&secret.0, body);
    want.ct_eq(&given).into()
}

// ---- the arguments ----

/// What `channel` was asked for on the webhook side.
#[derive(Debug)]
pub struct WebhookArgs {
    pub addr: String,
    pub secret: Secret,
}

fn is_env_name(s: &str) -> bool {
    let mut chars = s.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && s.len() <= 64
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The three options of the intake, as given (values still unread).
#[derive(Default)]
struct RawArgs<'a> {
    addr: Option<&'a str>,
    file: Option<&'a str>,
    name: Option<&'a str>,
}

/// Read the flags: each takes one value, none may repeat, and an unknown one is refused.
fn scan_flags<'a>(args: &[&'a str]) -> Result<RawArgs<'a>> {
    let mut raw = RawArgs::default();
    let mut i = 0;
    while i < args.len() {
        let slot = match args[i] {
            "--webhook" => &mut raw.addr,
            "--webhook-secret-file" => &mut raw.file,
            "--webhook-secret-env" => &mut raw.name,
            other if other.starts_with("--") && other.len() < 40 => {
                bail!("unknown option {}", klatch::domain::flat(other))
            }
            _ => bail!(
                "unexpected argument (the options are --webhook, --webhook-secret-file, --webhook-secret-env)"
            ),
        };
        let flag = args[i];
        let Some(value) = args.get(i + 1).filter(|v| !v.starts_with("--")) else {
            bail!("{flag} needs a value");
        };
        if slot.replace(value).is_some() {
            bail!("{flag} is given twice");
        }
        i += 2;
    }
    Ok(raw)
}

/// Parse the arguments after `channel`. `Ok(None)` when there are none (no intake). A non-loopback address and a
/// missing secret are errors here, before anything is bound.
pub fn parse_args(
    args: &[&str],
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<Option<WebhookArgs>> {
    let RawArgs { addr, file, name } = scan_flags(args)?;
    let Some(addr) = addr else {
        if file.is_some() || name.is_some() {
            bail!("--webhook-secret-file and --webhook-secret-env only apply with --webhook ADDR");
        }
        return Ok(None);
    };
    loopback::check_loopback(addr)?;
    if let Some(n) = name
        && !is_env_name(n)
    {
        // a name is letters, digits and underscores; anything else may be a secret typed by mistake, so it is not echoed
        bail!(
            "--webhook-secret-env takes the NAME of an environment variable, not a value (the secret must not be on the command line)"
        );
    }
    let secret = Secret::load(file.map(Path::new), name, env)?;
    Ok(Some(WebhookArgs {
        addr: addr.to_string(),
        secret,
    }))
}

// ---- the queue ----

/// Notices waiting for the channel: bounded, the oldest dropped (and counted) when it is full. `push` never blocks, so
/// neither the listener nor the MCP side waits on the other.
pub struct Queue {
    cap: usize,
    inner: Mutex<(VecDeque<Notice>, usize)>,
}

impl Queue {
    pub fn new(cap: usize) -> Queue {
        Queue {
            cap: cap.max(1),
            inner: Mutex::new((VecDeque::new(), 0)),
        }
    }

    pub fn push(&self, notice: Notice) {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if g.0.len() >= self.cap {
            g.0.pop_front();
            g.1 += 1;
        }
        g.0.push_back(notice);
    }

    /// Everything waiting, oldest first, and how many were dropped since the last drain.
    pub fn drain(&self) -> (Vec<Notice>, usize) {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        (g.0.drain(..).collect(), std::mem::take(&mut g.1))
    }
}

/// Send everything the queue holds (a notice of what was dropped first), oldest first; `Err` when the channel is gone.
fn flush(queue: &Queue, out: &SharedOut) -> Result<(), ()> {
    let (notices, dropped) = queue.drain();
    let lost = (dropped > 0).then(|| dropped_notice(dropped));
    for n in lost.iter().chain(notices.iter()) {
        out.send(&n.notification()).map_err(|_| ())?;
    }
    Ok(())
}

/// Send what the queue holds to the channel once the client is `ready`, until `stop`. Nothing is sent before the client's
/// `notifications/initialized` (what arrives meanwhile waits in the bounded queue).
pub fn pump(queue: &Queue, out: &SharedOut, (ready, stop): (&AtomicBool, &AtomicBool)) {
    while !stop.load(Ordering::SeqCst) {
        if ready.load(Ordering::SeqCst) && flush(queue, out).is_err() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

// ---- the request handler ----

/// The [`Handler`] of the listener: one path, POST, a verified signature, then the translation into the queue.
pub struct Intake<'a> {
    secret: Secret,
    queue: &'a Queue,
    tracker: Tracker,
    deliveries: Recent<String>,
    contents: Recent<[u8; 32]>,
}

/// A bounded set of recent keys, oldest forgotten first, at most [`DELIVERY_IDS`] long.
struct Recent<K: std::hash::Hash + Eq + Clone> {
    order: std::collections::VecDeque<K>,
    seen: std::collections::HashSet<K>,
}

impl<K: std::hash::Hash + Eq + Clone> Default for Recent<K> {
    fn default() -> Self {
        Recent {
            order: Default::default(),
            seen: Default::default(),
        }
    }
}

impl<K: std::hash::Hash + Eq + Clone> Recent<K> {
    fn contains(&self, key: &K) -> bool {
        self.seen.contains(key)
    }

    fn remember(&mut self, key: &K) {
        if self.seen.insert(key.clone()) {
            self.order.push_back(key.clone());
            while self.order.len() > DELIVERY_IDS {
                if let Some(old) = self.order.pop_front() {
                    self.seen.remove(&old);
                }
            }
        }
    }
}

/// The identity of a signed request apart from its unsigned delivery id: SHA-256 over the event name and the body (the
/// HMAC covers the body only, so an id header can be swapped by anyone who holds a captured request, #1103).
fn content_key(event: &str, body: &[u8]) -> [u8; 32] {
    use sha2::Digest;
    let mut h = Sha256::new();
    h.update(event.as_bytes());
    h.update([0]);
    h.update(body);
    h.finalize().into()
}

/// A usable delivery id: GitHub sends a GUID; anything long or with odd characters is not cached.
fn delivery_id(v: Option<&str>) -> Option<&str> {
    v.filter(|s| {
        !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    })
}

impl<'a> Intake<'a> {
    pub fn new(secret: Secret, queue: &'a Queue) -> Intake<'a> {
        Intake {
            secret,
            queue,
            tracker: Tracker::default(),
            deliveries: Recent::default(),
            contents: Recent::default(),
        }
    }
}

impl Handler for Intake<'_> {
    fn route(&mut self, head: &Head) -> Result<(), Response> {
        if head.path != PATH {
            return Err(Response::new(404, "not found"));
        }
        if head.method != "POST" {
            return Err(Response {
                allow: Some("POST"),
                ..Response::new(405, "method not allowed")
            });
        }
        Ok(())
    }

    fn handle(&mut self, request: Request) -> Response {
        // the signature comes first: nothing about an unsigned body is looked at
        if !verify_signature(
            &self.secret,
            &request.body,
            request.head.header("x-hub-signature-256"),
        ) {
            return Response::new(401, "unauthorized");
        }
        self.accept(&request)
    }
}

impl Intake<'_> {
    /// A signed request: dedupe, parse, translate, queue.
    fn accept(&mut self, request: &Request) -> Response {
        // the id is looked at only after the signature check; a repeat is acknowledged and dropped
        let delivery = delivery_id(request.head.header("x-github-delivery"));
        if delivery.is_some_and(|id| self.deliveries.contains(&id.to_string())) {
            return Response::new(200, "duplicate");
        }
        let Some(event) = request.head.header("x-github-event") else {
            return Response::new(400, "missing event header");
        };
        // the same event and body under a fresh id is a replay of a captured signed request, or GitHub resending: the
        // notice it would make was already made, so it is acknowledged and dropped
        let content = content_key(event, &request.body);
        if self.contents.contains(&content) {
            return Response::new(200, "duplicate");
        }
        let Ok(body) = serde_json::from_slice::<Value>(&request.body) else {
            return Response::new(400, "body is not JSON");
        };
        let Ok(outcome) = translate(event, &body, &mut self.tracker) else {
            return Response::new(400, "unusable event");
        };
        self.contents.remember(&content);
        if let Some(id) = delivery {
            self.deliveries.remember(&id.to_string());
        }
        match outcome {
            Outcome::Notice(n) => {
                self.queue.push(n);
                Response::new(200, "ok")
            }
            Outcome::Ignored => Response::new(200, "ignored"),
        }
    }
}

// ---- startup ----

/// The intake after startup checks: a bound loopback listener and the secret.
pub struct Bound {
    pub listener: loopback::Listener,
    pub secret: Secret,
}

/// Parse the options, read the secret, and bind (or refuse). `Ok(None)` when `--webhook` was not given.
pub fn setup(args: &[&str]) -> Result<Option<Bound>> {
    let Some(parsed) = parse_args(args, &|n| std::env::var(n).ok())? else {
        return Ok(None);
    };
    let listener = loopback::bind_loopback(&parsed.addr)?;
    Ok(Some(Bound {
        listener,
        secret: parsed.secret,
    }))
}
