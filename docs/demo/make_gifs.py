#!/usr/bin/env python3
"""Cut the take record.py made into one clip per marker pair: a GIF (for the README) plus an
MP4 and a JPEG poster (for the website, where they are a fraction of the GIF's size).

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
# GIF frame delays are whole hundredths of a second: 3 cs (33 fps) is smooth enough for the
# 0.18 s pane animations without the size of 2 cs.
FPS = "100/3"
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


def make_mp4(take: Path, out: Path, start: float, end: float, width: int) -> None:
    """H.264 for every browser; faststart so playback begins before the download ends."""
    subprocess.run(["ffmpeg", "-loglevel", "error", "-y", "-ss", f"{start:.3f}", "-to", f"{end:.3f}",
                    "-i", str(take), "-an", "-vf", f"fps={FPS},scale={width}:-2:flags=lanczos",
                    "-c:v", "libx264", "-crf", "24", "-preset", "slow", "-pix_fmt", "yuv420p",
                    "-movflags", "+faststart", str(out)], check=True)


def make_poster(take: Path, out: Path, start: float, end: float, width: int) -> None:
    """A frame 60% of the way in, once the clip has something on screen."""
    at = start + (end - start) * 0.6
    subprocess.run(["ffmpeg", "-loglevel", "error", "-y", "-ss", f"{at:.3f}", "-i", str(take),
                    "-vframes", "1", "-vf", f"scale={width}:-2:flags=lanczos", "-q:v", "4", str(out)],
                   check=True)


def main() -> None:
    take_dir = Path(sys.argv[1]) if len(sys.argv) > 1 else REPO / "target" / "demo"
    out_dir = Path(sys.argv[2]) if len(sys.argv) > 2 else REPO / "site" / "public" / "demo"
    out_dir.mkdir(parents=True, exist_ok=True)
    marks = json.loads((take_dir / "timeline.json").read_text())
    for name, (start, end) in clips(marks).items():
        take, width = take_dir / "take.mkv", WIDTH.get(name, DEFAULT_WIDTH)
        for ext, make in (("gif", make_gif), ("mp4", make_mp4), ("jpg", make_poster)):
            out = out_dir / f"{name}.{ext}"
            make(take, out, start, end, width)
            print(f"{out}  {end - start:5.1f}s  {out.stat().st_size / 1e6:5.1f} MB")


if __name__ == "__main__":
    main()
