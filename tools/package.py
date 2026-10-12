"""Builds the player's release zip: dist/f13-rs-windows-x64.zip.

    python tools/package.py [--dotnet path/to/dotnet.exe]

It holds f13-installer.exe, f13-game.exe and f13-host.exe (cargo, release), the exporter published self-contained
(tools/f13-export/: no .NET install needed). No game files: the installer builds the
export from the player's own copy.
"""
import argparse, os, shutil, subprocess, sys, zipfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DIST = os.path.join(ROOT, "dist")
STAGE = os.path.join(DIST, "f13-rs")
ZIP = os.path.join(DIST, "f13-rs-windows-x64.zip")

PLAYER_README = """f13-rs: Friday the 13th: The Game, rebuilt from your own copy

1. Run f13-installer.exe.
2. Pick your unpacked Friday the 13th: The Game folder.
3. Press Install (a few minutes: it reads the gameplay values, map, characters and sounds from your game), then Play.

Afterwards start f13-game.exe directly. Reinstall after a game update.
This package contains no game files: everything is read from your copy.
"""


def run(cmd):
    print("+", " ".join(cmd), flush=True)
    subprocess.run(cmd, cwd=ROOT, check=True)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dotnet", default=shutil.which("dotnet") or "dotnet")
    args = ap.parse_args()

    run(["cargo", "build", "--release", "-p", "f13-game", "-p", "f13-host", "-p", "f13-installer"])
    if os.path.isdir(STAGE):
        shutil.rmtree(STAGE)
    os.makedirs(os.path.join(STAGE, "tools"))
    run([args.dotnet, "publish", os.path.join("tools", "f13-export"), "-c", "Release", "-r", "win-x64", "--self-contained", "true",
         "-o", os.path.join(STAGE, "tools", "f13-export")])
    for exe in ["f13-installer.exe", "f13-game.exe", "f13-host.exe"]:
        shutil.copy2(os.path.join(ROOT, "target", "release", exe), STAGE)
    for f in ["LICENSE-MIT", "LICENSE-APACHE", "LICENSE"]:
        if os.path.isfile(os.path.join(ROOT, f)):
            shutil.copy2(os.path.join(ROOT, f), STAGE)
    with open(os.path.join(STAGE, "README.txt"), "w", newline="\r\n") as f:
        f.write(PLAYER_README)

    # Symbols aren't shipped.
    for dirpath, _, files in os.walk(STAGE):
        for name in files:
            if name.endswith(".pdb"):
                os.remove(os.path.join(dirpath, name))
    if os.path.exists(ZIP):
        os.remove(ZIP)
    with zipfile.ZipFile(ZIP, "w", zipfile.ZIP_DEFLATED) as z:
        for dirpath, _, files in os.walk(STAGE):
            for name in files:
                full = os.path.join(dirpath, name)
                z.write(full, os.path.relpath(full, DIST))
    print(f"{ZIP}: {os.path.getsize(ZIP) / 2**20:.0f} MB")


if __name__ == "__main__":
    sys.exit(main())
