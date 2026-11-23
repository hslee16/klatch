# Conventions

- Text is UTF-8 and lines end with a single newline.
- Wrapped text breaks only at single spaces; a word longer than the width stays whole on a line of its own.
- Ranges are inclusive on both ends: (3, 5) means 3, 4 and 5.
- Masks are 16 bit unsigned integers. `bits.py` works on bit positions: position 0 is the least significant bit (0x0001)
  and position 15 is 0x8000.
- A range index (0 to 15) becomes a bit position through the index map. Legacy map, no longer used: position = 15 - index
  (most significant bit first). The current map is not recorded in the repository; its owner is the owner of `bits.py`.
- Sequences are never modified in place; every function returns a new list.
