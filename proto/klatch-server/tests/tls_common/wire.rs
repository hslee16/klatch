//! The wire-level test client of the TLS listener tests: one client that speaks plain TCP (the loopback listener) or TLS
//! with a client certificate, server-sent events sessions, and the credentials of a throwaway store.
#![allow(dead_code)]

use super::*;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The library reads its environment (issuing a credential opens a store in this process): one test at a time.
pub static ENV: Mutex<()> = Mutex::new(());

pub fn issue_credential(dir: &Dir, member: &str) -> String {
    unsafe {
        std::env::set_var("KLATCH_HOME", dir.home());
        std::env::set_var("KLATCH_KEY_DIR", dir.home().join("keys"));
        std::env::remove_var("KLATCH_NAME");
        std::env::remove_var("KLATCH_IDENTITY_FILE");
    }
    let store = klatch::member::custody::open_store(&klatch::member::db_path().unwrap()).unwrap();
    let gateway =
        klatch::gateway::Gateway::new(store, klatch::member::CliClock, klatch::gateway::AllowAll);
    let spec = klatch::gateway::CredentialSpec {
        member,
        clearance: "public",
        ttl: 3600,
        max_life: None,
        parent: None,
    };
    let who = klatch::domain::Principal::unattributed("launcher");
    gateway
        .admin_issue_credential(&who, &spec)
        .unwrap()
        .to_text()
        .to_string()
}

/// A way to reach a server: plain TCP (the loopback listener) or TLS with a client certificate.
#[derive(Clone)]
pub struct Net {
    pub addr: SocketAddr,
    pub tls: Option<Arc<rustls::ClientConfig>>,
}

pub enum Conn {
    Plain(TcpStream),
    Tls(Box<Tls>),
}

impl Read for Conn {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Conn::Plain(s) => s.read(b),
            Conn::Tls(s) => s.read(b),
        }
    }
}

impl Write for Conn {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        match self {
            Conn::Plain(s) => s.write(b),
            Conn::Tls(s) => s.write(b),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Conn::Plain(s) => s.flush(),
            Conn::Tls(s) => s.flush(),
        }
    }
}

impl Conn {
    pub fn set_read_timeout(&self, d: Duration) {
        let sock = match self {
            Conn::Plain(s) => s,
            Conn::Tls(s) => &s.sock,
        };
        sock.set_read_timeout(Some(d)).unwrap();
    }
}

impl Net {
    pub fn connect(&self) -> Conn {
        let conn = match &self.tls {
            None => Conn::Plain(TcpStream::connect(self.addr).expect("connect")),
            Some(c) => Conn::Tls(Box::new(tls_connect(self.addr, c).expect("handshake"))),
        };
        conn.set_read_timeout(Duration::from_secs(20));
        conn
    }

    pub fn request(
        &self,
        (method, path): (&str, &str),
        headers: &[(&str, &str)],
        body: &[u8],
    ) -> Conn {
        let mut c = self.connect();
        let mut head = format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\n");
        for (n, v) in headers {
            head.push_str(&format!("{n}: {v}\r\n"));
        }
        if method == "POST" {
            head.push_str(&format!("Content-Length: {}\r\n", body.len()));
        }
        head.push_str("\r\n");
        c.write_all(head.as_bytes()).unwrap();
        c.write_all(body).unwrap();
        c.flush().unwrap();
        c
    }
}

pub fn read_head(r: &mut BufReader<Conn>) -> (u16, Vec<(String, String)>) {
    let mut line = String::new();
    r.read_line(&mut line).expect("a status line");
    let status = line
        .split(' ')
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| panic!("status line {line:?}"));
    let mut headers = Vec::new();
    loop {
        let mut h = String::new();
        r.read_line(&mut h).unwrap();
        let h = h.trim_end();
        if h.is_empty() {
            return (status, headers);
        }
        let (n, v) = h.split_once(':').unwrap();
        headers.push((n.to_ascii_lowercase(), v.trim().to_string()));
    }
}

pub fn status_of(net: &Net, target: (&str, &str), headers: &[(&str, &str)], body: &[u8]) -> u16 {
    read_head(&mut BufReader::new(net.request(target, headers, body))).0
}

/// One server-sent-events session of a member.
pub struct Sse {
    pub net: Net,
    pub member: String,
    pub bearer: String,
    pub session: String,
    pub r: BufReader<Conn>,
    pub extra: Vec<Value>,
    pub next: u32,
}

impl Sse {
    pub fn open(net: &Net, member: &str, token: &str) -> Sse {
        let bearer = format!("Bearer {token}");
        let c = net.request(
            ("GET", "/v1/stream"),
            &[("X-Klatch-Member", member), ("Authorization", &bearer)],
            b"",
        );
        let mut r = BufReader::new(c);
        let (status, headers) = read_head(&mut r);
        assert_eq!(status, 200, "{headers:?}");
        let session = headers
            .iter()
            .find(|(h, _)| h == "x-klatch-session")
            .map(|(_, v)| v.clone())
            .unwrap();
        Sse {
            net: net.clone(),
            member: member.into(),
            bearer,
            session,
            r,
            extra: Vec::new(),
            next: 0,
        }
    }

    /// The next frame (the `data:` line of the next event); `None` when the stream ended or was quiet for the timeout.
    pub fn recv(&mut self) -> Option<Value> {
        loop {
            let mut line = String::new();
            match self.r.read_line(&mut line) {
                Ok(0) => return None,
                Ok(_) => {}
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock
                            | std::io::ErrorKind::TimedOut
                            | std::io::ErrorKind::ConnectionReset
                            | std::io::ErrorKind::UnexpectedEof
                    ) =>
                {
                    return None;
                }
                Err(e) => panic!("read: {e}"),
            }
            if let Some(v) = line.trim_end().strip_prefix("data: ") {
                return Some(serde_json::from_str(v).unwrap());
            }
        }
    }

    pub fn post_frame(&self, frame: &Value) -> u16 {
        let mut body = frame.to_string().into_bytes();
        body.push(b'\n');
        let h = [
            ("X-Klatch-Member", self.member.as_str()),
            ("Authorization", self.bearer.as_str()),
            ("X-Klatch-Session", self.session.as_str()),
        ];
        status_of(&self.net, ("POST", "/v1/frames"), &h, &body)
    }

    pub fn request(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        let id = format!("r{}", self.next);
        assert_eq!(
            self.post_frame(
                &json!({"v":1,"type":"request","id":id,"method":method,"params":params})
            ),
            202
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

/// A TLS server with a valid set-up, and a client config that has a certificate from its CA.
pub struct TlsRig {
    pub dir: Dir,
    pub server: Server,
    pub net: Net,
    pub tokens: [String; 2],
}

pub fn tls_server() -> TlsRig {
    tls_server_with(&[])
}

/// [`tls_server`] with more options for the server (limits, deadlines).
pub fn tls_server_with(extra: &[&str]) -> TlsRig {
    let dir = Dir::new();
    let (server_ca, client_ca) = (Ca::new("server ca"), Ca::new("client ca"));
    let files = server_files(
        &dir,
        &server_ca.issue("klatch", Use::Server),
        &client_ca,
        0o600,
    );
    let tokens = [
        issue_credential(&dir, "alice"),
        issue_credential(&dir, "bob"),
    ];
    let mut args = tls_args(&files);
    args.extend(extra.iter().map(|s| s.to_string()));
    let server = Server::started(&dir, "https", &args);
    let leaf = client_ca.issue("worker", Use::Client);
    let net = Net {
        addr: server.addr,
        tls: Some(client_config(&server_ca, Some(&leaf), None)),
    };
    TlsRig {
        dir,
        server,
        net,
        tokens,
    }
}
