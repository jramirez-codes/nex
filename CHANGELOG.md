# Changelog

All notable changes to nex are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and nex uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Changed

- In an open team card on the Agents pane, a pipeline's stages hang off it by rounded
  `├` / `└` hairline connectors, and nested rows sit on a wider indent.
- In an open team card on the Agents pane, the helper or stage you're viewing is shown by a
  soft rounded accent tint behind its row, in place of the thin accent bar down its left edge.
- An open team card on the Agents pane no longer gets the "Viewing" ring and pill while only
  one of its helpers or stages is on screen: that row's tint shows it. A closed card, or one
  whose lead is on screen, is marked as before.

### Fixed

- Jumping to an agent from its card (a click, or Enter) no longer leaves that card ringed by
  the keyboard highlight: coming back to the Agents tab, no card has a highlighted border
  until an arrow key moves the highlight again (Enter still goes to the card you last chose).
- On the Agents tab, a terminal opened beside the Agents pane (say, with Claude running over
  ssh in it) now marks its agent's card "Viewing", and only that card: the pinned pane and
  the terminal tab you go back to still don't count.
- On the Agents tab, the agents of the terminal tab you go back to no longer show the
  "Viewing" ring: that tab isn't on screen. An Agents pane split beside terminals still
  marks them as before.
- On the Agents tab, the card of the pinned (sticky) pane's agent no longer shows the
  "Viewing" ring just because that pane is pinned beside the Agents pane.
- Opening the Agents tab while an Agents pane is pinned shows that pane instead of adding a
  second Agents pane beside it.

## [0.14.1] - 2026-11-24

### Changed

- The website is redesigned: one near-black palette with a single blue, large type, and a
  real recording in a plain frame for each section in place of the drawn mock-ups (Agents
  and SSH show the new `agents` and `ssh` recorder scenes once they're recorded, the promo
  and hero until then). The header keeps four links, a GitHub icon and a Download pill.
- The website reads like a product page: a near full-screen "nex" opener whose recording grows
  in as you scroll, a sticky bar with Overview, Agents, SSH and Specs, one chapter per big
  feature (Agents, SSH, Tabs & splits, Files & Changes, Search) with its recording, a specs row,
  one Compare table with the benchmark numbers, an "Available on" chapter, and a closing
  "Get nex." with the install command. Section bands fade in and out instead of ending in an
  edge, and every recording sits on the same soft glow. The build now fails if the page shows
  anything but real recordings from `demo/`.

### Fixed

- Closing a pane on a `nex-host` host (a tab on the phone, or in a window connected to the
  host) no longer ends an agent someone started by typing `claude` or `codex` at its
  shell's prompt: the agent keeps running and stays in the Agents pane, as one opened from
  the Agents pane already did. A plain shell still ends with its pane.

## [0.14.0] - 2026-11-24

### Added

- Agents pane: the agents on screen say so. Each card whose agent is in the terminal tab
  you're on (or the one you go back to from the Agents tab, where clicking a card took you)
  has a "Viewing" pill with an eye and a soft accent ring: the one in the focused pane fully,
  with a glow, and one sharing its tab (the other half of a split) more faintly. A zoomed tab
  counts only its focused pane. In an open team a viewed helper's row has an accent rule and
  an eye; on a narrow card the pill keeps only its eye.

### Fixed

- A pane showing an agent on a `nex-host` host (reattached with `ssh … nex-host attach`, or
  in a window connected to the host) opens new tabs, the file explorer and Changes in the
  agent's directory there, not the home folder. The Files tab opens on the host
  too, for any pane on one.

## [0.13.0] - 2026-11-24

### Added

- nex-host: a `mockups` example pipeline for design ideas. Three designers take a brief in
  different directions (calm, bold, dense) at the same time, each writing a self-contained
  HTML mockup under `mockups/` (with desktop and phone screenshots when a headless browser
  is installed), and a critic ranks them and may send them back. Copy
  `nex-host/examples/mockups/pipeline.toml` to `~/.config/nex/pipelines/`.
- The Agents pane shows the sub-agents a Claude Code or Codex agent is running (Claude
  Code's Task tool, Codex's spawned threads) as a team in its card, like a pipeline's stages:
  a capsule each, with what it was asked to do, a tick for two minutes once it finishes, and a
  click jumps to its agent's pane. A pipeline stage's sub-agents sit under that stage, with a
  count on its capsule. It works over ssh too: `nex-host list --json` reports each session's
  `subagents`. Codex support reads its rollout files and is untested.
- Agents pane: a helper's row in an open team says which helper it is and what it's doing:
  a tag in a small capsule (`Build-01`: its role or pipeline stage, else its name, and its
  number; a sub-agent's is its type, `Explore-01`) and then the task from its terminal's
  title, which follows along as the agent retitles it. The folder it works in is the quiet
  line under it. The closed card's capsules say the same word (`Build`).
- Agents pane: a pipeline's card says how its run stands and what happened last, e.g.
  `Feature · 2 building · 1 in review · 3 merged` over `✓ build (PR 7) approved: …`. A
  pipeline without an `each` stage says `stage 2 of 3: review`. `nex-host pipe` puts both in
  its terminal's title, so the server needs this version's `nex-host`; with an older one the
  card is as before.
- The phone apps' connect screen (and `nex --remote`'s) shows the nex dog in its card's
  header, waking up as the screen appears. Once connected, the card becomes the first pane
  in one motion: its fields fade, it glides into the pane's place as its title turns into the
  pane's, the sidebar and tabs rise in around it, and the terminal shows through, while the dog
  slides over to the Files tile with a blep (skipped with `animations = false`).

### Changed

- Agents pane: the helpers inside a team or pipeline card are sorted by status, like the
  cards are (needs you, done, working, idle; each in the order they started), and glide to
  their new place when one changes: its row in the open list slides over the others, its
  capsule along the closed card's strip. A pipeline's steps keep their order (build before
  merge); only the agents of one step change places. They jump with `animations = false`.

### Fixed

- Updating nex now updates the nex-host daemon on ssh hosts too, not just the program on
  disk: a daemon started before the update used to keep serving the old code (new commands
  failed with `bad request`) until it happened to exit. Without agents running, the next
  `nex-host` command replaces it at once. While agents run on it, it stays up (they'd end
  with it), the Agents pane shows "nex-host update pending" for that host, and it hands over
  by itself when the last one finishes.
- Ctrl+G in a pane ssh'd into a host opens the Changes pane in the folder the remote shell is
  in, not the remote home, while a program there (Claude Code, vim, tmux) has set its own
  window title. Titles like oh-my-zsh's `user@host:~/src` count too.
- Swiping right on the first tab opens the Agents tab on touch screens, even before it has
  been opened.
- The phone apps and `nex --remote` connect to hosts whose login shell is fish or csh: the
  nex-host check and install scripts run under `sh`. A timed-out or cancelled command now
  ends on the host too.
- Linux (Wayland): pasting no longer freezes the window while the app holding the clipboard
  is slow to answer.
- The pane header's icons share one height and sit centered, and keep their colors while
  tabs glide.

## [0.12.2] - 2026-11-23

### Added

- An iPhone app, like the Android one: the phone runs only nex's window, and every pane is a
  `nex-host` session on an ssh host. `nex-ios/build-ipa.sh` builds a signed ad hoc `.ipa`, or
  an App Store one for TestFlight (see [`nex-ios/README.md`](nex-ios/README.md)).

### Fixed

- The Agents pane lists Claude Code and Codex started by typing `claude` or `codex` in a pane
  on an ssh host (every pane, when nex is connected to the host). `nex-host` now reports what
  a shell session's foreground job is (`running` in `nex-host list --json`), with Claude's
  name and status, so the pane no longer says "No agents running" while they're open.

## [0.12.1] - 2026-11-23

### Added

- `Ctrl+Shift+P` (`Cmd+Shift+P` on macOS) pins the focused pane to every tab, or unpins it,
  the same as Shift + the pane header's ×.

### Changed

- The connect screen (Android, iOS, `nex --remote`) is one card drawn like a focused
  terminal pane: the pane's header with the ssh glyph and the step's title, the fields on the
  card's own fill, and the desktop alert's buttons and recessed panels (Cancel beside "Trust
  and Connect"). It sits in the screen's safe area.

### Fixed

- macOS: closing a pane frees its terminal. Closing a terminal hangs its program up and
  waited for it to exit, but a program that left output unread can't finish exiting on
  macOS until it's read, so the wait never ended: each closed pane kept a pseudo-terminal,
  a thread and a half-exited `login`, and a long session could run out of terminals
  (`failed to spawn shell: Device not configured`). nex now reads what's left while it
  waits.

## [0.12.0] - 2026-11-23

### Added

- Touch screens: the Changes pane (its file list and its diff) and the Agents pane scroll
  with a finger too, flinging on after a quick swipe.
- Touch screens: a finger on a terminal no longer only selects text. Swipe up or down to
  scroll (through the scrollback, or as the wheel in `less`, tmux and other programs), and
  a quick swipe flings on. Swipe sideways on any pane (a terminal, Files, Changes, Agents)
  to go to the next or previous tab; the home tab comes first, and the ends don't wrap.
  Long-press to
  select a word, then drag to extend the selection. A tap focuses the pane (and clicks, in
  programs that take the mouse).
- Android: the Agents pane lists the agents on the server the app is connected to, watched
  over the app's own connection. Opening one attaches a pane to it, and closing that pane
  only detaches (the agent keeps running); stopping one goes over the same connection. A
  server with a `nex-host` from before `connect` gets this one installed.
- Android: the title-bar tabs keep one width, two and a half to the screen, and the strip
  swipes sideways (with a fling) instead of squeezing every tab in. Long-press a tab, then
  drag, to reorder it; a new or newly shown tab scrolls into view.
- Android: hold the keyboard button in a terminal to talk to Claude Code's voice mode. After
  about half a second it turns red with a mic, the phone's microphone is shared with the
  host (over the app's own connection: the shared mic works on Android now), and Space goes
  to the terminal as if held until you let go, and a moment after so the last words get
  through. Android asks for the microphone the first time. Taps, double-taps and drags keep
  their meaning.
- Android: a round glass keyboard button in the bottom-right corner of terminals and the
  Agents tab, above the on-screen keyboard while it's up. Tap it to put the keyboard away or
  bring it back. Putting the keyboard away (the button, the back gesture or its hide key)
  keeps it down, so you can read and scroll a full screen.
  - Drag the button as a joystick for the arrow keys. They repeat while you hold, faster
    the further you push. On the Agents tab it moves the selection.
  - Double-tap it to arm Ctrl for the next key (double-tap, then `c`, is `^C`). It turns
    blue with a ⌃ until the key is pressed; double-tap again to cancel.
    While Ctrl is armed, the focused pane's × is a pin, as with Shift held on a desktop:
    tap it to pin the pane to every tab.
  - Taps, arrows and Ctrl give a light haptic tick.
- Android: swipe (glide) typing on the on-screen keyboard types into the terminal. Words go
  in as you swipe, and the keyboard's corrections replace them.
  - A swiped word gets a space after it, as in other apps, so the next word you swipe
    doesn't run into it. Tapped characters don't. Tapping space after a swiped word
    doesn't add a second one.
- Files and Changes panes on ssh hosts. Ctrl+E or Ctrl+G in a terminal that's ssh'd into a
  host opens the explorer or the Changes pane in the session's folder **on that host**. This
  works for panes running `ssh` and for panes on a host the window is connected to itself
  (`--remote`, the Android app). The explorer browses, renames, creates, copies, moves and
  deletes there, and shows Properties. "Open Terminal Here" opens another session on the host.
  git runs on the host, and the Changes pane follows the focused ssh pane like a local one.
  Files can't yet be copied between the host and this machine or opened with a local app,
  and these panes aren't restored at startup.
- A **Done** section in the Agents pane: an agent that finishes while you're not looking at
  its pane moves there, in blue, instead of going straight to Idle. It becomes Idle once you
  look at its pane. The title bar shows "1 done" too.
- Agents light their tab's attention dot when they finish or start needing input, without
  ringing the bell. This works for agents over ssh with no Claude Code setup on the server.
  While nex is in the background, the dock or taskbar is flagged too.
- Syntax colors in the Changes pane: diffs are highlighted for their language (Rust, TOML,
  Python, TypeScript, shell, Markdown and the rest of bat's grammars), in colors taken from
  your terminal palette. Changed words keep their tint on top. Coloring runs in the
  background, starting with the hunks on screen: most diffs arrive fully colored, and a very
  large one shows with its first screen colored while the rest fills in from wherever you've
  scrolled to. Hunks that haven't changed keep their colors across refreshes.

- Agents on ssh hosts that keep running after their pane closes. nex installs its helper,
  `nex-host`, in `~/.nex` on hosts its panes connect to (no root; one static binary). An agent
  started there survives closing the pane, a dropped connection, or quitting nex: nex offers
  (once per host) to make `claude` and `codex` typed in its ssh panes start that way, with a
  few marked lines in the host's shell config (`ssh-agent-shim`), or start one by hand with
  `~/.nex/bin/nex-host run -- claude`. The Agents pane lists it with its host, folder and live status
  (Detached when no pane shows it). Click it to open a tab attached to it, with its screen
  redrawn where it left off; closing that tab only detaches it again ("Claude Code keeps
  running on devbox"). `Delete` stops one (after asking), and one whose program ends drops
  off the list. Hosts nex can't reach get a quiet line at the top of the pane. Hosts with
  agents are remembered across restarts. `ssh-agents = false` turns it off. See
  `nex-host/README.md`.

- An Agents pane: every Claude Code and Codex agent running in nex at a glance.
  `Ctrl+Shift+A` (`Cmd+Shift+A` on macOS) splits it off the focused pane, and the title bar's
  label (the one with the nex dog) now opens it as a tab of its own, with a live headline
  ("1 needs you") underneath. Each agent is a card with its task, folder and tab, and a status
  pill: Needs you (it rang the bell or is waiting on a prompt), Working (with a breathing dot),
  or Idle, plus how long it has been that way. Cards are grouped by status, two or three to a
  row when there's room. Click a card, or pick one with
  the arrow keys and press `Enter`, to jump to that agent's pane. Claude Code's status comes
  from its own session file; Codex's from its title, bell and output.

- A Changes pane: `Ctrl+G` (`Cmd+G` on macOS) splits a read-only `git diff` viewer off the
  focused pane, and `Ctrl+G` again closes it. Changed files sit in a sidebar (Staged and
  Changes sections, with status badges and line counts, filtered by a segmented control) and
  the selected file's diff shows as rounded hunk cards with line numbers, the enclosing
  function and word-level highlights. It follows the focused pane's repository, refreshes
  while on screen, keeps its zoom, and comes back with the session.
- A sticky pane that follows you to every tab, the Files tab included. Hold Shift over a pane
  header and its × turns into a pin; click it to pin the pane. Each tab keeps its own place and
  size for it (it starts on the side it was pinned from), and on a tab switch it glides to its
  new spot instead of leaving with the old tab. On the pinned pane the × slot is the pin: click
  it to unpin, or hold Shift to get the × back and close it. It stays pinned across restarts.
- nex brings back the last window when it starts: every tab, its splits and their sizes, each
  pane's folder and zoom, which pane and tab were focused, and the Files tab. Terminals start a
  fresh shell in their folder (ssh panes come back as local shells; a folder that is gone opens
  home). Closing every tab starts the next launch fresh. Only a plain `nex` restores, not `-e`,
  a folder, `--working-directory`, `--explorer`, a new window, or a second nex while one is
  running. Turn it off with
  `restore-session = false`.
- A restored pane runs again what it was running when nex quit (`npm run dev`, `ssh host`,
  anything in the foreground), typed into its fresh shell so quitting it leaves you at the
  prompt. Claude Code comes back on the same conversation with `claude --resume`; its prompt
  and `-p` are dropped so the first message isn't sent again. Not on Windows yet.
- nex can answer other apps' "Show in Folder" (Firefox, Chromium, VS Code and anything else that
  calls D-Bus `org.freedesktop.FileManager1`). It opens the file's folder on the Files tab with
  the file selected, showing hidden files if the file is one. Opt in with
  `NEX_FILE_MANAGER=1 ./install.sh` (see the README's "Can nex be my file manager?").
- `--select=PATH` opens the Files tab in PATH's folder with PATH selected.
- A pulsing dot on the top-left corner of a tab's icon when a program in it rings the bell or sends a
  desktop notification (OSC 9, OSC 99 or OSC 777) and you aren't looking at it, such as Claude
  Code waiting for permission. It clears when you switch to that pane. When nex is in the
  background, the dock or taskbar is flagged as well. Claude Code needs
  `preferredNotifChannel` set to `terminal_bell` (or `iterm2`) to send one.
  `tab-attention-color = #rrggbb` sets the dot's color (yellow, palette 3, by default).
- More ways to move a pane by dragging its header. Drop it on the edge of another pane to
  put it on that side, which turns side-by-side panes into stacked ones and back. Dropping
  it on the middle of another pane still swaps them. While you drag, the other panes slide
  to show where everything will land. Drop it on the empty part of the title bar (or the
  mobile sidebar) to give it a tab of its own. Hold it over another tab for half a second
  and that tab opens so you can place it there, or let go on the tab to put it beside that
  tab's focused pane. A tab whose last pane moves away closes, but the pane keeps running.
  Esc cancels and goes back to the tab you started in.

### Changed

- `nex-host pipe` from an agent: the pipeline shows under the agent that asked for it, so
  asking for several features is one card in the Agents pane, the agent with its
  pipeline's stages as steps (`feature: build, build-2 → merge`). Builders of the same
  stage sit side by side, the next stage after a step line. The run outlives the agent
  that asked, as before.
- Agents pane: a closed team card is a cell of the grid, as wide as the other cards, with
  teams on rows of their own ahead of the single cards; opened, it spans the row for its
  list. On a desktop-width pane teams no longer stretch into full-width bars.
- Android: title-bar tabs no longer show a close (×) button, which a tap set off as a hover;
  the title fills the tab. Close a tab from its pane header's × (closing its last pane closes
  the tab).
- Android: switching tabs slides the way the swipe goes: the next tab comes in from the
  right and the old one leaves to the left (the previous tab, the reverse). Desktops and the
  sidebar keep their direction.
- Agents pane: each card and helper row shows the folder its agent works in (`nex`, or a
  worktree's own folder) instead of the host it runs on, so you can see which checkout each
  is in. The cards are calmer too: the title and folder sit together beside the tile, with a
  quieter status pill and a softer shadow.
- The `mobile` setting is now `tab-bar = top | sidebar`: `mobile = true` is `tab-bar = sidebar`.
  `mobile` is still read, so existing configs keep working.

- When the Agents pane can't see a host's agents because it wants a password, nex offers to
  share the ssh connection (once a run), so it reuses the login typed into the pane. If sharing
  is on but that pane logged in before it was, the note says to log out and ssh back in.

- nex's own connections to an ssh host (the Agents watch, installing `nex-host`, uploads,
  the shared mic, stopping an agent) share one ssh connection per host, so only the first one
  logs in; each one after it starts in about one round trip. It is the pane's own shared
  connection where there is one, otherwise one nex keeps up until two minutes after its last
  use. A host whose `nex-host` is up to date is watched in one connection, not two.
  `nex-host` travels gzipped (1.2 MB down to about 540 KB), and uploads of text and source
  folders are gzipped on the way. Screenshots and other already-compressed files go as they are.

- The title bar's directory label opens the Agents tab instead of the Files tab. `nex
  --explorer`, a folder argument and the file-manager service still open Files, and `Ctrl+E`
  still splits an explorer off any pane.
- The file explorer's list / grid toggle moves to `Ctrl+Shift+G` (`Cmd+Shift+G`), since
  `Ctrl+G` now opens Changes. `Ctrl+G` no longer reaches the shell as `^G`.
- Dropping a dragged pane header on the title bar now opens it in a new tab instead of
  cancelling the drag.
- The Files tab's icon is now the nex dog instead of a monitor. It wakes up when nex starts,
  sticks its tongue out with a head wiggle each time you switch to Files, and is dimmed
  while another tab is showing. With `animations = false` it stays still.

### Fixed

- nex-host: a message an agent sends the moment it starts is from that agent. It could
  be taken for one from someone at a shell (`user`), so a merger never heard which builder
  opened which PR, when the agent ran before nex-host had it on its list.
- Android: the pane header's pin (Ctrl armed with the keyboard button) is the same pin as
  desktop's Shift pin: the same glyph, always from the built-in font, at the same size. The
  button stays finger-sized.
- `nex-host pipe`: two features joining a run in the same second no longer end it. Their
  worktrees got the same branch name, the second failed to start, and the run stopped, so
  nothing reviewed or merged the PRs its builders went on to open. Worktrees made in the
  same second now get `-2`, `-3`…, and a joined feature that can't start is reported to
  whoever sent it while the run goes on.
- The feature pipeline example deletes a merged PR's branch on GitHub with `git push
  --delete`; `gh pr merge --delete-branch` stopped at the local branch, which the builder's
  worktree still has checked out.
- Toasts fit narrow screens: a message wider than the window (a phone held upright) wraps
  onto more lines instead of running off the sides, and a very long one stops at a few
  lines, ending in "…".
- Agents pane: clicking a team's or pipeline's dropdown arrow opens and closes its list of
  agents (it used to jump to the lead instead, and an open list couldn't be closed with the
  mouse). A card lifted off the grid also takes clicks where it's drawn.
- Agent teams: a pipeline's `each` stage reviews every report from the stages before it,
  not just each one's first. A re-report after `--verdict changes` and any later PR from a
  build agent get in line too, and one reviewer agent takes them one at a time. A
  reviewer can now have `worktree = true` to check each PR out in a worktree of its own.
- Agent teams: running `nex-host pipe NAME TASK` again from the same repository joins the
  run that's going when it ends in an `each` stage. Before, each call made a run of its
  own, so the Agents pane and `nex-host ls` showed one pipeline group per feature. Now one
  run is one group, with every builder and the one reviewer under it. The run stays up for
  more until you press Enter in it.
- Agent teams: a pipeline stage with `worktree = true` no longer quits on start. Claude Code
  asks whether to trust each new worktree, and nex-host typed the stage's task (or its
  reminder to report) into that question, answering "No, exit". An agent showing one of
  Claude Code's prompts (trusting the folder, a new MCP server, a tool's permission) now
  counts as needing you: messages wait for it, it isn't reminded, and the Agents pane says
  "Needs you" instead of Idle.
- Android: panes come back within seconds after the phone sleeps. Waking checks the app's
  connections and reconnects the ones the sleep killed, instead of waiting up to a minute
  for keepalives to give up. Retries right after a wake come every second instead of backing
  off to ten. A server whose name doesn't resolve yet (a VPN's DNS, such as Tailscale's,
  comes back last) is reached at the address it last had. A pane no longer closes when its
  connection drops at the wrong moment.
- Restoring the session reattaches panes that showed an agent on an ssh host. A tab opened
  from the Agents pane came back as an empty local shell, and an ssh pane running `claude`
  through the shim came back logged in but with the agent left detached.

- A crash in one agent on an ssh host no longer takes down the others. `nex-host` kept going
  only until another agent or pane touched the shared agent list, then stopped too, ending
  every agent on the host.
- Android: Ctrl armed with the keyboard button now works like a held Ctrl for nex's own
  shortcuts. Ctrl then `e` opens the file explorer and Ctrl then `g` the Changes pane, as
  Ctrl+E and Ctrl+G do on a desktop; keys with no shortcut still reach the program as
  control characters.

## [0.11.0] - 2026-11-17

### Added

- Share your microphone with an SSH session (experimental). The mic button in an `ssh`
  pane's header, or `Ctrl+Shift+M` (`Cmd+Shift+M` on macOS), makes this machine's mic the
  host's default microphone, so programs there that record can hear you. That includes
  Claude Code's voice mode. The local mic only records while a program on the host is
  recording, and the button turns red while it does. Nothing is installed on the host. On
  hosts running PipeWire the mic is a source there ("Microphone (shared by nex)"), made the
  default while shared. Other Linux hosts with `libasound2` get a block in `~/.asoundrc`,
  removed when sharing stops. `ssh-mic = auto` shares with every `ssh` pane, and
  `ssh-mic = off` hides the button.

## [0.10.1] - 2026-11-17

### Fixed

- On macOS the menu bar now names the app "Nex". It could show "NSMenuItem" instead, and the
  app menu's About, Hide and Quit entries said "nex".

### Changed

- The README and website now cover the SSH features from 0.10.0 in their own section: splits
  and tabs that follow `ssh` sessions, file and image uploads, and connection sharing.

## [0.10.0] - 2026-11-17

### Added

- Files and images reach SSH sessions. Dropping files onto a pane running `ssh`, or pasting
  a copied image or copied files there (Ctrl+V or Ctrl+Shift+V; Cmd+V on macOS), copies
  them to a new folder under the remote host's `$TMPDIR` (or `/tmp`) and types the remote
  paths, so a program on the other end, such as Claude Code, can open them. The upload runs
  over a second `ssh` with the pane's own options and `BatchMode`, so it can't type a
  password. When a host asks for one, nex offers to turn on connection sharing: a short block
  at the end of `~/.ssh/config`, matching only ssh run inside nex, that makes each session a
  `ControlMaster` the upload can ride on. Reconnect once and later uploads reuse the login
  typed into the pane. Hosts reached with keys or an agent need nothing. If an upload can't
  log in, the local paths are typed instead. Turn it off with `ssh-upload = false`.
- Splits and new tabs follow SSH sessions. Splitting a pane running `ssh`, or opening a new
  tab from it, opens another session on the same host, in the same remote folder. Typing
  `exit` there drops to a local shell, as in the original pane. With connection sharing on,
  the new session reuses the pane's login. When it's off and the host would ask for a password
  again, nex offers to turn it on first. The remote folder comes from the remote shell's
  OSC 7 report (fish sends one; see the README for bash and zsh), or from a `user@host: dir`
  window title such as Debian and Ubuntu's bash sets. Otherwise the session opens in the
  remote home. Turn it off with `ssh-follow = false`.
- nex can be your file manager. `nex --explorer` opens on the Files tab, and a folder argument
  (`nex ~/Downloads`, or a `file://` URI) opens the Files tab in that folder. A new hidden
  `nex-files.desktop` entry handles `inode/directory`, so
  `xdg-mime default nex-files.desktop inode/directory` makes folders open in nex.
- `explorer-view = list | grid` picks the layout file explorers open in.

### Fixed

- Ctrl+V with an image (and no text) on the clipboard now reaches the program as `^V`, so
  Claude Code can paste the image itself. Before, nex swallowed the key.
- An image dragged in from a browser (Firefox, Chromium) onto a pane now drops. nex saves it
  to a temp file and types that path, so Claude Code picks it up as an image. Dropped on an
  explorer, it lands in that folder. Before, the drop did nothing on Wayland: the drag's only
  URI was the image's web address, not a file.
- nex could crash as it quit on Wayland: the drag and drop thread kept reading the
  display's connection while it was being closed. The thread now stops first.

## [0.9.1] - 2026-11-16

### Changed

- The glide tab switch now moves the panes the same way the glass pill moves, so the old tab
  glides out ahead of the pill and the new one follows it in, instead of going against it.

## [0.9.0] - 2026-11-16

### Added

- Switching tabs animates. By default the old tab glides out toward the side it is leaving
  as the new one glides in. `tab-animation = ink` makes the new tab soak through the old one
  like ink instead, from a drop under the tab you picked, and `tab-animation = none` switches
  instantly. `animations = false` still turns every animation off.
- Closing a tab animates. The active tab's glass pill folds into its × and grows back out
  onto the tab now showing; a background tab shrinks into its × and fades. The tabs beside
  it slide over to fill the space.
- Zooming a pane scales its card and text between its slot and the whole tab, with the other
  panes fading out and back in under it. Equalizing splits eases the panes into place.

## [0.8.2] - 2026-11-16

### Changed

- nex uses much less memory at idle. On macOS its resident memory drops from about 480 MB to
  about 60 MB: font lookup now maps font files instead of reading every system font into
  memory.
- egui's glyph atlas is capped at 4096 px wide instead of the GPU's maximum (16384 px), which
  saves a few MB in memory and on the GPU.
- Memory freed during startup and after a font change is given back to the system (on Linux,
  glibc also keeps fewer per-thread heaps).

### Added

- `bench/mem.sh` shows where nex's idle memory goes on Linux: heap, GL driver, the binary
  and fonts.

## [0.8.1] - 2026-11-16

### Fixed

- On macOS, the window buttons (traffic lights) no longer flicker up for a frame when the
  terminal's title changes.
- On macOS, tab tiles and pane header icons follow the program running in the pane again
  (Claude Code, yazi, and the rest). Shells start through `login`, which runs as root, and
  nex couldn't look past it, so every pane kept its shell icon.
- On macOS, Claude Code's native install shows as `claude` rather than its version number.

### Changed

- Refreshed the benchmark results in the README and on the site.

## [0.8.0] - 2026-11-15

### Added

- Swap panes. Drag a pane by its header onto another pane: the card follows the mouse or
  finger, the pane under it slides into the free slot as a preview, and letting go swaps
  them (`Esc`, or letting go elsewhere, puts it back). `Alt+Shift+Arrows`
  (`Cmd+Shift+Option+Arrows` on macOS) swaps the focused pane with its neighbor.
- File explorer, Thunar-style:
  - Back / Forward (`Alt+Left` / `Alt+Right`, the mouse's side buttons), and clickable
    breadcrumbs that are also drop targets. `Ctrl+L`, or a click beside the breadcrumbs, types
    a location, with `Tab` completion.
  - Rename in place (`F2`), new folders (`Ctrl+N`, `Cmd+Shift+N` on macOS) and new empty files.
  - Copies, moves and deletes show a live progress bar in the action bar: bytes done, percent,
    time left and the current file, with a Cancel button.
  - Open With (from the desktop's app list on Linux, installed apps on macOS), Open Terminal
    Here, and a Properties dialog (`Alt+Enter`, `Cmd+I` on macOS) with kind, size or folder
    contents, dates, permissions and owner, where the name can be edited too.
  - Type to jump to a name, and icons by file type (code, images, video, audio, archives,
    documents, executables and more), with marks on symlinks and well-known folders.
  - Dragging files carries the grabbed item's own icon, at the size it has in the listing and
    held where it was grabbed (with a count when several go). On Wayland the icon keeps
    following the pointer after the drag leaves the window, instead of disappearing.

### Changed

- Faster launch. The first shell starts before the window opens, so its startup overlaps
  window and GPU setup, and the system font lookups run in parallel instead of one after
  another. On one Linux machine, the first frame arrives in about 123 ms (was 216 ms).
- On Wayland, dragging files from the explorer into another window (another nex, or a file
  manager such as Thunar or Nautilus) now moves them, as a drag within the window does. Before,
  it always copied and left the original in place. Files dropped into a nex explorer from
  another window move too; hold `Ctrl` to copy instead.
- File explorers watch their folder (inotify on Linux), so a file moved away by another
  window, `mv` in a terminal or a file manager disappears at once, even in a window in the
  background. Elsewhere listings are still re-read every couple of seconds.

### Fixed

- Drag and drop between windows on Hyprland. Hyprland only sends drags and the clipboard to
  the first data device a client makes, which was egui's clipboard, so no drag ever reached
  nex and drags between two nex windows were cancelled. nex now runs eframe on its own event
  loop, makes its data device before the window opens, and reads pasted text from it.
- "Application not responding" on Wayland for nex windows on another workspace. With vsync
  on, drawing a frame waits for the compositor's frame callback, which a hidden window never
  gets, so nex's main thread blocked and missed the compositor's pings. On Wayland vsync is
  now off and nex paces frames to the display's refresh rate itself.

## [0.7.4] - 2026-11-14

### Fixed

- macOS: in full screen, the title bar kept an empty gap where the hidden window buttons
  would be. The workspace label and tabs now start at the left edge.

## [0.7.3] - 2026-11-14

### Fixed

- macOS: the Dock icon switched from the nex collie to the egui logo once nex had started.
  nex now keeps the app's own icon.
- macOS: the nex icon had a white square behind its rounded corners. `nex.icns` is
  re-rendered with a transparent background by the new `packaging/macos/make-icns.sh`.

## [0.7.2] - 2026-11-14

### Fixed

- macOS: the close / minimize / zoom buttons sat above the middle of nex's title bar. They
  are now centered in it, in line with the tabs.

## [0.7.1] - 2026-11-14

### Added

- `nex.app` for macOS, with the nex icon and the name "nex" in the menu bar. Releases include
  it as `nex-<version>-macos-app.zip`, and `install.sh` installs it to `/Applications` (or
  `~/Applications`). Built by `packaging/macos/make-app.sh`; ad-hoc signed, not notarized.

### Fixed

- macOS: opened from Finder or the Dock, the first shell started in `/` instead of your home
  folder, and without `LANG`, so programs fell back to ASCII. nex now starts it at home and
  sets `LANG` from your region (e.g. `en_US.UTF-8`) when it's missing.

## [0.7.0] - 2026-11-14

### Added

- macOS support. Releases include a universal binary (Apple Silicon and Intel) as a tarball
  with `install.sh`; it isn't notarized yet. Shortcuts follow Ghostty's macOS defaults (Cmd+T, Cmd+D,
  Cmd+C / Cmd+V, Cmd+1…9; Ctrl+C always reaches the shell), the window uses the native traffic
  lights, pane headers show the foreground process and folder, and fonts are found without
  fontconfig. Dragging files out to Finder isn't supported yet.

### Changed

- Everything OS-specific now lives in `src/platform/`, one folder per OS behind the same API
  (Wayland drag and drop moved there from `src/wl_dnd/`). The keyboard shortcuts are in
  `src/app/keymap.rs`, with one table per OS. Pane-header tooltips show the shortcuts for the
  current OS.
- `config.example` no longer sets `window-decorations` / `window-controls`, since their
  defaults now depend on the OS.
- `install.sh` works on macOS (binary and config only; the desktop entry and icon are
  Linux-only).

## [0.6.3] - 2026-11-14

### Fixed

- Launching nex from an app launcher (rofi, Hyprland, etc.) did nothing when the session's PATH
  didn't include `~/.local/bin`. `install.sh` now writes the absolute binary path into the
  installed desktop entry's `Exec=` line.

## [0.6.2] - 2026-11-14

### Fixed

- Symbols such as ✢ ✻ ✘ ☒ ⎯ ⧉ (used by Claude Code's spinner, todos and rules) drew as empty
  boxes. The symbol fallback font was looked up as "Noto Sans Symbols 2" instead of
  fontconfig's "Noto Sans Symbols2", so it never loaded. nex now also falls back to DejaVu
  Sans Mono, Noto Sans Symbols and Noto Sans Math when they're installed.

## [0.6.1] - 2026-11-14

### Changed

- New logo and app icon: a border collie head in the accent gradient, replacing the prompt
  icon. It's used for the desktop icon, the README and the website.

## [0.6.0] - 2026-11-14

### Added

- Clipboard toasts: copying (Ctrl+Shift+C or copy-on-select) or pasting (Ctrl+Shift+V) in a
  terminal shows a short "Copied 42 characters" / "Pasted 3 lines" toast at the bottom of the
  window. `clipboard-toasts = false` turns them off.

### Changed

- nex is now "the nex terminal" (was "the nex-gen terminal") in the README, the site and the
  package description. The site's version tag shows 0.6.

## [0.5.0] - 2026-11-14

### Added

- `mobile = true` for phone-sized touch screens: the title bar becomes a sidebar on the
  left, with the window buttons, the Files tab, one large app icon per tab (title
  underneath) and the new-tab button stacked top to bottom. Split tabs fan their icon deck
  out wider, so the shuffle when focus moves is easy to see. Pane headers get taller, with
  finger-sized buttons.
- Drag and drop between windows on Wayland: drag files out of the explorer into another nex
  window (onto a terminal types their paths, onto an explorer copies them) or into another
  app, and drop files from other apps into nex. winit has no Wayland drag and drop, so nex
  speaks the protocol itself (`src/wl_dnd.rs`). On sway a finger's drag stays with the window
  it started in, so nex passes it to the nex window under the finger over a local socket.
  A window that receives files from another window takes keyboard focus, and the window a
  drag left no longer thinks the finger (or button) is still down. `NEX_DEBUG_DND=1` logs each
  step.

### Fixed

- Files dropped where there's no pane (between panes, or outside the window) are ignored.
  They used to go to the focused pane, so a drop on nothing could copy files into the
  explorer's folder.
- Touchscreen drag and drop onto a terminal now drops where the finger lifts. Before, it
  could use the hidden mouse cursor's position (sway moves the pointer there when a finger
  lifts), so the paths went to the wrong terminal or nowhere.

## [0.4.0] - 2026-11-14

### Added

- Command-line flags: `--config=PATH` reads another config file, `--app-id=ID` sets the
  window's Wayland app_id (X11 class), `--working-directory=DIR` starts the first shell there,
  and `-e COMMAND [ARGS...]` runs a program in the first pane instead of the shell. Also
  `--help` and `--version`. A new window (Ctrl+Shift+N) keeps `--config` and `--app-id`.
- Delete files and folders in the explorer: the `Delete` key, "Delete…" in the right-click
  menu, or the Delete button in the action bar (for touchscreens). A confirmation dialog warns
  that items are permanently deleted (not moved to the trash) before anything is removed.
- Touchscreen drag and drop: while selecting, drag a selected item to move the selection
  without another long press. The drag label floats above your finger, and a terminal under
  the drag lights up with a "Drop to insert path" hint. Dropping anywhere on a terminal's card,
  header included, types the paths.

## [0.3.0] - 2026-11-13

### Added

- Pane animations: a split slides the new pane out of the original's edge, a closed pane
  collapses and fades while its neighbor grows into the space, and focus dimming fades instead
  of switching. Terminals are sized for the final layout up front, so the shell never sees a
  resize mid-animation.
- `animations = false` turns off every UI animation (panes, the title-bar tab pill and the
  pane-icon deck). It applies on live reload.

### Changed

- Faster terminal drawing: each pane is now one mesh of glyph quads taken from a per-character
  glyph cache, instead of text laid out every frame. A full screen of changing colored text
  (htop-style redraws) uses about 2.4x less CPU, with identical output.
- Output in background tabs no longer redraws the window, so many busy tabs cost much less.
- Panes whose contents haven't changed reuse their last frame instead of being rebuilt, so
  one busy pane among many idle splits only costs about what that one pane does.
- Lower memory use: font files are memory-mapped instead of read into memory (egui copied
  every font it was handed, so each one was held twice). Idle memory drops by about 55 MB with
  a system Nerd Font and the CJK fallback installed.

### Fixed

- A key press that runs a shortcut (such as `Alt+1`) no longer also sends a stray character
  to the shell.

## [0.2.0] - 2026-11-13

### Added

- A Files tab: the directory label in the title bar is now a tab that browses the current
  folder (click again to go back, middle-click to close). Folders open in place, files open
  in their default app, and the pane splits like any other, starting new terminals in the
  folder you're browsing.
- `Ctrl+E` splits a file explorer off the focused pane, open at that pane's folder.
- File management in the explorer: list and grid views, multi-select (keyboard, Ctrl/Shift+click,
  long-press), copy, cut and paste, a right-click menu and an action bar, and drag and drop
  (into folders, onto terminals as paths, and from other apps).
- Pinch zoom (touchscreen or touchpad) and Ctrl+wheel, for terminal fonts and explorer
  listings.
- `window-controls = false` hides the close, minimize and maximize buttons in the title bar.

### Changed

- The config reloads automatically when you save the file, not only on `Ctrl+Shift+,`.

### Fixed

- Reloading the config now updates `scrollback-limit`, `cursor-style`, `cursor-style-blink` and
  `window-decorations` in the running window, and keeps per-pane zoom unless `font-size`
  changed.

## [0.1.1] - 2026-11-13

### Changed

- The active tab's glass pill now glides to the newly selected tab (200ms, ease-out) instead
  of jumping, whether you click, switch with the keyboard, close or reorder tabs.

## [0.1.0] - 2026-11-13

The first public release of nex, the nex-gen terminal.

### Added

- GPU-rendered terminal built on `alacritty_terminal` and egui.
- Tabs in the title bar, in a segmented-control track with a glass active pill, per-app icons
  and fading titles. Drag tabs to reorder them.
- Split panes as rounded cards with headers, plus split, zoom and close buttons, draggable
  dividers, directional focus and keyboard resizing.
- Smart titles showing the foreground process and its directory, e.g. `Node ~/code/nex`.
- Truecolor and 256-color support, and bold, italic, underline, strikethrough, dim and inverse.
- Procedurally drawn box-drawing, block, braille and Powerline glyphs, so TUI borders line up
  exactly.
- Embedded JetBrains Mono Nerd Font Mono. An installed Nerd Font Mono is used if present.
- Scrollback search (`Ctrl+Shift+F`) with match highlighting.
- Ctrl+click for URLs and OSC 8 hyperlinks.
- Mouse selection (word, line and block), SGR mouse reporting, alternate scroll and bracketed
  paste.
- Ghostty-style config file at `~/.config/nex/config`, with live reload.
- Release builds for x86_64 and aarch64 Linux, as a tarball and a `.deb`.

[Unreleased]: https://github.com/jramirez-codes/nex/compare/v0.14.1...HEAD
[0.14.1]: https://github.com/jramirez-codes/nex/compare/v0.14.0...v0.14.1
[0.14.0]: https://github.com/jramirez-codes/nex/compare/v0.13.0...v0.14.0
[0.13.0]: https://github.com/jramirez-codes/nex/compare/v0.12.2...v0.13.0
[0.12.2]: https://github.com/jramirez-codes/nex/compare/v0.12.1...v0.12.2
[0.12.1]: https://github.com/jramirez-codes/nex/compare/v0.12.0...v0.12.1
[0.12.0]: https://github.com/jramirez-codes/nex/compare/v0.11.0...v0.12.0
[0.11.0]: https://github.com/jramirez-codes/nex/compare/v0.10.1...v0.11.0
[0.10.1]: https://github.com/jramirez-codes/nex/compare/v0.10.0...v0.10.1
[0.10.0]: https://github.com/jramirez-codes/nex/compare/v0.9.1...v0.10.0
[0.9.1]: https://github.com/jramirez-codes/nex/compare/v0.9.0...v0.9.1
[0.9.0]: https://github.com/jramirez-codes/nex/compare/v0.8.2...v0.9.0
[0.8.2]: https://github.com/jramirez-codes/nex/compare/v0.8.1...v0.8.2
[0.8.1]: https://github.com/jramirez-codes/nex/compare/v0.8.0...v0.8.1
[0.8.0]: https://github.com/jramirez-codes/nex/compare/v0.7.4...v0.8.0
[0.7.4]: https://github.com/jramirez-codes/nex/compare/v0.7.3...v0.7.4
[0.7.3]: https://github.com/jramirez-codes/nex/compare/v0.7.2...v0.7.3
[0.7.2]: https://github.com/jramirez-codes/nex/compare/v0.7.1...v0.7.2
[0.7.1]: https://github.com/jramirez-codes/nex/compare/v0.7.0...v0.7.1
[0.7.0]: https://github.com/jramirez-codes/nex/compare/v0.6.3...v0.7.0
[0.6.3]: https://github.com/jramirez-codes/nex/compare/v0.6.2...v0.6.3
[0.6.2]: https://github.com/jramirez-codes/nex/compare/v0.6.1...v0.6.2
[0.6.1]: https://github.com/jramirez-codes/nex/compare/v0.6.0...v0.6.1
[0.6.0]: https://github.com/jramirez-codes/nex/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/jramirez-codes/nex/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/jramirez-codes/nex/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/jramirez-codes/nex/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/jramirez-codes/nex/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/jramirez-codes/nex/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/jramirez-codes/nex/releases/tag/v0.1.0
