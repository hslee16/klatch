<!-- PROPOSED (#1427), T-shared variant of B0: no coordination tools, one shared working directory. Same outline and level of
detail as every other shared arm. No placeholders beyond the common ones. -->
Arm B0. The only thing you share with the other agents is the directory and the files in it. You have no messaging, no task
board and no other tool that connects you to them. Use it the way a careful team that cannot talk would.

Telling others what you are doing: you can only say it through the directory. A commit message, or a file that you add and
commit, is visible to everyone who looks. Nobody will look unless they choose to, so run `git log` and `git status` yourself
before you start work and again before you commit.

Not stepping on each other's work: nothing stops two agents saving the same file at the same moment, and a save replaces
the whole file. Read a file again right before you change it, change only the lines you need with a small targeted edit, and
never write the whole file back from an earlier copy. After you save, read the file once more to see that your change and the
others' changes are all there. Check `git status` and `git diff PATH` to see what a colleague changed.

Asking and answering questions: you cannot reach anyone. If you need a fact that you do not have, look in the files and the
history first. If it is not there, make the most reasonable choice, write the choice in the commit message, and go on.

Handing work over: if you must stop before a piece is finished, commit what you have with a message that says what is done,
what is not and the exact next step, so that whoever reads the log can continue from it.

Waiting: if you need another agent's result, run `git log` and read the files; to wait, run `sleep 20` and look again. Do
not wait more than 5 minutes in total for any one result; if it has not come by then, do the part that you can do without
it and go on.

Finishing: when your part is saved and the tests pass, make sure the last commit message says so, and stop.
