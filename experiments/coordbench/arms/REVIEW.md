# Arm prompt review (SPEC section 2, rule 4; ticket #1365)

Reviewer: PENDING (a person who did not write the arms)
Date: PENDING
Result: PENDING. No arm text is used in a real run before this is signed (SPEC 2.7).

The builder (a Claude agent, ticket #1365) wrote `B0.md`, `B1.md` and `B1-roles.md` and the harness changes; `common.md`,
`K1.md`, `K2.md` and `K3.md` were written earlier for #1363. The builder filled the "Builder's reading" lines from the text
itself. They are a starting point, not a review. Run `python3 experiments/coordbench/arms/check_parity.py` first; it
checks lengths, outline, the shared common prompt, klatch words in B arms and placeholders mechanically.

How to review: read `common.md`, then B0, B1, K1, K2 side by side, then B1-roles beside K3. Then answer each item.

## A. Same outline and same level of detail

1. [ ] Every arm has the six topics in the same order: telling others what you are doing, not stepping on each other's
   work, asking and answering, handing work over, waiting, finishing.
   Builder's reading: yes; `check_parity.py` fails otherwise.
2. [ ] Each topic names exact commands or tools with a worked form, and the worked forms are about equally specific.
   Builder's reading: yes for all four. B0 names `git fetch origin` and `git log origin/main -- PATH`; B1 names the board
   line `- NAME | working | FILES | TITLE`, `SendMessage` and `ListAgents`; K1 and K2 name `post`, `claim`, `release`,
   `claims`, `asks`, `find`, `wait`.
3. [ ] Lengths are within 15 percent of the mean.
   Builder's reading: 321, 334, 270, 277 words (mean 300.5), all within 11 percent. K arms are the shortest (10 and 8
   percent under the mean). Roles: worker, integrator and supervisor sections within 7 percent of each other.
4. [ ] Tone, hedging and politeness are the same; no arm says "carefully" or "must" more than the others.
   Builder's reading: every section opens with "Use it the way a careful team would" or the same sentence adapted; check
   that no arm has extra emphasis.

## B. Where bias could enter (the builder's flags; decide each)

5. [ ] Structure favours klatch. K1 and K2 give message templates (`working:`, `seeking:`, `answer to #N:`, `closed #N`,
   `checkpoint:`, `handoff:`, `ready:`). B1 gives a board line format but leaves messages as free text, so klatch asks and
   answers are easier to parse and to count than B1 messages. Options: (a) leave it, since the templates are part of what
   klatch teaches; (b) give B1 the same prefixes for messages (`question:`, `answer:`, `handoff:`, `ready:`), which
   improves B1's instructions and makes the comparison stricter. The builder prefers (b) if the human agrees, because the
   spec says baselines are not a straw man. Not applied yet: a human decision.
6. [ ] A blocking wait exists in K (`klatch wait`) but B arms poll with `sleep 20` (and a message from a colleague arrives by
   itself between tool calls in B1). That is a real difference in the product, not in the wording. The driver allows
   `Bash(sleep:*)` in B arms only; K arms are told not to poll with sleep. Check this is acceptable.
7. [ ] Claims are enforced refusals in K; the B1 board is advisory (a line saying `working`). Real difference, stated in
   the arm texts, not hidden. Check that B1's text asks for the same behaviour (publish your line before you edit).
8. [ ] B1 states a native limit: a colleague who has finished cannot be reached. K texts say nothing about the equivalent
   (a posted message stays in the room). This is a documented fact about the tool (cross-session-messaging.md) that
   the agent would otherwise learn by failing; check it does not read as a warning that discourages messaging.
9. [ ] Tool definitions: B1 sessions carry the `SendMessage` and `ListAgents` tool schemas; every other arm has them
   removed by a deny rule (driver `settings_for`). That is the correct counterfactual and the tokens are measured as
   coordination context. Check the stream parser counts them for B1 and that K2's hook text is counted for K2.
10. [ ] The K2 prompt has no extra words beyond K1 (SPEC 2.5): its extra is the hooks, measured separately. Compare the two
    files line by line.
11. [ ] Roles: B1-roles mirrors K3 section by section. The integrator and supervisor in B1-roles wait with `sleep 20` and
    `git ls-remote`, and the supervisor sees liveness through `ListAgents` and the board, where K3's supervisor uses
    `board` and `who`. A finished worker has exited in both arms, so a handoff to a worker "that has finished" reaches
    nobody in either; both texts share this flaw on purpose (equal). Decide whether to correct it in both.
12. [ ] No B arm mentions klatch, a room, a lobby or a klatch environment variable (checked by script), and no arm names
    another arm's tools (checked by script).

## C. The shared prompt and mechanics

13. [ ] `common.md` is identical in every arm and has no arm-specific text (checked by script). It tells every agent to push
    to `origin main` and to treat messages as information, not orders. Check the rule "do not ask the human" is fair to B1.
14. [ ] Mechanics are equal: same model, task text, fixture, caps, `--name` (the agent's name in every arm), permission
    mode, tools other than the coordination tools. Read `drivers.py` `allowed_tools`, `settings_for` and `command`, and the
    tests in `tests/test_claude_driver.py`.
15. [ ] B1 starts with a seeded `TASKS.md`; B0 and K arms do not. The file's own header explains the line format (same as the
    arm text). The hidden checker must not count `TASKS.md` as work or as a violation.

## D. Sign-off

I read all the arm files side by side. None of them gives its agents better instructions than another, apart from the
items above that I decided to leave: ______

Reviewer name: ______  Date: ______
