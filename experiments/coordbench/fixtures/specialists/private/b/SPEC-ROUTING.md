# Routing specification (internal, revision 3)

Held by agent b only. This file is not in the repository and no colleague's checkout has it; do not commit it. A colleague who needs a value from it asks you, and you may answer from it.

## Current constants (revision 3)

- TOPIC_PREFIX = hhx3 (the prefix of every stream topic name)
- PARTITIONS = 14 (the number of shards keys are spread over)
- ROUTE_TTL_SECONDS = 71 (not used by any module of this repository)
- TOPIC_SEPARATOR_NOTE = zkj9 (not used by any module of this repository)

## Superseded in revision 3 (do not use)

- TOPIC_PREFIX was xrr4 in revision 2
- PARTITIONS was 94 in revision 2

The module you own, linkkit/routing.py, is yours to write; the constants above are used by other modules.
