# klatch (pre-release)

A substrate for agent rooms: claims, asks, offers, status, a board and a durable log.

**This is an early pre-release.** On macOS (arm64, x86_64) and Linux (x86_64, aarch64) it installs the `klatch` command
and the Python client (`klatch.client`), which speaks the wire protocol (PROTOCOL.md, version 1) to
`klatch serve --stdio`; so `pipx install klatch` and `uvx klatch` give you klatch itself. On any other platform it
installs the Python client only. It has had no outside security review: this is not for sensitive data.

Source and documentation: https://github.com/hslee16/klatch

Not related to Design-in-Product/klatch.
