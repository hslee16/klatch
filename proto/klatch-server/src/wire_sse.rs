//! The frames of a session as server-sent events (#1059; split from `wire_http` for #1449): one frame line is one event,
//! and [`SseWriter`] turns the bytes a session writes into events, a whole line at a time.

use klatch::protocol::MAX_LINE_BYTES;
use serde_json::Value;
use std::io::{self, Write};

/// The frame's type as an event name: only lower-case letters and underscores, so a name never breaks an event.
fn event_type(frame: &Value) -> Option<&str> {
    frame["type"]
        .as_str()
        .filter(|t| t.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'))
}

/// `<room>:<seq>` for a message frame that has both, and a room of printable ASCII.
fn message_id(frame: &Value) -> Option<String> {
    let (room, seq) = (frame["room"].as_str()?, frame["seq"].as_u64()?);
    room.bytes()
        .all(|b| (0x21..0x7f).contains(&b))
        .then(|| format!("{room}:{seq}"))
}

/// One frame line (without its newline) as one server-sent event. The event name is the frame's type, a `message`
/// frame carries `id: <room>:<seq>`, and the data is the line exactly as the stdio session writes it.
pub(crate) fn sse_event(line: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(line);
    let frame: Option<Value> = serde_json::from_str(&text).ok();
    let mut out = String::new();
    if let Some(f) = &frame
        && let Some(t) = event_type(f)
    {
        out.push_str(&format!("event: {t}\n"));
        if let Some(id) = message_id(f).filter(|_| t == "message") {
            out.push_str(&format!("id: {id}\n"));
        }
    }
    // a frame line holds no raw newline (JSON escapes them); a stray carriage return would end an SSE line early
    out.push_str(&format!("data: {}\n\n", text.replace('\r', "\\r")));
    out.into_bytes()
}

/// The output of a session: bytes in, whole lines out as events. The session writes one frame per write, but a line is
/// only an event once its newline has come.
pub(crate) struct SseWriter<W: Write> {
    out: W,
    pending: Vec<u8>,
}

impl<W: Write> SseWriter<W> {
    pub(crate) fn new(out: W) -> Self {
        SseWriter {
            out,
            pending: Vec::new(),
        }
    }
}

impl<W: Write> Write for SseWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.pending.extend_from_slice(buf);
        while let Some(at) = self.pending.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=at).collect();
            let event = sse_event(&line[..at]);
            self.out.write_all(&event)?;
        }
        if self.pending.len() > 2 * MAX_LINE_BYTES {
            return Err(io::Error::other("a frame is over the size limit"));
        }
        self.out.flush()?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn events(out: &[u8]) -> Vec<String> {
        String::from_utf8(out.to_vec())
            .unwrap()
            .split("\n\n")
            .filter(|e| !e.is_empty())
            .map(String::from)
            .collect()
    }

    #[test]
    fn an_event_carries_the_type_the_line_as_data_and_for_a_message_its_room_and_sequence_number() {
        let line = br#"{"v":1,"type":"message","room":"lobby","seq":42,"id":"m","sender":"a","body":"hi"}"#;
        let e = String::from_utf8(sse_event(line)).unwrap();
        assert_eq!(
            e,
            format!(
                "event: message\nid: lobby:42\ndata: {}\n\n",
                std::str::from_utf8(line).unwrap()
            )
        );
        let e = String::from_utf8(sse_event(br#"{"v":1,"type":"heartbeat","ts":5}"#)).unwrap();
        assert_eq!(
            e,
            "event: heartbeat\ndata: {\"v\":1,\"type\":\"heartbeat\",\"ts\":5}\n\n"
        );
        // no id for a frame that is not a message, none when the room is odd
        let e =
            String::from_utf8(sse_event(br#"{"type":"caught_up","room":"r","seq":3}"#)).unwrap();
        assert!(!e.contains("\nid:"), "{e}");
        let e = String::from_utf8(sse_event(
            "{\"type\":\"message\",\"room\":\"a b\\n\",\"seq\":1}".as_bytes(),
        ))
        .unwrap();
        assert!(!e.contains("\nid:"), "{e}");
    }

    #[test]
    fn a_line_that_is_not_json_is_still_one_data_line_and_never_breaks_the_event() {
        for bad in [&b"not json"[..], b"{\"type\":\"x y\"}", b"a\rb: c"] {
            let e = String::from_utf8(sse_event(bad)).unwrap();
            assert!(e.ends_with("\n\n"), "{e:?}");
            let lines: Vec<&str> = e.trim_end_matches('\n').split('\n').collect();
            assert!(
                lines
                    .iter()
                    .all(|l| l.starts_with("data: ") || l.starts_with("event: ")),
                "{e:?}"
            );
            assert_eq!(
                lines.iter().filter(|l| l.starts_with("data: ")).count(),
                1,
                "{e:?}"
            );
            assert!(!e.contains('\r'), "{e:?}");
        }
    }

    #[test]
    fn the_writer_makes_an_event_of_each_whole_line_and_holds_a_partial_one() {
        let mut out = Vec::new();
        {
            let mut w = SseWriter::new(&mut out);
            w.write_all(b"{\"type\":\"hel").unwrap();
            w.write_all(b"lo\"}\n{\"type\":\"heartbeat\"}\n{\"typ")
                .unwrap();
        }
        let evs = events(&out);
        assert_eq!(evs.len(), 2, "{evs:?}");
        assert!(evs[0].starts_with("event: hello\n"));
        assert!(evs[1].starts_with("event: heartbeat\n"));
    }

    #[test]
    fn the_writer_refuses_a_line_with_no_end_that_grows_without_bound() {
        let mut w = SseWriter::new(Vec::new());
        let chunk = vec![b'x'; MAX_LINE_BYTES];
        assert!(w.write_all(&chunk).is_ok());
        assert!(w.write_all(&chunk).is_ok());
        assert!(w.write_all(&chunk).is_err());
    }
}
