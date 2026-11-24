//! The real HTTPS transport to Jev (#871), behind the `jev-http` feature (off by default, not part of `cli` or `client`).
//!
//! It is [ureq](https://docs.rs/ureq) 3 with rustls (ring) and the bundled web PKI roots, nothing else (no gzip, no
//! proxy support, no cookies, no platform verifier), configured so that it can only do what the design allows:
//!
//! - TLS with certificate verification on, always. There is no setting that turns it off.
//! - https only, and only to [`HOST`]: [`Target::parse`] refuses `http`, another host, a port, user info and a path.
//!   The URL of a call is that validated base plus one of the fixed paths of [`super::transport`].
//! - No redirect is ever followed (`max_redirects` is 0): a 3xx is returned as a status, and the key is never sent to
//!   the `Location`.
//! - No proxy, and the proxy environment variables (`HTTPS_PROXY`, `ALL_PROXY`, ...) are not read. There is no opt-in
//!   yet; a proxy needs its own design (it would see the key's destination and could break the host pin).
//! - A hard connect timeout and a hard total timeout (the whole call, body included), and a bound on the reply size.
//! - The key goes only in the `Authorization: Bearer` header, from the [`ApiKey`] handed to `send` (which the client
//!   reads from `TYPESAFE_API_KEY` at each call). A key that could split a header is refused. No error, Debug or
//!   Display of this module holds the key, a header, or a body: the only body kept is that of a 400, 401 or 403 (to read
//!   `error_type`, bounded), because a 422 body echoes the whole request.
//! - No compression is negotiated, so a reply with a `Content-Encoding` other than identity is refused. Request
//!   compression stays unbuilt (#916): it saves bandwidth only and needs a compressor.

use super::error::{JevTransportError, RetryHeaders};
use super::retry::{Jitter, RetryPolicy, SystemClock, ThreadSleeper};
use super::transport::{JevRequest, JevTransport, Method};
use super::{ApiKey, JevClient, JevConfig};
use std::collections::hash_map::RandomState;
use std::fmt;
use std::hash::{BuildHasher, Hasher};
use std::time::Duration;
use ureq::config::Config;
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

/// The one host the transport talks to.
pub const HOST: &str = "api.typesafe.ai";

/// Why a base URL was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetError {
    NotHttps,
    WrongHost,
    Userinfo,
    Port,
    Path,
}

impl fmt::Display for TargetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            TargetError::NotHttps => "the Jev URL must be https",
            TargetError::WrongHost => "the Jev URL must be on api.typesafe.ai",
            TargetError::Userinfo => "the Jev URL must carry no user info",
            TargetError::Port => "the Jev URL must carry no port",
            TargetError::Path => {
                "the Jev URL must be the bare host, with no path, query or fragment"
            }
        })
    }
}

impl std::error::Error for TargetError {}

/// A base URL that passed the pin: `https://api.typesafe.ai`, nothing more.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target(String);

impl Target {
    pub fn parse(url: &str) -> Result<Target, TargetError> {
        let rest = url
            .get(..8)
            .filter(|p| p.eq_ignore_ascii_case("https://"))
            .map(|_| &url[8..])
            .ok_or(TargetError::NotHttps)?;
        let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let (authority, tail) = rest.split_at(end);
        check_authority(authority)?;
        if !(tail.is_empty() || tail == "/") {
            return Err(TargetError::Path);
        }
        Ok(Target(format!("https://{HOST}")))
    }

    pub fn production() -> Target {
        Target(super::transport::BASE_URL.to_string())
    }

    pub fn base(&self) -> &str {
        &self.0
    }
}

/// The authority must be exactly the host: no userinfo, no port. A wrong host is named as such even then.
fn check_authority(authority: &str) -> Result<(), TargetError> {
    if authority.contains('@') {
        // `https://api.typesafe.ai@evil` has the host after the `@`; either way it is not the bare host
        let host = authority.rsplit('@').next();
        return Err(host_or_wrong(
            host.is_some_and(is_host),
            TargetError::Userinfo,
        ));
    }
    if let Some((host, _port)) = authority.split_once(':') {
        return Err(host_or_wrong(is_host(host), TargetError::Port));
    }
    if !is_host(authority) {
        return Err(TargetError::WrongHost);
    }
    Ok(())
}

/// The named fault when the host part is the pinned host, otherwise the wrong host.
fn host_or_wrong(host_ok: bool, fault: TargetError) -> TargetError {
    if host_ok {
        fault
    } else {
        TargetError::WrongHost
    }
}

fn is_host(h: &str) -> bool {
    h.eq_ignore_ascii_case(HOST)
}

/// The limits of one call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HttpOptions {
    /// The most time to make the TCP and TLS connection.
    pub connect_timeout: Duration,
    /// The most time for the whole call, reply body included.
    pub total_timeout: Duration,
    /// The most bytes of a reply body that are read; a longer reply is a network failure.
    pub max_reply_bytes: usize,
}

impl Default for HttpOptions {
    fn default() -> Self {
        HttpOptions {
            connect_timeout: Duration::from_secs(10),
            total_timeout: Duration::from_secs(60),
            // a reply is the answers and the usage; this is far above any real one (the request limit is 32 MiB)
            max_reply_bytes: 4 * 1024 * 1024,
        }
    }
}

/// The agent configuration. `https_only` is true in every shipped constructor; only a test turns it off, to reach a
/// loopback server.
fn agent_config(opts: HttpOptions, https_only: bool) -> Config {
    agent_config_with_roots(opts, https_only, RootCerts::WebPki)
}

/// The same configuration with the trust anchors named. Shipped code always passes the web PKI roots (through
/// [`agent_config`]); only a test passes another set, to prove that a certificate is checked against the anchors given.
fn agent_config_with_roots(opts: HttpOptions, https_only: bool, roots: RootCerts) -> Config {
    let tls = TlsConfig::builder()
        .provider(TlsProvider::Rustls)
        .root_certs(roots)
        .disable_verification(false)
        .build();
    Config::builder()
        .https_only(https_only)
        .max_redirects(0)
        .proxy(None)
        .http_status_as_error(false)
        .tls_config(tls)
        .timeout_connect(Some(opts.connect_timeout))
        .timeout_global(Some(opts.total_timeout))
        .user_agent("klatch-jev")
        .build()
}

pub struct HttpTransport {
    agent: ureq::Agent,
    base: String,
    opts: HttpOptions,
    https_only: bool,
}

impl fmt::Debug for HttpTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HttpTransport({})", self.base)
    }
}

impl HttpTransport {
    /// The production transport: `https://api.typesafe.ai`, default limits.
    pub fn new() -> HttpTransport {
        Self::build(Target::production().0, HttpOptions::default(), true)
    }

    pub fn with_options(opts: HttpOptions) -> HttpTransport {
        Self::build(Target::production().0, opts, true)
    }

    /// A transport for a base URL, which must pass [`Target::parse`] (so it can only be the one host over https).
    pub fn with_base(base: &str, opts: HttpOptions) -> Result<HttpTransport, TargetError> {
        Ok(Self::build(Target::parse(base)?.0, opts, true))
    }

    fn build(base: String, opts: HttpOptions, https_only: bool) -> HttpTransport {
        Self::build_with_config(base, opts, https_only, agent_config(opts, https_only))
    }

    fn build_with_config(
        base: String,
        opts: HttpOptions,
        https_only: bool,
        config: Config,
    ) -> HttpTransport {
        HttpTransport {
            agent: ureq::Agent::new_with_config(config),
            base,
            opts,
            https_only,
        }
    }

    #[cfg(test)]
    pub(crate) fn https_only(&self) -> bool {
        self.https_only
    }

    /// A test server on 127.0.0.1. `https_only` true shows that http is refused before a connection is made.
    #[cfg(test)]
    pub(crate) fn loopback_for_tests(
        port: u16,
        https_only: bool,
        opts: HttpOptions,
    ) -> HttpTransport {
        Self::build(format!("http://127.0.0.1:{port}"), opts, https_only)
    }

    /// An https server on 127.0.0.1 whose certificate is checked against `roots` (the https only agent, so a plain
    /// connection is refused). Used to prove that the transport verifies the certificate and the host name.
    #[cfg(test)]
    pub(crate) fn tls_loopback_for_tests(
        port: u16,
        roots: RootCerts,
        opts: HttpOptions,
    ) -> HttpTransport {
        Self::build_with_config(
            format!("https://127.0.0.1:{port}"),
            opts,
            true,
            agent_config_with_roots(opts, true, roots),
        )
    }
}

impl Default for HttpTransport {
    fn default() -> Self {
        Self::new()
    }
}

/// A key that cannot split a header or smuggle a line: visible ASCII and space only.
fn header_safe(key: &str) -> bool {
    key.bytes().all(|b| (0x20..0x7f).contains(&b))
}

fn read_bounded(resp: &mut ureq::http::Response<ureq::Body>, max: usize) -> Option<String> {
    resp.body_mut()
        .with_config()
        .limit(max as u64)
        .read_to_string()
        .ok()
}

/// A reply with a content encoding other than identity is refused.
fn is_encoded(resp: &ureq::http::Response<ureq::Body>) -> bool {
    header_text(resp, "content-encoding").is_some_and(|enc| !enc.eq_ignore_ascii_case("identity"))
}

fn header_text(resp: &ureq::http::Response<ureq::Body>, name: &str) -> Option<String> {
    resp.headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
}

impl HttpTransport {
    /// Make the call with the key in the `Authorization` header only; the body is the multipart form when there is one.
    fn call(
        &self,
        key: &ApiKey,
        request: &JevRequest,
    ) -> Result<ureq::http::Response<ureq::Body>, ureq::Error> {
        let url = format!("{}{}", self.base, request.path);
        let auth = format!("Bearer {}", key.expose());
        let multipart = request.multipart();
        match request.method {
            Method::Get => {
                let mut b = self.agent.get(&url).header("Authorization", &auth);
                for (k, v) in request.headers() {
                    b = b.header(k, v);
                }
                b.call()
            }
            Method::Post => {
                let mut b = self.agent.post(&url).header("Authorization", &auth);
                for (k, v) in request.headers() {
                    b = b.header(k, v);
                }
                match &multipart {
                    Some((content_type, body)) => {
                        b.header("Content-Type", content_type).send(body.as_slice())
                    }
                    None => b.send(request.body.as_deref().unwrap_or("").as_bytes()),
                }
            }
        }
    }
}

impl JevTransport for HttpTransport {
    fn send(&self, key: &ApiKey, request: &JevRequest) -> Result<String, JevTransportError> {
        if !header_safe(key.expose()) || !request.path.starts_with('/') {
            return Err(JevTransportError::Network);
        }
        // Defence in depth: the agent is https only too, but refuse here so that nothing is even attempted.
        if self.https_only && !self.base.starts_with("https://") {
            return Err(JevTransportError::Network);
        }
        let result = self.call(key, request);
        // Any failure below (connect, TLS, timeout, an oversized or unreadable reply) carries no detail.
        let mut resp = result.map_err(|_| JevTransportError::Network)?;
        if is_encoded(&resp) {
            return Err(JevTransportError::Network);
        }
        let status = resp.status().as_u16();
        if (200..300).contains(&status) {
            return read_bounded(&mut resp, self.opts.max_reply_bytes)
                .ok_or(JevTransportError::Network);
        }
        let headers = RetryHeaders {
            retry_after: header_text(&resp, "retry-after"),
            retry_after_ms: header_text(&resp, "retry-after-ms"),
        };
        // Only these three bodies are read, and only for `error_type`; a 422 body echoes the whole request.
        let body = if matches!(status, 400 | 401 | 403) {
            read_bounded(&mut resp, self.opts.max_reply_bytes).unwrap_or_default()
        } else {
            String::new()
        };
        Err(JevTransportError::Status {
            status,
            body,
            headers,
        })
    }
}

/// Jitter from the standard library's randomly keyed hasher (no dependency): a fraction in [0, 1).
pub struct OsJitter;

impl Jitter for OsJitter {
    fn unit(&self) -> f64 {
        (RandomState::new().build_hasher().finish() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// A client on the real transport with the default retry policy (2 retries on 429 and 529, 30 s budget) and the key
/// read from `TYPESAFE_API_KEY` at each call. The label gate is closed until [`JevClient::with_gate`] opens it.
pub fn real_client(config: JevConfig) -> JevClient<HttpTransport> {
    JevClient::new(HttpTransport::new(), config).with_retry(
        RetryPolicy::default(),
        Box::new(SystemClock::new()),
        Box::new(ThreadSleeper),
        Box::new(OsJitter),
    )
}

#[cfg(test)]
mod live;
#[cfg(test)]
mod tests;
