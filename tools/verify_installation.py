#!/usr/bin/env python3
"""Verify release checksums, native architecture, installation and startup."""
import argparse
import hashlib
import os
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile

from package_installers import extract, validate


def architecture(executable, platform, arch):
    data = Path(executable).read_bytes()
    if platform == "windows":
        assert data[:2] == b"MZ"
        offset = struct.unpack_from("<I", data, 0x3c)[0]
        assert data[offset:offset + 4] == b"PE\0\0"
        assert struct.unpack_from("<H", data, offset + 4)[0] == 0x8664
    elif platform == "linux":
        assert data[:5] == b"\x7fELF\x02"
        assert struct.unpack_from("<H", data, 18)[0] == 62
    else:
        assert data[:4] == b"\xcf\xfa\xed\xfe"
        assert struct.unpack_from("<I", data, 4)[0] == {"arm64": 0x100000c, "x86_64": 0x1000007}[arch]


def run(executable):
    result = subprocess.run([str(executable), "--demo", "--headless", "--frames", "120"], capture_output=True, text=True, check=True)
    if "frame=3a0419cf3d3b1731" not in result.stdout:
        raise RuntimeError(f"unexpected engine smoke result: {result.stdout} {result.stderr}")
    missing = subprocess.run([str(executable), "--boot", "--headless", "--frames", "1", "--rom", str(Path(executable).parent / "missing-cartridge.bin")], capture_output=True, text=True)
    if missing.returncode == 0:
        raise RuntimeError("startup unexpectedly succeeded without a cartridge")


def windows(installer, workspace):
    directory = workspace / "installed"
    log = workspace / "install.log"
    result = subprocess.run(["msiexec", "/i", str(installer), "/qn", "/norestart", f"INSTALLDIR={directory}", "/l*v", str(log)])
    if result.returncode not in (0, 3010):
        raise RuntimeError(log.read_text(errors="replace"))
    try:
        validate(directory)
        run(directory / "ninja-master-native.exe")
        sentinel = directory / "user-cartridge.bin"
        sentinel.write_bytes(b"user-owned file must survive uninstall")
    finally:
        result = subprocess.run(["msiexec", "/x", str(installer), "/qn", "/norestart"])
        if result.returncode not in (0, 3010):
            raise RuntimeError("MSI uninstall failed")
    assert sentinel.read_bytes() == b"user-owned file must survive uninstall"
    assert not (directory / "ninja-master-native.exe").exists()


def macos(image, workspace):
    mount = workspace / "mounted"
    mount.mkdir()
    subprocess.run(["hdiutil", "attach", "-nobrowse", "-readonly", "-mountpoint", str(mount), str(image)], check=True)
    try:
        installed = workspace / "Ninja Master.app"
        shutil.copytree(mount / "Ninja Master.app", installed, symlinks=True)
        subprocess.run(["codesign", "--verify", "--deep", "--strict", str(installed)], check=True)
        run(installed / "Contents/MacOS/ninja-master-native")
    finally:
        subprocess.run(["hdiutil", "detach", str(mount)], check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--platform", required=True, choices=["windows", "linux", "macos"])
    parser.add_argument("--arch", required=True, choices=["x86_64", "arm64"])
    parser.add_argument("--extract-only", action="store_true")
    args = parser.parse_args()
    directory = args.directory.resolve()
    suffixes = {"linux": [".tar.gz", ".deb", ".rpm"], "windows": [".zip", ".msi"], "macos": [".tar.gz", ".dmg"]}[args.platform]
    selected = {}
    for suffix in suffixes:
        matches = list(directory.glob("*" + suffix))
        if len(matches) != 1:
            raise RuntimeError(f"expected one {suffix} package")
        path = matches[0]
        digest = Path(str(path) + ".sha256").read_text().split()[0]
        assert hashlib.sha256(path.read_bytes()).hexdigest() == digest
        selected[suffix] = path
    with tempfile.TemporaryDirectory(prefix="ninja-installed-") as tmp:
        workspace = Path(tmp)
        stage, manifest = extract(selected[suffixes[0]], workspace / "portable")
        assert manifest["platform"] == args.platform and manifest["architecture"] == args.arch
        executable = stage / manifest["executable"]
        architecture(executable, args.platform, args.arch)
        run(executable)
        if args.platform == "windows":
            windows(selected[".msi"], workspace)
        elif args.platform == "macos":
            macos(selected[".dmg"], workspace)
        else:
            extracted = workspace / "deb"
            subprocess.run(["dpkg-deb", "--extract", str(selected[".deb"]), str(extracted)], check=True)
            validate(extracted / "usr/lib/ninja-master")
            run(extracted / "usr/lib/ninja-master/ninja-master-native")
            subprocess.run(["rpm", "--checksig", str(selected[".rpm"])], check=True)
            if not args.extract_only:
                subprocess.run(["sudo", "apt-get", "install", "-y", str(selected[".deb"])], check=True)
                run(Path("/usr/bin/ninja-master"))
    print(f"Verified {args.platform} {args.arch}: archive, installer, architecture and cartridge-free smoke")


if __name__ == "__main__":
    main()
