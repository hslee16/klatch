# tallykit

A small Python package (standard library only) of eight independent helper modules. Run the tests with
`python3 -m unittest discover -s tests -t .`. Each module's functions are described in its docstrings; a line each here:

- `tallykit/units.py`: `convert_length(value, from_unit, to_unit)` and `convert_temperature(value, from_unit, to_unit)`.
- `tallykit/tokenizer.py`: `tokenize(text)` and `token_kinds(text)`, a tiny lexer for numbers, words and symbols.
- `tallykit/dates.py`: `is_leap_year(year)`, `days_in_month(year, month)` and `days_between(first, second)` on ISO dates.
- `tallykit/checksum.py`: `adler32(data)` and `luhn_valid(number)`.
- `tallykit/slug.py`: `slugify(text, max_length=None)` and `unique_slug(text, taken)`.
- `tallykit/roman.py`: `to_roman(n)` and `from_roman(text)` for canonical Roman numerals, 1 to 3999.
- `tallykit/rle.py`: `rle_encode(text)` and `rle_decode(text)`, count-then-character run-length text.
- `tallykit/matrix.py`: `transpose(m)`, `multiply(a, b)` and `identity(n)` on lists of lists.

Shared files: `tallykit/__init__.py` is the registry (every function is exported from the package; the file says how to
add yours) and `CHANGELOG.md` has one line per ticket under "Unreleased". Everyone who finishes a ticket edits both.
