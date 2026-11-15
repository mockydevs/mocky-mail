#!/usr/bin/env python3
"""Write latest.json for the download site, so installed apps offer the update.

Run it in the release step, after the new installers and SHA256SUMS are in the
downloads folder, then upload latest.json next to them:

    scripts/write-latest-json.py path/to/downloads --notes "Faster, more reliable sync."

It reads the version from apps/desktop/src-tauri/tauri.conf.json, finds that
version's installers in SHA256SUMS and checks each file's SHA-256 before
writing. Installers that aren't there are left out; those installs are sent to
the download page instead.
"""

import argparse
import hashlib
import json
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
CONFIG = ROOT / "apps/desktop/src-tauri/tauri.conf.json"
INSTALLERS = {
    "linux-deb": "mocky-mail_{version}_amd64.deb",
    "windows-nsis": "mocky-mail_{version}_x64-setup.exe",
}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("downloads", type=pathlib.Path, help="folder with the installers and SHA256SUMS")
    parser.add_argument("--notes", default="", help="one or two lines on what's new, shown in the app")
    parser.add_argument("--base-url", default="https://mailer.co.ke/downloads")
    args = parser.parse_args()

    version = json.loads(CONFIG.read_text())["version"]
    sums = {}
    for line in (args.downloads / "SHA256SUMS").read_text().splitlines():
        parts = line.split()
        if len(parts) == 2:
            sums[parts[1].lstrip("*")] = parts[0].lower()

    installers = {}
    for kind, pattern in INSTALLERS.items():
        name = pattern.format(version=version)
        path = args.downloads / name
        if name not in sums or not path.is_file():
            print(f"note: no {kind} installer ({name}); those installs get the download page", file=sys.stderr)
            continue
        actual = hashlib.sha256(path.read_bytes()).hexdigest()
        if actual != sums[name]:
            print(f"error: {name} doesn't match SHA256SUMS", file=sys.stderr)
            return 1
        installers[kind] = {"url": f"{args.base_url.rstrip('/')}/{name}", "sha256": actual}

    if not installers:
        print(f"error: no installers for {version} in {args.downloads}", file=sys.stderr)
        return 1
    manifest = {"version": version, "notes": args.notes.strip(), "installers": installers}
    out = args.downloads / "latest.json"
    out.write_text(json.dumps(manifest, indent=2) + "\n")
    print(f"wrote {out} for {version}: {', '.join(installers)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
