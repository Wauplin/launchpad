#!/usr/bin/env python3
"""Regenerate Launchpad's icon exports (requires Pillow)."""

import base64
from pathlib import Path

from PIL import Image

root = Path(__file__).resolve().parent.parent
icons = root / "packaging/icons"
source = Image.open(icons / "source.png").convert("RGBA")
if source.width != source.height:
    raise ValueError("The icon source must be square")

desktop = root / "desktop/src-tauri/icons"
for size in (16, 24, 32, 48, 64, 96, 128, 256, 512, 1024):
    image = source.resize((size, size), Image.Resampling.LANCZOS)
    image.save(icons / f"{size}x{size}.png")
    if size in (32, 128, 256, 512):
        name = {32: "32x32.png", 128: "128x128.png", 256: "128x128@2x.png", 512: "icon.png"}[size]
        image.save(desktop / name)
    if size == 64:
        image.save(root / "web/src/assets/launchpad.png")
    if size == 512:
        image.save(icons / "launchpad.webp", lossless=True)

# Native desktop resources, derived from the unchanged master.
source.resize((256, 256), Image.Resampling.LANCZOS).save(desktop / "icon.ico", sizes=[(16, 16), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
source.resize((1024, 1024), Image.Resampling.LANCZOS).save(desktop / "icon.icns")

# The sculpted artwork is raster, so SVG carries the PNG rather than tracing it.
encoded = base64.b64encode((icons / "1024x1024.png").read_bytes()).decode("ascii")
(root / "packaging/launchpad.svg").write_text(
    '<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024">'
    '<title>Launchpad</title>'
    f'<image width="1024" height="1024" href="data:image/png;base64,{encoded}"/>'
    '</svg>\n'
)

# Installable web app (web/public, served by the daemon without a session cookie).
web = root / "web/public/icons"
web.mkdir(parents=True, exist_ok=True)
for size, name in ((192, "icon-192.png"), (512, "icon-512.png"), (180, "apple-touch-icon.png")):
    source.resize((size, size), Image.Resampling.LANCZOS).save(web / name)
# Maskable: the artwork enlarged so its frame falls outside the mask and the plane stays inside the
# 80% safe zone, over the frame color for the corners.
background = source.getpixel((source.width // 2, source.height // 8))[:3]
maskable = Image.new("RGBA", (540, 540), background + (255,))
maskable.alpha_composite(source.resize((540, 540), Image.Resampling.LANCZOS))
maskable = maskable.crop((14, 14, 526, 526))
maskable.save(web / "maskable-512.png")
# Notification badge (Android): a white silhouette of the light parts of the artwork.
small = source.resize((96, 96), Image.Resampling.LANCZOS)
badge = Image.new("RGBA", small.size, (255, 255, 255, 0))
for y in range(small.height):
    for x in range(small.width):
        r, g, b, a = small.getpixel((x, y))
        if a > 128 and min(r, g, b) > 170:
            badge.putpixel((x, y), (255, 255, 255, 255))
badge.save(web / "badge-96.png")
