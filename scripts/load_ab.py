"""Startup milestones of load probe launches (-Pbench.loadProbe=true), per side and run, with medians.

Usage: python scripts/load_ab.py <dir>
The directory holds <side>-<n>.out (Gradle output: the probe's early marks are printed before logging starts) and
<side>-<n>.log (the game log) per launch, as scratch A/B loops write them.
"""
import re
import statistics
import sys
from pathlib import Path

MARKS = [
    ("main", "Main.main"),
    ("bootstrap", "before Bootstrap.bootStrap"),
    ("caches0", "block state caches start"),
    ("caches1", "block state caches done"),
    ("bootstrapped", "after Bootstrap.bootStrap"),
    ("mcInit", "Minecraft.<init>: pack sources made"),
    ("device", "Minecraft.<init>: device created"),
    ("reload0", "Minecraft.<init>: initial resource reload starts"),
    ("mcDone", "Minecraft.<init> returns"),
    ("reload1", "loading overlay: reload done, fade out starts"),
]


def parse(out_file, log_file):
    log = log_file.read_text(encoding="utf-8", errors="replace")
    # The probe's early marks are only in the Gradle output (printed before logging starts)
    text = out_file.read_text(encoding="utf-8", errors="replace") + "\n" + log
    row = {}
    for key, needle in MARKS:
        m = re.search(r"\[LoadProbe\] (\d+) ms " + re.escape(needle), text)
        if m:
            row[key] = int(m.group(1))
    m = re.search(r"Title screen after ([\d.]+) s", text)
    if m:
        row["title"] = int(float(m.group(1)) * 1000)
    reloads = [int(b) - int(a) for a, b in re.findall(r"Reload from uptime (\d+) ms to (\d+) ms", log)]
    row["reloads"] = reloads
    works = [int(x) for x in re.findall(r"Resource reload finished after (\d+) ms", log)]
    row["works"] = works
    m = re.search(r"Built \d+ block state caches in (\d+) ms", log)
    if m:
        row["parallelCaches"] = int(m.group(1))
    m = re.search(r"Block state cache digest: (\w+)", log)
    if m:
        row["digest"] = m.group(1)[:12]
    return row


def main():
    d = Path(sys.argv[1])
    runs = {}
    for out in sorted(d.glob("*.out")):
        side, n = out.stem.split("-")
        log = out.with_suffix(".log")
        if log.exists():
            runs.setdefault(side, []).append((int(n), parse(out, log)))
    cols = ["main", "bootstrap", "caches0", "caches1", "bootstrapped", "mcInit", "device", "reload0", "mcDone", "reload1", "title"]
    print("side run " + " ".join(f"{c:>12}" for c in cols) + "  F3+T (future) / reload work ms")
    for side in sorted(runs):
        rows = sorted(runs[side])
        for n, r in rows:
            print(f"{side:>4} {n:>3} " + " ".join(f"{r.get(c, ''):>12}" for c in cols)
                  + f"  {r['reloads']} / {r['works']}" + (f" parallel caches {r['parallelCaches']} ms" if 'parallelCaches' in r else "")
                  + (f" digest {r['digest']}" if 'digest' in r else ""))
        med = []
        for c in cols:
            vals = [r[c] for _, r in rows if c in r]
            med.append(f"{statistics.median(vals):>12.0f}" if vals else f"{'':>12}")
        print(f"{side:>4} med " + " ".join(med))
    # Durations of the steps
    print()
    steps = [("pre-main", None, "main"), ("main->bootstrap", "main", "bootstrap"), ("bootstrap", "bootstrap", "bootstrapped"),
             ("  state caches", "caches0", "caches1"), ("bootstrapped->mcInit", "bootstrapped", "mcInit"),
             ("Minecraft.<init>", "mcInit", "mcDone"), ("reload (initial)", "reload0", "reload1"), ("fade", "reload1", "title")]
    print(f"{'step':<22}" + "".join(f"{s:>10}" for s in sorted(runs)))
    for name, a, b in steps:
        line = f"{name:<22}"
        for side in sorted(runs):
            vals = [r[b] - (r[a] if a else 0) for _, r in runs[side] if b in r and (a is None or a in r)]
            line += f"{statistics.median(vals):>10.0f}" if vals else f"{'':>10}"
        print(line)


if __name__ == "__main__":
    main()
