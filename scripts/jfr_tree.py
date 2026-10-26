"""Inclusive-time breakdown of JFR execution samples for one thread.

Usage:
    jfr print --json --stack-depth 96 --events jdk.ExecutionSample build/run/bench/profiles/X.jfr > exec.json
    python scripts/jfr_tree.py exec.json main [--from HH:MM:SS] [--to HH:MM:SS] [substring ...]

Prints, for the given thread (the render thread is named "main" in JFR), the share of samples in which each
method appears anywhere on the stack. Optional substrings restrict to samples whose stack contains one of them;
--from/--to restrict to samples in a wall clock window (e.g. one benchmark phase, from the game log timestamps).
"""
import json
import sys
from collections import Counter


def frame_name(frame):
    method = frame['method']
    return f"{method['type']['name'].split('.')[-1]}.{method['name']}"


def main():
    data = json.load(open(sys.argv[1], encoding='utf-8'))
    thread = sys.argv[2]
    args = sys.argv[3:]
    start, end = '', '99'
    while args and args[0] in ('--from', '--to'):
        if args[0] == '--from':
            start = args[1]
        else:
            end = args[1]
        args = args[2:]
    focus = args
    events = [e for e in data['recording']['events'] if e['values']['sampledThread']['javaName'] == thread
              and start <= e['values']['startTime'][11:19] < end]

    inclusive = Counter()
    for event in events:
        names = [frame_name(f) for f in event['values']['stackTrace']['frames']]
        if focus and not any(any(s in n for n in names) for s in focus):
            continue
        for name in set(names):
            inclusive[name] += 1

    total = len(events)
    print(f"{thread}: {total} samples")
    for name, count in inclusive.most_common(70):
        print(f"{100 * count / total:6.1f}%  {name}")


if __name__ == '__main__':
    main()
