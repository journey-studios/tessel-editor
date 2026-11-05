#!/usr/bin/env python3
"""Generate the original procedural demo artwork shipped under the project MIT license."""

import argparse
import math
from pathlib import Path
import struct
import zlib


def chunk(kind, data):
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))


def png(path, width, height, rows):
    pixels = b"".join(b"\0" + row for row in rows)
    path.write_bytes(
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(pixels, 9))
        + chunk(b"IEND", b"")
    )


def color(values):
    return bytes(max(0, min(255, round(value))) for value in values)


def landscape(width, height, coastal):
    ridges = [
        [int(height * (base + 0.045 * math.sin(x / width * 16 + layer)
                       - 0.08 * abs(math.sin(x / width * 10 + layer))))
         for x in range(width)]
        for layer, base in enumerate((0.43, 0.54, 0.68))
    ]
    for y in range(height):
        v = y / max(1, height - 1)
        sky = (47 + 137 * v, 76 + 90 * v, 106 + 33 * v)
        river_x = width * (0.50 + 0.10 * math.sin(v * 13))
        river_w = 2 + width * max(0, v - 0.52) ** 1.7 * 0.20
        shore = width * (0.88 - 0.70 * v + 0.035 * math.sin(v * 19))
        row = bytearray()
        for x in range(width):
            u = x / max(1, width - 1)
            rgb = sky
            if coastal:
                if v > 0.35:
                    ripple = 7 * math.sin(y * 0.13 + x * 0.025)
                    rgb = (22 + ripple, 102 + ripple, 122 + ripple)
                    if x < shore:
                        ledge = int((u * 5 + v * 12) % 1 * 13)
                        rgb = (64 + ledge, 77 + ledge, 64 + ledge)
                        if v < 0.46 + 0.04 * math.sin(x * 0.009):
                            rgb = (126, 133, 78)
                    elif x - shore < 9 + 7 * math.sin(y * 0.04):
                        rgb = (191, 215, 204)
            else:
                if y >= ridges[0][x]:
                    rgb = (93, 116, 125)
                if y >= ridges[1][x]:
                    rgb = (65, 91, 91)
                if y >= ridges[2][x]:
                    rgb = (64 + 18 * u, 82 + 24 * u, 57 + 13 * u)
                if v > 0.56 and abs(x - river_x) < river_w:
                    rgb = (130 + 25 * v, 174 + 20 * v, 177 + 17 * v)
            row.extend(color(rgb))
        yield bytes(row)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=Path(__file__).resolve().parents[1] / "assets/demo")
    parser.add_argument("--width", type=int, default=1672)
    parser.add_argument("--height", type=int, default=941)
    args = parser.parse_args()
    if not 1 <= args.width <= 4096 or not 1 <= args.height <= 4096:
        parser.error("dimensions must be between 1 and 4096")
    args.output.mkdir(parents=True, exist_ok=True)
    for name, coastal in (("highlands.png", False), ("coast.png", True)):
        png(args.output / name, args.width, args.height, landscape(args.width, args.height, coastal))


if __name__ == "__main__":
    main()
