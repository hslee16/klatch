The asker closes an ask with an outcome: `closed #N ok` or `closed #N unresolved` (#232; no schema change; 2026-11-21).
Decision made by the human on 2026-11-21: add the two outcome words to the close, with no retry limit for now. `reopen #N: why` was built earlier (design/items/232-reopen-ask.md).

    The words. `closed #N ok` says the answer served; `closed #N unresolved` says the asker gave up without a usable answer; a bare `closed #N` still closes the ask and states no result. The outcome is the first word after the number (lower-cased, trailing punctuation ignored, and it needs a space after the number); any other word is a bare close. All three end the ask exactly as before, only the asker's own close counts, and a later `reopen #N:` puts the ask back, so the latest close after the latest reopen is the ask's outcome. The words are read from the room's messages: no new state, wire method, operation or schema, so no AUTHZ-AUDIT row changes.

    Where it shows. An ended ask is not on the board, `klatch asks` or stuck, so the outcome is read where ended asks are read: `klatch find` marks an answered ask `(closed ok)` or `(closed unresolved)`, so an asker does not trust an answer the earlier asker gave up on. The domain reads it with `close_outcome` and `latest_close` (domain/asks.rs).

    Not built. A limit on retries (the human said none for now). The member prompt is at its byte bound and does not teach the outcome words. The recorded real-agent run (an insufficient answer leading to a second helper attempt) is left.
