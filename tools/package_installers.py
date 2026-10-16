#!/usr/bin/env python3
"""Create native installers from a verified portable release archive."""
import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import subprocess
import tarfile
import tempfile
import uuid
import xml.etree.ElementTree as ET
import zipfile

NAMESPACE = uuid.uuid5(uuid.NAMESPACE_DNS, "org.ninja-master.native.windows.x64")
WIX = "http://schemas.microsoft.com/wix/2006/wi"


def checksum(path):
    path = Path(path)
    digest = hashlib.sha256(path.read_bytes()).hexdigest()
    Path(str(path) + ".sha256").write_text(f"{digest}  {path.name}\n")


def validate(stage):
    stage = Path(stage)
    manifest = json.loads((stage / "release.json").read_text())
    if manifest["rom"]["included"]:
        raise ValueError("cartridge data must not be included")
    for name, digest in manifest["files"].items():
        relative = PurePosixPath(name.replace("\\", "/"))
        if relative.is_absolute() or ".." in relative.parts:
            raise ValueError("unsafe manifest path")
        path = stage.joinpath(*relative.parts)
        if hashlib.sha256(path.read_bytes()).hexdigest() != digest:
            raise ValueError(f"package file checksum mismatch: {name}")
    known = {"1e07d7998e3048fcfba4238ae96496460e91b3a5",
             "23579c8f7e2396080b478b113aff36d2382395a3",
             "e3dbe326aa93320d405c561c306fc3954ab8ea7c"}
    for path in stage.rglob("*"):
        if not path.is_file():
            continue
        data = path.read_bytes()
        if path.suffix.lower() in {".bin", ".smd", ".gen", ".rom"} or hashlib.sha1(data).hexdigest() in known or re.search(rb"SEGA(?: GENESIS| MEGA DRIVE)", data):
            raise ValueError(f"refusing to package cartridge data: {path.name}")
    if not re.fullmatch(r"\d+\.\d+\.\d+(?:-[A-Za-z0-9.+-]+)?", manifest["version"]):
        raise ValueError("installer version must be a semantic version")
    return manifest


def extract(archive, destination):
    archive, destination = Path(archive), Path(destination)
    if archive.suffix == ".zip":
        with zipfile.ZipFile(archive) as handle:
            for name in handle.namelist():
                relative = PurePosixPath(name)
                if relative.is_absolute() or ".." in relative.parts or "\\" in name:
                    raise ValueError("unsafe archive path")
            handle.extractall(destination)
    else:
        with tarfile.open(archive) as handle:
            handle.extractall(destination, filter="data")
    roots = list(destination.iterdir())
    if len(roots) != 1 or not roots[0].is_dir():
        raise ValueError("archive must contain one release directory")
    return roots[0], validate(roots[0])


def linux_tree(stage, destination):
    stage, destination = Path(stage), Path(destination)
    payload = destination / "usr/lib/ninja-master"
    shutil.copytree(stage, payload)
    launcher = destination / "usr/bin/ninja-master"
    launcher.parent.mkdir(parents=True)
    launcher.write_text('''#!/bin/sh
set -eu
export NINJA_MASTER_ART_PACKS="${NINJA_MASTER_ART_PACKS:-${XDG_DATA_HOME:-$HOME/.local/share}/ninja-master/art-packs}"
exec /usr/lib/ninja-master/ninja-master-native "$@"
''')
    launcher.chmod(0o755)
    applications = destination / "usr/share/applications"
    applications.mkdir(parents=True)
    shutil.copyfile(stage / "ninja-master.desktop", applications / "ninja-master.desktop")
    for size in (16, 32, 48, 64, 128, 256, 512):
        icons = destination / f"usr/share/icons/hicolor/{size}x{size}/apps"
        icons.mkdir(parents=True)
        shutil.copyfile(stage / f"icons/ninja-master-{size}.png", icons / "ninja-master.png")
    icons = destination / "usr/share/icons/hicolor/scalable/apps"
    icons.mkdir(parents=True)
    shutil.copyfile(stage / "icons/ninja-master.svg", icons / "ninja-master.svg")
    docs = destination / "usr/share/doc/ninja-master"
    docs.mkdir(parents=True)
    for name in ("LICENSE", "README.txt", "MANUAL.md", "THIRD-PARTY-NOTICES.txt"):
        shutil.copyfile(stage / name, docs / name)
    shutil.copytree(stage / "assets", docs / "assets")
    shutil.copytree(stage / "third-party-licenses", docs / "third-party-licenses")
    return payload


def deb(stage, manifest, output, workspace):
    root = workspace / "deb"
    payload = linux_tree(stage, root)
    metadata = root / "debian"
    metadata.mkdir()
    maintainer = "Jake Cattrall <krazyjakee@gmail.com>"
    (metadata / "control").write_text(f"Source: ninja-master\nSection: games\nPriority: optional\nMaintainer: {maintainer}\n\nPackage: ninja-master\nArchitecture: any\nDescription: Native Shinobi III port requiring a user-supplied US, EU or JP ROM\n")
    dependencies = subprocess.check_output(["dpkg-shlibdeps", "-O", str(payload / "ninja-master-native")], cwd=root, text=True)
    dependencies = next(line.split("=", 1)[1] for line in dependencies.splitlines() if line.startswith("shlibs:Depends="))
    if "libsdl2-2.0-0" not in dependencies:
        dependencies += ", libsdl2-2.0-0 (>= 2.0.18)"
    shutil.rmtree(metadata)
    control = root / "DEBIAN"
    control.mkdir()
    version = manifest["version"].replace("-", "~", 1)
    size = sum(p.stat().st_size for p in root.rglob("*") if p.is_file()) // 1024 + 1
    (control / "control").write_text(f"Package: ninja-master\nVersion: {version}\nArchitecture: amd64\nMaintainer: {maintainer}\nSection: games\nPriority: optional\nInstalled-Size: {size}\nDepends: {dependencies}\nHomepage: https://github.com/krazyjakee/ninja-master\nDescription: Native Shinobi III desktop port\n Requires a user-supplied compatible US, EU or JP cartridge dump.\n No original game assets are included.\n")
    files = []
    for path in sorted(root.rglob("*")):
        if path.is_file() and "DEBIAN" not in path.relative_to(root).parts:
            files.append(f"{hashlib.md5(path.read_bytes()).hexdigest()}  {path.relative_to(root).as_posix()}")
    (control / "md5sums").write_text("\n".join(files) + "\n")
    result = output / f'ninja-master-{manifest["version"]}-linux-x86_64.deb'
    subprocess.run(["dpkg-deb", "--root-owner-group", "--build", str(root), str(result)], check=True)
    checksum(result)
    return result


def rpm(stage, manifest, output, workspace):
    top = workspace / "rpm"
    for name in ("BUILD", "BUILDROOT", "RPMS", "SOURCES", "SPECS", "SRPMS"):
        (top / name).mkdir(parents=True)
    linux_tree(stage, top / "SOURCES/payload")
    numeric, _, suffix = manifest["version"].partition("-")
    release = "1" if not suffix else "0.1." + suffix.replace("-", ".")
    spec = top / "SPECS/ninja-master.spec"
    spec.write_text(f'''%global debug_package %{{nil}}
%global __os_install_post %{{nil}}
Name: ninja-master
Version: {numeric}
Release: {release}
Summary: Native Shinobi III desktop port
License: MIT
URL: https://github.com/krazyjakee/ninja-master
BuildArch: x86_64

%description
Native desktop gameplay using a user-supplied compatible US, EU or JP cartridge dump.
No original game assets are included.

%prep
%build
%install
mkdir -p %{{buildroot}}
cp -a %{{_sourcedir}}/payload/. %{{buildroot}}/

%files
/usr/bin/ninja-master
/usr/lib/ninja-master
/usr/share/applications/ninja-master.desktop
/usr/share/icons/hicolor/*/apps/ninja-master.*
/usr/share/doc/ninja-master
''')
    subprocess.run(["rpmbuild", "-bb", "--define", f"_topdir {top}", str(spec)], check=True)
    packages = list((top / "RPMS").rglob("*.rpm"))
    if len(packages) != 1:
        raise RuntimeError("expected exactly one RPM")
    result = output / f'ninja-master-{manifest["version"]}-linux-x86_64.rpm'
    shutil.copyfile(packages[0], result)
    checksum(result)
    return result


def wix_source(stage, manifest, filename):
    ET.register_namespace("", WIX)
    def add(parent, tag, **attributes):
        return ET.SubElement(parent, f"{{{WIX}}}{tag}", attributes)
    def identifier(prefix, name):
        return prefix + hashlib.sha256(name.encode()).hexdigest()[:20]
    def guid(name):
        return str(uuid.uuid5(NAMESPACE, name)).upper()
    root = ET.Element(f"{{{WIX}}}Wix")
    product = add(root, "Product", Id="*", Name="Ninja Master", Language="1033", Version=manifest["version"].split("-")[0], Manufacturer="Jake Cattrall", UpgradeCode=guid("upgrade"))
    add(product, "Package", InstallerVersion="500", Compressed="yes", InstallScope="perUser", Platform="x64")
    add(product, "MajorUpgrade", Schedule="afterInstallInitialize", AllowSameVersionUpgrades="yes", DowngradeErrorMessage="A newer Ninja Master version is already installed.")
    add(product, "MediaTemplate", EmbedCab="yes")
    add(product, "Property", Id="ARPPRODUCTICON", Value="AppIcon")
    location = add(product, "Property", Id="INSTALLDIR", Secure="yes")
    add(location, "RegistrySearch", Id="InstalledLocation", Root="HKCU", Key="Software\\Ninja Master", Name="InstallLocation", Type="raw", Win64="yes")
    add(product, "Icon", Id="AppIcon", SourceFile=str((stage / "icons/ninja-master.ico").resolve()))
    target = add(product, "Directory", Id="TARGETDIR", Name="SourceDir")
    programs = add(add(target, "Directory", Id="LocalAppDataFolder"), "Directory", Id="UserPrograms", Name="Programs")
    install = add(programs, "Directory", Id="INSTALLDIR", Name="Ninja Master")
    directories = {".": install}
    components = []
    for path in sorted(stage.rglob("*")):
        if not path.is_file():
            continue
        relative = path.relative_to(stage)
        current = Path()
        parent = install
        for part in relative.parts[:-1]:
            current /= part
            key = current.as_posix()
            if key not in directories:
                directories[key] = add(parent, "Directory", Id=identifier("D", key), Name=part)
            parent = directories[key]
        key = relative.as_posix()
        component_id = identifier("C", key)
        component = add(parent, "Component", Id=component_id, Guid=guid(key), Win64="yes")
        add(component, "File", Id=identifier("F", key), Name=path.name, Source=str(path.resolve()))
        add(component, "RegistryValue", Root="HKCU", Key="Software\\Ninja Master\\Components", Name=component_id, Type="integer", Value="1", KeyPath="yes")
        components.append(component_id)
    cleanup = add(install, "Component", Id="Cleanup", Guid=guid("cleanup"), Win64="yes")
    add(cleanup, "RegistryValue", Root="HKCU", Key="Software\\Ninja Master\\Components", Name="Cleanup", Type="integer", Value="1", KeyPath="yes")
    add(cleanup, "RegistryValue", Root="HKCU", Key="Software\\Ninja Master", Name="InstallLocation", Type="string", Value="[INSTALLDIR]")
    for key, directory in directories.items():
        add(cleanup, "RemoveFolder", Id=identifier("R", key), Directory=directory.attrib["Id"], On="uninstall")
    add(cleanup, "RemoveFolder", Id="RemoveUserPrograms", Directory="UserPrograms", On="uninstall")
    menu = add(add(target, "Directory", Id="ProgramMenuFolder"), "Directory", Id="AppMenu", Name="Ninja Master")
    shortcut = add(menu, "Component", Id="AppShortcut", Guid=guid("shortcut"), Win64="yes")
    add(shortcut, "Shortcut", Id="GameShortcut", Name="Ninja Master", Target="[INSTALLDIR]ninja-master-native.exe", Arguments="--boot", WorkingDirectory="INSTALLDIR", Icon="AppIcon")
    add(shortcut, "RemoveFolder", Id="RemoveAppMenu", On="uninstall")
    add(shortcut, "RegistryValue", Root="HKCU", Key="Software\\Ninja Master", Name="Installed", Type="integer", Value="1", KeyPath="yes")
    feature = add(product, "Feature", Id="Complete", Title="Ninja Master", Level="1")
    for component_id in [*components, "Cleanup", "AppShortcut"]:
        add(feature, "ComponentRef", Id=component_id)
    add(product, "UIRef", Id="WixUI_Minimal")
    license_text = (stage / "LICENSE").read_text().replace("\\", "\\\\").replace("{", "\\{").replace("}", "\\}").replace("\n", "\\par\n")
    license_file = Path(filename).with_suffix(".rtf")
    license_file.write_text("{\\rtf1\\ansi\\deff0 {\\fonttbl {\\f0 Segoe UI;}}\\f0\\fs18\n" + license_text + "}")
    add(product, "WixVariable", Id="WixUILicenseRtf", Value=str(license_file.resolve()))
    ET.indent(root)
    ET.ElementTree(root).write(filename, encoding="utf-8", xml_declaration=True)


def msi(stage, manifest, output, workspace, wix):
    source = workspace / "ninja-master.wxs"
    wix_source(stage, manifest, source)
    wix = Path(wix or os.environ["WIX"]) / "bin"
    obj = workspace / "ninja-master.wixobj"
    subprocess.run([str(wix / "candle.exe"), "-arch", "x64", "-ext", "WixUIExtension", str(source), "-out", str(obj)], check=True)
    result = output / f'ninja-master-{manifest["version"]}-windows-x86_64.msi'
    subprocess.run([str(wix / "light.exe"), "-ext", "WixUIExtension", str(obj), "-out", str(result)], check=True)
    checksum(result)
    return result


def dmg(stage, manifest, output, workspace):
    source = workspace / "dmg"
    source.mkdir()
    shutil.copytree(stage / "Ninja Master.app", source / "Ninja Master.app", symlinks=True)
    (source / "Applications").symlink_to("/Applications")
    for name in ("README.txt", "MANUAL.md", "LICENSE", "THIRD-PARTY-NOTICES.txt"):
        shutil.copyfile(stage / name, source / name)
    shutil.copytree(stage / "assets", source / "assets")
    result = output / f'ninja-master-{manifest["version"]}-macos-{manifest["architecture"]}.dmg'
    subprocess.run(["hdiutil", "create", "-volname", "Ninja Master", "-srcfolder", str(source), "-ov", "-format", "UDZO", str(result)], check=True)
    subprocess.run(["hdiutil", "verify", str(result)], check=True)
    checksum(result)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archive", type=Path)
    parser.add_argument("--formats", required=True, help="deb,rpm,msi or dmg")
    parser.add_argument("--wix", help="WiX 3 installation directory")
    args = parser.parse_args()
    archive = args.archive.resolve()
    with tempfile.TemporaryDirectory(prefix="ninja-installers-") as tmp:
        workspace = Path(tmp)
        stage, manifest = extract(archive, workspace / "archive")
        for kind in args.formats.split(","):
            platform = {"deb": "linux", "rpm": "linux", "msi": "windows", "dmg": "macos"}.get(kind)
            if platform != manifest["platform"]:
                parser.error(f"{kind} does not match the archive platform")
            if kind == "msi":
                result = msi(stage, manifest, archive.parent, workspace, args.wix)
            else:
                result = {"deb": deb, "rpm": rpm, "dmg": dmg}[kind](stage, manifest, archive.parent, workspace)
            print(f"installer: {result}")


if __name__ == "__main__":
    main()
