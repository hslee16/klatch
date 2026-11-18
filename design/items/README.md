# Design item fragments

One file per new DESIGN item, so two pull requests never fight over the next free item number in DESIGN.md (#807).

- Name: `<ticket>-<slug>.md`, for example `807-design-fragments.md`. The ticket number is the GitHub issue the item is
  about (not a sequence number); the slug is lower-case letters, digits and hyphens.
- Content: line 1 is the item title, the text that goes inside the bold (for example
  `Design items are fragments (#807; no schema change; 2026-11-18).`), with no `**` and no heading marker. The rest is the
  body, as the items in DESIGN.md have it: the first paragraph follows the title on the same line; later paragraphs,
  separated by a blank line, are indented under it. No em dashes.
- Do not edit DESIGN.md or pick an item number in a pull request.
- Refer to an item that is not yet assembled as "the design item for #NNN". Items 1 to 104 and anything already in
  DESIGN.md keep their numbers, and code comments may cite them.
- At release, `release/design-fragments.sh assemble` appends the fragments to DESIGN.md with the next item numbers in
  ticket order and deletes the files (RELEASING.md); it prints which number each ticket got, so references to
  "the design item for #NNN" can be changed to the number. `collect` shows the result without changing anything.
- `release/design-fragments.sh check` (run by the gate) rejects a badly named, empty or bodiless fragment.
- A pull request that still appends a numbered item to DESIGN.md keeps working; the sweep resolves that conflict.
