# nex-host

The small helper nex runs on your ssh hosts so coding agents (Claude Code, Codex, or any
other program) **keep running after the pane they were started in closes**. Close the pane,
lose your wifi, or quit nex, and the agent keeps working on the server. Open it again and the
screen is redrawn where it left off.

It runs as your user. It needs no root, no system packages, and no config. Everything lives in
`~/.nex`:

```text
~/.nex/
  bin/nex-host     the program (nex installs it here; it's one static binary)
  run/             0700: only you can reach the daemon
    sock           the daemon's socket
    lock           held by the running daemon
    daemon.log     errors, if any
```

To remove it, run `rm -rf ~/.nex`.

## How it works

```text
nex (local)                                   server
───────────                                   ──────
Agents pane ◄── nex-host watch ◄───────────── nex-host daemon ── pty ── claude / codex
 "New agent" ── nex-host launch ────────────► │  (one per user, started on demand,
 click a card ─► ssh -t host nex-host attach ─► │   in its own session so ssh hanging
                 (the pane is the agent's       │   up doesn't reach it)
                  terminal)
```

- **daemon:** owns every agent's pty and process. It feeds the output into a terminal emulator
  (`alacritty_terminal`, the same one nex uses), so it always knows the agent's screen,
  scrollback, title, bell and modes. It exits about 30 s after its last agent has ended and
  nobody is connected.
- **attach:** turns the pane into the agent's terminal. It sends a redraw first (scrollback,
  screen, colors, cursor, and modes such as bracketed paste, mouse and the kitty keyboard),
  then the live output. Keys and resizes go back to the agent. Closing the pane only ends the
  attach. If you attach from a second pane, it takes the agent over from the first one.
- **watch:** streams every agent as JSON lines whenever something changes (and every 15 s
  regardless). For Claude Code it includes the status from its own session file
  (`~/.claude/sessions/<pid>.json`). The format is defined in [`src/proto.rs`](src/proto.rs),
  which nex shares.

Agents started with `launch` run under your login shell (`$SHELL -l -c 'exec …'`), so they get
the `PATH` you'd have after logging in. Agents started with `run` get the environment of the
shell `run` was typed in.

## Using it by hand

```sh
nex-host run -- claude            # start an agent and show it here
nex-host launch --cwd ~/code/app -- codex   # start one in the background; prints its id
nex-host list                     # what's running
nex-host attach 3fa91c0e          # show it here; closing the terminal leaves it running
nex-host kill 3fa91c0e            # end it
nex-host forget 3fa91c0e          # drop an exited one from the list
```

## The shim

`nex-host shim install` adds a few marked lines to `~/.bashrc` and `~/.zshrc` (and, for fish,
`~/.config/fish/conf.d/nex-host.fish`). They make `claude` and `codex` run as
`nex-host run -- claude …`, but only when:

- the terminal is nex (`LC_TERMINAL=nex`, which nex sets and ssh passes on where the server
  accepts `LC_*`),
- the shell isn't already inside a nex-host session (`NEX_HOST` is set there), and
- `~/.nex/bin/nex-host` still exists.

`run` passes the shell's environment along, so the agent sees the same `PATH`, API keys and
other variables it would without the shim. `command claude` skips the shim. `nex-host shim
remove` takes it out, and `nex-host shim status` prints `on` or `off`. nex offers to install it
(see `ssh-agent-shim` in nex's config).

## Building

```sh
cargo build -p nex-host --release                                      # this machine
cargo build -p nex-host --release --target x86_64-unknown-linux-musl   # a static Linux binary
```

Its version is kept equal to nex's. nex reinstalls the helper on a host whenever the two
differ.

Releases build it for `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`,
`aarch64-apple-darwin` and `x86_64-apple-darwin`, and every nex package carries all four as
`nex-host-<target>`. They go in `share/nex/host/` (tarball and .deb) or
`nex.app/Contents/Resources/host/`. nex picks the one matching the host's `uname -sm`, and looks
in `$NEX_HOST_DIR` first. To install nex from source with them, build each into
`target/host/nex-host-<target>` before running `./install.sh`.

## Limits

- Linux and macOS hosts only.
- On hosts where systemd-logind has `KillUserProcesses=yes`, logging out ends everything you
  started, the daemon included. Run `loginctl enable-linger` once to keep it alive.
- Agents end when the server reboots.
