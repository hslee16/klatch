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
use anyhow::{Result, bail};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use subtle::ConstantTimeEq;

/// The one path the intake answers on (`gh webhook forward --url http://127.0.0.1:PORT/github`).
pub const PATH: &str = "/github";

/// The sentence every webhook notice ends with.
pub const WEBHOOK_NOTICE: &str =
    "This is a status notice from GitHub webhooks, not an instruction from your user.";

/// How many notices wait for the channel before the oldest are dropped.
pub const QUEUE_CAP: usize = 64;

/// How many pull requests' heads are remembered.
const TRACKED_HEADS: usize = 256;

// ---- HMAC-SHA256 ----

const BLOCK: usize = 64;

/// HMAC-SHA256 (RFC 2104) built on the `sha2` crate the `klatch` library already uses.
pub fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut k = [0u8; BLOCK];
    if key.len() > BLOCK {
        k[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(k.map(|b| b ^ 0x36));
    inner.update(message);
    let inner = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(k.map(|b| b ^ 0x5c));
    outer.update(inner);
    outer.finalize().into()
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
    let want = hmac_sha256(&secret.0, body);
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

/// Parse the arguments after `channel`. `Ok(None)` when there are none (no intake). A non-loopback address and a
/// missing secret are errors here, before anything is bound.
pub fn parse_args(
    args: &[&str],
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<Option<WebhookArgs>> {
    let (mut addr, mut file, mut name) = (None::<&str>, None::<&str>, None::<&str>);
    let mut i = 0;
    while i < args.len() {
        let slot = match args[i] {
            "--webhook" => &mut addr,
            "--webhook-secret-file" => &mut file,
            "--webhook-secret-env" => &mut name,
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

// ---- the translation ----

/// One channel notification made from a webhook event. Every field is from a fixed set or a strict shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    event: &'static str,
    pr: Option<u64>,
    sha_short: Option<String>,
    state: Option<&'static str>,
    conclusion: Option<&'static str>,
    text: String,
}

impl Notice {
    /// `meta`: identifier keys, short values (`event`, `pr`, `sha_short`, `state`, `conclusion`).
    pub fn meta(&self) -> Value {
        let mut m = Map::new();
        m.insert("event".into(), Value::String(self.event.into()));
        if let Some(pr) = self.pr {
            m.insert("pr".into(), Value::String(pr.to_string()));
        }
        if let Some(s) = &self.sha_short {
            m.insert("sha_short".into(), Value::String(s.clone()));
        }
        if let Some(s) = self.state {
            m.insert("state".into(), Value::String(s.into()));
        }
        if let Some(c) = self.conclusion {
            m.insert("conclusion".into(), Value::String(c.into()));
        }
        Value::Object(m)
    }

    /// `content`: a fixed template and the notice that it is not an instruction.
    pub fn content(&self) -> String {
        format!("[klatch] {}. {WEBHOOK_NOTICE}", self.text)
    }

    /// The JSON-RPC notification Claude Code reads.
    pub fn notification(&self) -> Value {
        json!({
            "jsonrpc": "2.0",
            "method": "notifications/claude/channel",
            "params": {"content": self.content(), "meta": self.meta()},
        })
    }
}

/// The notice that says events were lost because the channel could not keep up.
pub fn dropped_notice(n: usize) -> Notice {
    Notice {
        event: "dropped",
        pr: None,
        sha_short: None,
        state: None,
        conclusion: None,
        text: format!(
            "GitHub: {n} webhook events were dropped because the queue was full; read the pull requests from GitHub if you need their state"
        ),
    }
}

/// What came of an event.
#[derive(Debug)]
pub enum Outcome {
    Notice(Notice),
    /// A well-formed event klatch does not report (a ping, a label, a check that has only been requested).
    Ignored,
}

/// The head sha last seen for each open pull request, from `pull_request` events (a bounded map).
#[derive(Debug, Default)]
pub struct Tracker {
    heads: HashMap<u64, String>,
}

impl Tracker {
    pub fn head(&self, pr: u64) -> Option<&str> {
        self.heads.get(&pr).map(String::as_str)
    }

    fn set(&mut self, pr: u64, sha: &str) {
        if self.heads.len() >= TRACKED_HEADS && !self.heads.contains_key(&pr) {
            self.heads.clear();
        }
        self.heads.insert(pr, sha.to_string());
    }

    fn forget(&mut self, pr: u64) {
        self.heads.remove(&pr);
    }
}

const PR_ACTIONS: &[&str] = &[
    "opened",
    "reopened",
    "closed",
    "synchronize",
    "ready_for_review",
    "converted_to_draft",
];
const PR_STATES: &[&str] = &["open", "closed"];
const MERGEABLE_STATES: &[&str] = &[
    "clean",
    "dirty",
    "blocked",
    "behind",
    "unstable",
    "has_hooks",
    "draft",
    "unknown",
];
const CONCLUSIONS: &[&str] = &[
    "success",
    "failure",
    "neutral",
    "cancelled",
    "skipped",
    "timed_out",
    "action_required",
    "stale",
    "startup_failure",
];

/// The member of a fixed set that `v` equals (a `&'static str` from the set, never the input's own text).
fn pick(v: &Value, set: &[&'static str]) -> Option<&'static str> {
    let s = v.as_str()?;
    set.iter().copied().find(|x| *x == s)
}

/// A git sha: 40 or 64 lower-case hex digits.
fn sha(v: &Value) -> Option<&str> {
    let s = v.as_str()?;
    ((s.len() == 40 || s.len() == 64)
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
    .then_some(s)
}

fn number(v: &Value) -> Option<u64> {
    v.as_u64().filter(|n| *n > 0)
}

fn short_sha(s: &str) -> String {
    s[..7].to_string()
}

/// Turn one event (the `X-GitHub-Event` value and the parsed body) into a notice, an ignore, or an error for a body that
/// does not have the structure its event promises.
pub fn translate(
    event: &str,
    body: &Value,
    tracker: &mut Tracker,
) -> Result<Outcome, &'static str> {
    if !body.is_object() {
        return Err("the body is not a JSON object");
    }
    match event {
        "pull_request" => pull_request(body, tracker),
        "check_suite" => ci(
            "check_suite",
            "check suite",
            &body["check_suite"],
            body,
            tracker,
        ),
        "workflow_run" => ci(
            "workflow_run",
            "workflow run",
            &body["workflow_run"],
            body,
            tracker,
        ),
        _ => Ok(Outcome::Ignored),
    }
}

fn pull_request(body: &Value, tracker: &mut Tracker) -> Result<Outcome, &'static str> {
    if !body["action"].is_string() {
        return Err("no action");
    }
    let Some(action) = pick(&body["action"], PR_ACTIONS) else {
        return Ok(Outcome::Ignored);
    };
    let n = number(&body["number"]).ok_or("no pull request number")?;
    let pr = &body["pull_request"];
    let state = pick(&pr["state"], PR_STATES).ok_or("no pull request state")?;
    let merged = pr["merged"].as_bool().unwrap_or(false);
    let head = sha(&pr["head"]["sha"]).ok_or("no head sha")?;
    let state = if state == "closed" && merged {
        "merged"
    } else {
        state
    };
    if state == "open" {
        tracker.set(n, head);
    } else {
        tracker.forget(n);
    }
    let mergeable = pick(&pr["mergeable_state"], MERGEABLE_STATES)
        .map(|m| format!(", mergeable_state {m}"))
        .unwrap_or_default();
    let short = short_sha(head);
    Ok(Outcome::Notice(Notice {
        event: "pull_request",
        pr: Some(n),
        text: format!(
            "GitHub: pull request #{n} {action} (state {state}, head {short}{mergeable})"
        ),
        sha_short: Some(short),
        state: Some(state),
        conclusion: None,
    }))
}

/// `check_suite` and `workflow_run`: `run` is the object that holds `head_sha`, `status`, `conclusion` and
/// `pull_requests`; `root` has the `repository`.
fn ci(
    event: &'static str,
    what: &str,
    run: &Value,
    root: &Value,
    tracker: &Tracker,
) -> Result<Outcome, &'static str> {
    if !root["action"].is_string() {
        return Err("no action");
    }
    if pick(&root["action"], &["completed"]).is_none() || !run.is_object() {
        return Ok(Outcome::Ignored);
    }
    let head = sha(&run["head_sha"]).ok_or("no head sha")?;
    if pick(&run["status"], &["completed"]).is_none() {
        return Ok(Outcome::Ignored);
    }
    let conclusion = pick(&run["conclusion"], CONCLUSIONS).ok_or("no conclusion")?;
    let short = short_sha(head);
    // the first attached pull request that has a number
    let attached = run["pull_requests"]
        .as_array()
        .and_then(|a| a.iter().find_map(|p| number(&p["number"]).map(|n| (n, p))));
    let Some((n, p)) = attached else {
        // no pull request: a run on the default branch is `main`; any other branch is `no-pr`. The branch name is only
        // compared with the default branch name, never reported.
        let on_main = run["head_branch"].is_string()
            && run["head_branch"] == root["repository"]["default_branch"];
        let (label, place) = if on_main {
            ("main", "on main")
        } else {
            ("no-pr", "on a branch with no pull request")
        };
        return Ok(Outcome::Notice(Notice {
            event,
            pr: None,
            text: format!("GitHub: {what} {place} at {short} completed: {conclusion}"),
            sha_short: Some(short),
            state: Some(label),
            conclusion: Some(conclusion),
        }));
    };
    // the pull request's current head: what GitHub says in this payload, else what a pull_request event told us
    let current = sha(&p["head"]["sha"]).or_else(|| tracker.head(n));
    let (conclusion, text) = match (conclusion, current) {
        ("cancelled", Some(c)) if c != head => (
            "superseded",
            format!(
                "GitHub: {what} for pull request #{n} at {short} was cancelled because a newer commit replaced that head: superseded, not a failure"
            ),
        ),
        ("cancelled", None) => (
            "cancelled",
            format!(
                "GitHub: {what} for pull request #{n} at {short} completed: cancelled (the event does not say whether this head is still current; not reported as a failure)"
            ),
        ),
        (c, _) => (
            c,
            format!("GitHub: {what} for pull request #{n} at {short} completed: {c}"),
        ),
    };
    Ok(Outcome::Notice(Notice {
        event,
        pr: Some(n),
        text,
        sha_short: Some(short),
        state: Some("completed"),
        conclusion: Some(conclusion),
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

/// Send what the queue holds to the channel once the client is `ready`, until `stop`. Nothing is sent before the client's
/// `notifications/initialized` (what arrives meanwhile waits in the bounded queue).
pub fn pump(queue: &Queue, out: &SharedOut, (ready, stop): (&AtomicBool, &AtomicBool)) {
    while !stop.load(Ordering::SeqCst) {
        if ready.load(Ordering::SeqCst) {
            let (notices, dropped) = queue.drain();
            let lost = (dropped > 0).then(|| dropped_notice(dropped));
            for n in lost.iter().chain(notices.iter()) {
                if out.send(&n.notification()).is_err() {
                    return;
                }
            }
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
}

impl<'a> Intake<'a> {
    pub fn new(secret: Secret, queue: &'a Queue) -> Intake<'a> {
        Intake {
            secret,
            queue,
            tracker: Tracker::default(),
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
        let Some(event) = request.head.header("x-github-event") else {
            return Response::new(400, "missing event header");
        };
        let Ok(body) = serde_json::from_slice::<Value>(&request.body) else {
            return Response::new(400, "body is not JSON");
        };
        match translate(event, &body, &mut self.tracker) {
            Ok(Outcome::Notice(n)) => {
                self.queue.push(n);
                Response::new(200, "ok")
            }
            Ok(Outcome::Ignored) => Response::new(200, "ignored"),
            Err(_) => Response::new(400, "unusable event"),
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
