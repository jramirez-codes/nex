# App Store screenshots

- `6.3/`: the set to upload. App Store Connect's iPhone slot ("iPhone with Dynamic Island,
  medium display") takes 1206 × 2622 and uses it for every iPhone size. JPEG, no alpha. In
  upload order:
  1. Agents pane: the demo pipeline, three builders working in parallel
  2. Agents pane: review and merge, one agent waiting on you
  3. Claude Code running in a pane
  4. Shell with git history
  5. Connect screen
- `0*.jpg`: the first three at 1320 × 2868 (the 6.9" size), used for 0.14.0.
- `source/`: full-size Simulator captures the Agents shots came from.

Captured on the iPhone 18 Pro Max Simulator (iOS 27.2), connected to an ssh host. The Agents
shots run `tools/demo/agents/` (stub agents, nothing real) in `~/nex-demo` on that host:
`nex-host pipe demo "add stats, search and export" --bg`. `raw/` (frames and video takes)
is ignored.
