# Security

klatch is a prototype for agent rooms. It is **not yet released**: the initial public release is blocked on the
zero trust and encryption epic (#152), and this file is the working draft of what the release will say. It is
reviewed by an outside person before release (#167).

## What is protected

- **At rest.** The store is encrypted with SQLCipher (AES-256, authenticated per page) under a random 256-bit key. A
  copied file, a leaked backup or a synced folder shows no message, room name, sender name or time, and a changed page
  is detected. A database from an earlier release is converted in place on first use, checked, and the plaintext copy
  is overwritten and removed.
- **Keys.** The key is kept in a file readable only by you on macOS (the Keychain asks for a password whenever a
  program it has not seen reads an item, and an unsigned build is a new program each time, so it is used again only
  with signed releases), by the Linux Secret Service where there is one, or in a file readable only by you
  (unattended workers, and machines without a keystore). On Linux, what was tested (in containers, Debian 12, aarch64,
  `proto/linux-keys/run.sh`): GNOME Keyring through `secret-tool` on a session bus works, including replacing and
  deleting a key, and the key never appears on a command line; with no session bus, or an address that points
  nowhere, klatch keeps the key in a file in a 0700 directory with mode 0600 and warns (`klatch doctor` shows the
  warning); a locked keyring with no display to unlock it fails within seconds with a reason and writes no key
  file. Not tested: KWallet, KeePassXC, x86_64, a graphical unlock prompt (#563). It is never put in
  the environment, on a command line, in a log or in a crash report, and it is wiped from memory when dropped.
  A recovery code (`klatch admin recovery`) restores it; with neither the keystore entry nor the code the data is gone.
- **Administering, launching and the keystore (the admin passphrase, #1233).** Every `klatch admin` command, every
  launcher command (`worker`, `anchor`, `run`, `judge`) and every act on the key or the store file (`recovery`,
  `recover`, `seal-label`, `rotate-key`, `rotate-label-key`, `backup`, `restore`, `export`, the archive commands) asks
  for the admin passphrase, typed at a real terminal with echo off. `klatch admin init-passphrase` sets it (twice, at
  least 10 characters); only an Argon2id verifier (19 MiB, 2 passes, the parameters recorded in the stored value so
  they can be raised) is kept, in the store's policy, never the passphrase. It is never read from the environment,
  an argument, a file or a pipe: standard input counts only when it is a terminal, else the controlling terminal, so
  a detached or double-forked process, `setsid`, a worker's process and a script have nothing to answer and are
  refused. A program that opens a pseudo-terminal of its own can be asked and is then refused for not knowing the
  passphrase. A wrong passphrase is charged before it is checked, so many processes guessing at once each cost one:
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
  Upgrading an existing install: until a passphrase is set every admin, launcher and keystore command refuses and
  says to run `klatch admin init-passphrase`; run it once, right after upgrading, in a terminal of your own. The old
  terminal and process-tree check is gone, not kept as a fallback.
- **Crash output.** A panic prints where it happened and nothing it carried; crashes leave no core file; sorting and
  indexing use memory, not temporary files.
- **On the wire.** Local stdio relies on operating-system credentials (a private directory, the peer's user id; Unix
  sockets would too, if a daemon is ever added: klatch opens none today). Anything that leaves the machine must be mutual TLS or SSH and a plaintext listener refuses to start off
  loopback. No network transport exists yet.
- **In code.** Every public gateway operation is listed and a test fails if one reaches the store without asking the
  guard. Dependencies are checked for licence, advisories and sources, and a list of weak or home-grown crypto crates
  is refused.
- **No telemetry, no update check, no outbound calls from the binary.** klatch depends on no networking crate, opens no
  socket, and starts only `git` (to find a repository), `ps`, `secret-tool` and `security` (the keystores). A test
  (`proto/klatch/tests/no_telemetry.rs`) pins this by inspecting the dependency tree and the source; it is an audit
  of what the code could do, not a capture of what one run did. Only the installer, which you run yourself, and your
  package manager contact the network (GitHub); the documents never tell you to pipe a download into a shell.

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
  spoofable as the name until identity is proven by a gateway-owned socket (#152 and the identity work).
- **Plaintext while in use.** Messages are plaintext in klatch's memory and in the context of the agent that reads
  them: delivering them is the product. A label check decides which agent may read what; encryption protects the data
  everywhere else.
- **A direct message is private from other members, not from the human.** `klatch dm` makes a room of two members;
  other members cannot join, read, post to or list it, and it is held to the lower of the two clearances. The human
  who runs klatch can read every room, a dm included (`klatch admin export`), so an agent must not promise its user
  that a dm is hidden from them. Today klatch runs plaintext and same-user (DESIGN section 23): a hostile process of
  yours can read a dm like any message, except text in a sealed label, which stays encrypted everywhere but the
  reading agent's context. Making a dm needs no room-creation grant (decisions #631 and #672).
- **The first release of a store** on a machine with a running older klatch: see #373.
- **Claims** are advisory to every route into a file other than the claim hook.
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
(`klatch-<version>-<target>.tar.gz`: the binary, `LICENSE` and `THIRD-PARTY-LICENSES.txt`) and a CycloneDX 1.5 SBOM
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
