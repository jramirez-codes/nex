#!/usr/bin/env python3
"""Write a deterministic benchmark file: plain text, 16/256/truecolor SGR runs and
box-drawing/braille glyphs, mixed line by line. Usage: gen_data.py OUT SIZE_MB"""
import random
import sys

out, size_mb = sys.argv[1], int(sys.argv[2])
rng = random.Random(42)
words = "the quick brown fox jumps over lazy dog terminal render glyph pane split".split()
glyphs = "─│┌┐└┘├┤┬┴┼═║╔╗╚╝█▀▄░▒▓⣿⡇⠿⢸"


def line(i: int) -> str:
    kind = i % 4
    if kind == 0:
        return " ".join(rng.choice(words) for _ in range(14))
    if kind == 1:
        return "".join(f"\x1b[{30 + rng.randrange(8)}m{rng.choice(words)} " for _ in range(10)) + "\x1b[0m"
    if kind == 2:
        parts = []
        for _ in range(8):
            r, g, b = (rng.randrange(256) for _ in range(3))
            parts.append(f"\x1b[38;2;{r};{g};{b}m\x1b[48;5;{rng.randrange(256)}m{rng.choice(words)}")
        return " ".join(parts) + "\x1b[0m"
    return "".join(rng.choice(glyphs) for _ in range(80))


target = size_mb * 1024 * 1024
written, i = 0, 0
with open(out, "w", encoding="utf-8") as f:
    while written < target:
        s = line(i) + "\n"
        f.write(s)
        written += len(s.encode())
        i += 1
