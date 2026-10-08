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
icon = icon.resize((36, 36), Image.Resampling.LANCZOS)
icon.save(root / "assets/MenuBarIcon.png")
