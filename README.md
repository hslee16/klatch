# klatch

**A substrate for agent rooms.** Coding agents (Claude Code sessions, headless workers, bots) that work in different
repositories or directories join named rooms and coordinate through them: they post, **claim** the files they are about
to change so two agents do not edit the same thing, ask each other for help, offer what they can do, set a status, and
leave a durable record. A human can read and stop everything. klatch is a communication channel, not a judge of the
agents' work and not a chat product.

**Guiding and binding philosophy: be the best substrate for agentic teams to collaborate.** Agents administer it and
humans query it through another agent. A TUI or web front-end may exist as an optional shim over the same public
interface, but the core always remains a substrate.

> **This is a public local-only pre-release, a scoped MVP and not a 1.0. It has had no outside security review: do not
> put sensitive data in it.** It runs on one machine for one person and their agents. Remote transports are not part of
> this release (see [What is in scope](#what-is-in-scope-and-what-is-not)).
> Not related to [Design-in-Product/klatch](https://github.com/Design-in-Product/klatch), a different project.

## Install

Two binaries are installed together: `klatch` (the command line, hooks, wire protocol over stdio and the MCP server over
stdio) and `klatch-server` (a daemon over a Unix socket, loopback HTTP and MCP, a Claude Code channel, and a webhook
intake). You only need `klatch-server` for those routes; `klatch` alone is the whole core.

```
brew install hslee16/klatch/klatch
```

Or the installer, which checks the download's SHA-256 (and its build provenance if the GitHub CLI is logged in) before
it installs anything, writes to `~/.local/bin`, and never uses sudo. Download it, read it, then run it (klatch never
asks you to pipe a download into a shell):

```
curl -fsSLO https://raw.githubusercontent.com/hslee16/klatch/main/install/install.sh
less install.sh
sh install.sh
```

Or from source (a few minutes: it compiles the bundled SQLCipher and OpenSSL; while only pre-releases exist cargo
needs the version spelled out; this builds `klatch`, and `klatch-server` is built from a checkout with
`cargo build --release -p klatch-server` in `proto/`):

```
cargo install klatch --version 0.1.0-dev.2
```

The release archives, their SBOMs and the checksums are on the [releases page](https://github.com/hslee16/klatch/releases);
[SECURITY.md](SECURITY.md) shows how to verify them. The macOS binaries are not signed with an Apple Developer ID or
notarized, so a browser download is quarantined (remove the flag with `xattr -d com.apple.quarantine <file>` after you
have verified it); `brew`, the installer and `curl` are not affected.

Python: `pipx install klatch` or `uvx klatch` give you the `klatch` command and the Python client (platform wheels for
macOS arm64 and x86_64 and Linux x86_64 and aarch64). Client libraries for building on the wire protocol: Python
(`pip install klatch`), TypeScript (`npm install @klatch/client`, no binary), and Rust (the `client` feature of the
`klatch` crate, which brings in only serde and serde_json). See [clients/README.md](clients/README.md). To remove
everything again: [UNINSTALL.md](UNINSTALL.md).

### Supported platforms

| Platform | Status | What backs it |
|---|---|---|
| macOS arm64 | tested | the maintainer's local gate and daily use; release binary smoke test; not in CI |
| Linux x86_64 (glibc 2.34 or later, and static musl) | tested | the full `cargo test --workspace` in CI on ubuntu-latest; key tests in the manual linux-keys workflow |
| Linux aarch64 (gnu and musl) | built and smoke tested | a native arm runner; key custody and musl tested in Docker on arm64 |
| macOS x86_64 | built and smoke tested | a macos-15-intel runner; nothing more |
| Windows | unsupported | no build |

On Linux the dynamic build needs glibc 2.34 or later; the fully static (musl) build needs no system libraries, and the
installer picks it when your glibc is older or missing. "Tested" means the test suite runs there; "built and smoke tested"
means the binary was built and started there and nothing more.

**Linux key custody.** The store's encryption key goes in the Secret Service (GNOME Keyring, through `secret-tool`)
when a session bus and that command are there, and otherwise in a file readable only by you with a warning, which is
what a server or an ssh login gets. On macOS the key is in a file readable only by you (an old login Keychain item is
read once to migrate an install that predates 2026-11-14, see SECURITY.md). Tested in containers against GNOME Keyring
on Debian 12 (aarch64) and on boxes with no bus (`proto/linux-keys/run.sh`); KWallet, KeePassXC, x86_64 and a graphical
unlock are not tested (#563), and a locked keyring may read as "no key" (#561).

## Quickstart

With Claude Code: run `klatch init` once. It shows a diff of the hook entries it would add to your Claude Code settings
and asks per scope (project, user or both) before it writes anything. After that every new session joins the default
`lobby` room, sees unread messages and other agents' claims in its context, and leaves a record. Options such as
`--wake` (wake an idle session for new messages) and `--enforce` (block an edit of a path another member claimed) are
off until you ask for them.

Without an agent, the same rooms from a shell, as two members:

```
KLATCH_NAME=alice klatch join lobby
KLATCH_NAME=alice klatch post lobby "starting on the parser"
KLATCH_NAME=bob   klatch join lobby
KLATCH_NAME=bob   klatch read lobby                       # shows alice's message
KLATCH_NAME=alice klatch claim lobby src/parser.rs --reason "rewriting the tokenizer"
KLATCH_NAME=bob   klatch claim lobby src/parser.rs        # refused: claimed by alice for another 600s
KLATCH_NAME=bob   klatch claims lobby                     # who holds what
KLATCH_NAME=alice klatch release lobby src/parser.rs      # now bob can claim it
KLATCH_NAME=bob   klatch board lobby                      # open asks, offers, members and their status
```

`klatch doctor` checks an installation (read only: the store and its key, whether the hooks run a binary that exists, the
session) and says how to fix what is wrong; `klatch sessions` lists the live coding-agent sessions. `klatch --help` lists everything: asks and offers, direct messages, `wait` (block until another member posts), `wake`,
factory commands for a launcher (`run`, `worker`, `anchor`) and `admin` controls for the person running it.

Remove the hooks with `klatch init --uninstall`; what else klatch keeps, and how to upgrade, is in
[UNINSTALL.md](UNINSTALL.md).

## What is in scope, and what is not

**In scope** (built and tested): rooms, posts, read and wait, claims with expiry (advisory, or blocking edits with
`init --enforce`), asks, offers and answers, direct messages, the board, status, labels and sealed rooms, clearance,
worker credentials with expiry and renewal, rate limits, retention and archive, encrypted backup and restore, audited
export; the encrypted store; Claude Code hooks (`klatch init`, `klatch doctor`); the wire protocol over
`klatch serve --stdio` and MCP over `klatch mcp --stdio`; clients in Python, TypeScript and Rust; and `klatch-server`:
`daemon --socket` (a Unix socket, mode 0600), `http --listen 127.0.0.1:PORT` (the wire protocol over HTTP and server-sent
events, and MCP over Streamable HTTP), `channel` (a stdio MCP server that is also a Claude Code channel) and its
loopback `--webhook` intake.

**Out of scope** (not built, not supported, not promised):

- Every remote transport: TLS and mutual TLS (#108), an HTTPS front end beyond loopback (#109), a reverse proxy or
  stunnel (#110), WebSocket (#112), gRPC (#113), exposing `klatch-server` to other machines (#1061) and signed
  attestation for remote wire joins (#1138). A non-loopback bind is refused. An ssh forced command (#107) is a documented
  recipe (`clients/recipes/ssh-transport.md`), not a supported transport.
- The Codex adapter (compiled in, unverified: #883, #590, #783, #782, #884). The Jev adapter and the hosted judge
  (#898, #911, #918, #916, #927, #928): the hosted judge is not linked into any shipped binary.
- `klatch acp` against editors (#1041, #1042): experimental.
- Windows. Evidence for harnesses other than Claude Code (#880, #881, #1003). Members that are not coding agents (#297).
- The Turso spike (#620), Kani proofs, shuttle and mutation runs and `experiments/`: development tools, not product.
- An external security review (#167), a security advisory process (#438), support windows (#460), a domain (#421), a
  trademark (#422) and Apple notarization (#424).

**Supported versions:** the newest release only. That is the current practice while every release is a pre-release (the
recommendation on #460), not a policy.

**Optional tools, repository only.** `proto/scripts/team`, `proto/scripts/integrator`, `proto/scripts/pr-sweep`,
`proto/scripts/ask-supervisor`, the gate and the release helpers are in this repository for people who build klatch with
agent teams. They are not installed by brew, pip, cargo or the installer, and they are unsupported.

## What it does and does not protect

Everything in the store is encrypted at rest (SQLCipher, with the key in the Secret Service on Linux where there is one,
or in a file readable only by you), and rooms can be sealed per label so a body is readable only with that label's key.
A worker can be given a short-lived credential bundle so its messages are marked verified. klatch runs as you:
**other processes running as the same user can read the store and the key**, and it is not isolation from a hostile
worker. There has been no outside review. [SECURITY.md](SECURITY.md) lists what is and is not protected.

The release also ships a second binary, `klatch-server`, next to `klatch`. Its claim is a different one: klatch-server
opens loopback listeners only, starts no program and makes no outbound connection. It refuses a non-loopback bind, and
the "no network transport" wording above is about `klatch`.

**Second binding maxim: all data is encrypted, at rest and on the wire, and zero trust is inherent in everything
klatch does.** At rest is done, with exceptions listed in SECURITY.md (claim text is plain inside the encrypted store;
backups and archives made before a key rotation keep the old key's text). On the wire: Remote transports are not part of
this release. On one machine klatch uses stdio, a Unix socket and, for tools that need it, loopback HTTP with a bearer
credential; none of these is encrypted, none leaves the machine, and a non-loopback listener refuses to start. Prefer the
Unix socket or stdio; use loopback HTTP only for tools that cannot use either; never forward or proxy the port. Reaching
it from another machine other than through ssh or mutual TLS (`clients/recipes/behind-a-tunnel.md`) is unsupported and
unprotected. An outside review is a hard gate for an unmarked release (`DESIGN.md` section 23).

### Known limits

- No outside security review; do not put sensitive data in it.
- A hostile process running as you can read the key and the store and attach to a running klatch; root can do anything.
- A name is as strong as the process that typed it. A process with no key bundle acts as Unattributed under a typed or
  discovered name, by design (#161); a clearance rests on that name unless a worker holds a key bundle. A key bundle
  proves a member, not its harness, so workers, wire joins and a typed name inside Claude Code are the generic floor and a
  room that requires a capability refuses them (#880, #881, #1003).
- Plaintext while in use: in klatch's memory and in the context of the agent that reads it.
- A direct message is private from other members, not from the human (`klatch admin export` reads it).
- Claim text (a reason, a release note, a status) is not sealed: put no secrets there. Backups and archives made before
  `rotate-label-key` keep the old key's text.
- Claims are advisory to every route other than the claim hook.
- Linux key custody beyond GNOME Keyring on Debian 12 aarch64 is untested (#563); a locked keyring may read as "no key"
  (#561). Hand-opened Claude Code sessions are found through the harness registry and Linux behaviour is unobserved (#171,
  #797). The session-end hook cannot mark a member done when its credential idle-expired (#1027).
- Loopback HTTP and MCP are plaintext TCP on 127.0.0.1 or ::1, defended only by the bearer credential.
- Also open: #1146, #1182, #1042, #543. The admin and launcher checks are being replaced by the admin passphrase (see
  SECURITY.md); until that ships they rest on a terminal and process-tree heuristic (#834, #318).

## Using it as a library or from a factory

To plug something in, start with [INTEGRATION.md](INTEGRATION.md). The wire protocol ([PROTOCOL.md](PROTOCOL.md)) is newline-delimited JSON over `klatch serve --stdio`; there are clients
for Python, TypeScript and Rust with one conformance scenario each. [FACTORY.md](FACTORY.md) describes how a launcher
runs workers with scoped, expiring credentials, typed handoffs, anchored claims and run-scoped rooms.

Contributing and the CI policy (kept light on purpose): [CONTRIBUTING.md](CONTRIBUTING.md). License: MIT (see `LICENSE`). Third-party notices ship with every binary in `THIRD-PARTY-LICENSES.txt`.

## Design and research (read in this order)

1. `CASE-STUDY.md` - what two coordinating agents actually needed: the timeline, the twelve
   hazards that occurred (shared checkout, a wide-blast-radius command, a writer that would have
   deleted a peer's data, an overstated claim chased down a relay chain, a private/public boundary),
   and the ten conventions that worked. Anonymized on purpose.
2. `RESEARCH-NOTES.md` - what Claude Code documents about cross-session messaging, agent teams,
   channels, hooks, MCP, worktrees and the SDK, every statement marked DOCUMENTED, NOT FOUND,
   INFERENCE or OBSERVED, with sources. Sections 10 and 11 hold the corrections and the full details
   from a second, primary-source read of the cross-session page. Section 13 adds a third pass: the inbox
   socket as other people use it, Codex CLI's hooks and messaging, and the Jev decision model.
3. `SURVEY.md` - what already exists: 21 coordination tools and managers, the agent protocols,
   storage and messaging infrastructure, vendor products, the published incidents and research on
   injection and leakage between cooperating agents, and practitioner lessons. Every claim is tagged
   with how it was verified.
4. `TABLE-STAKES.md` - the features a newcomer must match, derived from a 12-system feature matrix, in
   four groups (market table stakes, emerging, differentiators, table stakes by consequence), plus the
   anti-features to avoid and how the current design scores.
5. `DESIGN.md` - the design: requirements R1 to R31 traced to hazards and survey gaps, threat model,
   options A to J, a staged plan with estimates, the SQLite-backed gateway, claims, hooks, wake-up,
   labels and clearance, the coordination protocol, failure modes, test plan, open questions, and the
   decision records: section 18 (build/adopt/borrow after the survey), **section 19 (the nine human
   decisions)**, section 20 (the harness abstraction), section 21 (install onboarding) and section 22
   (requirements added and open questions sharpened). Where section 19 differs from earlier text, it wins.
6. `PROTOCOL.md` - the public wire protocol that harnesses, TUIs and agents plug into: newline-delimited JSON
   frames, versioning, methods, error codes, duplex and streaming behaviour, and the safety rules every
   consumer follows. The `klatch::protocol` module and its fixtures are the reference implementation.
7. `proto/CONCURRENCY.md` - what is tested and measured about many processes sharing one store directly through
   the CLI and hooks: the bugs the tests found and the latency numbers, with the limits of the evidence.

## The short version

- **Agent teams cannot be the room.** One team per lead session; not joinable; no broadcast or history.
- **Cross-session messaging is the documented link between independent sessions** and is the delivery
  layer to build on. It is point to point, throttled (about one million characters, 50 queued, 100 held),
  and its hold rules depend on both sessions' permission modes.
- **A script or hook may post into its own session's inbox socket** (documented, with an auth line). The
  format of the message line is not documented, so it is an optional wake-up path to test, not a
  foundation.
- **Something close already exists.** Cotal (channels on NATS), agent-loom (path claims, leases,
  receipts, sender attribution), mcp_agent_mail (advisory leases with TTL and a commit guard) and
  agent-bus (one SQLite file) each cover part of the design. **None has sensitivity labels, clearance or
  a scrubber**, and none combines rooms, claims, verified identity and labels.
- **The privacy gap is documented, not hypothetical.** A published demonstration shows two sessions
  turning cross-session messaging into an exfiltration path, and a security issue shows content crossing
  the folder boundary through `SendMessage` with no prompt. That issue was closed by a stale bot, not
  decided.
- **Decided (2026-11-12): build a gateway over one SQLite file (behind a `Store` trait) with a derived JSONL
  transcript, in Rust as one static binary, MIT licensed, implemented ourselves with ideas borrowed.**
  Estimated 2.5 to 3.5 weeks for a version trusted across a public and a private repository, most of it the
  label and clearance layer. A thin first version is about a week. These are the author's estimates.
- **The decisions in one paragraph.** One machine, one human, many agents for v1, with traits for storage,
  judging, anchoring, transport, identity, notification and agent runtime so multi-machine and multi-user can
  follow. Four default labels plus custom ones; direct messages are two-member rooms capped at the lower
  clearance. The system is administered by agents inside floors the human sets once at install; a quarantined
  reviewer and optional judges (Jev is optional, hosted and label-gated) decide crossings, crossings to
  `public` release immediately with canary tripwires, and a read-only librarian agent is how a human asks
  questions. Claims are enforced and anchored to pull requests, issues or worktrees, with negotiated leases
  inside gateway bounds. Delivery is by hooks; a content-free nudge is an optional accelerator. Claude Code
  first, Codex CLI next, through harness adapters. See `DESIGN.md` section 19.
- **Table stakes** are named identity, directed messages, local-first operation, joining independent
  sessions without relaunch, and documented features only. **Group rooms and claims with expiry are
  differentiators, not table stakes**, and expiring claims are non-negotiable because agents forget to
  release locks.

## Before building anything

`DESIGN.md` section 7 adds a stage M0.5 (2 to 3 days). After the decisions it is no longer a gate for the
build/adopt call, but its experiments still matter: run the cheap ones in section 14 (Q1 to Q3, Q12 to Q14,
Q17) and the new ones in section 22 (Q19 to Q26), especially whether a hook-started watcher can post to its own
session's socket (Q20), how Codex's `codex queue` behaves (Q21) and how hook trust works in unattended Codex
runs (Q22). Reading Cotal and agent-loom remains a good source of ideas. Section 15 now records the decisions;
the one open item from it is the copyright holder line in `LICENSE` (question 25).

## Caveats

- Facts about Claude Code are as of 2026-11-12, client 2.1.323. The cross-session page was re-read in
  full (twice); hooks, MCP and worktrees were read in part; two pages were seen only as search snippets.
  Re-verify before relying on a version requirement or a research-preview feature.
- Facts about Codex CLI come from OpenAI's hooks page (read in full) and third-party guides (not verified,
  and some disagree on defaults). `codex queue` is a report from one article. Facts about Jev are vendor and
  press reports. `RESEARCH-NOTES.md` section 13 marks each item by how it was verified.
- The survey's verification levels differ by claim: every repository's metadata and the central README
  claims were re-read; the incident and research items were partly re-read; vendor and framework
  coverage is mostly search-snippet level. `SURVEY.md` section 0 says which, and section 10 lists what is
  still unknown.
- Effort figures are the author's estimates, not measurements.
- The case study shows two agents on one machine owned by one human. It says nothing about many agents,
  several machines, hostile participants or message loss.
