# Uninstalling, and upgrading

klatch keeps five kinds of things on your machine, and removing one does not remove the others: the **hooks** in your
Claude Code settings, the **binaries** (`klatch` and `klatch-server`), anything `klatch-server` is running or has left
(a **daemon** and its **socket**, a webhook secret file you made), your **rooms** (the encrypted store), and the **key**
that opens the store. There is no telemetry and no update check, so an upgrade is something you do. That is checked by
inspection, not by watching the network: `proto/klatch/tests/no_telemetry.rs` fails if `klatch` gains a networking
dependency, opens a socket, runs a program it is not known to run, or if a document tells you to pipe a download into a
shell, and `proto/klatch-server/tests/no_network.rs` pins what `klatch-server` may open (loopback listeners and the
daemon's Unix socket only, no outbound connection, no program started).

## Remove the hooks

```
klatch init --uninstall --scope project     # .claude/settings.local.json in this directory
klatch init --uninstall --scope user        # ~/.claude/settings.json
klatch init --uninstall --scope both
```

It shows a diff and asks before writing each file (`--yes` skips the question). It removes only the hook entries klatch
wrote (the session, prompt, tool, session end, `--enforce` and `--wake` hooks) and leaves every other hook and every
other setting as it was; an event that held only klatch's entries disappears, and a hook that merely mentions klatch is
not touched. Running it again says there is nothing to remove, and `klatch init` puts the hooks back.

Sessions that are already open keep the hooks they started with until they end.

## Stop klatch-server, its daemon and its socket

Do this before removing the binaries. Stop every `klatch-server` you started (`daemon`, `http`, `channel`): a Ctrl-C in its
terminal, or `kill -TERM <pid>` (SIGTERM and SIGINT stop it, and the daemon removes its socket on the way out). Find the
processes with `ps aux | grep klatch-server`. If one will not stop, `kill -9 <pid>`.

A daemon's socket is the path you gave to `klatch-server daemon --socket PATH`; there is no default location. After the
daemon has stopped, remove a leftover file at that path (`rm PATH`) and, if you made it only for this, its directory. A
webhook secret file you made for `klatch-server channel --webhook-secret-file` is yours to delete. If your `.mcp.json`
or another tool's configuration names `klatch-server`, remove that entry; `klatch init --uninstall` does not touch it.
Worker credentials the daemon or `klatch-server http` accepted stop working when the store is gone, or earlier with
`klatch worker revoke`.

## Remove the binaries

How you installed them decides. Both binaries, `klatch` and `klatch-server`, go together:

| Installed with | Remove with |
|---|---|
| Homebrew | `brew uninstall klatch` removes both (and `brew untap hslee16/klatch`) |
| the install script | `rm ~/.local/bin/klatch ~/.local/bin/klatch-server` (or the same names in your `KLATCH_INSTALL_DIR`) |
| `cargo install` | `cargo uninstall klatch`; a `klatch-server` you built yourself is wherever you copied it (`cargo uninstall klatch-server` if you installed it from a checkout with `cargo install --path proto/klatch-server`) |
| `pipx` / `uvx` / `pip` | `pipx uninstall klatch` (the wheel carries both binaries) or `pip uninstall klatch` |
| `npm` | `npm uninstall @klatch/client` (the TypeScript client library; it never contained a binary) |

`klatch init` writes the absolute path of `klatch` into the hooks, so remove the hooks first (or at the same time);
a hook that points at a binary that is gone prints one visible line and does nothing else.

## Remove your rooms

The store is `~/.klatch/rooms.db` (and its `-wal` and `-shm` files, and a `wake-<name>.lock` per watcher), or the same
names under `$KLATCH_HOME` if you set it. **This is every message and claim klatch has recorded.** It is encrypted, but
nothing else keeps a copy. To keep a copy first: `klatch admin backup <file>` or `klatch admin export` (both need a real
terminal).

```
rm -r ~/.klatch
```

## Remove the key

The key is not in the store directory. It is one of:

- **macOS:** a file `<hash>.key` in the klatch key directory (mode 600; `store-<hash>.key`), and, on an install that
  predates 2026-11-14, an older login Keychain item, service `klatch`, account `store-<hash>`, which klatch copies
  into the file the first time it can read it and never writes again. List the Keychain item with
  `security find-generic-password -s klatch` (this shows the account name and never the key) and delete it with
  `security delete-generic-password -s klatch -a store-<hash>`, or in Keychain Access. A new install has no Keychain item.
- **Linux with a Secret Service:** `secret-tool clear service klatch name store-<hash>`.
- **A key file** (no keystore, or `KLATCH_KEY_DIR` set): `$XDG_STATE_HOME/klatch/keys`, else
  `~/.local/state/klatch/keys`, or the directory you named.

**Delete the key only when you are done with the rooms.** Without it the store cannot be opened, and there is no escrow:
the printed recovery code (`klatch admin recovery`) is the only other way back to it.

## What stays

Removing the hooks and the binaries keeps your rooms and the key, on purpose. Backups you made with `klatch admin backup`,
exports and archives are separate files you placed; they keep the key's text from the day they were made (a backup taken
before `klatch admin rotate-label-key` keeps the old label key's text, SECURITY.md). Delete them yourself if that matters.

## Upgrading

Install the new binary the same way you installed the old one; hooks keep pointing at the same path, so a Homebrew or
`cargo` upgrade that moves the binary needs `klatch init` run again.

- **The store is migrated the first time the new binary opens it**, and the migration is one way. An older binary
  refuses a store written by a newer schema. To keep the option of going back, take `klatch admin backup` before
  upgrading. `klatch --version` shows the store schema a binary understands (see [VERSIONING.md](VERSIONING.md)).
- **Upgrade both binaries together** (`klatch` and `klatch-server` share the store and its schema), and restart any
  daemon or `http` server you run so it uses the new one.
- **On macOS, an install that predates the key file asks once.** Its key is in the login keychain, and the Keychain
  asks for your login password the first time a different build reads it (#480). Choose Always Allow once: klatch
  copies the key into a 600 file and never asks again, however often you rebuild or upgrade. A klatch read of the
  keychain gives up after 5 seconds with a message (a hook prints one line and exits) and never waits for the dialog.
  New installs never use the Keychain. Binaries are not Apple-signed or notarized (#424).
- Pre-releases are replaced, not patched: the next number is the fix.
