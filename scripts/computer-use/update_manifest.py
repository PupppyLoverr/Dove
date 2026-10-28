"""Pin Dove Computer Use to a cua-driver release.

    python scripts/computer-use/update_manifest.py [--version 0.38.0] [--check]

Downloads the five platform archives of trycua's `cua-driver-rs-v<version>`
release (the newest `cua-driver-rs-v*` tag when --version is omitted), checks
each against the SHA-256 GitHub publishes for it and against the release's
own `SHA256SUMS`, and writes `apps/dove/assets/computer-use-release.json`
with every file inside each archive (path, size, SHA-256, role). Archives are
read as data only: nothing is extracted to disk or executed. Archives with a
link, more than one top folder, or no driver are refused.

--check exits 1 when the pinned version is older than the newest release
(for the weekly workflow), without downloading anything.
"""
from __future__ import annotations

import argparse
import hashlib
import io
import json
import os
import pathlib
import re
import sys
import tarfile
import urllib.request
import zipfile

REPO = "trycua/cua"
OUT = pathlib.Path(__file__).resolve().parents[2] / "apps" / "dove" / "assets" / "computer-use-release.json"
# Dove platform key -> (archive suffix, kind, driver entrypoint or None for macOS).
PLATFORMS = {
    "windows-x86_64": ("windows-x86_64.zip", "zip", "cua-driver.exe"),
    "windows-arm64": ("windows-arm64.zip", "zip", "cua-driver.exe"),
    "macos-universal": ("darwin-universal.tar.gz", "tar.gz", "CuaDriver.app/Contents/MacOS/cua-driver"),
    "linux-x86_64": ("linux-x86_64.tar.gz", "tar.gz", "cua-driver"),
    "linux-arm64": ("linux-arm64.tar.gz", "tar.gz", "cua-driver"),
}
TAG = re.compile(r"^cua-driver-rs-v(\d+)\.(\d+)\.(\d+)$")


def api(path: str):
    req = urllib.request.Request(f"https://api.github.com/repos/{REPO}/{path}", headers={"Accept": "application/vnd.github+json"})
    token = os.environ.get("GITHUB_TOKEN")
    if token:
        req.add_header("Authorization", f"Bearer {token}")
    with urllib.request.urlopen(req, timeout=60) as r:
        return json.load(r)


def latest_version() -> str:
    """Highest semantic version among `cua-driver-rs-v*` tags (never GitHub's repo-wide "latest")."""
    best = None
    page = 1
    while page <= 10:
        tags = api(f"tags?per_page=100&page={page}")
        if not tags:
            break
        for t in tags:
            m = TAG.match(t["name"])
            if m:
                v = tuple(int(x) for x in m.groups())
                best = max(best or v, v)
        page += 1
    if best is None:
        sys.exit("no cua-driver-rs-v* tag found")
    return ".".join(map(str, best))


def download(url: str) -> bytes:
    with urllib.request.urlopen(url, timeout=600) as r:
        return r.read()


def role(path: str, entry: str) -> str:
    base = path.rsplit("/", 1)[-1]
    if path == entry:
        return "driver"
    if base == "cua-driver-uia.exe":
        return "uia-sidecar"
    if base == "LICENSE":
        return "license"
    if base.startswith("THIRD_PARTY") or base == "NOTICE.md":
        return "notice"
    return "support"


def members(data: bytes, kind: str):
    """(path, bytes, executable) for every regular file; refuses links."""
    if kind == "zip":
        z = zipfile.ZipFile(io.BytesIO(data))
        for i in z.infolist():
            if i.is_dir():
                continue
            mode = i.external_attr >> 16
            if (mode & 0o170000) == 0o120000:
                sys.exit(f"{i.filename}: links are not allowed")
            yield i.filename, z.read(i), bool(mode & 0o111)
    else:
        t = tarfile.open(fileobj=io.BytesIO(data), mode="r:gz")
        for m in t.getmembers():
            if m.isdir():
                continue
            if not m.isfile():
                sys.exit(f"{m.name}: links and special files are not allowed")
            yield m.name, t.extractfile(m).read(), bool(m.mode & 0o111)


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--version")
    ap.add_argument("--check", action="store_true")
    a = ap.parse_args()
    version = a.version or latest_version()
    current = json.loads(OUT.read_text(encoding="utf-8")).get("version", "") if OUT.exists() else ""
    if a.check:
        newer = tuple(map(int, version.split("."))) > tuple(map(int, (current or "0.0.0").split("-")[0].split(".")))
        print(f"pinned {current or 'nothing'}, newest {version}")
        sys.exit(1 if newer else 0)
    tag = f"cua-driver-rs-v{version}"
    release = api(f"releases/tags/{tag}")
    assets = {x["name"]: x for x in release["assets"]}
    sums = {}
    if "SHA256SUMS" in assets:
        for line in download(assets["SHA256SUMS"]["browser_download_url"]).decode().splitlines():
            parts = line.split()
            if len(parts) == 2:
                sums[parts[1].lstrip("*")] = parts[0].lower()
    manifest = {"manifest_version": 1, "name": "cua-driver", "version": version, "tag": tag,
                "repository": REPO, "min_dove_version": "0.1.0", "platforms": {}}
    for key, (suffix, kind, entry) in PLATFORMS.items():
        name = f"cua-driver-rs-{version}-{suffix}"
        asset = assets.get(name)
        if asset is None:
            print(f"{key}: {name} is not in the release; skipped", file=sys.stderr)
            continue
        data = download(asset["browser_download_url"])
        digest = hashlib.sha256(data).hexdigest()
        published = (asset.get("digest") or "").removeprefix("sha256:")
        if published and published != digest:
            sys.exit(f"{name}: SHA-256 {digest} is not GitHub's {published}")
        if name in sums and sums[name] != digest:
            sys.exit(f"{name}: SHA-256 {digest} is not the release's SHA256SUMS {sums[name]}")
        files, roots = [], set()
        for rel, body, exe in members(data, kind):
            top, _, inner = rel.partition("/")
            roots.add(top)
            files.append((inner, body, exe))
        if len(roots) != 1:
            sys.exit(f"{name}: expected one top folder, found {sorted(roots)}")
        if not any(f == entry for f, _, _ in files):
            sys.exit(f"{name}: no driver at {entry}")
        manifest["platforms"][key] = {
            "name": name, "url": asset["browser_download_url"], "sha256": digest, "size": len(data),
            "archive": kind, "root": roots.pop(), "entrypoint": entry,
            "files": [{"path": f, "sha256": hashlib.sha256(b).hexdigest(), "size": len(b),
                       "executable": e or f == entry, "role": role(f, entry)} for f, b, e in sorted(files)],
        }
        print(f"{key}: {len(files)} files, {len(data)} bytes")
    if len(manifest["platforms"]) != len(PLATFORMS):
        sys.exit("not every platform is in this release; refusing to pin it")
    OUT.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    print(f"pinned cua-driver {version} in {OUT}")


if __name__ == "__main__":
    main()
