# Research note fragments

One file per new RESEARCH-NOTES section, so two pull requests never fight over the next free section number in
RESEARCH-NOTES.md (#859). Same idea as `design/items/` (#807) and `changelog.d/`.

- Name: `<ticket>-<slug>.md`, for example `859-research-fragments.md`. The ticket number is the GitHub issue the note is
  about (not a sequence number); the slug is lower-case letters, digits and hyphens.
- Content: line 1 is the section title, the text that follows `## N. ` (for example
  `Fragment files for research notes (2026-11-20, ticket #859)`), with no heading marker. The rest is the body in markdown.
  Use `### Title` for subsections; they are numbered `N.1`, `N.2`, ... at assembly. Do not use `## ` headings in the body.
  No em dashes.
- Do not edit RESEARCH-NOTES.md to add a section and do not pick a section number in a pull request. Sections 1 to 53 and
  their subsections (52.8a to 52.8d and the like) keep their numbers; code, tests and other documents cite them.
- Refer to a note that is not yet assembled as "the research note for #NNN". To add to an existing section (an update to
  section 52, say), edit that section in RESEARCH-NOTES.md as before, or write a new fragment that cites it.
- At release, `release/research-fragments.sh assemble` appends the fragments to RESEARCH-NOTES.md with the next section
  numbers in ticket order and deletes the files (RELEASING.md); it prints which number each ticket got, so references to
  "the research note for #NNN" can be changed to the number. `collect` shows the result without changing anything.
- `release/research-fragments.sh check` (run by the gate) rejects a badly named, empty or bodiless fragment, a `## ` heading
  in the body and an em dash.
