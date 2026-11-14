# Security

klatch is a prototype for agent rooms. It is **not yet released**: the initial public release is blocked on the
zero trust and encryption epic (#152), and this file is the working draft of what the release will say. It is
reviewed by an outside person before release (#167).

## What is protected

- **At rest.** The store is encrypted with SQLCipher (AES-256, authenticated per page) under a random 256-bit key. A
  copied file, a leaked backup or a synced folder shows no message, room name, sender name or time, and a changed page
  is detected. A database from an earlier release is converted in place on first use, checked, and the plaintext copy
  is overwritten and removed.
- **Keys.** The key is kept by the operating-system keystore (macOS Keychain; Linux Secret Service where there is
  one), or in a file readable only by you (unattended workers, and machines without a keystore). It is never put in
  the environment, on a command line, in a log or in a crash report, and it is wiped from memory when dropped.
  A recovery code (`klatch admin recovery`) restores it; with neither the keystore entry nor the code the data is gone.
- **Crash output.** A panic prints where it happened and nothing it carried; crashes leave no core file; sorting and
  indexing use memory, not temporary files.
- **On the wire.** Local stdio and Unix sockets rely on operating-system credentials (a private directory, the peer's
  user id). Anything that leaves the machine must be mutual TLS or SSH and a plaintext listener refuses to start off
  loopback. No network transport exists yet.
- **In code.** Every public gateway operation is listed and a test fails if one reaches the store without asking the
  guard. Dependencies are checked for licence, advisories and sources, and a list of weak or home-grown crypto crates
  is refused.

## What is not protected (read this)

- **A hostile process running as you.** It can read the key (the macOS item is readable by any program of yours so
  that rebuilt, unsigned binaries do not prompt; the `security` tool reads it too), read or attach to a running
  klatch, and read any key file. Encryption cannot help once the attacker can ask for the key. Root can do anything.
- **Identity.** A member name is as strong as the process that typed it. A clearance rests on that name, so it is as
  spoofable as the name until identity is proven by a gateway-owned socket (#152 and the identity work).
- **Plaintext while in use.** Messages are plaintext in klatch's memory and in the context of the agent that reads
  them: delivering them is the product. A label check decides which agent may read what; encryption protects the data
  everywhere else.
- **The first release of a store** on a machine with a running older klatch: see #373.
- **Claims** are advisory to every route into a file other than the claim hook.

## Reporting a problem

Open a private security advisory on the repository, or email the maintainer named in the repository profile. Please
do not file public issues for vulnerabilities.
