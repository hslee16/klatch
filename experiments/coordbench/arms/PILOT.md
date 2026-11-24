# Pilot plan for the arms (ticket #1365): for the human to run, not run by the builder

Purpose: before any M0 money is spent, check on the trivial fixture that each arm's mechanics work, in particular that B1's
native messaging works under `claude -p`, that K2 joins its room, and that nothing leaks between arms. Only the messaging probe
(part 1) has been run, on 2026-11-23; the rest has not. Real agents cost money and need your approval (SPEC section 8); `run.py` refuses `--driver claude` without
`--budget-approved-in`.

## Budget (from SPEC 8.1 and `cost_model.json`)

- Part 1, the messaging probe (two tiny sessions): about $0.05 to $0.15 on Haiku.
- Part 2, one run per arm on `trivial` (B0, B1, K1, K2; 4 runs of 2 agents each): the dry run prints
  `estimated cost: 0.15 to 0.25 USD` (assumed $0.05 a run on Haiku, range 0.75x to 1.25x). The spec's proposed caps
  ($0.15 a run, $0.25 the suite) are tighter than the estimate's own range allows, so for the pilot the plan proposes
  per-run $0.50 and suite $2.00 as hard stops. Worst case is the suite cap, $2.00.
- On Sonnet the same pilot is about 3 times as much ($0.45 to $0.75 expected; keep the $2.00 cap). Haiku is enough for
  mechanics. The pilot replaces the planning figure for `trivial`; it does not measure the M0 scenarios.
- Total proposed ceiling for part 1 and part 2: $2.50. Name where you approved it (an issue comment link) and pass it as
  `--budget-approved-in`.

## Before you start

- Your other interactive Claude Code sessions need not be closed: the probe of 2026-11-23 (below) showed that `ListAgents`
  lists only the sessions that share the probe's config directory, never the user's other sessions. Run nothing else with
  `claude` that shares the pilot's config directory.
- `claude --version` must be 2.1.248 or later (cross-session messaging on every provider, docs availability section); the
  builder's machine has 2.1.330. Record the version.
- Build klatch (the K arms need the binary): `cargo build` in `proto/`, then the binary is `proto/target/debug/klatch`.
- `ANTHROPIC_API_KEY` set in the shell that runs the harness (the harness passes it to agent processes only).
- Note the state of `~/.klatch` and `~/.claude` (for example `ls -laR ~/.klatch | shasum`) to compare afterwards.

## Part 0: isolation probe (#1519), about $0.10

Run `python3 experiments/coordbench/isolation_probe.py --claude-bin claude --model MODEL_ID --klatch-bin PATH` once before part 2.
It starts one tiny session under the sandbox settings the harness gives real agents and prints what each escape attempt
shows; the expected output is in README.md, "Isolation of agents from each other and from the harness". If a SHOWS line appears
for anything but the control, or `klatch --version` fails inside the sandbox, stop and report; with a K arm broken by the
sandbox only, rerun the pilot with `--isolation layout` and say so. Every record of a run has an `isolation` object, and a
run in which the scan found an out-of-bounds tool call is invalid.

## Part 1: messaging probe (does B1 messaging work under `-p`)

Measured on 2026-11-23 (#1520): three real two-session probes, Haiku, claude 2.1.331, macOS, with the flags the driver gives a B1
agent (`--setting-sources local --no-session-persistence --permission-mode acceptEdits --allowedTools SendMessage ListAgents
"Bash(sleep:*)" --settings '{"crossSessionInbound":"accept"}'`):

1. One shared HOME and one shared `CLAUDE_CONFIG_DIR` for both sessions: `ListAgents` in `a` names `b`, `SendMessage` PING to `b` is
   delivered, `b` replies PONG with `SendMessage`, `a` prints PONG. Messaging works under `-p`. The user's three other interactive
   sessions (other projects) were running and were NOT visible: `ListAgents` showed only the partner.
2. A separate HOME and a separate `CLAUDE_CONFIG_DIR` per agent: `ListAgents` shows no reachable agents and `SendMessage` fails with
   `No agent named b is reachable`. Messaging is BROKEN. (This is how the harness started B1 agents after #1519, which made B1 a
   straw man: B0 plus a board.)
3. A separate HOME per agent and ONE shared `CLAUDE_CONFIG_DIR`: `ListAgents` names `b` and `SendMessage` is delivered. The peer
   registry follows `CLAUDE_CONFIG_DIR`, not HOME.

So since #1520 the agents of one B1 or B1-roles run share ONE config directory (`<scratch>/agents/cfg-TOKEN`, mode 0700, never
`~/.claude`) and keep their own HOME, TMPDIR and clone; every other arm keeps one config directory per agent. A reply by raw socket
path (`uds:/tmp/cc-socks/N.sock`) exists, but the sockets are per process and nothing here uses it.

Result texts seen: `Peer sessions (1): b [eab75d] ... busy` and `No reachable agents`; a failed send says `No agent named b is
reachable`. To repeat the probe (shared config directory, separate homes), from an empty scratch directory:

    P=$(mktemp -d) && cd "$P" && mkdir home-a home-b cfg
    FLAGS=(--model claude-haiku-4-5-20251001 --setting-sources local --no-session-persistence
           --permission-mode acceptEdits --allowedTools SendMessage ListAgents "Bash(sleep:*)"
           --max-turns 12 --max-budget-usd 0.25 --output-format stream-json --verbose
           --settings '{"crossSessionInbound":"accept"}')
    env HOME=$P/home-b CLAUDE_CONFIG_DIR=$P/cfg claude -p "You are b. Run sleep 20 three times, one after another. If a colleague messages you, reply with SendMessage to that colleague with the text PONG. Then print DONE b." --name b "${FLAGS[@]}" > b.jsonl &
    sleep 15
    env HOME=$P/home-a CLAUDE_CONFIG_DIR=$P/cfg claude -p "You are a. Run ListAgents, then send b the message PING with SendMessage, then run sleep 20 once and print what you received. Then print DONE a." --name a "${FLAGS[@]}" > a.jsonl
    wait

`--allowedTools` is variadic, so the list must be followed by another flag, as here. Because HOME is changed, the probe needs
`ANTHROPIC_API_KEY` in the environment, not a subscription login.

The harness now judges this itself in every B1 and B1-roles run (`messaging.py`), and the record has a `messaging_verified` object:
`{"tools_present", "listagents_named_peer", "messages_sent", "messages_delivered", "listagents_calls", "status", "reason",
"config_dir_entries"}`. `status` is `failed` (the run is INVALID, the reason is in `notes`) when an init event that carries a tools
list lacks `SendMessage` or `ListAgents`, or when a `ListAgents` result came back while a colleague had been alive for at least 15 s
(a session binds its inbox a few seconds after it starts) and no `ListAgents` result of the run ever named a teammate; `verified`
when some result named a teammate; `unknown` (the run stays valid) when no `ListAgents` ran while a colleague had been alive that
long, for instance because every agent finished before the next started, or the run has one agent. `config_dir_entries` lists what the
run left in the shared config directory, so the state B1 agents share through it can be read off a real run.

## Part 1b: the B1 trivial run is the harness's proof

The coordinator runs the B1 arm of the trivial pilot (part 2, `--arm B1`) so that the harness proves messaging in a real run. The two
trivial tasks are independent, so an agent may never call `ListAgents`; read `messaging_verified.status` of that record. `verified`
is the proof. `unknown` means the run proved nothing (no `ListAgents` call while the colleague was alive): repeat it, or run the
probe above. `failed` means the shared config directory did not let the sessions see each other: stop and report. Read
`config_dir_entries` as well, and record what the first real run left there.

## Part 2: one run per arm on the trivial fixture

    python3 experiments/coordbench/run.py --fixture trivial --suite-id pilot-1365 --arm B0,B1,K1,K2 --runs 1 \
        --driver claude --model claude-haiku-4-5-20251001 --claude-bin "$(command -v claude)" \
        --klatch-bin "$PWD/proto/target/debug/klatch" --logs-dir /tmp/coordbench-pilot-logs \
        --max-run-usd 0.50 --max-suite-usd 2.00 --max-turns 30 --wall-s 600 \
        --arm-text-version proposed-2 --budget-approved-in "ISSUE-COMMENT-URL"

Run `--dry-run` with the same arguments first; it prints the plan and the range and starts nothing.

Look for, in `experiments/coordbench/results/pilot-1365/` and `/tmp/coordbench-pilot-logs/`:

1. Outcomes: all four runs `ok`, with both checks passing (the fixture is trivial; a failure means the mechanics, not the
   model, are wrong). Spend per run against the $0.05 assumption; record it in `cost_model.json` if it is very different.
2. B1 mechanics: `SendMessage` and `ListAgents` in the init tools of B1 agents; `messaging_verified` of the record (see part 1);
   any `SendMessage` call and its result; board lines added to `TASKS.md` in origin, in the format `- NAME | working | FILES |
   TITLE` (`.gitattributes` makes two appends merge without a conflict, so a `git_conflicts` count on the board is a bug).
3. K2 join: each K2 agent ran `klatch init`'s hooks (hook events in the stream), joined the room, posted, and the run's
   store shows both members; K1 shows posts without hook events.
4. Leaks between arms (all should be none):
   - B0, K1 and K2 init tool lists contain neither `SendMessage` nor `ListAgents`; a B0 or K agent never calls them.
   - The built prompts (kept in the logs directory) of B0 and B1 contain no `klatch`, `room` or `lobby`.
   - B1's `ListAgents` result lists only `a` and `b`, never one of your own sessions or an agent of another run (the probe of
     2026-11-23 showed none of three running interactive sessions).
   - `~/.klatch` and `~/.claude` are unchanged (compare with your note); the klatch store of K arms is under the run's
     scratch, which is removed after the run.
   - No process of a finished run is left (`ps` for `claude` and `klatch`); no stray files in `/tmp/cc-socks-*` beyond the usual.
5. Cost of coordination: B1 tool-schema tokens show as coordination context; K2 hook text shows as coordination context.

If any B1 messaging check fails, B1 is not yet the fair baseline: the fallback in SPEC section 3 (board file plus a shared
append-only message file, limitation recorded in every B1 result) applies, and the B1 text needs a change. Tell the builder.
