"""Generate the Hangar Run layout from 2-2's warehouse modules.

Run: python3 generate.py hangar.txt legend.json, then copy legend.json into
expansion.json's legend.
"""
import json, sys

W = 112
grid = [['00'] * W for _ in range(32)]

def put(row, col, m):
    if 0 <= row < 32 and 0 <= col < W and m is not None:
        grid[row][col] = m

def art(row, col, chars):
    """Place new art cells (layout characters) row by row from (row, col)."""
    for r, line in enumerate(chars):
        for c, ch in enumerate(line):
            put(row + r, col + c, 'art:' + ch)

def crate(col, slab, high=1):
    """Cargo crates stacked `high` on the slab at `slab`."""
    for n in range(high):
        art(slab - 2 - 2 * n, col, ['<>', '{}'])

def floor(c0, c1):
    for c in range(c0, c1):
        put(31, c, '7e' if c % 2 == 0 else '7f')

def support(c, slab):
    """A support column under a walkway whose slab is at row `slab`."""
    put(slab - 2, c + 1, '5b'); put(slab - 2, c + 2, '5c')
    for i, m in enumerate(['57', '58', '59', '5a']):
        put(slab - 1, c + i, m)
    for r in range(slab, 31):
        for i, m in enumerate(['53', '54', '55', '56']):
            put(r, c + i, m)
    for i, m in enumerate(['4f', '50', '51', '52']):
        put(31, c + i, m)

def railing(c0, c1, slab):
    """Railing pairs between columns c0..c1 (exclusive), above `slab`."""
    pattern = 'ABBA'
    for k, c in enumerate(range(c0, c1, 2)):
        top, bottom = (('28', '29'), ('2a', '2b')) if pattern[k % 4] == 'A' else (('2e', '2f'), ('30', '31'))
        put(slab - 2, c, top[0]); put(slab - 2, c + 1, top[1])
        put(slab - 1, c, bottom[0]); put(slab - 1, c + 1, bottom[1])

def walkway(c0, c1, slab):
    """Posts at c0 and c1 - 1, a solid slab between them."""
    assert (c1 - c0 - 2) % 2 == 0
    put(slab - 3, c0, '32'); put(slab - 2, c0, '33'); put(slab - 1, c0, '34')
    put(slab - 3, c1 - 1, '35'); put(slab - 2, c1 - 1, '36'); put(slab - 1, c1 - 1, '37')
    railing(c0 + 1, c1 - 1, slab)
    for c in range(c0, c1):
        put(slab, c, '3c')

def pillar(c, bottom, phase=0, top=0):
    """A pillar hanging from `top` whose body ends at row `bottom`."""
    for r in range(top, bottom + 1):
        band = (r % 4) == phase
        for i in range(4):
            put(r, c + i, ['5f', '60', '61', '62'][i] if band else ['53', '54', '55', '56'][i])
    for i, m in enumerate(['63', '64', '65', '66']):
        put(bottom + 1, c + i, m)
    put(bottom + 2, c + 1, '5d'); put(bottom + 2, c + 2, '5e')

def ledge(c0, pc, slab):
    """A one-way railed platform from c0 into the pillar at pc (on its left)."""
    assert (pc - c0 - 1) % 2 == 0
    put(slab - 3, c0, '32'); put(slab - 2, c0, '33'); put(slab - 1, c0, '34')
    railing(c0 + 1, pc, slab)
    put(slab, c0, '38')
    for c in range(c0 + 1, pc):
        put(slab, c, '39')
    put(slab, pc, '68')

# A: the start walkway under a hanging pillar.
walkway(0, 22, 30)
support(22, 30)
pillar(8, 13, phase=1)
# B: pit, columns 26..31.
# C: a raised walkway with a one-way ledge above, fixed to a pillar.
support(32, 29)
walkway(36, 58, 29)
support(58, 29)
pillar(54, 23, phase=2)
ledge(43, 54, 23)
# D: pit, columns 62..66.
# E: a long low walkway under a pillar.
support(67, 30)
walkway(71, 91, 30)
support(91, 30)
pillar(78, 15, phase=3)
# F: pit, columns 95..99.
# G: the last raised walkway to the exit.
support(100, 29)
walkway(104, 112, 29)
pillar(106, 9, phase=0)
# New art: crates to hop and climb, hazard signs on the wall.
crate(12, 30)
crate(40, 29, high=2)
crate(76, 30)
crate(78, 30, high=2)
art(23, 3, ['[]'])
art(20, 62, ['[]'])
art(23, 96, ['[]'])

chars = {}
pool = iter('.abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789#=+*')
lines = []
for row in grid:
    line = ''
    for m in row:
        if m.startswith('art:'):
            line += m[4:]
            continue
        if m not in chars:
            chars[m] = next(pool)
        line += chars[m]
    lines.append(line)
open(sys.argv[1], 'w').write('\n'.join(lines) + '\n')
json.dump({v: k.upper() for k, v in chars.items()}, open(sys.argv[2], 'w'))
print('\n'.join(lines))
