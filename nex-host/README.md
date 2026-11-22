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

## Agents working together

Every agent nex-host starts knows who it is (`NEX_AGENT_ID`, `NEX_AGENT_NAME`) and can run
`nex-host` (its folder is put first on the agent's `PATH`). The daemon is their post office.
Everything they say to each other is a message: a task, a question (`ask`), an answer
(`reply`), a report (`done`) or a `note`. Each message goes in the recipient's mailbox
(`~/.nex/run/mail/`) and is typed into its input straight away, as a bracketed paste and
then Enter.

```sh
nex-host spawn reviewer --kind codex "Review the diff against PLAN.md. done --verdict approved|changes."
nex-host send reviewer --ask "Did you check the tests?"
nex-host done "Renamed foo to bar in 3 files" --verdict approved --files src/a.rs   # inside an agent
nex-host recv --from reviewer --kind done        # from a script or a shell
nex-host read reviewer --lines 40                # its screen, as text
nex-host ls                                      # a tree: each agent under the one that spawned it
```

A message reaches the agent as one XML block that makes sense on its own:

```text
<nex-message from="reviewer (b7c1d2e3)" id="m12" kind="done" verdict="changes" reply-to="m9">
<re>m9 (you → reviewer): Review the diff against PLAN.md</re>
Two issues: …
</nex-message>
```

- Its `kind` says what's expected.
- `verdict` and `files` are fields, not wording conventions.
- Long text is cut short. The whole of it goes in a file named in `<more path=…/>`.
- `task` and `ask` messages end with the exact command to answer with.
- It says it comes from an agent, so it's never mistaken for the user's say-so.

Delivery rules:

- An agent asking the user something (a permission prompt) doesn't get messages typed in.
  They wait in its mailbox until it's free (`--force` overrides).
- A message that a waiting `recv` matches goes to the `recv` instead of being typed in.
- Helpers end when the agent that spawned them does.
- An agent that exits without reporting sends a `done` with `verdict="failed"` on its way
  out.
- One that goes quiet for 45 s after working, without reporting, is reminded once.
- Claude Code spawned this way may run `nex-host` commands without asking:
  `--allowedTools "Bash(nex-host:*)"`.

`nex-host skill` prints the manual for agents. `nex-host skill install` makes Claude Code
(`~/.claude/skills/nex/SKILL.md`) and Codex (a marked block in `~/.codex/AGENTS.md`) find it
on their own, and `skill remove` takes them out.

### Pipelines

A pipeline is a list of stages in `.nex/pipelines/NAME.toml` (or
`~/.config/nex/pipelines/`). Each stage is an agent given a task, and its `done` report
feeds the stages after it:

```toml
worktree = true                       # one git worktree for the run
[[stage]]
name = "plan"
task = "Plan this: {task}. Write PLAN.md."
[[stage]]
name = "build"
task = "Implement PLAN.md. The plan in short: {plan}"
[[stage]]
name = "tests"
parallel = true                       # alongside the stage before it
task = "Write tests for what PLAN.md describes."
[[stage]]
name = "review"
kind = "codex"
task = "Review the diff. Finish with done --verdict approved|changes."
retry = { stage = "build", unless = "approved", max = 3 }
```

- `nex-host pipe feature "add --json to ls"` runs it in a session of its own and shows it.
  Add `--bg` to only start it. `nex-host pipe ls` lists the pipelines it can find.
- The runner uses the same `spawn`, `send` and `recv` as any agent.
- When `retry` sends the run back, agents still running get the new task as a message, so
  they keep their context.

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
