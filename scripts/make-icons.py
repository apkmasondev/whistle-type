"""Generates the WhistleType .ico files in res/icons (app icon + tray state icons).

Requires Pillow. Run: python scripts/make-icons.py
Everything is drawn procedurally at 8x and downsampled, so there are no third-party artwork assets.
"""
from pathlib import Path

from PIL import Image, ImageDraw

OUT = Path(__file__).resolve().parent.parent / "res" / "icons"
SS = 8  # supersampling factor


def lerp(a, b, t):
    return tuple(int(a[i] + (b[i] - a[i]) * t) for i in range(len(a)))


def squircle_mask(size, radius_frac=0.24):
    m = Image.new("L", (size, size), 0)
    d = ImageDraw.Draw(m)
    d.rounded_rectangle((0, 0, size - 1, size - 1), radius=int(size * radius_frac), fill=255)
    return m


def gradient(size, top, bottom):
    img = Image.new("RGBA", (size, size))
    px = img.load()
    for y in range(size):
        c = lerp(top, bottom, y / max(1, size - 1))
        for x in range(size):
            px[x, y] = c + (255,)
    return img


def draw_mic(d, cx, cy, s, color, stroke):
    """Microphone glyph centred at (cx, cy) with overall height ~ s."""
    w = s * 0.34
    h = s * 0.52
    top = cy - s * 0.46
    d.rounded_rectangle((cx - w / 2, top, cx + w / 2, top + h), radius=w / 2, fill=color)
    # cradle
    r = s * 0.31
    cradle_cy = top + h * 0.55
    d.arc((cx - r, cradle_cy - r, cx + r, cradle_cy + r), start=0, end=180, fill=color, width=int(stroke))
    # stem + base
    stem_top = cradle_cy + r
    d.line((cx, stem_top, cx, stem_top + s * 0.14), fill=color, width=int(stroke))
    d.line((cx - s * 0.16, stem_top + s * 0.14, cx + s * 0.16, stem_top + s * 0.14), fill=color, width=int(stroke))


def base_tile(size, top, bottom):
    big = size * SS
    tile = gradient(big, top, bottom)
    out = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    out.paste(tile, (0, 0), squircle_mask(big))
    return out


def app_icon(size):
    big = size * SS
    img = base_tile(size, (38, 198, 218), (40, 92, 216))
    d = ImageDraw.Draw(img)
    stroke = max(SS * 1.5, big * 0.07)
    if size >= 32:
        draw_mic(d, big * 0.42, big * 0.53, big * 0.74, (255, 255, 255, 255), stroke)
        # sound waves on the right, clear of the microphone
        for i, rr in enumerate((0.33, 0.43)):
            r = big * rr
            cx, cy = big * 0.42, big * 0.40
            d.arc((cx - r, cy - r, cx + r, cy + r), start=-32, end=32,
                  fill=(255, 255, 255, 230 - i * 80), width=int(stroke * 0.75))
    else:
        draw_mic(d, big * 0.5, big * 0.52, big * 0.78, (255, 255, 255, 255), stroke)
    return img.resize((size, size), Image.LANCZOS)


def state_icon(size, kind):
    big = size * SS
    if kind == "rec":
        img = base_tile(size, (240, 82, 82), (196, 30, 58))
    elif kind == "busy":
        img = base_tile(size, (90, 120, 240), (52, 64, 190))
    else:  # warn
        img = base_tile(size, (250, 196, 64), (226, 140, 20))
    d = ImageDraw.Draw(img)
    white = (255, 255, 255, 255)
    stroke = max(SS * 1.5, big * 0.08)
    if kind == "rec":
        draw_mic(d, big * 0.5, big * 0.52, big * 0.78, white, stroke)
        # solid "recording" dot top-right (shape cue, not only colour)
        r = big * 0.17
        d.ellipse((big - 2.2 * r, r * 0.2, big - 0.2 * r, 2.2 * r), fill=white)
        d.ellipse((big - 1.9 * r, r * 0.5, big - 0.5 * r, 1.9 * r), fill=(220, 30, 50, 255))
    elif kind == "busy":
        r = big * 0.085
        for i, x in enumerate((0.26, 0.5, 0.74)):
            d.ellipse((big * x - r, big * 0.5 - r, big * x + r, big * 0.5 + r), fill=white)
    else:
        d.rounded_rectangle((big * 0.44, big * 0.18, big * 0.56, big * 0.62), radius=big * 0.05, fill=(60, 40, 0, 255))
        d.ellipse((big * 0.43, big * 0.70, big * 0.57, big * 0.84), fill=(60, 40, 0, 255))
    return img.resize((size, size), Image.LANCZOS)


def save_ico(name, frames):
    frames = sorted(frames, key=lambda im: -im.size[0])
    path = OUT / name
    frames[0].save(path, format="ICO", sizes=[im.size for im in frames], append_images=frames[1:])
    print("wrote", path, [im.size[0] for im in frames])


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    app_sizes = (16, 20, 24, 32, 40, 48, 64, 128, 256)
    save_ico("app.ico", [app_icon(s) for s in app_sizes])
    tray_sizes = (16, 20, 24, 32, 40, 48, 64)
    for kind in ("rec", "busy", "warn"):
        save_ico(f"tray-{kind}.ico", [state_icon(s, kind) for s in tray_sizes])
    app_icon(256).save(OUT.parent.parent / "docs" / "icon-256.png")


if __name__ == "__main__":
    main()
