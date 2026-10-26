"""Hand-placed Hangar Run scenery; coordinates are native game pixels.

Drawn by Codex CLI from a brief. Run: python3 paint_props.py (writes props.png)
No resampling, antialiasing, random values, or generated source artwork.
"""

from pathlib import Path

from PIL import Image, ImageDraw


ROOT = Path(__file__).resolve().parent
PALETTE = {
    ".": (0, 0, 0, 0),
    "K": "#000000",
    "O": "#242424",
    "S": "#484824",
    "M": "#6d6d48",
    "L": "#91916d",
    "D": "#6d6d00",
    "Y": "#b6b600",
    "R": "#914800",
    "A": "#da9148",
    "C": "#ffffda",
    "W": "#ffffff",
    "G": "#919191",
    "H": "#dadada",
    "B": "#9191b6",
    "T": "#6dffff",
}


class Prop:
    """Small integer-coordinate drawing helpers, with named palette inks."""

    def __init__(self, width, height=32):
        self.image = Image.new("RGBA", (width, height), PALETTE["."])
        self.draw = ImageDraw.Draw(self.image)

    def pixel(self, x, y, ink):
        self.draw.point((x, y), fill=PALETTE[ink])

    def rect(self, x0, y0, x1, y1, ink):
        self.draw.rectangle((x0, y0, x1, y1), fill=PALETTE[ink])

    def line(self, points, ink):
        self.draw.line(points, fill=PALETTE[ink], width=1)

    def poly(self, points, ink):
        self.draw.polygon(points, fill=PALETTE[ink])

    def dither(self, x0, y0, x1, y1, ink, phase=0):
        for y in range(y0, y1 + 1):
            for x in range(x0, x1 + 1):
                if (x + y) % 2 == phase:
                    self.pixel(x, y, ink)

    def stamp(self, x, y, rows):
        for dy, row in enumerate(rows):
            for dx, ink in enumerate(row):
                if ink != ".":
                    self.pixel(x + dx, y + dy, ink)


def bolt(p, x, y):
    # Recessed square fasteners echo the existing olive crate.
    p.rect(x, y, x + 2, y + 2, "O")
    p.line([(x, y), (x + 2, y), (x + 2, y + 1)], "G")
    p.pixel(x, y, "H")
    p.pixel(x + 1, y + 1, "L")


def drum():
    p = Prop(16)
    # Chamfered silhouette. The continuous upper edge is the standing surface.
    p.poly([(3, 0), (12, 0), (14, 2), (14, 30), (12, 31),
            (3, 31), (1, 30), (1, 2)], "O")
    p.rect(2, 4, 13, 29, "M")
    p.rect(2, 4, 3, 29, "L")
    p.dither(4, 4, 5, 29, "L")
    p.rect(11, 4, 13, 29, "S")
    p.dither(10, 4, 11, 29, "M")
    p.rect(13, 5, 13, 29, "O")
    # A closed, shallow lid, with a small recessed bung at the back right.
    p.line([(3, 1), (12, 1), (13, 2)], "G")
    p.line([(2, 2), (3, 1), (8, 1)], "H")
    p.rect(3, 2, 12, 3, "L")
    p.line([(2, 3), (3, 4), (12, 4), (13, 3)], "S")
    p.line([(3, 5), (12, 5)], "O")
    p.rect(9, 2, 10, 3, "S")
    p.pixel(9, 2, "O")
    # Two raised steel hoops, with a dark underside and a bright left edge.
    for y in (8, 23):
        p.rect(1, y, 14, y + 3, "O")
        p.rect(2, y, 12, y, "G")
        p.rect(2, y + 1, 13, y + 1, "M")
        p.rect(2, y + 2, 12, y + 2, "S")
        p.rect(2, y, 4, y, "H")
        p.rect(2, y + 1, 3, y + 1, "L")
        p.pixel(13, y, "S")
    # Miniature hazard placard: striped corners and a cream warning field.
    p.stamp(4, 13, [
        "OYYYYYYO",
        "YKYCCCCY",
        "YYKCACCY",
        "YKKCRCCY",
        "YKYCCCCY",
        "YYKCRCCY",
        "YKKCCCCY",
        "OYYYYYYO",
    ])
    # A little wear on the lower skirt, away from the main identifying label.
    p.line([(4, 28), (5, 28)], "S")
    p.pixel(10, 27, "R")
    p.pixel(11, 28, "R")
    p.line([(2, 30), (3, 29), (12, 29), (13, 30)], "G")
    p.line([(3, 30), (11, 30)], "S")
    p.line([(3, 29), (6, 29)], "L")
    return p.image


def lamp():
    p = Prop(16)
    # Alternating edge-on and open links form a short central chain.
    p.stamp(6, 0, [
        ".GO.",
        ".HO.",
        "GOOG",
        "G..O",
        ".GO.",
        ".HO.",
        "GOOG",
        "G..O",
        ".GO.",
    ])
    p.rect(6, 9, 9, 11, "O")
    p.rect(7, 9, 8, 10, "G")
    # Olive enamel shade; the lower lip overhangs the safety cage.
    p.poly([(5, 11), (10, 11), (13, 14), (14, 16),
            (1, 16), (2, 14)], "O")
    p.line([(5, 12), (10, 12), (12, 14)], "L")
    p.line([(3, 14), (4, 13), (8, 13)], "L")
    p.line([(3, 15), (11, 15)], "M")
    p.pixel(11, 14, "S")
    p.line([(2, 16), (13, 16)], "G")
    p.line([(3, 16), (6, 16)], "H")
    # Transparent gaps around a cyan glass bulb; no painted glow outside it.
    p.poly([(6, 17), (9, 17), (10, 19), (10, 25),
            (9, 27), (6, 27), (5, 25), (5, 19)], "S")
    p.rect(6, 18, 9, 26, "T")
    p.rect(6, 18, 7, 24, "C")
    p.rect(6, 19, 6, 23, "W")
    p.rect(7, 27, 8, 28, "G")
    # Heavy external cage, two rails, and a small rounded bottom shoe.
    p.line([(3, 17), (3, 27), (5, 30), (10, 30),
            (12, 27), (12, 17)], "O")
    p.line([(4, 17), (4, 27), (5, 29), (10, 29), (11, 27),
            (11, 17)], "G")
    p.line([(4, 18), (4, 23)], "H")
    for y in (21, 26):
        p.rect(3, y, 12, y + 1, "O")
        p.line([(4, y), (11, y)], "G")
        p.line([(4, y), (6, y)], "H")
    p.line([(5, 30), (10, 30)], "S")
    p.line([(6, 31), (9, 31)], "O")
    return p.image


def vent():
    p = Prop(32)
    p.poly([(3, 0), (28, 0), (30, 2), (30, 29), (28, 31),
            (3, 31), (1, 29), (1, 2)], "O")
    p.rect(2, 2, 29, 29, "M")
    p.line([(3, 1), (28, 1), (29, 2)], "H")
    p.line([(2, 2), (2, 28)], "G")
    p.line([(3, 2), (28, 2)], "G")
    p.line([(29, 3), (29, 29), (3, 29)], "S")
    p.line([(3, 30), (28, 30)], "S")
    p.dither(6, 27, 25, 28, "S")
    # Double inset bevel and a nearly black fan well.
    p.rect(5, 5, 26, 26, "S")
    p.line([(5, 26), (26, 26), (26, 5)], "G")
    p.rect(6, 6, 25, 25, "K")
    p.line([(6, 6), (25, 6)], "O")
    p.line([(6, 6), (6, 25)], "O")
    # Four bent fan blades, shown behind the grille, with subtle olive shade.
    blades = [
        [(15, 14), (11, 11), (11, 8), (16, 8), (18, 10), (17, 14)],
        [(17, 15), (20, 11), (23, 11), (23, 16), (21, 18), (17, 17)],
        [(16, 17), (20, 20), (20, 23), (15, 23), (13, 21), (14, 17)],
        [(14, 16), (11, 20), (8, 20), (8, 15), (10, 13), (14, 14)],
    ]
    for blade in blades:
        p.poly(blade, "M")
    p.line([(12, 9), (16, 9), (17, 10)], "L")
    p.line([(21, 12), (22, 12), (22, 16)], "L")
    p.line([(19, 21), (19, 22), (15, 22)], "S")
    p.line([(9, 19), (9, 15), (10, 14)], "L")
    for x, y in ((12, 10), (13, 11), (14, 12), (19, 13),
                 (20, 14), (21, 15), (17, 19), (18, 20), (16, 21),
                 (10, 16), (11, 17), (10, 18)):
        p.pixel(x, y, "S")
    p.poly([(14, 13), (17, 13), (19, 15), (19, 17), (17, 19),
            (14, 19), (12, 17), (12, 15)], "O")
    p.rect(14, 14, 17, 17, "G")
    p.line([(14, 14), (17, 14)], "G")
    p.pixel(14, 15, "L")
    p.pixel(16, 16, "O")
    # Foreground steel bars are continuous over the fan and its hub.
    for x in (9, 21):
        p.rect(x, 6, x + 1, 25, "O")
        p.rect(x, 7, x, 24, "G")
        p.pixel(x, 7, "H")
    for y in (11, 20):
        p.line([(6, y + 1), (25, y + 1)], "K")
        p.line([(6, y), (25, y)], "M")
        for x in (9, 21):
            p.pixel(x, y, "G")
    for x, y in ((3, 3), (26, 3), (3, 26), (26, 26)):
        bolt(p, x, y)
    p.line([(9, 3), (14, 3)], "L")
    p.line([(22, 28), (24, 28)], "R")
    return p.image


def panel():
    p = Prop(32)
    p.poly([(3, 0), (28, 0), (30, 2), (30, 29), (28, 31),
            (3, 31), (1, 29), (1, 2)], "O")
    p.rect(2, 2, 29, 29, "M")
    p.line([(3, 1), (28, 1)], "G")
    p.line([(2, 2), (2, 28)], "L")
    p.line([(3, 2), (27, 2)], "L")
    p.line([(29, 3), (29, 29), (3, 29)], "S")
    p.line([(3, 30), (28, 30)], "S")
    # Inset door seam and frame, matching the crate's olive construction.
    p.rect(4, 5, 27, 26, "S")
    p.rect(5, 6, 26, 25, "M")
    p.line([(5, 25), (26, 25), (26, 6)], "L")
    # Recessed CRT: sparse cyan phosphor, no outside glow or smooth shading.
    p.rect(6, 7, 21, 16, "O")
    p.line([(6, 16), (21, 16), (21, 7)], "G")
    p.rect(7, 8, 20, 15, "K")
    p.line([(8, 9), (11, 9)], "T")
    p.line([(8, 10), (9, 10)], "S")
    p.line([(8, 13), (10, 13), (11, 11), (12, 14), (13, 12),
            (15, 12), (16, 10), (17, 13), (19, 13)], "T")
    p.pixel(8, 9, "C")
    p.line([(8, 15), (12, 15)], "S")
    p.line([(16, 15), (19, 15)], "S")
    # Three indicator lamps in a separate vertical bank.
    for y, ink, gleam in ((7, "A", "C"), (11, "Y", "C"),
                           (15, "G", "W")):
        p.rect(23, y, 25, y + 2, "O")
        p.rect(23, y, 24, y + 1, ink)
        p.pixel(23, y, gleam)
    # Three toggles: pale metal levers offset from dark recessed slots.
    for x, raised in ((7, True), (13, False), (19, True)):
        p.rect(x, 19, x + 3, 23, "S")
        p.rect(x + 1, 20, x + 2, 23, "O")
        y = 19 if raised else 21
        p.rect(x + 1, y, x + 2, y + 1, "G")
        p.pixel(x + 1, y, "H")
        p.pixel(x, 24, "L")
    # Right-hand latch and two small hinge blocks.
    p.rect(25, 20, 26, 23, "O")
    p.line([(25, 20), (25, 22)], "G")
    for y in (8, 22):
        p.rect(1, y, 3, y + 3, "O")
        p.rect(1, y, 2, y + 2, "G")
        p.pixel(1, y, "H")
    # The bottom apron has quiet stippled shading and a yellow ID strip.
    p.dither(7, 27, 23, 28, "S")
    p.line([(11, 27), (18, 27)], "D")
    p.line([(11, 27), (13, 27)], "Y")
    for x, y in ((3, 3), (26, 3), (3, 26), (26, 26)):
        bolt(p, x, y)
    return p.image


def main():
    sheet = Image.new("RGBA", (96, 32), PALETTE["."])
    for x, prop in ((0, drum()), (16, lamp()), (32, vent()), (64, panel())):
        sheet.paste(prop, (x, 0))
    sheet.save(ROOT / "props.png")


if __name__ == "__main__":
    main()
