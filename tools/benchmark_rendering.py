#!/usr/bin/env python3
"""Run and summarize the renderer benchmark; no ROM is required.

python3 tools/benchmark_rendering.py --out out/rendering --frames 60
Add --rom /path/to/game.bin for real stage samples, or --summarize DIR
to summarize an existing run. Timing data and procedural artwork stay in DIR.
"""
import argparse
import csv
import json
import math
import os
import statistics
import subprocess
from collections import defaultdict
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
TIMINGS = ("compose_ms", "resolve_ms", "upload_ms", "submit_ms", "gpu_wait_ms", "present_ms", "frame_ms")


def percentile(values, fraction):
    ordered = sorted(values)
    return ordered[max(0, math.ceil(len(ordered) * fraction) - 1)]


def summarize(directory):
    groups = defaultdict(list)
    with (directory / "frames.csv").open(newline="") as stream:
        for row in csv.DictReader(stream):
            key = tuple(row[name] for name in ("scene", "viewport_width", "viewport_height", "view", "art", "path"))
            groups[key].append(row)
    if not groups:
        raise ValueError("Benchmark has no measured samples")
    results = []
    for key, rows in groups.items():
        scene, width, height, view, art, path = key
        result = dict(scene=scene, width=int(width), height=int(height), view=view, art=art, path=path, samples=len(rows))
        for name in TIMINGS:
            values = [float(row[name]) for row in rows]
            if any(not math.isfinite(value) or value < 0 for value in values):
                raise ValueError(f"Invalid timing in {key}: {name}")
            result[name] = {"median": round(statistics.median(values), 6), "p95": round(percentile(values, .95), 6), "max": max(values)}
        result["upload_bytes_per_frame"] = statistics.mean(int(row["upload_bytes"]) for row in rows)
        result["atlas_init_bytes"] = int(rows[0]["atlas_init_bytes"])
        result["draw_calls"] = int(rows[0]["draw_calls"])
        results.append(result)
    report = {"metadata": json.loads((directory / "metadata.json").read_text()),
              "comparisons": json.loads((directory / "comparisons.json").read_text()), "results": results}
    (directory / "summary.json").write_text(json.dumps(report, indent=2) + "\n")
    columns = ["scene", "width", "height", "view", "art", "path", "samples", "upload_bytes_per_frame", "atlas_init_bytes", "draw_calls"]
    columns += [f"{name}_{stat}" for name in TIMINGS for stat in ("median", "p95", "max")]
    with (directory / "summary.csv").open("w", newline="") as stream:
        writer = csv.DictWriter(stream, fieldnames=columns, lineterminator="\n")
        writer.writeheader()
        for result in results:
            row = {name: result[name] for name in columns if name in result}
            row.update({f"{name}_{stat}": result[name][stat] for name in TIMINGS for stat in ("median", "p95", "max")})
            writer.writerow(row)
    print("scene / viewport / view / art / path: median frame ms, p95 ms, uploaded bytes")
    for result in results:
        print(f"{result['scene']} / {result['width']}x{result['height']} / {result['view']} / {result['art']} / {result['path']}: "
              f"{result['frame_ms']['median']:.3f}, {result['frame_ms']['p95']:.3f}, {result['upload_bytes_per_frame']:.0f}")
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, default=ROOT / "out/rendering")
    parser.add_argument("--frames", type=int, default=60)
    parser.add_argument("--rom", type=Path)
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--summarize", type=Path)
    args = parser.parse_args()
    if args.summarize:
        summarize(args.summarize)
        return
    if not 1 <= args.frames <= 10000:
        parser.error("--frames must be 1..10000")
    binary = args.binary
    if binary is None:
        subprocess.run(["cargo", "build", "--release", "--locked", "--manifest-path", str(ROOT / "native/Cargo.toml")], check=True)
        metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--no-deps", "--format-version", "1", "--manifest-path", str(ROOT / "native/Cargo.toml")]))
        binary = Path(metadata["target_directory"]) / "release/ninja-master-native"
        if os.name == "nt":
            binary = binary.with_suffix(".exe")
    command = [str(binary.resolve()), "--render-benchmark", str(args.out.resolve()), "--frames", str(args.frames)]
    if args.rom:
        command += ["--rom", str(args.rom.resolve())]
    subprocess.run(command, check=True, cwd=ROOT)
    summarize(args.out)


if __name__ == "__main__":
    main()
