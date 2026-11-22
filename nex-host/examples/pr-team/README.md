# pr-team: three builders, one merger, real PRs

The flow agent teams are tested against:

```
build-stats  ─▶ open PR ─┐
build-search ─▶ open PR ─┼─▶ merge: review + merge, first done first merged
build-export ─▶ open PR ─┘
```

- Each builder is a Claude Code agent in a git worktree of its own (`worktree = true`). It
  builds one feature of a tiny todo app, opens a PR, and reports it with `done --pr N`.
- The `merge` stage has `each = true`. The pipeline gives each report, in the order they
  arrive, to a fresh Claude agent that reviews the PR and merges it, one PR at a time.
- If the merger sends a PR back (`--verdict changes`: a conflict, missing tests), the
  builder, still running, gets that as a new task, fixes it, and its PR rejoins the line.

The three features live in their own files (commands are discovered, so none edit a shared
file), so this run should merge cleanly. To watch the send-back loop, give two builders the
same file to touch.

## Running it

You need `gh` signed in, Claude Code, and `nex-host` on your `PATH` (on an ssh host nex
installs it in `~/.nex/bin`).

```sh
./setup.sh ~/nex-pr-team-demo              # the toy app + .nex/pipelines/pr-team.toml
cd ~/nex-pr-team-demo
gh repo create nex-pr-team-demo --private --source . --push
nex-host pipe pr-team "go"                  # or --bg, and watch it in nex's Agents pane
```

The seed's `.claude/settings.json` lets the agents use git, `gh pr`, the tests and
nex-host without asking, and accept their own edits. Nothing else is pre-allowed.

The runner's screen shows the line as it moves:

```
▸ build-search (PR 2) is next: added search …
  merge-1 (claude) is on it: 5d1e…
✓ build-search (PR 2) approved: merged PR 2 …
...
✓ finished. merge approved 3: build-search (PR 2), build-stats (PR 1), build-export (PR 3)
```

Afterwards:

```sh
git pull                    # main has all three
nex-host worktree prune     # removes the builders' worktrees and branches, now merged
gh repo delete nex-pr-team-demo
```

`tests/teams.rs` runs the same flow with shell scripts standing in for the agents.
