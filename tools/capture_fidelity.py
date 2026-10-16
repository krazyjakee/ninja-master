#!/usr/bin/env python3
"""Capture numeric gameplay evidence from a player-supplied US ROM.

Requires a Genesis Plus GX libretro shared library exporting work_ram and
m68k_get_reg/set_reg/run (the upstream development build does).
No emulator, ROM, disassembly, sound or image data is copied to the repository.
Usage: python3 tools/capture_fidelity.py --core /path/to/core.so --rom /path/to/US.bin --out out/fidelity
"""
import argparse
import csv
import ctypes as C
import hashlib
import itertools
import json
from pathlib import Path

ENV = C.CFUNCTYPE(C.c_bool, C.c_uint, C.c_void_p)
VIDEO = C.CFUNCTYPE(None, C.c_void_p, C.c_uint, C.c_uint, C.c_size_t)
SAMPLE = C.CFUNCTYPE(None, C.c_int16, C.c_int16)
BATCH = C.CFUNCTYPE(C.c_size_t, C.c_void_p, C.c_size_t)
POLL = C.CFUNCTYPE(None)
INPUT = C.CFUNCTYPE(C.c_int16, C.c_uint, C.c_uint, C.c_uint, C.c_uint)

class GameInfo(C.Structure):
    _fields_ = [("path", C.c_char_p), ("data", C.c_void_p), ("size", C.c_size_t), ("meta", C.c_char_p)]

class Original:
    def __init__(self, core, rom, directory):
        self.core = C.CDLL(str(core.resolve()), mode=C.RTLD_GLOBAL)
        self.directory = str(directory.resolve()).encode()
        self.options = {b"genesis_plus_gx_region_detect": b"auto", b"genesis_plus_gx_bios": b"disabled"}
        self.held = 0
        self.deadline = 2_000_000
        self.callbacks = [ENV(self.environment), VIDEO(lambda *args: None), SAMPLE(lambda *args: None),
                          BATCH(lambda data, count: count), POLL(lambda: None), INPUT(self.input)]
        for name, callback in zip(["environment", "video_refresh", "audio_sample", "audio_sample_batch", "input_poll", "input_state"], self.callbacks):
            getattr(self.core, "retro_set_" + name)(callback)
        self.core.retro_init()
        self.buffer = C.create_string_buffer(rom.read_bytes())
        info = GameInfo(str(rom.resolve()).encode(), C.cast(self.buffer, C.c_void_p), len(rom.read_bytes()), None)
        self.core.retro_load_game.argtypes = [C.POINTER(GameInfo)]
        if not self.core.retro_load_game(C.byref(info)):
            raise RuntimeError("Core could not load cartridge")
        self.core.m68k_get_reg.restype = C.c_uint
        self.core.m68k_set_reg.argtypes = [C.c_int, C.c_uint]
        self.core.m68k_run.argtypes = [C.c_uint]
        self.base = C.addressof(C.c_uint8.in_dll(self.core, "work_ram"))
        self.ram = (C.c_uint8 * 65536).from_address(self.base)

    def environment(self, command, data):
        if command in (9, 31):
            C.cast(data, C.POINTER(C.c_char_p))[0] = self.directory
            return True
        if command in (2, 3):
            C.cast(data, C.POINTER(C.c_bool))[0] = command == 3
            return True
        if command == 10:
            return True
        if command == 17:
            C.cast(data, C.POINTER(C.c_bool))[0] = False
            return True
        if command == 15:
            pair = C.cast(data, C.POINTER(C.c_char_p))
            value = self.options.get(pair[0])
            if value:
                pair[1] = value
                return True
        return False

    def input(self, port, device, index, button):
        if port or device != 1:
            return 0
        # native tape Left/Right/Up/Down/C/B/Start/A -> RetroPad IDs
        ids = [6, 7, 4, 5, 8, 0, 3, 1]
        if button == 256:
            return sum(1 << key for bit, key in enumerate(ids) if self.held & (1 << bit))
        return int(any(button == key and self.held & (1 << bit) for bit, key in enumerate(ids)))

    def frame(self, held):
        self.held = held
        self.core.retro_run()

    def read(self, address, size=2, signed=False):
        return int.from_bytes(bytes(self.ram[((address+i) & 65535) ^ 1] for i in range(size)), "big", signed=signed)

    def poke(self, address, value, size=2):
        for i, byte in enumerate((value & ((1 << (size*8))-1)).to_bytes(size, "big")):
            self.ram[((address+i) & 65535) ^ 1] = byte

    def call(self, pc, registers):
        # Return into a two-byte BRA-to-self in unused work RAM. Stop after
        # enough master cycles for the measured routine, without a video tick.
        self.poke(0xFFF400, 0xFFF420, 4)
        self.poke(0xFFF420, 0x60FE)
        for register, value in {**registers, 17: 0x2700, 15: 0xFFF400, 16: pc}.items():
            self.core.m68k_set_reg(register, value)
        self.core.m68k_run(self.deadline)
        self.deadline += 10000
        if self.core.m68k_get_reg(16) != 0xFFF420:
            raise RuntimeError(f"Measured routine did not return: PC={self.core.m68k_get_reg(16):x}")


def write_csv(path, header, rows):
    with path.open("w", newline="") as stream:
        writer = csv.writer(stream, lineterminator="\n")
        writer.writerow(header)
        writer.writerows(rows)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--core", type=Path, required=True)
    parser.add_argument("--rom", type=Path, required=True)
    parser.add_argument("--out", type=Path, default=Path("out/fidelity"))
    args = parser.parse_args()
    if hashlib.sha1(args.rom.read_bytes()).hexdigest() != "1e07d7998e3048fcfba4238ae96496460e91b3a5":
        parser.error("The raw US retail cartridge is required for these code addresses")
    args.out.mkdir(parents=True, exist_ok=True)
    original = Original(args.core, args.rom, args.out)
    boot = ([0]*50 + [0x40]*6)*6
    for frame in range(560):
        original.frame(boot[frame] if frame < len(boot) else 0)
    player = original.read(0xFF5EC2, 4)
    if original.read(0xFF002A, 4) != 0xAEDA or original.read(player+0x24) != 496:
        raise RuntimeError("Boot did not reach the initial forest stance")
    rows = []
    fired = False
    for tick in range(110):
        vy = original.read(player+0x1C, 4, signed=True)
        held = 16 if tick < 15 else 0
        if tick > 15 and not fired and abs(vy) < 0x17000:
            held = 16
            fired = True
        original.frame(held)
        rows.append([tick, held, original.read(player+0x20, 4), original.read(player+0x24, 4),
                     original.read(player+0x1C, 4, signed=True), original.read(player+0x2C), original.read(player+0x2E)])
    write_csv(args.out/"double_jump.csv", ["tick", "held", "x16", "y16", "vy16", "state", "cursor"], rows)
    rows = []
    target = 0xFF5000
    for dx, dy, down, left, same_priority in itertools.product(
            [-65,-64,-63,-17,-16,0,16,17,63,64,65], [-81,-80,-17,-16,0,16,63,64], [0,1], [0,1], [0,1]):
        original.poke(target+0x20, 256+dx)
        original.poke(target+0x24, 256+dy)
        original.poke(target+8, 0 if same_priority else 0x8000)
        original.poke(player+8, 0x800 if left else 0)
        original.poke(0xFF377E, down)
        original.call(0x22810, {8:player, 9:target, 10:0, 12:0xFF377C, 4:0x8000 if left else 0, 5:0x7FFF, 6:256, 7:256})
        distance = original.core.m68k_get_reg(5) & 65535
        rows.append([dx,dy,down,left,same_priority,distance])
    write_csv(args.out/"melee_boundary.csv", ["dx","dy","down","left","same_priority","distance"], rows)
    metadata = {"rom_sha1": hashlib.sha1(args.rom.read_bytes()).hexdigest(),
                "core_sha256": hashlib.sha256(args.core.read_bytes()).hexdigest(),
                "motion_frames":110, "boundary_cases":len(rows),
                "motion_units":"16.16 pixels, one original NTSC video frame per row",
                "boundary_routine":"Player_MeasureEnemy $22810, called before the state handler"}
    (args.out/"capture.json").write_text(json.dumps(metadata, indent=2)+"\n")
    print(json.dumps(metadata, indent=2))
    original.core.retro_unload_game()
    original.core.retro_deinit()

if __name__ == "__main__":
    main()
