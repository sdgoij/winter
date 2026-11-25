"""Decodes a frame from `winter-frame` and writes it as a PNG.

Debug helper for the demo: the protocol is a u32 big-endian width, height and
byte length followed by that many RGBA bytes.
"""

import struct
import sys
import zlib


def chunk(tag: bytes, body: bytes) -> bytes:
    return (
        struct.pack(">I", len(body))
        + tag
        + body
        + struct.pack(">I", zlib.crc32(tag + body) & 0xFFFFFFFF)
    )


def main(source: str, target: str) -> None:
    with open(source, "rb") as handle:
        data = handle.read()

    kind, width, height, cell_width, cell_height, length = struct.unpack(">IIIIII", data[:24])
    if kind != 0:
        raise SystemExit(f"not a frame: kind {kind}")
    pixels = data[24 : 24 + length]
    assert len(pixels) == width * height * 4, (len(pixels), width, height)

    stride = width * 4
    raw = b"".join(
        b"\x00" + pixels[row * stride : (row + 1) * stride] for row in range(height)
    )

    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(raw, 9))
    png += chunk(b"IEND", b"")

    with open(target, "wb") as handle:
        handle.write(png)

    distinct = len({pixels[i : i + 3] for i in range(0, len(pixels), 4)})
    print(f"{width}x{height}, cell {cell_width}x{cell_height}, {distinct} distinct colours")


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
