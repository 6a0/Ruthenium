#!/bin/sh
# Interleaved A/B benchmark runs: N pairs in ABBA order (A then B, then B then A, ...), moving each run's results into
# build/run/bench/results/<dir>/{a,b}. Arguments are extra Gradle arguments for each side. Move earlier thallium-*
# results out of results/ first; every one there is moved into the run's side.
# Usage: scripts/run-ab.sh <pairs> <dir> "<common args>" "<A args>" "<B args>"
#   e.g. scripts/run-ab.sh 3 entities-dev/meshref "benchThallium -Pbench.scene=entities" "-Pjvm.x=true" ""
# Summarize with: python scripts/bench_summary.py build/run/bench/results/<dir>/a  (and .../b)
pairs=$1; dir=$2; common=$3; a=$4; b=$5
R=build/run/bench/results
mkdir -p build/bench-logs "$R/$dir/a/frames" "$R/$dir/b/frames"
run() {
  side=$1; args=$2
  n=$(ls build/bench-logs | wc -l)
  L=build/bench-logs/$n-ab-$side.log
  # shellcheck disable=SC2086
  ./gradlew --console=plain $common $args > "$L" 2>&1
  r=$(grep -oE "NTSTATUS 0x[0-9A-F]+|AssertionError: .{0,120}|BUILD [A-Z]+" "$L" | sort -u | tr '\n' ' ')
  for f in "$R"/thallium*-2026*.json "$R"/thallium*-2026*.txt; do [ -e "$f" ] && mv "$f" "$R/$dir/$side/"; done
  for f in "$R"/frames/thallium*-2026*; do [ -e "$f" ] && mv "$f" "$R/$dir/$side/frames/"; done
  cp "$L" "$R/$dir/$side/"
  echo "$side: $r $(grep -hE '^(static|pan|initial|particles) ' "$L" | sed -E 's/^([a-z]+) .*avg= *([0-9.]+).*p99 low= *([0-9.]+).*/\1 \2 (p99 \3)/' | tr '\n' ' ')"
}
i=1
while [ $i -le "$pairs" ]; do
  if [ $((i % 2)) -eq 1 ]; then run a "$a"; run b "$b"; else run b "$b"; run a "$a"; fi
  i=$((i+1))
done
