# Run three: a real blocking dependency

Follows run two (`RESEARCH-NOTES.md` section 19), where nobody needed to ask because the dependency resolved itself.
Ticket #195, epic #180. Same method as run two (headless sessions, hook wrapper logging every call, absolute paths, no
shell variables), with these differences.

## Scenario
| Agent | Repository | Task | Blocked on |
|---|---|---|---|
| `alpha` | `api` | `cli.py` printing `slugify(title)` for its first argument, with tests | `slugify`, owned by someone it does not know, in a place it does not know |
| `beta` | `lib` | Owns `lib`. Ticket LIB-1 first: `wrap(text, width)` in `text.py` with thorough edge-case tests. Then LIB-2: `slugify` | Nothing; busy for a while, does not know anyone needs `slugify` |
| `gamma` | `docs` | README for the `lib` package, from what it really provides | The location and API of `lib`, owned by someone it does not know |

alpha and gamma are blocked from the start and must find the owner through the lobby. beta is mid-task when asks arrive,
which tests whether the hook interrupts it usefully and whether it answers.

## Differences from run two
- alpha and gamma are not told where `lib` is or who owns it, and are told not to search the filesystem for other
  repositories: ask the lobby. Both are granted read access to `lib` (`--add-dir`) and told not to edit it.
- beta has a longer first task, so a reply arrives while it works.
- The same convention text as run two (`seeking:`, `offering:`, claim `ask:<name>` before answering, `dm-<a>-<b>` rooms).

## Recorded
Verbatim asks and replies; time from ask to first answer; replies per ask (the herd); whether claim-the-ask held; rooms of
two; lobby content that should have been in a room of two; whether alpha or gamma bypassed the lobby; hook log
(delivery, size, repeats after #194); stale asks left at the end; edits outside an agent's own repository (checked
with `git status` in all three).

## Success and failure
Success: `slugify` exists and is tested, `cli.py` runs, the README matches `lib`, no edits outside own repositories. A
failure is a finding; stop only for a safety issue.
