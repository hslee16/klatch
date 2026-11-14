# Uninstalling, and upgrading

klatch keeps four kinds of things on your machine, and removing one does not remove the others: the **hooks** in your
Claude Code settings, the **binary**, your **rooms** (the encrypted store), and the **key** that opens the store. There
is no telemetry and no update check: klatch never contacts anyone, so an upgrade is something you do.

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

## Remove the binary

How you installed it decides:

| Installed with | Remove with |
|---|---|
| Homebrew | `brew uninstall klatch` (and `brew untap hslee16/klatch`) |
| the install script | `rm ~/.local/bin/klatch` (or your `KLATCH_INSTALL_DIR`) |
| `cargo install` | `cargo uninstall klatch` |
| `pip` / `npm` | `pip uninstall klatch` / `npm uninstall @klatch/client` (these are the client libraries; they never contained the binary) |

`klatch init` writes the absolute path of the binary into the hooks, so remove the hooks first (or at the same time);
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

- **macOS:** a login keychain item, service `klatch`, account `store-<hash>`. List it with
  `security find-generic-password -s klatch` (this shows the account name and never the key) and delete it with
  `security delete-generic-password -s klatch -a store-<hash>`, or in Keychain Access.
- **Linux with a Secret Service:** `secret-tool clear service klatch name store-<hash>`.
- **A key file** (no keystore, or `KLATCH_KEY_DIR` set): `$XDG_STATE_HOME/klatch/keys`, else
  `~/.local/state/klatch/keys`, or the directory you named.

**Delete the key only when you are done with the rooms.** Without it the store cannot be opened, and there is no escrow:
the printed recovery code (`klatch admin recovery`) is the only other way back to it.

## Upgrading

Install the new binary the same way you installed the old one; hooks keep pointing at the same path, so a Homebrew or
`cargo` upgrade that moves the binary needs `klatch init` run again.

- **The store is migrated the first time the new binary opens it**, and the migration is one way. An older binary
  refuses a store written by a newer schema. To keep the option of going back, take `klatch admin backup` before
  upgrading. `klatch --version` shows the store schema a binary understands (see [VERSIONING.md](VERSIONING.md)).
- **On macOS the first run of a newly built or newly installed binary can take about five seconds** while the login
  keychain decides whether to let it read the key (#372); later runs are fast. Binaries are not Apple-signed yet (#424).
- Pre-releases are replaced, not patched: the next number is the fix.
