"""Native installer payload, upgrade identity and cartridge exclusion tests."""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch
import xml.etree.ElementTree as ET

from bundle_macos import dependencies, minimum_version
from package_installers import WIX, deb, linux_tree, validate, wix_source

ROOT = Path(__file__).resolve().parents[1]


def fixture(stage, platform="windows"):
    stage.mkdir()
    for name in ("LICENSE", "README.txt", "MANUAL.md", "THIRD-PARTY-NOTICES.txt"):
        (stage / name).write_text("Independent synthetic package fixture.\n")
    shutil.copyfile(ROOT / "LICENSE", stage / "LICENSE")
    shutil.copyfile(ROOT / "native/resources/ninja-master.desktop", stage / "ninja-master.desktop")
    shutil.copytree(ROOT / "native/resources", stage / "icons")
    (stage / "assets").mkdir()
    (stage / "assets/banner.svg").write_text("<svg/>")
    (stage / "third-party-licenses").mkdir()
    (stage / "third-party-licenses/test-license.txt").write_text("Synthetic license fixture.")
    (stage / "ninja-master-native.exe").write_bytes(b"synthetic executable fixture")
    manifest = {"platform": platform, "architecture": "x86_64", "version": "0.1.2", "rom": {"included": False},
                "files": {p.relative_to(stage).as_posix(): hashlib.sha256(p.read_bytes()).hexdigest() for p in stage.rglob("*") if p.is_file()}}
    (stage / "release.json").write_text(json.dumps(manifest))
    return manifest


class InstallerTests(unittest.TestCase):
    def test_installer_refuses_modified_files_and_cartridge_payload(self):
        with tempfile.TemporaryDirectory() as tmp:
            stage = Path(tmp) / "stage"
            fixture(stage)
            self.assertEqual(validate(stage)["version"], "0.1.2")
            (stage / "ninja-master-native.exe").write_bytes(b"changed")
            with self.assertRaisesRegex(ValueError, "checksum mismatch"):
                validate(stage)
            (stage / "ninja-master-native.exe").write_bytes(b"synthetic executable fixture")
            (stage / "hidden.bin").write_bytes(b"synthetic cartridge bytes")
            with self.assertRaisesRegex(ValueError, "cartridge data"):
                validate(stage)

    def test_msi_owns_only_payload_files_with_stable_upgrade_identifiers(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            stage = tmp / "stage"
            manifest = fixture(stage)
            first, second = tmp / "one.wxs", tmp / "two.wxs"
            wix_source(stage, manifest, first)
            wix_source(stage, dict(manifest, version="0.1.3"), second)
            a, b = ET.parse(first), ET.parse(second)
            ns = {"w": WIX}
            self.assertEqual(a.find("w:Product", ns).get("UpgradeCode"), b.find("w:Product", ns).get("UpgradeCode"))
            self.assertEqual([c.get("Guid") for c in a.findall(".//w:Component", ns)], [c.get("Guid") for c in b.findall(".//w:Component", ns)])
            self.assertEqual(a.find(".//w:Package", ns).get("InstallScope"), "perUser")
            for component in a.findall(".//w:Component", ns):
                key = component.find("w:RegistryValue[@KeyPath='yes']", ns)
                self.assertIsNotNone(key)
                self.assertEqual(key.get("Root"), "HKCU")
            actual = {Path(f.get("Source")) for f in a.findall(".//w:File", ns)}
            expected = {p.resolve() for p in stage.rglob("*") if p.is_file()}
            self.assertEqual(actual, expected)
            self.assertFalse(a.findall(".//w:RemoveFile", ns))
            self.assertTrue(a.findall(".//w:RemoveFolder", ns))
            self.assertEqual(a.find(".//w:Shortcut", ns).get("Target"), "[INSTALLDIR]ninja-master-native.exe")

    def test_linux_launcher_uses_user_pack_folders_and_integrated_icons(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            stage = tmp / "stage"
            fixture(stage, "linux")
            root = tmp / "installed"
            payload = linux_tree(stage, root)
            self.assertTrue((payload / "LICENSE").is_file())
            script = (root / "usr/bin/ninja-master").read_text()
            self.assertIn("XDG_DATA_HOME", script)
            self.assertIn("/ninja-master/art-packs}", script)
            self.assertIn("/ninja-master/audio-packs}", script)
            self.assertIn('ninja-master-native "$@"', script)
            self.assertTrue((root / "usr/share/icons/hicolor/256x256/apps/ninja-master.png").is_file())
            self.assertTrue((root / "usr/share/doc/ninja-master/third-party-licenses/test-license.txt").is_file())

    @unittest.skipUnless(shutil.which("dpkg-deb") and shutil.which("dpkg-shlibdeps") and shutil.which("cc"), "Debian packaging tools required")
    def test_deb_metadata_and_payload_integrity(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            stage = tmp / "stage"
            manifest = fixture(stage, "linux")
            source = tmp / "fixture.c"
            source.write_text("int main(void) { return 0; }\n")
            subprocess.run(["cc", str(source), "-o", str(stage / "ninja-master-native")], check=True)
            manifest["files"]["ninja-master-native"] = hashlib.sha256((stage / "ninja-master-native").read_bytes()).hexdigest()
            (stage / "release.json").write_text(json.dumps(manifest))
            output, workspace = tmp / "output", tmp / "workspace"
            output.mkdir()
            workspace.mkdir()
            result = deb(stage, manifest, output, workspace)
            self.assertEqual(subprocess.check_output(["dpkg-deb", "--field", str(result), "Architecture"], text=True).strip(), "amd64")
            self.assertIn("libsdl2-2.0-0", subprocess.check_output(["dpkg-deb", "--field", str(result), "Depends"], text=True))
            installed = tmp / "extracted"
            subprocess.run(["dpkg-deb", "--extract", str(result), str(installed)], check=True)
            self.assertEqual(validate(installed / "usr/lib/ninja-master")["version"], "0.1.2")

    def test_macos_minimum_ignores_library_version_and_sdk_version(self):
        output = """Load command 1
          cmd LC_ID_DYLIB
      current version 3200.0.0
Load command 2
          cmd LC_BUILD_VERSION
        minos 15.0
          sdk 26.0
Load command 3
          cmd LC_SOURCE_VERSION
      version 9000.1.0
"""
        with patch("bundle_macos.subprocess.check_output", return_value=output):
            self.assertEqual(minimum_version(Path("synthetic.dylib")), (15, 0))

    def test_macos_universal_library_headers_are_not_dependencies(self):
        output = """/bundle/SDL2 (architecture x86_64):
\t@rpath/SDL2 (compatibility version 1.0.0, current version 1.0.0)
\t/usr/lib/libSystem.B.dylib (compatibility version 1.0.0, current version 1.0.0)
/bundle/SDL2 (architecture arm64):
\t@rpath/SDL2 (compatibility version 1.0.0, current version 1.0.0)
\t/usr/lib/libSystem.B.dylib (compatibility version 1.0.0, current version 1.0.0)
"""
        with patch("bundle_macos.subprocess.check_output", return_value=output):
            self.assertEqual(dependencies(Path("SDL2")), ["@rpath/SDL2", "/usr/lib/libSystem.B.dylib"] * 2)


if __name__ == "__main__":
    unittest.main()
