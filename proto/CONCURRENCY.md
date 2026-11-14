# Concurrency: what is tested, what was found, what was measured

Ticket #18. Scope: many processes using one SQLite store through the CLI and hooks, with no daemon
(DESIGN section 19 D6 and the question 17 in section 14).

## What the tests cover

| Test | Level | Checks |
|---|---|---|
| `concurrent_posters_get_dense_unique_seqs` | 8 connections | 200 posts give exactly seq 1 to 200 |
| `concurrent_claims_on_one_path_have_exactly_one_winner` | 8 connections | one claim wins, the rest get `Conflict` |
| `concurrent_claims_on_disjoint_paths_all_succeed` | 8 connections | no false conflicts |
| `readers_and_writers_together_lose_and_duplicate_nothing` | 4 writers, 3 readers | each reader sees all 100 messages once, in order |
| `many_connections_opening_a_fresh_database_at_once_all_succeed` | 12 connections, 30 rounds | first-time opens never fail |
| `a_held_write_lock_fails_a_write_after_the_timeout_and_never_hangs` | 2 connections | a held lock gives `Busy` within the limit, and leaves no seq gap |
| `pure_reads_proceed_while_another_connection_holds_the_write_lock` | 2 connections | `claims` and `members` do not block on a writer |
| `the_cursor_advances_to_the_last_message_returned_never_past_it` | 1 connection | deterministic guard for the message-loss bug below |
| CLI: sixteen processes posting, sixteen claiming one path, readers racing writers | real processes | the same properties through the binary |

The full suite was run 80 times in a row after the fixes with no failure.

## Bugs found by these tests, both fixed

1. **Message loss in `read`.** The cursor advanced to the room's current maximum `seq` in a separate query after the
   select, so a message posted in between was skipped permanently. The hook path uses this. It now advances only to
   the last message actually returned.
2. **First-time opens failed under contention.** With many processes starting at once on a new database, some
   failed with "database is locked" while switching to WAL mode or creating the schema, which SQLite's busy handler
   does not always cover. `open` now retries on contention up to the timeout, and `Error::Busy` marks transient lock
   contention so callers (the hooks) can tell it from a real failure.

## Measurements

Release build, Apple M3 Pro (12 cores), macOS 14.3, four writer threads on separate connections, one reader that
runs `read` (which also writes the cursor) and `claims` (a pure read) every 10 ms for 5 seconds. One run per row; the
`measure_hook_path_latency_under_write_load` test prints these (`cargo test --release -p klatch-store -- --ignored
--nocapture`).

| Writer pause | Posts per second | `read` p50 | `read` p99 | `read` max | `claims` p99 |
|---|---|---|---|---|---|
| 100 ms | 38 | 166 us | 1.7 ms | 11.8 ms | 66 us |
| 10 ms | 320 | 260 us | 4.2 ms | 4.3 ms | 70 us |
| 1 ms | 1,982 | 1.5 ms | 119 ms | 560 ms | 106 us |
| none (saturating) | 27,973 | 286 ms | 1.7 s | 1.7 s | 25 us |

## What this means

- **The no-daemon plan holds at the intended scale.** Dozens of agents posting a few messages a minute sit far below
  the first row, and the hook path costs about a millisecond.
- **The hook's `read` degrades under heavy write contention** because it writes (the cursor and last-seen time) and
  queues behind other writers; SQLite's busy handling is not fair. Pure reads stay in microseconds throughout.
  Mitigations are tracked in a ticket (make the hook path read-only where it can, coalesce last-seen writes).
- **Limits of this evidence:** one machine, one run per row, threads for the benchmark (processes for the CLI tests),
  macOS only (CI also runs the tests on Linux), local disk only. A database on a network filesystem is still not
  supported (question 17).

## With encryption (ticket #166, 2026-11-14)

The store is now SQLCipher under a raw 256-bit key. The cost that matters is per process, because every command and
every hook is a new process that opens the store and fetches the key. Release builds, Apple M3 Pro, macOS 14.3, 150
runs of each command against a store of 300 messages in 5 rooms, run one after another as a hook would be. The old
column is the build deployed before encryption (plaintext).

| Command | Plaintext p50 / p99 | Encrypted, key file p50 / p99 | Encrypted, Keychain p50 / p99 |
|---|---|---|---|
| `hook` (nothing to show) | 3.7 / 5.4 ms | 6.5 / 8.0 ms | 20.2 / 32.3 ms |
| `who` | 3.1 / 3.6 ms | 5.8 / 7.4 ms | 19.6 / 31.9 ms |
| `post` | 3.2 / 3.8 ms | 5.9 / 6.7 ms | not measured |

In process (from the spike, `experiments/crypto-spike`): reading 20 messages 0.007 ms with and without SQLCipher; a
5000-row bulk insert about 550k rows a second plain and 485k with SQLCipher; open, key and first query 0.21 ms plain
and 0.34 ms with a raw key. A passphrase run through SQLCipher's default key derivation costs 77 ms per open, which is
why the database key is never derived from a passphrase.

**Budget.** A hook may add at most 50 ms at p99 over its work, so a prompt is never visibly slower. The measured
worst case, the Keychain, is 32 ms at p99, of which about 17 ms is the keystore. Within budget. The first read of the
key by a newly built binary on macOS takes about 5 seconds once (#372); it is a one-time cost of each new build.
