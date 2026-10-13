"""Release layout, integrity metadata, desktop installation and ROM exclusion."""
import hashlib
import json
from pathlib import Path
import plistlib
import subprocess
import tarfile
import tempfile
import unittest
import zipfile

from install_desktop import install, exec_quote

ROOT = Path(__file__).resolve().parents[1]


class PackageTests(unittest.TestCase):
    def package(self, platform, binary, output, *extra):
        return subprocess.run([str(ROOT / "tools/package.sh"), platform, str(binary),
                               "--out", str(output), "--version", "0.1.0-test", *extra],
                              capture_output=True, text=True)

    def test_release_layouts_and_integrity(self):
        with tempfile.TemporaryDirectory(prefix="ninja-package-") as tmp:
            tmp = Path(tmp)
            binary = tmp / "executable"
            binary.write_bytes(b"placeholder executable, no cartridge content")
            dll = tmp / "SDL2.dll"
            dll.write_bytes(b"placeholder SDL2")
            sdl_license = tmp / "LICENSE-SDL2.txt"
            sdl_license.write_text("Synthetic license fixture for package layout tests.\n")
            for platform in ("linux", "windows", "macos"):
                with self.subTest(platform=platform):
                    output = tmp / platform
                    extra = ["--sdl-dll", str(dll), "--sdl-license", str(sdl_license)] if platform == "windows" else []
                    result = self.package(platform, binary, output, *extra)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    extension = "*.zip" if platform == "windows" else "*.tar.gz"
                    archive = next(output.glob(extension))
                    checksum = Path(str(archive) + ".sha256").read_text().split()[0]
                    self.assertEqual(checksum, hashlib.sha256(archive.read_bytes()).hexdigest())
                    extracted = tmp / f"extracted {platform}"
                    extracted.mkdir()
                    if platform == "windows":
                        with zipfile.ZipFile(archive) as handle:
                            handle.extractall(extracted)
                    else:
                        with tarfile.open(archive) as handle:
                            # This archive was just produced from the fixture above.
                            handle.extractall(extracted, filter="data")
                    package = next(extracted.iterdir())
                    manifest = json.loads((package / "release.json").read_text())
                    self.assertEqual(manifest['platform'], platform)
                    self.assertFalse(manifest['rom']['included'])
                    for name, digest in manifest['files'].items():
                        self.assertEqual(hashlib.sha256((package / name).read_bytes()).hexdigest(), digest)
                    self.assertFalse(list(package.rglob("*.bin")))
                    self.assertTrue((package / "LICENSE").is_file())
                    self.assertTrue((package / "THIRD-PARTY-NOTICES.txt").is_file())
                    self.assertTrue(list((package / "third-party-licenses").iterdir()))
                    self.assertTrue((package / "assets/banner.svg").is_file())
                    if platform == "linux":
                        installed = install(package, tmp / "xdg")
                        self.assertIn(exec_quote(str(package / "play.sh")), installed.read_text())
                        self.assertTrue((tmp / "xdg/icons/hicolor/256x256/apps/ninja-master.png").exists())
                    elif platform == "macos":
                        contents = package / "Ninja Master.app/Contents"
                        plist = plistlib.loads((contents / "Info.plist").read_bytes())
                        self.assertTrue((contents / "MacOS" / plist['CFBundleExecutable']).exists())
                        self.assertTrue((contents / "Resources" / plist['CFBundleIconFile']).exists())
                        self.assertIn("Ninja Master.app/Contents/MacOS/ninja-master-native", (package / "play.sh").read_text())
                    else:
                        self.assertIn(b"\r\n", (package / "play.bat").read_bytes())

    def test_refuses_cartridge_and_unsafe_version(self):
        with tempfile.TemporaryDirectory(prefix="ninja-package-") as tmp:
            cartridge = Path(tmp) / "synthetic-cartridge.bin"
            data = bytearray(1_048_576)
            data[0x100:0x10c] = b"SEGA GENESIS"
            cartridge.write_bytes(data)
            result = self.package("linux", cartridge, tmp)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("refusing to package cartridge data", result.stderr)
            self.assertFalse(list(Path(tmp).glob("*.tar.gz")))
            result = self.package("linux", ROOT / "native/Cargo.toml", tmp, "--version", "../../bad")
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("simple filename components", result.stderr)

    def test_desktop_exec_escapes_reserved_characters(self):
        self.assertEqual(exec_quote('/tmp/100%/$a"b`c\\d'), '"/tmp/100%%/\\\\$a\\\\"b\\\\`c\\\\\\\\d"')


if __name__ == "__main__":
    unittest.main()
