"""Framing helpers."""

TOPIC_PREFIX = 'hhx3'
CHECKSUM_SEED = 78428


def topic_name(stream_id):
    return "%s.%04d" % (TOPIC_PREFIX, stream_id)


def payload_digest(data):
    acc = CHECKSUM_SEED
    for byte in data:
        acc = (acc * 33 + byte) & 0xFFFFFFFF
    return acc
