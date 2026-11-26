//! Helpers of the TLS listener tests (#108): throwaway certificates (rcgen, a dev-dependency only; nothing here is
//! production code and no key lives outside a test's own temp directory), a temp store, the server as a child process,
//! and a rustls client that can present, omit or misuse a certificate. Not a test file: both `tls_*` tests include it.
#![allow(dead_code)]

pub mod der;
pub mod scenario;
pub mod wire;
pub mod ws;

use rcgen::{
    BasicConstraints, CertificateParams, CertificateRevocationListParams, DnType,
    ExtendedKeyUsagePurpose, IsCa, Issuer, KeyIdMethod, KeyPair, KeyUsagePurpose, RevocationReason,
    RevokedCertParams, SanType, SerialNumber, date_time_ymd,
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
    /// This CA's certificate, and the certificates above it (empty for a root): what a client sends after its own.
    der: CertificateDer<'static>,
    above: Vec<CertificateDer<'static>>,
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
    /// The issuing CA's certificate and those above it, sent with the leaf by a client of an intermediate CA.
    chain: Vec<CertificateDer<'static>>,
}

impl Ca {
    pub fn new(name: &str) -> Ca {
        Ca::new_serial(name, None)
    }

    /// [`Ca::new`] with a serial number the test chose for the CA's own certificate, so that a CRL can name it.
    pub fn new_serial(name: &str, serial: Option<u64>) -> Ca {
        let mut params = CertificateParams::new(Vec::new()).unwrap();
        params.serial_number = serial.map(SerialNumber::from);
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        params.distinguished_name.push(DnType::CommonName, name);
        let key = KeyPair::generate().unwrap();
        let cert = params.self_signed(&key).unwrap();
        Ca {
            params,
            key,
            pem: cert.pem(),
            der: cert.der().clone(),
            above: Vec::new(),
        }
    }

    /// What a client of this CA sends after its leaf: nothing for a root, else this certificate and the intermediates
    /// above it (never the root, which the server holds).
    fn chain_to_send(&self) -> Vec<CertificateDer<'static>> {
        match self.above.split_last() {
            None => Vec::new(),
            Some((_root, between)) => std::iter::once(self.der.clone())
                .chain(between.iter().cloned())
                .collect(),
        }
    }

    /// An intermediate CA signed by this one, with the key usages the test chooses (an intermediate that may sign
    /// certificates but not CRLs is the point of one test).
    pub fn intermediate(&self, name: &str, usages: Vec<KeyUsagePurpose>) -> Ca {
        self.intermediate_serial(name, usages, None)
    }

    /// [`Ca::intermediate`] with a serial number the test chose, so a CRL of this CA's own issuer can name it.
    pub fn intermediate_serial(
        &self,
        name: &str,
        usages: Vec<KeyUsagePurpose>,
        serial: Option<u64>,
    ) -> Ca {
        let mut params = CertificateParams::new(Vec::new()).unwrap();
        params.serial_number = serial.map(SerialNumber::from);
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = usages;
        params.distinguished_name.push(DnType::CommonName, name);
        let key = KeyPair::generate().unwrap();
        let issuer = Issuer::new(self.params.clone(), &self.key);
        let cert = params.signed_by(&key, &issuer).unwrap();
        let mut above = vec![self.der.clone()];
        above.extend(self.above.iter().cloned());
        Ca {
            params,
            key,
            pem: cert.pem(),
            der: cert.der().clone(),
            above,
        }
    }

    pub fn issue(&self, name: &str, usage: Use) -> Leaf {
        self.issue_serial(name, usage, None)
    }

    /// [`Ca::issue`] with a serial number the test chose, so a CRL can name the certificate.
    pub fn issue_serial(&self, name: &str, usage: Use, serial: Option<u64>) -> Leaf {
        let mut params = CertificateParams::new(Vec::new()).unwrap();
        params.distinguished_name.push(DnType::CommonName, name);
        let (eku, expired) = match usage {
            Use::Server => (ExtendedKeyUsagePurpose::ServerAuth, false),
            Use::Client => (ExtendedKeyUsagePurpose::ClientAuth, false),
            Use::ExpiredClient => (ExtendedKeyUsagePurpose::ClientAuth, true),
        };
        params.extended_key_usages = vec![eku];
        params.serial_number = serial.map(SerialNumber::from);
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
            chain: self.chain_to_send(),
        }
    }
}

impl Ca {
    /// A CRL (PEM) signed by this CA that lists the serial numbers given. Dates are relative to now, never literal: a
    /// CRL that `expired` was issued two days ago and was due a day ago; a current one is due in thirty days.
    pub fn crl_pem(&self, revoked: &[u64], expired: bool) -> String {
        let due = if expired { -86_400 } else { 30 * 86_400 };
        self.crl_signed_by(revoked, due, &self.key)
    }

    /// A CRL (PEM) with no revocations whose nextUpdate is `due_in` seconds from now (negative: already past).
    pub fn crl_pem_due(&self, due_in: i64) -> String {
        self.crl_signed_by(&[], due_in, &self.key)
    }

    /// A CRL that names this CA as its issuer but is signed by another key: its signature does not verify.
    pub fn forged_crl_pem(&self, revoked: &[u64]) -> String {
        self.crl_signed_by(revoked, 30 * 86_400, &KeyPair::generate().unwrap())
    }

    fn crl_signed_by(&self, revoked: &[u64], due_in: i64, signer: &KeyPair) -> String {
        use std::time::{Duration as D, SystemTime};
        let now = SystemTime::now();
        let next = if due_in < 0 {
            now - D::from_secs(due_in.unsigned_abs())
        } else {
            now + D::from_secs(due_in.unsigned_abs())
        };
        self.crl_between(revoked, now - 2 * D::from_secs(86_400), next, signer)
    }

    /// A CRL (PEM) with no revocations, `this_update` and `next_update` as given (a test of a time before 1970).
    pub fn crl_pem_between(
        &self,
        this_update: std::time::SystemTime,
        next_update: std::time::SystemTime,
    ) -> String {
        self.crl_between(&[], this_update, next_update, &self.key)
    }

    fn crl_between(
        &self,
        revoked: &[u64],
        this_update: std::time::SystemTime,
        next_update: std::time::SystemTime,
        signer: &KeyPair,
    ) -> String {
        use std::time::{Duration as D, SystemTime};
        let now = SystemTime::now();
        let day = D::from_secs(86_400);
        let params = CertificateRevocationListParams {
            this_update: this_update.into(),
            next_update: next_update.into(),
            crl_number: SerialNumber::from(1u64),
            issuing_distribution_point: None,
            revoked_certs: revoked
                .iter()
                .map(|n| RevokedCertParams {
                    serial_number: SerialNumber::from(*n),
                    revocation_time: (now - day).into(),
                    reason_code: Some(RevocationReason::KeyCompromise),
                    invalidity_date: None,
                })
                .collect(),
            key_identifier_method: KeyIdMethod::Sha256,
        };
        // rcgen refuses to sign a CRL for a CA whose usages lack cRLSign; the test CRL claims it anyway (the CA
        // certificate that goes in the CA file does not), which is exactly what a real-world misissue looks like
        let mut signing = self.params.clone();
        if !signing.key_usages.contains(&KeyUsagePurpose::CrlSign) {
            signing.key_usages.push(KeyUsagePurpose::CrlSign);
        }
        let issuer = Issuer::new(signing, signer);
        params.signed_by(&issuer).unwrap().pem().unwrap()
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

/// [`write_file`] over a file that exists (a test changing what the set-up wrote).
pub fn rewrite_file(path: &Path, text: &str, mode: u32) {
    let _ = std::fs::remove_file(path);
    write_file(path, text, mode);
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
    spawn_env(dir, subcommand, args, short_limits, &[])
}

/// [`spawn_with`] and more environment variables for the child (a test knob of a debug build, #1621).
pub fn spawn_env(
    dir: &Dir,
    subcommand: &str,
    args: &[String],
    short_limits: bool,
    env: &[(&str, &str)],
) -> Child {
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
    command.envs(env.iter().copied());
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
        Server::started_env(dir, subcommand, extra, &[])
    }

    /// [`Server::started`] with more environment variables for the child.
    pub fn started_env(
        dir: &Dir,
        subcommand: &str,
        extra: &[String],
        env: &[(&str, &str)],
    ) -> Server {
        let mut args = vec!["--listen".to_string(), "127.0.0.1:0".to_string()];
        args.extend_from_slice(extra);
        let mut server = Server {
            child: spawn_env(dir, subcommand, &args, true, env),
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
                std::iter::once(l.cert.clone())
                    .chain(l.chain.iter().cloned())
                    .collect(),
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
