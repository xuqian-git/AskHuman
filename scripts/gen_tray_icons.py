#!/usr/bin/env python3
"""Cut the three robot states out of the design artwork
(`src-tauri/icons/tray/source.png`, the "light menu bar" row of the AskHuman
icon-system mockup) into clean, transparent, monochrome elements:

  - idle     : awake robot          (daemon running, nothing pending)
  - active   : awake robot + "?"     (questions are waiting to be answered)
  - stopped  : sleeping robot + moon (shown in "always" mode when stopped)

Pipeline (per state): crop the state's region, derive an alpha channel from
pixel darkness (dark ink -> opaque, light paper / soft drop shadow ->
transparent, edges keep their anti-aliasing), force the RGB to pure black, then
trim to the tight content box. The white "?" and the white ring around the badge
therefore become real transparent holes.

NOTE ON THE FINAL ICONS
=======================
These are the raw *cut-outs* (head + accent, tightly trimmed), written to
`src-tauri/icons/tray/cutouts/`. They are NOT the files the app ships.

The shipped menu-bar icons — `src-tauri/icons/tray/tray-{idle,active,stopped}.png`
— are **hand-composed by the designer** from these cut-outs (equal canvas size,
head centered, badge / moon overlapping the corner, exported @2x). This script
must NOT overwrite them; regenerate the cut-outs here and recompose by hand.

Final-icon spec (for whoever recomposes them):
  - 32-bit RGBA PNG, ink = pure black #000000, transparency via alpha.
  - macOS uses them as *template* images (only alpha matters; auto-tinted).
  - All three the SAME canvas size with the head at the SAME position, because
    the tray scales each icon to 18pt height (aspect preserved) — equal height
    keeps the on-screen head size identical across states.

Requires: ImageMagick (`magick`). `brew install imagemagick` /
          `apt install imagemagick`.
"""

import os
import shutil
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
TRAY = os.path.join(HERE, "..", "src-tauri", "icons", "tray")
SRC = os.path.join(TRAY, "source.png")
OUT = os.path.join(TRAY, "cutouts")

# Per-state crop window in source.png (wide enough to contain one robot, narrow
# enough to exclude the neighbouring column dividers). The element is trimmed
# tight afterwards. state -> (output basename, window x, window width).
STATES = {
    "active": ("active", 0, 240),
    "idle": ("idle", 400, 280),
    "stopped": ("stopped", 820, 250),
}
SRC_H = 200
# Darkness -> alpha mapping (input black% , white%): clamps the light paper AND
# the mockup's soft drop shadow to fully transparent, keeping the ink opaque.
LEVEL = "26%,92%"


def cut(x: int, w: int, out_path: str) -> None:
    subprocess.run(
        [
            "magick", SRC,
            "-crop", f"{w}x{SRC_H}+{x}+0", "+repage",
            # alpha from darkness (anti-aliased)
            "(", "+clone", "-colorspace", "Gray", "-negate", "-level", LEVEL, ")",
            "-compose", "CopyOpacity", "-composite",
            # force the ink to pure black, keep the derived alpha
            "-fill", "black", "-colorize", "100",
            # tight bounding box, plain 32-bit RGBA
            "-trim", "+repage",
            f"PNG32:{out_path}",
        ],
        check=True,
    )


def main() -> int:
    if not shutil.which("magick"):
        print("error: ImageMagick `magick` not found", file=sys.stderr)
        return 1
    if not os.path.exists(SRC):
        print(f"error: source artwork not found: {SRC}", file=sys.stderr)
        return 1
    os.makedirs(OUT, exist_ok=True)
    for _, (base, x, w) in STATES.items():
        cut(x, w, os.path.join(OUT, f"{base}.png"))
        print(f"  cutouts/{base}.png")
    print(f"transparent cut-outs written to {os.path.normpath(OUT)}")
    print("NOTE: recompose the shipped tray-*.png by hand (see module docstring).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
