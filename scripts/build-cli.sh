#!/usr/bin/env bash
# Build the mocky-mail command line (apps/cli) and put it where Tauri looks
# for the sidecar it bundles with the desktop installers:
# apps/desktop/src-tauri/binaries/mocky-mail-<target triple>[.exe].
#
# Then build the installers with the sidecar configuration:
#   cd apps/desktop && npx @tauri-apps/cli@2.12.4 build \
#     --config src-tauri/tauri.cli.conf.json [--target <triple>]
#
# Usage: scripts/build-cli.sh [<target triple> | host | universal-apple-darwin]
# "host" (the default) builds for this machine. universal-apple-darwin
# builds both Mac processors and joins them with lipo.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
out="$root/apps/desktop/src-tauri/binaries"
target="${1:-host}"
host="$(rustc -vV | sed -n 's/^host: //p')"
mkdir -p "$out"

build() {
    # The release profile of the workspace; the target directory is the
    # workspace's own (or CARGO_TARGET_DIR).
    if [ "$1" = "$host" ]; then
        cargo build --release --locked -p mocky-mail-cli --manifest-path "$root/Cargo.toml"
    else
        cargo build --release --locked -p mocky-mail-cli --manifest-path "$root/Cargo.toml" --target "$1"
    fi
}

built() {
    local dir="${CARGO_TARGET_DIR:-$root/target}"
    local ext=""
    case "$1" in *windows*) ext=".exe" ;; esac
    if [ "$1" = "$host" ]; then
        echo "$dir/release/mocky-mail$ext"
    else
        echo "$dir/$1/release/mocky-mail$ext"
    fi
}

if [ "$target" = "universal-apple-darwin" ]; then
    for arch in aarch64-apple-darwin x86_64-apple-darwin; do
        build "$arch"
    done
    lipo -create -output "$out/mocky-mail-universal-apple-darwin" \
        "$(built aarch64-apple-darwin)" "$(built x86_64-apple-darwin)"
    echo "$out/mocky-mail-universal-apple-darwin"
    exit 0
fi

[ "$target" = "host" ] && target="$host"
build "$target"
ext=""
case "$target" in *windows*) ext=".exe" ;; esac
cp "$(built "$target")" "$out/mocky-mail-$target$ext"
echo "$out/mocky-mail-$target$ext"
