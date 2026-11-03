"""Genera los iconos de empaquetado (macOS .icns y Linux .png) desde logo.jpg.

Uso: python scripts/make-icons.py
Fuente: logo.jpg (1024x1024); assets/icon.png mide solo 256 px y no llega a 512.
"""
from pathlib import Path
from PIL import Image

root = Path(__file__).resolve().parent.parent
src = Image.open(root / "logo.jpg").convert("RGBA")
assets = root / "crates" / "temiktli-shell" / "assets"

src.resize((512, 512), Image.LANCZOS).save(assets / "icon-512.png")
src.save(assets / "icon.icns", format="ICNS", sizes=[(16, 16), (32, 32), (64, 64), (128, 128), (256, 256), (512, 512), (1024, 1024)])
print("icon-512.png e icon.icns generados")
