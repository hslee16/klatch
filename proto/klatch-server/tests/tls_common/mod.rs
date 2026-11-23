//! Helpers of the TLS listener tests (#108): throwaway certificates (rcgen, a dev-dependency only; nothing here is
//! production code and no key lives outside a test's own temp directory), a temp store, the server as a child process,
//! and a rustls client that can present, omit or misuse a certificate. Not a test file: both `tls_*` tests include it.
#![allow(dead_code)]

pub mod wire;

use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    KeyUsagePurpose, SanType, date_time_ymd,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

static N: AtomicU32 = AtomicU32::new(0);

/// A short directory under /tmp with a store home and a place for certificates.
pub struct Dir(pub PathBuf);

impl Dir {
    pub fn new() -> Dir {
        let n = N.fetch_add(1, Ordering::SeqCst);
        let dir = PathBuf::from(format!("/tmp/kt{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for sub in ["", "h", "pki"] {
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(dir.join(sub))
                .unwrap();
        }
        Dir(dir)
    }
    pub fn home(&self) -> PathBuf {
        self.0.join("h")
    }
    pub fn log(&self) -> String {
        std::fs::read_to_string(self.0.join("err.log")).unwrap_or_default()
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!("server log of {}:\n{}", self.0.display(), self.log());
        }
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A certificate authority that exists for one test.
pub struct Ca {
    params: CertificateParams,
    key: KeyPair,
    pub pem: String,
}

/// What a leaf certificate is for, and when it is valid.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Use {
    Server,
    Client,
    ExpiredClient,
}

/// A leaf certificate and its key.
pub struct Leaf {
    pub cert_pem: String,
    pub key_pem: String,
    cert: CertificateDer<'static>,
    key: Vec<u8>,
}

impl Ca {
    pub fn new(name: &str) -> Ca {
        let mut params = CertificateParams::new(Vec::new()).unwrap();
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        params.distinguished_name.push(DnType::CommonName, name);
        let key = KeyPair::generate().unwrap();
        let pem = params.self_signed(&key).unwrap().pem();
        Ca { params, key, pem }
    }

    pub fn issue(&self, name: &str, usage: Use) -> Leaf {
        let mut params = CertificateParams::new(Vec::new()).unwrap();
        params.distinguished_name.push(DnType::CommonName, name);
        let (eku, expired) = match usage {
            Use::Server => (ExtendedKeyUsagePurpose::ServerAuth, false),
            Use::Client => (ExtendedKeyUsagePurpose::ClientAuth, false),
            Use::ExpiredClient => (ExtendedKeyUsagePurpose::ClientAuth, true),
        };
        params.extended_key_usages = vec![eku];
        if usage == Use::Server {
            params
                .subject_alt_names
                .push(SanType::IpAddress(IpAddr::from([127, 0, 0, 1])));
        }
        if expired {
            params.not_before = date_time_ymd(2020, 1, 1);
            params.not_after = date_time_ymd(2021, 1, 1);
        }
        let key = KeyPair::generate().unwrap();
        let issuer = Issuer::new(self.params.clone(), &self.key);
        let cert = params.signed_by(&key, &issuer).unwrap();
        Leaf {
            cert_pem: cert.pem(),
            key_pem: key.serialize_pem(),
            cert: cert.der().clone(),
            key: key.serialize_der(),
        }
    }
}

/// Write a file with the given mode (the key files of the tests are 0600 unless a test says otherwise).
pub fn write_file(path: &Path, text: &str, mode: u32) {
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(path)
        .unwrap();
    f.write_all(text.as_bytes()).unwrap();
    // the umask may have cleared bits the test wants (group or other access)
    std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(mode)).unwrap();
}

/// The files of one server set-up: its certificate and key, and the CA file for client certificates.
pub struct Files {
    pub cert: PathBuf,
    pub key: PathBuf,
    pub ca: PathBuf,
}

pub fn server_files(dir: &Dir, server: &Leaf, client_ca: &Ca, key_mode: u32) -> Files {
    let pki = dir.0.join("pki");
    let files = Files {
        cert: pki.join("server.pem"),
        key: pki.join("server.key"),
        ca: pki.join("client-ca.pem"),
    };
    write_file(&files.cert, &server.cert_pem, 0o644);
    write_file(&files.key, &server.key_pem, key_mode);
    write_file(&files.ca, &client_ca.pem, 0o644);
    files
}

pub fn tls_args(files: &Files) -> Vec<String> {
    let p = |f: &Path| f.display().to_string();
    vec![
        "--tls-cert".into(),
        p(&files.cert),
        "--tls-key".into(),
        p(&files.key),
        "--tls-client-ca".into(),
        p(&files.ca),
    ]
}

/// Start `klatch-server SUBCOMMAND ARGS` against the store of `dir`; stderr goes to `err.log`.
pub fn spawn(dir: &Dir, subcommand: &str, args: &[String]) -> Child {
    spawn_with(dir, subcommand, args, true)
}

/// [`spawn`], with or without the knob that lets a debug build take limits below the shipped minimums (a short stall
/// time, so that a test need not wait a minute). `false` is the server as an operator runs it.
pub fn spawn_with(dir: &Dir, subcommand: &str, args: &[String], short_limits: bool) -> Child {
    let mut command = Command::new(env!("CARGO_BIN_EXE_klatch-server"));
    command
        .arg(subcommand)
        .args(args)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", dir.home())
        .env("KLATCH_HOME", dir.home())
        .env("KLATCH_KEY_DIR", dir.home().join("keys"));
    if short_limits {
        command.env("KLATCH_TEST_SHORT_LIMITS", "1");
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.0.join("err.log"))
                .unwrap(),
        ))
        .spawn()
        .expect("start klatch-server")
}

/// A running server, killed (only this child) when dropped.
pub struct Server {
    pub child: Child,
    pub addr: SocketAddr,
}

impl Server {
    /// Start on a port the system chooses and wait until the log names it (a complete line, never half an address).
    pub fn started(dir: &Dir, subcommand: &str, extra: &[String]) -> Server {
        let mut args = vec!["--listen".to_string(), "127.0.0.1:0".to_string()];
        args.extend_from_slice(extra);
        let mut server = Server {
            child: spawn(dir, subcommand, &args),
            addr: "127.0.0.1:0".parse().unwrap(),
        };
        let marker = format!("{subcommand} listening on ");
        let until = Instant::now() + Duration::from_secs(60);
        while Instant::now() < until {
            let text = dir.log();
            if let Some(rest) = text.split(marker.as_str()).nth(1)
                && rest.contains('\n')
            {
                server.addr = rest.split_whitespace().next().unwrap().parse().unwrap();
                return server;
            }
            if let Some(status) = server.child.try_wait().unwrap() {
                panic!(
                    "the server stopped ({status}) before listening:\n{}",
                    dir.log()
                );
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        panic!("the server never listened:\n{}", dir.log());
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        unsafe {
            libc::kill(self.child.id() as libc::pid_t, libc::SIGKILL);
        }
        let _ = self.child.wait();
    }
}

/// Run a server that is expected to refuse to start: its exit code and stderr. It fails fast, by assertion, when the
/// server does not refuse: a log line that says it is listening, or no exit within 20 seconds, kills the child (only
/// the one started here) and panics. A check that was removed from the server therefore fails a test and never hangs it.
pub fn refused_start(dir: &Dir, subcommand: &str, args: &[String]) -> (Option<i32>, String) {
    refused_start_with(dir, subcommand, args, true)
}

/// [`refused_start`] for the server as an operator runs it (no test knob for short limits).
pub fn refused_start_strict(dir: &Dir, subcommand: &str, args: &[String]) -> (Option<i32>, String) {
    refused_start_with(dir, subcommand, args, false)
}

fn refused_start_with(
    dir: &Dir,
    subcommand: &str,
    args: &[String],
    short_limits: bool,
) -> (Option<i32>, String) {
    let mut child = spawn_with(dir, subcommand, args, short_limits);
    let until = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return (status.code(), dir.log());
        }
        let listening = dir.log().contains("listening on");
        if listening || Instant::now() >= until {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "the server was expected to refuse to start but {}:\n{}",
                if listening {
                    "is listening"
                } else {
                    "did not exit in 20 seconds"
                },
                dir.log()
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The server's CA is what a client trusts; `client` is the certificate it presents, if any; `versions` limits what it
/// offers (`None` is rustls's default, TLS 1.3 and 1.2).
pub fn client_config(
    server_ca: &Ca,
    client: Option<&Leaf>,
    versions: Option<&[&'static rustls::SupportedProtocolVersion]>,
) -> Arc<rustls::ClientConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut roots = rustls::RootCertStore::empty();
    for c in rustls_pki_types::CertificateDer::pem_slice_iter(server_ca.pem.as_bytes()) {
        roots.add(c.unwrap()).unwrap();
    }
    let builder = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(versions.unwrap_or(rustls::DEFAULT_VERSIONS))
        .unwrap()
        .with_root_certificates(roots);
    let config = match client {
        Some(l) => builder
            .with_client_auth_cert(
                vec![l.cert.clone()],
                PrivateKeyDer::try_from(l.key.clone()).unwrap(),
            )
            .unwrap(),
        None => builder.with_no_client_auth(),
    };
    Arc::new(config)
}

use rustls_pki_types::pem::PemObject;

/// A TLS connection to the server, ready to read and write plaintext.
pub type Tls = rustls::StreamOwned<rustls::ClientConnection, TcpStream>;

/// Connect and finish the handshake. A server that refuses the certificate may only say so on the first read (TLS 1.3
/// lets the client finish before the server has checked its certificate), so a caller that wants "refused" uses
/// [`get_status`], which reads.
pub fn tls_connect(addr: SocketAddr, config: &Arc<rustls::ClientConfig>) -> std::io::Result<Tls> {
    let sock = TcpStream::connect(addr)?;
    sock.set_read_timeout(Some(Duration::from_secs(20)))?;
    sock.set_write_timeout(Some(Duration::from_secs(20)))?;
    let name = ServerName::IpAddress(addr.ip().into());
    let mut conn =
        rustls::ClientConnection::new(config.clone(), name).map_err(std::io::Error::other)?;
    let mut sock = sock;
    while conn.is_handshaking() {
        conn.complete_io(&mut sock)?;
    }
    Ok(rustls::StreamOwned::new(conn, sock))
}

/// `GET path` with the headers; the status, or the error that says the connection was refused or broken.
pub fn get_status(
    addr: SocketAddr,
    config: &Arc<rustls::ClientConfig>,
    path: &str,
) -> std::io::Result<u16> {
    let mut tls = tls_connect(addr, config)?;
    tls.write_all(format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes())?;
    let mut buf = [0u8; 12];
    tls.read_exact(&mut buf)?;
    let line = String::from_utf8_lossy(&buf).to_string();
    line.split(' ')
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| std::io::Error::other(format!("status line {line:?}")))
}
