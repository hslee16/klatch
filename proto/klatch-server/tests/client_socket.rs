//! The client libraries over the daemon's Unix socket (#1095), Rust side. The daemon (`klatch-server daemon --socket
//! PATH`, #102) is a child process against a throwaway store and a short socket directory under /tmp.
//!
//! Two things are verified. First, the conformance scenario of `klatch/tests/client_scenario` (the same steps and
//! assertions `klatch/tests/client.rs` runs over `klatch serve --stdio`) runs unchanged over the socket: `home()` and
//! `connect()` below are the only difference, so the same events and sequence numbers are required of both transports.
//! Second, what only a socket has: the identity argument (a typed name, a credential, none), a refused identity, a
//! socket nobody listens on, and a close that shuts the socket instead of waiting for a child.

#![cfg(unix)]

use klatch::client::{Client, Error, Identity, PostRequest};
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

#[path = "../../klatch/tests/client_scenario/mod.rs"]
mod client_scenario;

/// What the hello says of a member here: a typed name is unattributed.
pub const ATTRIBUTION: &str = "unattributed";

static N: AtomicU32 = AtomicU32::new(0);
/// Issuing a credential reads the environment of this process: one test at a time may set it.
static ENV: Mutex<()> = Mutex::new(());

/// A throwaway store with its own daemon, started on first use. The directory is short because a socket path is.
pub struct Home {
    dir: PathBuf,
    daemon: Mutex<Option<Child>>,
}

impl Home {
    fn new() -> Home {
        let n = N.fetch_add(1, Ordering::SeqCst);
        let dir = PathBuf::from(format!("/tmp/kc{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for sub in ["", "h", "s"] {
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(dir.join(sub))
                .unwrap();
        }
        Home {
            dir,
            daemon: Mutex::new(None),
        }
    }

    fn sock(&self) -> PathBuf {
        self.dir.join("s").join("d.sock")
    }

    fn log(&self) -> PathBuf {
        self.dir.join("err.log")
    }

    /// Start the daemon once and wait until it says it listens.
    fn socket(&self) -> PathBuf {
        let mut daemon = self.daemon.lock().unwrap();
        if daemon.is_none() {
            let home = self.dir.join("h");
            let child = Command::new(env!("CARGO_BIN_EXE_klatch-server"))
                .args(["daemon", "--socket"])
                .arg(self.sock())
                .env_clear()
                .env("PATH", "/usr/bin:/bin")
                .env("HOME", &home)
                .env("KLATCH_HOME", &home)
                .env("KLATCH_KEY_DIR", home.join("keys"))
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::from(std::fs::File::create(self.log()).unwrap()))
                .spawn()
                .expect("start klatch-server");
            *daemon = Some(child);
            let until = Instant::now() + Duration::from_secs(30);
            loop {
                let said = std::fs::read_to_string(self.log())
                    .map(|l| l.contains("daemon listening on"))
                    .unwrap_or(false);
                if said && std::os::unix::net::UnixStream::connect(self.sock()).is_ok() {
                    break;
                }
                assert!(Instant::now() < until, "the daemon never listened");
                std::thread::sleep(Duration::from_millis(25));
            }
        }
        self.sock()
    }

    /// Stop the daemon now (its connections close).
    fn stop(&self) {
        if let Some(mut child) = self.daemon.lock().unwrap().take() {
            unsafe {
                libc::kill(child.id() as libc::pid_t, libc::SIGKILL);
            }
            let _ = child.wait();
        }
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        self.stop();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

pub fn home() -> Home {
    Home::new()
}

/// A member of the store, over the daemon's socket, by a typed name.
pub fn connect(h: &Home, name: &str) -> Client {
    Client::connect_socket(&h.socket(), &Identity::name(name)).unwrap()
}

fn api(e: Error) -> klatch::client::ApiError {
    match e {
        Error::Api(a) => a,
        other => panic!("not an api error: {other}"),
    }
}

/// Issue a credential for `member` in the store of `h` (the caller holds ENV).
fn issue_credential(h: &Home, member: &str) -> String {
    let store_home = h.dir.join("h");
    unsafe {
        std::env::set_var("KLATCH_HOME", &store_home);
        std::env::set_var("KLATCH_KEY_DIR", store_home.join("keys"));
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
    gateway
        .admin_issue_credential(&klatch::domain::Principal::unattributed("launcher"), &spec)
        .unwrap()
        .to_text()
        .to_string()
}

#[test]
fn a_typed_name_is_the_member_and_unattributed_after_the_handshake() {
    let h = home();
    let mut a = connect(&h, "alice");
    assert_eq!(
        (
            a.member.as_str(),
            a.attribution.as_str(),
            a.clearance.as_str()
        ),
        ("alice", "unattributed", "public")
    );
    a.post(PostRequest::new("r", "hi")).unwrap();
    let ms = a.history("r", Some(5), None).unwrap().messages;
    assert_eq!(ms[0].sender, "alice");
}

#[test]
fn no_identity_is_the_unmapped_process_the_daemon_gives_every_local_process() {
    let h = home();
    let a = Client::connect_socket(&h.socket(), &Identity::none()).unwrap();
    assert_eq!(
        (a.member.as_str(), a.attribution.as_str()),
        ("unmapped-process", "unattributed")
    );
}

#[test]
fn a_credential_gives_a_verified_member() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let h = home();
    let token = issue_credential(&h, "w-1");
    let sock = h.socket();
    let mut w = Client::connect_socket(&sock, &Identity::credential("w-1", &token)).unwrap();
    assert_eq!(
        (w.member.as_str(), w.attribution.as_str()),
        ("w-1", "verified")
    );
    w.post(PostRequest::new("r", "from a worker")).unwrap();
    let seen = w.history("r", Some(5), None).unwrap().messages;
    assert_eq!(seen[0].attribution, "verified");
    // a wrong token, or another member's name with this token, ends the connection: the error names the code
    // and never repeats the token
    for bad in [
        Identity::credential("w-1", &"0".repeat(64)),
        Identity::credential("w-2", &token),
    ] {
        let e = Client::connect_socket(&sock, &bad).map(|_| ()).unwrap_err();
        let a = api(e);
        assert_eq!(a.code, "identity.refused");
        assert!(!a.message.contains(&token), "{}", a.message);
    }
}

#[test]
fn a_bad_typed_name_is_refused_and_the_error_never_echoes_it() {
    let h = home();
    let sock = h.socket();
    for bad in ["admin", "a b", "x/y"] {
        let e = Client::connect_socket(&sock, &Identity::name(bad))
            .map(|_| ())
            .unwrap_err();
        let a = api(e);
        assert_eq!(a.code, "identity.refused");
        assert!(!a.message.contains(bad), "{}", a.message);
    }
}

#[test]
fn a_socket_nobody_listens_on_is_a_session_error_and_close_shuts_the_socket() {
    let h = home();
    let missing = h.dir.join("s").join("nobody.sock");
    let e = Client::connect_socket(&missing, &Identity::name("alice"))
        .map(|_| ())
        .unwrap_err();
    assert!(matches!(e, Error::Session(_)), "{e}");
    let a = connect(&h, "alice");
    assert_eq!(a.close().unwrap(), 0);
    // the daemon is still up and a later connection works
    let mut b = connect(&h, "bob");
    b.post(PostRequest::new("r", "after")).unwrap();
}

#[test]
fn the_daemon_ending_the_session_is_a_session_error_not_a_hang() {
    let h = home();
    let mut a = connect(&h, "alice");
    a.post(PostRequest::new("r", "x")).unwrap();
    h.stop();
    let e = a.post(PostRequest::new("r", "y")).unwrap_err();
    assert!(matches!(e, Error::Session(_)), "{e}");
}
