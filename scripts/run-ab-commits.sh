#!/bin/sh
# Interleaved A/B of two commits, checked out in turn in the main tree (ABBA), for one bench task.
# Usage (from the repo root): sh scripts/run-ab-commits.sh <pairs> <dir> <commitA> <commitB> <gradle args...>
# Copy this script out of the tree first if commit A predates it (checking A out removes it mid-run).
# Commits are resolved to hashes first (a symbolic HEAD would follow the checkouts)
pairs=$1; dir=$2; ca=$(git rev-parse --short "$3") || exit 1; cb=$(git rev-parse --short "$4") || exit 1; shift 4
gargs="$*"
R=build/run/bench/results
mkdir -p build/bench-logs "$R/$dir/a/frames" "$R/$dir/b/frames"
back=$(git branch --show-current)
run() {
  side=$1; commit=$2
  git checkout -q "$commit" || exit 1
  n=$(ls build/bench-logs | wc -l)
  L=build/bench-logs/$n-abc-$side.log
  # shellcheck disable=SC2086
  ./gradlew --console=plain $gargs > "$L" 2>&1
  r=$(grep -oE "NTSTATUS 0x[0-9A-F]+|AssertionError: .{0,120}|BUILD [A-Z]+" "$L" | sort -u | tr '\n' ' ')
  for f in "$R"/thallium*-2026*.json "$R"/thallium*-2026*.txt; do [ -e "$f" ] && mv "$f" "$R/$dir/$side/"; done
  for f in "$R"/frames/thallium*-2026*; do [ -e "$f" ] && mv "$f" "$R/$dir/$side/frames/"; done
  cp "$L" "$R/$dir/$side/"
  echo "$side ($commit): $r $(grep -hE '^(static|pan|initial|particles) ' "$L" | sed -E 's/^([a-z]+) .*avg= *([0-9.]+).*p99 low= *([0-9.]+).*/\1 \2 (p99 \3)/' | tr '\n' ' ')"
}
i=1
while [ $i -le "$pairs" ]; do
  if [ $((i % 2)) -eq 1 ]; then run a "$ca"; run b "$cb"; else run b "$cb"; run a "$ca"; fi
  i=$((i+1))
done
git checkout -q "$back"
