use crate::PROTOCOL_VERSION;
use crate::frames::{ClientFrame, ServerFrame};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::io::BufRead;

/// The longest line, in bytes, a reader or decoder will accept. Longer lines are refused, not truncated.
pub const MAX_LINE_BYTES: usize = 1_048_576;

#[derive(Debug, PartialEq, Eq)]
pub enum DecodeError {
    Empty,
    TooLong(usize),
    Io(String),
    Json(String),
    MissingVersion,
    UnsupportedVersion(u64),
    Shape(String),
}

/// One frame as one line, newline terminated. JSON escaping keeps embedded newlines out of the line.
pub fn encode_line<T: Serialize>(frame: &T) -> String {
    let mut s = serde_json::to_string(frame).expect("frames always serialise");
    s.push('\n');
    s
}

fn parse(line: &str) -> Result<Value, DecodeError> {
    if line.len() > MAX_LINE_BYTES {
        return Err(DecodeError::TooLong(line.len()));
    }
    let text = line.trim();
    if text.is_empty() {
        return Err(DecodeError::Empty);
    }
    let value: Value = serde_json::from_str(text).map_err(|e| DecodeError::Json(e.to_string()))?;
    let object = value
        .as_object()
        .ok_or_else(|| DecodeError::Shape("a frame must be a JSON object".into()))?;
    match object.get("v") {
        None => Err(DecodeError::MissingVersion),
        Some(v) => match v.as_u64() {
            Some(n) if n == u64::from(PROTOCOL_VERSION) => Ok(value),
            Some(n) => Err(DecodeError::UnsupportedVersion(n)),
            None => Err(DecodeError::Shape(
                "v must be a non-negative integer".into(),
            )),
        },
    }
}

fn shape<T: DeserializeOwned>(value: Value) -> Result<T, DecodeError> {
    serde_json::from_value(value).map_err(|e| DecodeError::Shape(e.to_string()))
}

pub fn decode_server_line(line: &str) -> Result<ServerFrame, DecodeError> {
    let frame: ServerFrame = shape(parse(line)?)?;
    match &frame {
        ServerFrame::Message { seq: 0, .. } => {
            Err(DecodeError::Shape("a message seq starts at 1".into()))
        }
        ServerFrame::Response {
            ok: true,
            error: Some(_),
            ..
        } => Err(DecodeError::Shape(
            "a successful response carries no error".into(),
        )),
        ServerFrame::Response {
            ok: false,
            error: None,
            ..
        } => Err(DecodeError::Shape(
            "a failed response must carry an error".into(),
        )),
        _ => Ok(frame),
    }
}

pub fn decode_client_line(line: &str) -> Result<ClientFrame, DecodeError> {
    shape(parse(line)?)
}

/// Reads frames from any `BufRead`, skipping blank lines and resynchronising after an oversize line.
pub struct FrameReader<R: BufRead> {
    inner: R,
    done: bool,
}

impl<R: BufRead> FrameReader<R> {
    pub fn new(inner: R) -> Self {
        Self { inner, done: false }
    }

    /// The next non-blank line, or `None` at the end of input. An oversize line is consumed to its end
    /// without being stored, and reported as `TooLong`.
    fn next_line(&mut self) -> Option<Result<String, DecodeError>> {
        if self.done {
            return None;
        }
        let mut line: Vec<u8> = Vec::new();
        let mut total = 0usize;
        let mut saw_any = false;
        loop {
            let buf = match self.inner.fill_buf() {
                Ok(b) => b,
                Err(e) => {
                    self.done = true;
                    return Some(Err(DecodeError::Io(e.to_string())));
                }
            };
            if buf.is_empty() {
                self.done = true;
                if !saw_any {
                    return None;
                }
                break;
            }
            saw_any = true;
            let (chunk, ended) = match buf.iter().position(|b| *b == b'\n') {
                Some(i) => (&buf[..i], Some(i + 1)),
                None => (buf, None),
            };
            total += chunk.len();
            if total <= MAX_LINE_BYTES {
                line.extend_from_slice(chunk);
            }
            let consumed = ended.unwrap_or(chunk.len());
            self.inner.consume(consumed);
            if ended.is_some() {
                break;
            }
        }
        if total > MAX_LINE_BYTES {
            return Some(Err(DecodeError::TooLong(total)));
        }
        Some(String::from_utf8(line).map_err(|e| DecodeError::Json(e.to_string())))
    }

    fn next_with<T>(
        &mut self,
        decode: fn(&str) -> Result<T, DecodeError>,
    ) -> Option<Result<T, DecodeError>> {
        loop {
            match self.next_line()? {
                Ok(line) if line.trim().is_empty() => continue,
                Ok(line) => return Some(decode(&line)),
                Err(e) => return Some(Err(e)),
            }
        }
    }

    pub fn next_server(&mut self) -> Option<Result<ServerFrame, DecodeError>> {
        self.next_with(decode_server_line)
    }

    pub fn next_client(&mut self) -> Option<Result<ClientFrame, DecodeError>> {
        self.next_with(decode_client_line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frames::{Attribution, ErrorBody};
    use serde_json::{Value, json};

    const SERVER: &str = include_str!("../fixtures/server_basic.ndjson");
    const CLIENT: &str = include_str!("../fixtures/client_basic.ndjson");
    const FORWARD: &str = include_str!("../fixtures/forward_compat.ndjson");

    fn json_of(line: &str) -> Value {
        serde_json::from_str(line).unwrap()
    }

    #[test]
    fn every_server_fixture_line_round_trips() {
        for line in SERVER.lines() {
            let frame = decode_server_line(line).unwrap_or_else(|e| panic!("{line}: {e:?}"));
            assert_ne!(frame, ServerFrame::Unknown, "{line}");
            assert_eq!(
                json_of(encode_line(&frame).trim_end()),
                json_of(line),
                "{line}"
            );
        }
    }

    #[test]
    fn every_client_fixture_line_round_trips() {
        for line in CLIENT.lines() {
            let frame = decode_client_line(line).unwrap_or_else(|e| panic!("{line}: {e:?}"));
            assert_ne!(frame, ClientFrame::Unknown, "{line}");
            let mut expected = json_of(line);
            if expected.get("params").is_none() && expected["type"] == "request" {
                expected["params"] = Value::Null; // an omitted params round-trips as null
            }
            assert_eq!(json_of(encode_line(&frame).trim_end()), expected, "{line}");
        }
    }

    #[test]
    fn a_message_frame_has_the_documented_fields() {
        let f = decode_server_line(SERVER.lines().nth(1).unwrap()).unwrap();
        match f {
            ServerFrame::Message {
                room,
                seq,
                sender,
                attribution,
                label,
                body,
                ..
            } => {
                assert_eq!((room.as_str(), seq, sender.as_str()), ("core", 1, "bob"));
                assert_eq!(
                    (attribution, label.as_str(), body.as_str()),
                    (Attribution::Verified, "internal", "claiming src/a.rs")
                );
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_unattributed_sender_is_explicit_on_the_wire() {
        let f = decode_server_line(SERVER.lines().nth(2).unwrap()).unwrap();
        assert!(matches!(
            f,
            ServerFrame::Message {
                attribution: Attribution::Unattributed,
                ..
            }
        ));
    }

    #[test]
    fn a_missing_attribution_is_rejected() {
        let line = r#"{"v":1,"type":"message","room":"r","seq":1,"id":"x","sender":"b","label":"public","body":"hi","ts":1}"#;
        assert!(matches!(
            decode_server_line(line),
            Err(DecodeError::Shape(_))
        ));
    }

    #[test]
    fn a_message_with_seq_zero_is_rejected() {
        let line = r#"{"v":1,"type":"message","room":"r","seq":0,"id":"x","sender":"b","attribution":"verified","label":"public","body":"hi","ts":1}"#;
        assert!(matches!(
            decode_server_line(line),
            Err(DecodeError::Shape(_))
        ));
    }

    #[test]
    fn caught_up_may_report_seq_zero_for_an_empty_room() {
        let f = decode_server_line(r#"{"v":1,"type":"caught_up","room":"r","seq":0}"#).unwrap();
        assert!(matches!(f, ServerFrame::CaughtUp { seq: 0, .. }));
    }

    #[test]
    fn newlines_in_a_body_are_escaped_so_a_frame_is_one_line() {
        let f = decode_server_line(SERVER.lines().nth(2).unwrap()).unwrap();
        let line = encode_line(&f);
        assert!(line.ends_with('\n'));
        assert_eq!(line.matches('\n').count(), 1);
        match decode_server_line(line.trim_end()).unwrap() {
            ServerFrame::Message { body, .. } => assert_eq!(body, "line one\nline two"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn unknown_types_and_extra_fields_are_tolerated() {
        let mut lines = FORWARD.lines();
        assert_eq!(
            decode_server_line(lines.next().unwrap()).unwrap(),
            ServerFrame::Unknown
        );
        assert!(matches!(
            decode_server_line(lines.next().unwrap()).unwrap(),
            ServerFrame::Message { seq: 4, .. }
        ));
    }

    #[test]
    fn a_missing_version_is_rejected() {
        assert_eq!(
            decode_server_line(r#"{"type":"heartbeat","ts":1}"#),
            Err(DecodeError::MissingVersion)
        );
    }

    #[test]
    fn another_major_version_is_rejected() {
        assert_eq!(
            decode_server_line(r#"{"v":2,"type":"heartbeat","ts":1}"#),
            Err(DecodeError::UnsupportedVersion(2))
        );
        assert_eq!(
            decode_client_line(r#"{"v":0,"type":"ack","room":"r","seq":1}"#),
            Err(DecodeError::UnsupportedVersion(0))
        );
    }

    #[test]
    fn garbage_blank_and_non_object_lines_are_errors() {
        assert!(matches!(
            decode_server_line("not json"),
            Err(DecodeError::Json(_))
        ));
        assert_eq!(decode_server_line("   "), Err(DecodeError::Empty));
        assert!(matches!(
            decode_server_line("[1,2]"),
            Err(DecodeError::Shape(_))
        ));
    }

    #[test]
    fn a_response_must_agree_with_its_ok_flag() {
        let bad_ok =
            r#"{"v":1,"type":"response","id":"r","ok":true,"error":{"code":"x","message":"y"}}"#;
        let bad_fail = r#"{"v":1,"type":"response","id":"r","ok":false}"#;
        assert!(matches!(
            decode_server_line(bad_ok),
            Err(DecodeError::Shape(_))
        ));
        assert!(matches!(
            decode_server_line(bad_fail),
            Err(DecodeError::Shape(_))
        ));
        let ok = decode_server_line(SERVER.lines().nth(7).unwrap()).unwrap();
        assert!(matches!(
            ok,
            ServerFrame::Response {
                ok: false,
                error: Some(ErrorBody { .. }),
                ..
            }
        ));
    }

    #[test]
    fn a_refusal_carries_the_rule_and_a_reason_and_nothing_that_could_echo_content() {
        let f = decode_server_line(SERVER.lines().nth(5).unwrap()).unwrap();
        let keys: Vec<String> = json_of(encode_line(&f).trim_end())
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        let mut keys = keys;
        keys.sort();
        assert_eq!(keys, vec!["reason", "request", "rule", "type", "v"]);
    }

    #[test]
    fn an_oversize_line_is_rejected_by_the_decoder() {
        let long = format!(
            r#"{{"v":1,"type":"heartbeat","ts":1,"pad":"{}"}}"#,
            "x".repeat(MAX_LINE_BYTES)
        );
        assert!(matches!(
            decode_server_line(&long),
            Err(DecodeError::TooLong(_))
        ));
    }

    #[test]
    fn the_reader_skips_blank_lines_and_resynchronises_after_an_oversize_line() {
        let good = r#"{"v":1,"type":"heartbeat","ts":1}"#;
        let huge = "x".repeat(MAX_LINE_BYTES * 2);
        let input = format!("{good}\n\n{huge}\n{good}\n");
        let mut r = FrameReader::new(std::io::Cursor::new(input));
        assert!(matches!(
            r.next_server(),
            Some(Ok(ServerFrame::Heartbeat { .. }))
        ));
        assert!(matches!(
            r.next_server(),
            Some(Err(DecodeError::TooLong(_)))
        ));
        assert!(matches!(
            r.next_server(),
            Some(Ok(ServerFrame::Heartbeat { .. }))
        ));
        assert!(r.next_server().is_none());
    }

    #[test]
    fn the_reader_handles_a_final_line_without_a_newline() {
        let mut r = FrameReader::new(std::io::Cursor::new(r#"{"v":1,"type":"heartbeat","ts":1}"#));
        assert!(matches!(r.next_server(), Some(Ok(_))));
        assert!(r.next_server().is_none());
    }

    #[test]
    fn client_frames_decode_from_the_reader() {
        let mut r = FrameReader::new(std::io::Cursor::new(CLIENT));
        let mut n = 0;
        while let Some(f) = r.next_client() {
            f.unwrap();
            n += 1;
        }
        assert_eq!(n, 4);
        let _ = json!(null);
    }

    const SPEC: &str = include_str!("../../../PROTOCOL.md");

    #[test]
    fn the_written_spec_documents_every_frame_type_and_error_rule() {
        for name in [
            "hello",
            "message",
            "caught_up",
            "heartbeat",
            "refusal",
            "response",
            "request",
            "ack",
            "unattributed",
            "MAX_LINE_BYTES",
            "\"v\"",
        ] {
            assert!(SPEC.contains(name), "PROTOCOL.md does not mention {name}");
        }
        for method in [
            "subscribe",
            "unsubscribe",
            "post",
            "read",
            "who",
            "claim",
            "release",
            "claims",
        ] {
            assert!(
                SPEC.contains(&format!("`{method}`")),
                "PROTOCOL.md does not list method {method}"
            );
        }
    }
}
