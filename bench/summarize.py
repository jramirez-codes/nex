#!/usr/bin/env python3
"""Turn run.sh's raw results into bench.json, merged over the terminals already in it.
Usage: summarize.py RESULTS VERSIONS RUNS EXISTING_JSON"""
import json
import os
import platform
import statistics
import subprocess
import sys
from collections import defaultdict

results, versions, runs, existing = sys.argv[1], sys.argv[2], int(sys.argv[3]), sys.argv[4]
samples = defaultdict(lambda: defaultdict(list))
for row in open(results):
    term, metric, value = row.rstrip("\n").split("\t")
    samples[metric][term].append(int(value))

names = dict(row.rstrip("\n").split("\t", 1) for row in open(versions))
cpu = next(l.split(":", 1)[1].strip() for l in open("/proc/cpuinfo") if l.startswith("model name"))
gpu = subprocess.run("lspci | grep -iE 'vga|3d' | head -1 | cut -d: -f3", shell=True,
                     capture_output=True, text=True).stdout.strip()

# Start from the previous results so re-measuring one terminal keeps everyone else's.
old = json.load(open(existing)) if os.path.exists(existing) else {}
out = {
    "machine": {"cpu": cpu, "gpu": gpu, "kernel": platform.release()},
    "runs": old.get("runs", {}),
    "versions": old.get("versions", {}),
    "metrics": old.get("metrics", {}),
}
if isinstance(old.get("machine", {}).get("runs"), int):  # older format: one count for all
    out["runs"] = {t: old["machine"]["runs"] for t in out["versions"]}
for term in names:
    out["runs"][term] = runs
    out["versions"][term] = names[term]
for metric, per in samples.items():
    out["metrics"].setdefault(metric, {}).update({t: statistics.median(v) for t, v in per.items()})
print(json.dumps(out, indent=2))
