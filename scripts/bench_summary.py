"""Summarize benchmark result JSONs (build/run/bench/results) into a per-configuration table.

Each value is the median across runs of the same label, with the run count shown per configuration.
Usage: python scripts/bench_summary.py [results_dir] [--since YYYYMMDD-HHMMSS]
"""
import json
import statistics
import sys
from collections import defaultdict
from pathlib import Path

METRICS = [
    ("avgFps", "avg fps", "{:.1f}"),
    ("p95LowFps", "p95 low", "{:.1f}"),
    ("p99LowFps", "p99 low", "{:.1f}"),
    ("allocMBps", "alloc MB/s", "{:.0f}"),
    ("gcCount", "GCs", "{:.0f}"),
    ("avgEntitiesRendered", "entities", "{:.0f}"),
    ("avgBlockEntitiesRendered", "blk ents", "{:.0f}"),
    ("renderThreadCpuMsPerFrame", "cpu ms/f", "{:.2f}"),
]
PHASES = ["initial", "pan", "static", "particles", "rebuild"]
# Per-run values the terrain benchmark records in its environment
ENV_METRICS = [("rebuildSeconds", "rebuild s", "{:.3f}"), ("loadSeconds", "load s", "{:.2f}"), ("joinSeconds", "join s", "{:.2f}"),
               ("renderedSections", "sections", "{:.0f}")]


def main():
    args = sys.argv[1:]
    since = None
    if "--since" in args:
        i = args.index("--since")
        since = args[i + 1]
        del args[i:i + 2]
    results_dir = Path(args[0]) if args else Path("build/run/bench/results")

    runs = defaultdict(list)
    for path in sorted(results_dir.glob("*.json")):
        stamp = path.stem.rsplit("-", 2)[-2:]
        if since and "-".join(stamp) < since:
            continue
        data = json.loads(path.read_text())
        runs[data["environment"]["label"]].append(data)

    if not runs:
        print("no results")
        return

    env_rows = []
    for label, label_runs in sorted(runs.items()):
        values = []
        for key, _, fmt in ENV_METRICS:
            samples = [r["environment"][key] for r in label_runs if key in r["environment"]]
            values.append(fmt.format(statistics.median(samples)) if samples else None)
        if any(values):
            env_rows.append((label, len(label_runs), values))
    if env_rows:
        print("\n## terrain loading")
        header = f"{'config':<20}{'runs':>5}" + "".join(f"{title:>12}" for _, title, _ in ENV_METRICS)
        print(header)
        print("-" * len(header))
        for label, count, values in env_rows:
            print(f"{label:<20}{count:>5}" + "".join(f"{v or '-':>12}" for v in values))

    for phase in PHASES:
        if not any(p["name"] == phase for rs in runs.values() for r in rs for p in r["phases"]):
            continue
        print(f"\n## {phase}")
        header = f"{'config':<20}{'runs':>5}" + "".join(f"{title:>12}" for _, title, _ in METRICS)
        print(header)
        print("-" * len(header))
        for label, label_runs in sorted(runs.items()):
            values = []
            for key, _, fmt in METRICS:
                samples = [p[key] for r in label_runs for p in r["phases"] if p["name"] == phase and key in p]
                values.append(fmt.format(statistics.median(samples)) if samples else "-")
            if all(v == "-" for v in values):
                continue
            print(f"{label:<20}{len(label_runs):>5}" + "".join(f"{v:>12}" for v in values))


if __name__ == "__main__":
    main()
