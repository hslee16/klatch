# Run four: delivery works, two agents can answer

Follows run three (`RESEARCH-NOTES.md` section 20), where hook output never reached the model. Built from the branch with `klatch hook --event` (#204) and the separate hook cursor (#198, PR #205).

## Scenario
| Agent | Repository | Task |
|---|---|---|
| `alpha` | `api` | `cli.py` printing `slugify(title)`; does not know who or where |
| `gamma` | `docs` | README for a text-helpers package; does not know who or where |
| `beta` | `lib` | LIB-1 `wrap`, then LIB-2 `slugify`, in `text.py` |
| `delta` | `lib` (same directory) | LIB-3 `truncate`, LIB-4 `title_case`, in `trunc.py`; must not touch beta's files |

beta and delta can both answer gamma's ask, and delta is also in the owner's repository, so the herd (two answerers) and claim-the-ask are possible. Same method as run three (headless, throwaway `--settings`, hook wrapper logging every call, absolute paths). Differences: the hook runs with `--event <event>` for every event; blocked agents were told to wait with `sleep 30` at most six times, then report what they are blocked on.

## Recorded
Lobby verbatim, hook log, file hashes before and after, final report of every agent. See `RESEARCH-NOTES.md` section 22.
