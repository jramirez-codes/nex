#!/usr/bin/env python3
"""Turn the `promo` clip of a record.py take into a social post: the nex window floating over a
still, macOS-style dusk landscape (layered hills fading into haze under a warm horizon glow),
with a soft shadow and a caption under it naming each feature as it plays. The clip is sped up
to fit DURATION. The backdrop doesn't move, so the GIF only stores the window's changes.

Writes promo.mp4 and promo.gif (both 1920x1080) to OUT_DIR; with --no-captions, hero.mp4 and
hero.gif instead (the README's top GIF): no caption, and the window centered.
Usage: DEMO_SCENES=promo tools/demo/record.py [TAKE_DIR]
       tools/demo/promo.py [TAKE_DIR] [OUT_DIR] [--background IMAGE] [--no-captions]
       (defaults: target/demo, site/public/demo; IMAGE replaces the generated landscape and is
       cropped to fill the frame)
       tools/demo/promo.py --still OUT.png [--background IMAGE]   just the backdrop, to preview
Needs ffmpeg and Pillow.
"""

import argparse
import json
import math
import random
import subprocess
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw, ImageFilter, ImageFont, ImageOps

REPO = Path(__file__).resolve().parents[2]
DURATION = 10.0       # seconds of output
FPS = 30              # MP4
GIF_FPS = 20
W, H = 1920, 1080     # canvas, for the MP4 and the GIF alike
WIN_W = 1440          # window width on the canvas: the take's own, so text isn't resampled
CORNER = 13           # Hyprland's window rounding in the take, in take pixels
FONT = "/usr/share/fonts/truetype/ubuntu/UbuntuSans[wdth,wght].ttf"

# Dusk sky, top to horizon, as (position 0..1 of the height, color).
SKY = [(0.00, (12, 16, 44)), (0.30, (34, 38, 96)), (0.50, (92, 70, 150)),
       (0.62, (196, 120, 150)), (0.70, (250, 176, 140)), (0.74, (255, 206, 160))]
HORIZON = 0.72
# Hills, far to near: (ridge height 0..1, amplitude px, top color, bottom color, blur px).
HILLS = [
    (0.60, 60, (176, 128, 170), (150, 108, 160), 4),
    (0.66, 70, (126, 92, 150), (98, 72, 132), 2.5),
    (0.73, 80, (78, 58, 116), (56, 42, 94), 1.5),
    (0.81, 90, (46, 34, 82), (30, 24, 58), 1.2),
    (0.90, 70, (22, 18, 46), (10, 9, 24), 0.8),
]


def lerp(a, b, k):
    return tuple(round(x + (y - x) * k) for x, y in zip(a, b))


def vertical_gradient(stops, top: int = 0, bottom: int = H) -> Image.Image:
    img = Image.new("RGB", (1, H))
    for y in range(H):
        k = min(1.0, max(0.0, (y - top) / max(1, bottom - top)))
        for (p0, c0), (p1, c1) in zip(stops, stops[1:]):
            if k <= p1 or (p1 == stops[-1][0]):
                img.putpixel((0, y), lerp(c0, c1, min(1.0, max(0.0, (k - p0) / (p1 - p0)))))
                break
    return img.resize((W, H))


def landscape(seed: int = 3) -> Image.Image:
    rnd = random.Random(seed)
    img = vertical_gradient(SKY).convert("RGBA")

    # The sun's glow just under the horizon, a little left of center.
    glow = Image.new("L", (W, H), 0)
    cx, cy = W * 0.42, H * HORIZON
    ImageDraw.Draw(glow).ellipse([cx - 800, cy - 300, cx + 800, cy + 220], fill=200)
    glow = glow.filter(ImageFilter.GaussianBlur(180))
    warm = Image.new("RGBA", (W, H), (255, 214, 170, 0))
    warm.putalpha(glow)
    img = Image.alpha_composite(img, warm)

    for i, (ridge, amp, top_c, bot_c, blur) in enumerate(HILLS):
        waves = [(rnd.uniform(0.6, 1.0) * amp, rnd.uniform(1.0, 2.2), rnd.uniform(0, 2 * math.pi)),
                 (rnd.uniform(0.25, 0.45) * amp, rnd.uniform(2.5, 4.5), rnd.uniform(0, 2 * math.pi)),
                 (rnd.uniform(0.05, 0.12) * amp, rnd.uniform(7, 11), rnd.uniform(0, 2 * math.pi))]
        pts = [(x, ridge * H - sum(a * math.sin(2 * math.pi * f * x / W + p) for a, f, p in waves))
               for x in range(-20, W + 21, 8)]
        mask = Image.new("L", (W, H), 0)
        ImageDraw.Draw(mask).polygon(pts + [(W + 20, H + 20), (-20, H + 20)], fill=255)
        if blur:
            mask = mask.filter(ImageFilter.GaussianBlur(blur))
        top = int(min(y for _, y in pts))
        fill = vertical_gradient([(0.0, top_c), (1.0, bot_c)], top, H).convert("RGBA")
        fill.putalpha(mask)
        img = Image.alpha_composite(img, fill)
        # Haze settling in the valley in front of every hill but the nearest.
        if i < len(HILLS) - 1:
            haze = Image.new("L", (W, H), 0)
            y = ridge * H + amp * 0.6
            ImageDraw.Draw(haze).rectangle([0, y - 30, W, y + 50], fill=110 - i * 18)
            haze = haze.filter(ImageFilter.GaussianBlur(40))
            mist = Image.new("RGBA", (W, H), (*lerp(SKY[-1][1], top_c, 0.5), 0))
            mist.putalpha(haze)
            img = Image.alpha_composite(img, mist)

    # A gentle vignette pulls the eye to the window.
    vig = Image.new("L", (W, H), 0)
    ImageDraw.Draw(vig).ellipse([-W * 0.15, -H * 0.25, W * 1.15, H * 1.2], fill=255)
    vig = ImageOps.invert(vig.filter(ImageFilter.GaussianBlur(220)))
    shade = Image.new("RGBA", (W, H), (6, 6, 18, 0))
    shade.putalpha(vig.point(lambda v: v * 0.55))
    return Image.alpha_composite(img, shade)


def backdrop(path: Path | None) -> Image.Image:
    if path is None:
        return landscape()
    return ImageOps.fit(Image.open(path).convert("RGB"), (W, H), Image.LANCZOS).convert("RGBA")


def take_frames(take: Path, start: float, end: float, size: tuple[int, int], fps: float = FPS):
    """The clip, sped up to DURATION and scaled to `size`, as RGB images at `fps`."""
    speed = (end - start) / DURATION
    vf = f"setpts=PTS/{speed:.5f},fps={fps},scale={size[0]}:{size[1]}:flags=lanczos"
    p = subprocess.Popen(["ffmpeg", "-loglevel", "error", "-ss", f"{start:.3f}", "-to", f"{end:.3f}",
                          "-i", str(take), "-vf", vf, "-f", "rawvideo", "-pix_fmt", "rgb24", "-"],
                         stdout=subprocess.PIPE)
    n = size[0] * size[1] * 3
    while (buf := p.stdout.read(n)) and len(buf) == n:
        yield Image.frombytes("RGB", size, buf)
    p.wait()


def rounded_mask(size: tuple[int, int], r: float, ss: int = 4) -> Image.Image:
    big = Image.new("L", (size[0] * ss, size[1] * ss), 0)
    ImageDraw.Draw(big).rounded_rectangle([0, 0, big.width - 1, big.height - 1], r * ss, fill=255)
    return big.resize(size, Image.LANCZOS)


def captions(marks: list[dict], start: float, end: float) -> list[tuple[float, str]]:
    speed = (end - start) / DURATION
    return sorted(((m["t"] - start) / speed, m["clip"].split(":", 1)[1])
                  for m in marks if m["clip"].startswith("caption:"))


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("take_dir", nargs="?", type=Path, default=REPO / "target" / "demo")
    ap.add_argument("out_dir", nargs="?", type=Path, default=REPO / "site" / "public" / "demo")
    ap.add_argument("--background", type=Path, help="image to use instead of the generated landscape")
    ap.add_argument("--still", type=Path, help="only write the backdrop to this PNG")
    ap.add_argument("--no-captions", action="store_true",
                    help="leave the captions out and center the window; writes hero.mp4/hero.gif")
    args = ap.parse_args()

    bg = backdrop(args.background)
    if args.still:
        bg.convert("RGB").save(args.still)
        print(args.still)
        return

    args.out_dir.mkdir(parents=True, exist_ok=True)
    marks = json.loads((args.take_dir / "timeline.json").read_text())
    start = next(m["t"] for m in marks if m["clip"] == "promo" and m["event"] == "start")
    end = next(m["t"] for m in marks if m["clip"] == "promo" and m["event"] == "end")
    caps = [] if args.no_captions else captions(marks, start, end)
    name = "hero" if args.no_captions else "promo"

    win = (WIN_W, round(WIN_W * 900 / 1440))
    # With captions the window sits high, leaving a band under it for them.
    wx, wy = (W - win[0]) // 2, (H - win[1]) // 2 if args.no_captions else 46
    radius = CORNER * WIN_W / 1440 + 1
    mask = rounded_mask(win, radius)

    # Shadow: a big soft one for depth and a tight one for contact, baked into the backdrop.
    for off, blur, a in ((34, 50, 170), (8, 12, 120)):
        s = Image.new("L", (W, H), 0)
        ImageDraw.Draw(s).rounded_rectangle([wx, wy + off, wx + win[0], wy + win[1] + off], radius, fill=a)
        layer = Image.new("RGBA", (W, H), (0, 0, 0, 0))
        layer.putalpha(s.filter(ImageFilter.GaussianBlur(blur)))
        bg = Image.alpha_composite(bg, layer)
    # Glass edge: a faint light rim around the window.
    rim = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    ImageDraw.Draw(rim).rounded_rectangle([wx - 1, wy - 1, wx + win[0], wy + win[1]], radius + 1,
                                          outline=(255, 255, 255, 60), width=2)

    font = ImageFont.truetype(FONT, 42)
    try:
        font.set_variation_by_axes([100, 600])
    except Exception:
        pass
    cap_y = wy + win[1] + (H - wy - win[1]) // 2 + 4
    rise_px = 12

    def caption_layer(text: str, alpha: float, rise: float) -> Image.Image:
        layer = Image.new("RGBA", (W, H), (0, 0, 0, 0))
        ImageDraw.Draw(layer).text((W // 2, cap_y + rise), text, font=font, anchor="mm",
                                   fill=(255, 255, 255, round(240 * alpha)))
        return layer

    # GIF colors, chosen once for the whole clip (one global palette, nothing dithered per frame):
    #  - the still backdrop is dithered once onto BG_COLORS of its own, so its gradients don't
    #    band and its pixels never change from frame to frame;
    #  - the window and the captions are mapped, undithered, onto the backdrop's colors plus
    #    colors picked from the recording and a white ramp, so text stays crisp and flat.
    bg_colors, cap_colors = 80, 8
    bg_rgb = bg.convert("RGB")
    # Pick the colors first, then map onto them: Pillow only dithers when given a palette.
    bg_q = bg_rgb.quantize(palette=bg_rgb.quantize(bg_colors, method=Image.Quantize.MEDIANCUT),
                           dither=Image.Dither.FLOYDSTEINBERG)
    bg_pal = [tuple(c) for _, c in bg_q.convert("RGB").getcolors(bg_colors)]
    samples = list(take_frames(args.take_dir / "take.mkv", start, end, win, fps=3))
    sheet = Image.new("RGB", (win[0], win[1] * len(samples)))
    for k, im in enumerate(samples):
        sheet.paste(im, (0, k * win[1]))
    win_q = sheet.quantize(256 - bg_colors - cap_colors, method=Image.Quantize.MEDIANCUT,
                           dither=Image.Dither.NONE)
    win_pal = [tuple(c) for _, c in win_q.convert("RGB").getcolors(256)]
    hills = bg.convert("RGB").getpixel((W // 2, cap_y))
    cap_pal = [lerp(hills, (255, 255, 255), (k + 1) / cap_colors) for k in range(cap_colors)]
    colors = (bg_pal + win_pal + cap_pal)[:256]
    pal_img = Image.new("P", (1, 1))
    pal_img.putpalette([v for c in colors for v in c] + [0] * (768 - 3 * len(colors)))
    palette_png = args.out_dir / f"{name}-palette.png"
    swatch = Image.new("RGB", (16, 16))
    swatch.putdata(colors + [colors[0]] * (256 - len(colors)))
    swatch.save(palette_png)
    bg_q = bg_q.convert("RGB")

    win_box = (wx - 4, wy - 4, wx + win[0] + 4, wy + win[1] + 4)
    cap_w = max((font.getbbox(t, anchor="mm")[2] for _, t in caps), default=0) + 10
    cap_box = (W // 2 - cap_w, cap_y - 40, W // 2 + cap_w, min(H, cap_y + rise_px + 40))

    def snap(img: Image.Image, box, where: Image.Image | None = None) -> None:
        """Map `box` (or just the pixels of it that `where` marks) onto the GIF's colors. Pillow
        maps at reduced precision, so pixels already on the palette are best left alone."""
        region = img.crop(box).quantize(palette=pal_img, dither=Image.Dither.NONE).convert("RGB")
        img.paste(region, box[:2], where.crop(box).point(lambda v: 255 if v else 0) if where else None)

    def ffmpeg_out(args_: list[str]) -> subprocess.Popen:
        return subprocess.Popen(["ffmpeg", "-loglevel", "error", "-y", "-f", "rawvideo",
                                 "-pix_fmt", "rgb24", "-s", f"{W}x{H}", "-framerate", str(FPS),
                                 "-i", "-", *args_], stdin=subprocess.PIPE)

    master = args.out_dir / f"{name}.mp4"
    gif = args.out_dir / f"{name}.gif"
    mp4_enc = ffmpeg_out(["-c:v", "libx264", "-crf", "14", "-preset", "slow", "-tune", "stillimage",
                          "-pix_fmt", "yuv420p", "-movflags", "+faststart", str(master)])
    # Every frame already uses only `colors`, so paletteuse just looks them up: no dithering, and
    # the GIF stores only the rectangle that changed since the frame before.
    gif_enc = ffmpeg_out(["-i", str(palette_png), "-filter_complex",
                          f"[0:v]fps={GIF_FPS}[v];[v][1:v]paletteuse=dither=none:diff_mode=rectangle",
                          "-loop", "0", str(gif)])
    fade = 0.3
    for i, shot in enumerate(take_frames(args.take_dir / "take.mkv", start, end, win)):
        t = i / FPS
        smooth = bg.copy()
        smooth.paste(shot, (wx, wy), mask)
        smooth = Image.alpha_composite(smooth, rim)
        flat = bg_q.copy()
        flat.paste(smooth.crop(win_box).convert("RGB"), win_box[:2])
        ink = None  # where the caption covers the backdrop this frame
        for k, (ct, text) in enumerate(caps):
            nxt = caps[k + 1][0] if k + 1 < len(caps) else DURATION + fade
            fade_in = 1.0 if k == 0 else (t - ct) / fade  # the first caption is up from frame one
            a = min(1.0, max(0.0, fade_in), max(0.0, (nxt - t) / fade))
            if a > 0:
                cap = caption_layer(text, a, round((1 - a) * rise_px))
                smooth = Image.alpha_composite(smooth, cap)
                flat = Image.alpha_composite(flat.convert("RGBA"), cap).convert("RGB")
                alpha = cap.getchannel("A")
                ink = alpha if ink is None else ImageChops.lighter(ink, alpha)
        mp4_enc.stdin.write(smooth.convert("RGB").tobytes())
        snap(flat, win_box)
        if ink is not None:
            snap(flat, cap_box, ink)
        gif_enc.stdin.write(flat.tobytes())
    for enc in (mp4_enc, gif_enc):
        enc.stdin.close()
        enc.wait()
    palette_png.unlink()
    for out in (master, gif):
        print(f"{out}  {out.stat().st_size / 1e6:5.1f} MB")


if __name__ == "__main__":
    main()
