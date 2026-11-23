Three small changes are being made to the `onefile` repository (a tiny settings renderer, standard library only; see
README.md). Everything lives in `onefile/core.py`. Other agents are working in the repository at the same time, on their
own changes, and nobody has said who goes first. Your change is the retries option:

- Add an option `retries` to the option table `OPTIONS`: default `3`, kind `"count"`.
- Teach `format_value` the kind `"count"`: the number followed by `x`, so `render()` shows `retries = 3x`; a negative number
  raises `ValueError`.
- Add a function `backoff_delays(retries, base=1.0, cap=30.0)` that returns the wait before each retry in seconds, as a list
  of floats: the first is `base`, each next one is twice the one before, and none is more than `cap`. `retries` is how many
  retries there are (0 gives an empty list); a negative number raises `ValueError`.

Publish your work to main, and make sure main passes the full test run when you finish.
