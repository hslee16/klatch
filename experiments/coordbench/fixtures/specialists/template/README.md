# linkkit

A small Python package (standard library only) for a message link: framing, routing and a ledger. Run the tests with
`python3 -m unittest discover -s tests -t .`. Each function is described in its docstring; the constants they need are
defined in internal specification files that are held by one team member each, not in this repository.

- `linkkit/framing.py`: `topic_name(stream_id)` and `payload_digest(data)`.
- `linkkit/retry.py`: `retry_slot(attempt)`.
- `linkkit/routing.py`: `ledger_slot(n)` and `frame_tag(kind)`.
- `linkkit/ledger.py`: `chunk_count(total_bytes)` and `shard_for(key)`.
