"""Paint Hangar Run's new pieces in 2-2's palette line 3.

Run: python3 paint.py art.png preview.png (needs Pillow).
"""
from PIL import Image
import sys

C = {  # palette line 3
    'k': (0x24, 0x24, 0x24), 'K': (0, 0, 0), 'o': (0xda, 0x91, 0x48), 'c': (0xff, 0xff, 0xda),
    'L': (0x91, 0x91, 0x6d), 'M': (0x6d, 0x6d, 0x48), 'D': (0x48, 0x48, 0x24), 'g': (0x91, 0x91, 0x91),
    'G': (0xda, 0xda, 0xda), 'y': (0xb6, 0xb6, 0x00), 'Y': (0x6d, 0x6d, 0x00), 'r': (0x91, 0x48, 0x00),
}
sheet = Image.new('RGBA', (64, 32), (0, 0, 0, 0))

def px(x, y, c):
    sheet.putpixel((x, y), C[c] + (255,))

# Cargo crate, 32x32 at (0, 0).
for y in range(32):
    for x in range(32):
        if x in (0, 31) or y in (0, 31):
            c = 'k'
        elif x in (1,) or y in (1,):
            c = 'L'
        elif x == 30 or y == 30:
            c = 'D'
        elif y % 8 == 0:
            c = 'D'  # plank seams
        elif y % 8 == 1:
            c = 'L'  # plank highlight
        elif y >= 22 and (x + y) % 2 == 0:
            c = 'D'  # dithered shade toward the floor
        else:
            c = 'M'
        px(x, y, c)
# Steel corner brackets with rivets.
for cx, cy in ((1, 1), (25, 1), (1, 25), (25, 25)):
    for y in range(6):
        for x in range(6):
            edge = x in (0, 5) or y in (0, 5)
            px(cx + x, cy + y, 'k' if edge and (x, y) not in ((0, 0),) else 'g')
    px(cx + 2, cy + 2, 'G'); px(cx + 3, cy + 3, 'k')
# Stencil plate: yellow with dark marks.
for y in range(12, 20):
    for x in range(9, 23):
        edge = x in (9, 22) or y in (12, 19)
        px(x, y, 'k' if edge else 'y')
for x in range(11, 21, 3):
    px(x, 14, 'Y'); px(x + 1, 14, 'Y'); px(x, 15, 'Y')
for x in range(11, 21):
    if x % 4 != 2:
        px(x, 17, 'Y')

# Hazard sign, 32x16 at (32, 0).
for y in range(16):
    for x in range(32):
        X = 32 + x
        if x in (0, 31) or y in (0, 15):
            c = 'k'
        elif (x + y) // 3 % 2 == 0:
            c = 'K'
        else:
            c = 'y'
        px(X, y, c)
for y in range(3, 13):
    for x in range(10, 22):
        edge = x in (10, 21) or y in (3, 12)
        px(32 + x, y, 'k' if edge else 'c')
for y in (5, 6, 7, 8):  # the warning mark
    px(32 + 15, y, 'o'); px(32 + 16, y, 'r')
px(32 + 15, 10, 'o'); px(32 + 16, 10, 'r')

sheet.save(sys.argv[1])
sheet.resize((256, 128), Image.NEAREST).save(sys.argv[2])
