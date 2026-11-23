#!/bin/sh
# Make the promo's demo repository: pr-team's toy todo app, with the stub pipeline in
# .nex/pipelines/demo.toml. Run `nex-host pipe demo "add stats, search and export"` in it.
# usage: setup.sh DIR
set -eu
[ $# -eq 1 ] || { echo "usage: $0 DIR" >&2; exit 2; }
here=$(cd "$(dirname "$0")" && pwd)
seed=$here/../../../nex-host/examples/pr-team/seed
[ ! -e "$1" ] || { echo "$1 already exists" >&2; exit 1; }
mkdir -p "$1/.nex/pipelines"
cp -R "$seed/." "$1/"
sed "s|@STUB@|$here/stub-agent|g" "$here/pipeline.toml" > "$1/.nex/pipelines/demo.toml"
cd "$1"
git init -q -b main
git add -A
git -c user.name=demo -c user.email=demo@example.com commit -q -m "A todo app"
echo "Made $(pwd)"
