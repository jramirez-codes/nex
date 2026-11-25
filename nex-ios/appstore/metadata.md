# App Store listing: nex (iPhone)

The text to enter in App Store Connect for the iPhone app. Character limits are Apple's; the
counts are for the text as written here.

| Field | Value |
|---|---|
| Name (30) | Nex: AI Coding Agents & SSH |
| Subtitle (30) | Remote terminal, files & git |
| Bundle ID | io.github.jramirez-codes.nex |
| SKU | nex-ios |
| Primary category | Developer Tools |
| Secondary category | Productivity |
| Price | Free |
| Age rating | 4+ (every questionnaire answer "None"/"No"; see notes below) |
| Copyright | 2026 Jordan Ramirez |
| Marketing URL | https://jramirez-codes.github.io/nex/ |
| Support URL | https://github.com/jramirez-codes/nex/issues |
| Privacy policy URL | https://jramirez-codes.github.io/nex/privacy/ |
| App Privacy | Data Not Collected |

## Promotional text (170)

SSH into any server from your iPhone and run Claude Code, Codex or any coding agent. Panes survive dropped connections, and you can hold to talk to your agent.

## Keywords (100)

claude,codex,code,shell,client,server,linux,devops,cli,tmux,vps,homelab,bash,console,sysadmin,zsh

Search only indexes the name, subtitle and keywords, so no word appears in more than one of
them ("nex", "ai", "coding", "agents", "ssh", "remote", "terminal", "files", "git" are covered by
the name and subtitle). Single words, no spaces: "claude" + "code" also match "claude code".

## Description (4000)

Your servers and your AI coding agents, in your pocket. Nex is a fast SSH client for iPhone, built for running Claude Code, Codex and any other coding agent on a remote server, and for checking on them from anywhere.

Every pane runs on an SSH host you choose, like a home server, a VPS or a cloud machine. Close the app, lose signal, switch networks: your shell and your agents keep running, and the screen comes right back when you return.

CONNECT ONCE
Enter the host, user and password a single time. Nex installs a key for this phone on the server, set to expire after 30 days, and logs in with it from then on. Your password is never saved.

PANES THAT SURVIVE
Each pane's shell lives on the server, not the phone. Dropped connections reconnect on their own and pick up exactly where you left off, with no tmux setup needed.

ANY AGENT, ON YOUR SERVER
Run Claude Code, Codex or any CLI agent in a pane and leave it working. The Agents pane gathers your Claude Code and Codex sessions in one place, flags the ones waiting on you, and takes you straight to them.

HOLD TO TALK
Hold the keyboard button to speak. Your phone's microphone is shared with the server while you hold it, so voice input in your agent (like Claude Code's voice mode) works from your phone.

A KEYBOARD MADE FOR THE COMMAND LINE
• Drag the keyboard button like a joystick for arrow keys that speed up the further you push
• Double-tap it to arm Ctrl for the next key (double-tap, then c, sends Ctrl-C)
• Light haptic ticks for taps, arrows and Ctrl
• Swipe typing works in the shell, corrections included

TABS, FILES AND GIT CHANGES
Scroll the tab strip, long-press to reorder tabs, and swipe a pane sideways to switch. Browse the server's files and review git changes without leaving the app.

FREE AND OPEN SOURCE
Nex is MIT licensed, with no accounts, ads, analytics or tracking. The same GPU-accelerated terminal runs on macOS, Linux, Windows and Android: jramirez-codes.github.io/nex

You'll need a Linux or Mac server you can log in to over SSH. Nex installs its small helper, nex-host, in your account there; no root needed.

## What's new in 0.14.0

First App Store release.

## App Review information

- **Sign-in required:** yes, an ssh login to a server.
- **Demo account:** a throwaway account on a demo VPS, kept up for the whole review (and
  every later review). Enter it in App Store Connect only, never in this file:
  - Host: TODO (put the host in the Notes too, since App Store Connect's sign-in fields are
    only user name and password)
  - User name: TODO
  - Password: TODO
  - The VPS needs: OpenSSH with password login allowed for this user, a non-root user with
    no sudo, bash, git, and `~/demo` holding a small git repo with a few commits. Running
    agents isn't needed. nex installs `nex-host` in `~/.nex` on first connect.
- **Notes:**
  > nex is a terminal and ssh client. Tap Connect, enter host TODO with the user name and
  > password above, and trust the host key. The app opens a shell on that server. Try: `ls`, `git log`
  > in ~/demo, swipe a pane sideways to switch tabs, open the Agents and Files panes from the
  > menu. Holding the round keyboard button asks for the microphone, which nex shares with
  > the server for voice input; the app never records or stores audio.

## Export compliance

nex encrypts its ssh connections itself, with standard algorithms (russh with ring), rather
than only through Apple's APIs. Answer App Store Connect's encryption questions from that.
`ITSAppUsesNonExemptEncryption` in `nex-ios/xcode/Info.plist` is currently `false`; it must
agree with the answer, so change it if App Store Connect says otherwise.

## Age rating notes

Unrestricted web access: No (nex has no web browser). User-generated content: No. Every
other category: None.
