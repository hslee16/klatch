"""Ledger helpers."""
import zlib

MAX_PAYLOAD = 3282
PARTITIONS = 14


def chunk_count(total_bytes):
    if total_bytes <= 0:
        return 0
    return -(-total_bytes // MAX_PAYLOAD)


def shard_for(key):
    return zlib.crc32(key.encode()) % PARTITIONS
