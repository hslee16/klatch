# recoverkit

A small Python package (standard library only) of four independent helper modules. Run the tests with
`python3 -m unittest discover -s tests -t .`. Each module's functions are described in its docstrings; a line each here:

- `recoverkit/ranges.py`: `compress(numbers)`, `expand(ranges)` and `to_mask(ranges)`, integer ranges and their bit mask.
- `recoverkit/wrap.py`: `wrap(text, width)` and `unwrap(lines)`, greedy word wrapping.
- `recoverkit/pairs.py`: `pairs(seq)`, `chunk(seq, n)` and `window(seq, n)`, walking a sequence in groups.
- `recoverkit/bits.py`: `set_bit`, `clear_bit`, `test_bit`, `popcount` and `bit_indexes` on 16 bit masks.

`docs/FORMATS.md` records the conventions the modules share. There is no shared registry file: the modules do not import
each other, so each can be published on its own.
