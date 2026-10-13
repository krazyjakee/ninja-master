#!/usr/bin/env python3
"""Regenerate the original, ROM-independent app icon using only the stdlib."""
import pathlib
import struct
import zlib

ROOT = pathlib.Path(__file__).resolve().parents[1] / "native/resources"
# All coordinates are in a 128-square viewbox. The SVG and raster formats
# share this geometry; no game graphics or external renderer are involved.
SHAPES = [
    ("circle", (64, 64, 62), "#1b2030"),
    ("circle", (64, 64, 54), "#bd3047"),
    ("polygon", [(23, 114), (30, 91), (48, 80), (81, 80), (98, 94), (106, 114)], "#111624"),
    ("polygon", [(89, 51), (119, 35), (110, 62), (95, 69)], "#ee5360"),
    ("polygon", [(92, 58), (119, 74), (106, 81), (90, 70)], "#bd3047"),
    ("circle", (64, 60, 36), "#111624"),
    ("polygon", [(31, 48), (95, 48), (98, 58), (30, 58)], "#ee5360"),
    ("polygon", [(33, 61), (60, 65), (64, 69), (68, 65), (94, 61), (89, 76), (39, 76)], "#f5e4c9"),
    ("polygon", [(41, 65), (57, 68), (54, 71), (43, 70)], "#111624"),
    ("polygon", [(71, 68), (87, 65), (85, 70), (74, 71)], "#111624"),
    ("polygon", [(38, 78), (90, 78), (79, 93), (49, 93)], "#242d42"),
]


def inside(kind, shape, x, y):
    if kind == "circle":
        cx, cy, radius = shape
        return (x - cx) ** 2 + (y - cy) ** 2 <= radius ** 2
    hit = False
    ax, ay = shape[-1]
    for bx, by in shape:
        if (ay > y) != (by > y) and x < (bx - ax) * (y - ay) / (by - ay) + ax:
            hit = not hit
        ax, ay = bx, by
    return hit


def raster(size):
    pixels = bytearray()
    colors = [tuple(bytes.fromhex(color[1:])) for _, _, color in SHAPES]
    samples = 2 if size <= 256 else 1
    for row in range(size):
        for col in range(size):
            sums = [0, 0, 0, 0]
            for sy in range(samples):
                for sx in range(samples):
                    x = (col + (sx + .5) / samples) * 128 / size
                    y = (row + (sy + .5) / samples) * 128 / size
                    for (kind, shape, _), color in reversed(list(zip(SHAPES, colors))):
                        if inside(kind, shape, x, y):
                            for channel, value in enumerate((*color, 255)):
                                sums[channel] += value
                            break
            count = samples ** 2
            alpha = sums[3] // count
            # Unpremultiply the RGB at the transparent edge.
            rgb = [value * 255 // sums[3] if sums[3] else 0 for value in sums[:3]]
            pixels.extend((*rgb, alpha))
    return bytes(pixels)


def chunk(kind, data):
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))


def png(size, pixels):
    rows = b"".join(b"\0" + pixels[y * size * 4:(y + 1) * size * 4] for y in range(size))
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(rows, 9)) + chunk(b"IEND", b""))


def main():
    ROOT.mkdir(parents=True, exist_ok=True)
    svg = ['<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 128 128">',
           '<title>Ninja Master</title>']
    for kind, shape, color in SHAPES:
        if kind == "circle":
            x, y, r = shape
            svg.append(f'<circle cx="{x}" cy="{y}" r="{r}" fill="{color}"/>')
        else:
            points = " ".join(f"{x},{y}" for x, y in shape)
            svg.append(f'<polygon points="{points}" fill="{color}"/>')
    (ROOT / "ninja-master.svg").write_text("\n".join(svg + ["</svg>", ""]))
    images = {}
    for size in (16, 32, 48, 64, 128, 256, 512, 1024):
        pixels = raster(size)
        images[size] = png(size, pixels)
        (ROOT / f"ninja-master-{size}.png").write_bytes(images[size])
        if size == 64:
            # SDL loads BMP without the optional SDL_image library.
            rgb = bytearray()
            for row in range(size - 1, -1, -1):
                for col in range(size):
                    r, g, b, a = pixels[(row * size + col) * 4:(row * size + col + 1) * 4]
                    rgb.extend((b * a // 255, g * a // 255, r * a // 255))
            header = struct.pack("<2sIHHI", b"BM", 54 + len(rgb), 0, 0, 54)
            dib = struct.pack("<IiiHHIIiiII", 40, size, size, 1, 24, 0, len(rgb), 0, 0, 0, 0)
            (ROOT / "ninja-master.bmp").write_bytes(header + dib + rgb)
    sizes = (16, 32, 48, 64, 128, 256)
    ico = bytearray(struct.pack("<HHH", 0, 1, len(sizes)))
    offset = 6 + 16 * len(sizes)
    for size in sizes:
        data = images[size]
        ico.extend(struct.pack("<BBBBHHII", size % 256, size % 256, 0, 0, 1, 32, len(data), offset))
        offset += len(data)
    (ROOT / "ninja-master.ico").write_bytes(ico + b"".join(images[size] for size in sizes))
    body = b""
    for kind, size in ((b"ic07", 128), (b"ic08", 256), (b"ic09", 512), (b"ic10", 1024)):
        body += kind + struct.pack(">I", len(images[size]) + 8) + images[size]
    (ROOT / "ninja-master.icns").write_bytes(b"icns" + struct.pack(">I", len(body) + 8) + body)


if __name__ == "__main__":
    main()
