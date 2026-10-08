"""Create an unlit foreground for Icon Composer from the supplied silhouette.

Run: uv run --with pillow scripts/render-app-icon.py
The original source's RGB contains painted glow. Reuse only its alpha shape,
trim nearly invisible fringe, and center a larger glyph on a square canvas so
Icon Composer owns all colour, glass, shadow, and lighting effects.
"""
from pathlib import Path
from PIL import Image

root = Path(__file__).resolve().parent.parent
assets = root / "assets/capstan.icon/Assets"
source = Image.open(assets / "ChatGPT Image 7 Oct 2026, 16_17_00.png").convert("RGBA")
alpha = source.getchannel("A")
# Preserve the original antialiased contour, not its faint painted glow fringe.
alpha = alpha.point(lambda value: 0 if value < 8 else min(255, round(value * 255 / 250)))
bounds = alpha.getbbox()
if bounds is None:
    raise ValueError("Source foreground has no silhouette")
alpha = alpha.crop(bounds)
alpha.thumbnail((960, 960), Image.Resampling.LANCZOS)
canvas = Image.new("L", (1024, 1024), 0)
canvas.paste(alpha, ((1024 - alpha.width) // 2, (1024 - alpha.height) // 2))
foreground = Image.new("RGBA", canvas.size, (255, 255, 255, 0))
foreground.putalpha(canvas)
foreground.save(assets / "CapstanSilhouette.png")
