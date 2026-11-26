# PR sweep

`proto/scripts/pr-sweep` runs ONE pass over `pr-ledger` and takes the next step on every PR that has one, so a green PR does not sit unmerged until someone remembers (#702, #706). Run it in a loop to keep it standing.

## Run it

```
proto/scripts/pr-sweep --dry-run     # print what it would do; changes nothing
proto/scripts/pr-sweep               # one real pass
proto/scripts/pr-sweep --bench-ledger FILE   # keep the benched-PR list somewhere else
proto/scripts/pr-sweep --budget-floor 1500   # skip the pass (exit 0, with a message) when fewer than 1500 API calls are left
proto/scripts/pr-sweep --batch [--max N]     # land several PRs per integration (see Batch mode below)
```

Use a clone or worktree that nobody else is editing in: integrating switches that checkout to a throwaway branch `alee/integ-N` (it refuses a dirty tree). Needs `gh` logged in, `python3`, and `cargo` (it adds `$HOME/.cargo/bin` to PATH itself; it never prints the environment).

The bench ledger defaults to `$TMPDIR/pr-sweep-bench.json`. A fresh ledger makes every recently merged PR show as `merged-not-benched`, so the first real pass would bench them all; seed it first with `python3 -I proto/scripts/pr-ledger --record-bench N --bench-ledger FILE` for the PRs already benched.

## What it does, per stage

| Stage | Action |
| --- | --- |
| checks-cancelled | `pr-ledger --rerun-cancelled PR` |
| ready | re-checks live that every check is green on the latest head and the PR is mergeable; if behind, integrates main first and waits for green on the new head; merges with `gh pr merge PR --merge --match-head-commit SHA`; runs `cargo run -q --release -p klatch-bench` (must say "N passed, 0 failed") and records it with `--record-bench`; closes the linked ticket if still open |
| behind, conflict | merges origin/main into the PR branch on `alee/integ-N` (conflicts in DESIGN.md, CHANGELOG.md, VERSIONING.md, scenarios `mod.rs` keep both sides, DESIGN items sorted by number; new design items are fragments in `design/items/` (#807), so a DESIGN.md conflict is now rare, but PRs that still append a numbered item keep resolving this way), runs the gate, pushes to the PR's own branch |
| merged-not-benched, merged-open-ticket | bench and record; close the ticket |
| checks-red, code conflict, no ticket, stuck over 3 h, abandoned, failed gate or bench | not touched; one lobby message via `pr-ledger --post --only` names them (only when the list changed) |

It exits 0 with one summary line (`nothing needs a step` when there is nothing to do).

## Batch mode (#852)

```
proto/scripts/pr-sweep --batch --dry-run   # print the plan: which PRs, which dropped and why; changes nothing
proto/scripts/pr-sweep --batch [--max N]   # default N is 6, minimum 2
```

One at a time, every merge makes every other ready PR fall behind, so a queue of 10 takes hours. `--batch` lands several PRs per integration. The human's rule (2026-11-18): no merge conflicts before merges.

1. **Candidates**: PRs at stage `ready` or `conflict` with a linked ticket and every check green on their own head, in PR-number order. Up to three times `--max` are looked at, and the pre-check below stops once `--max` PRs survive, so PRs dropped for a code conflict do not use up slots (#854). A conflict only counts when it is in a docs file the resolver handles (DESIGN.md, CHANGELOG.md, VERSIONING.md, RESEARCH-NOTES.md, scenarios `mod.rs`).
2. **Pre-check**: the sweep builds a throwaway branch `alee/batch-TIMESTAMP` from `origin/main`. Before each candidate touches it, `git merge-tree --write-tree --name-only` (needs git 2.38 or newer; it only writes objects) says which files would conflict, first against `origin/main`, then against the branch so far. A conflict in any non-docs file drops the PR BEFORE it is merged: it is reported, posted via `pr-ledger --post --only`, and never merged. Two PRs that conflict with each other: the lower number stays, the higher is dropped. A head that moved since it was seen green is dropped too. The survivors are added with a real `git merge` of `refs/pr-sweep/N` (fetched with `--no-prune`); docs conflicts keep both sides; a merge that still conflicts is aborted and the PR dropped.
3. **Fewer than 2 survivors**: nothing is batched; the one-at-a-time path takes over for the survivors.
4. **Gate once** on the combined branch. If it fails the sweep says `batch gate failed`, pushes nothing, and the next pass takes that same set one at a time (the failed set is remembered in `$TMPDIR/pr-sweep-batch-failed`; a different set batches again). Bisecting is out of scope.
5. **One pull request** `Batch: #A #B #C` is opened from the pushed batch branch. Its body lists the PRs and copies each one's `Closes`/`Fixes`/`Resolves`/`Refs` lines, so tickets still close, and ends with the Generated-with line. The sweep waits for BOTH checks green on that head, then merges with `gh pr merge --merge --match-head-commit SHA`. If main moved (the existing is_behind rule) it merges main into the batch branch, re-gates, pushes and waits again, always merging the new head; a code conflict with main closes the batch PR with a comment and posts every included PR.
6. **After the merge**: each included PR's head is checked with `git merge-base --is-ancestor HEAD origin/main` and a REST read. GitHub normally marks them merged on its own; a PR still open after a short wait whose head is in main gets the comment `merged via #BATCH` and is closed with REST (`PATCH state=closed`). A still-open PR whose head is NOT in main is never closed, it is posted. Then the bench runs once (must say `N passed, 0 failed`), every included PR is recorded as benched, `merged: #N` is posted for each, the tickets the PRs close (Closes/Fixes/Resolves only) are closed, and one lobby summary is posted.

Safety rules stay: only PRs green on their own head, no force-push, no push to main (only `alee/batch-*` and a PR's own branch are pushed), no `git merge` on main, one sweep at a time via the lock, a real failure is never rerun.

If a pass dies with its batch PR still open (checks not green in time), the next pass sees the open `alee/batch-*` PR, starts no new batch, leaves the PRs listed in its body alone, and lands the batch PR the ordinary way (the included PRs are then benched one by one as `merged-not-benched`).

`--dry-run` fetches refs only (`refs/pr-sweep/*`, `origin/main`); it never switches branches, merges, commits, pushes, opens a PR or posts. It checks each candidate against main and against the earlier survivors in pairs.

## When a call fails (#754, #756)

- Every `gh` call has a 60 s timeout and two retries (a hung call once stalled a pass for 33 minutes). A rate limit (HTTP 403, "rate limit") is not retried.
- A call that still fails, or a ledger that failed, makes the pass incomplete: the sweep prints `N call(s) failed, pass incomplete` (and `GitHub API rate limit exhausted; resets at TIME` for a rate limit) and exits 1. It never prints `nothing needs a step` after a failure, and a PR whose checks could not be read is never treated as "not green".
- `--budget-floor N` reads `gh api rate_limit` (it costs no call) and skips the pass with exit 0 when fewer than N calls are left. Use 1500 in the loop.
- A lock held by a live pid for over 30 minutes is reported as `held the lock for N minutes ... it may be hung: kill -9 PID` and the sweep exits 3, so the loop line shows it. The next pass takes the lock once that pid is gone.
- `pr-ledger` makes few calls per PR: the open and recently merged lists once each, one check-runs call per open PR that is not a draft, the pull itself only for a PR whose checks are all green (for its mergeable state), and the ticket only for a merged PR. Agents poll it at most every 3 minutes with `--budget-floor 1000` (AGENT-PR-RULES.md).

## What it never does

- Merge a PR whose checks are not all green on its latest push (the merge is pinned to the head it saw green), or one that is conflicted or has no ticket.
- Rerun a real failure (only `cancelled` jobs, which never ran).
- Force-push, push to main, or run `git merge` while on main.
- Overlap with itself: a lock directory `$TMPDIR/pr-sweep.lock` holds the pid of the running sweep; a second sweep exits at once. A lock whose pid is gone is taken over.
- Touch a PR the code cannot merge cleanly: a code conflict aborts the merge and the PR is posted.

The recorded human reading on #706 is that a green PR may be merged without a per-PR prompt under the standing rule (every check on the latest push green). Keep that gate strict.

## Run it as a loop

In a Claude Code session, in the clone it should use:

```
/loop 10m proto/scripts/pr-sweep --budget-floor 1500
```

Each tick is one pass. Run `--dry-run` first and read the output before switching the loop on.

To batch, the coordinator changes the loop line to `/loop 10m proto/scripts/pr-sweep --batch --budget-floor 1500` once a `--batch --dry-run` plan looks right. Rows that are not candidates (cancelled reruns, merged-not-benched, red, no ticket) are handled as before in the same pass.

## Tests

Files marked `merge=union` in `.gitattributes` (#1065: the `mod x;` registries) are merged by git itself, in the sweep's real `git merge` and in the `git merge-tree` pre-check, so they never reach the docs resolver; the resolver and its docs list stay as the fallback for the files git cannot union (DESIGN.md, CHANGELOG.md, and two edits of one line of the others). GitHub ignores the attribute, so a PR can show `dirty` there and merge cleanly here.

`proto/scripts/test-sweep.sh` (run by the gate) drives the sweep with a fake `gh`, `git`, ledger, gate and bench on PATH; nothing touches the network or a real repo.

## GitHub watcher: webhooks instead of polling for awareness (#1329)

`proto/scripts/gh-watch` turns GitHub events into one-line notes in a log file the coordinator can `tail -F` (for example with the Monitor tool), so awareness no longer waits for the next sweep pass. It is read-only: it never calls `gh` or the GitHub API, and merges, labels, comments, pushes and closes nothing (a test asserts that the receiver has no `gh` call and no HTTP client). The sweep still acts; the watcher only tells.

### Setup (the human runs two commands)

`proto/scripts/gh-watch forward-command` only prints them (it runs nothing); an agent must never run them, because they need your `admin:repo_hook` scope and a token:

```
gh auth refresh -s admin:repo_hook
gh webhook forward --repo=hslee16/klatch --events=pull_request,check_run,workflow_run,push,issues,issue_comment \
  --url=http://127.0.0.1:8787/hook --secret "$(cat ~/.local/state/klatch-gh-watch/secret)"
```

Before the second, create the secret file once (printed by `forward-command` too): `(umask 077; mkdir -p DIR; python3 -c 'import secrets; print(secrets.token_hex(32))' > DIR/secret)`. The receiver refuses a secret file that is missing, empty, owned by someone else or readable by group or others, and it reads the secret only from that file (never the environment, an argument or stdin). `gh webhook forward` takes the secret only as `--secret VALUE`, so it is visible in `ps` to your own user while it runs; that is the forwarder's limit, not the receiver's.

Durable way (#1343): run both under launchd so they outlive Claude Code sessions (background tasks are owned by the Claude Code process and die with it, and nothing restarts them on a reboot or `--resume`). `proto/scripts/gh-watch install-launchd` PRINTS two LaunchAgent plists (`dev.klatch.gh-watch-receiver`, `dev.klatch.gh-watch-forwarder`) with absolute paths, `RunAtLoad`, `KeepAlive` and a 30 second `ThrottleInterval`, logs in the state dir, plus the `launchctl bootstrap gui/$(id -u) PLIST`, `launchctl print` and `launchctl bootout` commands. It writes nothing, runs no `launchctl` and no `gh`; you save the plists under `~/Library/LaunchAgents` and run the commands. No plist holds the secret: the receiver reads the mode-0600 secret file, and the forwarder job runs `proto/scripts/gh-watch-forward`, a small reviewed script that refuses to run unless the secret file exists with mode 0600, never prints the secret, and ends by exec of `gh webhook forward ... --secret VALUE` (its only `gh` call; the secret stays visible in `ps` to your own user until #1333 is settled). Everything is loopback only. `gh-watch install-launchd --systemd` prints the same as two systemd user units for Linux. `gh-watch status` also prints whether each launchd job is loaded (read-only `launchctl print`, skipped where `launchctl` is missing); those lines do not change its exit code.

Fallback, when you do not want launchd: run the receiver detached (state dir `~/.local/state/klatch-gh-watch`, never `~/.klatch`; override with `--out-dir` or `GH_WATCH_DIR`):

```
nohup proto/scripts/gh-watch serve > ~/.local/state/klatch-gh-watch/serve.out 2>&1 &
tail -F ~/.local/state/klatch-gh-watch/gh-watch.log
```

Stop the receiver with `kill -9 PID` (the pid is in `gh-watch.pid` in the state dir); with the watcher claim on (below), `kill -TERM` of the `serve` wrapper is the clean stop that releases it, and `kill -9` leaves it to lapse. `proto/scripts/gh-watch selftest` runs the fixture tests; they use hand-written payloads and a receiver on an ephemeral loopback port.

To also post each note to a klatch room, add `--klatch-bin /path/to/klatch --as-member NAME --room ROOM` (optionally `--post-min ALERT`). The post is `KLATCH_NAME=NAME klatch post ROOM LINE` with whatever `KLATCH_HOME` the caller has, so start it with the isolated factory store's home (FACTORY.md), never the live store.

### The watcher claim (#1338)

With `--klatch-bin`, the receiver holds a named claim `watch:gh` in the room it posts to (the isolated status room, with the `KLATCH_HOME` the post already uses). Any session then sees the watcher in `klatch claims ROOM` and on the board, and when the receiver dies without releasing, `klatch stuck ROOM` reports a `lapsed_claim` for `watch:gh` (#1339), which is the dead-watcher signal.

- **Holder:** each receiver holds the claim under its own name, `MEMBER.PID` (the `--as-member` name cut to 30 characters, then the receiver's pid). A klatch member name is an unverified label (1 to 40 of `A-Za-z0-9._-`), so no credential is issued or read; the claim row shows which receiver pid holds it. Posts still go out as plain `MEMBER`.
- **Interval and TTL:** the receiver renews every 30 seconds (`--claim-renew`) with a 90 second lease (`--claim-ttl`). The lease is three intervals, so two renewals can be missed (a slow klatch call, a busy store) without a false lapse, and a dead receiver shows within 90 seconds. Renewal is `klatch claim ROOM watch:gh --ttl 90 --shared` again, like an `ask:N` claim; a stall renews once, never in a burst. Both are bounded (`1 <= renew < ttl <= 300`).
- **Shared:** the claim is `--shared`, so it conflicts with every scope. It only guards one store: a receiver with a different `KLATCH_HOME` is a different store and is not refused.
- **Second receiver:** it binds its port first (a taken port already refuses it), then claims. A claim conflict (the real binary prints `Error: refused: ...` and exits 1, after a `klatch: this is ..., but the installed klatch is ...` line when the settings name another binary (#513); the receiver matches `Error: refused: ` at the start of any stderr line, never mid-line) makes it log one ALERT line, print one line to stderr and exit 2 without taking or releasing anything, because its holder name differs from the first receiver's. That keeps a second receiver from running on a store that already has one; it is not a retry: `ensure` prints the reason (`refused because another receiver ... holds the watch:gh claim`, lapsing within 90 s), reports `receiver: down` (exit 3) and does not retry by itself, so rerun it after the lease lapses (#1601). It waits for the claim verdict before it reports, because the receiver binds its port before it claims. A refusal later stops it the same way: if a renewal is refused, the receiver retries once at once, and when that is refused too (another receiver took the claim) it releases nothing, logs the lost ALERT, prints one line and exits 2. Any other claim failure (klatch missing, store busy) is not a refusal: the receiver keeps serving and retries each interval. Renewal is timed on the wall clock (the monotonic clock stands still while a Mac sleeps): after a sleep longer than the 90 second lease the claim lapsed, and the first tick after waking claims it again (or exits 2 if another receiver took it). A clock set back renews at once. After a `kill -9` the dead receiver's claim blocks a restarted one (a new pid) until it lapses, at most 90 seconds; a supervisor that restarts it just retries.
- **Log lines (fixed templates, no klatch text):** `INFO watcher claim watch:gh taken`; `ALERT watcher claim watch:gh not held: klatch failed, retrying` (also when the binary is missing); `ALERT watcher claim watch:gh lost: another holder has it, retrying`; `ALERT second receiver refused: another holds the watch:gh claim`. A line is written when the state changes, not on every renewal. These go to `gh-watch.log` only, not to the room.
- **Never the live store:** with `--klatch-bin`, the receiver refuses to start (exit 2, one stderr line, nothing run, no port opened) when `KLATCH_HOME` is unset or is, or has a component named, `.klatch`, compared case-folded (the filesystem may be case-insensitive) and after resolving symlinks; klatch with no `KLATCH_HOME` is the live store for a post as much as for the claim, so `--no-claim` does not lift this. Set `KLATCH_HOME` to the factory store's home (`gh-watch ensure` and the launchd plist do not pass it yet; tracked separately).
- **Clean stop:** SIGTERM, SIGHUP and SIGINT (Ctrl-C) stop the server and then `klatch release ROOM watch:gh`, so a deliberate stop (launchd `bootout`, the `serve` wrapper passing SIGTERM) leaves no lapse and no alarm; a stop during the first claim still releases and `listening` is not printed. No renewal follows the release, and a renewal still running when the stop arrives is released again. `kill -9` or a crash skips the release; the claim lapses after at most 90 seconds, and that is the signal.
- **Off:** `--no-claim`, or no `--klatch-bin`. The claim text is fixed (reason `gh-watch receiver`); no webhook text reaches it.

### The watcher restart rule (#1342)

`proto/scripts/gh-watch-supervise` restarts a dead local receiver with `gh-watch ensure`, and does nothing else. It needs no credential: it reads the receiver's pid file and the `watch:gh` line of `klatch claims ROOM` (held while it says `Ns left`, N above 0), in the isolated status store (`KLATCH_HOME` unset or with a `.klatch` component is refused, exit 2). Run it with the same `--out-dir`, `--port`, `--klatch-bin`, `--as-member` and `--room` the receiver has; they are handed to `ensure` as they are. It never starts the forwarder, never installs launchd units, never runs `gh`, never kills anything.

- **Placement:** a new small script, not a mode of `ask-supervisor` or `team`. `ask-supervisor` is about open asks, issued helper identities, the human-run admin `could-answer` check on the live store and a spend cap; `team` is about issued worker credentials. This rule needs none of that and must never reach it, so extending either would carry the credential and live-store surface into a keyless job. It reuses the pattern of `ask-supervisor` (read-only checks, bounded polls, an off file, fixed-template lines) and the receiver's own log, which the session Monitor already tails.
- **Decision per pass:** while the receiver is alive the supervisor records its holder name (`MEMBER` cut to 30 characters, then the pid, as the receiver builds it). Only a claim or lapse by THAT holder is ours: another member's `watch:gh` claim, or a second receiver's on the same store, is never crash evidence (with no holder recorded yet, our member name with any pid counts). Pid alive and our claim held is healthy. Pid alive and claim lapsed (a hung receiver) is an ALERT, never a restart. Pid dead and our claim still held is the `kill -9` window (#1601: `ensure` reports down for up to 90 seconds because the dead claim has not lapsed and the new receiver is refused by it): that is the crash evidence, and the supervisor WAITS for the lapse and runs nothing, however often it polls. Pid dead and another holder's claim is an ALERT, not a restart. Pid dead and claim lapsed runs `ensure` only with fresh evidence: our claim was seen held with the pid dead within `GHWS_EVIDENCE_SECS` (180), or `klatch stuck ROOM` shows a `lapsed_claim` for `watch:gh` by our holder younger than `GHWS_LAPSE_MAX_AGE_SECS` (600), which works at any poll interval (a cron `--once`, a 120 second poll) and after earlier restarts. A clean stop releases the claim at once and leaves no evidence, so a receiver the human stopped stays stopped. The flag holds time and holder, is dropped when stale, when the holder differs, and whenever a live receiver pid is seen; a loop run clears old evidence at startup (`--once` keeps it, it expires by itself); every attempt extends it so a failed `ensure` is retried on the backoff.
- **One at a time, own session, bounded calls:** a lock symlink in the state dir (target: the pid; a dead holder's lock is taken over) lets one supervisor run per state dir, so the restart count and the pid file are never raced: a second loop exits 1, a second `--once` pass exits 0 doing nothing. `ensure` runs in its own session (`setsid`, else `perl POSIX::setsid`, else a job-control subshell; macOS has only the perl form), so Ctrl-C or TERM to the supervisor's process group does not stop the receiver it started. Every `klatch claims` and `klatch stuck` is killed after `GHWS_KLATCH_TIMEOUT_SECS` (15) with an ALERT and no restart. `KLATCH_HOME` must already hold `rooms.db` (klatch would create a store in a missing home; exit 2), and a home at or under the real path of `~/.klatch` is refused like the receiver does. Numbers with a leading zero are decimal.
- **Bounds:** at most `GHWS_MAX_RESTARTS` (3) `ensure` runs in any `GHWS_WINDOW_SECS` (3600), the wait before the next one starting at `GHWS_BACKOFF_SECS` (60) and doubling (60, 120, 240; at most 600). An attempt counts whether or not it worked, so a receiver that dies again at once waits too. When the next restart is due and 3 are already in the window, it logs one ALERT (written once, not on every `--once` pass), `run gh-watch ensure yourself`, and exits 3. The window is rolling, so a later run may restart again once old attempts age out. The evidence stays until the restarted receiver holds the claim again, so a failed `ensure` is retried on the backoff.
- **Off:** while `GHWS_OFF_FILE` (default `~/.klatch-gh-watch-supervisor-off`) exists nothing is restarted. If `klatch claims` fails (store busy, a credential would be needed) it logs an ALERT and restarts nothing: it reports instead.
- **Log (fixed templates, a line per change of state, in `gh-watch.log`):** `INFO watcher supervisor: receiver pid is dead, waiting for the watch:gh claim to lapse`; `INFO ...: claim lapsed and receiver pid dead, running gh-watch ensure (attempt N of M in the last W s)`; `INFO ...: receiver restarted by ensure`; `ALERT ...: ensure did not bring the receiver up, next try in S s`; `ALERT ...: restart limit reached (N in the last W s), giving up; run gh-watch ensure yourself`; `ALERT ...: claim state unreadable, not restarting`; `ALERT ...: receiver pid is alive but the watch:gh claim lapsed; not restarting (...)`. Only numbers it counted itself appear; claim, room and webhook text is never logged or run.
- **Run it:** `KLATCH_HOME=... proto/scripts/gh-watch-supervise --klatch-bin PATH --as-member NAME --room NAME` (foreground; detach it yourself). `--once` is one pass. Tests: `proto/scripts/test-gh-watch-supervise.sh` (a fake klatch and a fake gh-watch, the clock a file, no sleep).

### At the start of a session (#1335)

Every coordinating Claude session (one that merges, sweeps or answers for the open pull requests) starts the watcher routine before anything else:

1. Run `proto/scripts/gh-watch ensure`. (The durable setup is the launchd jobs from `gh-watch install-launchd`; `ensure` is the `nohup` fallback and does not start a second receiver while one is up; when a start is refused or fails it says why and exits 3, it never retries.) It creates the state dir (mode 0700) and the secret file (mode 0600) if they are missing, starts ONLY the receiver, detached (`nohup`, output in `serve.out` in the state dir), when it is down, and then prints the status. It never starts the forwarder and never runs `gh`; when the forwarder is down it prints a reminder to run `gh-watch forward-command`, and the human starts it (an agent only when the human explicitly asked for that in the same session). `proto/scripts/gh-watch status` prints the same report and runs nothing else.
2. Arm a Monitor on `tail -F ~/.local/state/klatch-gh-watch/gh-watch.log | grep --line-buffered -E 'ALERT|INFO'` (use the state dir the `ensure` output names if you set `GH_WATCH_DIR` or `--out-dir`). ALERT lines are what you report to the human; INFO lines are context. A Monitor expires after 30 minutes: re-arm it when it does.
3. Keep the slow poll as the backstop (see Limits below): events are missed while the forwarder or the receiver is down.

`status` prints the receiver (up or down), whether a `gh webhook forward` process for this repository exists, the age of the last line in `gh-watch.log`, and a last line that is `gh-watch: ok` (exit 0), `gh-watch: receiver down` (exit 3) `gh-watch: forwarder down` (exit 4) or `gh-watch: forwarder down: stale webhook` (exit 5, see "A stale webhook" below). The receiver counts as up only when the pid in `gh-watch.pid` (written by `serve`, removed when it ends) is alive AND a TCP connect to `127.0.0.1:PORT` succeeds: the pid file alone goes stale after a crash, and the port alone would count any other program on it. The forwarder is found with `ps` and only a yes or no is printed, never its arguments, because the secret is on its command line. The age is that of the log file's last write, one write per line.

A SessionStart hook that runs `gh-watch ensure` is not part of this: installing hooks belongs to `klatch init` and the hook installer.

### A stale webhook (#1350)

`gh webhook forward` creates a development webhook (name `cli`, forwarding URL on github.com) on the repository and deletes it on a clean exit. Stop the forwarder with `launchctl bootout gui/$(id -u)/dev.klatch.gh-watch-forwarder` (SIGTERM; the wrapper passes it to gh, which removes the hook), never with `kill -9`, a crash or a reboot: those leave the hook, and the next `gh webhook forward` fails with `HTTP 422: Hook already exists on this repository` and exit 1.

`gh-watch-forward` runs gh as a child (not an exec) so it can read gh's stderr. On exit 1 with `Hook already exists` it deletes nothing: it writes one line (`stale webhook on the repository: run gh-watch clean-hooks`) to its stderr (`forwarder.launchd.err`), writes the marker `forwarder.stale` in the state dir and exits 75, so launchd's `ThrottleInterval` spaces the retries. Any other end removes the marker. `gh-watch status` then ends with `gh-watch: forwarder down: stale webhook` (exit 5; a running forwarder wins over a leftover marker).

`gh-watch clean-hooks` runs one read-only `gh api repos/OWNER/REPO/hooks` (only when you run it; never `status`, `serve`, `install-launchd`, `forward-command` or the receiver), lists hooks named `cli` whose URL is the GitHub forwarding service, and PRINTS `gh api -X DELETE repos/OWNER/REPO/hooks/ID` for you to run after you stopped the forwarder. There is no flag that deletes: another machine's live forwarder owns a hook that looks identical, so deleting is the human's call.

### What the receiver accepts and writes

Loopback only (any other `--bind` is refused until #157/#161). Each request must be `POST /hook` with a valid `X-Hub-Signature-256` (HMAC SHA-256 over the body), a body of at most 1 MiB (`--max-body`), and a known event; events outside the allow-list (`pull_request`, `check_run`, `workflow_run`, `push`, `issues`, `issue_comment`, `ping`) are answered `ignored`, and a repeated `X-GitHub-Delivery` is answered `duplicate` and dropped. All payload text (titles, branch names, logins, labels) is data: control and format characters become spaces, fields are cut at 80 characters, a note at 300, and nothing is run or put into a command.

Each line of `gh-watch.log` is `TIME SEVERITY text`, written and flushed one line at a time:

| Event | Note |
|---|---|
| `pull_request` opened, reopened, ready_for_review, synchronize, closed (merged or not) | INFO |
| `pull_request` with `mergeable_state` `dirty` on an open PR | ALERT: the PR conflicts with its base |
| `check_run` or `workflow_run` completed `failure`, `timed_out` or `startup_failure` on the default branch | ALERT: main is red |
| the same on a branch with an open pull request that was not last seen as a draft | ALERT: a check failed on PR #N |
| `push` to the default branch | INFO: `main moved to SHA; open PRs may now conflict` |
| `issues` opened or labeled | INFO |
| `issue_comment`, `ping` | accepted, no note (comment text is the least trusted input) |

### Limits

- Webhooks are missed while the forwarder or the receiver is down, and `gh webhook forward` does not replay. Keep one slow poll as the backstop (for example `/loop 30m proto/scripts/pr-sweep --dry-run`); the watcher is awareness, the sweep is the source of truth.
- A conflict is only known when GitHub computes `mergeable_state`, which is often `null` in the first events after a push. A PR that turns conflicting after `main moved` may only be reported by a later event or by the poll.
- A failed check is reported whether or not it is a required check (the payload does not say which are required and the watcher may not ask GitHub). A pull request never seen by the watcher is treated as not a draft; draft memory is lost on restart.
- The notes are one line each, deduplicated only by delivery id, so a flapping check reports each failure.

### How it relates to the klatch-server webhook intake and `klatch anchor github`

It uses neither. Those ask for the human passphrase (#1267) and write into klatch; this watcher only reads forwarded events and notifies, with no key, no store access and no credential of its own.
