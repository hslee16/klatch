//! `klatch-server channel` driven over stdio (#1078, from #1057): the time from another member's post to the push
//! notification on the channel, and the quiet of an idle channel. The behaviour itself (meta keys, no message text,
//! muted rooms) is checked by klatch-server's own conformance test; this scenario is the timing and the idle check.
//! Every wait on the child's output has a deadline: a channel that never answers fails the scenario, it never hangs it.

use super::*;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::Duration;

/// The longest a request or a notification is waited for.
const LIMIT: Duration = Duration::from_secs(20);
/// How long an idle channel must stay silent.
const QUIET: Duration = Duration::from_millis(2500);
/// A watcher looks at once when the store changes; give it this long to settle after a join.
const SETTLE: Duration = Duration::from_millis(700);

pub(super) fn all() -> Vec<Scenario> {
    vec![Scenario {
        name: "a_post_reaches_the_channel_as_one_notification_and_an_idle_channel_is_quiet",
        origin: "#1078, #1057",
        layer: "functional",
        run: latency_and_quiet,
    }]
}

/// One channel child with a reader thread that parses each output line.
struct Channel {
    child: Child,
    stdin: Option<ChildStdin>,
    rx: Receiver<Value>,
    seen: Vec<Value>,
    next_id: i64,
}

impl Channel {
    fn start(w: &World, who: &str) -> Result<Channel, String> {
        let mut child = w.spawn_channel(who)?;
        let stdout = child.stdout.take().ok_or("no standard output")?;
        let stdin = child.stdin.take();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Ok(v) = serde_json::from_str::<Value>(&line)
                    && tx.send(v).is_err()
                {
                    break;
                }
            }
        });
        let mut c = Channel {
            child,
            stdin,
            rx,
            seen: Vec::new(),
            next_id: 1,
        };
        let init = c.request(
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "bench", "version": "0"}}),
        )?;
        check(init["result"].is_object(), format!("initialize: {init}"))?;
        c.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))?;
        Ok(c)
    }

    fn send(&mut self, message: &Value) -> Result<(), String> {
        let stdin = self.stdin.as_mut().ok_or("standard input is closed")?;
        writeln!(stdin, "{message}")
            .and_then(|()| stdin.flush())
            .map_err(|e| format!("write to the channel: {e}"))
    }

    /// Send a request and return its response, within `LIMIT`; notifications that arrive meanwhile are kept.
    fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))?;
        let deadline = Instant::now() + LIMIT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(left) {
                Ok(v) if v["id"] == json!(id) => return Ok(v),
                Ok(v) => self.seen.push(v),
                Err(e) => return Err(format!("no answer to {method} within {LIMIT:?}: {e:?}")),
            }
        }
    }

    fn call(&mut self, tool: &str, arguments: Value) -> Result<(), String> {
        let r = self.request("tools/call", json!({"name": tool, "arguments": arguments}))?;
        check(
            r["error"].is_null() && r["result"]["isError"] != json!(true),
            format!("{tool} failed: {r}"),
        )
    }

    fn events(&self) -> usize {
        self.seen
            .iter()
            .filter(|v| v["method"] == "notifications/claude/channel")
            .count()
    }

    /// Keep reading for `window`, returning how many channel notifications were seen in all.
    fn listen(&mut self, window: Duration) -> usize {
        let end = Instant::now() + window;
        loop {
            let left = end.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(left) {
                Ok(v) => self.seen.push(v),
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => break,
            }
        }
        self.events()
    }

    /// Read until `n` notifications were seen or `LIMIT` passes; the elapsed milliseconds since `from` when they came.
    fn wait_for(&mut self, n: usize, from: Instant) -> Result<f64, String> {
        let deadline = from + LIMIT;
        while self.events() < n {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(left) {
                Ok(v) => self.seen.push(v),
                Err(e) => {
                    return Err(format!(
                        "no channel notification within {LIMIT:?} of the post: {e:?}"
                    ));
                }
            }
        }
        Ok(ms(from))
    }

    /// End the child: close its input and wait up to `LIMIT` for it, then kill it.
    fn stop(mut self) {
        self.stdin.take();
        let end = Instant::now() + LIMIT;
        while Instant::now() < end {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn latency_and_quiet(w: &World) -> Res {
    let mut alice = Channel::start(w, "alice")?;
    alice.call("klatch_join", json!({"room": "lobby"}))?;
    let mut bob = Channel::start(w, "bob")?;
    bob.call("klatch_join", json!({"room": "lobby"}))?;
    std::thread::sleep(SETTLE);
    // an idle channel is silent: the joins are drained first, then nothing more arrives
    let after_joins = alice.listen(QUIET);
    let still = alice.listen(QUIET);
    check(
        still == after_joins,
        format!(
            "an idle channel pushed {} notifications in {QUIET:?}",
            still - after_joins
        ),
    )?;
    // post to notification: three posts, each announced once; the middle value is kept
    let mut times = Vec::new();
    for i in 0..3 {
        let before = alice.events();
        let start = Instant::now();
        bob.call(
            "klatch_post",
            json!({"room": "lobby", "body": format!("bench message {i}")}),
        )?;
        times.push(alice.wait_for(before + 1, start)?);
    }
    times.sort_by(f64::total_cmp);
    let total = alice.events();
    let settled = alice.listen(QUIET);
    check(
        settled == total,
        format!("a post was announced more than once ({total} then {settled})"),
    )?;
    alice.stop();
    bob.stop();
    Ok(vec![("notify_ms".into(), times[1])])
}
