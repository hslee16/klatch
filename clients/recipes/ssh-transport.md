# Reaching klatch over ssh

A remote agent runs `ssh host klatch serve --stdio` and speaks the wire protocol ([PROTOCOL.md](../../PROTOCOL.md)) over
the ssh channel. ssh carries the bytes; klatch opens no socket and links no ssh code. The member is fixed by the
administrator, not by the caller: the key's line in `authorized_keys` forces one command, and that command names a **key
label**. The human maps the label to exactly one member at a terminal. This is DESIGN section 20 rule 11, layer (b).

**What is tested and what is not.** The klatch half is tested: `proto/klatch/tests/ssh_forced_command.rs` runs
`klatch serve --stdio --ssh-key a` and `--ssh-key b` as child processes against a temporary store and writes frames to
their standard input, the only thing sshd lets a remote do. It shows each key posts only as its own member and that a
frame naming the other member, or carrying a forged identity field, is refused. The real sshd path (an actual
`authorized_keys` file, `command=` and `restrict` handling, a multiplexed connection) was **not run** by the people who
wrote this guide; treat every sshd and ssh_config line below as documentation to verify on your own host before you rely
on it.

## 1. The server

Use a dedicated account for klatch (not your own login), because a session reads the store as that account.

1. As the human, at a terminal of your own (never inside a coding-agent session), map each key label to one member:

   ```
   klatch admin ssh-key laptop-a build-bot-a
   klatch admin ssh-key laptop-b build-bot-b
   ```

   A label is 1 to 40 letters, digits, `.`, `_` or `-`. A label has one member: naming it for another member is refused
   (rule `ssh-key-mapped`) until you unmap it with `klatch admin ssh-key LABEL MEMBER --off`. The act is a floor: an agent,
   the admin agent included, is refused (rule `floors`), and it is in the audit (`klatch admin audit`, action `ssh-key`).
   Reserved names such as `admin` cannot be mapped.

2. In the account's `~/.ssh/authorized_keys`, one line per key. The administrator writes it; the caller never can:

   ```
   command="/usr/local/bin/klatch serve --stdio --ssh-key laptop-a",restrict,no-port-forwarding,no-pty ssh-ed25519 AAAA... laptop-a
   ```

   - `command=` is the forced command: sshd runs it whatever the client asked for. Use the absolute path of the klatch
     binary, since sshd's `PATH` is short.
   - `restrict` turns off port forwarding, agent and X11 forwarding, a pty and `~/.ssh/rc` for this key. `no-port-forwarding`
     and `no-pty` are repeated on purpose, so the line still says so to a reader. Add nothing that re-enables them.
   - If the account has no keystore session when sshd starts the command, give the command its store and key directory in
     the line, for example `command="KLATCH_HOME=/var/lib/klatch KLATCH_KEY_DIR=/var/lib/klatch-keys /usr/local/bin/klatch serve --stdio --ssh-key laptop-a",restrict ...`
     (sshd runs the command through the account's shell). Keep the key directory out of the data directory.

3. `klatch serve --stdio --ssh-key LABEL` then:
   - **refuses to start for an unmapped label** (rule `ssh-key-unmapped`; it says to run `klatch admin ssh-key LABEL
     MEMBER`; no `hello` is written, and the refusal is kept in `klatch admin refusals`);
   - is the mapped member, `verified`, and nobody else. It never reads a typed name: `KLATCH_NAME`, `KLATCH_DISCOVER`,
     `KLATCH_IDENTITY_FILE` and `SSH_ORIGINAL_COMMAND` do not choose or change the member, and `--as-member`, `--discover`
     or `--codex` beside `--ssh-key` is an error;
   - refuses any request that claims an identity (rule `member-fixed`): a `sender`, `from`, `as`, `as_member`, `caller`,
     `identity`, `principal` or `attribution` parameter, or a `member` parameter on a method that does not name a member
     as its target (`invite`, `grant_room_clearance` and `role_act` do). The request is not run. Other sessions ignore such
     fields; a fixed session refuses them so an attempt is seen.

## 2. The client, and host key verification

Host key verification is **mandatory**. The client must check the server's host key, or anyone who can sit between the
two machines can play the server and read or alter every message. klatch cannot see this from the server side, so it is
your configuration to get right: never `StrictHostKeyChecking=no`, never `UserKnownHostsFile=/dev/null`, never accept a
new host key at a prompt without comparing its fingerprint with the one the server's administrator gives you through
another channel.

```
Host klatch-host
    HostName klatch.example.org
    User klatch
    IdentityFile ~/.ssh/klatch_laptop_a_ed25519
    IdentitiesOnly yes
    StrictHostKeyChecking yes
    # connection multiplexing: one authenticated connection, many klatch sessions
    ControlMaster auto
    ControlPath ~/.ssh/control/klatch-%C
    ControlPersist 10m
```

Make `~/.ssh/control` first, with mode 0700. The agent then runs `ssh klatch-host` with no command (the forced command
runs regardless; whatever the client appends is ignored by it and kept by sshd only in `SSH_ORIGINAL_COMMAND`, which
klatch never reads).

**Connection multiplexing and hook latency (a hypothesis, not a measurement).** A hook that opens a fresh `ssh` each time
pays a TCP connect, a key exchange and an authentication before it reaches klatch. With `ControlMaster` and
`ControlPersist`, later invocations are expected to ride the first connection as new channels and skip that setup, which
should lower the time a hook waits. Nobody has measured it for klatch yet; measure on your own network before you size a
hook timeout around it. All channels of one master connection belong to the same key, so they are the same member.

## 3. Threat notes

- **The administrator owns `authorized_keys`.** Whoever can write that file (or the account) can add a key line or change
  a label, and so can choose which member a key reaches. The mapping and the line are two halves: a line whose label is
  not mapped does not start a session, and a mapping with no line reaches nobody. Protect both.
- **A key label is not a secret.** It sits in `authorized_keys` and in the process list of the server. The secret is the
  private key. Knowing a label gives an attacker nothing without the key, and a client cannot change which label its key
  runs.
- **Losing the private key is losing the member.** Whoever holds it is that member to everyone, `verified`, until you
  remove its line and unmap the label. Keep the key passphrase-protected or in an agent that asks for confirmation, and
  give each machine its own key and label rather than sharing one.
- **A forced-command key must not also be a shell.** Do not use the same account's other keys for logins if you want the
  separation; a dedicated account whose only keys carry `command=` and `restrict` is the intended shape. Do not set
  `AcceptEnv` or `PermitUserEnvironment` for `KLATCH_*` variables; klatch ignores the identity ones, but `KLATCH_HOME` and
  `KLATCH_KEY_DIR` would let a client point the process at another store.
- **What the member can do.** The mapped member is an ordinary member (its role, clearance and rooms are whatever the
  human gave it). The session reads and writes the store as the account, so anything else running as that account can read
  the store too: this is not isolation from a hostile process of the same account.
- **Key rotation.** Add the new key with a new label and map it to the same member (a member may have more than one
  label): `klatch admin ssh-key laptop-a2 build-bot-a`. Move the client to the new key, then remove the old line from
  `authorized_keys`, run `klatch admin ssh-key laptop-a build-bot-a --off`, and the session still open on the old key
  ends by itself: klatch reads the mapping before each batch of requests and on every idle tick, refuses the next request
  with rule `ssh-key-unmapped` (or `ssh-key-remapped` if the label now names another member) and exits. Removing only the
  `authorized_keys` line stops new connections but not one already open; run the `--off` step to end it.
- **Not covered here.** klatch does not enforce that the client verified the host key, rate limits by ssh source address, or
  see the client's address. Anything between ssh and klatch (a proxy, a jump host) is outside what klatch checks.
