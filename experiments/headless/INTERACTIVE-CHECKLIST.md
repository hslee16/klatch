# Interactive session checklist

Interactive Claude Code sessions cannot be driven headless, so this is a short procedure with the checks scripted:
the keystrokes are yours, the verification is commands. Run it after a change to hooks, identity, `init` or the
lobby, and after a Claude Code update. Record the Claude Code version (`claude --version`).

Use a scratch project and store so nothing real is touched:

    D=$(mktemp -d); mkdir -p $D/proj $D/home
    export KLATCH_HOME=$D/home
    K=~/.local/share/klatch/klatch            # or the binary under test
    (cd $D/proj && $K init --scope project --yes)

Then, in a second terminal with the same `KLATCH_HOME` exported, start Claude Code interactively in `$D/proj`.

| # | You do | Expected | Verify with |
|---|---|---|---|
| 1 | Start `claude` in `$D/proj` | A line under the banner: `SessionStart:startup says: klatch: joined lobby as <name> (N members, M open asks)` | look at the screen |
| 2 | Nothing yet | The session is a member of the lobby | `$K who lobby --all` lists `<name>` |
| 3 | Send any prompt that uses a tool, for example "list the files here" | `UserPromptSubmit` and `PostToolUse` recorded | `$K hooks --of <name>` |
| 4 | From the second terminal: `KLATCH_NAME=someone $K post lobby "hello from someone"`, then in the session ask it to run `ls` again | The message reaches the model at that tool call and it quotes it as data from `someone` | the session's reply |
| 5 | In the session: `/exit` (or Ctrl-D) | `SessionEnd` recorded; the session is done | `$K who lobby --all` shows `status: done session ended`; `$K hooks --of <name>` lists `SessionEnd` |
| 6 | Start a second `claude`, and from inside it ask it to run a child `claude -p "say hi"` through the Bash tool | Two members; when the child ends the parent is not marked done | `$K who lobby --all` while the parent is still open |
| 7 | Restart (resume) a session in the same project | It is told it is `back in lobby` under its first name, not a new member | the line under the banner, and `who` shows no duplicate |

A step that fails is a finding: file a ticket with the Claude Code version, what you saw, and the output of the
verify command. The scripted headless equivalent is `experiments/headless/run.sh`; the bench's fake harness
covers the identity logic without a model (DESIGN section 26.1).
