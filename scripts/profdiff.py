"""Mean light profiler ms/frame per section, on minus off, of an in-launch A/B log (bench.toggleField with
bench.lightProfile). Usage: python scripts/profdiff.py <game log> [first round, default 0] [rows, default 40]"""
import re, sys, statistics
from collections import defaultdict
log = open(sys.argv[1], encoding='utf-8', errors='replace').read().splitlines()
minround = int(sys.argv[2]) if len(sys.argv) > 2 else 0
data = {'on': defaultdict(list), 'off': defaultdict(list)}
i = 0
while i < len(log):
    m = re.match(r'Light profile of (\w+)-(on|off)-(\d+): (\d+) frames, ([\d.]+) ms/frame', log[i])
    if m and int(m.group(3)) >= minround:
        side = m.group(2)
        data[side]['TOTAL'].append(float(m.group(5)))
        stack = []
        i += 2
        while i < len(log) and log[i].startswith(' ') or (i < len(log) and re.match(r'^\S+\s+[\d.]+\s+[\d.]+', log[i])):
            line = log[i]
            mm = re.match(r'^( *)(\S.*?)\s+([\d.]+)\s+([\d.]+)\s+([\d.]+)\s+([\d.]+)$', line)
            if mm:
                depth = len(mm.group(1)) // 2
                stack = stack[:depth] + [mm.group(2)]
                data[side]['/'.join(stack)].append(float(mm.group(3)))
            i += 1
        continue
    i += 1
rows = []
for k in data['on']:
    if k in data['off']:
        a = statistics.mean(data['off'][k]); b = statistics.mean(data['on'][k])
        rows.append((b - a, k, a, b))
rows.sort(key=lambda r: -abs(r[0]))
for d, k, a, b in rows[:int(sys.argv[3]) if len(sys.argv) > 3 else 40]:
    print(f"{d*1000:8.1f} us  off {a:.4f} on {b:.4f}  {k}")
