# Icônes PNG de la PWA (web/public) depuis web/public/icon.svg et scripts/icons/icon-maskable.svg.
# Usage : scrapling-python scripts/icons/make-icons.py  (Playwright + Chromium ; CHROME=chemin facultatif)
import os
from pathlib import Path
from playwright.sync_api import sync_playwright

root = Path(__file__).resolve().parents[2]
pub = root / "web" / "public"
src_dir = {"icon.svg": pub, "icon-maskable.svg": Path(__file__).resolve().parent}
jobs = [
    ("icon.svg", "icon-192.png", 192),
    ("icon.svg", "icon-512.png", 512),
    ("icon-maskable.svg", "icon-maskable-512.png", 512),
    ("icon-maskable.svg", "apple-touch-icon.png", 180),
]
with sync_playwright() as p:
    b = p.chromium.launch(executable_path=os.environ.get("CHROME") or None)
    for src, out, size in jobs:
        pg = b.new_page(viewport={"width": size, "height": size})
        svg = (src_dir[src] / src).read_text()
        pg.set_content(f'<html><body style="margin:0;background:transparent">'
                       f'<img src="data:image/svg+xml;base64,{__import__("base64").b64encode(svg.encode()).decode()}" '
                       f'width="{size}" height="{size}" style="display:block"></body></html>')
        pg.wait_for_timeout(100)
        pg.screenshot(path=str(pub / out), omit_background=True, clip={"x": 0, "y": 0, "width": size, "height": size})
        pg.close()
        print(out)
    b.close()
