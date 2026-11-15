"""Generates assets/lumen.ico from the same geometry as src/icons.rs.

The window icon and the icon Explorer shows for lumen.exe have to look the
same, so the shape is duplicated here on purpose: the Rust side paints it at
runtime into an egui texture, and this script rasterises it into a real .ico
file that the build script embeds into the executable.

Run it after changing the logo:

    python tools/make_icon.py
"""

import struct
import zlib
from pathlib import Path

# Sizes stored in the .ico. Windows picks the closest one for the taskbar,
# the title bar, Explorer's small and large views and the Alt-Tab switcher.
SIZES = [16, 24, 32, 48, 64, 128, 256]

# Samples per axis for anti-aliasing. 4 is enough to look smooth at 16 px.
SUPERSAMPLE = 4

# Gradient end points, identical to icons.rs.
TOP = (0x3B, 0x82, 0xF6)
BOTTOM = (0x1C, 0xB6, 0xA8)


def lerp(a: int, b: int, t: float) -> float:
    return a + (b - a) * t


def in_triangle(x: float, y: float, a, b, c) -> bool:
    """Barycentric point-in-triangle test, same maths as icons.rs."""
    def sign(p, q, r):
        return (p[0] - r[0]) * (q[1] - r[1]) - (q[0] - r[0]) * (p[1] - r[1])

    d1 = sign((x, y), a, b)
    d2 = sign((x, y), b, c)
    d3 = sign((x, y), c, a)
    has_neg = d1 < 0 or d2 < 0 or d3 < 0
    has_pos = d1 > 0 or d2 > 0 or d3 > 0
    return not (has_neg and has_pos)


def sample(fx: float, fy: float):
    """Colour and coverage of the logo at a normalised position."""
    nx = abs(fx - 0.5) * 2.0
    ny = abs(fy - 0.5) * 2.0
    distance = (nx ** 4 + ny ** 4) ** 0.25
    mask = min(max((1.0 - distance) / 0.05, 0.0), 1.0)
    if mask <= 0.0:
        return (0.0, 0.0, 0.0, 0.0)

    t = min(max(fx * 0.55 + fy * 0.45, 0.0), 1.0)
    colour = [lerp(TOP[0], BOTTOM[0], t), lerp(TOP[1], BOTTOM[1], t), lerp(TOP[2], BOTTOM[2], t)]
    alpha = 255.0

    # Sun.
    if ((fx - 0.34) ** 2 + (fy - 0.33) ** 2) ** 0.5 < 0.105:
        colour, alpha = [255.0, 255.0, 255.0], 235.0

    # Two overlapping mountains.
    if in_triangle(fx, fy, (0.08, 0.80), (0.40, 0.38), (0.72, 0.80)) or in_triangle(
        fx, fy, (0.40, 0.80), (0.66, 0.50), (0.94, 0.80)
    ):
        colour, alpha = [255.0, 255.0, 255.0], 225.0

    return (colour[0], colour[1], colour[2], alpha * mask)


def render(size: int):
    """Returns RGBA bytes, top-down, anti-aliased with supersampling."""
    pixels = bytearray(size * size * 4)
    step = 1.0 / (size * SUPERSAMPLE)
    for y in range(size):
        for x in range(size):
            # Accumulate premultiplied colour so transparent samples do not
            # darken the edge of the squircle.
            r = g = b = a = 0.0
            for sy in range(SUPERSAMPLE):
                for sx in range(SUPERSAMPLE):
                    fx = (x * SUPERSAMPLE + sx + 0.5) * step
                    fy = (y * SUPERSAMPLE + sy + 0.5) * step
                    sr, sg, sb, sa = sample(fx, fy)
                    r += sr * sa
                    g += sg * sa
                    b += sb * sa
                    a += sa
            count = SUPERSAMPLE * SUPERSAMPLE
            if a > 0.0:
                index = (y * size + x) * 4
                pixels[index] = int(round(r / a))
                pixels[index + 1] = int(round(g / a))
                pixels[index + 2] = int(round(b / a))
                pixels[index + 3] = int(round(a / count))
    return bytes(pixels)


def dib_frame(size: int, rgba: bytes) -> bytes:
    """A classic 32-bit bottom-up bitmap with an empty AND mask."""
    header = struct.pack(
        "<IiiHHIIiiII",
        40,          # biSize
        size,        # biWidth
        size * 2,    # biHeight: XOR image plus AND mask
        1,           # biPlanes
        32,          # biBitCount
        0,           # biCompression (BI_RGB)
        size * size * 4,
        0, 0, 0, 0,
    )
    rows = []
    for y in range(size - 1, -1, -1):
        row = bytearray()
        for x in range(size):
            i = (y * size + x) * 4
            row += bytes((rgba[i + 2], rgba[i + 1], rgba[i], rgba[i + 3]))
        rows.append(bytes(row))
    # AND mask: one bit per pixel, rows padded to four bytes. All zero means
    # "use the alpha channel", which is what 32-bit icons want.
    mask_row = ((size + 31) // 32) * 4
    mask = bytes(mask_row * size)
    return header + b"".join(rows) + mask


def png_frame(size: int, rgba: bytes) -> bytes:
    """A PNG frame, allowed in .ico files since Windows Vista (used for 256)."""
    def chunk(tag: bytes, data: bytes) -> bytes:
        return (
            struct.pack(">I", len(data))
            + tag
            + data
            + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
        )

    raw = bytearray()
    for y in range(size):
        raw.append(0)  # filter: none
        raw += rgba[y * size * 4 : (y + 1) * size * 4]
    ihdr = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", ihdr)
        + chunk(b"IDAT", zlib.compress(bytes(raw), 9))
        + chunk(b"IEND", b"")
    )


def main() -> None:
    frames = []
    for size in SIZES:
        rgba = render(size)
        # 256 px icons are stored as PNG because a 256 px bitmap alone is
        # 256 kB; everything smaller stays a plain bitmap for compatibility.
        data = png_frame(size, rgba) if size >= 256 else dib_frame(size, rgba)
        frames.append((size, data))

    offset = 6 + 16 * len(frames)
    directory = b""
    payload = b""
    for size, data in frames:
        # Width and height are one byte each; 256 is encoded as 0.
        directory += struct.pack(
            "<BBBBHHII",
            size if size < 256 else 0,
            size if size < 256 else 0,
            0,    # colour palette size
            0,    # reserved
            1,    # colour planes
            32,   # bits per pixel
            len(data),
            offset,
        )
        payload += data
        offset += len(data)

    out = Path(__file__).resolve().parent.parent / "assets" / "lumen.ico"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_bytes(struct.pack("<HHH", 0, 1, len(frames)) + directory + payload)
    print(f"wrote {out} ({out.stat().st_size} bytes, sizes {SIZES})")


if __name__ == "__main__":
    main()
