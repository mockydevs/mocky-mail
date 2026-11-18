#!/usr/bin/env python3
"""Write latest.json for the download site, so installed apps offer the update.

Run it in the release step, after the new installers and SHA256SUMS are in the
downloads folder, then upload latest.json next to them:

    scripts/write-latest-json.py path/to/downloads --notes "Faster, more reliable sync."

The manifest is signed with Mocky Digital's update key (Ed25519), kept off the
server at ~/.config/mocky-mail-release/update-signing.key; apps only believe a
manifest signed by the key whose public half is in
apps/desktop/src-tauri/update-key.pub.

It reads the version from apps/desktop/src-tauri/tauri.conf.json, finds that
version's installers in SHA256SUMS and checks each file's SHA-256 before
writing. Installers that aren't there are left out; those installs are sent to
the download page instead.
"""

import argparse
import base64
import hashlib
import json
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
CONFIG = ROOT / "apps/desktop/src-tauri/tauri.conf.json"
PUBLIC_KEY = ROOT / "apps/desktop/src-tauri/update-key.pub"
DEFAULT_KEY = pathlib.Path.home() / ".config/mocky-mail-release/update-signing.key"
INSTALLERS = {
    "linux-deb": "mocky-mail_{version}_amd64.deb",
    "windows-nsis": "mocky-mail_{version}_x64-setup.exe",
}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("downloads", type=pathlib.Path, help="folder with the installers and SHA256SUMS")
    parser.add_argument("--notes", default="", help="one or two lines on what's new, shown in the app")
    parser.add_argument("--base-url", default="https://mailer.co.ke/downloads")
    parser.add_argument("--key", type=pathlib.Path, default=DEFAULT_KEY, help="the private update-signing key (PEM)")
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
    manifest = json.dumps(
        {"version": version, "notes": args.notes.strip(), "installers": installers}, separators=(",", ":")
    )
    from cryptography.hazmat.primitives import serialization
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey

    key = serialization.load_pem_private_key(args.key.read_bytes(), None)
    signature = key.sign(manifest.encode())
    # Never publish what the apps would refuse.
    trusted = Ed25519PublicKey.from_public_bytes(base64.b64decode(PUBLIC_KEY.read_text().strip()))
    trusted.verify(signature, manifest.encode())
    out = args.downloads / "latest.json"
    signed = {"manifest": manifest, "signature": base64.b64encode(signature).decode()}
    out.write_text(json.dumps(signed, indent=2) + "\n")
    print(f"wrote {out} for {version}: {', '.join(installers)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
