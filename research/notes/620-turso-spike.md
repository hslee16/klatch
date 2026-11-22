Turso Database as a Store backend: spike findings (2026-11-21, ticket #620)
Question: could a Turso Database (the Rust rewrite of SQLite, MIT, beta) sit behind the `Store` trait, pass the conformance suite, survive 16 processes and encrypt at rest with a key from klatch's key custody? This was a spike to learn, not to ship: the code lives in `experiments/turso-spike/`, a standalone crate (its own `[workspace]`, its own `Cargo.lock`, path dependency on `klatch`). It is not a member of `proto/`, CI does not build it, and the main `Cargo.lock`, `deny.toml`, binaries and release artifacts are unchanged. The measurements are one run on one machine (macOS, Apple silicon, release build) with small N; read them as orders of magnitude.

### Can it be built here

Yes. `turso` 0.9.0-pre.6 (MIT, pre-release, pinned with `=`) fetches from crates.io and builds with stable Rust; the whole spike with `klatch` builds in about three minutes in debug and the release binary is 32 MB (it links both SQLCipher and Turso). There is no encryption feature flag to enable at build time: encryption is a runtime option, `Builder::experimental_encryption(true)` plus `with_encryption(EncryptionOpts { cipher, hexkey })`. Ciphers in 0.9.0-pre.6: aes128gcm, aes256gcm, aegis128l, aegis128x2, aegis128x4, aegis256. The spike uses aegis256 with the 32 bytes of a klatch `Key` hex encoded, so key custody needs no change to feed it.

Dependencies: `turso` adds about 300 crates to the tree (klatch alone is about 70), including `mimalloc` (a C allocator), `chrono` and `turso_sync_engine`/`hyper`/`tokio` machinery that the `sync` feature gates but the lockfile still resolves. Licences are all permissive (MIT, Apache-2.0, BSD, Zlib, Unicode-3.0, ISC and dual licences that include one of those; `cfg_block` has no licence field). `cargo deny` with `proto/deny.toml` on the spike: licenses ok, bans ok, sources ok, advisories FAILED: `chrono` 0.4.45 is flagged unmaintained (RUSTSEC-2026-0543, "all versions are affected"), pulled in by `turso_core`. The main workspace sets `all-features = true` and denies, so adding the dependency there would fail its `cargo deny` job today. That is the reason the spike is standalone.

### Conformance

The suite is `proto/klatch/src/store/conformance/` (108 behaviours today, not the 90 the ticket counted), wired for SQLite in `sqlite/tests/conformance.rs`. The spike includes both files from the klatch source by `#[path]` and runs them unchanged against `TursoStore` (a file in a temp directory, encrypted with aegis256), so the suite cannot drift from the copy. `TursoStore` ports the required methods and these areas: messages (post, read, look, peek, wait and wake positions, hook read, tail, head, advance cursor), members (list, rooms of, mute), plain claims (conflict, expiry, owner reclaim, scopes), policy, audit, refusals and credentials. It keeps klatch's SQLite schema for the tables it uses. It does not port sealed labels, claim waits and deadlock notes, claim changes, anchors, asks, reviews, sessions, member notes, parking, pruning or room state.

Result: 65 of 108 pass, 43 fail.

- Pass: messages 32 of 33, claims 11 of 23, members 5 of 15, asks 7 of 16, policy 6 of 6, credentials 3 of 3, anchors 1 of 4.
- Fail (43): every failure is a method the spike did not port, so the trait default answered "nothing" or "this store does not keep that" (claims 12, members 10, asks 9, reviews 7, anchors 3, pruning 1, plus the one test that checks a store declares every feature). No failure was traced to something Turso cannot do. The SQL the ported areas use (upsert with `ON CONFLICT`, `BEGIN IMMEDIATE`, scalar `MAX`/`MIN`, `COALESCE`, `substr`, `AUTOINCREMENT`, unique indexes, BLOB columns, correlated subquery) all ran. Porting the rest is mechanical work on the same SQL; it was left out because the spike had answered the questions that matter.
- Not tried, so unknown: `PRAGMA user_version` based migrations and `sqlite_master` introspection (the SQLite store's migration path), and the backup and archive paths (DESIGN item 30, `VACUUM INTO`, `ATTACH`), which in Turso sit behind `experimental_attach` and `experimental_vacuum`.

### Encryption at rest (the point of the spike)

`turso-spike inspect` writes a known marker and looks at the files.

- Turso with aegis256: the file starts with the bytes `Turso` and a version, not `SQLite format 3`; the marker appears in neither the database file nor its WAL; a wrong key fails at open with `Decryption failed for page=1`; opening with no key fails with `file is not a database`.
- Turso without a key: the file is an ordinary SQLite file (`SQLite format 3`) and the marker is plainly visible in the WAL. Encryption is opt-in per file.
- Interchange: klatch's SQLCipher store refuses the Turso file (encrypted or plain) and Turso refuses the SQLCipher file (plain open: `file is not a database`; with a key: `invalid page size in database header`). Neither engine can read the other's encrypted files, as the research expected.
- The header is not hidden: an observer can tell a file is a Turso database. SQLCipher hides the header too. Whether that matters under klatch's zero trust maxim is a human call.
- Turso's encryption is behind `experimental_encryption`, and Turso describes it as experimental.

### Several processes on one file

`turso-spike race` starts 16 processes that each open the same file and post to one room, then a fresh open checks that every post is stored with dense `seq` and unique ids.

- Default Turso open: only the first process can open the file; the other 15 fail at open with `Locking error: Failed locking file ... File is locked by another process`. That breaks every klatch use that has a CLI process, a hook process and a server touching one store.
- With `experimental_multiprocess_wal(true)`: 16 processes by 50 posts and, three times, 16 by 300 posts all succeeded (4800 written, 4800 stored, dense seq, unique ids, no failed post and no busy error). This is a small run, not a proof; the flag name says experimental and the ticket's source said several processes on one file can break. A real verdict needs the bench's 16-process race at its full size, crash injection (kill -9 mid transaction) and a soak, none of which were done.
- SQLCipher store in the same race: 4800 of 4800, 1.14 s; Turso with multiprocess WAL: 1.5 s (three runs, 1.49 to 1.57 s).

### Speed

`turso-spike bench <dir> 300`, microseconds per call, file backed, release build, one process, so single-run noise is large:

| backend | post | read (post then read) | wait poll (nothing new) | tail 20 |
|---|---|---|---|---|
| SQLCipher (klatch store) | 161 | 273 | 57 | 283 |
| Turso, aegis256 | 162 | 249 | 120 | 32 |
| Turso, plain | 201 | 253 | 132 | 29 |
| Turso, aegis256, multiprocess WAL | 171 | 278 | 129 | 31 |

Posts and reads are on par; a wait poll costs about twice as much (the spike's `touch` is a write on every poll; the SQLite store skips it when already seen that second), and a plain read of 20 rows is faster on Turso. After 600 messages the Turso files held about 2.8 MB (database plus WAL) against 316 KB for SQLCipher, probably because the WAL is checkpointed less eagerly (not investigated). A debug build of the suite is slow (about 12 ms a post), so do not judge speed from `cargo test`.

### Migration and archive

Migration from an SQLCipher store through the Store trait works for messages: 500 messages read with `tail` from the SQLCipher store and written with `post_as` to the Turso store in 0.09 s, with `seq`, sender and body identical. Message ids are not kept, because `post_as` mints a fresh id; the trait has no way to import a message with its id, so a migration or an archive restore through the trait changes ids (ticket #1217). The archive and restore items (DESIGN item 30) and the rekey and seal paths are SQLite specific (files, `VACUUM INTO`) and would need a Turso equivalent; they were not attempted.

### Could Rust verifiers cover more

Kani and Shuttle model klatch's own Rust, which is the same on either backend. A Turso backend would remove the C layers (SQLite, SQLCipher, vendored OpenSSL) from klatch's own build, but it adds a large Rust dependency that no verifier here covers (about 300 crates), so the verified surface would not grow. Nothing in the spike changes the existing proofs.

### Recommendation

Stay on SQLite, SQLCipher and OpenSSL, as the human decided on 2026-11-15, and keep #620 as the revisit marker. Reasons, in order of weight: (1) multi-process use needs an experimental flag and was only lightly exercised; (2) encryption is experimental, hides no header and makes files that neither engine reads; (3) the dependency set adds an unmaintained crate (`chrono`) that fails `cargo deny` as configured and about 230 more crates than today; (4) no data on crash safety. In favour: the trait seam held (a second engine passed 65 behaviours with 744 lines of backend and no change to klatch), speed is comparable, and encryption with klatch's own key works. Revisit when Turso's encryption leaves experimental, multi-process WAL is documented as supported, and `chrono` is gone from `turso_core`, or at the pre-1.0 review. To resume: port the unported areas in `experiments/turso-spike/src/turso_store.rs` and rerun `cargo test` there (the suite will say what is left), then `turso-spike race` at full size with `kill -9` injection.

### How to rerun

From `experiments/turso-spike/` (set `CARGO_TARGET_DIR` outside the repository; delete it afterwards): `cargo test` runs the conformance suite on `TursoStore` (`TURSO_SPIKE_PLAIN=1` skips encryption, `TURSO_SPIKE_DIR` picks the directory); `cargo run --release -- inspect|bench|race|migrate <dir> ...` runs the measurements (`cargo run --release -- help` lists them).
