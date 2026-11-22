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
  (#1233) [PASSPHRASE-PROOF-PLACEHOLDER: finish this sentence once #1233 has merged; until then these acts rest on a
  terminal and process-tree heuristic that a detached process escapes, #834 and #318].
- **In code.** Every public gateway operation is listed and a test fails if one reaches the store without asking the
  guard. Dependencies are checked for licence, advisories and sources, and a list of weak or home-grown crypto crates
  is refused.
- **No telemetry, no update check, no outbound calls from the `klatch` binary.** `klatch` depends on no networking crate,
  binds and connects to no socket, and starts only `git` (to find a repository), `ps`, `secret-tool` and `security` (the keystores). A
  test (`proto/klatch/tests/no_telemetry.rs`) pins this by inspecting the dependency tree and the source; it is an audit
  of what the code could do, not a capture of what one run did. Only the installer, which you run yourself, and your
  package manager contact the network (GitHub); the documents never tell you to pipe a download into a shell.
- **`klatch-server` is a separate claim.** It opens loopback listeners only (`127.0.0.1` or `::1` for `http` and the
  webhook intake) and a Unix socket for `daemon`; it starts no program and makes no outbound connection. A test
  (`proto/klatch-server/tests/no_network.rs`) pins this by inspecting the dependency tree and the source, again an audit
  of what the code could do and not of one run. The shorter claim above covers `klatch` only.

## What is not protected (read this)

- **A hostile process running as you.** It can read the key (the macOS item is readable by any program of yours so
  that rebuilt, unsigned binaries do not prompt; the `security` tool reads it too), read or attach to a running
  klatch, and read any key file. Encryption cannot help once the attacker can ask for the key. Root can do anything.
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
- **Platforms.** Tested: macOS arm64 and Linux x86_64; built and smoke tested only: Linux aarch64 and macOS x86_64.
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
