#!/usr/bin/env python3
"""Install the extracted Linux release's menu entry for the current user."""
import os
from pathlib import Path
import shutil


def exec_quote(value):
    # Exec argument quoting, followed by desktop-entry string escaping.
    value = value.replace("%", "%%")
    for character in ('\\', '"', '`', '$'):
        value = value.replace(character, '\\' + character)
    return '"' + value.replace('\\', '\\\\') + '"'


def install(package, data):
    package, data = Path(package).resolve(), Path(data)
    template = (package / "ninja-master.desktop").read_text()
    launcher = str(package / "play.sh")
    if any(c in launcher for c in "\r\n"):
        raise ValueError("Move the package to a path without line breaks before installing.")
    desktop = template.replace("Exec=ninja-master", "Exec=" + exec_quote(launcher))
    applications = data / "applications"
    applications.mkdir(parents=True, exist_ok=True)
    (applications / "ninja-master.desktop").write_text(desktop)
    for size in (16, 32, 48, 64, 128, 256, 512):
        target = data / f"icons/hicolor/{size}x{size}/apps"
        target.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(package / f"icons/ninja-master-{size}.png", target / "ninja-master.png")
    target = data / "icons/hicolor/scalable/apps"
    target.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(package / "icons/ninja-master.svg", target / "ninja-master.svg")
    return applications / "ninja-master.desktop"


if __name__ == "__main__":
    package = Path(__file__).resolve().parent
    data = Path(os.environ.get("XDG_DATA_HOME", Path.home() / ".local/share"))
    print(f"Installed: {install(package, data)}")
    print("Keep this release directory in place; rerun after moving or upgrading it.")
