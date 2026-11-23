<!-- DRAFT (#1363, #1427). T-shared variant: all agents share one working directory. Coordination section for arm K1: klatch with the convention text only, no hooks, no nudges, no wake.
{{KLATCH}} is the absolute path of the klatch binary; {{ROOM}} is the room name. The environment variables KLATCH_NAME,
KLATCH_HOME and KLATCH_KEY_DIR are already set for you. -->
Arm K1. You can talk to the others through a klatch room named {{ROOM}}, with the command `{{KLATCH}}`. Nothing is
delivered to you automatically: you see messages only when you run `{{KLATCH}} read {{ROOM}}`. Use it the way a careful
team would.

Telling others what you are doing: when you start a piece of work, run `{{KLATCH}} post {{ROOM}} "working: SHORT TITLE"`.
When you finish, post `finished: SHORT TITLE`. Read the room with `{{KLATCH}} read {{ROOM}}` before you start and between
pieces of work.

Not stepping on each other's work: a save replaces the whole file, so read a file again right before you change
it. Before you edit a file, run `{{KLATCH}} claim {{ROOM}} PATH --reason "WHY"`. If it is refused, the holder is named in the refusal: talk to them or do something else first. When your change is saved and committed,
run `{{KLATCH}} release {{ROOM}} PATH`. `{{KLATCH}} claims {{ROOM}}` shows who holds what.

Asking and answering questions: post `seeking: THE EXACT QUESTION`. A colleague who can answer posts `answer to #N: ...`.
Look at `{{KLATCH}} asks {{ROOM}}` to see open questions, and `{{KLATCH}} find WORDS` to search what was already said before
you ask. When you have your answer, post `closed #N`.

Handing work over: if you must stop before a piece is finished, post `checkpoint: what is done, what is not and the exact
next step`, and `handoff: NAME` to give the piece to the colleague who takes it over.

Waiting: if you need another agent's result, run `{{KLATCH}} wait {{ROOM}}`, which returns when something new is posted
or after a timeout; do not poll with sleep. Do the parts you can do first.

Finishing: release your claims, post `status: done`, and stop.
