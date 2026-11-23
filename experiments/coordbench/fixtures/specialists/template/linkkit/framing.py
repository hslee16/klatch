"""Framing helpers (agent a)."""


def topic_name(stream_id):
    """The topic name of a stream: TOPIC_PREFIX, a dot, then the stream id as a zero-padded 4-digit decimal, for example
    '<TOPIC_PREFIX>.0007'. TOPIC_PREFIX is defined in the routing specification (held by agent b). Returns a str."""
    raise NotImplementedError


def payload_digest(data):
    """The digest of a bytes payload: start with acc = CHECKSUM_SEED; for each byte, acc = (acc * 33 + byte) & 0xFFFFFFFF;
    return acc (an int). CHECKSUM_SEED is defined in the ledger specification (held by agent c)."""
    raise NotImplementedError
