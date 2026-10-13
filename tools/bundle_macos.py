#!/usr/bin/env python3
"""Bundle non-system Mach-O libraries and sign a self-contained application."""
import os
from pathlib import Path
import re
import shutil
import subprocess


def dependencies(path):
    output = subprocess.check_output(["otool", "-L", str(path)], text=True)
    return [line.strip().split(" (", 1)[0] for line in output.splitlines()[1:] if " (" in line]


def system_library(name):
    return name.startswith(("/System/Library/", "/usr/lib/"))


def minimum_version(path):
    output = subprocess.check_output(["otool", "-l", str(path)], text=True)
    versions = []
    deployment = False
    for line in output.splitlines():
        text = line.strip()
        if text.startswith("cmd "):
            deployment = text in {"cmd LC_BUILD_VERSION", "cmd LC_VERSION_MIN_MACOSX"}
        if deployment and re.match(r"(?:minos|version) \d", text):
            versions.append(text.split()[1])
    return max((tuple(map(int, v.split("."))) for v in versions), default=(11, 0))


def resolve(name, source):
    if name.startswith("/"):
        return Path(name).resolve(strict=True)
    leaf = Path(name).name
    for directory in (source.parent, Path("/opt/homebrew/lib"), Path("/usr/local/lib")):
        candidate = directory / leaf
        if candidate.is_file():
            return candidate.resolve()
    raise RuntimeError(f"Cannot resolve application library: {name}")


def bundle(app):
    app = Path(app)
    executable = app / "Contents/MacOS/ninja-master-native"
    frameworks = app / "Contents/Frameworks"
    frameworks.mkdir()
    queue = [executable]
    origins = {executable: executable}
    copied = {}
    index = 0
    while index < len(queue):
        target = queue[index]
        index += 1
        source = origins[target]
        for dependency in dependencies(source):
            if system_library(dependency):
                continue
            original = resolve(dependency, source)
            if original == source.resolve():
                continue  # A dylib's own install-name entry.
            name = Path(dependency).name
            destination = frameworks / name
            if name in copied and copied[name] != original:
                raise RuntimeError(f"Conflicting library filenames: {name}")
            if name not in copied:
                copied[name] = original
                shutil.copyfile(original, destination)
                destination.chmod(0o755)
                origins[destination] = original
                queue.append(destination)
            prefix = "@loader_path/../Frameworks/" if target == executable else "@loader_path/"
            subprocess.run(["install_name_tool", "-change", dependency, prefix + name, str(target)], check=True)
        if target != executable:
            subprocess.run(["install_name_tool", "-id", "@rpath/" + target.name, str(target)], check=True)
    if not any("SDL2" in name for name in copied):
        raise RuntimeError("The application did not bundle an SDL2 runtime")
    minimum = max(minimum_version(path) for path in queue)
    deployment = tuple(map(int, os.environ.get("MACOSX_DEPLOYMENT_TARGET", "15.0").split(".")))
    minimum = max(minimum, deployment)
    for path in queue:
        for dependency in dependencies(path):
            if not system_library(dependency) and not dependency.startswith(("@loader_path/", "@rpath/")):
                raise RuntimeError(f"Unbundled library dependency: {dependency}")
    return ".".join(map(str, minimum))


def sign(app):
    app = Path(app)
    for library in sorted((app / "Contents/Frameworks").iterdir()):
        subprocess.run(["codesign", "--force", "--sign", "-", str(library)], check=True)
    subprocess.run(["codesign", "--force", "--deep", "--sign", "-",
                    "--identifier", "org.ninja-master.native", str(app)], check=True)
    subprocess.run(["codesign", "--verify", "--deep", "--strict", str(app)], check=True)
