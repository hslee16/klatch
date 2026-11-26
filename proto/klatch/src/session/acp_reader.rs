//! The input of the ACP agent, read on a helper thread so a `session/cancel` is seen while a prompt is being handled
//! (#1042). The thread reads one line at a time and passes each on, in order, over a bounded channel. It also looks
//! at each line just enough to do two things: give every `session/prompt` request its own cancel flag, and, when a
//! `session/cancel` notification arrives, raise the flag of every prompt of that session that has not yet finished.
//! The notification itself is not passed on (the handler never answered one). Everything else, a malformed line
//! included, is passed on untouched for the handler to answer exactly as it always did.
//!
//! The thread does the one thing a stream can do: it ends. At the end of the input it drops its end of the channel,
//! which the handler sees as the input closing. A read error is passed on as the last item, then the thread ends.
//! When the channel is full the thread waits in `send` (the handler is busy), which stops reading, so a client that
//! sends more than [`QUEUE`] requests behind a prompt that is running waits too and cannot grow memory without limit;
//! its cancel is read once the handler has made room. An ACP client has one prompt per session outstanding, so a
//! full queue is a client that is not following the protocol.

use super::CancelHook;
use crate::sync::atomic::{AtomicBool, Ordering};
use crate::sync::{mpsc, thread};
use serde_json::Value;
use std::io::{self, BufRead};
use std::sync::Arc;

/// The most lines waiting for the handler. The handler holds one more, the one it is answering.
pub(crate) const QUEUE: usize = 64;

/// The flag of one prompt: raised when a `session/cancel` for its session arrives while it is still outstanding.
pub(crate) type Cancel = Arc<AtomicBool>;

/// One line of input and the cancel flag of the prompt it carries (`None` for any other message), or the read error.
pub(crate) type Line = io::Result<(String, Option<Cancel>)>;

/// The call id a prompt's look is made under, from the prompt's JSON-RPC request id (#1562). The reader names a look
/// to the backend's cancel hook by it and the handler makes the look under it, so the two agree without sharing state.
pub(crate) fn call_id(request_id: &Value) -> String {
    format!("acp-{request_id}")
}

/// Read `input` on its own thread. The handle is for a caller that knows the input has ended or failed.
#[cfg_attr(not(shuttle), allow(dead_code))]
pub(crate) fn spawn<R: BufRead + Send + 'static>(
    input: R,
) -> (mpsc::Receiver<Line>, thread::JoinHandle<()>) {
    spawn_with(input, Arc::new(super::NoCancel))
}

/// `spawn`, and a cancel also goes to `hook` for the look the cancelled prompt has in the backend (#1562).
pub(crate) fn spawn_with<R: BufRead + Send + 'static>(
    input: R,
    hook: Arc<dyn CancelHook>,
) -> (mpsc::Receiver<Line>, thread::JoinHandle<()>) {
    let (tx, rx) = mpsc::sync_channel(QUEUE);
    let handle = thread::spawn(move || {
        let mut outstanding: Vec<Outstanding> = Vec::new();
        for line in input.lines() {
            let line = match line {
                Ok(line) => line,
                Err(e) => {
                    let _ = tx.send(Err(e));
                    return;
                }
            };
            if line.trim().is_empty() {
                continue;
            }
            // A prompt whose handler has let go of its flag is over; keep only those still outstanding.
            outstanding.retain(|p| Arc::strong_count(&p.flag) > 1);
            let Some(flag) = classify(&line, &mut outstanding, hook.as_ref()) else {
                continue; // a cancel, dealt with
            };
            if tx.send(Ok((line, flag))).is_err() {
                return; // the handler has stopped listening
            }
        }
    });
    (rx, handle)
}

/// A prompt the handler has not finished: its session, the call id of its look and its flag.
struct Outstanding {
    session: String,
    call_id: String,
    flag: Cancel,
}

/// `None` for a `session/cancel` notification (after raising the flags it names); otherwise `Some` of the flag of the
/// prompt the line carries, if it is a `session/prompt` request for a named session.
fn classify(
    line: &str,
    outstanding: &mut Vec<Outstanding>,
    hook: &dyn CancelHook,
) -> Option<Option<Cancel>> {
    let Ok(message) = serde_json::from_str::<Value>(line) else {
        return Some(None);
    };
    let method = message.get("method").and_then(Value::as_str);
    let session = message.pointer("/params/sessionId").and_then(Value::as_str);
    let is_request = message.get("id").is_some();
    match (method, is_request, session) {
        (Some("session/cancel"), false, session) => {
            for p in outstanding.iter() {
                if Some(p.session.as_str()) == session {
                    p.flag.store(true, Ordering::SeqCst);
                    // The look may already be inside the backend, where the flag is not looked at (#1562).
                    hook.cancel(&p.call_id);
                }
            }
            None
        }
        (Some("session/prompt"), true, Some(session)) => {
            let flag: Cancel = Arc::new(AtomicBool::new(false));
            outstanding.push(Outstanding {
                session: session.to_string(),
                call_id: call_id(message.get("id").unwrap_or(&Value::Null)),
                flag: flag.clone(),
            });
            Some(Some(flag))
        }
        _ => Some(None),
    }
}
