"""Builds docs/screenshot.png (README hero image): the settings window + the three overlay states.

Inputs come from scripts/e2e/capture-windows.ps1 (settings-en.png) and scripts/e2e/capture-ui.ps1 (overlay-*.png).
usage: python scripts/compose-screenshot.py
"""
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

DOCS = Path(__file__).resolve().parent.parent / "docs"
BG = (236, 238, 242)


def main():
    settings = Image.open(DOCS / "settings-en.png").convert("RGB")
    overlays = [("Hold F8: listening (live level)", "overlay-listening.png"),
                ("Release: transcribing", "overlay-transcribing.png"),
                ("Silence / noise: nothing is typed", "overlay-nospeech.png")]
    imgs = [(t, Image.open(DOCS / f).convert("RGB")) for t, f in overlays]
    try:
        font = ImageFont.truetype("segoeui.ttf", 22)
    except OSError:
        font = ImageFont.load_default()
    pad = 30
    right_w = max(i.width for _, i in imgs) + 40
    w = pad + settings.width + pad + right_w + pad
    h = pad + settings.height + pad
    out = Image.new("RGB", (w, h), BG)
    out.paste(settings, (pad, pad))
    d = ImageDraw.Draw(out)
    x, y = pad + settings.width + pad, pad
    for title, img in imgs:
        d.text((x, y), title, fill=(40, 40, 40), font=font)
        y += 34
        out.paste(img, (x, y))
        y += img.height + 40
    out.save(DOCS / "screenshot.png", optimize=True)
    print("docs/screenshot.png", out.size)


if __name__ == "__main__":
    main()
