"""Ask counts from the agents' streams, and the cross-check against the store (#1433).

The harness reads asks twice: from what the agents ran (the streams: a `klatch ask` that printed `asked #N`, a `klatch
answer` that printed `answered #N`, a lost race that printed `not answered:` is not an answer) and from the run store (the
counts-only export, storeexport.py). The two should agree; a difference is recorded with both sides and flagged, never
resolved silently, because it means the stream parser or the store report is wrong about something. Standard library only.
"""
import re

import chain

ASKED = re.compile(r"\basked #\d+")
ANSWERED = re.compile(r"(?<!not )\banswered #\d+")


def from_streams(streams, klatch_bin):
    """{"created", "answered"} counted over {agent: stream lines}: successful ask and answer commands, by their results."""
    binre = re.escape(klatch_bin) if klatch_bin else r"(?:\S*/)?klatch"
    pat = re.compile(binre + r"\s+(?:-\S+\s+)*(ask|answer)\b")
    created = answered = 0
    for lines in streams.values():
        for turn in chain.parse_stream(lines):
            for tool in turn["tools"]:
                cmd = chain._command(tool)
                res = tool["result"] or ""
                for m in pat.finditer(cmd):
                    if m.group(1) == "ask" and ASKED.search(res):
                        created += 1
                    elif m.group(1) == "answer" and ANSWERED.search(res):
                        answered += 1
    return {"created": created, "answered": answered}


def cross_check(stream_counts, store_counts):
    """Both sides and whether they agree: {"streams", "store", "match", "differences"}."""
    diffs = ["%s: streams %d, store %d" % (k, stream_counts[k], store_counts[k])
             for k in ("created", "answered") if stream_counts[k] != store_counts[k]]
    return {"streams": dict(stream_counts), "store": dict(store_counts), "match": not diffs, "differences": diffs}
