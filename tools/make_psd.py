"""Writes test-images/artwork.psd, a small PSD with a real layer-less layout.

The file is built the way Photoshop writes one: header, three skipped sections,
then the composite image data, RLE compressed. Lumen only reads that composite,
so this is a complete and representative fixture even without layers.

Run:  python tools/make_psd.py
"""

import struct
from pathlib import Path

WIDTH = 320
HEIGHT = 240


def packbits_literal(data: bytes) -> bytes:
    """Encodes bytes as literal PackBits runs of at most 128 bytes."""
    out = bytearray()
    for start in range(0, len(data), 128):
        chunk = data[start : start + 128]
        out.append(len(chunk) - 1)
        out += chunk
    return bytes(out)


def build() -> bytes:
    red, green, blue, alpha = [], [], [], []
    for y in range(HEIGHT):
        for x in range(WIDTH):
            # A soft diagonal gradient with a bright disc in the middle.
            red.append(int(30 + 180 * x / WIDTH))
            green.append(int(40 + 120 * y / HEIGHT))
            blue.append(int(200 - 90 * (x + y) / (WIDTH + HEIGHT)))
            dx, dy = x - WIDTH / 2, y - HEIGHT / 2
            alpha.append(255 if dx * dx + dy * dy < 70 * 70 else 128)

    planes = [bytes(red), bytes(green), bytes(blue), bytes(alpha)]

    out = bytearray()
    out += b"8BPS"
    out += struct.pack(">H", 1)          # version
    out += b"\0" * 6                     # reserved
    out += struct.pack(">H", len(planes))  # channels
    out += struct.pack(">I", HEIGHT)
    out += struct.pack(">I", WIDTH)
    out += struct.pack(">H", 8)          # bits per channel
    out += struct.pack(">H", 3)          # colour mode: RGB

    for _ in range(3):                   # colour mode data, resources, layers
        out += struct.pack(">I", 0)

    out += struct.pack(">H", 1)          # compression: RLE
    row = packbits_literal(planes[0][:WIDTH])
    for _ in planes:                     # one byte count per channel per row
        for _ in range(HEIGHT):
            out += struct.pack(">H", len(row))
    for plane in planes:
        for y in range(HEIGHT):
            out += packbits_literal(plane[y * WIDTH : (y + 1) * WIDTH])
    return bytes(out)


def main() -> None:
    out = Path(__file__).resolve().parent.parent / "test-images" / "artwork.psd"
    out.write_bytes(build())
    print(f"wrote {out} ({out.stat().st_size} bytes, {WIDTH}x{HEIGHT}, RGBA, RLE)")


if __name__ == "__main__":
    main()
