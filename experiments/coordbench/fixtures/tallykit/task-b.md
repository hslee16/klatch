Four tickets are open on the tallykit repository (a small Python package, standard library only; see README.md). You own
the ticket named B: implement `tallykit/dates.py` and `tallykit/checksum.py` so that their tests pass (the visible tests are
`tests/test_dates.py` and `tests/test_checksum.py`; the functions are described in the docstrings of the stubs, which raise
NotImplementedError today). Your ticket also has two shared-file changes, which every ticket makes:

- Registry: add one line at the end of `tallykit/__init__.py` for each of your two modules, as that file describes.
- Changelog: add one line at the end of `CHANGELOG.md` under "Unreleased": `- ticket B: dates, checksum`.

Publish your work to main, and make sure main passes the full test run when you finish. Work only on your own ticket;
the other tickets belong to the other agents.
