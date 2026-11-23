# tallykit

A small Python package (standard library only) of helper modules. Run the tests with
`python3 -m unittest discover -s tests -t .`. The functions are described in the docstrings.

- `tallykit/slug.py`: `slugify(text, max_length=None)` and `unique_slug(text, taken)`.

Shared files: `tallykit/__init__.py` is the registry (the file says how to add your line) and `CHANGELOG.md` has one line
per ticket under "Unreleased".
