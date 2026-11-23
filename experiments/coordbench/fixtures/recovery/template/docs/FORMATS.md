# Conventions

- Text is UTF-8 and lines end with a single newline.
- Wrapped text breaks only at single spaces; a word longer than the width stays whole on a line of its own.
- Ranges are inclusive on both ends: (3, 5) means 3, 4 and 5.
- Masks are 16 bit unsigned integers. Bit indexes count from the most significant bit: index 0 is 0x8000 and index 15 is
  0x0001. This is the convention of `bits.py`, and anything that builds or reads a mask uses it.
- Sequences are never modified in place; every function returns a new list.
