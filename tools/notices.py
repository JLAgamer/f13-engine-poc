"""Writes THIRD-PARTY-NOTICES.md for the release zip: every Rust crate's licence (cargo-about, about.toml /
about.hbs), libopus (compiled into f13-game by opusic-sys), and every .NET package the published exporter carries
(its deps.json, each package's licence from the local NuGet cache).

    python tools/notices.py <published exporter folder> <out file>

Needs `cargo install cargo-about --features cli`.
"""
import glob, json, os, re, subprocess, sys
import xml.etree.ElementTree as ET

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
NUGET = os.path.join(os.path.expanduser("~"), ".nuget", "packages")


def rust():
    out = subprocess.run(["cargo", "about", "generate", "about.hbs"], cwd=ROOT, check=True, capture_output=True, text=True,
                         encoding="utf-8").stdout
    opus = glob.glob(os.path.join(os.path.expanduser("~"), ".cargo", "registry", "src", "*", "opusic-sys-*", "opus", "COPYING"))
    if opus:
        out += "## libopus (compiled into f13-game by opusic-sys)\n\n```\n" + open(opus[0], encoding="utf-8").read().strip() + "\n```\n\n"
    return out


def nuspec_licence(name, version):
    """(licence expression or None, licence text or None, url or None) from the package's nuspec."""
    folder = os.path.join(NUGET, name.lower(), version.lower())
    specs = glob.glob(os.path.join(folder, "*.nuspec"))
    if not specs:
        return None, None, None
    tree = ET.parse(specs[0])
    ns = {"n": tree.getroot().tag.split("}")[0].strip("{")}
    meta = tree.getroot().find("n:metadata", ns)
    lic = meta.find("n:license", ns)
    url = meta.findtext("n:licenseUrl", default=None, namespaces=ns) or meta.findtext("n:projectUrl", default=None, namespaces=ns)
    expr = text = None
    if lic is not None and lic.get("type") == "expression":
        expr = lic.text
    elif lic is not None and lic.get("type") == "file":
        p = os.path.join(folder, lic.text)
        text = open(p, encoding="utf-8", errors="replace").read() if os.path.isfile(p) else None
    # A licence file in the package, when the nuspec names none.
    if text is None:
        for f in glob.glob(os.path.join(folder, "LICENSE*")) + glob.glob(os.path.join(folder, "license*")):
            if os.path.isfile(f):
                text = open(f, encoding="utf-8", errors="replace").read()
                break
    # Its NOTICE and third-party notices files (Apache-2.0 asks for NOTICE to be passed on).
    for f in sorted(set(glob.glob(os.path.join(folder, "NOTICE*")) + glob.glob(os.path.join(folder, "THIRD-PARTY-NOTICES*")))):
        if os.path.isfile(f):
            text = (text or "") + f"\n\n--- {os.path.basename(f)} ---\n\n" + open(f, encoding="utf-8", errors="replace").read()
    return expr, text, url


def dotnet(published):
    deps = json.load(open(glob.glob(os.path.join(published, "*.deps.json"))[0], encoding="utf-8"))
    libs = sorted(k for k, v in deps["libraries"].items() if v.get("type") in ("package", "runtimepack"))
    out = "## .NET packages (the exporter)\n\n"
    for lib in libs:
        name, version = lib.split("/")
        # The .NET runtime itself (published self-contained): its pack's own folder.
        name = name.removeprefix("runtimepack.")
        expr, text, url = nuspec_licence(name, version)
        out += f"### {name} {version}\n\n"
        if expr:
            out += f"Licence: {expr}\n\n"
        if url:
            out += f"{url}\n\n"
        if text:
            out += "```\n" + text.strip() + "\n```\n\n"
        if not (expr or text or url):
            out += "Licence: see the package (not found in the local NuGet cache)\n\n"
    return out


def main():
    published, out_path = sys.argv[1], sys.argv[2]
    head = ("# Third-party notices\n\nf13-engine-poc is MIT OR Apache-2.0 (LICENSE-MIT, LICENSE-APACHE). It bundles the software "
            "below under its own licences.\n\n")
    text = head + rust() + dotnet(published)
    open(out_path, "w", encoding="utf-8", newline="\n").write(text)
    print(f"{out_path}: {len(text) // 1024} KB")


if __name__ == "__main__":
    main()
