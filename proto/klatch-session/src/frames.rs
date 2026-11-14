//! Building the frames a session sends: responses, refusals, messages, and the output they are written to.

use klatch_core::{Attribution as Who, Error, Message};
use klatch_protocol::{
    Attribution, DecodeError, ErrorBody, PROTOCOL_VERSION, ServerFrame, encode_line,
};
use serde_json::{Value, json};
use std::io::{self, Write};
use std::time::Instant;

/// The output stream and when anything was last written to it.
pub(crate) struct Out<W: Write> {
    pub w: W,
    pub last: Instant,
}

impl<W: Write> Out<W> {
    pub fn new(w: W) -> Self {
        Self {
            w,
            last: Instant::now(),
        }
    }

    pub fn send(&mut self, frame: &ServerFrame) -> io::Result<()> {
        self.w.write_all(encode_line(frame).as_bytes())?;
        self.w.flush()?;
        self.last = Instant::now();
        Ok(())
    }

    pub fn send_all(&mut self, frames: &[ServerFrame]) -> io::Result<()> {
        frames.iter().try_for_each(|f| self.send(f))
    }
}

/// A refusal for a stream failure. Transient lock contention (`Busy`) is retried and never reported.
pub(crate) fn stream_refusal(e: &Error) -> Option<ServerFrame> {
    let (rule, reason) = match e {
        Error::Busy(_) => return None,
        Error::Refused { rule, reason } => (rule.clone(), reason.clone()),
        Error::Backend(m) => ("stream.backend".to_string(), m.clone()),
        Error::Invalid(m) => ("stream.invalid".to_string(), m.clone()),
        Error::NotFound(m) => ("stream.not_found".to_string(), m.clone()),
        Error::Conflict { .. } => ("stream.conflict".to_string(), e.to_string()),
    };
    Some(ServerFrame::Refusal {
        v: PROTOCOL_VERSION,
        request: None,
        rule,
        reason,
    })
}

pub(crate) fn message_frame(room: &str, m: &Message) -> ServerFrame {
    ServerFrame::Message {
        v: PROTOCOL_VERSION,
        room: room.to_string(),
        seq: u64::try_from(m.seq).unwrap_or(0),
        id: m.id.clone(),
        sender: m.sender.clone(),
        attribution: attribution(m.attribution),
        label: m.label.clone(),
        body: m.body.clone(),
        ts: m.ts,
        kind: m.kind.clone(),
        payload: m.payload.clone(),
    }
}

pub(crate) fn attribution(a: Who) -> Attribution {
    match a {
        Who::Verified => Attribution::Verified,
        Who::Unattributed => Attribution::Unattributed,
    }
}

/// A refusal for a frame that could not be read. Says what was wrong, never what the frame contained.
pub(crate) fn decode_refusal(e: &DecodeError) -> ServerFrame {
    let (rule, reason) = match e {
        DecodeError::MissingVersion => (
            "protocol.version",
            "the frame has no protocol version".to_string(),
        ),
        DecodeError::UnsupportedVersion(n) => (
            "protocol.version",
            format!(
                "protocol version {n} is not supported; this gateway speaks {PROTOCOL_VERSION}"
            ),
        ),
        DecodeError::TooLong(_) => (
            "protocol.decode",
            "the line is longer than the limit".to_string(),
        ),
        DecodeError::Empty | DecodeError::Json(_) | DecodeError::Shape(_) | DecodeError::Io(_) => {
            ("protocol.decode", "the frame could not be read".to_string())
        }
    };
    ServerFrame::Refusal {
        v: PROTOCOL_VERSION,
        request: None,
        rule: rule.into(),
        reason,
    }
}

pub(crate) fn ok(id: String, result: Value) -> ServerFrame {
    ServerFrame::Response {
        v: PROTOCOL_VERSION,
        id,
        ok: true,
        result: Some(result),
        error: None,
    }
}

pub(crate) fn fail_with(id: String, code: &str, message: String) -> ServerFrame {
    ServerFrame::Response {
        v: PROTOCOL_VERSION,
        id,
        ok: false,
        result: None,
        error: Some(ErrorBody {
            code: code.into(),
            message,
        }),
    }
}

/// The code and message of a failed request.
fn error_body(e: &Error) -> (&'static str, String) {
    match e {
        Error::Invalid(m) => ("invalid", m.clone()),
        Error::Conflict { .. } => ("conflict", e.to_string()),
        Error::Refused { reason, .. } => ("refused", reason.clone()),
        Error::NotFound(m) => ("not_found", m.clone()),
        Error::Busy(m) => ("busy", m.clone()),
        Error::Backend(m) => ("backend", m.clone()),
    }
}

/// The frames that answer a failed request. A policy refusal also names its rule in a `refusal` frame.
pub(crate) fn failure(id: String, e: &Error) -> Vec<ServerFrame> {
    let (code, message) = error_body(e);
    let mut frames = Vec::new();
    if let Error::Refused { rule, reason } = e {
        frames.push(ServerFrame::Refusal {
            v: PROTOCOL_VERSION,
            request: Some(id.clone()),
            rule: rule.clone(),
            reason: reason.clone(),
        });
    }
    frames.push(fail_with(id, code, message));
    frames
}

pub(crate) fn message_json(m: &Message) -> Value {
    let mut v = json!({
        "id": m.id, "seq": m.seq, "sender": m.sender,
        "attribution": match m.attribution {
            Who::Verified => "verified",
            Who::Unattributed => "unattributed",
        },
        "label": m.label, "body": m.body, "ts": m.ts
    });
    // a handoff is added when there is one, so an ordinary message looks as it always did
    if let Some(k) = &m.kind {
        v["kind"] = json!(k);
    }
    if let Some(p) = &m.payload {
        v["payload"] = json!(p);
    }
    v
}
