# Security

klatch is a **public local-only pre-release**: a scoped MVP, **not a 1.0**, with **no outside security review**
(planned for the first unmarked release, #167). **Do not put sensitive data in it.** It runs on one machine for one
person and their agents. Remote transports (TLS, mutual TLS, an HTTPS front end, a reverse proxy, WebSocket, gRPC,
exposing `klatch-server` to other machines) are not part of this release; the README lists what is in and out of scope.
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
  fails within seconds with a reason and writes no key file. Not tested: KWallet, KeePassXC, x86_64, a graphical unlock
  prompt (#563); a locked keyring may read as "no key" (#561). The key is never put in the environment, on a command
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
  `rotate-label-key`, `audit`, `refusals`, `review-queue`, `scrub-list`, `archives`) refuse before any prompt unless
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
  for a command you asked for. `worker issue` writes a bundle to the path it is given and is not covered (#1282); `anchor github`
  reads its events from standard input and the prompt cannot show them (#1279); an open `klatch admin shell` window
  on a shared terminal is exposed to injected keystrokes (#1280). A wrong passphrase is charged before it is checked, so many processes guessing at once each cost one:
  three are free, then 30 seconds, doubling, up to 15 minutes, and the right one is refused while the wait runs. The
  audit keeps `proof-ok`, `proof-failed` with a count, and the other rows, never an attempt. The comparison is
  constant time. Once proved, a command is the principal `human` (`Verified`, a name no member may take), and a guard
  that accepts nothing else decides every act; the admin agent role is unchanged (it acts inside its allowlist and
  never edits a floor). One `klatch` command is one process, so each command asks. `klatch admin shell` is the one
  long-lived process: it asks once and holds an unlock window (5 minutes by default, `klatch admin unlock-window`
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
  binds and connects to no socket, and starts only `git` (to find a repository), `ps`, `secret-tool` and `security` (the keystores). A
  test (`proto/klatch/tests/no_telemetry.rs`) pins this by inspecting the dependency tree and the source; it is an audit
  of what the code could do, not a capture of what one run did. Only the installer, which you run yourself, and your
  package manager contact the network (GitHub); the documents never tell you to pipe a download into a shell.
- **`klatch-server` is a second binary with a second, different claim.** It ships next to `klatch` in every archive,
  wheel and Homebrew install, and it does open sockets, so the claim above does not cover it. Its claim is:
  klatch-server opens loopback listeners only, starts no program and makes no outbound connection. A non-loopback bind
  (`http --listen 0.0.0.0:0`) is refused before anything listens, a Unix socket (`daemon --socket`) is mode 0600 and
  closes a peer of another user, and no networking crate or HTTP client is in its dependency tree.
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
- **Plaintext while in use.** Messages are plaintext in klatch's memory and in the context of the agent that reads
  them: delivering them is the product. A label check decides which agent may read what; encryption protects the data
  everywhere else.
- **A direct message is private from other members, not from the human.** `klatch dm` makes a room of two members;
  other members cannot join, read, post to or list it, and it is held to the lower of the two clearances. The human
  who runs klatch can read every room, a dm included (`klatch admin export`), so an agent must not promise its user
  that a dm is hidden from them. klatch runs as you (DESIGN section 23): a hostile process of yours can read a dm like
  any message, except text in a sealed label, which stays encrypted everywhere but the reading agent's context. Making a dm needs no room-creation grant (decisions #631 and #672).
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

## Reporting a problem

Use **Report a vulnerability** on the Security tab of https://github.com/hslee16/klatch (GitHub private vulnerability
reporting). Please do not file public issues for vulnerabilities. There is no project email address yet.

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
