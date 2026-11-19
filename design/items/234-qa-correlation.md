Questions and answers stay correlated by text; there is no reply-to field (#234; no schema or wire change; 2026-11-19).
The ticket asked for an optional `in_reply_to` and `kind` on `post` and the stream, or a recorded decision to stay with text conventions. The owner had recorded only that structured fields "wait for Q43 and #234" (section 25.4), so this item chooses the smallest option consistent with DESIGN.md, which is to stay with text and to fix it in tests; it is a default, not a ruling, and the human may reopen it.

What main already has, found before building. `kind` and `payload` exist on `post` and the stream for handoffs (PROTOCOL.md, DESIGN section 32 item 16) and are opaque to klatch. A direct message exists (`dm`, #43): a two-member room `dm.<a>.<b>`. The wire has no reply-to or thread field, and the `reply_to` column in the section 8.2 sketch was never built.

The decision.
- Correlation is the ask's `seq` in the text: `answer to #<seq>:`, `closed #<seq>`, `need-info #<seq>:`, `ask:<seq>` claims. A `seq` is dense and stable per room, so the number is the reference. No `in_reply_to` is added to `post` or the stream.
- A direct question to one named member is a `dm` whose body starts `seeking:`. Open asks are derived per room, so it is never a lobby ask and no one else sees it; inside the DM room the same `answer to #<seq>:` text ends it, and `seq` is the DM room's own. This answers Q40 for asks; whether a wake rule should fire on "addressed to me" stays open there.
- Several answers to one ask: every answer is an ordinary message and reaches the asker, in order. The first answer from someone other than the asker ends the ask (section 25.4); later answers change nothing and are not dropped. There is no "best answer" and no vote.
- `kind` stays what it is, the handoff kind. It is not a message-type vocabulary for questions, because `seeking:` and the other words already carry that and a second channel for the same meaning would disagree with the text.

Why not the field. It needs a schema change (a column, a migration), a wire addition on `post`, `read`, `stream` and the MCP tools, and a rule for a `reply-to` that names a seq in another room; the text convention costs none of that and already works for every harness that can post a body. A harness that wants a machine-readable link can put the number in a handoff `payload`, which klatch carries and does not read.

Proof. `gateway/tests/qa_correlation.rs`: several answers are all delivered and the first ends the ask; a direct question is a DM, not a lobby ask, and is answered by the same text inside the DM room. Both tests describe behavior main already had, so they passed when first written (not run red first); they fix the chosen correlation so a later change shows.

Not done. A structured reply-to remains possible later as an additive optional field (a new schema version and a wire addition) if text correlation proves too fragile in use; ticket #944 tracks that evidence.
