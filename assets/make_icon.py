"""Build assets/app.ico (16-256 px) and assets/icon-256.png from assets/icon-source.png.

Source is 128x128 RGBA. Resampling is done on premultiplied alpha (no dark
fringes); 256 is a Lanczos 2x upscale, smaller sizes are Lanczos downscales
with a mild unsharp mask at <= 32 px. ICO entries are PNG-compressed (Vista+).
Run: python3 assets/make_icon.py
"""
import io, struct
from pathlib import Path
from PIL import Image, ImageFilter

here = Path(__file__).parent
src = Image.open(here / "icon-source.png").convert("RGBA")
assert src.width == src.height, src.size
SIZES = [16, 24, 32, 48, 64, 128, 256]

def render(n):
    if n == src.width:
        return src.copy()
    im = src.convert("RGBa").resize((n, n), Image.LANCZOS).convert("RGBA")
    if n <= 32:
        a = im.getchannel("A")
        im = im.filter(ImageFilter.UnsharpMask(radius=0.6, percent=60, threshold=1))
        im.putalpha(a)
    return im

imgs = {n: render(n) for n in SIZES}
imgs[256].save(here / "icon-256.png", optimize=True)

blobs = []
for n in SIZES:
    b = io.BytesIO(); imgs[n].save(b, "PNG", optimize=True); blobs.append(b.getvalue())
head = struct.pack("<HHH", 0, 1, len(SIZES))
off = 6 + 16 * len(SIZES)
dirs = b""
for n, data in zip(SIZES, blobs):
    dirs += struct.pack("<BBBBHHII", n % 256, n % 256, 0, 0, 1, 32, len(data), off)
    off += len(data)
(here / "app.ico").write_bytes(head + dirs + b"".join(blobs))

# preview sheet (not shipped)
sheet = Image.new("RGBA", (sum(SIZES) + 10 * len(SIZES) + 10, 276), (240, 240, 240, 255))
dark = Image.new("RGBA", sheet.size, (32, 32, 32, 255))
x = 10
for n in SIZES:
    for s in (sheet, dark):
        s.alpha_composite(imgs[n], (x, 10))
    x += n + 10
both = Image.new("RGBA", (sheet.width, 552)); both.paste(sheet, (0, 0)); both.paste(dark, (0, 276))
both.save("/tmp/icon_preview.png")
print("ok", [len(b) for b in blobs])
