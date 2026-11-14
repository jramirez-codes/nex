#!/usr/bin/env bash
# Benchmark nex against the other terminals installed on this machine and write the medians
# to site/src/data/bench.json. Opens (and closes) real terminal windows, so leave the desktop
# alone while it runs. Usage: bench/run.sh [RUNS] [TERMINAL...]   (default: 3 runs, every
# installed terminal). Naming terminals re-measures only those and keeps the others' results.
set -euo pipefail

repo=$(cd "$(dirname "$0")/.." && pwd)
runs=${1:-3}
shift || true
only=("$@")
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

cargo build --release --manifest-path "$repo/Cargo.toml" -q
nex="$repo/target/release/nex"
inner="$repo/bench/inner.sh"
data="$tmp/data.txt"
python3 "$repo/bench/gen_data.py" "$data" 50

terminals=()
for t in nex alacritty kitty ghostty gnome-terminal; do
  if [ ${#only[@]} -gt 0 ] && [[ ! " ${only[*]} " =~ " $t " ]]; then continue; fi
  if [ "$t" = nex ] || command -v "$t" >/dev/null; then terminals+=("$t"); fi
done

# launch TERMINAL MODE OUTDIR: start the terminal running inner.sh in the background and print
# the PID whose memory represents it.
launch() {
  local term=$1 mode=$2 out=$3
  local envs=(BENCH_OUT="$out" BENCH_MODE="$mode" BENCH_DATA="$data")
  case $term in
    nex)
      mkdir -p "$out/cfg/nex"
      echo "command = env ${envs[*]} $inner" >"$out/cfg/nex/config"
      XDG_CONFIG_HOME="$out/cfg" "$nex" >/dev/null 2>&1 & echo $! ;;
    alacritty) alacritty -e env "${envs[@]}" "$inner" >/dev/null 2>&1 & echo $! ;;
    kitty) kitty env "${envs[@]}" "$inner" >/dev/null 2>&1 & echo $! ;;
    ghostty) ghostty --gtk-single-instance=false -e env "${envs[@]}" "$inner" >/dev/null 2>&1 & echo $! ;;
    gnome-terminal)
      gnome-terminal --wait -- env "${envs[@]}" "$inner" >/dev/null 2>&1 &
      wait_for "$out/started" 30
      pgrep -nf gnome-terminal-server ;;
  esac
}

wait_for() { # FILE TIMEOUT_S
  local i=0
  until [ -s "$1" ]; do
    sleep 0.05; i=$((i + 1))
    [ $i -lt $(($2 * 20)) ] || return 1
  done
}

finish() { # PID TERM: let the window close on its own, then make sure it's gone
  local i=0
  while kill -0 "$1" 2>/dev/null && [ $i -lt 100 ]; do sleep 0.05; i=$((i + 1)); done
  [ "$2" = gnome-terminal ] || kill "$1" 2>/dev/null || true
  sleep 0.5
}

results="$tmp/results.tsv"
for term in "${terminals[@]}"; do
  for run in $(seq "$runs"); do
    # Startup + throughput in one launch.
    out="$tmp/$term-$run-t"; mkdir -p "$out"
    t0=$(date +%s%N)
    pid=$(launch "$term" throughput "$out")
    wait_for "$out/throughput" 300 || { echo "$term: throughput run $run timed out" >&2; continue; }
    printf '%s\tstartup_ms\t%s\n' "$term" $(( ($(cat "$out/started") - t0) / 1000000 )) >>"$results"
    printf '%s\tthroughput_ms\t%s\n' "$term" $(( $(cat "$out/throughput") / 1000000 )) >>"$results"
    finish "$pid" "$term"

    # Idle memory: one shell sitting at rest for 2s.
    out="$tmp/$term-$run-i"; mkdir -p "$out"
    pid=$(launch "$term" idle "$out")
    wait_for "$out/started" 30 && sleep 2
    rss=$(awk '/VmRSS/ {print $2}' "/proc/$pid/status" 2>/dev/null || echo "")
    [ -n "$rss" ] && printf '%s\tidle_rss_mb\t%s\n' "$term" $((rss / 1024)) >>"$results"
    finish "$pid" "$term"
    echo "$term run $run/$runs done" >&2
  done
done

versions="$tmp/versions.tsv"
for term in "${terminals[@]}"; do
  case $term in
    nex) v="nex $(grep -m1 '^version' "$repo/Cargo.toml" | cut -d'"' -f2)" ;;
    gnome-terminal) v=$(gnome-terminal --version | sed 's/^# //; s/ +.*//') ;;
    *) v=$("$term" --version 2>/dev/null | head -1 | sed "s/ created by.*//") ;;
  esac
  printf '%s\t%s\n' "$term" "$v" >>"$versions"
done

out_json="$repo/site/src/data/bench.json"
python3 "$repo/bench/summarize.py" "$results" "$versions" "$runs" "$out_json" >"$tmp/bench.json"
mv "$tmp/bench.json" "$out_json"
echo "wrote site/src/data/bench.json" >&2
