//! A WebSocket test client written by hand (#112): it can send any frame a conforming client sends and the ones a broken
//! client sends (unmasked, oversize, fragmented, a control frame that is too long), which a library would not let a test do.
//! It reads the server's frames, which must never be masked. Plain TCP (the loopback listener) or TLS, through `Net`.
#![allow(dead_code)]

use super::scenario::{normal, scenario};
use super::wire::{Conn, Net, read_head};
use serde_json::{Value, json};
use std::io::{BufReader, Read, Write};
use std::time::Duration;

/// The example key and the accept value of RFC 6455 section 1.3.
pub const KEY: &str = "dGhlIHNhbXBsZSBub25jZQ==";
pub const ACCEPT: &str = "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=";

pub const TEXT: u8 = 0x1;
pub const BINARY: u8 = 0x2;
pub const CONT: u8 = 0x0;
pub const CLOSE: u8 = 0x8;
pub const PING: u8 = 0x9;
pub const PONG: u8 = 0xA;

/// The upgrade request, with the credential of a member when given and any extra headers.
pub fn upgrade_request(headers: &[(&str, &str)]) -> String {
    let mut head = format!(
        "GET /v1/ws HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {KEY}\r\nSec-WebSocket-Version: 13\r\n"
    );
    for (n, v) in headers {
        head.push_str(&format!("{n}: {v}\r\n"));
    }
    head.push_str("\r\n");
    head
}

/// The credential headers of a member.
pub fn credential(member: &str, token: &str) -> [(String, String); 2] {
    [
        ("X-Klatch-Member".to_string(), member.to_string()),
        ("Authorization".to_string(), format!("Bearer {token}")),
    ]
}

/// One frame the server sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    Text(String),
    Binary(Vec<u8>),
    Ping(Vec<u8>),
    Pong(Vec<u8>),
    /// The status code (when the frame has one) and the reason.
    Close(Option<u16>, String),
}

/// What a read of the next frame found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Got {
    Frame(Frame),
    /// The connection ended (a close of the transport, or a reset).
    Eof,
    /// Nothing arrived within the read timeout.
    Quiet,
}

fn quiet(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    )
}

/// A WebSocket connection that has been upgraded.
pub struct Ws {
    pub r: BufReader<Conn>,
    pub next: u32,
    /// Frames a request skipped over while it waited for its response.
    pub extra: Vec<Value>,
}

impl Net {
    /// Send an upgrade request (the credential and `extra` headers) and read the head of the answer.
    pub fn ws_upgrade(
        &self,
        extra: &[(&str, &str)],
    ) -> (u16, Vec<(String, String)>, BufReader<Conn>) {
        let mut c = self.connect();
        c.write_all(upgrade_request(extra).as_bytes()).unwrap();
        c.flush().unwrap();
        let mut r = BufReader::new(c);
        let (status, headers) = read_head(&mut r);
        (status, headers, r)
    }

    /// Open a WebSocket as `member`; the upgrade must succeed.
    pub fn ws_open(&self, member: &str, token: &str) -> Ws {
        let creds = credential(member, token);
        let extra: Vec<(&str, &str)> = creds
            .iter()
            .map(|(n, v)| (n.as_str(), v.as_str()))
            .collect();
        let (status, headers, r) = self.ws_upgrade(&extra);
        assert_eq!(status, 101, "{headers:?}");
        let accept = headers
            .iter()
            .find(|(n, _)| n == "sec-websocket-accept")
            .map(|(_, v)| v.as_str());
        assert_eq!(accept, Some(ACCEPT), "{headers:?}");
        Ws {
            r,
            next: 0,
            extra: Vec::new(),
        }
    }
}

/// The bytes of one frame. A masked frame uses a fixed key that is not all zero (a server that skipped unmasking would
/// show it).
pub fn encode(fin: bool, opcode: u8, payload: &[u8], masked: bool) -> Vec<u8> {
    const MASK: [u8; 4] = [0x37, 0xfa, 0x21, 0x3d];
    let mut out = vec![(if fin { 0x80 } else { 0 }) | opcode];
    let mask_bit = if masked { 0x80 } else { 0 };
    match payload.len() {
        n if n < 126 => out.push(mask_bit | n as u8),
        n if n <= 0xffff => {
            out.push(mask_bit | 126);
            out.extend_from_slice(&(n as u16).to_be_bytes());
        }
        n => {
            out.push(mask_bit | 127);
            out.extend_from_slice(&(n as u64).to_be_bytes());
        }
    }
    if masked {
        out.extend_from_slice(&MASK);
        out.extend(payload.iter().enumerate().map(|(i, b)| b ^ MASK[i % 4]));
    } else {
        out.extend_from_slice(payload);
    }
    out
}

/// A frame header that declares `len` payload bytes and carries none (a client that lies about a size).
pub fn header_only(opcode: u8, len: u64) -> Vec<u8> {
    let mut out = vec![0x80 | opcode, 0x80 | 127];
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(&[0x37, 0xfa, 0x21, 0x3d]);
    out
}

impl Ws {
    pub fn set_timeout(&self, d: Duration) {
        self.r.get_ref().set_read_timeout(d);
    }

    /// Write bytes as they are.
    pub fn send_raw(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        let w = self.r.get_mut();
        w.write_all(bytes)?;
        w.flush()
    }

    pub fn send(&mut self, fin: bool, opcode: u8, payload: &[u8]) {
        self.send_raw(&encode(fin, opcode, payload, true)).unwrap();
    }

    pub fn text(&mut self, s: &str) {
        self.send(true, TEXT, s.as_bytes());
    }

    /// A close frame with a status code.
    pub fn close(&mut self, code: u16) {
        self.send(true, CLOSE, &code.to_be_bytes());
    }

    fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), Got> {
        match self.r.read_exact(buf) {
            Ok(()) => Ok(()),
            Err(e) if quiet(&e) => Err(Got::Quiet),
            Err(_) => Err(Got::Eof),
        }
    }

    /// The next frame, or that the connection ended or stayed quiet.
    pub fn got(&mut self) -> Got {
        match self.frame() {
            Ok(f) => Got::Frame(f),
            Err(g) => g,
        }
    }

    fn frame(&mut self) -> Result<Frame, Got> {
        let mut h = [0u8; 2];
        self.read_exact(&mut h)?;
        assert_eq!(h[1] & 0x80, 0, "the server sent a masked frame");
        assert_eq!(h[0] & 0x70, 0, "the server set a reserved bit");
        let len = match h[1] & 0x7f {
            126 => {
                let mut b = [0u8; 2];
                self.read_exact(&mut b)?;
                u16::from_be_bytes(b) as usize
            }
            127 => {
                let mut b = [0u8; 8];
                self.read_exact(&mut b)?;
                u64::from_be_bytes(b) as usize
            }
            n => n as usize,
        };
        let mut payload = vec![0u8; len];
        self.read_exact(&mut payload)?;
        assert!(h[0] & 0x80 != 0, "the server fragmented a message");
        Ok(match h[0] & 0x0f {
            TEXT => Frame::Text(String::from_utf8(payload).expect("text frames are UTF-8")),
            BINARY => Frame::Binary(payload),
            PING => Frame::Ping(payload),
            PONG => Frame::Pong(payload),
            CLOSE => {
                let code =
                    (payload.len() >= 2).then(|| u16::from_be_bytes([payload[0], payload[1]]));
                let reason = String::from_utf8_lossy(payload.get(2..).unwrap_or(&[])).to_string();
                Frame::Close(code, reason)
            }
            op => panic!("the server sent opcode {op}"),
        })
    }

    /// The next frame of the session (a text frame, parsed). Pings are answered and skipped. `None` when the connection
    /// ended, was quiet for the timeout, or the server closed it.
    pub fn recv(&mut self) -> Option<Value> {
        loop {
            match self.got() {
                Got::Frame(Frame::Text(t)) => return Some(serde_json::from_str(&t).unwrap()),
                Got::Frame(Frame::Ping(p)) => self.send(true, PONG, &p),
                Got::Frame(Frame::Pong(_)) => {}
                Got::Frame(f) => panic!("unexpected {f:?}"),
                Got::Eof | Got::Quiet => return None,
            }
        }
    }

    /// Everything the server sends until the connection ends: the frames, in order. A connection that is quiet, or that
    /// goes on sending pings for a minute, is a failure of the test (the server did not end it), never a hang.
    pub fn until_end(&mut self) -> Vec<Frame> {
        let until = std::time::Instant::now() + Duration::from_secs(60);
        let mut frames = Vec::new();
        loop {
            match self.got() {
                Got::Frame(f) => frames.push(f),
                Got::Eof => return frames,
                Got::Quiet => panic!("the server kept the connection open: {frames:?}"),
            }
            assert!(
                std::time::Instant::now() < until,
                "the server kept the connection open for a minute: {frames:?}"
            );
        }
    }

    /// Send a request and wait for its response (frames that arrive first are kept in `extra`).
    pub fn request(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        let id = format!("r{}", self.next);
        self.text(
            &json!({"v":1,"type":"request","id":id,"method":method,"params":params}).to_string(),
        );
        loop {
            let f = self
                .recv()
                .unwrap_or_else(|| panic!("closed before the response to {method}"));
            if f["type"] == "response" && f["id"] == id.as_str() {
                return f;
            }
            self.extra.push(f);
        }
    }

    pub fn post(&mut self, room: &str, body: &str) -> i64 {
        let r = self.request("post", json!({"room": room, "body": body}));
        assert_eq!(r["ok"], true, "{r}");
        r["result"]["seq"].as_i64().unwrap()
    }
}

/// The scenario of one member over WebSocket: the frames after the hello, in the order they arrived (as
/// `scenario::run_scenario` gives them over server-sent events).
pub fn run_scenario_ws(net: &Net, name: &str, token: &str) -> Vec<Value> {
    let mut c = net.ws_open(name, token);
    let hello = c.recv().unwrap();
    assert_eq!(
        (hello["member"].as_str(), hello["attribution"].as_str()),
        (Some(name), Some("verified"))
    );
    for f in scenario(name) {
        c.text(&f.to_string());
    }
    let mut frames = Vec::new();
    loop {
        let f = c.recv().expect("a frame");
        let last = f["type"] == "response" && f["id"] == "bad";
        frames.push(f);
        if last {
            break;
        }
    }
    c.set_timeout(Duration::from_millis(700));
    while let Some(f) = c.recv() {
        frames.push(f);
    }
    frames
        .into_iter()
        .filter(|f| f["type"] != "heartbeat")
        .map(|f| normal(f, true))
        .collect()
}

/// A loopback HTTP server (`klatch-server http`) with two members, for the tests of the WebSocket route on the plain listener.
pub struct PlainRig {
    pub dir: super::Dir,
    pub server: super::Server,
    pub net: Net,
    pub tokens: [String; 2],
}

/// Start `klatch-server http` with `extra` options (in a debug build with the knob for short limits set, as every server
/// of these tests is) and issue a credential to alice and to bob.
pub fn plain_server(extra: &[&str]) -> PlainRig {
    let dir = super::Dir::new();
    let tokens = [
        super::wire::issue_credential(&dir, "alice"),
        super::wire::issue_credential(&dir, "bob"),
    ];
    let args: Vec<String> = extra.iter().map(|s| s.to_string()).collect();
    let server = super::Server::started(&dir, "http", &args);
    let net = Net {
        addr: server.addr,
        tls: None,
    };
    PlainRig {
        dir,
        server,
        net,
        tokens,
    }
}

/// Everything a server answers to an upgrade request, to the end of the connection (a refusal ends it): the raw text.
pub fn raw_answer(net: &Net, head: &str) -> String {
    let mut c = net.connect();
    c.write_all(head.as_bytes()).unwrap();
    c.flush().unwrap();
    let mut out = Vec::new();
    let _ = c.read_to_end(&mut out);
    String::from_utf8_lossy(&out).into_owned()
}

/// The upgrade request of `member` as text, with the headers of `upgrade_request`.
pub fn upgrade_for(member: &str, token: &str) -> String {
    let creds = credential(member, token);
    let extra: Vec<(&str, &str)> = creds
        .iter()
        .map(|(n, v)| (n.as_str(), v.as_str()))
        .collect();
    upgrade_request(&extra)
}
