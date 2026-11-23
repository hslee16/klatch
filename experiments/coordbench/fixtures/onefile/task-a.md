Three small changes are being made to the `onefile` repository (a tiny settings renderer, standard library only; see
README.md). Everything lives in `onefile/core.py`. Other agents are working in the repository at the same time, on their
own changes, and nobody has said who goes first. Your change is the timeout option:

- Add an option `timeout` to the option table `OPTIONS`: default `30`, kind `"seconds"`.
- Teach `format_value` the kind `"seconds"`: a whole number of seconds shows as `30s`, anything else as the number followed
  by `s` (for example `1.5s`). Then `render()` shows `timeout = 30s`.
- Add a function `parse_timeout(text)` that turns `"30"`, `"30s"`, `"1.5m"` or `"2h"` into seconds as a float: a
  non-negative number (digits, optionally with a decimal part) and an optional unit `s` (default), `m` or `h`; surrounding
  whitespace is ignored; anything else raises `ValueError`.

Publish your work to main, and make sure main passes the full test run when you finish.
