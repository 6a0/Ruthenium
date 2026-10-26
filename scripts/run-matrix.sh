#!/bin/sh
# Runs each benchmark task N times, logging to build/bench-logs/ and printing one result line per run.
# Usage: scripts/run-matrix.sh <runs> <task>...   e.g. scripts/run-matrix.sh 3 benchVulkanMod benchVanillaVulkan
runs=$1; shift
mkdir -p build/bench-logs
for t in "$@"; do
  i=1
  while [ $i -le $runs ]; do
    n=$(ls build/bench-logs | wc -l)
    L=build/bench-logs/$n-$t.log
    ./gradlew --console=plain $t > $L 2>&1
    r=$(grep -oE "NTSTATUS 0x[0-9A-F]+|AssertionError: .{0,120}|BUILD [A-Z]+" $L | sort -u | tr '\n' ' ')
    echo "$t run $i: $r"
    i=$((i+1))
  done
done
