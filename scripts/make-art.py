#!/usr/bin/env python3
"""Draws the logo and the README's feature icons, as pixel art in the PICO-8 palette.

  scripts/make-art.py    writes docs/assets/
"""
import os
import struct
import zlib

ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "docs", "assets")

# PICO-8, by the characters the sprites below use. A space is see-through.
PALETTE = {
    "k": (0x00, 0x00, 0x00),
    "n": (0x1D, 0x2B, 0x53),
    "v": (0x7E, 0x25, 0x53),
    "g": (0x00, 0x87, 0x51),
    "b": (0xAB, 0x52, 0x36),
    "d": (0x5F, 0x57, 0x4F),
    "l": (0xC2, 0xC3, 0xC7),
    "w": (0xFF, 0xF1, 0xE8),
    "r": (0xFF, 0x00, 0x4D),
    "o": (0xFF, 0xA3, 0x00),
    "y": (0xFF, 0xEC, 0x27),
    "t": (0x00, 0xE4, 0x36),
    "c": (0x29, 0xAD, 0xFF),
    "i": (0x83, 0x76, 0x9C),
    "p": (0xFF, 0x77, 0xA8),
    "f": (0xFF, 0xCC, 0xAA),
    "m": (0x29, 0x36, 0x6F),  # The band across the logo's top.
    "x": (0x0B, 0x0B, 0x14),  # Near black, for the window.
}


def write_png(path, rows, scale):
    """Writes rows of palette characters, with each pixel a square of `scale`."""
    raw = bytearray()
    for row in rows:
        line = bytearray([0])
        for ch in row:
            rgba = PALETTE[ch] + (255,) if ch != " " else (0, 0, 0, 0)
            line.extend(bytes(rgba) * scale)
        raw.extend(bytes(line) * scale)

    def chunk(tag, data):
        body = tag + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    size = (len(rows[0]) * scale, len(rows) * scale)
    header = struct.pack(">IIBBBBB", size[0], size[1], 8, 6, 0, 0, 0)
    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(bytes(raw), 9)) + chunk(b"IEND", b""))


def logo():
    """A window of tiles: the panes of a tuios session, as a mosaic."""
    g = [["n"] * 32 for _ in range(32)]

    def box(x, y, w, h, ch):
        for r in range(y, y + h):
            for c in range(x, x + w):
                g[r][c] = ch

    box(0, 0, 32, 2, "m")
    # The window, with its corners cut.
    box(3, 5, 26, 24, "x")
    for x, y in [(3, 5), (28, 5), (3, 28), (28, 28)]:
        g[y][x] = "n"
    # Its title bar and three lights.
    box(4, 6, 24, 3, "n")
    for x, ch in [(6, "r"), (9, "o"), (12, "t")]:
        g[7][x] = ch

    def tile(x, y, w, h, face, light, dark):
        box(x, y, w, h, face)
        box(x, y, w, 1, light)
        box(x, y, 1, h, light)
        box(x, y + h - 1, w, 1, dark)
        box(x + w - 1, y, 1, h, dark)
        g[y][x + w - 1] = face
        g[y + h - 1][x] = face

    tile(5, 10, 12, 17, "r", "p", "v")
    tile(18, 10, 9, 8, "o", "y", "b")
    tile(18, 19, 4, 8, "t", "w", "g")
    tile(23, 19, 4, 8, "c", "w", "n")
    # A prompt and a cursor in the big tile.
    box(7, 13, 2, 1, "w")
    box(10, 13, 4, 1, "f")
    box(7, 16, 5, 1, "f")
    box(7, 19, 3, 1, "f")
    box(11, 19, 1, 2, "w")
    return ["".join(r) for r in g]


ICONS = {
    "panes": """
................
.xxxxxxxxxxxxxx.
.xrrrrrrxooooox.
.xrpppprxoyyyox.
.xrrrrrrxoooooX.
.xrrrrrrxxxxxxx.
.xrpprrrxtttttx.
.xrrrrrrxtwwttx.
.xrrrrrrxtttttx.
.xrpppprxxxxxxx.
.xrrrrrrxcccccx.
.xrrrrrrxcwwccx.
.xrrrrrrxcccccx.
.xxxxxxxxxxxxxx.
................
................
""",
    "agents": """
................
...t........t...
....t......t....
...tttttttttt...
..tttttttttttt..
..ttkkttttkktt..
.tttkkttttkkttt.
.tttttttttttttt.
.t.tttttttttt.t.
.t.t........t.t.
.t.t........t.t.
....ttt..ttt....
................
................
................
................
""",
    "inbox": """
................
................
..oooooooooooo..
.oyyyyyyyyyyyyo.
.ooyyyyyyyyyyoo.
.oboyyyyyyyyobo.
.obboyyyyyyobbo.
.obbboyyyyobbbo.
.obbbboyyobbbbo.
.obbbbboobbbbbo.
.obbbbbbbbbbbbo.
.oooooooooooooo.
..........rr....
.........rrrr...
..........rr....
................
""",
    "review": """
................
.lllllllll......
.lwwwwwwwl......
.lwttwttwl......
.lwwwwwwwl.ccc..
.lwrrwrrwlcwwwc.
.lwwwwwwwcwwwwwc
.lwttttwwcwwwwwc
.lwwwwwwwcwwwwwc
.lwrrwwwwlcwwwc.
.lwwwwwwwl.cccd.
.lwttwttwl....dd
.lwwwwwwwl.....d
.lllllllll......
................
................
""",
    "files": """
................
................
..ooooo.........
.oyyyyyo........
.oyyyyyoooooooo.
.obbbbbbbbbbbbo.
.oyyyyyyyyyyyyo.
.oyyyyyyyyyyyyo.
.oyyyyyyyyyyyyo.
.oyyyyyyyyyyyyo.
.oyyyyyyyyyyyyo.
.oyyyyyyyyyyyyo.
.oooooooooooooo.
................
................
................
""",
    "markdown": """
................
................
.llllllllllllll.
.lxxxxxxxxxxxxl.
.lxwxxxwxxxyxxl.
.lxwwxwwxxxyxxl.
.lxwxwxwxxxyxxl.
.lxwxxxwxxxyxxl.
.lxwxxxwxyyyyyl.
.lxwxxxwxxyyyxl.
.lxwxxxwxxxyxxl.
.lxxxxxxxxxxxxl.
.llllllllllllll.
................
................
................
""",
    "mouse": """
................
.....wwwwww.....
....wlllwwww....
....wlllwwww....
....wlrlwwww....
....wlrlwwww....
....wlllwwww....
....wwwwwwww....
....wwwwwwww....
....wwwwwwww....
....wwwwwwww....
....wwwwwwww....
....wwwwwwww....
.....wwwwww.....
................
................
""",
    "keys": """
................
................
.iiiiii..iiiiii.
.iwwwwi..iwwwwi.
.iwrrwi..iwttwi.
.iwrrwi..iwttwi.
.iwwwwi..iwwwwi.
.iiiiii..iiiiii.
.dddddd..dddddd.
................
.iiiiiiiiiiiiii.
.iwwwwwwwwwwwwi.
.iwwwwwwwwwwwwi.
.iiiiiiiiiiiiii.
.dddddddddddddd.
................
""",
    "machines": """
................
.ccccccc........
.cnnnnnc........
.cnttnnc........
.cnnnnnc........
.ccccccc........
...ddd...y......
..ddddd.yyy.....
.........y......
.....y...ooooooo
....yyy..onnnnno
.....y...onrrnno
.........onnnnno
.........ooooooo
...........ddd..
..........ddddd.
""",
    "themes": """
................
.....ooooooo....
...oowwwwwwwoo..
..owwwwwwwwwwwo.
..owrrwwwwttwwo.
.owwrrwwwwttwwwo
.owwwwwwwwwwwwwo
.owwccwwwwwwwwwo
.owwccwwwwwwoooo
.owwwwwwwwwo....
..owwyywwwwo....
..owwyywwwo.....
...oowwwwoo.....
.....oooo.......
................
................
""",
}


def icon(text):
    rows = [line for line in text.strip("\n").split("\n")]
    width = max(len(r) for r in rows)
    return [r.ljust(width, ".").replace(".", " ").replace("X", "x") for r in rows]


def main():
    os.makedirs(ROOT, exist_ok=True)
    art = logo()
    write_png(os.path.join(ROOT, "icon.png"), art, 8)
    write_png(os.path.join(ROOT, "favicon.png"), art, 2)
    for name, text in ICONS.items():
        write_png(os.path.join(ROOT, f"icon-{name}.png"), icon(text), 4)


if __name__ == "__main__":
    main()
