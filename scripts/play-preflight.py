#!/usr/bin/env python3
"""Check a release AAB against Google Play's upload and review rules before
uploading it, so a rejection is caught here instead of after the upload.

Usage:
    scripts/play-preflight.py [path/to/app.aab]

With no path it checks the AAB that `tauri android build --aab` writes.
Exit status 0 means every check passed; 1 means at least one failed.

Needs the Android SDK (ANDROID_HOME, for aapt2), JAVA_HOME (for jarsigner
and keytool) and the NDK (NDK_HOME, for llvm-readelf). Uses no network
unless --check-urls is given.
"""

from __future__ import annotations

import argparse
import os
import re
import shutil
import subprocess
import sys
import tempfile
import urllib.request
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ANDROID = ROOT / "apps/desktop/src-tauri/gen/android"
DEFAULT_AAB = ANDROID / "app/build/outputs/bundle/universalRelease/app-universal-release.aab"
# The last versionCode uploaded to Play. Play refuses an upload whose
# versionCode is not higher. Update it after each successful upload.
LAST_VERSION_FILE = ROOT / "docs/play-last-version-code.txt"

PACKAGE = "ke.co.mocky.mail"
# Play's target API rule: new apps and updates must target a recent API.
# 35 was required from August 2025; raise this when Google raises it.
MIN_TARGET_SDK = 35
# Play's limit for the compressed download of the base module and all
# native code is 200 MB. Warn well before it.
MAX_AAB_MB = 150
PRIVACY_POLICY_URL = "https://mailer.co.ke/privacy.html"

# Permissions that Play only allows with a policy declaration form, or
# rejects for most apps. Mocky Mail needs none of them.
RESTRICTED_PERMISSIONS = {
    "android.permission.READ_SMS",
    "android.permission.SEND_SMS",
    "android.permission.RECEIVE_SMS",
    "android.permission.READ_CALL_LOG",
    "android.permission.WRITE_CALL_LOG",
    "android.permission.PROCESS_OUTGOING_CALLS",
    "android.permission.MANAGE_EXTERNAL_STORAGE",
    "android.permission.QUERY_ALL_PACKAGES",
    "android.permission.REQUEST_INSTALL_PACKAGES",
    "android.permission.ACCESS_BACKGROUND_LOCATION",
    "android.permission.READ_MEDIA_IMAGES",
    "android.permission.READ_MEDIA_VIDEO",
    "android.permission.USE_FULL_SCREEN_INTENT",
    "android.permission.SYSTEM_ALERT_WINDOW",
    "android.permission.BIND_ACCESSIBILITY_SERVICE",
    "android.permission.SCHEDULE_EXACT_ALARM",
    "android.permission.FOREGROUND_SERVICE_SPECIAL_USE",
}

# 32-bit ABIs need their 64-bit partner (Play's 64-bit rule).
ABI_PAIRS = {"armeabi-v7a": "arm64-v8a", "x86": "x86_64"}
# 64-bit ABIs whose libraries must use 16 KB pages (required for apps
# targeting Android 15+ since November 2025).
PAGE_16K_ABIS = {"arm64-v8a", "x86_64"}


class Report:
    def __init__(self) -> None:
        self.failed = 0
        self.warned = 0

    def ok(self, what: str) -> None:
        print(f"  PASS  {what}")

    def fail(self, what: str) -> None:
        self.failed += 1
        print(f"  FAIL  {what}")

    def warn(self, what: str) -> None:
        self.warned += 1
        print(f"  WARN  {what}")

    def check(self, passed: bool, what: str, why: str = "") -> None:
        if passed:
            self.ok(what)
        else:
            self.fail(f"{what}{': ' + why if why else ''}")


def tool(env: str, *parts: str) -> Path | None:
    base = os.environ.get(env)
    if not base:
        return None
    path = Path(base).expanduser().joinpath(*parts)
    return path if path.exists() else None


def aapt2() -> Path | None:
    tools = tool("ANDROID_HOME", "build-tools")
    if not tools:
        return None
    versions = sorted(tools.iterdir(), key=lambda p: [int(x) for x in re.findall(r"\d+", p.name)])
    for version in reversed(versions):
        if (version / "aapt2").exists():
            return version / "aapt2"
    return None


def run(*args: str | Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run([str(a) for a in args], capture_output=True, text=True, check=False)


def manifest_xml(aab: zipfile.ZipFile, work: Path, aapt: Path) -> str | None:
    """Turn the base module's protobuf manifest into readable XML.

    aapt2 reads proto-format APKs, so the manifest and resource table are
    packed into a small stand-in APK for it."""
    stand_in = work / "base-proto.apk"
    with zipfile.ZipFile(stand_in, "w") as out:
        out.writestr("AndroidManifest.xml", aab.read("base/manifest/AndroidManifest.xml"))
        if "base/resources.pb" in aab.namelist():
            out.writestr("resources.pb", aab.read("base/resources.pb"))
    result = run(aapt, "dump", "xmltree", "--file", "AndroidManifest.xml", stand_in)
    return result.stdout if result.returncode == 0 else None


def attr(xml: str, name: str) -> str | None:
    """First value of an attribute in aapt2's xmltree dump."""
    match = re.search(rf'A: (?:http://schemas\.android\.com/apk/res/android:)?{name}\(0x[0-9a-f]+\)="?([^"\s]+)"?', xml)
    if match:
        return match.group(1)
    match = re.search(rf'A: {name}="([^"]+)"', xml)
    return match.group(1) if match else None


def check_manifest(xml: str, report: Report) -> int | None:
    package = attr(xml, "package")
    report.check(package == PACKAGE, f"package is {PACKAGE}", f"found {package}")

    raw_code = attr(xml, "versionCode")
    version_code = int(raw_code, 0) if raw_code else None
    report.check(version_code is not None, f"versionCode is set ({version_code})")
    if version_code is not None and LAST_VERSION_FILE.exists():
        last = int(LAST_VERSION_FILE.read_text().strip() or "0")
        report.check(version_code > last, f"versionCode {version_code} is higher than the last upload ({last})",
                     "raise the version in tauri.conf.json")

    raw_target = attr(xml, "targetSdkVersion")
    target = int(raw_target, 0) if raw_target else 0
    report.check(target >= MIN_TARGET_SDK, f"targetSdkVersion {target} meets Play's minimum ({MIN_TARGET_SDK})")

    debuggable = re.search(r"debuggable\(0x[0-9a-f]+\)=\(type 0x12\)0xffffffff|debuggable[^\n]*=\"?true", xml)
    report.check(not debuggable, "app is not debuggable")

    cleartext = re.search(r"usesCleartextTraffic\(0x[0-9a-f]+\)=\(type 0x12\)0xffffffff|usesCleartextTraffic[^\n]*=\"?true", xml)
    report.check(not cleartext, "plain-HTTP traffic is off")

    permissions = set(re.findall(r'name\(0x01010003\)="(android\.permission\.[A-Z_]+)"', xml))
    restricted = sorted(permissions & RESTRICTED_PERMISSIONS)
    report.check(not restricted, "no permissions that need a Play declaration", ", ".join(restricted))
    print(f"        permissions: {', '.join(sorted(p.split('.')[-1] for p in permissions)) or 'none'}")
    return version_code


def check_signature(aab: Path, report: Report, expected_sha256: str | None) -> None:
    jarsigner = tool("JAVA_HOME", "bin", "jarsigner")
    keytool = tool("JAVA_HOME", "bin", "keytool")
    if not jarsigner or not keytool:
        report.fail("signature: set JAVA_HOME so jarsigner and keytool can be found")
        return
    verified = run(jarsigner, "-verify", aab)
    report.check(verified.returncode == 0 and "jar verified" in verified.stdout, "AAB is signed",
                 "build with keystore.properties in gen/android (see docs/release-setup.md)")
    cert = run(keytool, "-printcert", "-jarfile", aab).stdout
    owner = re.search(r"Owner: (.*)", cert)
    report.check(bool(owner) and "Android Debug" not in owner.group(1), "signed with the upload key, not the debug key",
                 owner.group(1) if owner else "no certificate")
    fingerprint = re.search(r"SHA256: ([0-9A-F:]+)", cert)
    if expected_sha256 and fingerprint:
        report.check(fingerprint.group(1).upper() == expected_sha256.upper(),
                     "upload key matches the one registered with Play", fingerprint.group(1))
    elif fingerprint:
        print(f"        upload key SHA-256: {fingerprint.group(1)}")


def check_native(aab: zipfile.ZipFile, work: Path, report: Report) -> None:
    libs: dict[str, list[str]] = {}
    for name in aab.namelist():
        match = re.match(r"base/lib/([^/]+)/(.+\.so)$", name)
        if match:
            libs.setdefault(match.group(1), []).append(name)
    if not libs:
        report.warn("no native libraries found (unexpected for a Tauri app)")
        return
    print(f"        native ABIs: {', '.join(sorted(libs))}")
    for abi, partner in ABI_PAIRS.items():
        if abi in libs:
            report.check(partner in libs, f"{abi} has its 64-bit partner {partner}")

    readelf = tool("NDK_HOME", "toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-readelf") or (
        Path(found) if (found := shutil.which("llvm-readelf") or shutil.which("readelf")) else None)
    if not readelf:
        report.fail("16 KB pages: set NDK_HOME so llvm-readelf can be found")
        return
    for abi in sorted(PAGE_16K_ABIS & libs.keys()):
        for name in libs[abi]:
            path = work / Path(name).name
            path.write_bytes(aab.read(name))
            headers = run(readelf, "-lW", path).stdout
            aligns = [int(a, 16) for a in re.findall(r"^\s*LOAD\s+.*\s(0x[0-9a-f]+)\s*$", headers, re.M)]
            report.check(bool(aligns) and min(aligns) >= 0x4000, f"{abi}/{Path(name).name} uses 16 KB pages",
                         f"LOAD alignment {hex(min(aligns)) if aligns else 'unknown'}; link with -z max-page-size=16384")


def check_dev_leftovers(aab: zipfile.ZipFile, report: Report) -> None:
    """A release must not point at the development server."""
    found = []
    for name in aab.namelist():
        if name.startswith("base/assets/") or name.endswith(".dex"):
            data = aab.read(name)
            if b"localhost:1420" in data or b"10.0.2.2:1420" in data:
                found.append(name)
    report.check(not found, "no development-server address in the app", ", ".join(found[:3]))


def check_urls(report: Report) -> None:
    try:
        request = urllib.request.Request(PRIVACY_POLICY_URL, method="GET", headers={"User-Agent": "play-preflight"})
        with urllib.request.urlopen(request, timeout=15) as response:
            report.check(response.status == 200, f"privacy policy loads ({PRIVACY_POLICY_URL})", str(response.status))
    except Exception as error:  # noqa: BLE001 - any failure means the URL is not usable
        report.fail(f"privacy policy loads ({PRIVACY_POLICY_URL}): {error}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("aab", nargs="?", type=Path, default=DEFAULT_AAB)
    parser.add_argument("--upload-key-sha256", default=os.environ.get("PLAY_UPLOAD_KEY_SHA256"),
                        help="expected SHA-256 of the upload certificate")
    parser.add_argument("--check-urls", action="store_true", help="also load the privacy policy URL")
    args = parser.parse_args()

    report = Report()
    print(f"Play preflight: {args.aab}")
    if not args.aab.exists() or args.aab.suffix != ".aab":
        print("  FAIL  no .aab at that path; build with: npm run tauri -- android build --aab")
        return 1

    size_mb = args.aab.stat().st_size / 1_000_000
    report.check(size_mb < MAX_AAB_MB, f"size {size_mb:.1f} MB is under {MAX_AAB_MB} MB")

    aapt = aapt2()
    with zipfile.ZipFile(args.aab) as aab, tempfile.TemporaryDirectory() as tmp:
        work = Path(tmp)
        report.check("BundleConfig.pb" in aab.namelist(), "file is an Android App Bundle")
        if aapt is None:
            report.fail("manifest: set ANDROID_HOME so aapt2 can be found")
        else:
            xml = manifest_xml(aab, work, aapt)
            if xml is None:
                report.fail("manifest could not be read")
            else:
                check_manifest(xml, report)
        check_native(aab, work, report)
        check_dev_leftovers(aab, report)
    check_signature(args.aab, report, args.upload_key_sha256)
    if args.check_urls:
        check_urls(report)

    print()
    if report.failed:
        print(f"{report.failed} check(s) failed, {report.warned} warning(s). Fix these before uploading.")
        return 1
    print(f"All checks passed ({report.warned} warning(s)). Ready to upload.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
