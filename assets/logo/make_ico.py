#!/usr/bin/env python3
"""Pack PNG files into a Windows .ico (PNG-compressed entries, Vista+).
Usage: make_ico.py out.ico in16.png in24.png ..."""
import struct
import sys


def png_size(data):
    assert data[:8] == b"\x89PNG\r\n\x1a\n", "not a PNG"
    return struct.unpack(">II", data[16:24])


out, inputs = sys.argv[1], sys.argv[2:]
images = [open(p, "rb").read() for p in inputs]
header = struct.pack("<HHH", 0, 1, len(images))
offset = 6 + 16 * len(images)
entries, blobs = b"", b""
for data in images:
    w, h = png_size(data)
    # 256 is stored as 0 in the one-byte width/height fields.
    entries += struct.pack("<BBBBHHII", w % 256, h % 256, 0, 0, 1, 32, len(data), offset)
    blobs += data
    offset += len(data)
with open(out, "wb") as f:
    f.write(header + entries + blobs)
