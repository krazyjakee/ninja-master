#!/bin/sh
# Package an already-built native executable into a per-platform release archive.
#
#   tools/package.sh linux   BINARY [--out DIR] [--version VER] [--arch ARCH]
#   tools/package.sh macos   BINARY [--out DIR] [--version VER] [--arch ARCH]
#   tools/package.sh windows BINARY.exe --sdl-dll SDL2.dll [--sdl-license LICENSE.txt]
#                            [--out DIR] [--version VER] [--arch ARCH]
#
# Produces DIR (default obj/dist)/ninja-master-VER-PLATFORM-ARCH.tar.gz (Linux, macOS) or .zip (Windows)
# holding the executable, a launcher, README.txt, LICENSE-NOTES.txt and the native
# manual. Cartridge images are supplied by the user and excluded from packages.
set -eu

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
python_cmd=${PYTHON:-python3}

usage() {
    sed -n '4,7p' "$0" | sed 's/^# \{0,1\}//' >&2
    exit 2
}

[ "$#" -ge 2 ] || usage
platform=$1
binary=$2
shift 2

out_dir="$project_root/obj/dist"
version=""
arch=""
sdl_dll=""
sdl_license=""
while [ "$#" -gt 0 ]; do
    case $1 in
        --out) out_dir=${2:?--out needs a directory}; shift 2 ;;
        --version) version=${2:?--version needs a value}; shift 2 ;;
        --arch) arch=${2:?--arch needs a value}; shift 2 ;;
        --sdl-dll) sdl_dll=${2:?--sdl-dll needs a path}; shift 2 ;;
        --sdl-license) sdl_license=${2:?--sdl-license needs a path}; shift 2 ;;
        *) usage ;;
    esac
done

case $platform in
    linux|macos) exe_name=ninja-master-native; default_arch=$(uname -m) ;;
    windows) exe_name=ninja-master-native.exe; default_arch=x86_64 ;;
    *) echo "error: platform must be linux, macos or windows" >&2; exit 2 ;;
esac
[ -n "$arch" ] || arch=$default_arch

if [ ! -f "$binary" ]; then
    echo "error: executable not found: $binary" >&2
    exit 1
fi
if [ "$platform" = windows ] && [ ! -f "$sdl_dll" ]; then
    echo "error: Windows packages need --sdl-dll PATH/SDL2.dll" >&2
    exit 1
fi
if [ "$platform" = windows ] && [ ! -f "$sdl_license" ]; then
    echo "error: Windows packages need --sdl-license PATH/LICENSE.txt" >&2
    exit 1
fi

if [ -z "$version" ]; then
    cargo_version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$project_root/native/Cargo.toml" | head -n 1)
    revision=$(git -C "$project_root" rev-parse --short HEAD 2>/dev/null || echo unknown)
    version="$cargo_version-$revision"
fi

name="ninja-master-$version-$platform-$arch"
# These values form paths below out_dir. Reject traversal before removing staging.
"$python_cmd" - "$version" "$arch" <<'EOF'
import re, sys
for value in sys.argv[1:]:
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._+-]*", value):
        sys.exit("error: version and arch must be simple filename components")
EOF
mkdir -p -- "$out_dir"
out_dir=$(CDPATH= cd -- "$out_dir" && pwd)
stage="$out_dir/$name"
rm -rf -- "$stage"
mkdir -p -- "$stage"

cp -- "$binary" "$stage/$exe_name"
chmod 755 "$stage/$exe_name"
cp -- "$project_root/README.md" "$stage/MANUAL.md"
cp -- "$project_root/LICENSE" "$stage/LICENSE"
mkdir -p -- "$stage/assets"
cp -- "$project_root/assets/banner.svg" "$stage/assets/banner.svg"
"$python_cmd" "$project_root/tools/package_licenses.py" "$stage" "$project_root"

if [ "$platform" = windows ]; then
    cp -- "$sdl_dll" "$stage/SDL2.dll"
    if [ -n "$sdl_license" ]; then
        cp -- "$sdl_license" "$stage/LICENSE-SDL2.txt"
    fi
    # Batch files keep CRLF line endings.
    awk '{ printf "%s\r\n", $0 }' > "$stage/play.bat" <<'EOF'
@echo off
setlocal
rem The executable discovers the ROM or opens its startup file picker.
"%~dp0ninja-master-native.exe" --boot %*
exit /b %errorlevel%
EOF
    launcher=play.bat
    run_example='play.bat'
    direct_example='ninja-master-native.exe --boot --rom C:\path\to\shinobi3_us.bin'
    verify_example='certutil -hashfile shinobi3_us.bin SHA1
  (or PowerShell: Get-FileHash -Algorithm SHA1 shinobi3_us.bin)'
else
    cat > "$stage/play.sh" <<'EOF'
#!/bin/sh
# The executable discovers the ROM or opens its startup file picker.
set -eu
dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
exec "$dir/ninja-master-native" --boot "$@"
EOF
    chmod 755 "$stage/play.sh"
    launcher=play.sh
    run_example='./play.sh'
    direct_example='./ninja-master-native --boot --rom /path/to/shinobi3_us.bin'
    if [ "$platform" = macos ]; then
        verify_example='shasum -a 1 shinobi3_us.bin'
        direct_example='"./Ninja Master.app/Contents/MacOS/ninja-master-native" --boot --rom /path/to/shinobi3_us.bin'
    else
        verify_example='sha1sum shinobi3_us.bin'
    fi
fi

if [ "$platform" = windows ]; then
    sdl_note='SDL2.dll (SDL 2, zlib license, see LICENSE-SDL2.txt) must stay next to the .exe.'
elif [ "$platform" = macos ]; then
    sdl_note='The application bundles SDL2. No Homebrew installation is needed.
Choose the Intel (x86_64) or Apple Silicon (arm64) download for your Mac.'
else
    sdl_note='The executable links the system SDL2 library; install it first
(Debian/Ubuntu: "sudo apt install libsdl2-2.0-0", Fedora: "sudo dnf install SDL2").'
fi

cat > "$stage/README.txt" <<EOF
Ninja Master native port ($version, $platform $arch)
=================================================

A native PC port of Shinobi III: Return of the Ninja Master (Sega Mega Drive /
Genesis, 1993), complete and playable from start to finish across all seven
rounds. Full playthroughs have been tested on Linux and Windows.

YOU MUST SUPPLY YOUR OWN ROM
----------------------------
This package contains no game data. The executable decodes graphics, stage
data, music and sound from the original US cartridge image at start-up, and
refuses any other image. Provide a dump of your own cartridge:

  Region  USA ("Shinobi III - Return of the Ninja Master")
  Size    1,048,576 bytes (1 MiB, plain big-endian .bin; no header, not interleaved)
  CRC32   5381506f
  SHA-1   1e07d7998e3048fcfba4238ae96496460e91b3a5

Check it with:
  $verify_example

The European and Japanese images are not accepted.

How the ROM is found
--------------------
The $launcher launcher runs --boot (from the SEGA logo). The executable checks
--rom FILE, then NINJA_MASTER_ROM, then shinobi3_us.bin beside the executable
(or beside the macOS .app), a remembered selection, and the source checkout.
If the ROM is missing or invalid, desktop startup shows a warning and file
picker. A valid selection is remembered next to settings.ini as rom-path.txt.
Cancel exits cleanly. Headless runs report an error without opening a picker.

Run the executable directly with an explicit --rom for other modes:
  $direct_example
  ... --assets --rom FILE    original asset viewer
  ... --sound --rom FILE     music/sound demo
  ... --demo                 synthetic engine test scene (needs no ROM)
--rom FILE and NINJA_MASTER_ROM take priority over automatic lookup.

Running
-------
  $run_example            (extra arguments are passed through, e.g. --mute)

$sdl_note

Linux: run "python3 install-desktop.py" to add this release to your user app
menu. Keep the extracted directory in place; rerun after moving or upgrading.
macOS: drag Ninja Master.app from the DMG into Applications, then open it.
Mac apps are ad-hoc signed and are not notarized. If macOS blocks the
first launch, use System Settings > Privacy & Security > Open Anyway.
release.json records the version, source revision and SHA-256 of each file.
The archive's SHA-256 is in its adjacent .sha256 file.

Controls, settings file locations and command-line options are described in
MANUAL.md.
EOF

cat > "$stage/LICENSE-NOTES.txt" <<'EOF'
License notes
=============

Shinobi III, its characters, graphics, music and sound are copyright Sega.
This package does not contain or redistribute the game ROM or any data
extracted from it; the executable reads a ROM the user supplies at run time.

The project's original code and app artwork are licensed under the MIT
license; see LICENSE. The license does not cover Sega's game content.
Third-party license texts are included in third-party-licenses/.

Third-party components:
- SDL2 (https://libsdl.org), zlib license. Windows and macOS packages include
  the runtime and license notices; Linux packages use the system library.
- Rust dependencies: see THIRD-PARTY-NOTICES.txt and third-party-licenses/.
- Dear ImGui and imgui/imgui-sys (https://github.com/imgui-rs/imgui-rs), MIT
  and MIT/Apache-2.0 respectively. See upstream projects for license texts.
- The application icon is original project artwork, not extracted ROM data.
EOF

"$python_cmd" "$project_root/tools/package_metadata.py" "$stage" "$project_root" "$platform" "$version" "$arch"

# Refuse to publish cartridge data: compare every staged file against the known
# US/EU/JP SHA-1s and, when available, search for the repository ROM's header block.
"$python_cmd" - "$stage" "$project_root/rom/shinobi3_us.bin" <<'EOF'
import hashlib, os, re, sys

stage, reference = sys.argv[1], sys.argv[2]
known = {
    "1e07d7998e3048fcfba4238ae96496460e91b3a5",  # US
    "23579c8f7e2396080b478b113aff36d2382395a3",  # EU
    "e3dbe326aa93320d405c561c306fc3954ab8ea7c",  # JP
}
header = None
if os.path.isfile(reference):
    with open(reference, "rb") as handle:
        header = handle.read()[0x100:0x200]
bad = []
for root, _, files in os.walk(stage):
    for name in files:
        path = os.path.join(root, name)
        with open(path, "rb") as handle:
            data = handle.read()
        if hashlib.sha1(data).hexdigest() in known:
            bad.append(path)
        elif (header and header in data) or re.search(rb"SEGA(?: GENESIS| MEGA DRIVE)", data):
            bad.append(path)
if bad:
    for path in bad:
        print(f"error: refusing to package cartridge data: {path}", file=sys.stderr)
    sys.exit(1)
EOF

if [ "$platform" = windows ]; then
    archive="$out_dir/$name.zip"
    rm -f -- "$archive"
    (cd "$out_dir" && "$python_cmd" -m zipfile -c "$archive" "$name")
else
    archive="$out_dir/$name.tar.gz"
    rm -f -- "$archive"
    tar -C "$out_dir" -czf "$archive" "$name"
fi
"$python_cmd" - "$archive" <<'EOF'
import hashlib, pathlib, sys
archive = pathlib.Path(sys.argv[1])
digest = hashlib.sha256(archive.read_bytes()).hexdigest()
pathlib.Path(str(archive) + ".sha256").write_text(f"{digest}  {archive.name}\n")
EOF
rm -rf -- "$stage"

echo "package: $archive"
