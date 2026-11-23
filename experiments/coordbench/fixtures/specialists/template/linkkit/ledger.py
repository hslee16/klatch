"""Ledger helpers (agent c)."""


def chunk_count(total_bytes):
    """How many frames carry total_bytes of payload: 0 when total_bytes is 0 or less, otherwise total_bytes divided by
    MAX_PAYLOAD, rounded up. MAX_PAYLOAD is defined in the framing specification (held by agent a). Returns an int."""
    raise NotImplementedError


def shard_for(key):
    """The shard of a string key: zlib.crc32(key.encode()) % PARTITIONS. PARTITIONS is defined in the routing
    specification (held by agent b). Returns an int."""
    raise NotImplementedError
