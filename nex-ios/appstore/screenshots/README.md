# App Store screenshots

- `6.3/`: the set to upload. App Store Connect's iPhone slot ("iPhone with Dynamic Island,
  medium display") takes 1206 × 2622 and uses it for every iPhone size. JPEG, no alpha. In
  upload order:
  1. Agents pane: the demo pipeline, three builders working in parallel
  2. Agents pane: review and merge, one agent waiting on you
  3. Claude Code running in a pane
  4. Shell with git history
  5. Connect screen
- `promo/`: six captioned screenshots at the same 1206 × 2622, made by
  `tools/demo/appstore.py` on Linux (no Mac or Simulator): the Agents pane with the demo
  pipeline (builders working, then review), one of its agents, the Changes pane's diff, the
  file explorer and a shell. `promo/screen/` has the bare screens. The script runs
  `nex --remote` at an iPhone's shape against a private sshd with a throwaway home, and draws
  the status bar and home indicator itself; the on-screen keyboard button isn't in them, and
  the window is 4% wider than the phone (a desktop nex's minimum), so everything is that much
  smaller than on a real iPhone. Run it again after a UI change: `tools/demo/appstore.py`
  (`--plain` for no captions).
- `0*.jpg`: the first three at 1320 × 2868 (the 6.9" size), used for 0.14.0.
- `source/`: full-size Simulator captures the Agents shots came from.

Captured on the iPhone 18 Pro Max Simulator (iOS 27.2), connected to an ssh host. The Agents
shots run `tools/demo/agents/` (stub agents, nothing real) in `~/nex-demo` on that host:
`nex-host pipe demo "add stats, search and export" --bg`. `raw/` (frames and video takes)
is ignored.
