#!/usr/bin/env python3
"""Add platform integration and a machine-readable release manifest to staging."""
import hashlib
import json
from pathlib import Path
import plistlib
import shutil
import subprocess
import sys
import tomllib


def prepare(stage, root, platform, version, arch):
    stage, root = Path(stage), Path(root)
    cargo = tomllib.loads((root / "native/Cargo.toml").read_text())['package']
    resources = root / "native/resources"
    icons = stage / "icons"
    icons.mkdir()
    for name in resources.glob("ninja-master*.png"):
        shutil.copyfile(name, icons / name.name)
    for extension in ("svg", "ico", "icns"):
        shutil.copyfile(resources / f"ninja-master.{extension}", icons / f"ninja-master.{extension}")
    executable = "ninja-master-native.exe" if platform == "windows" else "ninja-master-native"
    if platform == "linux":
        shutil.copyfile(resources / "ninja-master.desktop", stage / "ninja-master.desktop")
        shutil.copyfile(root / "tools/install_desktop.py", stage / "install-desktop.py")
    elif platform == "macos":
        contents = stage / "Ninja Master.app/Contents"
        (contents / "MacOS").mkdir(parents=True)
        (contents / "Resources").mkdir()
        shutil.move(stage / executable, contents / "MacOS" / executable)
        shutil.copyfile(resources / "ninja-master.icns", contents / "Resources/ninja-master.icns")
        info = {
            "CFBundleDevelopmentRegion": "en",
            "CFBundleDisplayName": "Ninja Master",
            "CFBundleName": "Ninja Master",
            "CFBundleIdentifier": "org.ninja-master.native",
            "CFBundleInfoDictionaryVersion": "6.0",
            "CFBundlePackageType": "APPL",
            "CFBundleExecutable": executable,
            "CFBundleIconFile": "ninja-master.icns",
            "CFBundleShortVersionString": cargo['version'],
            "CFBundleVersion": cargo['version'],
            "NSHighResolutionCapable": True,
            "LSApplicationCategoryType": "public.app-category.action-games",
            "LSMinimumSystemVersion": "11.0" if arch in ("arm64", "aarch64") else "10.13",
            "NinjaMasterReleaseVersion": version,
        }
        (contents / "Info.plist").write_bytes(plistlib.dumps(info))
        executable = "Ninja Master.app/Contents/MacOS/" + executable
        launcher = stage / "play.sh"
        launcher.write_text(launcher.read_text().replace('$dir/ninja-master-native', '$dir/' + executable))
    revision = subprocess.run(["git", "-C", str(root), "rev-parse", "HEAD"],
                              capture_output=True, text=True, check=True).stdout.strip()
    manifest = {
        "schema_version": 1,
        "name": "Ninja Master",
        "description": cargo['description'],
        "version": version,
        "application_version": cargo['version'],
        "revision": revision,
        "platform": platform,
        "architecture": arch,
        "executable": executable,
        "rom": {"included": False, "region": "US", "size": 1048576,
                "crc32": "5381506f", "sha1": "1e07d7998e3048fcfba4238ae96496460e91b3a5"},
        "files": {str(path.relative_to(stage)): hashlib.sha256(path.read_bytes()).hexdigest()
                  for path in sorted(stage.rglob("*")) if path.is_file()},
    }
    (stage / "release.json").write_text(json.dumps(manifest, indent=2) + "\n")


if __name__ == "__main__":
    prepare(*sys.argv[1:])
