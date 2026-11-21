| `subscribe` | `head`, then `Subscription::poll` per tick (`tail`) | `Subscribe` (every poll) | `zero_trust.rs` GUARDED `head`, `tail`; `session/tests/live.rs` |
