#!/usr/bin/env python3
"""Rebuild the Bolo.com icon from the Nastaliq master, then simplify 16–32px.

`tauri icon` scales one 1024 source into every size. The sound waves beside
بولو disappear at 16–32px and crush the word, so those three sizes are drawn
again: the same dark gold-rimmed tile, the word larger, no waves.
"""

from __future__ import annotations

import base64
import io
import struct
import subprocess
import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parents[1]
ICONS = ROOT / "src-tauri" / "icons"
MASTER = ICONS / "icon-source.png"
WORD = "\u0628\u0648\u0644\u0648"
GOLD = (0xC6, 0xA1, 0x5A, 255)
DARK = (0x10, 0x0F, 0x0C, 255)
INK = (0x10, 0x0F, 0x0C, 255)
CREAM = (0xF4, 0xEF, 0xE6, 255)
MUTED = (0xA8, 0x9E, 0x90, 255)
SERIF_BOLD = "/usr/share/fonts/truetype/liberation/LiberationSerif-Bold.ttf"
SERIF = "/usr/share/fonts/truetype/liberation/LiberationSerif-Regular.ttf"
WORD_FILE = Path("/tmp/bolo-word.txt")


def pango_word(font: str, px: int) -> Image.Image:
    WORD_FILE.write_text(WORD + "\n", encoding="utf-8")
    out = Path(f"/tmp/bolo-pango-{px}.png")
    subprocess.check_call(
        [
            "pango-view",
            "--no-display",
            "--rtl",
            "--language=ur",
            "--pixels",
            f"--font={font} {px}",
            "--hinting=full",
            "--antialias=gray",
            "--foreground=#C6A15A",
            "--background=transparent",
            "--margin=8",
            "-o",
            str(out),
            str(WORD_FILE),
        ]
    )
    image = Image.open(out).convert("RGBA")
    return image.crop(image.getbbox())


_NAST: Image.Image | None = None


def nastaliq() -> Image.Image:
    global _NAST
    if _NAST is None:
        _NAST = pango_word("Noto Nastaliq Urdu Bold", 280)
    return _NAST


def simplified(size: int) -> Image.Image:
    """Dark tile, gold rim, بولو kept inside the rim with the waves left off."""
    image = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    draw = ImageDraw.Draw(image)
    pad = 0 if size <= 24 else max(1, round(size * 36 / 1024))
    radius = max(2, round((size - 2 * pad) * 0.22))
    draw.rounded_rectangle((pad, pad, size - 1 - pad, size - 1 - pad), radius=radius, fill=GOLD)
    ring = 1 if size <= 24 else max(2, round((size - 2 * pad) * 28 / 952))
    inner_radius = max(1, radius - ring)
    inner = (pad + ring, pad + ring, size - 1 - pad - ring, size - 1 - pad - ring)
    draw.rounded_rectangle(inner, radius=inner_radius, fill=DARK)
    # Keep a dark gap so the descending dot does not fuse with the rim.
    gap = 1 if size <= 24 else 2
    room = inner[2] - inner[0] - 2 * gap
    glyph = nastaliq()
    scale = min(room / glyph.width, room / glyph.height)
    resized = glyph.resize(
        (max(1, round(glyph.width * scale)), max(1, round(glyph.height * scale))),
        Image.Resampling.LANCZOS,
    )
    image.alpha_composite(
        resized,
        (inner[0] + (inner[2] - inner[0] - resized.width) // 2, inner[1] + (inner[3] - inner[1] - resized.height) // 2),
    )
    return snap(image, 80 if size <= 16 else 70)


def snap(image: Image.Image, threshold: int) -> Image.Image:
    out = image.copy()
    pixels = out.load()
    for y in range(out.height):
        for x in range(out.width):
            red, green, blue, alpha = pixels[x, y]
            if alpha < threshold:
                pixels[x, y] = (0, 0, 0, 0)
            elif red > 110 and green > 80 and red > blue:
                pixels[x, y] = GOLD
            else:
                pixels[x, y] = DARK if alpha > 200 else (0, 0, 0, 0)
    return out


def png_bytes(image: Image.Image) -> bytes:
    buffer = io.BytesIO()
    image.save(buffer, format="PNG")
    return buffer.getvalue()


def dib_bytes(image: Image.Image) -> bytes:
    """Uncompressed 32-bit icon image (BGRA, bottom-up) plus an empty AND mask.

    NSIS copies icon images into fixed slots and Windows `ExtractAssociatedIcon`
    reads a 32px bitmap. A PNG stuffed in that slot is shown as an unrelated
    32px image, and Chrome's download list then falls back to a generic icon.
    """
    image = image.convert("RGBA")
    width, height = image.size
    raw = image.tobytes()
    pixels = bytearray()
    for y in range(height - 1, -1, -1):
        row = raw[y * width * 4 : (y + 1) * width * 4]
        bgra = bytearray(width * 4)
        for x in range(width):
            red, green, blue, alpha = row[x * 4 : x * 4 + 4]
            bgra[x * 4 : x * 4 + 4] = bytes((blue, green, red, alpha))
        pixels += bgra
    stride = ((width + 31) // 32) * 4
    mask = bytes(stride * height)
    header = struct.pack(
        "<IiiHHIIiiII",
        40,
        width,
        height * 2,
        1,
        32,
        0,
        len(pixels) + len(mask),
        0,
        0,
        0,
        0,
    )
    return header + bytes(pixels) + mask


def write_ico(path: Path, images: list[Image.Image]) -> None:
    blobs = [dib_bytes(image) for image in images]
    out = bytearray(struct.pack("<HHH", 0, 1, len(blobs)))
    cursor = 6 + 16 * len(blobs)
    directory = bytearray()
    payload = bytearray()
    for image, blob in zip(images, blobs):
        width = 0 if image.width >= 256 else image.width
        height = 0 if image.height >= 256 else image.height
        directory += struct.pack("<BBBBHHII", width, height, 0, 0, 1, 32, len(blob), cursor)
        payload += blob
        cursor += len(blob)
    path.write_bytes(out + directory + payload)


def assert_ico(path: Path, sizes: tuple[int, ...]) -> None:
    data = path.read_bytes()
    count = struct.unpack_from("<H", data, 4)[0]
    found: list[int] = []
    for index in range(count):
        width, _height, _pal, _res, _planes, bpp, size, offset = struct.unpack_from(
            "<BBBBHHII", data, 6 + 16 * index
        )
        width = width or 256
        blob = data[offset : offset + size]
        found.append(width)
        if blob[:8] == b"\x89PNG\r\n\x1a\n":
            raise SystemExit(f"{path.name} size {width} is a PNG; Windows would show a bad 32px icon")
        bi_size, bi_width, bi_height, _planes, bi_bit, compression = struct.unpack_from("<IiiHHI", blob, 0)
        if bi_size != 40 or bi_bit != 32 or compression != 0 or bi_width != width or bi_height != width * 2:
            raise SystemExit(f"{path.name} size {width} is not a 32-bit DIB")
        if bpp != 32:
            raise SystemExit(f"{path.name} size {width} directory is not 32-bit")
    if tuple(found) != sizes:
        raise SystemExit(f"{path.name} sizes {found} != {list(sizes)}")


def windows_frames(master: Image.Image) -> list[Image.Image]:
    """32px first, then the rest. Small sizes keep the readable word; larger
    sizes are the sound-wave master."""
    small = {size: simplified(size) for size in (16, 24, 32)}
    order = (32, 16, 24, 48, 64, 128, 256)
    frames: list[Image.Image] = []
    for size in order:
        if size in small:
            frames.append(small[size])
        else:
            frames.append(master.resize((size, size), Image.Resampling.LANCZOS))
    return frames


def rebuild_windows_icons(master: Image.Image) -> None:
    frames = windows_frames(master)
    write_ico(ICONS / "icon.ico", frames)
    assert_ico(ICONS / "icon.ico", tuple(frame.width for frame in frames))
    favicon = [frame for frame in frames if frame.width in (16, 32, 48)]
    # Browsers want 16 then 32. Keep 32 available without making it the only image.
    favicon.sort(key=lambda frame: frame.width)
    website = ROOT / "website" / "assets"
    write_ico(website / "favicon.ico", favicon)
    assert_ico(website / "favicon.ico", tuple(frame.width for frame in favicon))
    next(frame for frame in frames if frame.width == 32).save(website / "favicon-32.png")
    desktop = {
        32: ICONS / "32x32.png",
        64: ICONS / "64x64.png",
        128: ICONS / "128x128.png",
        256: ICONS / "128x128@2x.png",
    }
    for frame in frames:
        path = desktop.get(frame.width)
        if path is not None:
            frame.save(path)


def patch_ico(path: Path, replacements: dict[int, Image.Image]) -> None:
    data = path.read_bytes()
    count = struct.unpack_from("<H", data, 4)[0]
    entries: list[tuple[int, int, bytes]] = []
    for index in range(count):
        width, height, _pal, _res, _planes, _bpp, size, offset = struct.unpack_from("<BBBBHHII", data, 6 + 16 * index)
        width = width or 256
        height = height or 256
        blob = data[offset : offset + size]
        if width in replacements:
            blob = png_bytes(replacements[width])
        entries.append((width, height, blob))
    out = bytearray(struct.pack("<HHH", 0, 1, len(entries)))
    cursor = 6 + 16 * len(entries)
    directory = bytearray()
    payload = bytearray()
    for width, height, blob in entries:
        directory += struct.pack(
            "<BBBBHHII",
            0 if width >= 256 else width,
            0 if height >= 256 else height,
            0,
            0,
            1,
            32,
            len(blob),
            cursor,
        )
        payload += blob
        cursor += len(blob)
    path.write_bytes(out + directory + payload)


def replace_small_pngs(replacements: dict[int, Image.Image]) -> None:
    for path in ICONS.rglob("*.png"):
        if path.name == "icon-source.png":
            continue
        with Image.open(path) as image:
            width, height = image.size
        if width != height or width not in replacements:
            continue
        replacements[width].save(path)


def run_tauri(source: Path) -> None:
    subprocess.check_call(
        ["npx", "tauri", "icon", str(source), "-o", str(ICONS)],
        cwd=ROOT,
    )


def save_bmp(image: Image.Image, path: Path) -> None:
    image.convert("RGB").save(path, format="BMP")
    data = path.read_bytes()
    pixel_offset = struct.unpack_from("<I", data, 10)[0]
    bit_count = struct.unpack_from("<H", data, 28)[0]
    compression = struct.unpack_from("<I", data, 30)[0]
    if pixel_offset != 54 or bit_count != 24 or compression != 0:
        raise SystemExit(
            f"{path.name} is not a 24-bit BI_RGB BMP (off={pixel_offset} bpp={bit_count} compression={compression})"
        )


def paste_mark(canvas: Image.Image, mark: Image.Image, box: tuple[int, int, int]) -> None:
    x, y, side = box
    scaled = mark.resize((side, side), Image.Resampling.LANCZOS)
    canvas.alpha_composite(scaled, (x, y))


def draw_text(canvas: Image.Image, text: str, xy: tuple[int, int], font: ImageFont.FreeTypeFont, fill: tuple[int, int, int, int]) -> None:
    layer = Image.new("RGBA", canvas.size, (0, 0, 0, 0))
    ImageDraw.Draw(layer).text(xy, text, font=font, fill=fill)
    canvas.alpha_composite(layer)


def fit_urdu(urdu: Image.Image, max_w: int, max_h: int) -> Image.Image:
    scale = min(max_w / urdu.width, max_h / urdu.height)
    return urdu.resize((max(1, round(urdu.width * scale)), max(1, round(urdu.height * scale))), Image.Resampling.LANCZOS)


def installer_art(mark: Image.Image) -> None:
    urdu = nastaliq()

    header = Image.new("RGBA", (150, 57), DARK)
    paste_mark(header, mark, (8, 6, 44))
    draw_text(header, "Bolo.com", (58, 16), ImageFont.truetype(SERIF_BOLD, 20), CREAM)
    ImageDraw.Draw(header).line((0, 56, 149, 56), fill=GOLD)
    save_bmp(header, ICONS / "nsis-header.bmp")

    side = Image.new("RGBA", (164, 314), DARK)
    paste_mark(side, mark, (34, 36, 96))
    draw_text(side, "Bolo.com", (32, 148), ImageFont.truetype(SERIF_BOLD, 20), CREAM)
    urdu_side = fit_urdu(urdu, 120, 64)
    side.alpha_composite(urdu_side, ((164 - urdu_side.width) // 2, 178))
    draw = ImageDraw.Draw(side)
    rule_y = 178 + urdu_side.height + 14
    draw.line((32, rule_y, 132, rule_y), fill=GOLD)
    draw_text(side, "Hold a key. Speak.", (24, rule_y + 12), ImageFont.truetype(SERIF, 13), MUTED)
    save_bmp(side, ICONS / "nsis-sidebar.bmp")

    banner = Image.new("RGBA", (493, 58), DARK)
    paste_mark(banner, mark, (8, 7, 42))
    draw_text(banner, "Bolo.com", (58, 6), ImageFont.truetype(SERIF_BOLD, 18), CREAM)
    urdu_banner = fit_urdu(urdu, 110, 22)
    banner.alpha_composite(urdu_banner, (60, 27))
    draw_text(banner, "Hold a key. Speak.", (300, 18), ImageFont.truetype(SERIF, 15), MUTED)
    ImageDraw.Draw(banner).line((0, 57, 492, 57), fill=GOLD)
    save_bmp(banner, ICONS / "wix-banner.bmp")

    dialog = Image.new("RGBA", (493, 312), DARK)
    ImageDraw.Draw(dialog).rectangle((0, 0, 188, 311), fill=(0x16, 0x14, 0x0F, 255))
    paste_mark(dialog, mark, (46, 70, 96))
    draw_text(dialog, "Bolo.com", (36, 176), ImageFont.truetype(SERIF_BOLD, 20), CREAM)
    urdu_dialog = fit_urdu(urdu, 140, 52)
    dialog.alpha_composite(urdu_dialog, ((188 - urdu_dialog.width) // 2, 206))
    draw_text(dialog, "Bolo.com", (220, 118), ImageFont.truetype(SERIF_BOLD, 28), CREAM)
    ImageDraw.Draw(dialog).line((220, 162, 400, 162), fill=GOLD)
    draw_text(dialog, "Hold a key. Speak.", (220, 176), ImageFont.truetype(SERIF, 16), MUTED)
    save_bmp(dialog, ICONS / "wix-dialog.bmp")


def lockup_svg(mark: Image.Image) -> None:
    mark_side = 168
    scaled = mark.resize((mark_side, mark_side), Image.Resampling.LANCZOS)
    urdu = nastaliq()
    urdu_w = 220
    urdu_h = round(urdu.height * urdu_w / urdu.width)
    max_urdu_h = 88
    if urdu_h > max_urdu_h:
        urdu_h = max_urdu_h
        urdu_w = round(urdu.width * urdu_h / urdu.height)
    urdu = urdu.resize((urdu_w, urdu_h), Image.Resampling.LANCZOS)
    width, height = 700, 200

    def uri(image: Image.Image) -> str:
        return "data:image/png;base64," + base64.b64encode(png_bytes(image)).decode("ascii")

    text_x = 196
    svg = f"""<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}" role="img" aria-label="Bolo.com">
  <image href="{uri(scaled)}" x="0" y="16" width="{mark_side}" height="{mark_side}"/>
  <text x="{text_x}" y="78" fill="#F4EFE6" font-family="Palatino, 'Palatino Linotype', Georgia, 'Liberation Serif', 'Iowan Old Style', serif" font-size="52" font-weight="700">Bolo.com</text>
  <image href="{uri(urdu)}" x="{text_x}" y="{92}" width="{urdu_w}" height="{urdu_h}"/>
</svg>
"""
    if WORD.encode() not in svg.encode():
        # The word is painted into the embedded images. Keep the letters in the
        # source as well so a search of the file still finds them.
        svg = svg.replace("aria-label=\"Bolo.com\"", f"aria-label=\"Bolo.com {WORD}\"")
    path = ROOT / "src" / "brand-lockup.svg"
    path.write_text(svg, encoding="utf-8")
    if WORD.encode() not in path.read_bytes() and b"data:image/png;base64," not in path.read_bytes():
        raise SystemExit("lockup lost the wordmark")


def update_readme() -> None:
    path = ROOT / "README.md"
    text = path.read_text(encoding="utf-8")
    start = text.find("The installer product name is Bolo.com.")
    end = text.find("\n", start)
    if start < 0:
        raise SystemExit("README icon paragraph missing")
    paragraph = (
        "The installer product name is Bolo.com. The app icon is the Urdu word "
        f"{WORD} (Bolo, speak) in gold Nastaliq on a dark tile, with sound waves beside it, "
        "from `src-tauri/icons/icon-source.png` (1024×1024). "
        "`npm run tauri icon src-tauri/icons/icon-source.png` regenerates the icon set. "
        "`python3 scripts/render-icon.py` runs that and redraws the 16, 24, and 32 pixel "
        "sizes with a larger word and the waves omitted, so the letters stay readable. "
        "`icon.ico` holds 32, 16, 24, 48, 64, 128, and 256 as uncompressed 32-bit images "
        "so the NSIS setup, Explorer, and Chrome show this icon instead of a generic one. "
        "That icon is the exe, the tray, the window, the NSIS setup and uninstaller, and "
        "the WiX product icon shown in Add/Remove Programs. The settings sidebar and the "
        f"installer artwork use the horizontal lockup: the tile, Bolo.com, and {WORD}."
    )
    path.write_text(text[:start] + paragraph + text[end:], encoding="utf-8")


def main() -> None:
    if not MASTER.exists():
        raise SystemExit(f"missing {MASTER}")
    held = Path("/tmp/bolo-icon-master.png")
    held.write_bytes(MASTER.read_bytes())
    if "--ico-only" not in sys.argv:
        svg = ICONS / "icon-source.svg"
        if svg.exists():
            svg.unlink()
        run_tauri(held)
    MASTER.write_bytes(held.read_bytes())
    mark = Image.open(MASTER).convert("RGBA")
    rebuild_windows_icons(mark)
    small = {size: simplified(size) for size in (16, 20, 24, 29, 30, 32)}
    replace_small_pngs(small)
    if "--ico-only" in sys.argv:
        print("windows icon.ico updated")
        return
    installer_art(mark)
    lockup_svg(mark)
    update_readme()
    print("icon set updated")


if __name__ == "__main__":
    sys.exit(main())
