#!/usr/bin/env python3
"""Capture the original cave boss's damage gates using a user-supplied ROM.

Uses the same development Genesis Plus GX core as capture_fidelity.py.
Only numeric observations are written; no cartridge data is exported.
"""
import argparse
import ctypes as C
import hashlib
import itertools
import json
from pathlib import Path

from capture_fidelity import Original, write_csv


def capture(original):
    boot = ([0] * 50 + [0x40] * 6) * 6
    for frame in range(560):
        original.frame(boot[frame] if frame < len(boot) else 0)
    baseline = bytes(original.ram)
    player = original.read(0xFF5EC2, 4)
    boss, attack = 0xFF5000, 0xFF5100
    rows = []
    # Walk, decide, high wind-up/recovery, low attack, stance, charge.
    routines = [(4, 0), (8, 0), (12, 0), (12, 3), (16, 0), (24, 0), (28, 0)]
    # Throw, slash, dive, dash kick, Kariu: player state and list-3 type.
    attacks = [(7, 0), (5, 0x24), (20, 0x3C), (29, 0x4C), (65, 0x50)]
    for (routine, phase), (state, kind), flip in itertools.product(routines, attacks, [0, 0x800]):
        C.memmove(original.base, baseline, len(baseline))
        for address in range(boss, boss + 0x200):
            original.poke(address, 0, 1)
        original.poke(0xFF5EC8, attack, 4)
        for offset, value in [(0, 0x8000), (4, routine), (0xA, 0x23D),
                              (0x20, 160), (0x24, 496), (0x2A, 15), (0x2E, 33)]:
            original.poke(boss + offset, value)
        for offset, value in [(0x32, phase), (0x34, 40), (0x36, 50), (0x43, 11)]:
            original.poke(boss + offset, value, 1)
        original.poke(attack, 0x8000)
        original.poke(attack + 2, kind)
        original.poke(attack + 0x50, 0xFFFFFFFF, 4)
        original.poke(attack + 0x20, 160)
        original.poke(attack + 0x24, 456)
        original.poke(attack + 0x12, 8, 1)
        original.poke(attack + 0x13, 8, 1)
        original.poke(player + 8, flip)
        original.poke(player + 0x20, 100)
        original.poke(player + 0x24, 496)
        original.poke(player + 0x2C, state)
        # Allow the complete collision/deflection/allocator path to return.
        original.deadline += 100000
        original.call(0x2A518, {8: boss, 11: player})
        rows.append([routine, phase, state, flip, kind,
                     11 - original.read(boss + 0x43, 1)])
    return rows


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--core", type=Path, required=True)
    parser.add_argument("--rom", type=Path, required=True)
    parser.add_argument("--out", type=Path, default=Path("out/fidelity"))
    args = parser.parse_args()
    rom_sha1 = hashlib.sha1(args.rom.read_bytes()).hexdigest()
    if rom_sha1 != "1e07d7998e3048fcfba4238ae96496460e91b3a5":
        parser.error("The raw US retail cartridge is required for these code addresses")
    args.out.mkdir(parents=True, exist_ok=True)
    original = Original(args.core, args.rom, args.out)
    try:
        rows = capture(original)
        write_csv(args.out / "cave_boss_hits.csv",
                  ["routine", "phase", "player_state", "player_flip", "attack_type", "damage"], rows)
        metadata = {"rom_sha1": rom_sha1,
                    "core_sha256": hashlib.sha256(args.core.read_bytes()).hexdigest(),
                    "cases": len(rows), "routine": "Boss1 $2A518, isolated handler calls"}
        (args.out / "cave_boss_capture.json").write_text(json.dumps(metadata, indent=2) + "\n")
        print(json.dumps(metadata, indent=2))
    finally:
        original.core.retro_unload_game()
        original.core.retro_deinit()


if __name__ == "__main__":
    main()
