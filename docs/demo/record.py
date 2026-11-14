#!/usr/bin/env python3
"""Record a scripted session of the real nex app, for the website's GIFs.

nex runs on a throwaway headless Hyprland output, so your own screen and windows are left
alone. Keystrokes go in through `hyprctl dispatch sendshortcut`, frames come out of `grim`
and are piped into ffmpeg at a steady frame rate. The scenes themselves live in scenes.py;
each one leaves a marker in the timeline so make_gifs.sh can cut the take into clips.

Needs Hyprland, grim, ffmpeg, plus the programs the scenes run (git, btop, nvim, tree).
Usage: docs/demo/record.py [OUT_DIR]   (default: target/demo)
       DEMO_SCENES=search,explorer docs/demo/record.py   records only those scenes
Then:  docs/demo/make_gifs.sh
"""

import json
import os
import random
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
OUTPUT = "NEXREC"
W, H = 1440, 900  # window size, in pixels: big enough for btop in half the window
FPS = 20

# Characters that aren't their own key name, as (shift, key).
CHAR_KEYS = {
    " ": (False, "space"), "-": (False, "minus"), "_": (True, "minus"), ".": (False, "period"),
    "/": (False, "slash"), "=": (False, "equal"), ",": (False, "comma"), ":": (True, "semicolon"),
    ";": (False, "semicolon"), "~": (True, "grave"), "'": (False, "apostrophe"),
    '"': (True, "apostrophe"), "|": (True, "backslash"), "*": (True, "8"), ">": (True, "period"),
    "\n": (False, "Return"),
}


def hypr(*args: str) -> str:
    return subprocess.run(["hyprctl", *args], check=True, capture_output=True, text=True).stdout


def hypr_json(*args: str):
    return json.loads(hypr("-j", *args))


class Recorder:
    """Grabs the window as fast as grim allows and feeds ffmpeg a constant FPS stream,
    repeating the newest frame when a grab is slower than one frame."""

    def __init__(self, geometry: str, out: Path):
        self.geometry = geometry
        self.latest: bytes | None = None
        self.stop = threading.Event()
        self.ffmpeg = subprocess.Popen(
            ["ffmpeg", "-loglevel", "error", "-y", "-f", "image2pipe", "-c:v", "ppm",
             "-framerate", str(FPS), "-i", "-", "-c:v", "libx264", "-preset", "veryfast",
             "-crf", "8", "-pix_fmt", "yuv444p", str(out)],
            stdin=subprocess.PIPE,
        )
        self.threads = [threading.Thread(target=self._grab), threading.Thread(target=self._emit)]
        self.t0 = 0.0

    def start(self) -> None:
        self.latest = self._shot()
        self.t0 = time.monotonic()
        for t in self.threads:
            t.start()

    def now(self) -> float:
        """Seconds since the first frame."""
        return time.monotonic() - self.t0

    def finish(self) -> None:
        self.stop.set()
        for t in self.threads:
            t.join()
        self.ffmpeg.stdin.close()
        self.ffmpeg.wait()

    def _shot(self) -> bytes:
        return subprocess.run(["grim", "-t", "ppm", "-g", self.geometry, "-"],
                              check=True, capture_output=True).stdout

    def _grab(self) -> None:
        while not self.stop.is_set():
            self.latest = self._shot()

    def _emit(self) -> None:
        n = 0
        while not self.stop.is_set():
            due = self.t0 + n / FPS
            delay = due - time.monotonic()
            if delay > 0:
                time.sleep(delay)
            self.ffmpeg.stdin.write(self.latest)
            n += 1


class Driver:
    """What scenes use to drive nex: keys, typing, waiting and timeline markers."""

    def __init__(self, window: str, recorder: Recorder):
        self.window = window
        self.rec = recorder
        self.marks: list[dict] = []
        self.extra_panes = 0  # panes a scene opened beyond the first, for its cleanup

    def key(self, key: str, mods: str = "", after: float = 0.25) -> None:
        hypr("dispatch", "sendshortcut", f"{mods},{key},{self.window}")
        time.sleep(after)

    def type(self, text: str, enter: bool = True, after: float = 0.4) -> None:
        """Type like a person: a little uneven, slower on spaces."""
        for ch in text:
            if ch in CHAR_KEYS:
                shift, key = CHAR_KEYS[ch]
            else:
                shift, key = ch.isupper(), ch.lower()
            self.key(key, "SHIFT" if shift else "", after=0)
            time.sleep(random.uniform(0.04, 0.09) + (0.06 if ch == " " else 0))
        if enter:
            time.sleep(0.25)
            self.key("Return", after=0)
        time.sleep(after)

    def wait(self, seconds: float) -> None:
        time.sleep(seconds)

    def mark(self, name: str, event: str) -> None:
        """`event` is "start" or "end" of the clip called `name`."""
        self.marks.append({"clip": name, "event": event, "t": round(self.rec.now(), 3)})
        print(f"  {self.rec.now():6.2f}s  {event:5} {name}", flush=True)


def write_config(tmp: Path) -> None:
    """A throwaway nex config (yours in ~/.config/nex is left alone) and a bash with a tidy
    prompt, starting in the repo."""
    (tmp / "nex").mkdir()
    (tmp / "nex" / "config").write_text(
        "font-size = 14\n"
        "adjust-cell-height = 12%\n"
        "background-opacity = 1.0\n"
        "window-decorations = false\n"
        "cursor-style-blink = false\n"
        f"command = bash --rcfile {tmp}/rc\n"
    )
    (tmp / "rc").write_text(
        # Only the first shell (started from your home) moves to the repo; later ones keep the
        # folder nex opened them in, e.g. a shell split off a file explorer.
        f'[ "$PWD" = "$HOME" ] && cd "{REPO}"\n'
        "export HISTFILE=/dev/null\n"
        r"PS1='\[\e[1;35m\]nex\[\e[0m\] \[\e[2m\]on\[\e[0m\] \[\e[36m\] $(git branch --show-current)"
        r"\[\e[0m\] \[\e[1;32m\]❯\[\e[0m\] '" "\n"
        "clear\n"
    )


def add_output() -> tuple[int, int, int]:
    """Create the headless output to the right of the real monitors (so they keep their
    positions). Returns (workspace, x, y)."""
    hypr("output", "create", "headless", OUTPUT)
    hypr("keyword", "monitor", f"{OUTPUT},1920x1080@60,auto-right,1")
    time.sleep(1.0)
    for m in hypr_json("monitors"):
        if m["name"] == OUTPUT:
            return m["activeWorkspace"]["id"], m["x"], m["y"]
    raise SystemExit("headless output never appeared")


def find_window(marker: str) -> dict | None:
    """The client whose process was started with our config dir in its environment."""
    for c in hypr_json("clients"):
        try:
            env = Path(f"/proc/{c['pid']}/environ").read_bytes()
        except OSError:
            continue
        if marker.encode() in env:
            return c
    return None


def main() -> None:
    from scenes import SCENES

    out_dir = Path(sys.argv[1] if len(sys.argv) > 1 else REPO / "target" / "demo").resolve()
    out_dir.mkdir(parents=True, exist_ok=True)
    subprocess.run(["cargo", "build", "--release", "-q", "--manifest-path", str(REPO / "Cargo.toml")],
                   check=True)

    tmp = Path(tempfile.mkdtemp(prefix="nex-demo-"))
    write_config(tmp)
    ws, _, _ = add_output()
    # Sending keys focuses the window, which would warp your pointer onto the hidden output
    # (and into the frames). Keep it where it is until we are done.
    warps = hypr_json("getoption", "cursor:no_warps")["int"]
    hypr("keyword", "cursor:no_warps", "true")
    try:
        x, y = (1920 - W) // 2, (1080 - H) // 2  # window rules place relative to the monitor
        rules = f"workspace {ws} silent; float; size {W} {H}; move {x} {y}"
        cmd = f"env XDG_CONFIG_HOME={tmp} {REPO}/target/release/nex"
        hypr("dispatch", "exec", f"[{rules}] {cmd}")
        client = None
        for _ in range(50):
            client = find_window(f"XDG_CONFIG_HOME={tmp}")
            if client:
                break
            time.sleep(0.2)
        if not client:
            raise SystemExit("nex window never appeared")
        window = f"address:{client['address']}"
        # Opaque and unblurred, so nothing behind the window (your wallpaper) bleeds into the
        # frames. Property names differ across Hyprland versions; unknown ones are ignored.
        for prop in ("opaque", "no_blur", "noblur"):
            subprocess.run(["hyprctl", "dispatch", "setprop", window, prop, "1"], capture_output=True)
        time.sleep(1.5)  # first shell and prompt
        client = find_window(f"XDG_CONFIG_HOME={tmp}")
        cx, cy = client["at"]
        geometry = f"{cx},{cy} {W}x{H}"

        rec = Recorder(geometry, out_dir / "take.mkv")
        drv = Driver(window, rec)
        print(f"recording {geometry} → {out_dir / 'take.mkv'}")
        rec.start()
        try:
            only = [s for s in os.environ.get("DEMO_SCENES", "").split(",") if s]
            for scene in SCENES:
                if not only or scene.__name__ in only:
                    scene(drv)
        finally:
            rec.finish()
            (out_dir / "timeline.json").write_text(json.dumps(drv.marks, indent=2))
        hypr("dispatch", "closewindow", window)
    finally:
        time.sleep(0.5)
        hypr("output", "remove", OUTPUT)
        hypr("keyword", "cursor:no_warps", str(warps))
        shutil.rmtree(tmp, ignore_errors=True)
    print(f"done: {out_dir / 'take.mkv'}, {out_dir / 'timeline.json'}")


if __name__ == "__main__":
    sys.path.insert(0, str(Path(__file__).parent))
    main()
