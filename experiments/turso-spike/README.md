# Turso spike (ticket #620)

A Turso Database (the Rust rewrite of SQLite) backend behind klatch's `Store` trait, to learn whether it passes the
conformance suite, survives several processes and encrypts at rest. It does not ship and changes no default build,
dependency set, release artifact or CI job: it is its own crate with its own `[workspace]` and `Cargo.lock`, path
depends on `klatch`, and is not built by CI. Findings: `research/notes/620-turso-spike.md`.

```
export CARGO_TARGET_DIR=/some/scratch/dir     # keep build output out of the repository, delete it afterwards
cargo test                                    # the klatch conformance suite (included by path) on TursoStore
cargo run --release -- inspect DIR            # what the files look like, cross-engine opens, wrong key
cargo run --release -- bench DIR 300          # post, read, wait, tail against the SQLCipher store
cargo run --release -- race DIR turso-mp 16 300   # 16 processes on one file (kinds: sqlcipher turso turso-plain turso-mp)
cargo run --release -- migrate DIR            # SQLCipher store to Turso through the Store trait
```

`cargo test` is expected to report 65 passed and 43 failed: the failures are the areas `TursoStore` does not port.
It uses encrypted files in the temp directory (`TURSO_SPIKE_DIR` picks another; `TURSO_SPIKE_PLAIN=1` turns
encryption off). The SQLCipher side of the measurements uses klatch's public test key, never a real key or the live store.
