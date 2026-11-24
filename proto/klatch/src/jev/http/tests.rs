//! The real transport against a loopback server written in this file (plain sockets, 127.0.0.1 only, nothing leaves
//! the machine). The configuration is asserted never to loosen TLS (see
//! `the_agent_is_configured_as_strictly_as_documented`), and the last section of this file proves the behaviour: a
//! loopback TLS server with certificates made at test time is refused when its certificate is from an unknown CA,
//! expired, or for another name, and accepted when it chains to a root the transport was told to trust (#927).

use super::*;
use crate::jev::fake::fixtures;
use crate::jev::retry::{Clock, NoJitter, RetryPolicy, Sleeper};
use crate::jev::transport::FilePart;
use crate::jev::{JevClient, JevConfig, JevError, Question};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const KEY: &str = "sk-live-SECRET-1234";

/// What a loopback server does with one connection.
enum Reply {
    /// Write these bytes, then close.
    Bytes(Vec<u8>),
    /// Accept and say nothing for a while (a stalled server).
    Hang,
}

fn http(status: &str, headers: &[&str], body: &str) -> Reply {
    let mut s = format!("HTTP/1.1 {status}\r\nConnection: close\r\n");
    for h in headers {
        s += h;
        s += "\r\n";
    }
    if !headers
        .iter()
        .any(|h| h.to_ascii_lowercase().starts_with("transfer-encoding"))
    {
        s += &format!("Content-Length: {}\r\n", body.len());
    }
    s += "\r\n";
    s += body;
    Reply::Bytes(s.into_bytes())
}

struct Server {
    port: u16,
    seen: Arc<Mutex<Vec<Vec<u8>>>>,
    count: Arc<AtomicUsize>,
}

impl Server {
    /// `script(n)` answers the n-th connection (from 0).
    fn start(script: impl Fn(usize) -> Reply + Send + 'static) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen: Arc<Mutex<Vec<Vec<u8>>>> = Arc::default();
        let count = Arc::new(AtomicUsize::new(0));
        let (s2, c2) = (seen.clone(), count.clone());
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                let Ok(mut conn) = conn else { return };
                let n = c2.fetch_add(1, Ordering::SeqCst);
                let raw = read_request(&mut conn);
                s2.lock().unwrap().push(raw);
                match script(n) {
                    Reply::Bytes(b) => {
                        let _ = conn.write_all(&b);
                        let _ = conn.flush();
                    }
                    Reply::Hang => std::thread::sleep(Duration::from_secs(5)),
                }
            }
        });
        Server { port, seen, count }
    }

    fn connections(&self) -> usize {
        self.count.load(Ordering::SeqCst)
    }

    fn request(&self, n: usize) -> Vec<u8> {
        self.seen.lock().unwrap()[n].clone()
    }

    fn head(&self, n: usize) -> String {
        let raw = self.request(n);
        let end = raw
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .unwrap_or(raw.len());
        String::from_utf8_lossy(&raw[..end]).to_string()
    }
}

fn read_request(conn: &mut TcpStream) -> Vec<u8> {
    conn.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let mut buf = vec![];
    let mut chunk = [0u8; 4096];
    while let Ok(n) = conn.read(&mut chunk) {
        if n == 0 {
            break;
        }
        buf.extend(&chunk[..n]);
        if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..end]).to_ascii_lowercase();
            let want = head
                .lines()
                .find_map(|l| l.strip_prefix("content-length:"))
                .and_then(|v| v.trim().parse::<usize>().ok())
                .unwrap_or(0);
            if buf.len() >= end + 4 + want {
                break;
            }
        }
    }
    buf
}

fn opts() -> HttpOptions {
    HttpOptions {
        connect_timeout: Duration::from_secs(2),
        total_timeout: Duration::from_secs(3),
        max_reply_bytes: 64 * 1024,
    }
}

fn loopback(server: &Server) -> HttpTransport {
    HttpTransport::loopback_for_tests(server.port, false, opts())
}

fn key() -> ApiKey {
    ApiKey::new(KEY.to_string())
}

fn status_of(e: JevTransportError) -> (u16, String, RetryHeaders) {
    match e {
        JevTransportError::Status {
            status,
            body,
            headers,
        } => (status, body, headers),
        other => panic!("expected a status, got {other:?}"),
    }
}

#[test]
fn the_target_is_https_and_the_one_host_and_nothing_else() {
    assert!(Target::parse("https://api.typesafe.ai").is_ok());
    assert!(Target::parse("https://api.typesafe.ai/").is_ok());
    assert!(Target::parse("https://API.TYPESAFE.AI").is_ok());
    assert_eq!(Target::production().base(), crate::jev::transport::BASE_URL);
    for (url, why) in [
        ("http://api.typesafe.ai", TargetError::NotHttps),
        ("ftp://api.typesafe.ai", TargetError::NotHttps),
        ("api.typesafe.ai", TargetError::NotHttps),
        ("https://evil.example", TargetError::WrongHost),
        (
            "https://api.typesafe.ai.evil.example",
            TargetError::WrongHost,
        ),
        (
            "https://evil.example/api.typesafe.ai",
            TargetError::WrongHost,
        ),
        (
            "https://api.typesafe.ai@evil.example",
            TargetError::WrongHost,
        ),
        (
            "https://evil.example@api.typesafe.ai",
            TargetError::Userinfo,
        ),
        ("https://api.typesafe.ai:8443", TargetError::Port),
        ("https://api.typesafe.ai:443", TargetError::Port),
        ("https://api.typesafe.ai/v1", TargetError::Path),
        ("https://api.typesafe.ai?x=1", TargetError::Path),
        ("https://api.typesafe.ai#f", TargetError::Path),
        ("https://api.typesafe.ai.", TargetError::WrongHost),
        ("https://127.0.0.1", TargetError::WrongHost),
        ("https://", TargetError::WrongHost),
    ] {
        assert_eq!(Target::parse(url).unwrap_err(), why, "{url}");
    }
    assert!(HttpTransport::with_base("http://api.typesafe.ai", opts()).is_err());
    assert!(HttpTransport::with_base("https://evil.example", opts()).is_err());
}

#[test]
fn the_agent_is_configured_as_strictly_as_documented() {
    let c = agent_config(opts(), true);
    assert!(c.https_only());
    assert_eq!(c.max_redirects(), 0);
    assert!(
        c.proxy().is_none(),
        "no proxy, whatever the environment says"
    );
    assert!(
        !c.http_status_as_error(),
        "a status is read, not turned into a bare error"
    );
    assert!(!c.tls_config().disable_verification());
    assert_eq!(c.timeouts().global, Some(opts().total_timeout));
    assert_eq!(c.timeouts().connect, Some(opts().connect_timeout));
    // the shipped transport is https only; only the test constructor relaxes it
    assert!(
        HttpTransport::with_base("https://api.typesafe.ai", opts())
            .unwrap()
            .https_only()
    );
    assert!(!agent_config(opts(), false).https_only());
}

#[test]
fn defaults_are_a_hard_connect_and_total_timeout_and_a_bounded_reply() {
    let d = HttpOptions::default();
    assert!(d.connect_timeout > Duration::ZERO && d.connect_timeout <= Duration::from_secs(30));
    assert!(d.total_timeout >= d.connect_timeout && d.total_timeout <= Duration::from_secs(120));
    assert!(d.max_reply_bytes >= 64 * 1024 && d.max_reply_bytes <= 32 * 1024 * 1024);
}

#[test]
fn http_is_refused_before_any_connection_is_made() {
    let s = Server::start(|_| http("200 OK", &[], "{}"));
    let t = HttpTransport::loopback_for_tests(s.port, true, opts());
    assert!(matches!(
        t.send(&key(), &JevRequest::models()),
        Err(JevTransportError::Network)
    ));
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        s.connections(),
        0,
        "the key must not cross a plain connection"
    );
}

#[test]
fn a_get_sends_the_key_only_in_the_authorization_header() {
    let s = Server::start(|_| http("200 OK", &[], fixtures::LIVE_MODELS));
    let body = loopback(&s).send(&key(), &JevRequest::models()).unwrap();
    assert_eq!(body, fixtures::LIVE_MODELS);
    let head = s.head(0);
    assert!(head.starts_with("GET /v1/models HTTP/1.1"), "{head}");
    assert!(
        head.contains(&format!("authorization: Bearer {KEY}"))
            || head.contains(&format!("Authorization: Bearer {KEY}"))
    );
    assert_eq!(
        head.matches(KEY).count(),
        1,
        "the key is in no other header or the path"
    );
    assert!(!head.to_ascii_lowercase().contains("content-type"));
    assert!(!head.to_ascii_lowercase().contains("proxy"));
    assert!(
        !head.to_ascii_lowercase().contains("accept-encoding"),
        "no compression is negotiated"
    );
}

#[test]
fn a_json_post_carries_the_body_the_content_type_and_the_images_header_only_when_asked() {
    let s = Server::start(|_| http("200 OK", &[], "{}"));
    let r = JevRequest {
        method: Method::Post,
        path: "/v1/systemone",
        images: false,
        body: Some(r#"{"model":"m"}"#.into()),
        files: vec![],
    };
    loopback(&s).send(&key(), &r).unwrap();
    let head = s.head(0).to_ascii_lowercase();
    assert!(head.starts_with("post /v1/systemone"));
    assert!(head.contains("content-type: application/json"));
    assert!(!head.contains("typesafe-danger-zone-images"));
    assert!(String::from_utf8_lossy(&s.request(0)).ends_with(r#"{"model":"m"}"#));
    let r = JevRequest { images: true, ..r };
    loopback(&s).send(&key(), &r).unwrap();
    assert!(
        s.head(1)
            .to_ascii_lowercase()
            .contains("typesafe-danger-zone-images: true")
    );
}

#[test]
fn an_image_request_goes_as_multipart_with_the_file_as_it_is() {
    let s = Server::start(|_| http("200 OK", &[], "{}"));
    let bytes = vec![0x89, b'P', b'N', b'G', 0, 1, 2, 3, 255];
    let r = JevRequest {
        method: Method::Post,
        path: "/v1/systemone",
        images: true,
        body: Some(r#"{"model":"m"}"#.into()),
        files: vec![FilePart::new(
            "image-0000".into(),
            "image/png",
            bytes.clone(),
        )],
    };
    loopback(&s).send(&key(), &r).unwrap();
    let head = s.head(0).to_ascii_lowercase();
    assert!(
        head.contains("content-type: multipart/form-data; boundary=klatch-jev-boundary-"),
        "{head}"
    );
    assert!(head.contains("typesafe-danger-zone-images: true"));
    let raw = s.request(0);
    assert!(
        raw.windows(bytes.len()).any(|w| w == bytes.as_slice()),
        "raw bytes, not base64"
    );
    let text = String::from_utf8_lossy(&raw);
    assert!(text.contains("name=\"request\"") && text.contains("name=\"image-0000\""));
    assert!(!text.contains("data_base64"));
}

#[test]
fn a_redirect_is_refused_and_never_followed() {
    let target = Server::start(|_| http("200 OK", &[], "{}"));
    let url = format!("Location: http://127.0.0.1:{}/v1/models", target.port);
    let s = Server::start(move |_| http("302 Found", &[&url], ""));
    for code in [
        "301 Moved",
        "302 Found",
        "303 See Other",
        "307 Temporary",
        "308 Permanent",
    ] {
        let url = format!("Location: http://127.0.0.1:{}/v1/models", target.port);
        let c = code.to_string();
        let s2 = Server::start(move |_| http(&c, &[&url], ""));
        let e = loopback(&s2)
            .send(&key(), &JevRequest::models())
            .unwrap_err();
        assert_eq!(status_of(e).0, code[..3].parse::<u16>().unwrap());
        assert_eq!(s2.connections(), 1);
    }
    let _ = s;
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        target.connections(),
        0,
        "the redirect target was never contacted, so the key never went there"
    );
}

#[test]
fn a_stalled_server_ends_at_the_total_timeout() {
    let s = Server::start(|_| Reply::Hang);
    let t = HttpTransport::loopback_for_tests(
        s.port,
        false,
        HttpOptions {
            total_timeout: Duration::from_millis(400),
            ..opts()
        },
    );
    let at = Instant::now();
    assert!(matches!(
        t.send(&key(), &JevRequest::models()),
        Err(JevTransportError::Network)
    ));
    assert!(at.elapsed() < Duration::from_secs(3), "{:?}", at.elapsed());
}

#[test]
fn a_closed_port_is_a_network_error_with_no_detail() {
    let port = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let t = HttpTransport::loopback_for_tests(port, false, opts());
    let e = t.send(&key(), &JevRequest::models()).unwrap_err();
    assert_eq!(e, JevTransportError::Network);
}

#[test]
fn an_oversized_reply_is_refused_by_length_and_when_chunked() {
    let o = HttpOptions {
        max_reply_bytes: 1024,
        ..opts()
    };
    let big = "x".repeat(5000);
    let s = Server::start({
        let big = big.clone();
        move |_| http("200 OK", &[], &big)
    });
    let t = HttpTransport::loopback_for_tests(s.port, false, o);
    assert_eq!(
        t.send(&key(), &JevRequest::models()).unwrap_err(),
        JevTransportError::Network
    );
    // chunked with no length: the bound still holds
    let chunked = {
        let mut b = String::new();
        for _ in 0..50 {
            b += &format!("{:x}\r\n{}\r\n", 100, "y".repeat(100));
        }
        b += "0\r\n\r\n";
        b
    };
    let s = Server::start(move |_| http("200 OK", &["Transfer-Encoding: chunked"], &chunked));
    let t = HttpTransport::loopback_for_tests(s.port, false, o);
    assert_eq!(
        t.send(&key(), &JevRequest::models()).unwrap_err(),
        JevTransportError::Network
    );
    // a reply at the bound is fine
    let ok = "z".repeat(1024);
    let s = Server::start(move |_| http("200 OK", &[], &ok));
    let t = HttpTransport::loopback_for_tests(s.port, false, o);
    assert_eq!(t.send(&key(), &JevRequest::models()).unwrap().len(), 1024);
}

#[test]
fn an_error_body_is_bounded_and_only_read_for_400_401_403() {
    let echo = format!(r#"{{"detail":[{{"input":{{"state":"{KEY} and the message"}}}}]}}"#);
    let s = Server::start(move |_| http("422 Unprocessable Entity", &[], &echo));
    let (status, body, _) = status_of(
        loopback(&s)
            .send(&key(), &JevRequest::models())
            .unwrap_err(),
    );
    assert_eq!(status, 422);
    assert_eq!(
        body, "",
        "a 422 body echoes the request, so it is dropped here"
    );
    let s = Server::start(|_| http("500 Internal Server Error", &[], "internal trace"));
    assert_eq!(
        status_of(
            loopback(&s)
                .send(&key(), &JevRequest::models())
                .unwrap_err()
        )
        .1,
        ""
    );
    let s = Server::start(|_| http("401 Unauthorized", &[], fixtures::ERR_AUTH));
    let (st, body, _) = status_of(
        loopback(&s)
            .send(&key(), &JevRequest::models())
            .unwrap_err(),
    );
    assert_eq!(st, 401);
    assert_eq!(body, fixtures::ERR_AUTH);
    // a 400 body over the bound is not kept at all
    let huge = format!(
        r#"{{"detail":{{"error_type":"api_usage_error","m":"{}"}}}}"#,
        "a".repeat(200_000)
    );
    let s = Server::start(move |_| http("400 Bad Request", &[], &huge));
    let (st, body, _) = status_of(
        loopback(&s)
            .send(&key(), &JevRequest::models())
            .unwrap_err(),
    );
    assert_eq!(st, 400);
    assert!(body.len() <= opts().max_reply_bytes);
}

#[test]
fn the_retry_headers_come_through() {
    let s = Server::start(|_| {
        http(
            "429 Too Many Requests",
            &["Retry-After: 3", "retry-after-ms: 250"],
            "{}",
        )
    });
    let (st, _, h) = status_of(
        loopback(&s)
            .send(&key(), &JevRequest::models())
            .unwrap_err(),
    );
    assert_eq!(st, 429);
    assert_eq!(h.retry_after.as_deref(), Some("3"));
    assert_eq!(h.retry_after_ms.as_deref(), Some("250"));
}

#[test]
fn a_content_encoded_reply_is_refused_because_none_was_asked_for() {
    let s = Server::start(|_| http("200 OK", &["Content-Encoding: gzip"], "not really gzip"));
    assert_eq!(
        loopback(&s)
            .send(&key(), &JevRequest::models())
            .unwrap_err(),
        JevTransportError::Network
    );
}

#[test]
fn a_key_that_could_split_a_header_is_refused_and_nothing_is_sent() {
    let s = Server::start(|_| http("200 OK", &[], "{}"));
    for bad in ["abc\r\nX-Evil: 1", "abc\ndef", "abc\u{0}def"] {
        let k = ApiKey::new(bad.to_string());
        assert_eq!(
            loopback(&s).send(&k, &JevRequest::models()).unwrap_err(),
            JevTransportError::Network
        );
    }
    assert_eq!(s.connections(), 0);
}

struct Instant0;
impl Clock for Instant0 {
    fn now(&self) -> Duration {
        Duration::ZERO
    }
}
struct Counting(Arc<Mutex<Vec<Duration>>>);
impl Sleeper for Counting {
    fn sleep(&self, d: Duration) {
        self.0.lock().unwrap().push(d);
    }
}

#[test]
fn a_429_then_a_200_succeeds_through_the_retry_policy() {
    let ok = fixtures::LIVE_NOUL.to_string();
    let s = Server::start(move |n| {
        if n == 0 {
            http("429 Too Many Requests", &["retry-after-ms: 20"], "{}")
        } else {
            http("200 OK", &[], &ok)
        }
    });
    let slept = Arc::new(Mutex::new(vec![]));
    let client = JevClient::new(loopback(&s), JevConfig::new("jev-1.14.0", 0.8).unwrap())
        .with_key_source(Box::new(|| Some(KEY.to_string())))
        .with_gate(crate::jev::Gate::new(Default::default(), Some("internal")))
        .with_retry(
            RetryPolicy::default(),
            Box::new(Instant0),
            Box::new(Counting(slept.clone())),
            Box::new(NoJitter),
        );
    let q = Question::noul("billing", "is it about billing?");
    let a = client
        .ask(&crate::jev::Entry::text("public", "hello"), &[q])
        .unwrap();
    assert!(a.noul("billing").is_some());
    assert_eq!(s.connections(), 2);
    assert_eq!(*slept.lock().unwrap(), vec![Duration::from_millis(20)]);
}

#[test]
fn a_401_is_not_retried_and_a_persistent_429_stops_at_the_policy() {
    let s = Server::start(|_| http("401 Unauthorized", &[], fixtures::ERR_AUTH));
    let slept = Arc::new(Mutex::new(vec![]));
    let mk = |s: &Server| {
        JevClient::new(loopback(s), JevConfig::new("jev-1.14.0", 0.8).unwrap())
            .with_key_source(Box::new(|| Some(KEY.to_string())))
            .with_retry(
                RetryPolicy::default(),
                Box::new(Instant0),
                Box::new(Counting(slept.clone())),
                Box::new(NoJitter),
            )
    };
    assert!(matches!(
        mk(&s).list_models(),
        Err(JevError::Unauthorized { .. })
    ));
    assert_eq!(s.connections(), 1);
    let s = Server::start(|_| http("429 Too Many Requests", &["retry-after-ms: 1"], "{}"));
    assert!(matches!(mk(&s).list_models(), Err(JevError::RateLimited)));
    assert_eq!(s.connections(), 3, "one try and two retries");
}

#[test]
fn the_key_is_in_no_error_display_or_debug() {
    let mut texts = vec![];
    let echo = format!(r#"{{"detail":[{{"input":"{KEY}"}}]}}"#);
    for reply in [
        http(
            "401 Unauthorized",
            &[],
            &format!(
                r#"{{"detail":{{"error_type":"authentication_error","message":"bad {KEY}"}}}}"#
            ),
        ),
        http("422 Unprocessable Entity", &[], &echo),
        http("500 Oops", &[], KEY),
    ] {
        let cell = Mutex::new(Some(reply));
        let s = Server::start(move |_| cell.lock().unwrap().take().unwrap_or(Reply::Hang));
        let e = loopback(&s)
            .send(&key(), &JevRequest::models())
            .unwrap_err();
        texts.push(format!("{e:?}"));
        let client = JevClient::new(loopback(&s), JevConfig::new("jev-1.14.0", 0.8).unwrap())
            .with_key_source(Box::new(|| Some(KEY.to_string())));
        if let Err(e) = client.list_models() {
            texts.push(format!("{e:?} {e}"));
        }
    }
    let closed = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let e = HttpTransport::loopback_for_tests(closed, false, opts())
        .send(&key(), &JevRequest::models())
        .unwrap_err();
    texts.push(format!("{e:?}"));
    texts.push(format!(
        "{:?}",
        HttpTransport::with_base("https://api.typesafe.ai", opts()).unwrap()
    ));
    texts.push(format!("{:?}", key()));
    for t in texts {
        assert!(!t.contains(KEY), "the key leaked: {t}");
    }
}

#[test]
fn the_real_key_comes_from_the_environment_through_the_client_at_call_time() {
    // with no key in the lookup the client refuses before the transport is touched
    let s = Server::start(|_| http("200 OK", &[], "{}"));
    let client = JevClient::new(loopback(&s), JevConfig::new("jev-1.14.0", 0.8).unwrap())
        .with_key_source(Box::new(|| None));
    assert!(matches!(client.list_models(), Err(JevError::NoKey)));
    assert_eq!(s.connections(), 0);
}

// ---- TLS verification against a loopback TLS server (#927) ----

use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{ServerConfig, ServerConnection};
use std::net::IpAddr;
use std::sync::atomic::AtomicBool;
use ureq::tls::{Certificate, RootCerts};

/// A time `secs` from now (negative: in the past), from the system clock. Never a fixed date.
fn from_now(secs: i64) -> time::OffsetDateTime {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    time::OffsetDateTime::from_unix_timestamp(now + secs).unwrap()
}

const DAY: i64 = 24 * 60 * 60;

/// A throwaway certificate authority, valid from yesterday to tomorrow.
struct TestCa {
    params: rcgen::CertificateParams,
    key: rcgen::KeyPair,
    cert: rcgen::Certificate,
}

impl TestCa {
    fn new(name: &str) -> TestCa {
        let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, name);
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        params.key_usages = vec![
            rcgen::KeyUsagePurpose::KeyCertSign,
            rcgen::KeyUsagePurpose::CrlSign,
        ];
        params.not_before = from_now(-DAY);
        params.not_after = from_now(DAY);
        let key = rcgen::KeyPair::generate().unwrap();
        let cert = params.self_signed(&key).unwrap();
        TestCa { params, key, cert }
    }

    /// The root the transport is told to trust.
    fn root(&self) -> RootCerts {
        RootCerts::new_with_certs(&[Certificate::from_der(self.cert.der().as_ref()).to_owned()])
    }

    /// A server certificate for these names (an IP address is an address name) with this validity, relative to now.
    fn leaf(&self, names: &[&str], not_before: i64, not_after: i64) -> Leaf {
        let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
        params.subject_alt_names = names
            .iter()
            .map(|n| match n.parse::<IpAddr>() {
                Ok(ip) => rcgen::SanType::IpAddress(ip),
                Err(_) => rcgen::SanType::DnsName((*n).try_into().unwrap()),
            })
            .collect();
        params.is_ca = rcgen::IsCa::NoCa;
        params.not_before = from_now(not_before);
        params.not_after = from_now(not_after);
        let key = rcgen::KeyPair::generate().unwrap();
        let issuer = rcgen::Issuer::new(self.params.clone(), &self.key);
        let cert = params.signed_by(&key, &issuer).unwrap();
        Leaf {
            cert: cert.der().clone(),
            key: key.serialize_der(),
        }
    }
}

struct Leaf {
    cert: CertificateDer<'static>,
    key: Vec<u8>,
}

/// What a TLS test server saw, read after it has stopped.
#[derive(Debug, PartialEq, Eq)]
struct TlsSeen {
    handshakes: usize,
    requests: usize,
    head: String,
}

/// A TLS server on 127.0.0.1, on a free port, that answers `200` with `{}` to each request it receives. It serves one
/// connection at a time, every socket has a timeout, and the thread is stopped and joined when [`TlsServer::finish`]
/// runs or the value is dropped (so no thread or socket outlives the test, on any path, a panic included).
struct TlsServer {
    port: u16,
    stop: Arc<AtomicBool>,
    seen: Arc<Mutex<TlsSeen>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// The longest the server thread lives if nothing stops it, and the longest it waits on one connection.
const TLS_SERVER_LIFETIME: Duration = Duration::from_secs(30);
const TLS_CONN_TIMEOUT: Duration = Duration::from_secs(3);

impl TlsServer {
    fn start(leaf: &Leaf) -> TlsServer {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![leaf.cert.clone()],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf.key.clone())),
            )
            .unwrap();
        let config = Arc::new(config);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let stop = Arc::new(AtomicBool::new(false));
        let seen = Arc::new(Mutex::new(TlsSeen {
            handshakes: 0,
            requests: 0,
            head: String::new(),
        }));
        let (stop2, seen2) = (stop.clone(), seen.clone());
        let thread = std::thread::spawn(move || {
            let deadline = Instant::now() + TLS_SERVER_LIFETIME;
            while !stop2.load(Ordering::SeqCst) && Instant::now() < deadline {
                match listener.accept() {
                    Ok((conn, _)) => serve_tls(conn, &config, &seen2),
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => return,
                }
            }
        });
        TlsServer {
            port,
            stop,
            seen,
            thread: Some(thread),
        }
    }

    /// Stop the thread, wait for it, and report what it saw.
    fn finish(mut self) -> TlsSeen {
        self.shut_down();
        let mut seen = self.seen.lock().unwrap();
        TlsSeen {
            handshakes: seen.handshakes,
            requests: seen.requests,
            head: std::mem::take(&mut seen.head),
        }
    }

    fn shut_down(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for TlsServer {
    fn drop(&mut self) {
        self.shut_down();
    }
}

/// One connection: the handshake, then one request and one reply. A failed handshake (the client refused the
/// certificate and said so) ends the connection with nothing counted as a request.
fn serve_tls(mut sock: TcpStream, config: &Arc<ServerConfig>, seen: &Arc<Mutex<TlsSeen>>) {
    if !bound_socket(&sock) {
        return;
    }
    let Ok(mut conn) = ServerConnection::new(config.clone()) else {
        return;
    };
    while conn.is_handshaking() {
        if conn.complete_io(&mut sock).is_err() {
            return;
        }
    }
    seen.lock().unwrap().handshakes += 1;
    let mut tls = rustls::Stream::new(&mut conn, &mut sock);
    let Some(head) = read_tls_head(&mut tls) else {
        return;
    };
    {
        let mut s = seen.lock().unwrap();
        s.requests += 1;
        s.head = head;
    }
    let reply = "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 2\r\n\r\n{}";
    let _ = tls.write_all(reply.as_bytes());
    let _ = tls.flush();
    tls.conn.send_close_notify();
    let _ = tls.conn.complete_io(tls.sock);
}

/// Blocking mode (an accepted socket may inherit the listener's non-blocking mode) and a timeout both ways.
fn bound_socket(sock: &TcpStream) -> bool {
    sock.set_nonblocking(false).is_ok()
        && sock.set_read_timeout(Some(TLS_CONN_TIMEOUT)).is_ok()
        && sock.set_write_timeout(Some(TLS_CONN_TIMEOUT)).is_ok()
}

/// The request head (up to the blank line, at most 64 KiB), or `None` when the client went away first.
fn read_tls_head(tls: &mut rustls::Stream<'_, ServerConnection, TcpStream>) -> Option<String> {
    let mut buf = vec![];
    let mut chunk = [0u8; 4096];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") && buf.len() < 64 * 1024 {
        match tls.read(&mut chunk) {
            Ok(0) | Err(_) => return None,
            Ok(n) => buf.extend(&chunk[..n]),
        }
    }
    Some(String::from_utf8_lossy(&buf).to_string())
}

/// Connect a transport that trusts `roots` to the server and make one call.
fn call_tls(server: &TlsServer, roots: RootCerts) -> Result<String, JevTransportError> {
    HttpTransport::tls_loopback_for_tests(server.port, roots, opts())
        .send(&key(), &JevRequest::models())
}

#[test]
fn a_certificate_from_a_ca_it_is_told_to_trust_is_accepted() {
    let ca = TestCa::new("klatch test CA");
    let server = TlsServer::start(&ca.leaf(&["127.0.0.1"], -DAY, DAY));
    let reply = call_tls(&server, ca.root());
    let seen = server.finish();
    assert_eq!(reply.as_deref(), Ok("{}"));
    assert_eq!((seen.handshakes, seen.requests), (1, 1));
    assert!(
        seen.head
            .to_ascii_lowercase()
            .contains(&format!("authorization: bearer {KEY}").to_ascii_lowercase()),
        "the key goes over the verified connection"
    );
}

#[test]
fn a_certificate_from_an_unknown_ca_is_refused_and_no_request_is_sent() {
    let (trusted, unknown) = (TestCa::new("trusted CA"), TestCa::new("unknown CA"));
    let server = TlsServer::start(&unknown.leaf(&["127.0.0.1"], -DAY, DAY));
    let reply = call_tls(&server, trusted.root());
    let seen = server.finish();
    assert_eq!(reply, Err(JevTransportError::Network));
    assert_eq!(
        (seen.handshakes, seen.requests),
        (0, 0),
        "the key was not sent"
    );
}

#[test]
fn a_certificate_that_is_not_a_ca_and_is_not_trusted_is_refused() {
    // the usual self signed server: its own certificate is the only one, and nothing trusts it
    let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    params.subject_alt_names = vec![rcgen::SanType::IpAddress("127.0.0.1".parse().unwrap())];
    params.not_before = from_now(-DAY);
    params.not_after = from_now(DAY);
    let key = rcgen::KeyPair::generate().unwrap();
    let cert = params.self_signed(&key).unwrap();
    let server = TlsServer::start(&Leaf {
        cert: cert.der().clone(),
        key: key.serialize_der(),
    });
    let reply = call_tls(&server, TestCa::new("some other CA").root());
    let seen = server.finish();
    assert_eq!(reply, Err(JevTransportError::Network));
    assert_eq!(seen.requests, 0);
}

#[test]
fn the_shipped_trust_anchors_refuse_a_private_ca() {
    // the production transport trusts the bundled web PKI roots only, so a certificate from a CA made here is refused
    // even though it is valid and for the right address
    let ca = TestCa::new("private CA");
    let server = TlsServer::start(&ca.leaf(&["127.0.0.1"], -DAY, DAY));
    let reply = call_tls(&server, RootCerts::WebPki);
    let seen = server.finish();
    assert_eq!(reply, Err(JevTransportError::Network));
    assert_eq!((seen.handshakes, seen.requests), (0, 0));
}

#[test]
fn an_expired_certificate_from_a_trusted_ca_is_refused() {
    let ca = TestCa::new("klatch test CA");
    // expired by construction: it ended a day before now and began two days before now
    let server = TlsServer::start(&ca.leaf(&["127.0.0.1"], -2 * DAY, -DAY));
    let reply = call_tls(&server, ca.root());
    let seen = server.finish();
    assert_eq!(reply, Err(JevTransportError::Network));
    assert_eq!((seen.handshakes, seen.requests), (0, 0));
}

#[test]
fn a_certificate_that_is_not_yet_valid_is_refused() {
    let ca = TestCa::new("klatch test CA");
    let server = TlsServer::start(&ca.leaf(&["127.0.0.1"], DAY, 2 * DAY));
    let reply = call_tls(&server, ca.root());
    let seen = server.finish();
    assert_eq!(reply, Err(JevTransportError::Network));
    assert_eq!((seen.handshakes, seen.requests), (0, 0));
}

#[test]
fn a_certificate_for_another_host_name_is_refused() {
    let ca = TestCa::new("klatch test CA");
    // valid, from the trusted CA, and for a name that is not the one the transport connected to
    let server = TlsServer::start(&ca.leaf(&["not-this-host.invalid"], -DAY, DAY));
    let reply = call_tls(&server, ca.root());
    let seen = server.finish();
    assert_eq!(reply, Err(JevTransportError::Network));
    assert_eq!((seen.handshakes, seen.requests), (0, 0));
}

#[test]
fn a_certificate_for_another_address_is_refused() {
    let ca = TestCa::new("klatch test CA");
    let server = TlsServer::start(&ca.leaf(&["127.0.0.2"], -DAY, DAY));
    let reply = call_tls(&server, ca.root());
    let seen = server.finish();
    assert_eq!(reply, Err(JevTransportError::Network));
    assert_eq!((seen.handshakes, seen.requests), (0, 0));
}

#[test]
fn the_name_check_accepts_a_certificate_with_the_right_name_among_several() {
    // guards the refusals above against passing for the wrong reason (for example a broken server certificate)
    let ca = TestCa::new("klatch test CA");
    let server = TlsServer::start(&ca.leaf(&["other.invalid", "127.0.0.1"], -DAY, DAY));
    let reply = call_tls(&server, ca.root());
    assert_eq!(server.finish().requests, 1);
    assert_eq!(reply.as_deref(), Ok("{}"));
}
