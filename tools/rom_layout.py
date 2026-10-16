#!/usr/bin/env python3
"""Regenerate address-only JP layout metadata from two user-owned dumps.

No cartridge bytes, game assets or instructions are written to the output.
This is a maintenance tool, not a build dependency. Alignment can take minutes.
"""
import argparse
from difflib import SequenceMatcher
from pathlib import Path
import zlib


def layout(us, jp):
    words = lambda data: [data[i:i + 2] for i in range(0, len(data), 2)]
    ranges = []
    for tag, start, end, source, limit in SequenceMatcher(None, words(us), words(jp)).get_opcodes():
        if tag != "equal" and end - start != limit - source:
            continue
        start, end, source = start * 2, end * 2, source * 2
        if ranges and start == ranges[-1][1] and source - start == ranges[-1][2] - ranges[-1][0]:
            ranges[-1][1] = end
        else:
            ranges.append([start, end, source])

    def canonical(address):
        for start, end, source in ranges:
            if source <= address < source + end - start:
                return start + address - source
        return address

    fields = []
    for start, end, source in ranges:
        if start >= 0x39de2:
            continue
        for at in range(start, end - 1, 2):
            reference = int.from_bytes(us[at:at + 2], "big")
            original = int.from_bytes(jp[source + at - start:source + at - start + 2], "big")
            if reference != original and reference >= 0x200 and canonical(original) == reference:
                fields.append(at)
    lines = [
        "//! Regional address correspondence. No cartridge bytes are stored here.",
        "//! Generated from verified retail dumps by tools/rom_layout.py.",
        "#[rustfmt::skip]",
        "pub(super) const JP_RANGES: &[(usize, usize, usize)] = &[",
    ]
    lines.extend(f"    (0x{start:x}, 0x{end:x}, 0x{source:x})," for start, end, source in ranges)
    lines.extend(["];", "", "#[rustfmt::skip]", "pub(super) const JP_WORD_ADDRESSES: &[usize] = &["])
    for offset in range(0, len(fields), 8):
        lines.append("    " + ", ".join(f"0x{at:x}" for at in fields[offset:offset + 8]) + ",")
    return "\n".join([*lines, "];", ""])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("us", type=Path)
    parser.add_argument("jp", type=Path)
    parser.add_argument("--output", type=Path, default=Path(__file__).resolve().parents[1] / "native/src/rom_regions_data.rs")
    args = parser.parse_args()
    images = []
    for path, checksum in [(args.us, 0x5381506f), (args.jp, 0x5b412816)]:
        data = path.read_bytes()
        if len(data) != 0x100000 or zlib.crc32(data) != checksum:
            parser.error(f"{path}: incorrect retail dump")
        images.append(data)
    args.output.write_text(layout(*images))


if __name__ == "__main__":
    main()
