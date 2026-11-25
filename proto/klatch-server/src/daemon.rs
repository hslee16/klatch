//! `klatch-server daemon --socket PATH` (#102, step 2 of epic #13): one gateway, owned by one thread
//! (`SharedGateway`, #140), served over a local Unix domain socket to any number of connections. Each connection runs the
//! session protocol of `klatch serve --stdio` (PROTOCOL.md) over the socket, one session loop per connection.
//!
//! The only listener is a Unix socket, in one directory the daemon's own user owns and nobody else can write to, with
//! mode 0600. There is no TCP, no address and no name resolution here. This is the one module that may name the Unix
//! socket types and the kernel's peer-credential calls (`tests/no_network.rs` enforces it).
//!
//! **Who a connection is.** The kernel says which process is on the other end (`SO_PEERCRED` on Linux, `getpeereid` and
//! `LOCAL_PEERPID` on macOS). A peer whose uid is not the daemon's is closed before a byte is read or written. Otherwise
//! the evidence is `PeerEvidence::Process { pid, uid }`, same-host evidence that never proves a member, so a connection
//! starts as the principal the evidence table gives it: `unmapped-process`, unattributed (R23). The pid is evidence for
//! the record and never a name. A connection can say more, once, as its first request, `identify`:
//! - `{"name": "alice"}` is a typed name: unattributed, as `KLATCH_NAME` is for the CLI today;
//! - `{"member": "w-1", "token": "..."}` is a credential token: checked by the gateway exactly as `klatch serve --stdio`
//!   checks a credential bundle (`Gateway::verify_credential`: token, expiry, revocation, a refusal recorded without the
//!   token); only a live credential is verified. A bad token ends the connection: nothing falls back to a weaker name.
//!
//! **Keys and harness (#1096).** For each request the owner loads the keys of the sealed labels the connection's
//! principal is cleared to, and no others, and drops them after it. A connection that never sent `identify` and whose
//! kernel-reported pid is inside a verified Claude Code session joins as Claude Code (a room's `room-require` admits it);
//! every other connection joins as the generic floor.
//!
//! **Limits.** A frame is at most `MAX_LINE_BYTES`; a connection that sends no first frame within `--first-frame-secs`
//! is closed (a connection that has spoken is never closed for silence: a subscriber waits for pushes); at most
//! `--max-connections` connections at once, and the excess gets one refusal frame and is closed; output goes through the
//! session's bounded queue.

use anyhow::{Context, Result, bail};
use klatch::domain::Principal;
use klatch::gateway::{EvidenceTable, Identity, LocalIdentity, PeerEvidence};
use klatch::gateway::{SharedBackend, SharedGateway};
use klatch::protocol::{ErrorBody, MAX_LINE_BYTES, PROTOCOL_VERSION, ServerFrame, encode_line};
use klatch::session::{Backend, Session, SessionConfig, hello_frame};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Cursor, Read, Write};
use std::net::Shutdown;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Concurrent connections served by default.
pub const DEFAULT_MAX_CONNECTIONS: usize = 32;
/// How long a connection may send nothing before its first frame, by default.
pub const DEFAULT_FIRST_FRAME: Duration = Duration::from_secs(10);

/// What `klatch-server daemon` was asked for.
#[derive(Debug, Clone)]
pub struct Options {
    pub socket: PathBuf,
    pub max_connections: usize,
    pub first_frame: Duration,
}

/// Read the options after `daemon`. The socket path has no default: a daemon never picks a place by itself.
pub fn parse(args: &[&str]) -> Result<Options> {
    let mut socket = None;
    let mut max_connections = DEFAULT_MAX_CONNECTIONS;
    let mut first_frame = DEFAULT_FIRST_FRAME;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut value = |name: &str| -> Result<&str> {
            it.next()
                .copied()
                .with_context(|| format!("{name} needs a value"))
        };
        match *a {
            "--socket" => socket = Some(PathBuf::from(value("--socket")?)),
            "--max-connections" => {
                max_connections = value("--max-connections")?
                    .parse()
                    .ok()
                    .filter(|n| *n > 0)
                    .context("--max-connections is a positive number")?;
            }
            "--first-frame-secs" => {
                first_frame = Duration::from_secs(
                    value("--first-frame-secs")?
                        .parse()
                        .ok()
                        .filter(|n| *n > 0)
                        .context("--first-frame-secs is a positive number")?,
                );
            }
            other => bail!("unknown option {other} for daemon"),
        }
    }
    Ok(Options {
        socket: socket.context("daemon needs --socket PATH")?,
        max_connections,
        first_frame,
    })
}

// ---------------------------------------------------------------------------------------------------------------
// who is on the other end

/// What the kernel reports about the process at the other end of a connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerCred {
    pub pid: u32,
    pub uid: u32,
}

/// The evidence a local peer gives, or why it is refused: its uid must be the daemon's own. The pid goes into the
/// evidence and nowhere else; it is never a name.
pub fn local_evidence(peer: PeerCred, own_uid: u32) -> Result<PeerEvidence, WrongUid> {
    if peer.uid != own_uid {
        return Err(WrongUid);
    }
    Ok(PeerEvidence::Process {
        pid: peer.pid,
        uid: peer.uid,
    })
}

/// A peer that runs as another user.
#[derive(Debug, PartialEq, Eq)]
pub struct WrongUid;

#[cfg(target_os = "linux")]
fn peer_cred(stream: &UnixStream) -> io::Result<PeerCred> {
    let mut c: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let r = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut c as *mut libc::ucred).cast(),
            &mut len,
        )
    };
    if r != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(PeerCred {
        pid: c.pid as u32,
        uid: c.uid,
    })
}

#[cfg(target_os = "macos")]
fn peer_cred(stream: &UnixStream) -> io::Result<PeerCred> {
    let fd = stream.as_raw_fd();
    let (mut uid, mut gid): (libc::uid_t, libc::gid_t) = (0, 0);
    if unsafe { libc::getpeereid(fd, &mut uid, &mut gid) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let mut pid: libc::pid_t = 0;
    let mut len = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
    let r = unsafe {
        libc::getsockopt(
            fd,
            0, // SOL_LOCAL
            libc::LOCAL_PEERPID,
            (&mut pid as *mut libc::pid_t).cast(),
            &mut len,
        )
    };
    // the pid is for the record only: a peer that has gone since it connected has none
    Ok(PeerCred {
        pid: if r == 0 { pid as u32 } else { 0 },
        uid,
    })
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn peer_cred(_stream: &UnixStream) -> io::Result<PeerCred> {
    Err(io::Error::other(
        "no peer credentials on this system; the daemon cannot tell who connects",
    ))
}

// ---------------------------------------------------------------------------------------------------------------
// the socket file

/// The most bytes a socket path may have here (the room `sun_path` has, less its terminating zero).
pub fn path_limit() -> usize {
    let a: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    a.sun_path.len() - 1
}

/// Why `path` cannot be a socket path, if it cannot: too long for `sun_path`.
pub fn path_problem(path: &Path, limit: usize) -> Option<String> {
    let n = path.as_os_str().len();
    (n > limit).then(|| {
        format!(
            "the socket path is too long ({n} bytes; the limit on this system is {limit} bytes, the size of a socket address): use a shorter path, for example one in a directory made with `mktemp -d` under /tmp"
        )
    })
}

/// Why a directory (its owner, its permission bits) is not a place for the socket, if it is not: it must belong to the
/// daemon's user and be writable by that user alone.
pub fn directory_problem(dir: &Path, owner: u32, mode: u32, own_uid: u32) -> Option<String> {
    if owner != own_uid {
        return Some(format!(
            "the socket directory {} is not safe: it is owned by uid {owner}, not by uid {own_uid} (the user running the daemon)",
            dir.display()
        ));
    }
    (mode & 0o022 != 0).then(|| {
        format!(
            "the socket directory {} is not safe: it is writable by its group or by others (mode {:o}); make it owned by you and not group or world writable (chmod 700)",
            dir.display(),
            mode & 0o777
        )
    })
}

fn own_uid() -> u32 {
    unsafe { libc::geteuid() }
}

/// The directory of `socket`, checked.
fn checked_directory(socket: &Path) -> Result<PathBuf> {
    if let Some(p) = path_problem(socket, path_limit()) {
        bail!("{p}");
    }
    let dir = match socket.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let md = std::fs::metadata(&dir).ok().filter(|m| m.is_dir());
    let Some(md) = md else {
        bail!(
            "the socket directory {} does not exist or is not a directory",
            dir.display()
        );
    };
    if let Some(p) = directory_problem(&dir, md.uid(), md.mode(), own_uid()) {
        bail!("{p}");
    }
    Ok(dir)
}

/// Make the way free for a new socket at `path`: nothing there is fine; a socket nobody listens on (a dead daemon's) is
/// removed; a live daemon's is never taken; anything else is never removed.
fn clear_path(path: &Path) -> Result<()> {
    let md = match std::fs::symlink_metadata(path) {
        Ok(md) => md,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => bail!("cannot look at {}: {e}", path.display()),
    };
    if !md.file_type().is_socket() {
        bail!(
            "{} exists and is not a socket; not removing it",
            path.display()
        );
    }
    // the one connect in this crate: a probe of the daemon's own socket path, to tell a live daemon from a dead one's file
    match UnixStream::connect(path) {
        Ok(_) => bail!("a daemon is already listening on {}", path.display()),
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound
            ) =>
        {
            std::fs::remove_file(path)
                .with_context(|| format!("cannot remove the stale socket {}", path.display()))
        }
        Err(e) => bail!(
            "cannot tell whether {} is in use ({e}); not removing it",
            path.display()
        ),
    }
}

/// The listening socket, and what is needed to remove it again (only if it is still ours).
struct Bound {
    listener: UnixListener,
    path: PathBuf,
    ident: (u64, u64),
}

fn bind(path: &Path) -> Result<Bound> {
    // the file is born 0600: no moment where it is more open than that
    let old = unsafe { libc::umask(0o177) };
    let bound = UnixListener::bind(path);
    unsafe { libc::umask(old) };
    let listener = bound.with_context(|| format!("cannot listen on {}", path.display()))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    let md = std::fs::symlink_metadata(path)?;
    listener.set_nonblocking(true)?;
    Ok(Bound {
        listener,
        path: path.to_path_buf(),
        ident: (md.dev(), md.ino()),
    })
}

impl Drop for Bound {
    fn drop(&mut self) {
        // remove the file only if it is still the one this daemon made
        if let Ok(md) = std::fs::symlink_metadata(&self.path)
            && (md.dev(), md.ino()) == self.ident
        {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// stopping

pub(crate) static STOP: AtomicBool = AtomicBool::new(false);

/// Set by SIGHUP in the servers that install the hangup handler (the TLS listener reloads its files when it sees this;
/// the plain `http` server never reads it, so for it SIGHUP is caught and ignored). The handler does nothing else.
pub(crate) static RELOAD: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    STOP.store(true, Ordering::SeqCst);
}

extern "C" fn on_hangup(_: libc::c_int) {
    RELOAD.store(true, Ordering::SeqCst);
}

/// SIGTERM and SIGINT stop the server. The daemon keeps the default for SIGHUP.
pub(crate) fn install_signal_handlers() {
    install(false);
}

/// [`install_signal_handlers`], and SIGHUP sets [`RELOAD`] instead of ending the process (for the TLS listener, which
/// reloads, and the `http` listener, which has nothing to reload and so ignores it).
pub(crate) fn install_signal_handlers_with_hangup() {
    install(true);
}

fn install(hangup: bool) {
    let mut act: libc::sigaction = unsafe { std::mem::zeroed() };
    act.sa_sigaction = on_signal as extern "C" fn(libc::c_int) as usize;
    // no SA_RESTART: the wait for a connection returns when a signal comes
    act.sa_flags = 0;
    unsafe {
        libc::sigemptyset(&mut act.sa_mask);
        libc::sigaction(libc::SIGTERM, &act, std::ptr::null_mut());
        libc::sigaction(libc::SIGINT, &act, std::ptr::null_mut());
        if hangup {
            // restarted reads and writes where the system restarts them (macOS: all of them; on Linux a socket read with a
            // timeout, poll and nanosleep still return EINTR, which the std helpers retry)
            let mut hup = act;
            hup.sa_sigaction = on_hangup as extern "C" fn(libc::c_int) as usize;
            hup.sa_flags = libc::SA_RESTART;
            libc::sigaction(libc::SIGHUP, &hup, std::ptr::null_mut());
        }
    }
}

/// Wait up to `ms` for the listener to have a connection waiting.
fn wait_readable(fd: i32, ms: i32) {
    let mut p = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    unsafe {
        libc::poll(&mut p, 1, ms);
    }
}

// ---------------------------------------------------------------------------------------------------------------
// serving

/// What every connection thread shares.
struct Shared {
    backend: SharedBackend,
    /// Whether a process runs inside a verified Claude Code session (`klatch::member::peer_runs_claude_code`); a field
    /// so a test can stand in for the process table.
    claude_code_at: fn(u32) -> bool,
    first_frame: Duration,
    config: SessionConfig,
}

type Open = Arc<Mutex<HashMap<u64, UnixStream>>>;

/// The next waiting connection, or `None` when there is none (or the accept failed: it is tried again after the wait).
fn next_connection(listener: &UnixListener) -> Option<UnixStream> {
    listener.accept().ok().map(|(stream, _)| stream)
}

/// Decide what an accepted connection is: its slot, who it is, and the socket to serve. `None` closes it (a peer the
/// kernel cannot name, another user's, or one over the cap, which is told so first).
fn admit(
    stream: UnixStream,
    uid: u32,
    (open, max_connections): (&Open, usize),
    next_id: &mut u64,
) -> Option<(u64, PeerEvidence, UnixStream)> {
    // an accepted socket may inherit the listener's non-blocking mode
    stream.set_nonblocking(false).ok()?;
    // who first: another user's connection is closed before a byte moves in either direction
    let peer = match peer_cred(&stream) {
        Ok(p) => p,
        Err(e) => {
            // never serve a peer the kernel cannot name; say so (no peer data is involved)
            eprintln!(
                "klatch-server: closed a connection whose peer credentials could not be read: {e}"
            );
            return None;
        }
    };
    let evidence = local_evidence(peer, uid).ok()?;
    let handle = stream.try_clone().ok()?;
    let slot = {
        let mut open = open.lock().unwrap_or_else(|e| e.into_inner());
        if open.len() >= max_connections {
            None
        } else {
            *next_id += 1;
            open.insert(*next_id, handle);
            Some(*next_id)
        }
    };
    let Some(id) = slot else {
        refuse_excess(stream);
        return None;
    };
    Some((id, evidence, stream))
}

/// Run the daemon until SIGTERM or SIGINT. Order matters: the options and the socket directory are checked, then the
/// store is opened (a store that cannot be opened ends the daemon before a socket is touched), then the socket path is
/// cleared and bound.
pub fn run(opts: &Options) -> Result<()> {
    checked_directory(&opts.socket)?;
    let gateway: SharedGateway = klatch::member::spawn_shared_gateway()
        .context("the daemon cannot open the store, so it does not start")?;
    clear_path(&opts.socket)?;
    let bound = bind(&opts.socket)?;
    install_signal_handlers();
    eprintln!(
        "klatch-server: daemon listening on {} (local socket, mode 0600, up to {} connections)",
        opts.socket.display(),
        opts.max_connections
    );
    let shared = Shared {
        backend: gateway.session_backend(),
        claude_code_at: klatch::member::peer_runs_claude_code,
        first_frame: opts.first_frame,
        config: SessionConfig::default(),
    };
    let open: Open = Arc::default();
    let uid = own_uid();
    std::thread::scope(|scope| {
        let mut next_id = 0u64;
        while !STOP.load(Ordering::SeqCst) {
            wait_readable(bound.listener.as_raw_fd(), 200);
            while let Some(stream) = next_connection(&bound.listener) {
                let Some((id, evidence, stream)) =
                    admit(stream, uid, (&open, opts.max_connections), &mut next_id)
                else {
                    continue;
                };
                let (shared, open) = (&shared, &open);
                scope.spawn(move || {
                    serve_one(stream, evidence, shared);
                    open.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
                });
            }
        }
        // stopping: no new connections; close every open one so its session ends, then the scope joins the threads
        for s in open.lock().unwrap_or_else(|e| e.into_inner()).values() {
            let _ = s.shutdown(Shutdown::Both);
        }
    });
    // the gateway's owner stops and closes the store when the last handle goes; the socket file goes with `bound`
    drop(shared);
    drop(gateway);
    drop(bound);
    eprintln!("klatch-server: daemon stopped");
    Ok(())
}

fn write_frame(stream: &mut UnixStream, frame: &ServerFrame) -> io::Result<()> {
    stream.write_all(encode_line(frame).as_bytes())
}

fn refusal(rule: &str, reason: &str) -> ServerFrame {
    ServerFrame::Refusal {
        v: PROTOCOL_VERSION,
        request: None,
        rule: rule.into(),
        reason: reason.into(),
    }
}

/// One refusal frame to a connection over the cap, then close. A peer that does not read cannot hold the daemon: the
/// write has a short timeout.
fn refuse_excess(mut stream: UnixStream) {
    let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
    let _ = write_frame(
        &mut stream,
        &refusal(
            "daemon.connections",
            "the daemon is serving as many connections as it allows; try again shortly",
        ),
    );
    let _ = stream.shutdown(Shutdown::Both);
}

/// The first line of a connection: at most `cap` bytes up to and including the newline. `None` when the peer closed
/// before sending anything; `InvalidData` when the line is longer than `cap`.
fn first_line(reader: &mut impl BufRead, cap: usize) -> io::Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    reader.take(cap as u64 + 1).read_until(b'\n', &mut line)?;
    if line.is_empty() {
        return Ok(None);
    }
    if line.len() > cap && line.last() != Some(&b'\n') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "a frame is over the size limit",
        ));
    }
    Ok(Some(line))
}

fn timed_out(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}

/// What a first request asked for.
enum First {
    /// An `identify` request.
    Identify { id: String, params: Value },
    /// Anything else: an ordinary first request of the session.
    Other,
}

fn classify(line: &[u8]) -> First {
    let Ok(v) = serde_json::from_slice::<Value>(line) else {
        return First::Other;
    };
    let is_identify = v["type"] == "request" && v["method"] == "identify";
    match v["id"].as_str() {
        Some(id) if is_identify => First::Identify {
            id: id.to_string(),
            params: v["params"].clone(),
        },
        _ => First::Other,
    }
}

/// The principal an `identify` names, or the reason it is refused (never the name or the token).
fn identify(params: &Value, backend: &SharedBackend) -> Result<Principal, String> {
    let text = |k: &str| params.get(k).map(|v| v.as_str());
    match (text("name"), text("member"), text("token")) {
        (Some(Some(name)), None, None) => LocalIdentity
            .resolve(&PeerEvidence::DeclaredName(name.to_string()))
            .map_err(|e| e.to_string()),
        (None, Some(Some(member)), Some(Some(token))) => {
            let verified = backend
                .verify_credential(member, token)
                .map_err(|e| e.to_string())?;
            // the principal is built the way every credential becomes one
            LocalIdentity
                .resolve(&PeerEvidence::Credential {
                    member: verified.member,
                })
                .map_err(|e| e.to_string())
        }
        _ => Err("identify takes a name, or a member and a token".into()),
    }
}

/// The principal of a connection that did not identify itself: what the evidence table gives a local process nobody
/// registered, `unmapped-process`, unattributed. Typed names go through `DeclaredName`, the same rules as the CLI.
fn unidentified(evidence: &PeerEvidence) -> Principal {
    EvidenceTable::new()
        .resolve(evidence)
        .unwrap_or_else(|_| Principal::unattributed("unmapped-process"))
}

/// The backend of a connection that never identified itself (#1096): when the kernel's pid for the peer runs inside a
/// verified Claude Code session the walk up the process tree finds (the test `klatch` makes for a command), its `join`
/// is judged as Claude Code, so a room with a harness requirement admits it. Everything else is the generic floor: a
/// connection that identified with a typed name or a credential (a name proves nothing, #881; a credential proves a
/// member and not a harness, #880), a peer with no verified session above it, and any doubt. The harness is learned
/// from the pid alone, never from anything the peer sent.
fn unidentified_backend(shared: &Shared, evidence: &PeerEvidence) -> SharedBackend {
    match evidence {
        PeerEvidence::Process { pid, .. } if (shared.claude_code_at)(*pid) => {
            shared.backend.clone().with_harness(|| {
                Box::new(klatch::harness::ClaudeCode::probing(
                    klatch::harness::wake_support_here,
                ))
            })
        }
        _ => shared.backend.clone(),
    }
}

fn serve_one(stream: UnixStream, evidence: PeerEvidence, shared: &Shared) {
    let _ = serve_inner(stream, &evidence, shared);
}

/// The first line of a connection, or `None` when the connection is over before one came: closed, too large (told so),
/// or silent for `first_frame`.
fn read_first(
    out: &mut UnixStream,
    reader: &mut BufReader<UnixStream>,
) -> io::Result<Option<Vec<u8>>> {
    match first_line(reader, MAX_LINE_BYTES) {
        Ok(line) => Ok(line),
        Err(e) if e.kind() == io::ErrorKind::InvalidData => {
            let big = "a frame is over the size limit; the connection is closed";
            write_frame(out, &refusal("frame.too_large", big))?;
            Ok(None)
        }
        // nothing arrived in time, or the peer is gone
        Err(e) if timed_out(&e) => Ok(None),
        Err(e) => Err(e),
    }
}

/// The `response` to an `identify` request: who the connection is now, or why it is refused.
fn identify_answer(id: String, outcome: Result<&Principal, String>) -> ServerFrame {
    match outcome {
        Ok(who) => ServerFrame::Response {
            v: PROTOCOL_VERSION,
            id,
            ok: true,
            result: Some(json!({
                "member": who.member,
                "attribution": match who.attribution {
                    klatch::domain::Attribution::Verified => "verified",
                    klatch::domain::Attribution::Unattributed => "unattributed",
                },
            })),
            error: None,
        },
        Err(message) => ServerFrame::Response {
            v: PROTOCOL_VERSION,
            id,
            ok: false,
            result: None,
            error: Some(ErrorBody {
                code: "identity.refused".into(),
                message,
            }),
        },
    }
}

/// A connection whose first request is `identify`: answer it, then run the session as who it named (or end it).
fn serve_identified(
    (id, params): (String, Value),
    shared: &Shared,
    (stream, mut out, reader): (UnixStream, UnixStream, BufReader<UnixStream>),
    closer: klatch::session::Closer,
) -> io::Result<()> {
    let who = match identify(&params, &shared.backend) {
        Ok(who) => who,
        Err(message) => {
            let answer = identify_answer(id, Err(message));
            out.write_all(encode_line(&answer).as_bytes())?;
            let _ = stream.shutdown(Shutdown::Both);
            return Ok(());
        }
    };
    // written before the session starts: the answer is the first thing after the hello
    out.write_all(encode_line(&identify_answer(id, Ok(&who))).as_bytes())?;
    let session = Session::over(&shared.backend, &who, &shared.config).without_hello();
    session.serve_guarded(reader, out, Some(closer))
}

fn serve_inner(stream: UnixStream, evidence: &PeerEvidence, shared: &Shared) -> io::Result<()> {
    let mut out = stream.try_clone()?;
    let started = unidentified(evidence);
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    write_frame(&mut out, &hello_frame(&started))?;
    stream.set_read_timeout(Some(shared.first_frame))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let Some(first) = read_first(&mut out, &mut reader)? else {
        return Ok(());
    };
    stream.set_read_timeout(None)?;
    stream.set_write_timeout(None)?;
    let closer = {
        let c = stream.try_clone()?;
        Box::new(move || {
            let _ = c.shutdown(Shutdown::Both);
        }) as klatch::session::Closer
    };
    match classify(&first) {
        First::Other => {
            let input = Cursor::new(first).chain(reader);
            let backend = unidentified_backend(shared, evidence);
            let session = Session::over(&backend, &started, &shared.config).without_hello();
            session.serve_guarded(input, out, Some(closer))
        }
        First::Identify { id, params } => {
            serve_identified((id, params), shared, (stream, out, reader), closer)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whether `join gated` is admitted on the backend `unidentified_backend` gives a peer, with `probe` standing in for
    /// the process-tree walk. The room requires a capability only a verified harness has.
    fn joins_gated(probe: fn(u32) -> bool, evidence: &PeerEvidence) -> bool {
        use klatch::gateway::{AllowAll, Gateway, SystemClock};
        let dir = PathBuf::from(format!(
            "/tmp/kh{}-{:?}",
            std::process::id(),
            probe as usize
        ));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("rooms.db");
        let gw = SharedGateway::spawn(move || {
            let store =
                klatch::store::SqliteStore::open(&path, &klatch::store::insecure_test_key())?;
            let g = Gateway::new(store, SystemClock, AllowAll);
            g.admin_set_room_require(
                &Principal::unattributed("admin"),
                "gated",
                Some("refuse:session-identity"),
            )?;
            Ok(g)
        })
        .unwrap();
        let shared = Shared {
            backend: gw.session_backend(),
            claude_code_at: probe,
            first_frame: Duration::from_secs(1),
            config: SessionConfig::default(),
        };
        let backend = unidentified_backend(&shared, evidence);
        let who = Principal::unattributed("unmapped-process");
        let script = "{\"v\":1,\"type\":\"request\",\"id\":\"j\",\"method\":\"join\",\"params\":{\"room\":\"gated\"}}\n";
        let mut out = Vec::new();
        Session::over(&backend, &who, &shared.config)
            .without_hello()
            .serve(Cursor::new(script.as_bytes().to_vec()), &mut out)
            .unwrap();
        drop((backend, shared, gw));
        let _ = std::fs::remove_dir_all(&dir);
        String::from_utf8(out).unwrap().contains("\"ok\":true")
    }

    /// #1096: only a peer whose pid the walk verifies as Claude Code gets the harness; every other peer is the floor.
    #[test]
    fn a_peer_in_a_verified_claude_code_session_joins_as_claude_code_and_any_other_as_the_floor() {
        let peer = PeerEvidence::Process { pid: 77, uid: 501 };
        assert!(joins_gated(|pid| pid == 77, &peer));
        assert!(!joins_gated(|_| false, &peer));
        // the probe is asked about the kernel's pid and about nothing else
        assert!(!joins_gated(
            |pid| pid == 77,
            &PeerEvidence::Process { pid: 78, uid: 501 }
        ));
        assert!(!joins_gated(
            |_| true,
            &PeerEvidence::DeclaredName("claude".into())
        ));
    }

    /// A fake credential, as the kernel would report it: this is the whole of the uid check, and a connection from
    /// another uid cannot be made in a test.
    #[test]
    fn only_the_daemons_own_uid_is_accepted_and_the_pid_is_evidence_not_a_name() {
        let own = 501;
        let ev = local_evidence(
            PeerCred {
                pid: 4242,
                uid: 501,
            },
            own,
        )
        .unwrap();
        assert_eq!(
            ev,
            PeerEvidence::Process {
                pid: 4242,
                uid: 501
            }
        );
        for other in [0, 1, 500, 502, u32::MAX] {
            assert_eq!(
                local_evidence(
                    PeerCred {
                        pid: 4242,
                        uid: other
                    },
                    own
                ),
                Err(WrongUid),
                "uid {other}"
            );
        }
        // root is another user too, when the daemon is not root
        assert_eq!(
            local_evidence(PeerCred { pid: 1, uid: 0 }, own),
            Err(WrongUid)
        );
        // whatever the pid, the principal is the same unmapped name: the pid never becomes a name
        for pid in [1, 4242, 0] {
            let ev = local_evidence(PeerCred { pid, uid: own }, own).unwrap();
            let p = unidentified(&ev);
            assert_eq!(p.member, "unmapped-process");
            assert_eq!(p.attribution, klatch::domain::Attribution::Unattributed);
        }
    }

    #[test]
    fn a_directory_must_be_the_daemons_and_closed_to_group_and_others() {
        let d = Path::new("/tmp/x");
        assert!(directory_problem(d, 501, 0o40700, 501).is_none());
        assert!(directory_problem(d, 501, 0o40750, 501).is_none());
        assert!(directory_problem(d, 501, 0o40755, 501).is_none());
        for mode in [0o40777, 0o40770, 0o40707, 0o41777, 0o40722] {
            let p = directory_problem(d, 501, mode, 501).unwrap();
            assert!(
                p.contains("not safe") && p.contains("writable"),
                "{mode:o}: {p}"
            );
        }
        let p = directory_problem(d, 0, 0o40700, 501).unwrap();
        assert!(
            p.contains("not safe") && p.contains("owned by uid 0"),
            "{p}"
        );
    }

    #[test]
    fn a_socket_path_longer_than_the_limit_is_refused_and_the_limit_is_named() {
        let limit = path_limit();
        assert!((90..=108).contains(&limit), "{limit}");
        let exact = "a".repeat(limit);
        assert!(path_problem(Path::new(&exact), limit).is_none());
        let long = "a".repeat(limit + 1);
        let p = path_problem(Path::new(&long), limit).unwrap();
        assert!(
            p.contains("too long") && p.contains(&format!("{limit} bytes")),
            "{p}"
        );
    }

    #[test]
    fn the_first_line_is_capped_and_a_closed_peer_has_none() {
        let mut r = Cursor::new(b"abc\nrest".to_vec());
        assert_eq!(first_line(&mut r, 10).unwrap().unwrap(), b"abc\n");
        let mut r = Cursor::new(Vec::new());
        assert!(first_line(&mut r, 10).unwrap().is_none());
        let mut r = Cursor::new(vec![b'x'; 100]);
        assert_eq!(
            first_line(&mut r, 10).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        // exactly the cap, with its newline, is fine
        let mut line = vec![b'x'; 9];
        line.push(b'\n');
        let mut r = Cursor::new(line);
        assert!(first_line(&mut r, 10).unwrap().is_some());
    }

    #[test]
    fn identify_is_recognised_only_as_a_well_formed_request() {
        assert!(matches!(
            classify(
                br#"{"v":1,"type":"request","id":"a","method":"identify","params":{"name":"x"}}"#
            ),
            First::Identify { .. }
        ));
        for other in [
            &br#"{"v":1,"type":"request","id":"a","method":"post","params":{}}"#[..],
            br#"{"v":1,"type":"request","method":"identify"}"#,
            br#"{"v":1,"type":"ack","room":"r","seq":1}"#,
            b"garbage",
        ] {
            assert!(matches!(classify(other), First::Other));
        }
    }

    #[test]
    fn options_need_a_socket_and_take_two_limits() {
        assert!(parse(&[]).is_err());
        assert!(parse(&["--socket"]).is_err());
        assert!(parse(&["--socket", "/tmp/a", "--bogus"]).is_err());
        assert!(parse(&["--socket", "/tmp/a", "--max-connections", "0"]).is_err());
        let o = parse(&[
            "--socket",
            "/tmp/a",
            "--max-connections",
            "3",
            "--first-frame-secs",
            "2",
        ])
        .unwrap();
        assert_eq!(o.max_connections, 3);
        assert_eq!(o.first_frame, Duration::from_secs(2));
    }
}
