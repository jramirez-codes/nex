# App Store listing: nex terminal (iPhone)

The text to enter in App Store Connect for the iPhone app. Character limits are Apple's; the
counts are for the text as written here.

| Field | Value |
|---|---|
| Name (30) | nex terminal |
| Subtitle (30) | SSH, AI agents, files & git |
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

Your server's terminal, in your pocket. Run any coding agent in panes that survive dropped
connections, keep an eye on every one, and hold to talk.

## Keywords (100)

agent,claude code,codex

## Description (4000)

nex is a fast GPU terminal for your servers, with AI coding agents, files and git built in.
The iPhone app is nex's window on your phone: every pane runs on an ssh host you choose, like
a home server or a cloud machine, and nothing runs on the phone itself.

CONNECT ONCE
Enter the host, user and password a single time. nex installs a key for this phone on the
host, set to expire after 30 days, and logs in with it from then on. The password is never
saved.

PANES THAT SURVIVE
Each pane's shell lives on the host. Switch apps, lose signal, change networks: when you
come back, the pane reconnects and your screen is exactly where you left it.

ANY AGENT, ON YOUR SERVER
nex works with whatever agent CLI you use: Claude Code, Codex, or anything else that runs in
a terminal. Start it in a pane, close the app, and it keeps running on your host. The Agents
pane gathers your Claude Code and Codex sessions in one place, flags the ones waiting on you,
and takes you straight to them.

HOLD TO TALK
Hold the keyboard button to speak. The phone's microphone is shared with your host while
you hold it, so voice input in your agent (like Claude Code's voice mode) works from your
phone.

A KEYBOARD MADE FOR TERMINALS
• Drag the keyboard button like a joystick for arrow keys that speed up the further you push
• Double-tap it to arm Ctrl for the next key (double-tap, then c, sends Ctrl-C)
• Light haptic ticks for taps, arrows and Ctrl
• Swipe typing works, corrections included

TABS, FILES AND CHANGES
Scroll the tab strip, long-press to reorder tabs, and swipe a pane sideways to change tabs.
Browse the server's files and review git changes without leaving the app.

FREE AND OPEN SOURCE
nex is MIT licensed and has no accounts, ads, analytics or tracking. The same terminal runs
on macOS, Linux, Windows and Android: get it at jramirez-codes.github.io/nex.

You need an ssh server you can log in to. nex installs its small helper, nex-host, in your
account there.

## What's new in 0.14.0

First App Store release.

## App Review information

- **Sign-in required:** yes, an ssh login to a server.
- **Demo account:** TODO, choose one:
  - (A, recommended) a disposable demo host: `host`, `user`, `password` for a throwaway
    account on a small VPS with a sample git repo, kept up during review.
  - (B) no account: a note that the app is an ssh client for the reviewer's own server, plus a
    link to a screen recording of it working.
- **Notes:**
  > nex is a terminal and ssh client. Tap Connect, enter the demo host, user and password
  > above, and trust the host key. The app opens a shell on that server. Try: `ls`, `git log`
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
