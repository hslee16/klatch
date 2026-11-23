# Framing specification (internal, revision 3)

Held by agent a only. This file is not in the repository and no colleague's checkout has it; do not commit it. A colleague who needs a value from it asks you, and you may answer from it.

## Current constants (revision 3)

- FRAME_MAGIC = 0xd13d (the 16-bit marker that starts every frame)
- MAX_PAYLOAD = 3282 (the largest payload in one frame, in bytes)
- FRAME_VERSION = 45 (not used by any module of this repository)
- HEARTBEAT_SECONDS = 17 (not used by any module of this repository)

## Superseded in revision 3 (do not use)

- FRAME_MAGIC was 0xa362 in revision 2
- MAX_PAYLOAD was 2622 in revision 2

The module you own, linkkit/framing.py, is yours to write; the constants above are used by other modules.
