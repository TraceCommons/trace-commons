"""Generate a size-realistic Codex rollout fixture for the scan benchmark.

Mirrors the distribution measured on a real machine (median ~541KB,
p90 ~11.6MB, long tail), at 400 files instead of 3066 so the bench re-runs
quickly. cwd lives on line 1, exactly as real rollouts have it.
"""

import json
import os
import random
import sys

random.seed(1)
root = sys.argv[1] if len(sys.argv) > 1 else "/tmp/sb/sessions/2026/08/20"
os.makedirs(root, exist_ok=True)

sizes = []
for _ in range(400):
    r = random.random()
    if r < 0.5:
        sizes.append(random.randint(200_000, 900_000))
    elif r < 0.9:
        sizes.append(random.randint(900_000, 12_000_000))
    elif r < 0.99:
        sizes.append(random.randint(12_000_000, 40_000_000))
    else:
        sizes.append(random.randint(40_000_000, 90_000_000))

total = 0
blob = "x" * 800
for i, target in enumerate(sizes):
    p = os.path.join(root, "rollout-2026-08-20T10-00-%02d-sess%d.jsonl" % (i % 60, i))
    with open(p, "w") as f:
        f.write(
            json.dumps(
                {
                    "type": "session_meta",
                    "payload": {
                        "cwd": "/Users/z/code/proj%d" % (i % 7),
                        "cli_version": "1.2.3",
                    },
                }
            )
            + "\n"
        )
        written = 0
        while written < target:
            line = json.dumps({"type": "event", "payload": {"text": blob, "i": written}})
            f.write(line + "\n")
            written += len(line) + 1
    total += os.path.getsize(p)

print("generated %d files, %.2fGB" % (len(sizes), total / 1e9))
