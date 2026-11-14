# Does the wake watcher wake an idle interactive session? (ticket #350)

Headless runs are recorded in `RESEARCH-NOTES.md` section 39. This one needs a person at a real terminal, because the
interactive terminal could not be driven from a script. It uses a private store, so nothing touches your real one.

## Setup (once)

```
export KLATCH_HOME=$(mktemp -d)
BIN=$HOME/.local/share/klatch/klatch
mkdir -p /tmp/wake-try && cd /tmp/wake-try
$BIN init --scope project --yes --wake
claude        # the same shell, so KLATCH_HOME is set
```

In the session, say: `Reply with the word ready.` and wait for the reply. The session is now idle.

## The runs (from a second terminal with the same `KLATCH_HOME`)

1. **Idle wake.** Find the session's member name (`$BIN who lobby --all`, the `klatch-...` one), then
   `KLATCH_NAME=bob $BIN join lobby; KLATCH_NAME=bob $BIN post lobby "hello from bob"`.
   Write down: does a new turn start by itself? How many seconds after the post? Did the session run `klatch read`?
2. **Busy session.** Ask the session for something that takes a minute (`Run sleep 45 in the shell, then say done`),
   post again during it. Write down: is the message shown after the tool returns, and does a second turn follow?
3. **Waited past 540 seconds.** Leave the session idle for eleven minutes, then post. Write down: does it still wake
   (the `Stop` hook should have started a new watcher at the end of the last turn, but the watcher's own timeout is
   540 seconds, so an idle session beyond that is not watched; this is the expected failure to confirm)?
4. **Two sessions.** Start a second `claude` in another directory with the same setup. Post once. Write down: do both wake?

Paste the notes into ticket #350. Clean up with `rm -rf $KLATCH_HOME /tmp/wake-try`.
