#!/bin/sh
# Make the demo repository: the toy todo app, with the pipeline in .nex/pipelines/.
# usage: setup.sh DIR
set -eu
[ $# -eq 1 ] || { echo "usage: $0 DIR" >&2; exit 2; }
here=$(cd "$(dirname "$0")" && pwd)
[ ! -e "$1" ] || { echo "$1 already exists" >&2; exit 1; }
mkdir -p "$1/.nex/pipelines"
cp -R "$here/seed/." "$1/"
cp "$here/pipeline.toml" "$1/.nex/pipelines/pr-team.toml"
cd "$1"
git init -q -b main
git add -A
git commit -q -m "A todo app"
cat <<MSG
Made $(pwd).

Next, put it on GitHub (a throwaway repository) and run the team:
  cd $(pwd)
  gh repo create nex-pr-team-demo --private --source . --push
  nex-host pipe pr-team "go"
MSG
