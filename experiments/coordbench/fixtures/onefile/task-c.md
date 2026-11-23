Three small changes are being made to the `onefile` repository (a tiny settings renderer, standard library only; see
README.md). Everything lives in `onefile/core.py`. Other agents are working in the repository at the same time, on their
own changes, and nobody has said who goes first. Your change is the header and the label option:

- Change `render_header(count)` so the first line of the output reads `== settings (N options) ==` where N is `count`, the
  number of options in the table (`== settings (1 option) ==` when it is exactly one). Today it is `[settings]`.
- Add an option `label` to the option table `OPTIONS`: default `"default"`, kind `"text"`. `render()` then shows
  `label = default`.

Publish your work to main, and make sure main passes the full test run when you finish.
