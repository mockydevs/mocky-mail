#!/usr/bin/env bash
# Build the Microsoft Store package (MSIX) from the Windows build.
# The Store signs the package itself after certification, so no certificate
# is needed here. Run after:
#   cd apps/desktop && npx @tauri-apps/cli@2.12.4 build --target x86_64-pc-windows-gnu --no-bundle
#
# Required (from Partner Center > the app > Product identity):
#   MSIX_IDENTITY_NAME            Package/Identity/Name
#   MSIX_PUBLISHER                Package/Identity/Publisher (starts with CN=)
#   MSIX_PUBLISHER_DISPLAY_NAME   Package/Properties/PublisherDisplayName
# Needs `makemsix` (Microsoft MSIX SDK) on PATH.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
: "${MSIX_IDENTITY_NAME:?set MSIX_IDENTITY_NAME from Partner Center}"
: "${MSIX_PUBLISHER:?set MSIX_PUBLISHER from Partner Center}"
: "${MSIX_PUBLISHER_DISPLAY_NAME:?set MSIX_PUBLISHER_DISPLAY_NAME from Partner Center}"
command -v makemsix >/dev/null || { echo "package-msix.sh: makemsix not found (Microsoft MSIX SDK)" >&2; exit 1; }

release="$root/target/x86_64-pc-windows-gnu/release"
for f in mocky-mail-desktop.exe WebView2Loader.dll; do
  [[ -f "$release/$f" ]] || { echo "package-msix.sh: missing $release/$f; build the Windows app first" >&2; exit 1; }
done

# The Store needs a four-part version whose last part is 0.
version="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["version"])' "$root/apps/desktop/src-tauri/tauri.conf.json").0"

stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
mkdir -p "$stage/Assets"
cp "$release/mocky-mail-desktop.exe" "$release/WebView2Loader.dll" "$stage/"
icons="$root/apps/desktop/src-tauri/icons"
for a in StoreLogo Square44x44Logo Square150x150Logo Square310x310Logo; do
  cp "$icons/$a.png" "$stage/Assets/"
done

escape() { python3 -c 'import sys,html; print(html.escape(sys.argv[1], quote=True))' "$1"; }
sed -e "s|@IDENTITY_NAME@|$(escape "$MSIX_IDENTITY_NAME")|" \
    -e "s|@PUBLISHER@|$(escape "$MSIX_PUBLISHER")|" \
    -e "s|@PUBLISHER_DISPLAY_NAME@|$(escape "$MSIX_PUBLISHER_DISPLAY_NAME")|" \
    -e "s|@VERSION@|$version|" \
    "$root/packaging/msix/AppxManifest.xml.in" > "$stage/AppxManifest.xml"

out="$root/target/msix"
mkdir -p "$out"
package="$out/mocky-mail_${version}_x64.msix"
rm -f "$package"
makemsix pack -d "$stage" -p "$package"
echo "Built $package"
