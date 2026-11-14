#!/bin/sh
# Runs inside each terminal under test. Reads BENCH_OUT (result dir), BENCH_MODE and BENCH_DATA.
date +%s%N > "$BENCH_OUT/started"
case "$BENCH_MODE" in
  throughput)
    s=$(date +%s%N); cat "$BENCH_DATA"; e=$(date +%s%N)
    echo $((e - s)) > "$BENCH_OUT/throughput" ;;
  idle)
    sleep 5 ;;
esac
