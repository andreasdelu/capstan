"""Extract the selected icon's pale capstan as a monochrome AppKit template.

Run with: uv run --with pillow scripts/render-menu-icon.py
The colour app icon stays untouched. Only glyph alpha is retained, so AppKit
can tint the small menu-bar image for light/dark and selected states.
"""
from pathlib import Path
from PIL import Image

root = Path(__file__).resolve().parent.parent
source = Image.open(root / "assets/CapstanIcon.png").convert("RGBA")
luma = source.convert("L")
alpha = luma.point(lambda value: max(0, min(255, (value - 170) * 4)))
icon = Image.new("RGBA", source.size, (0, 0, 0, 0))
icon.putalpha(alpha)
# The app artwork includes generous rounded-background padding. Crop to the
# glyph before scaling so its visible height fills 16 of the 18 menu-bar points.
# One point of transparent inset avoids crowding neighbouring status items.
bounds = alpha.point(lambda value: 255 if value > 32 else 0).getbbox()
if bounds is None:
    raise ValueError("Selected app icon contains no bright capstan glyph")
icon = icon.crop(bounds)
icon.thumbnail((32, 32), Image.Resampling.LANCZOS)
canvas = Image.new("RGBA", (36, 36), (0, 0, 0, 0))
canvas.alpha_composite(icon, ((36 - icon.width) // 2, (36 - icon.height) // 2))
icon = canvas
icon.save(root / "assets/MenuBarIcon.png")
