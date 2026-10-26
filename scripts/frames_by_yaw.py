"""Summarizes a per-frame benchmark CSV: frame time by camera yaw, and the slowest frames.

Usage: python scripts/frames_by_yaw.py <frames.csv> [bin degrees]
"""
import csv
import statistics
import sys

rows = list(csv.DictReader(open(sys.argv[1])))
bin_deg = float(sys.argv[2]) if len(sys.argv) > 2 else 20
for r in rows:
    for k in r:
        r[k] = float(r[k])

total = sum(r['ms'] for r in rows)
bins = {}
for r in rows:
    bins.setdefault(int((r['yaw'] % 360) // bin_deg), []).append(r)
print(f"{'yaw':>9} {'frames':>6} {'time%':>6} {'median':>7} {'p99':>7} {'max':>7} {'BEs':>6} {'ents':>5}")
for b in sorted(bins):
    rs = bins[b]
    ms = sorted(r['ms'] for r in rs)
    print(f"{b * bin_deg:4.0f}-{(b + 1) * bin_deg:<4.0f} {len(rs):6d} {100 * sum(ms) / total:5.1f}% "
          f"{statistics.median(ms):7.2f} {ms[min(len(ms) - 1, int(0.99 * len(ms)))]:7.2f} {ms[-1]:7.2f} "
          f"{statistics.mean(r['blockEntities'] for r in rs):6.0f} {statistics.mean(r['entities'] for r in rs):5.0f}")

# A frame is a spike when it is much slower than the frames around it (not just a heavy view direction)
print("\nspikes (>2.5x the median of the 15 frames on each side):")
spikes = 0
for i, r in enumerate(rows):
    around = [x['ms'] for x in rows[max(0, i - 15):i] + rows[i + 1:i + 16]]
    local = statistics.median(around)
    if r['ms'] > 2.5 * local and r['ms'] > 4:
        spikes += 1
        gc = r['gcs'] != rows[i - 1]['gcs'] if i > 0 else False
        print(f"  frame {int(r['frame']):5d} yaw {r['yaw'] % 360:6.1f} {r['ms']:6.2f} ms (local {local:5.2f})"
              f"{'  GC' if gc else ''}")
print(f"{spikes} spikes, {sum(1 for r in rows if r['ms'] > 1000 / 60)} frames over 16.7 ms of {len(rows)}")
