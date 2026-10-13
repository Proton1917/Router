import json
import math
import os
from pathlib import Path
import re
import sys


def model_key(value):
    return re.sub(r"\[1m\]", "", value, flags=re.I).lower() if isinstance(value, str) else ""


def input_tokens(usage):
    if not isinstance(usage, dict) or not isinstance(usage.get("input_tokens"), int):
        return None
    values = [usage.get(key) or 0 for key in ("input_tokens", "cache_creation_input_tokens", "cache_read_input_tokens")]
    if any(isinstance(value, bool) or not isinstance(value, int) or value < 0 for value in values):
        raise ValueError("上下文 usage 必须包含非负整数")
    return sum(values)


def transcript_tokens(payload):
    source = payload.get("transcript_path")
    model = model_key(payload.get("model", {}).get("id"))
    if not source or not model:
        return None
    path = Path(source)
    if not path.exists():
        return None
    with path.open("rb") as transcript:
        transcript.seek(0, os.SEEK_END)
        start = max(0, transcript.tell() - 262144)
        transcript.seek(start)
        if start:
            transcript.readline()
        lines = transcript.read().splitlines(keepends=True)
    for line in reversed(lines):
        if not line.endswith(b"\n"):
            continue
        row = json.loads(line)
        if row.get("type") == "system" and row.get("subtype") == "compact_boundary":
            return None
        if row.get("type") != "assistant" or row.get("isSidechain") or row.get("isApiErrorMessage"):
            continue
        message = row.get("message", {})
        if model not in {model_key(row.get("requestedModel")), model_key(message.get("model"))}:
            continue
        count = input_tokens(message.get("usage"))
        if count is not None and count > 0:
            return count
    return None


def remaining(payload):
    context = payload.get("context_window", {})
    window = context.get("context_window_size")
    if isinstance(window, bool) or not isinstance(window, int) or window <= 0:
        return context.get("remaining_percentage")
    current = input_tokens(context.get("current_usage"))
    recorded = transcript_tokens(payload)
    counts = [count for count in (current, recorded) if count is not None]
    if not counts:
        return context.get("remaining_percentage")
    used = min(100, max(0, math.floor(max(counts) / window * 100 + 0.5)))
    return 100 - used


if __name__ == "__main__":
    result = remaining(json.load(sys.stdin))
    if result is not None:
        print(result)
