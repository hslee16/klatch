# Ledger specification (internal, revision 3)

Held by agent c only. This file is not in the repository and no colleague's checkout has it; do not commit it. A colleague who needs a value from it asks you, and you may answer from it.

## Current constants (revision 3)

- CHECKSUM_SEED = 78428 (the starting value of the payload digest)
- LEDGER_MODULUS = 56963 (the modulus of every ledger slot number)
- LEDGER_BLOCK_SIZE = 3015 (not used by any module of this repository)
- AUDIT_INTERVAL_MINUTES = 25 (not used by any module of this repository)

## Superseded in revision 3 (do not use)

- CHECKSUM_SEED was 106810 in revision 2
- LEDGER_MODULUS was 27967 in revision 2

The module you own, linkkit/ledger.py, is yours to write; the constants above are used by other modules.
