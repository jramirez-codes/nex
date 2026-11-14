# Benchmarks

`bench/run.sh` measures nex against the other terminals installed on the machine and writes
the medians to [`site/src/data/bench.json`](../site/src/data/bench.json), which the website's
charts and highlight numbers are built from. Nothing on the site is typed in by hand.

```sh
bench/run.sh        # 3 runs per terminal (default)
bench/run.sh 10     # steadier medians, slower
bench/run.sh 3 nex  # re-measure only nex, keep the other terminals' results
```

It opens and closes real windows, so leave the desktop alone while it runs. Terminals are
picked up automatically if installed: nex (built in release mode), Alacritty, kitty, Ghostty and
GNOME Terminal. Each runs with its default settings; nex gets a throwaway config so your own
`~/.config/nex` is not used.

## What is measured

Each terminal runs [`inner.sh`](inner.sh) as its command:

| Metric | How |
| --- | --- |
| **Output throughput** | `cat` of a 50 MB file made by [`gen_data.py`](gen_data.py) (plain text, 16-color and 24-bit SGR runs, box-drawing and braille glyphs; seeded, so identical every run), timed inside the terminal. `cat` only finishes once the terminal has read everything off the PTY. |
| **Launch to shell** | From just before the terminal is started to the moment its shell writes a timestamp. |
| **Memory at idle** | `VmRSS` of the terminal process (for GNOME Terminal, `gnome-terminal-server`) 2 s after its shell started. Child shells are not counted. |

The median of all runs is reported for each metric.

## Caveats

- One machine, one compositor (the site lists the CPU, GPU, kernel and versions used).
  Results on yours will differ; run it and see.
- Default font sizes and window sizes differ between terminals, which affects throughput.
- GNOME Terminal's server process may already be running and shared with other windows,
  which changes its launch and memory numbers.
