# Changelog fragments

One file per change, so two pull requests never edit the same line of CHANGELOG.md (#502).

- Name: `<ticket or topic>.<category>.md`, for example `495-identity-hints.added.md`. Categories: `added`, `changed`,
  `deprecated`, `removed`, `fixed`, `security`.
- Content: one or more lines, each starting with `- `. No em dashes.
- Do not edit the Unreleased section of CHANGELOG.md in a pull request.
- At release, `release/changelog-fragments.sh collect` prints the entries grouped by category; paste them under the
  new version and delete the fragment files (RELEASING.md).
- `release/changelog-fragments.sh check` (run by the gate) rejects a badly named or badly formed fragment.
