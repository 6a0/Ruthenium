"""Per-run table of an A/B directory (results/<dir>/{a,b}): avg fps, p99 low and render thread CPU ms per frame per
phase, in run order, with medians. Usage: python scripts/ab_table.py build/run/bench/results/<dir> [phase ...]"""
import json
import statistics
import sys
from pathlib import Path

root = Path(sys.argv[1])
phases = sys.argv[2:] or ["static", "pan"]
for side in ("a", "b"):
    runs = sorted((root / side).glob("*.json"))
    rows = {p: [] for p in phases}
    for run in runs:
        data = json.loads(run.read_text())
        for phase in data["phases"]:
            if phase["name"] in rows:
                rows[phase["name"]].append((phase["avgFps"], phase["p99LowFps"], phase.get("renderThreadCpuMsPerFrame", 0.0)))
    for p in phases:
        r = rows[p]
        if not r:
            continue
        cells = "  ".join(f"{a:7.1f} {p99:6.1f} {cpu:5.2f}" for a, p99, cpu in r)
        med = tuple(statistics.median(x[i] for x in r) for i in range(3))
        print(f"{side} {p:7s} {cells}   median {med[0]:7.1f} p99 {med[1]:6.1f} cpu {med[2]:5.2f}")
