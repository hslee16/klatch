# onefile

A tiny settings renderer (standard library only). Everything is in `onefile/core.py`: an option table (`OPTIONS`), a value
formatter (`format_value`), a header (`render_header`) and `render()`, which prints the whole block.

Run the tests with `python3 -m unittest discover -s tests -t .`.
