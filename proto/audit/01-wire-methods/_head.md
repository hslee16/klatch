## Wire methods (PROTOCOL.md section 5)

Every request is dispatched in `session/methods.rs` (`handle`) or `session/mod.rs` (`request`), always as the
transport's principal; a `member`, `sender` or `from` param has no effect.

| Wire method | Gateway call | Operation the guard sees | Proof |
|---|---|---|---|
