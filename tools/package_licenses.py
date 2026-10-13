#!/usr/bin/env python3
"""Include dependency attribution and license texts in a binary package."""
import json
from pathlib import Path
import shutil
import subprocess
import sys


def prepare(stage, root):
    stage, root = Path(stage), Path(root)
    metadata = json.loads(subprocess.run(
        ["cargo", "metadata", "--locked", "--offline", "--format-version", "1",
         "--manifest-path", str(root / "native/Cargo.toml")],
        capture_output=True, text=True, check=True).stdout)
    notices = ["Third-party dependencies", "========================", "",
               "These components retain their own copyright and license terms.",
               "The following list includes runtime and build dependencies.", ""]
    for package in sorted(metadata["packages"], key=lambda p: (p["name"], p["version"])):
        if package["name"] == "ninja-master-native":
            continue
        source = Path(package["manifest_path"]).parent
        name = f'{package["name"]}-{package["version"]}'
        destination = stage / "third-party-licenses" / name
        licenses = [p for p in source.iterdir() if p.is_file() and
                    p.name.upper().startswith(("LICENSE", "LICENCE", "COPYING", "NOTICE"))]
        if package.get("license_file"):
            licenses.append(source / package["license_file"])
        if package["name"] == "sdl2-sys":
            licenses.append(source / "SDL/LICENSE.txt")
            bindings = next(p for p in metadata["packages"] if p["name"] == "sdl2")
            binding_root = Path(bindings["manifest_path"]).parent
            licenses += [p for p in binding_root.iterdir() if p.is_file() and
                         p.name.upper().startswith(("LICENSE", "LICENCE", "COPYING"))]
        if not licenses:
            raise RuntimeError(f"No dependency license text found for {name}")
        destination.mkdir(parents=True, exist_ok=True)
        for path in sorted(set(licenses)):
            name_in_package = "LICENSE-SDL2.txt" if path == source / "SDL/LICENSE.txt" else path.name
            shutil.copyfile(path, destination / name_in_package)
        # Bundled Dear ImGui has its own upstream MIT notice.
        if package["name"] == "imgui-sys":
            for path in source.rglob("LICENSE.txt"):
                relative = path.relative_to(source)
                target = destination / relative
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(path, target)
        notices += [name, f'  License: {package.get("license") or "see license text"}',
                    f'  Project: {package.get("repository") or package.get("homepage") or "https://crates.io/crates/" + package["name"]}',
                    f"  Texts: third-party-licenses/{name}/", ""]
    (stage / "THIRD-PARTY-NOTICES.txt").write_text("\n".join(notices))


if __name__ == "__main__":
    prepare(*sys.argv[1:])
