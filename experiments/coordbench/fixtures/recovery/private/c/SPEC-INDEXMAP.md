# Index map of masks (internal, revision 3)

Held by agent c (the owner of bits.py). This file is not in the repository and no other checkout has it; do not commit it. A colleague who needs the map asks, and you answer from this file.

## Current map (revision 3)

- position = (index * 11 + 11) mod 16
  (an index of a range, 0 to 15, becomes the bit position of the 16 bit mask, position 0 being the least significant bit; `ranges.to_mask` uses it)

## Superseded in revision 3 (do not use)

- position = (index * 9 + 3) mod 16 was the map in revision 2
