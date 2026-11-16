#!/usr/bin/env bash
# Where nex's idle memory goes, on Linux. Starts nex (release build, throwaway config) with one
# shell at rest, waits as bench/run.sh does, then prints the resident-memory split and the
# mappings holding the most of it. Usage: bench/mem.sh [SECONDS_TO_WAIT]   (default 2, at most
# 4: the shell from inner.sh exits after 5 s, which closes nex)
set -euo pipefail

repo=$(cd "$(dirname "$0")/.." && pwd)
wait_s=${1:-2}
tmp=$(mktemp -d)
trap 'kill "$pid" 2>/dev/null || true; rm -rf "$tmp"' EXIT

cargo build --release --manifest-path "$repo/Cargo.toml" -q
mkdir -p "$tmp/cfg/nex"
echo "command = env BENCH_OUT=$tmp BENCH_MODE=idle $repo/bench/inner.sh" >"$tmp/cfg/nex/config"
XDG_CONFIG_HOME="$tmp/cfg" "$repo/target/release/nex" >/dev/null 2>&1 &
pid=$!
i=0
until [ -s "$tmp/started" ]; do
  sleep 0.05; i=$((i + 1))
  [ $i -lt 600 ] || { echo "nex's shell never started" >&2; exit 1; }
done
sleep "$wait_s"

echo "== nex $(grep -m1 '^version' "$repo/Cargo.toml" | cut -d'"' -f2), ${wait_s}s after its shell started"
grep -E '^(VmRSS|RssAnon|RssFile|RssShmem|VmHWM|Threads):' "/proc/$pid/status"
echo
echo "== Resident memory by mapping (MB), largest first"
awk '
  /^[0-9a-f]+-[0-9a-f]+ / { name = (NF >= 6) ? $6 : "[anon]"; for (i = 7; i <= NF; i++) name = name " " $i; next }
  /^Rss:/ { rss[name] += $2 }
  END { for (n in rss) if (rss[n] >= 256) printf "%8.1f  %s\n", rss[n] / 1024, n }
' "/proc/$pid/smaps" | sort -rn | head -20
