# Rendering measurements and backend decision

Issue: [#30](https://github.com/krazyjakee/ninja-master/issues/30).

## Adopted desktop changes

The existing OpenGL/SDL backend is retained. Each changed game frame uploads
only to its active destination: SDL for flat output, or the CRT texture for CRT
output. Changing effects, display size, artwork or focus invalidates the upload,
including while gameplay is paused. A failed CRT upload or draw disables the
effect and uploads the current frame to SDL before displaying it.

OpenGL uploads save, normalize and restore unpack alignment, row length, row and
pixel offsets, and pixel-buffer binding. This matters when mixing SDL texture
updates with raw GL: a stride inherited from a differently sized texture can
otherwise corrupt an atlas or CRT upload. Shader compilation failures clean up
their resources. Renderer-reset events rebuild the game texture, CRT resources,
font and preview textures while preserving game and ImGui state.

F2 separates CPU composition, RGB resolution, transfer, submission and presentation
costs. Headless rendering, save thumbnails/checksums, simulation and replay formats
continue to use the existing CPU renderer.

## Reproduce

From the repository root, with Rust, Python 3, SDL2 and a desktop OpenGL display:

```sh
python3 tools/benchmark_rendering.py --out out/rendering --frames 60
python3 tools/benchmark_rendering.py --out out/rendering-us --frames 60 --rom /path/to/shinobi3_us.bin
```

The runner builds a release executable. `--binary PATH` uses an existing build.
The equivalent executable option is `--render-benchmark DIR --frames N`; this
diagnostic mode does not load or alter the player's settings, saves or progress.
It opens a benchmark window; closing the window cancels the run.

The procedural matrix covers 1920×1080, 2560×1440, 3840×2160 and 3440×1440,
original/widescreen views, native art and a generated 10× RGBA replacement pack.
The pack exercises detailed texels, tinting and partial transparency through the
real HD renderer, with output scale chosen by the normal viewport-sizing code.
Optional ROM samples run stage parts 1-1, 3-2 and 4-3 using the supplied cartridge.
They sample the opening frames of these parts, rather than full playthroughs.
No cartridge assets or images are included in the published results.

Each workload runs five warmup frames followed by the requested measured samples:

| Path | Purpose |
| --- | --- |
| `flat` | CPU composition and one SDL texture upload |
| `legacy-crt` | Reproduce the former SDL upload plus CRT upload |
| `single-crt` | CPU composition and only the CRT upload |
| `indexed-batch` | Experimental native indexed atlas, palette lookup and ordered geometry |

`frames.csv` retains individual samples. `metadata.json` records hardware, SDL/GL
versions, revision, compiled renderer-source fingerprint and run conditions.
The revision is checkout HEAD; the fingerprint identifies the compiled renderer
snapshot, including local uncommitted changes.
`comparisons.json` records native-resolution GPU/CPU comparisons; any mismatch
aborts the benchmark and writes procedural diagnostic PNGs. The Python runner
writes `summary.json` and `summary.csv` with medians, nearest-rank p95 and maxima
for every phase. Existing results can be summarized with:

```sh
python3 tools/benchmark_rendering.py --summarize out/rendering
```

Timings are CPU wall times. `gpu_wait_ms` explicitly finishes queued rendering
and GPU work before the separately measured swap. This is a serialized comparison,
not a measurement of pipelined gameplay FPS or GPU timer-query execution time.
All workloads use the same large drawable with the requested viewport inside it.
Texture byte counts exclude geometry submission, driver overhead, font/preview
textures and screenshots. The atlas is initialized once; its byte count is
reported separately. Procedural command generation and game simulation are
outside the measured rendering interval for all paths. Scheduler/compositor
stalls remain visible in the distributions.

## Backend and migration decision

The checked-in [CSV baseline](baseline-linux-nvidia.csv),
[host/run metadata](baseline-linux-nvidia.json) and
[pixel-comparison counts](baseline-linux-nvidia-comparisons.json) contain a Linux
run on a Ryzen 9 5900X / RTX 3090 using NVIDIA 610.43.02. It includes 60 measured
frames per workload after warmup, with other desktop activity visible in p95/max
timings. Representative serialized frame medians are:

| Procedural workload | Former CRT uploads | Single CRT upload | Texture bytes/frame, before → after |
| --- | ---: | ---: | ---: |
| 1080p widescreen, native | 1.063 ms | 0.909 ms | 534,912 → 267,456 |
| 4K widescreen, native | 1.167 ms | 1.033 ms | 534,912 → 267,456 |
| 4K widescreen, 10× HD | 103.569 ms | 91.895 ms | 53,491,200 → 26,745,600 |

The native 4K widescreen atlas prototype measures 0.237 ms for its own supported
fixture, with 43,008 palette bytes per frame and a cached 2,048-byte tile atlas.
It excludes CRT processing and HD replacement support; compare it with the
corresponding `flat` row rather than treating it as a whole-game CRT speedup.
Every recorded native GPU/CPU comparison has zero differing colour channels.

The immediate optimization needs no new dependency or graphics API. Staying with
SDL's compatibility OpenGL context keeps the existing window, pacing, CRT shader,
capture and ImGui integrations. A wgpu migration would also require replacing
those integrations before delivering a comparable baseline; it is deferred until
there is a measured platform or rendering requirement that justifies that cost.

The experimental compositor caches 4-bit tile patterns in a single-byte atlas,
uploads a per-scanline RGB palette texture, and submits ordered tile/sprite
geometry in one logical batch. It supports transparent index zero, flips,
offscreen clipping, row masks, clipping below a display line, palette animation,
mid-frame palette splits and overlapping layers in caller-supplied painter order.
Native-resolution comparisons use the independent `Frame` software renderer.
The prototype is benchmark-only: it is not a complete encoder for game scenes.

Measurements demonstrate the value of eliminating duplicate transfers and allow
the atlas experiment to be evaluated separately. The generated full-coverage HD
workload also exposes CPU composition and RGB-resolution costs that a transfer
optimization alone cannot remove. A whole-game GPU speedup is not claimed.

Further composition migration has explicit fidelity gates:

1. Encode real plane and sprite commands, including SAT masks and high-priority
   plane/sprite interactions, and compare every migrated feature against `Frame`.
2. Add GPU representation for wave sampling, palette fades and raster changes.
   Retain CPU fallback for any unsupported frame before making a GPU path selectable.
3. Add HD atlas alpha, tinting, animated replacements, atlas update/invalidation
   and filtering tests, then measure the full-coverage HD workload again.
4. Integrate render snapshots/high-refresh presentation independently of the
   60 Hz simulation; reuse the measured upload and submission boundaries.
5. Reassess backend requirements with hardware results on Windows, Linux and
   both supported macOS architectures before adopting a new graphics API.

## Validation

Regular Rust tests preserve the CPU rendering and replay reference. Cartridge
integration checks require local supported US/EU/JP ROMs. Desktop GL regressions
are ignored by default and run serially under Xvfb/Mesa in Linux CI:

```sh
LIBGL_ALWAYS_SOFTWARE=1 xvfb-run -a -s '-screen 0 1280x960x24' \
  cargo test --locked --manifest-path native/Cargo.toml desktop:: -- --ignored --test-threads=1
```

The fullscreen test needs a window manager, as configured in the build workflow.
Coverage includes exact GPU/CPU atlas comparisons, unchanged native/HD CRT output,
inactive-texture poisoning, effect toggles without simulation ticks, failure
fallback, unpack-buffer/stride restoration, bad shader compilation, screenshot
capture, rebuilt UI textures and repeated fullscreen transitions. The regular
test suite runs on Windows, Linux and both macOS architectures in CI.

Published numeric baselines identify their test host. Linux NVIDIA measurements
and Mesa software-renderer checks do not establish Windows/macOS driver performance
or physical GPU-loss recovery on those systems; those require platform runs.
