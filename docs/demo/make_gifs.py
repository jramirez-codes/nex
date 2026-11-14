#!/usr/bin/env python3
"""Cut the take record.py made into one GIF per clip, for the website.

Each clip spans its start/end markers in timeline.json. GIFs get a palette of their own
(ffmpeg palettegen/paletteuse), so terminal colors stay true; no dithering, since a terminal
is flat colors and dither noise only blurs text and bloats the file. No transparent color
either: browsers would show the page through it.
Usage: docs/demo/make_gifs.py [TAKE_DIR] [OUT_DIR]
       (defaults: target/demo, site/public/demo)
"""

import json
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
FPS = 20
# Output width per clip: the hero shows up to 1000px wide, feature rows about half a page;
# a little extra keeps text crisp on HiDPI screens.
WIDTH = {"tour": 1200}
DEFAULT_WIDTH = 1000


def clips(marks: list[dict]) -> dict[str, tuple[float, float]]:
    spans: dict[str, list[float]] = {}
    for m in marks:
        spans.setdefault(m["clip"], [0.0, 0.0])[0 if m["event"] == "start" else 1] = m["t"]
    return {k: (a, b) for k, (a, b) in spans.items() if b > a}


def make_gif(take: Path, out: Path, start: float, end: float, width: int) -> None:
    scale = f"fps={FPS},scale={width}:-1:flags=lanczos"
    graph = (f"[0:v]{scale},split[a][b];[a]palettegen=max_colors=256:reserve_transparent=0:stats_mode=diff[p];"
             "[b][p]paletteuse=dither=none:diff_mode=rectangle")
    subprocess.run(["ffmpeg", "-loglevel", "error", "-y", "-ss", f"{start:.3f}", "-to", f"{end:.3f}",
                    "-i", str(take), "-filter_complex", graph, "-loop", "0", str(out)], check=True)


def main() -> None:
    take_dir = Path(sys.argv[1]) if len(sys.argv) > 1 else REPO / "target" / "demo"
    out_dir = Path(sys.argv[2]) if len(sys.argv) > 2 else REPO / "site" / "public" / "demo"
    out_dir.mkdir(parents=True, exist_ok=True)
    marks = json.loads((take_dir / "timeline.json").read_text())
    for name, (start, end) in clips(marks).items():
        out = out_dir / f"{name}.gif"
        make_gif(take_dir / "take.mkv", out, start, end, WIDTH.get(name, DEFAULT_WIDTH))
        print(f"{out}  {end - start:5.1f}s  {out.stat().st_size / 1e6:5.1f} MB")


if __name__ == "__main__":
    main()
