"""Client tick numbers of an A/B directory, per run in run order and as medians.

Usage: python scripts/tick_ab.py build/run/bench/results/<dir> [phase ...]

Reads the game logs that run-ab.sh / run-ab-commits.sh copy into <dir>/a and <dir>/b (the tick probe's
"<phase>: ticks N: X ms per tick, entities Y, block entities Z" line and the tick split's "<phase>: frames with a
tick: N, avg A ms, median M, p99 P; without: ...") and the result JSONs (p95/p99 low fps).
"""
import glob
import json
import os
import re
import statistics
import sys

TICK = re.compile(r"\]: (\w+): ticks \d+: ([\d.]+) ms per tick, entities ([\d.]+), block entities ([\d.]+)")
SPLIT = re.compile(r"\]: (\w+): frames with a tick: \d+, avg ([\d.]+) ms, median [\d.]+, p99 ([\d.]+); without: \d+, avg ([\d.]+) ms")


def runs(side_dir):
    out = []
    for log in sorted(glob.glob(os.path.join(side_dir, "*.log")), key=lambda p: int(os.path.basename(p).split("-")[0])):
        phases = {}
        with open(log, encoding="utf-8", errors="replace") as f:
            for line in f:
                m = TICK.search(line)
                if m:
                    phases.setdefault(m.group(1), {}).update(tick=float(m.group(2)), ent=float(m.group(3)))
                m = SPLIT.search(line)
                if m:
                    phases.setdefault(m.group(1), {}).update(tickframe=float(m.group(2)), tickp99=float(m.group(3)),
                                                             other=float(m.group(4)))
        out.append((os.path.basename(log), phases))
    fps = []
    for js in sorted(glob.glob(os.path.join(side_dir, "*.json"))):
        with open(js) as f:
            fps.append({p["name"]: p for p in json.load(f)["phases"]})
    return out, fps


def main():
    base = sys.argv[1]
    wanted = sys.argv[2:] or ["static", "pan"]
    for phase in wanted:
        print(f"{phase}: tick ms | entities ms | frame with tick ms | its p99 ms | frame without ms | p95 fps | p99 fps")
        for side in ("a", "b"):
            logs, fps = runs(os.path.join(base, side))
            cols = {k: [] for k in ("tick", "ent", "tickframe", "tickp99", "other", "p95", "p99")}
            for i, (name, phases) in enumerate(logs):
                p = phases.get(phase, {})
                j = fps[i].get(phase, {}) if i < len(fps) else {}
                row = [p.get("tick"), p.get("ent"), p.get("tickframe"), p.get("tickp99"), p.get("other"),
                       j.get("p95LowFps"), j.get("p99LowFps")]
                for k, v in zip(cols, row):
                    if v is not None:
                        cols[k].append(v)
                print(f"  {side} {name:<16} " + " ".join("%8.3f" % v if v is not None else "       -" for v in row))
            med = [statistics.median(v) if v else None for v in cols.values()]
            print(f"  {side} {'median':<16} " + " ".join("%8.3f" % v if v is not None else "       -" for v in med))


if __name__ == "__main__":
    main()
