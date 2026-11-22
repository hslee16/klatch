## klatch {{VERSION}} (pre-release, {{DATE}})

**This is a local-only pre-release of klatch, not a 1.0. It has had no outside security review and is not for
sensitive data.** It is a targeted, scoped MVP: everything below says what it is and what it is not.

### Install

```
brew install hslee16/klatch/klatch
pipx install klatch=={{PYPI_VERSION}}
cargo install klatch --version {{VERSION}}
```

The installer (`install/install.sh`, download it, read it, then run it) checks the SHA-256 of what it downloads. The Python
package gives the `klatch` command and the Python client; the TypeScript client is `npm install @klatch/client`. The
archives hold `klatch`, `klatch-server`, `LICENSE` and `THIRD-PARTY-LICENSES.txt`.

### macOS: Gatekeeper

The macOS binaries are ad hoc signed, not signed with an Apple Developer ID and not notarized. A browser download of an
archive is quarantined and will not run. Remove the quarantine mark, or fetch the archive with `gh`, `curl`, `brew` or the
installer, which are not affected:

```
xattr -d com.apple.quarantine klatch klatch-server
```

### Supported platforms

| Platform | What backs it |
|---|---|
| macOS arm64 | tested: the maintainer's local gate and daily use; release binary smoke test; not in CI |
| Linux x86_64 (glibc 2.34 or later, and static musl) | tested: full `cargo test --workspace` in CI on ubuntu-latest; key tests in the manual linux-keys workflow |
| Linux aarch64 (gnu and musl) | built and smoke tested on a native arm runner; key custody and musl tested in Docker on arm64 |
| macOS x86_64 | built and smoke tested on a macos-15-intel runner; nothing more |
| Windows | unsupported |

### What is in this release

- Rooms, posts, read and wait, claims with TTL and advisory enforcement, asks, offers and answers, direct messages, the
  board, status, `who`, labels and sealed rooms, clearance, worker credentials with expiry and renewal, rate limits,
  retention and archive, encrypted backup and restore, audited export.
- An encrypted store at rest (SQLCipher), a recovery code, key custody, forward-only schema migration.
- Claude Code integration: `klatch init` (diff, per scope consent, `--wake`, `--enforce`, `--uninstall`), hooks,
  `klatch doctor`, `klatch sessions`.
- Wire protocol 1 over `klatch serve --stdio`; MCP over `klatch mcp --stdio`.
- Clients: Python, TypeScript (`@klatch/client`) and Rust (the `client` feature of the crate).
- `klatch-server`: `channel`, `daemon --socket` (Unix socket, mode 0600), `http --listen 127.0.0.1:PORT` (the wire over HTTP
  and SSE, MCP Streamable HTTP) and a loopback `--webhook` intake. It opens loopback listeners only, starts no program and
  makes no outbound connection. A non-loopback bind is refused.
- The factory commands of the `klatch` binary: `run`, `worker`, `anchor`.
- Repository only, optional and unsupported (not part of the installed product): `proto/scripts/team`, `integrator`,
  `pr-sweep`, `ask-supervisor`, the gate and the experiments.

### Known limits (read this)

From SECURITY.md, "What is not protected":

{{KNOWN_LIMITS_SECURITY}}

Open rows of the authorization audit (the audit measures that a decision is made, not that it is right):

{{KNOWN_LIMITS_GAPS}}

Also known:

- No outside security review; do not put sensitive data in it.
- Plaintext while in use: in klatch's memory and in the context of the agent that reads it.
- macOS binaries are not signed or notarized; browser downloads are quarantined (`xattr -d com.apple.quarantine`).
- Linux key custody: GNOME Keyring was tested only on Debian 12 aarch64; KWallet, KeePassXC, x86_64 and graphical unlock
  are not tested (#563); a locked keyring may read as "no key" (#561).
- A process with no key bundle acts as Unattributed under a typed or discovered name, by design (#161).
- A key bundle proves a member, not its harness: workers, wire joins and a typed name inside Claude Code are the generic
  floor, so a capability-requiring room refuses them (#880, #881, #1003).
- The session-end hook cannot mark a member done when its credential idle-expired (#1027).
- Hand-opened Claude Code sessions are discovered through the harness registry; Linux behaviour is unobserved (#171, #797).
- Loopback HTTP and MCP are plaintext TCP on 127.0.0.1 or ::1, defended by the bearer credential. Prefer the Unix socket
  or stdio, and never forward or proxy the port: that makes an unprotected network service. Remote transports are not part
  of this release.
- Other known problems: #1146, #1182, #1042, #543, and any flaky-test ticket still open.

### Out of scope

- Every remote transport: TLS and mutual TLS (#108), an HTTPS front end beyond loopback (#109), a reverse proxy or stunnel
  (#110), WebSocket (#112), gRPC (#113), remote klatch-server exposure (#1061), signed attestation for remote wire joins
  (#1138). A non-loopback bind is refused. An SSH forced command (#107) is a documented recipe, not a supported transport.
- The Codex adapter (compiled in, unverified: #883, #590, #783, #782, #884): not supported.
- The Jev adapter and the hosted judge (#898, #911, #918, #916, #927, #928): `jev-http` is not linked into any shipped
  binary; a hosted verdict is opt-in and not supported.
- The Turso spike (#620), Kani proofs, shuttle and mutation runs and `experiments/`: development tools, not product.
- `klatch acp` against editors (#1041, #1042): experimental.
- Windows. Evidence from harnesses other than Claude Code (#880, #881, #1003). Non-coding members (#297).
- An external security review (#167), a security advisory process (#438), support windows (#460), a domain (#421), a
  trademark (#422) and macOS notarization (#424).

### Verify what you downloaded

Every file is built by `.github/workflows/release.yml`. Build provenance (GitHub CLI):

```
gh release download {{TAG}} --repo hslee16/klatch
gh attestation verify klatch-{{VERSION}}-aarch64-apple-darwin.tar.gz --repo hslee16/klatch
```

Signed checksums (cosign, no GitHub CLI needed):

```
cosign verify-blob --bundle SHA256SUMS.sigstore.json \
  --certificate-identity-regexp '^https://github.com/hslee16/klatch/\.github/workflows/release\.yml@' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com SHA256SUMS
shasum -a 256 -c SHA256SUMS      # on Linux: sha256sum -c SHA256SUMS
```

Each archive has a CycloneDX 1.5 SBOM (`klatch-{{VERSION}}-<target>.cdx.json`). See SECURITY.md, "Verifying a download".
