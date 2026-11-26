# Security

klatch is a **public local-only pre-release**: a scoped MVP, **not a 1.0**, with **no outside security review**
(planned for the first unmarked release, #167). **Do not put sensitive data in it.** It runs on one machine for one
person and their agents. Remote transports (TLS, mutual TLS, an HTTPS front end, a reverse proxy, WebSocket, gRPC,
exposing `klatch-server` to other machines) are not part of this release (an experimental mutual TLS listener and a WebSocket
route on the listeners that exist are in the source, off unless you configure them, unreviewed and unsupported); the README lists what is in and out of scope.
Supported versions: the newest release only, the current practice and not a policy (#460).

## What is protected

- **At rest.** The store is encrypted with SQLCipher (AES-256, authenticated per page) under a random 256-bit key. A
  copied file, a leaked backup or a synced folder shows no message, room name, sender name or time, and a changed page
  is detected. A database from an earlier release is converted in place on first use, checked, and the plaintext copy
  is overwritten and removed.
- **Keys.** On macOS the key is in a file readable only by you (the Keychain asks for a password whenever a program it
  has not seen reads an item, and an unsigned build is a new program each time, #480; an older login Keychain item is
  read once, within 5 seconds, to migrate an install that predates 2026-11-14, and never written again). On Linux it is
  in the Secret Service where there is one, or in a file readable only by you (unattended workers, and machines without
  a keystore). On Linux, what was tested (in containers, Debian 12, aarch64, `proto/linux-keys/run.sh`): GNOME Keyring
  through `secret-tool` on a session bus works, including replacing and deleting a key, and the key never appears on a
  command line; with no session bus, or an address that points nowhere, klatch keeps the key in a file in a 0700
  directory with mode 0600 and warns (`klatch doctor` shows the warning); a locked keyring with no display to unlock it
  fails within seconds with a reason and writes no key file, and so does a bus with no login collection (both read as a
  locked-keyring error, never as "no key"). Not tested: KWallet, KeePassXC, x86_64, a graphical unlock
  prompt (#563). The key is never put in the environment, on a command
  line, in a log or in a crash report, and it is wiped from memory when dropped. A recovery code
  (`klatch admin recovery`) restores it; with neither the keystore entry nor the code the data is gone.
- **Administering, launching and the keystore (the admin passphrase, #1233).** Every `klatch admin` command, every
  launcher command (`worker`, `anchor`, `run`, `judge`) and every act on the key or the store file (`recovery`,
  `recover`, `seal-label`, `rotate-key`, `rotate-label-key`, `backup`, `restore`, `export`, the archive commands) asks
  for the admin passphrase, typed at a real terminal with echo off. `klatch admin init-passphrase` sets it (twice, at
  least 10 characters); only an Argon2id verifier (19 MiB, 2 passes, the parameters recorded in the stored value so
  they can be raised) is kept, in the store's policy, never the passphrase. It is never read from the environment,
  an argument, a file or a pipe: standard input counts only when it is a terminal, else the controlling terminal.
  A process with no terminal at all (a detached or double-forked process, a script with no controlling terminal and
  none on standard input) has nothing to ask on and is refused. `setsid` alone is not a defence: a program that
  gives itself a pseudo-terminal of its own can be asked like anyone and is then refused for not knowing the
  passphrase. A program started from YOUR terminal shares it, so it can still make a prompt appear there even when
  it pipes its own standard input; it cannot answer it, and it cannot confirm for you (the typed-back confirmation
  of a destructive act is read from the terminal, never from a pipe) or feed `klatch admin shell` (which refuses
  unless standard input is itself a terminal). The prompt names the exact command it approves (`admin close
  lobby`; the shell says it opens an unlock window for ALL admin and launcher commands): read it, and type the
  passphrase only for a command you asked for.
  Commands that print a secret or stored text (`admin recovery`, `export`, `run-archive`, `rotate-key`,
  `rotate-label-key`, `audit`, `refusals`, `review-queue`, `scrub-list`, `archives`, `stats`) refuse before any prompt unless
  three things hold: standard output is this process's controlling terminal; it is the SAME terminal device (compared
  with `fstat`: device, inode, terminal) as the one the passphrase and the confirmation are read from and written to;
  and standard error is that terminal or `/dev/null`. `klatch admin shell` needs standard input and output to be one
  controlling terminal. What this stops: output sent to a pipe, a file or a pty that is not the terminal you are
  typing at, including a pty a program made in a session of its own while your terminal supplies the prompt. What it
  does not stop: a program that hosts or relays the terminal you type at, so that the prompt, your typing and the
  output all pass through it. `recovery` also asks for a typed confirmation that names what it confirms. Text from arguments is shown as printable ASCII only (everything else
  as `?`), a recovery code is never shown (decided from the parsed `recover` command, whatever flags come before it), and the prompt names the
  store file. Residual, said plainly: a passphrase or recovery code typed into a terminal that a program hosts or
  relays (a pty it owns and shows you, a screen it forwards) is captured, and these checks cannot prevent a relay of
  the whole terminal. The defence that remains is you reading what the prompt names and typing the passphrase only
  for a command you asked for. A worker or agent started from your terminal can cause the prompt to appear but cannot
  answer it: the prompt names the exact command and you decide (#1276; the old worker and agent refusals went with
  #1233, and the passphrase is the proof). A program that hosts or relays the whole terminal is the documented residual
  above (#1261 covers the other stated limits). While `klatch admin shell` is open, a program that shares its terminal
  may inject keystrokes (TIOCSTI, where the OS allows it) and run admin and launcher commands without a prompt; the
  60 second default window keeps that short, and klatch does not probe for TIOCSTI (#1280). `worker issue` writes a bundle to the path it is given and is not covered (#1282); `anchor github`
  reads its events from standard input and the prompt cannot show them (#1279); an open `klatch admin shell` window
  on a shared terminal is exposed to injected keystrokes (#1280; see below). A wrong passphrase is charged before it is checked, so many processes guessing at once each cost one:
  three are free, then 30 seconds, doubling, up to 15 minutes, and the right one is refused while the wait runs. The count is kept per controlling terminal (#1274): a program with a pseudo-terminal of its own that fails on purpose locks only that terminal, not yours. A program that opens new terminals gets a counter of its own each time, so the limit slows guesses from one terminal and does not bound a program that makes many. What makes that visible is an alarm (#1310), never a lockout: when 50 failures (passphrase or recovery code) fall inside a rolling 24 hours across all terminals, the proof path writes one `proof-failed-many` audit row (the count, the number of distinct terminals and the window; nothing typed) and `klatch doctor` warns while the count stays at or above 50. It locks nothing and adds no delay, so a program cannot use it to lock you out; it only means a slow guesser is seen. Said plainly: a recovery code carries 256 bits, so the alarm detects misbehaviour and is not a defence against guessing; the defence is the size of the code. Wrong guesses at the recovery code of `recover` and `restore` on an unreadable store (#1552) are kept in a file beside the store and reach the audit, and so the alarm, only after a later recovery succeeds without the store and the store then opens; guesses that never succeed are never counted, stay visible only in that file, and `klatch doctor` does not read it. The
  audit keeps `proof-ok`, `proof-failed` with a count, and the other rows, never an attempt. The comparison is
  constant time. Once proved, a command is the principal `human` (`Verified`, a name no member may take), and a guard
  that accepts nothing else decides every act; the admin agent role is unchanged (it acts inside its allowlist and
  never edits a floor). One `klatch` command is one process, so each command asks. `klatch admin shell` is the one
  long-lived process: it asks once and holds an unlock window (60 seconds by default, `klatch admin unlock-window`
  sets 10 to 1800 seconds) in its own memory and nowhere else; no file, environment variable or token carries it, a
  child or another `klatch` starts locked, and changing the passphrase voids it. If you forget the passphrase,
  `klatch admin reset-passphrase` takes the store's recovery code (`klatch admin recovery` shows it; keep it) and sets
  a new one; whoever holds that code can already restore the key and read the store, so it gives them nothing more.
  The verifier lives in the encrypted store, so when the key is lost (or the store file is) it cannot be read:
  `klatch admin recover` (the store key) and `klatch admin restore` are then approved by the recovery code (given
  as the argument for `recover`, and it must open the store file; typed for `restore`) and the typed confirmation
  instead of the passphrase, and the command says so. `recover` counts only when the code opens an existing encrypted store file (no store, an empty or a plaintext one is refused), and the `restore` prompt names the backup file. Whoever holds the code can already do both. These attempts have no failure limit (#1281).
  Upgrading an existing install: until a passphrase is set every admin, launcher and keystore command refuses and
  says to run `klatch admin init-passphrase`; run it once, right after upgrading, in a terminal of your own. The old
  terminal and process-tree check is gone, not kept as a fallback.
- **Crash output.** A panic prints where it happened and nothing it carried; crashes leave no core file; sorting and
  indexing use memory, not temporary files.
- **On the wire.** Remote transports are not part of this release. On one machine klatch uses stdio, a Unix socket and,
  for tools that need it, loopback HTTP with a bearer credential; none of these is encrypted, none leaves the machine,
  and a non-loopback listener refuses to start (`proto/klatch-server/src/loopback.rs`, tested in
  `proto/klatch-server/tests/http_wire.rs`).
  - **stdio and the Unix socket** rely on operating-system permissions: inherited pipes, and a socket in a directory only
    you can write to with mode 0600, where the daemon checks the peer's user id. Prefer these.
  - **Loopback HTTP and MCP over HTTP** (`klatch-server http`) are plaintext TCP on 127.0.0.1 or ::1. Any local user can
    reach the port, so the defence is the credential: every request needs `Authorization: Bearer` and `X-Klatch-Member`
    (a worker credential the human issued, checked live), there is no unauthenticated tier, and forwarded headers and
    peer addresses grant nothing (`proto/klatch-server/tests/route_audit.rs`). It is the weakest local route: use it
    only for tools that cannot use stdio or the socket, and never forward or proxy the port. Exposing it other than
    through ssh or mutual TLS (`clients/recipes/behind-a-tunnel.md`) is unsupported and unprotected.
- **Human proof for administration.** Admin, launcher and keystore acts ask for the admin passphrase typed at the terminal
  (#1233); the bullet "Administering, launching and the keystore" above says how it works and what it does not stop.
- **In code.** Every public gateway operation is listed and a test fails if one reaches the store without asking the
  guard. Dependencies are checked for licence, advisories and sources, and a list of weak or home-grown crypto crates
  is refused.
- **No telemetry, no update check, no outbound calls from the `klatch` binary.** `klatch` depends on no networking crate,
  binds and connects to no socket, and starts only `git` (to find a repository), `ps`, `secret-tool` and `security` (the keystores), plus `claude`
  for its version and, only if you run `klatch judge run --hosted-judge` yourself, for the opt-in model judge described
  under "Channel-safety screening" below. A
  test (`proto/klatch/tests/no_telemetry.rs`) pins this by inspecting the dependency tree and the source; it is an audit
  of what the code could do, not a capture of what one run did. Only the installer, which you run yourself, and your
  package manager contact the network (GitHub); the documents never tell you to pipe a download into a shell.
- **`klatch-server` is a second binary with a second, different claim.** It ships next to `klatch` in every archive,
  wheel and Homebrew install, and it does open sockets, so the claim above does not cover it. Its claim is:
  klatch-server opens loopback listeners only, plus one mutual TLS listener that exists only when you name your own
  certificates, starts no program and makes no outbound connection. A non-loopback bind of the plaintext listener
  (`http --listen 0.0.0.0:0`) is refused before anything listens, a Unix socket (`daemon --socket`) is mode 0600 and
  closes a peer of another user, and no HTTP client is in its dependency tree. The network crates it links are rustls
  (with ring), for `klatch-server https` (#108), and the frame layer of tungstenite for the WebSocket route (below), which is experimental and unreviewed: mutual TLS is mandatory (a client
  without a certificate from your CA file is refused at the handshake), there is no plaintext mode and no way to skip a
  check, the key file must be mode 0600, the listener is loopback unless you pass `--allow-off-loopback`, and a certificate
  gates the port only (the caller is still the worker credential). klatch makes no certificates and runs no CA.
  **WebSocket (#112, experimental, unreviewed):** `GET /v1/ws` on those same two listeners carries the same wire as text
  messages. It adds no listener (so `ws://` exists only on the loopback listener and a WebSocket off loopback is always the
  mutual TLS one) and no network stack: the frame layer of `tungstenite` (no handshake, no HTTP parser, no runtime) and
  ring's SHA-1 for the upgrade answer, which RFC 6455 requires and which protects nothing. The caller is the worker
  credential at the upgrade (the one `401` otherwise), then the gateway's live check on every request; a request with an
  `Origin` header is refused (`403`) unless you named that exact origin with `--ws-allow-origin` (off by default), which is
  what keeps a page on another site, or a DNS rebinding page, from opening a connection to a loopback port. Frames must be
  masked, a message is at most 1 MiB, binary is refused, and every wait is bounded (a ping every 20 s, a close for 60 s
  of silence, 2 s for a close). One credential cannot hold every connection slot: a member may hold 4 WebSocket
  connections (`--ws-max-per-member`, 1 up to `--max-connections`), the refusal being the `503` a full server gives anyone
  (it names nobody), and a connection ends after an hour whatever it does (`--ws-max-age-secs`, 60 to 86400; a client
  reconnects and resumes with `from_seq`). A socket whose credential is revoked or expires is closed (1008) when the
  gateway refuses its next request or poll. `GET /v1/stream` has the same two limits, counted apart from the WebSocket's and counted under the
  member the verified credential names (never a header or a name the client chose): at most `--stream-max-per-member`
  streams per member (default 4, 1 up to `--max-connections`; the same `503` that names nobody) and a stream ends after
  `--stream-max-age-secs` (default 3600, 60 to 86400; the client reconnects and resumes with `from_seq`) (#1517). Before
  the credential is verified only the global cap and the head limits apply (#1493). `POST /v1/frames` and `POST /mcp` are
  one exchange under a 5 second deadline and are not capped per member. The per-member `503` is sent after the request
  head and the credential check and lingers up to 1 s, so its timing differs from the global `503` (visible only to the
  credential holder). A peer that can reach the port can still hold connections up to the cap, as for the other routes.
  **Files.** The server certificate file and the client CA file are opened as the key and CRL files are: not through a symbolic link (name the real file; a symlinked CA file or certificate is refused, which is a change from earlier builds), without blocking (a FIFO is refused, it cannot hold start or a reload), a regular file of at most 4 MiB. The client CA file is a trust anchor, so it must also be owned by the server's user or root and not writable by group or other, or the server does not start (and a reload keeps the old configuration): whoever can write it chooses who may connect. **Revocation.** OCSP is never checked. Without `--tls-client-crl` no revocation list is checked: a client certificate stays
  accepted until it expires or the CA file is replaced (and the server reloaded with SIGHUP or restarted), so revocation means short-lived
  certificates or a new CA. With `--tls-client-crl FILE` (PEM, one or more CRLs signed by the client CA, read at start and on SIGHUP; #1491)
  a client certificate whose serial the CRL lists fails the handshake on every route (one verifier, one listener): the
  client sees a fatal TLS alert (`certificate revoked`) and never an HTTP byte, no session is made, and the server logs
  `client certificate refused: revoked` (at most one line every ten seconds, never a name or serial). The file must be a
  regular file, not a link, owned by the server's user or root, not writable by group or other, at most 4 MiB, valid PEM
  and hold 1 to 16 CRLs, or the server does not start; **one CRL per issuer: the verifier uses the first CRL it finds for an
  issuer, so a file with two CRLs of one issuer is refused at start (replace the file, never append: an older list ahead
  of a newer one would ignore the new revocations);** the option needs `--tls-client-ca`. **Expiry is enforced:** after a CRL's `nextUpdate` every client certificate its
  issuer signed is refused until the file is replaced and the server reloaded or restarted (a log line at start says so, and every
  refusal logs that the CRL has expired; a CRL with a bad signature, an unsupported signature algorithm or an issuer
  certificate that may not sign CRLs refuses its issuer's certificates too, with its own log line), which locks clients out loudly and never leaves a revoked certificate working on
  a stale list. **Unknown status is allowed:** a certificate no configured CRL covers (another issuer, an intermediate CA
  with no list from its parent) is accepted as it is without the option; deny would refuse the intermediate CAs of a
  chain. Start-up checks (#1614): the server reads each CRL's issuer and times at start with a
  small bounded DER reader (`der.rs`: definite minimal lengths, no recursion, nothing allocated, no dependency, no unsafe
  code) and **refuses to start** (in words naming the file and the CRL's number, never its content) on a CRL whose issuer
  or times cannot be read, **that has no `nextUpdate`** (optional in X.509 but the verifier refuses such a CRL, so the
  server says so at start rather than failing later), or whose times are before 1970 (strict UTCTime or GeneralizedTime,
  UTC), and on a system clock before 1970. It only **warns**, at start, in a fixed line with the file and the CRL's number
  (no subject, no serial), and still starts: when a CRL's `nextUpdate` has passed (expiry is enforced, so its issuer's
  clients are refused), when it passes within 7 days, and when a CRL's issuer is the subject of no CA in the client CA file
  (such a CRL applies only to clients that present an intermediate CA with that name, so it is how an intermediate's CRL
  works with a root-only CA file; if that is not what you meant it does nothing). There is no periodic re-check (reload is by SIGHUP only). Two CRLs
  of one issuer with different issuing distribution points (a partitioned CRL, valid for webpki) are refused by the
  one-CRL-per-issuer rule: the server fails closed. **Reload (#1492).** SIGHUP re-reads the server certificate chain and key,
  the client CA file and the client CRL file with exactly the checks of start (key a regular file, not a link, owned by the
  server's user, mode 0600; certificate and key fit; the CA file holds a certificate; every CRL check above, owner, caps,
  one CRL per issuer, `nextUpdate` rules, and the CRL warnings print again), builds the whole new rustls configuration, and
  replaces the old one with one pointer swap under a lock only if everything passed; on any failure the old configuration
  stays and one fixed log line names the kind of failure (never a file's content, a subject or a serial). New connections
  use the new configuration, connections already open keep theirs and are not cut (a certificate revoked by the new CRL is
  refused at its next handshake, not mid-stream. A revoked certificate keeps an open WebSocket usable for reads and writes until
  the worker credential is revoked (a socket whose credential is revoked is closed) or `--ws-max-age-secs` runs out, and an
  open event stream keeps receiving events; its writes go through `POST /v1/frames`, which needs a new handshake and is
  refused. Stop the server to cut every open connection). **A reload refuses a CRL file whose `nextUpdate` has passed while the
  CRL in force is still in date** (failure kind `client CRL file`, the old configuration stays): CRLs build on each other, so
  an expired file is almost always older than the one in force, refusing it loses no revocation, and adopting it would lock
  every client of its issuer out; the expiry is logged as an `ALERT` line. When the CRL in force has expired too the file is
  taken and the line says `ALERT`. At start nothing older exists, so an expired CRL still starts the server with a warning.
  **Replace the CA and CRL files by rename, never by rewriting in place:** a rewrite cut between PEM blocks parses as valid
  with fewer CRLs or CAs. The build runs on its own thread with a 5 second bound; stopping the server abandons a build
  that is still running. Signals are coalesced (at most one reload running,
  one pending, one start per half second), the handler only sets a flag, flags and file names are not re-read (no new
  address or limit), and the plain `http` server catches SIGHUP and ignores it (it survives closing its terminal and keeps its port). **Denial of service:** any peer that can reach the port
  can open connections without a certificate. A handshake deadline (3 s), a cap on connections in the handshake (a
  quarter of `--max-connections`) and a stall deadline (60 s, never under 30 s) bound it, and each has an upper bound
  the server enforces at start. When the cap is full, a newcomer is refused if every handshake in progress is younger
  than half the deadline (the newcomer loses; an honest client mid-handshake is never closed to make room), and
  otherwise takes the place of the oldest (the oldest, silent or crawling, loses). A flood that keeps the cap full of
  fresh connections still keeps newcomers out, and an honest client that is slow past half the deadline can be evicted,
  so the cap bounds a flood and does not remove denial of service by volume. **The real cost of that is low:** a peer
  with no certificate that keeps the cap full of fresh connections needs only a few connections per second (a reviewer
  starved 10 of 10 honest clients at a cap of 4 with about 3.3 per second; with the defaults, a cap of 8, a 3 s deadline
  and a 1.5 s minimum age, roughly 5 to 6 per second). It was equally cheap before the age rule, which only protects a
  client already mid-handshake from eviction. The remedies are a firewall, a tunnel or loopback (the listener is
  loopback unless `--allow-off-loopback` is given), and the per-source limit below.
  **Pre-handshake limits (#1493).** A connection that has not finished the handshake takes no place among the
  sessions: the handshake pool and the session cap (`--max-connections`, counted after the handshake) are separate, so
  silent sockets cannot take the place of a client that has a certificate. One source address (an IPv4 /32, an IPv6 /64;
  an IPv4-mapped IPv6 address counts as its IPv4 address) may have `--handshakes-per-source` handshakes in progress
  (default 2, at most `--max-handshakes`); a newcomer over that share is closed without a byte, unless the source's own
  oldest handshake is at least half the deadline old, which then gives way. The per-source table holds one entry per
  handshake in progress, so it is never larger than the pool, and an entry goes when its handshake ends (completes,
  fails, times out, resets or is evicted). A peer that trickles a byte at a time is cut by the deadline (the deadline is
  checked on every read, which it was not before #1493). Refusals are logged as one line with a count and no address, at
  most every ten seconds. **What this protects, and what it does not.** It protects OTHER sources and the sessions that
  are already open: silent sockets take no session place, and one address cannot fill the pool alone. It does NOT protect
  a client that shares an address with the attacker (a NAT, a shared host, a shared IPv6 /64): about two connections per
  second from that address keep its share full, and that client can be locked out cheaply. Measured on the real binary,
  from the attacker's own address: with 50 silent sockets held, a client retrying every 100 ms got in after 1.56 s; against
  an attacker opening 2 silent connections per second 8 of 58 attempts succeeded, at 4 per second 3 of 75, and against one
  that reconnects as soon as it is closed 0 of 60; a client that does not retry fails in every case. Nor does the share
  stop a flood from MANY addresses: once the pool is full, a newcomer of ANY source closes the oldest handshake that is old
  enough (any source's), so the churn attack described above works against everyone with a handful of IPv4 addresses or
  one IPv6 /48 (each /64 is a separate source). Clients behind one address share one share. The remedies stay a
  firewall, a tunnel or loopback.
  `proto/klatch-server/tests/no_network.rs` pins this by inspecting the dependency tree and the source (an audit of what
  the code could do, like the one for `klatch`), and every release runs `release/check-packaged-server.sh` on the
  packaged binary: it must print its version, refuse a `0.0.0.0` bind, and start and stop a daemon on a socket. It is
  not published on crates.io; it comes from the archives, the platform wheels and Homebrew.

## What is not protected (read this)

- **A hostile process running as you.** It can read the key (the macOS item is readable by any program of yours so
  that rebuilt, unsigned binaries do not prompt; the `security` tool reads it too), read or attach to a running
  klatch, and read any key file. Encryption cannot help once the attacker can ask for the key. Root can do anything.
- **What the admin passphrase does not stop.** It proves the caller knows a secret typed at a terminal; it does not
  prove the caller is a person. Out of scope, as for every secret typed on a machine: a process that watches your
  keyboard (a keylogger, a terminal you share, an agent that reads your terminal's screen or scrollback) and a process
  that reads the memory of an unlocked `klatch admin shell`. Also not stopped: a process of yours that does not use
  the `klatch` command line, but opens the store with the key it can read and rewrites the verifier or the failure
  count itself (#1261); stronger options for that (a Keychain item with user presence, a separate OS user) stay
  open in #318. An install that has not set a passphrase has an unauthenticated first set: until you run
  `klatch admin init-passphrase`, a program with a terminal could set it first (it is refused with no terminal and
  from inside a detected coding-agent session, and audited; #1262). Set it before anything else.
- **Identity.** A member name is as strong as the process that typed it. A clearance rests on that name, so it is as
  spoofable as the name unless the member holds a worker key bundle. A process with no key bundle acts as Unattributed
  under a typed or discovered name, by design (#161). A key bundle proves a member, not its harness: workers, wire joins
  and a typed name inside Claude Code are the generic floor, so a room that requires a capability refuses them (#880,
  #881, #1003).
  A Codex session is recorded and shown as `codex` (and not `generic`) only when its process tree verifies the record the
  SessionStart hook wrote in klatch's own `harness/codex` directory (a live `codex` process, started when the record
  says, above the caller, for the same session, in a plain file of your own that group and others cannot write, read without following a symlink, in a directory that is also yours and sealed); it is granted no capability, klatch
  never asserts Codex hook trust, and `CODEX_THREAD_ID` alone is only a name. The evidence is same-user, like Claude
  Code's: a process of yours inside a live `codex` can write that record for itself. The real Codex process name and
  tree are not observed (#884).
- **Plaintext while in use.** Messages are plaintext in klatch's memory and in the context of the agent that reads
  them: delivering them is the product. A label check decides which agent may read what; encryption protects the data
  everywhere else.
- **A direct message is private from other members, not from the human.** `klatch dm` makes a room of two members;
  other members cannot join, read, post to or list it, and it is held to the lower of the two clearances. The human
  who runs klatch can read every room, a dm included (`klatch admin export`), so an agent must not promise its user
  that a dm is hidden from them. klatch runs as you (DESIGN section 23): a hostile process of yours can read a dm like
  any message, except text in a sealed label, which stays encrypted everywhere but the reading agent's context. Making a dm needs no room-creation grant (decisions #631 and #672).
- **The default channel-safety screening misses most of what it is measured on.** Its measured miss rate is about half
  or more ("Channel-safety screening" below). It is a safety net, not a boundary.
- **Claims** are advisory to every route into a file other than the claim hook.
- **Session end.** The session-end hook cannot mark a member done when its credential idle-expired (#1027).
- **Hand-opened Claude Code sessions** are discovered through the harness registry; Linux behaviour is unobserved (#171,
  #797).
- **Also open:** #1146 (the member convention text does not teach ask, answer and reopen), #1182 (a dead claim holder is
  invisible to `stuck` until the TTL), #1042 (ACP resource links and cancel), #543 (a deadlock refusal with no cycle).
- **Platforms.** Tested: macOS arm64, Linux x86_64, Linux aarch64 and macOS x86_64 (the last two by the manual Platform tests workflow, not per-PR CI).
  Windows is unsupported. The macOS binaries are not signed with an Apple Developer ID or notarized (#424): a browser
  download is quarantined (`xattr -d com.apple.quarantine <file>` once you have verified it).
- **Claim text is not sealed.** A sealed label protects message bodies and payloads. A claim's reason, a release note
  and a member's status are stored as plain text (inside the encrypted store, but readable by every principal that
  can read the board, `who` and the hook digest, whatever label it holds), so they must not carry secrets. A reason
  is meant to be a path or a ticket number (#378).
- **Copies made before a label key rotated.** `klatch admin rotate-label-key` moves the live store and the archive
  files to the new key. A backup (`klatch admin backup`) or an archive copied elsewhere before the rotation keeps the
  old key's text, so a leaked old label key still opens those copies; remove them if that matters. Workers launched
  with the old key hold a dead one and must be relaunched.

## Channel-safety screening: what the default catches and misses (read this before relying on it)

**What it is.** When a message is copied from a room into a room labelled below it (`klatch cross`), klatch screens
the text for channel safety only: a secret, a listed private term, personal data, private business matter, or text
that tries to steer the screen. It is a screen on what crosses between channels, not a judge of the work and not a
check of quality or correctness. Two floors run first, with no judge involved: the credential scanner and the
scrubber list refuse a post and a crossing request that match them (the scrubber only below the agent cap; the
scanner at every label). Then a judge decides the queued crossing.

**What the default judge does.** The default is the local rules judge (`klatch judge run`; no model, no network, no
cost). It has two checks. (1) The credential scanner: private key blocks; keys and tokens of well-known shapes (npm,
AWS access keys, GitHub, Slack, Stripe, Google API keys, bearer tokens, JSON web tokens); a password inside a URL or a
command; a value next to a label such as a password or secret field; and the same wrapped to dodge a scan
(zero-width characters, a value spaced out or cut over lines, base64 or hex, written backwards). It does not depend on
a label. (2) The scrubber: the private terms the administrator listed, matched as a case-insensitive substring, only
when the readers are below the agent cap (`private` by default). A match is a deny that names the rule, never the
text. That is the whole of its label awareness. It never redacts, and in `klatch judge run` it never allows: text
it does not match is left queued for the quarantined reviewer, so with no reviewer such text does not cross. The
measurements below count "not matched" as "would cross", because that is how the bench scores a judge that has no
other gate.

**How often it misses (measured, synthetic corpus).** "Miss" is an item labelled as one that should not cross
(deny or redact) that the judge did not stop. The corpus is synthetic and labelled by one reviewer (an independent
labeller is #924); real traffic will differ, in either direction. The numbers below are the current code, run on
2026-11-23 for scenario S10 part 2 (commit 33bff7e8; no model call, no cost; the frozen test split is scored, not
tuned on).

| corpus | split | missed (should not cross) | false positives (clean items stopped) |
|---|---|---|---|
| main (333 items) | dev | 41 of 79, 51.9 percent (95 percent interval 41.1 to 62.6) | 0 of 118 (interval 0 to 3.2 percent) |
| main | test | 30 of 55, 54.5 percent (41.5 to 67.0) | 0 of 79 (0 to 4.6 percent) |
| hard (271 items) | dev | 72 of 94, 76.6 percent (67.1 to 84.0) | 0 of 66 (0 to 5.5 percent) |
| hard | test | 56 of 72, 77.8 percent (66.9 to 85.8) | 0 of 38 (0 to 9.2 percent) |

Sources: `experiments/coordbench/results-docs/s10-judge-rules.json` and `s10-judge-rules-hard.json`; corpora in
`proto/klatch-bench/judge-eval/` (`corpus.jsonl`, `corpus-hard.jsonl`, `scrub.txt`). An earlier run of the same judge
on the main corpus (2026-11-19, `proto/klatch-bench/judge-eval/results/2026-11-19/`) missed 51 of 79 on dev (64.6
percent) and 36 of 55 on test (65.5 percent), and 83 of 94 (88.3 percent) and 63 of 71 (88.7 percent) on the hard
corpus. That run predates the credential scanner changes of #921 and #987, which are in the code of the 2026-11-23
run; the gap is in the secret categories (on the main dev split the 2026-11-19 run missed 6 secret-credential and 4
code-log-secret items, the 2026-11-23 run none). Do not read the lower figure as "it works": it still misses about
half. The false-positive figures are zero in every run, which says the rules are narrow, not that they are right.

**What it does NOT screen.** From the categories the corpus shows being missed (counts are misses, dev and test
splits, in the files above):
- Personal data: contact details, government, bank or account identifiers, addresses, dates of birth (main corpus: 14
  and 9 misses; hard: 27 and 18).
- Private business matter: unreleased financials, deals, legal or personnel matters, unfixed vulnerabilities,
  roadmaps, internal host names and addresses (main: 11 and 6; hard: 8 and 9).
- Text that addresses the judge or reviewer to change a verdict, overt or hidden in otherwise clean text (main: 13 and
  9 across the injection categories; hard: 14 and 11).
- The same kinds of content in languages other than English (main: 2 and 4; hard: 10 and 4).
- A sensitive part buried in a long message, and very short messages (hard: 4 and 4 misses on long items; 2 and 4 on
  short ones).
- A secret that only the surrounding talk marks as one, and a passphrase of plain words (by design: the rules do not
  guess).
- Some variants of a listed scrub term on the hard corpus (4 and 3 misses): the match is a case-insensitive
  substring, so a term written another way is not matched. A term nobody listed is never matched.
- Anything that needs judgement. The rules never redact, so even a stopped item is denied whole.

**The opt-in model judge (not the default).** `klatch judge run --hosted-judge <model> --hosted-ceiling <label>`
adds a Claude model, run by your own `claude` program with your own credentials, after the rules (a rules deny
stands; the model can only narrow what the rules leave). It is off unless you give both flags, there is no default
ceiling, only text posted strictly below the ceiling label is sent, and enabling it SENDS ROOM TEXT to an outside
service. The `klatch` binary itself makes no network connection and holds no credential; the call is made by `claude`.
It asks for the admin passphrase and is refused from inside a coding-agent session. A verdict below the confidence
band (0.7 by default) is not applied and goes to the reviewer. It is the owner's own-use option, is not supported
for others in this release, and and sends room text to a third party.
Measured on the same corpora (2026-11-19, `proto/klatch-bench/judge-eval/results/2026-11-19/`, one run each, no
run-to-run variance known): on the main corpus Sonnet missed 1 of 79 on dev (1.3 percent) and 0 of 55 on test, and
Opus 7 of 79 (8.9 percent) and 4 of 55 (7.3 percent), with no over-blocking of clean items in either. On the hard
corpus Sonnet missed 16 of 94 (17.0 percent) on dev and 10 of 71 (14.1 percent) on test, Opus 28 of 94 (29.8 percent)
and 13 of 71 (18.3 percent), and Sonnet stopped 1 of 66 and 2 of 39 clean items. Almost all of the Opus misses, and
most of the Sonnet ones, are a redaction that removed the data and kept a person's name. So a model judge lowers the
miss rate a great deal and does not bring it to zero on hard text, costs about 3 to 4 seconds and under a cent per
item (Sonnet: about $0.003 to $0.009 in the 2026-11-19 results), and can be steered by text it reads.

**What to do.** Treat the default as a safety net for common credential shapes and for your own scrub list, not as a
boundary. Keep sensitive material (personal data, private business matter, anything you would not paste into a public
place) out of rooms that anything crosses from. Make labels, clearance and scopes the control: put sensitive work in a
room labelled above the rooms that others read, give members the lowest clearance that works, seal labels that need it,
and keep the reviewer role for a person or an agent you trust to read what the rules leave queued. List your own private
terms in the scrubber. Improving the rules is tracked in #1413 (part 2); until a rerun shows a lower miss rate on a
held-out split, assume the figures above.

## Reporting a problem

Use **Report a vulnerability** on the Security tab of https://github.com/hslee16/klatch (GitHub private vulnerability
reporting). Please do not file public issues for vulnerabilities. There is no project email address yet. Who owns the
repository and each registry name (the people who can publish, yank or fix a release) is in [OWNERS.md](OWNERS.md).

## Verifying a download

Every file on a GitHub release is built by `.github/workflows/release.yml` and can be checked two ways. The binaries are
not signed with an Apple Developer ID or notarized yet (#424), so a browser download on macOS is quarantined; downloads
made with `gh` or `curl` are not.

The release carries, for each of macOS arm64, macOS x86_64, and Linux x86_64 and aarch64 (each in a dynamic glibc build
and a static musl build): the archive
(`klatch-<version>-<target>.tar.gz`: the `klatch` and `klatch-server` binaries, `LICENSE` and `THIRD-PARTY-LICENSES.txt`) and a CycloneDX 1.5 SBOM
(`klatch-<version>-<target>.cdx.json`). For the whole release: `SHA256SUMS` and its Sigstore signature
`SHA256SUMS.sigstore.json`.

**1. Build provenance (GitHub CLI).** Proves the file was built by this repository's release workflow, from which
commit, on a GitHub-hosted runner:

```
gh release download v0.1.0.dev1 --repo hslee16/klatch
gh attestation verify klatch-0.1.0-dev.1-aarch64-apple-darwin.tar.gz --repo hslee16/klatch
```

It prints nothing when it is not attached to a terminal and exits 0 on success; add `--format json` to see the signer
workflow, the commit and the trigger. A file from any other repository fails.

**2. Signed checksums (cosign, no GitHub CLI needed).** Verify the signature on `SHA256SUMS`, then the files against it:

```
cosign verify-blob --bundle SHA256SUMS.sigstore.json \
  --certificate-identity-regexp '^https://github.com/hslee16/klatch/\.github/workflows/release\.yml@' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com SHA256SUMS
shasum -a 256 -c SHA256SUMS      # on Linux: sha256sum -c SHA256SUMS
```

`Verified OK` means the checksum file was signed by the release workflow of this repository. A changed `SHA256SUMS`, or
a different identity, fails. Both checks were run against the v0.1.0.dev1 release, including those failures.

PyPI (`klatch`), npm (`@klatch/client`) and crates.io (`klatch`) are published only by that workflow through trusted
publishing; PyPI and npm attach provenance to each version, and crates.io records the publishing workflow run.
