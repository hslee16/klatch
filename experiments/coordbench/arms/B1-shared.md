<!-- PROPOSED (#1427), T-shared variant of B1: native cross-session messaging (the message prefixes klatch uses) between
named `claude -p` sessions plus a task board file, TASKS.md, which lives in the shared working directory. Same outline and
level of detail as every other shared arm. -->
Arm B1. You can send a message to a named colleague with the `SendMessage` tool (the colleague's name is the `to` value;
`ListAgents` shows who is running now), and the directory holds a task board, `TASKS.md`, that every agent edits. A message
reaches a colleague between their tool calls, and only while they are still running. Use both the way a careful team would.

Telling others what you are doing: when you start a piece of work, add a line at the end of `TASKS.md`,
`- NAME | working | FILES | TITLE`. When you finish, change `working` to `done`. Send a short message to a colleague only
when it concerns them. Edit the board with a small targeted edit, never by writing the whole file back.

Not stepping on each other's work: before you edit a file, read `TASKS.md` again. If another agent's line lists that file as
`working`, message that agent and agree who goes first before you touch it. Put your own line on the board before you edit.
A save replaces the whole file, so read a file again right before you change it and change only the lines you need.

Asking and answering questions: send `seeking: THE EXACT QUESTION` to the colleague most likely to know. A colleague who can
answer replies `answer: ...` with the fact and where it comes from. Before you ask, read `TASKS.md` and `git log`.

Handing work over: if you must stop before a piece is finished, set its `TASKS.md` line to `handoff`, write in the TITLE what
is done, what is not and the exact next step, and send `handoff: TITLE` to the colleague who takes it over.

Waiting: if you need another agent's result, message them once and do the parts you can do first; to wait, run `sleep 20`,
then read `TASKS.md` again (a reply arrives by itself while you work). Do not wait more than a few minutes.

Finishing: set your lines to `done`, make sure your files are committed, and stop.
