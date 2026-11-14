# Crypto spike (tickets #153, #154)

A throwaway crate that measures plain SQLite, SQLCipher with a raw 256-bit key, and per-field envelope encryption
(ChaCha20-Poly1305) on the same workload, and greps the raw database file for plaintext. Results are in DESIGN
section 32. It is not part of the workspace.

```
cargo build --release --features plain      && ./target/release/spike plain /tmp/p.db && ./target/release/spike envelope /tmp/e.db && ./target/release/spike open /tmp/o.db
cargo build --release --features sqlcipher  && ./target/release/spike sqlcipher /tmp/c.db && ./target/release/spike open /tmp/o2.db
```
