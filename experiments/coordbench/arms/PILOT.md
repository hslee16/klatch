# Pilot plan for the arms (ticket #1365): for the human to run, not run by the builder

Purpose: before any M0 money is spent, check on the trivial fixture that each arm's mechanics work, in particular that B1's
native messaging works under `claude -p`, that K2 joins its room, and that nothing leaks between arms. Nothing here has been
run. Real agents cost money and need your approval (SPEC section 8); `run.py` refuses `--driver claude` without
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

- Close your other interactive Claude Code sessions (they are visible to `ListAgents`, see leak checks below), and run
  nothing else with `claude` during the pilot.
- `claude --version` must be 2.1.248 or later (cross-session messaging on every provider, docs availability section); the
  builder's machine has 2.1.330. Record the version.
- Build klatch (the K arms need the binary): `cargo build` in `proto/`, then the binary is `proto/target/debug/klatch`.
- `ANTHROPIC_API_KEY` set in the shell that runs the harness (the harness passes it to agent processes only).
- Note the state of `~/.klatch` and `~/.claude` (for example `ls -laR ~/.klatch | shasum`) to compare afterwards.

## Part 1: messaging probe (does B1 messaging work under `-p`)

The trivial fixture's two tasks are independent, so B1 agents may never message anyone, and part 2 alone would prove nothing
about messaging. This probe forces one message each way, with the same flags the driver gives a B1 agent. Run it from an empty
scratch directory:

    P=$(mktemp -d) && cd "$P" && mkdir home cfg
    FLAGS=(--model claude-haiku-4-5-20251001 --setting-sources local --no-session-persistence
           --permission-mode acceptEdits --allowedTools SendMessage ListAgents "Bash(sleep:*)"
           --max-turns 12 --max-budget-usd 0.25 --output-format stream-json --verbose
           --settings '{"crossSessionInbound":"accept"}')
    export HOME=$P/home CLAUDE_CONFIG_DIR=$P/cfg      # in this shell only; open a fresh shell afterwards
    claude -p "You are b. Run sleep 20 three times, one after another. If a colleague messages you, reply with SendMessage to that colleague with the text PONG. Then print DONE b." --name b "${FLAGS[@]}" > b.jsonl &
    sleep 15
    claude -p "You are a. Run ListAgents, then send b the message PING with SendMessage, then run sleep 20 once and print what you received. Then print DONE a." --name a "${FLAGS[@]}" > a.jsonl
    wait

`--allowedTools` is variadic, so the list must be followed by another flag, as here. The flags are the real driver's flags
for a B1 agent (see `drivers.py`). Because HOME is changed, the probe needs `ANTHROPIC_API_KEY` in the environment, not a
subscription login.

Look for, in `a.jsonl` and `b.jsonl`:

- `ListAgents` and `SendMessage` are in the `tools` list of the init event, and `a`'s `ListAgents` result names `b`.
  If `ListAgents` shows nothing, messaging is not working under `-p` here (the docs say a `-p` session binds an inbox socket
  and is listed; `--bare` would prevent that, and the driver does not use it).
- `a`'s `SendMessage` result does not begin `Not sent` or `Not delivered`; `b`'s log shows a `Message from` entry and a PONG
  reply; `a` receives the PONG.
- No held or expired notice (a message held by the default inbound policy would expire after 5 minutes; the driver sets
  `crossSessionInbound` to `accept` to avoid it).
- Whether HOME and CLAUDE_CONFIG_DIR being per run, and `--no-session-persistence`, still let the two sessions see each other
  (the docs do not say how the socket is located, only that it lives in a per-user directory; unknown until this probe).

## Part 2: one run per arm on the trivial fixture

    python3 experiments/coordbench/run.py --fixture trivial --suite-id pilot-1365 --arm B0,B1,K1,K2 --runs 1 \
        --driver claude --model claude-haiku-4-5-20251001 --claude-bin "$(command -v claude)" \
        --klatch-bin "$PWD/proto/target/debug/klatch" --logs-dir /tmp/coordbench-pilot-logs \
        --max-run-usd 0.50 --max-suite-usd 2.00 --max-turns 30 --wall-s 600 \
        --arm-text-version proposed-1 --budget-approved-in "ISSUE-COMMENT-URL"

Run `--dry-run` with the same arguments first; it prints the plan and the range and starts nothing.

Look for, in `experiments/coordbench/results/pilot-1365/` and `/tmp/coordbench-pilot-logs/`:

1. Outcomes: all four runs `ok`, with both checks passing (the fixture is trivial; a failure means the mechanics, not the
   model, are wrong). Spend per run against the $0.05 assumption; record it in `cost_model.json` if it is very different.
2. B1 mechanics: `SendMessage` and `ListAgents` in the init tools of B1 agents; any `SendMessage` call and its result; board
   lines added to `TASKS.md` in origin, in the format `- NAME | working | FILES | TITLE`; a board merge conflict handled.
3. K2 join: each K2 agent ran `klatch init`'s hooks (hook events in the stream), joined the room, posted, and the run's
   store shows both members; K1 shows posts without hook events.
4. Leaks between arms (all should be none):
   - B0, K1 and K2 init tool lists contain neither `SendMessage` nor `ListAgents`; a B0 or K agent never calls them.
   - The built prompts (kept in the logs directory) of B0 and B1 contain no `klatch`, `room` or `lobby`.
   - B1's `ListAgents` result lists only `a` and `b`, never one of your own sessions or an agent of another run.
   - `~/.klatch` and `~/.claude` are unchanged (compare with your note); the klatch store of K arms is under the run's
     scratch, which is removed after the run.
   - No process of a finished run is left (`ps` for `claude` and `klatch`); no stray files in `/tmp/cc-socks-*` beyond the usual.
5. Cost of coordination: B1 tool-schema tokens show as coordination context; K2 hook text shows as coordination context.

If any B1 messaging check fails, B1 is not yet the fair baseline: the fallback in SPEC section 3 (board file plus a shared
append-only message file, limitation recorded in every B1 result) applies, and the B1 text needs a change. Tell the builder.
