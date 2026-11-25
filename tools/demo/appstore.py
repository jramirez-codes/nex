#!/usr/bin/env python3
"""Take the App Store's iPhone screenshots of nex, with nothing real in them.

The iPhone app is nex's own window on a phone: every pane works on an ssh host. `nex --remote`
is the same thin client on a desktop, so this runs it at an iPhone's size (402 x 874 points,
3x) on a throwaway headless Hyprland output, as record.py does, and connects it to a private
sshd on this machine whose home is a throwaway folder: the demo todo repo with the stub
pipeline (see agents/), an uncommitted change for the Changes pane, and a nex-host of its
own. Your own ~/.ssh, ~/.nex and ~/.config/nex are left alone.

Each scene in SHOTS drives the app with keys and takes one screenshot. A shot is nex's
window under a drawn iPhone status bar (9:41) and over the home indicator, 1206 x 2622: the
size App Store Connect's 6.3" slot takes, and uses for every iPhone. With captions (the
default), that screen is set in a rounded frame on promo.py's dusk landscape under a
headline; the plain screens are kept beside them.

What isn't the iPhone app's: the status bar and home indicator are drawn here, and the
on-screen keyboard and its button aren't shown.

Needs Hyprland, grim, /usr/sbin/sshd, ssh-keygen, git, Pillow, and the
x86_64-unknown-linux-musl Rust target.
Usage: tools/demo/appstore.py [OUT_DIR] [--plain] [--only NAME[,NAME...]] [--background IMAGE]
       (default OUT_DIR: nex-ios/appstore/screenshots/promo; --plain leaves out the captions)
Writes OUT_DIR/NN-name.jpg (to upload) and OUT_DIR/screen/NN-name.png (the bare screens).
"""

import argparse
import io
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter, ImageFont

sys.path.insert(0, str(Path(__file__).parent))
from record import CHAR_KEYS, find_window, hypr, hypr_json  # noqa: E402

REPO = Path(__file__).resolve().parents[2]
OUTPUT = "NEXSHOT"
HOST_TRIPLE = "x86_64-unknown-linux-musl"  # the "server" is this machine
# An iPhone 17 Pro's screen in points, its scale, and the safe area's insets: nex's window
# covers the part between the status bar and the home indicator.
PT_W, PT_H, SCALE = 402, 874, 3
SAFE_TOP, SAFE_BOTTOM = 62, 34
WIN_H = PT_H - SAFE_TOP - SAFE_BOTTOM
# A desktop nex window is never narrower than this (`platform::WINDOW.min_size`), a little
# wider than the phone. The window is then made taller by as much, and the grab scaled down
# to the phone's pixels: everything in it comes out 4% smaller than on the phone.
MIN_W = 420
MAX_H = -(-WIN_H * MIN_W // PT_W)
MARGIN = 60  # logical pixels around the window on the headless output
FONT = "/usr/share/fonts/truetype/ubuntu/UbuntuSans[wdth,wght].ttf"
TASK = "add stats, search and export"

CTRL_SHIFT, CTRL_ALT = "CTRL_SHIFT", "CTRL_ALT"


# ── the "server": a private sshd, a throwaway home, the demo repo ───────────────────────────


def free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def run(*cmd, **kw) -> subprocess.CompletedProcess:
    return subprocess.run([str(c) for c in cmd], check=True, capture_output=True, text=True, **kw)


class Server:
    """sshd on a spare port of this machine (loopback only), run as you, letting in one key: the device key
    written into nex's throwaway config folder. Whoever logs in with it gets `home` as HOME,
    so the shells, the repo and nex-host all live under `base`."""

    def __init__(self, base: Path):
        self.base, self.home, self.port = base, base / "home", free_port()
        self.repo = self.home / "todo"
        self.user = os.environ.get("USER") or run("id", "-un").stdout.strip()
        # What pane headers call the host. Any name under .localhost is this machine where
        # the resolver follows RFC 6761 (systemd-resolved does); else the address has to do.
        self.host = "127.0.0.1"
        try:
            if all(a[4][0] in ("127.0.0.1", "::1") for a in socket.getaddrinfo("devbox.localhost", self.port)):
                self.host = "devbox.localhost"
        except OSError:
            pass
        self.sshd: subprocess.Popen | None = None

    def start(self, config_dir: Path) -> None:
        sshd_dir = self.base / "sshd"
        sshd_dir.mkdir(parents=True)
        self.home.mkdir()
        run("ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", sshd_dir / "host_key")
        (sshd_dir / "sshd_config").write_text(
            f"Port {self.port}\nListenAddress 127.0.0.1\nListenAddress ::1\nHostKey {sshd_dir}/host_key\n"
            f"PidFile {sshd_dir}/sshd.pid\nAuthorizedKeysFile {sshd_dir}/authorized_keys\n"
            "PasswordAuthentication no\nKbdInteractiveAuthentication no\nUsePAM no\n"
            "StrictModes no\nPermitUserEnvironment yes\n")
        # nex's side, as if this device had connected before: its key (already let in by the
        # host), the host it belongs to, and the host's own key as one nex has trusted.
        config_dir.mkdir(parents=True, exist_ok=True)
        comment = "nex-device-appstore"
        run("ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-C", comment, "-f", config_dir / "device_key")
        public = (config_dir / "device_key.pub").read_text().strip()
        (sshd_dir / "authorized_keys").write_text(
            f'environment="HOME={self.home}",environment="SHELL=/bin/bash" {public}\n')
        (config_dir / "remote.json").write_text(json.dumps({
            "server": {"host": self.host, "port": self.port, "user": self.user},
            "comment": comment, "key_expires": int(time.time()) + 30 * 86_400}))
        host_key = " ".join((sshd_dir / "host_key.pub").read_text().split()[:2])
        (config_dir / "known_hosts").write_text(f"[{self.host}]:{self.port} {host_key}\n")

        self.sshd = subprocess.Popen(["/usr/sbin/sshd", "-D", "-e", "-f", str(sshd_dir / "sshd_config")],
                                     stderr=subprocess.DEVNULL)
        for _ in range(100):
            try:
                socket.create_connection((self.host, self.port), timeout=0.2).close()
                break
            except OSError:
                time.sleep(0.05)
        else:
            raise SystemExit("the private sshd didn't start (is /usr/sbin/sshd installed?)")
        self._make_home()

    def _make_home(self) -> None:
        """The demo repo with something for Changes to show, and a shell with a tidy prompt
        that starts in it."""
        run(REPO / "tools/demo/agents/setup.sh", self.repo)
        git = ["git", "-c", "user.name=demo", "-c", "user.email=demo@example.com", "commit", "-q", "--allow-empty", "-m"]
        for subject in ("Keep todos in todos.json", "done: mark a todo by its id", "ls: show what's left first",
                        "Tests for add, done and ls"):
            run(*git, subject, cwd=self.repo)
        for todo in ("review the stats PR", "write the export docs", "ship 0.2.0"):
            run("python3", "-m", "app.cli", "add", todo, cwd=self.repo)
        cli = self.repo / "app" / "cli.py"
        if cli.exists():
            cli.write_text(cli.read_text().rstrip("\n") + '\n\n\ndef version() -> str:\n'
                           '    """The version `--version` prints."""\n    return "0.2.0"\n')
        (self.repo / "NOTES.md").write_text("# Notes\n\n- stats, search and export: one PR each\n"
                                            "- keep each command in its own file\n")
        (self.home / ".gitconfig").write_text("[user]\n\tname = demo\n\temail = demo@example.com\n")
        (self.home / ".bashrc").write_text(
            f'cd "{self.repo}" 2>/dev/null\n'
            'export HISTFILE=/dev/null PATH="$HOME/.nex/bin:$PATH"\n'
            # The window title is the tab's name: a server's, not this machine's.
            r"PS1='\[\e]0;dev@devbox: ~/todo\a\]\[\e[1;35m\]todo\[\e[0m\] \[\e[2m\]on\[\e[0m\] "
            r"\[\e[36m\] $(git branch --show-current)\[\e[0m\] \[\e[1;32m\]❯\[\e[0m\] '" "\n")
        (self.home / ".bash_profile").write_text(". ~/.bashrc\n")

    def stop(self) -> None:
        # nex-host's daemon and the agents it runs: everything started from the throwaway home.
        subprocess.run(["pkill", "-KILL", "-f", str(self.home)], capture_output=True)
        if self.sshd:
            subprocess.run(["pkill", "-KILL", "-P", str(self.sshd.pid)], capture_output=True)
            self.sshd.kill()
            self.sshd.wait()


# ── the phone: nex on a headless output, driven with keys ───────────────────────────────────


class Phone:
    """What scenes use: keys, typing, waiting, and a screenshot of the window."""

    def __init__(self, window: str, geometry: str, server: Server):
        self.window, self.geometry, self.server = window, geometry, server

    def key(self, key: str, mods: str = "", after: float = 0.3) -> None:
        hypr("dispatch", "sendshortcut", f"{mods},{key},{self.window}")
        time.sleep(after)

    def type(self, text: str, enter: bool = True, after: float = 0.5) -> None:
        for ch in text:
            shift, key = CHAR_KEYS.get(ch, (ch.isupper(), ch.lower()))
            self.key(key, "SHIFT" if shift else "", after=0.05)
        if enter:
            time.sleep(0.2)
            self.key("Return", after=0)
        time.sleep(after)

    def wait(self, seconds: float) -> None:
        time.sleep(seconds)

    def host(self, *argv: str) -> str:
        """Run a command on the "server", in the demo repo, outside the window."""
        env = {**os.environ, "HOME": str(self.server.home), "SHELL": "/bin/bash"}
        return subprocess.run(argv, cwd=self.server.repo, env=env, capture_output=True, text=True).stdout

    def shot(self) -> Image.Image:
        png = subprocess.run(["grim", "-t", "png", "-g", self.geometry, "-"], check=True, capture_output=True).stdout
        return Image.open(io.BytesIO(png)).convert("RGB")


def add_output() -> tuple[int, int, int, int]:
    """The headless output, to the right of the real monitors. As in record.py, the window
    goes below every real monitor's height, where your pointer (which Hyprland draws on the
    headless output too) can't follow. Returns (workspace, x, y, top) in layout coordinates."""
    top = max(round(m["height"] / m["scale"]) for m in hypr_json("monitors")) + MARGIN
    width, height = MIN_W + 2 * MARGIN, top + MAX_H + MARGIN
    hypr("output", "create", "headless", OUTPUT)
    hypr("keyword", "monitor", f"{OUTPUT},{width * SCALE}x{height * SCALE}@60,auto-right,{SCALE}")
    time.sleep(1.0)
    for m in hypr_json("monitors"):
        if m["name"] == OUTPUT:
            if m["scale"] != SCALE or round(m["height"] / m["scale"]) < top + MAX_H:
                raise SystemExit(f"the headless output came up {m['width']}x{m['height']} at scale "
                                 f"{m['scale']}, not {width * SCALE}x{height * SCALE} at {SCALE}")
            return m["activeWorkspace"]["id"], m["x"], m["y"], top
    raise SystemExit("headless output never appeared")


# ── the shots ───────────────────────────────────────────────────────────────────────────────
# Each scene starts where the one before it left the app and returns the screen to keep.


ZOOM = ("Return", CTRL_SHIFT)  # a pane fills the tab, as a phone's one pane at a time


def shell(p: Phone) -> Image.Image:
    """A shell on the server, in the repo."""
    p.type("clear", after=0.4)
    p.type("git log --oneline -5", after=0.8)
    p.type("git status --short", after=0.8)
    p.type("python3 -m app.cli ls", after=1.2)
    return p.shot()


def changes(p: Phone) -> Image.Image:
    """The Changes pane on the server: what git sees changed, and a file's diff."""
    p.key("G", "CTRL", after=1.5)
    p.key(*ZOOM, after=0.8)
    p.key("Down", after=0.4)
    p.key("Return", after=1.5)
    return p.shot()


def files(p: Phone) -> Image.Image:
    """The file explorer on the server, a couple of files selected."""
    p.key("T", CTRL_SHIFT, after=1.5)
    p.key("E", "CTRL", after=1.5)
    p.key(*ZOOM, after=0.8)
    for k in ("Down", "space", "Down", "space"):
        p.key(k, after=0.2)
    p.wait(0.6)
    return p.shot()


def working(grab: Image.Image) -> int:
    """How many rows of an Agents pane grab say "Working": runs of pixel rows with the state's
    green in the column the states are in."""
    w, h = grab.size
    band = grab.crop((round(w * 0.78), 0, round(w * 0.93), h))
    px, bw = band.load(), band.width
    rows = [sum(1 for x in range(bw) if px[x, y][1] > 170 and px[x, y][0] < 130 and px[x, y][2] < 150) >= 3
            for y in range(h)]
    return sum(1 for y in range(1, h) if rows[y] and not rows[y - 1])


def agents_pipeline(p: Phone) -> Image.Image:
    """The Agents pane: the stub pipeline's three builders working in parallel."""
    p.key("T", CTRL_SHIFT, after=1.5)
    p.host(str(p.server.home / ".nex/bin/nex-host"), "pipe", "demo", TASK, "--bg")
    p.key("A", CTRL_SHIFT, after=1.5)
    p.key(*ZOOM, after=0.8)
    p.key("Down", after=0.4)
    p.key("space", after=0.6)                    # open the team: plan, builders, merge
    for _ in range(100):                         # the plan is done, all three builders at work
        grab = p.shot()
        if working(grab) >= 3:
            p.wait(0.4)                          # let the rows settle
            return p.shot()
        p.wait(0.1)
    raise SystemExit("the demo pipeline's three builders never showed as Working")


def agents_review(p: Phone) -> Image.Image:
    """The same run later: review and merge, one agent waiting on you."""
    p.wait(4.0)                                  # two PRs in, the merger on the first
    return p.shot()


def agent(p: Phone) -> Image.Image:
    """Inside one of the pipeline's agents, in a tab of its own."""
    p.key("Down", after=0.5)
    p.key("Return", after=3.0)
    return p.shot()


# (name, headline, scene), in upload order. Scenes run in RUN order: the shell first (the
# app starts in one), the pipeline last (it leaves agents behind).
SHOTS = [
    ("agents-pipeline", "Every agent\nin one place", agents_pipeline),
    ("agents-review", "See who\nneeds you", agents_review),
    ("agent", "Step into\nany agent", agent),
    ("changes", "Review the diff\nfrom anywhere", changes),
    ("files", "Browse your\nserver's files", files),
    ("shell", "Your server,\nin your pocket", shell),
]
RUN = [shell, changes, files, agents_pipeline, agents_review, agent]


# ── from a window grab to an App Store screenshot ───────────────────────────────────────────


def font(size: float, weight: int) -> ImageFont.FreeTypeFont:
    f = ImageFont.truetype(FONT, round(size))
    try:
        f.set_variation_by_axes([100, weight])  # width, weight
    except OSError:
        pass
    return f


def status_bar(d: ImageDraw.ImageDraw) -> None:
    """9:41, full signal, Wi-Fi and a full battery, as Apple's own screenshots show."""
    s, white = SCALE, (255, 255, 255)
    mid = 33 * s  # the row's center
    d.text((66 * s, mid), "9:41", font=font(17 * s, 600), fill=white, anchor="mm")
    # Signal: four bars, rising.
    x = 281 * s
    for i in range(4):
        h = (4 + 2.6 * i) * s
        d.rounded_rectangle([x, mid + 6 * s - h, x + 3 * s, mid + 6 * s], radius=1 * s, fill=white)
        x += 4.7 * s
    # Wi-Fi: three arcs over a dot.
    cx, base = 314 * s, mid + 6 * s
    for r, w in ((11.5, 2.6), (7.6, 2.6)):
        d.arc([cx - r * s, base - r * s, cx + r * s, base + r * s], 225, 315, fill=white, width=round(w * s))
    d.pieslice([cx - 3.9 * s, base - 3.9 * s, cx + 3.9 * s, base + 3.9 * s], 225, 315, fill=white)
    # Battery: a full cell in its outline, and the cap.
    x0, y0, w, h = 331 * s, mid - 6.5 * s, 25 * s, 13 * s
    d.rounded_rectangle([x0, y0, x0 + w, y0 + h], radius=4 * s, outline=(255, 255, 255, 110), width=round(1 * s))
    d.rounded_rectangle([x0 + 2 * s, y0 + 2 * s, x0 + w - 2 * s, y0 + h - 2 * s], radius=2.2 * s, fill=white)
    d.rounded_rectangle([x0 + w + 1.2 * s, mid - 2.2 * s, x0 + w + 2.6 * s, mid + 2.2 * s], radius=0.8 * s,
                        fill=(255, 255, 255, 110))


def screen(window: Image.Image) -> Image.Image:
    """The whole iPhone screen: black, the status bar, nex's window, the home indicator."""
    img = Image.new("RGB", (PT_W * SCALE, PT_H * SCALE), (0, 0, 0))
    img.paste(window.resize((PT_W * SCALE, WIN_H * SCALE), Image.LANCZOS), (0, SAFE_TOP * SCALE))
    d = ImageDraw.Draw(img, "RGBA")
    status_bar(d)
    s = SCALE
    d.rounded_rectangle([(PT_W - 139) / 2 * s, (PT_H - 13) * s, (PT_W + 139) / 2 * s, (PT_H - 8) * s],
                        radius=2.5 * s, fill=(255, 255, 255, 235))
    return img


def captioned(shot: Image.Image, headline: str, backdrop: Image.Image) -> Image.Image:
    """The screen in a rounded frame on the backdrop, under its headline."""
    W, H = shot.size
    img = backdrop.copy()
    d = ImageDraw.Draw(img)
    f = font(104, 700)
    d.multiline_text((W / 2, 150), headline, font=f, fill=(255, 255, 255), anchor="ma", align="center", spacing=14)
    # The phone: the screen at 80%, its bottom running off the canvas, in a thin bezel.
    k = 0.80
    w, h = round(W * k), round(H * k)
    x, y = (W - w) // 2, 520
    radius, bezel = round(55 * SCALE * k), 14
    shadow = Image.new("RGBA", img.size, (0, 0, 0, 0))
    ImageDraw.Draw(shadow).rounded_rectangle([x - bezel, y - bezel + 30, x + w + bezel, y + h + bezel + 30],
                                             radius=radius + bezel, fill=(0, 0, 0, 150))
    img = Image.alpha_composite(img.convert("RGBA"), shadow.filter(ImageFilter.GaussianBlur(50)))
    d = ImageDraw.Draw(img)
    d.rounded_rectangle([x - bezel, y - bezel, x + w + bezel, y + h + bezel], radius=radius + bezel,
                        fill=(16, 16, 20), outline=(72, 72, 84), width=3)
    mask = Image.new("L", (w, h), 0)
    ImageDraw.Draw(mask).rounded_rectangle([0, 0, w, h], radius=radius, fill=255)
    img.paste(shot.resize((w, h), Image.LANCZOS), (x, y), mask)
    return img.convert("RGB")


def make_backdrop(image: str | None) -> Image.Image:
    """promo.py's dusk landscape (or `image`), cropped to the screenshot's shape."""
    from PIL import ImageOps
    size = (PT_W * SCALE, PT_H * SCALE)
    if image:
        return ImageOps.fit(Image.open(image).convert("RGB"), size, Image.LANCZOS)
    import promo
    land = promo.landscape().convert("RGB")
    # The landscape is 16:9: take a phone-shaped slice around the sun's glow, then darken the
    # top a little so the headline reads.
    slice_w = round(land.height * size[0] / size[1])
    left = round(land.width * 0.42 - slice_w / 2)
    img = land.crop((left, 0, left + slice_w, land.height)).resize(size, Image.LANCZOS)
    shade = Image.new("L", (1, size[1]))
    for y in range(size[1]):
        shade.putpixel((0, y), round(90 * max(0.0, 1 - y / (size[1] * 0.35))))
    img.paste((8, 10, 30), (0, 0), shade.resize(size))
    return img


# ── main ────────────────────────────────────────────────────────────────────────────────────


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("out_dir", nargs="?", default=str(REPO / "nex-ios/appstore/screenshots/promo"))
    ap.add_argument("--plain", action="store_true", help="the bare screens, no headline or backdrop")
    ap.add_argument("--only", default="", help="comma-separated shot names to write (all scenes still run)")
    ap.add_argument("--background", help="an image to use instead of the generated landscape")
    args = ap.parse_args()
    out_dir = Path(args.out_dir).resolve()
    only = {s for s in args.only.split(",") if s}
    unknown = only - {name for name, _, _ in SHOTS}
    if unknown:
        raise SystemExit(f"no shot called {', '.join(sorted(unknown))}: there are {', '.join(n for n, _, _ in SHOTS)}")

    cargo = ["cargo", "build", "--release", "-q", "--manifest-path", str(REPO / "Cargo.toml")]
    subprocess.run(cargo, check=True)
    subprocess.run([*cargo, "-p", "nex-host", "--target", HOST_TRIPLE], check=True)

    base = Path(tempfile.mkdtemp(prefix="nex-appstore-"))
    config = base / "config" / "nex"
    server = Server(base)
    grabs: dict[str, Image.Image] = {}
    warps = hypr_json("getoption", "cursor:no_warps")["int"]
    made_output = False
    try:
        server.start(config)
        (config / "config").write_text(
            "font-size = 12\n"
            "background-opacity = 1.0\n"
            "window-decorations = false\n"
            "window-controls = false\n"
            "cursor-style-blink = false\n"
            "agents-tab = icon\n"            # the phones' default: the dog alone
            "ssh-agent-shim = off\n")
        (base / "state").mkdir()
        (base / "host").mkdir()
        shutil.copy(REPO / "target" / HOST_TRIPLE / "release" / "nex-host", base / "host" / f"nex-host-{HOST_TRIPLE}")

        ws, mon_x, mon_y, top = add_output()
        made_output = True
        hypr("keyword", "cursor:no_warps", "true")
        x, y = mon_x + MARGIN, mon_y + top
        rules = f"workspace {ws} silent; float; size {PT_W} {WIN_H}; move {x} {y}"
        cmd = (f"env XDG_CONFIG_HOME={base}/config XDG_STATE_HOME={base}/state NEX_HOST_DIR={base}/host "
               f"{REPO}/target/release/nex --remote={server.user}@{server.host}:{server.port}")
        hypr("dispatch", "exec", f"[{rules}] {cmd}")
        marker, client = f"XDG_CONFIG_HOME={base}/config", None
        for _ in range(50):
            client = find_window(marker)
            if client:
                break
            time.sleep(0.2)
        if not client:
            raise SystemExit("nex window never appeared")
        window = f"address:{client['address']}"
        for prop in ("opaque", "no_blur", "noblur", "rounding 0", "no_rounding", "norounding", "no_border", "noborder",
                     "no_shadow", "noshadow"):
            name, _, value = prop.partition(" ")
            subprocess.run(["hyprctl", "dispatch", "setprop", window, name, value or "1"], capture_output=True)
        # Logging in, installing nex-host in the throwaway home, the first shell.
        nex_host = server.home / ".nex/bin/nex-host"
        for _ in range(100):
            if nex_host.exists():
                break
            time.sleep(0.2)
        else:
            raise SystemExit("nex never connected to the private sshd (nex-host wasn't installed there)")
        # The window came up as wide as nex lets it be: keep the phone's shape at that width.
        width = find_window(marker)["size"][0]
        height = round(WIN_H * width / PT_W)
        if not PT_W <= width <= MIN_W:
            raise SystemExit(f"nex's window is {width} wide: expected {PT_W} to {MIN_W}")
        hypr("dispatch", "resizewindowpixel", f"exact {width} {height},{window}")
        hypr("dispatch", "movewindowpixel", f"exact {x} {y},{window}")
        time.sleep(3.0)
        client = find_window(marker)
        cx, cy = client["at"]
        cw, ch = client["size"]
        if (cx, cy, cw, ch) != (x, y, width, height):
            raise SystemExit(f"nex is {cw}x{ch} at {cx},{cy}, not {width}x{height} at {x},{y} on the headless "
                             "output: the shots would show your own screen or pointer")
        phone = Phone(window, f"{cx},{cy} {cw}x{ch}", server)
        names = {scene: name for name, _, scene in SHOTS}
        for scene in RUN:
            print(f"  {names[scene]}", flush=True)
            grabs[names[scene]] = scene(phone)
        hypr("dispatch", "closewindow", window)
    finally:
        time.sleep(0.5)
        subprocess.run(["pkill", "-f", f"XDG_CONFIG_HOME={base}/config"], capture_output=True)  # if a scene failed
        if made_output:
            hypr("output", "remove", OUTPUT)
        hypr("keyword", "cursor:no_warps", str(warps))
        server.stop()
        shutil.rmtree(base, ignore_errors=True)

    (out_dir / "screen").mkdir(parents=True, exist_ok=True)
    backdrop = None if args.plain else make_backdrop(args.background)
    for n, (name, headline, _) in enumerate(SHOTS, 1):
        if only and name not in only:
            continue
        shot = screen(grabs[name])
        shot.save(out_dir / "screen" / f"{n:02}-{name}.png")
        final = shot if backdrop is None else captioned(shot, headline, backdrop)
        # App Store Connect wants no alpha: JPEG has none.
        final.save(out_dir / f"{n:02}-{name}.jpg", quality=92)
        print(out_dir / f"{n:02}-{name}.jpg")


if __name__ == "__main__":
    main()
