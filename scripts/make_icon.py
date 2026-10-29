#!/usr/bin/env python3
"""Draw the Bolo app icon: a dark tile, a tally lamp, and a short waveform."""

import struct
import zlib
from pathlib import Path

SIZE = 1024


def chunk(tag: bytes, data: bytes) -> bytes:
    return (
        struct.pack(">I", len(data))
        + tag
        + data
        + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
    )


def in_round_rect(x: int, y: int, x0: int, y0: int, x1: int, y1: int, radius: int) -> bool:
    if x < x0 or y < y0 or x >= x1 or y >= y1:
        return False
    if x0 + radius <= x < x1 - radius or y0 + radius <= y < y1 - radius:
        return True
    corners = (
        (x0 + radius, y0 + radius),
        (x1 - radius - 1, y0 + radius),
        (x0 + radius, y1 - radius - 1),
        (x1 - radius - 1, y1 - radius - 1),
    )
    for cx, cy in corners:
        if abs(x - cx) <= radius and abs(y - cy) <= radius:
            dx, dy = x - cx, y - cy
            return dx * dx + dy * dy <= radius * radius
    return False


def main() -> None:
    pixels = bytearray(SIZE * SIZE * 4)
    bg = (23, 22, 15, 255)
    lamp = (255, 75, 62, 255)
    cream = (244, 240, 230, 255)
    for y in range(SIZE):
        for x in range(SIZE):
            i = (y * SIZE + x) * 4
            if in_round_rect(x, y, 48, 48, SIZE - 48, SIZE - 48, 190):
                pixels[i : i + 4] = bytes(bg)
    def disc(cx: int, cy: int, radius: int, color: tuple[int, int, int, int]) -> None:
        r2 = radius * radius
        for y in range(cy - radius, cy + radius + 1):
            for x in range(cx - radius, cx + radius + 1):
                if 0 <= x < SIZE and 0 <= y < SIZE and (x - cx) ** 2 + (y - cy) ** 2 <= r2:
                    i = (y * SIZE + x) * 4
                    if pixels[i + 3]:
                        pixels[i : i + 4] = bytes(color)

    def bar(cx: int, cy: int, width: int, height: int) -> None:
        radius = width // 2
        x0, x1 = cx - radius, cx + radius
        y0, y1 = cy - height // 2, cy + height // 2
        for y in range(y0, y1):
            for x in range(x0, x1):
                if in_round_rect(x, y, x0, y0, x1, y1, radius):
                    i = (y * SIZE + x) * 4
                    if pixels[i + 3]:
                        pixels[i : i + 4] = bytes(cream)

    disc(390, 512, 108, lamp)
    disc(362, 476, 28, (255, 196, 186, 255))
    bar(640, 512, 52, 180)
    bar(730, 512, 52, 320)
    bar(820, 512, 52, 230)

    raw = b"".join(b"\x00" + pixels[y * SIZE * 4 : (y + 1) * SIZE * 4] for y in range(SIZE))
    png = (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", SIZE, SIZE, 8, 6, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(raw, 9))
        + chunk(b"IEND", b"")
    )
    out = Path("/tmp/bolo-icon.png")
    out.write_bytes(png)
    print(out)


if __name__ == "__main__":
    main()
