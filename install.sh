#!/usr/bin/env bash
# Install or update Mocky Mail on Debian, Ubuntu, Linux Mint, Pop!_OS and
# other apt-based Linux systems:
#
#   curl -fsSL https://raw.githubusercontent.com/mockydevs/mocky-mail/main/install.sh | bash
#
# It downloads the latest .deb from https://mailer.co.ke/downloads/, checks
# its SHA-256 against the published SHA256SUMS, and installs it with apt (so
# apt can update and remove it). It asks for your password only for apt.
#
# Options:
#   --dry-run     show what would happen; change nothing
#   --uninstall   remove Mocky Mail (your mail and settings stay)
#   --yes         don't ask before installing
#   --help        show this help
set -euo pipefail

BASE_URL="${MOCKY_MAIL_DOWNLOADS:-https://mailer.co.ke/downloads}"
PACKAGE="mocky-mail"

dry_run=0
assume_yes=0
action="install"

say() { printf '%s\n' "$*"; }
fail() { printf 'Error: %s\n' "$*" >&2; exit 1; }

for arg in "$@"; do
  case "$arg" in
    --dry-run) dry_run=1 ;;
    --uninstall) action="uninstall" ;;
    --yes | -y) assume_yes=1 ;;
    --help | -h) sed -n '2,17p' "$0" 2>/dev/null | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) fail "unknown option: $arg (try --help)" ;;
  esac
done

run() {
  if [ "$dry_run" -eq 1 ]; then
    say "  would run: $*"
  else
    "$@"
  fi
}

as_root() {
  if [ "$(id -u)" -eq 0 ]; then run "$@"; else run sudo "$@"; fi
}

confirm() {
  [ "$assume_yes" -eq 1 ] || [ "$dry_run" -eq 1 ] && return 0
  # Piped through bash, stdin is the script: ask on the terminal instead.
  if [ -r /dev/tty ]; then
    printf '%s [Y/n] ' "$1" > /dev/tty
    read -r reply < /dev/tty || reply=""
    case "$reply" in [nN]*) say "Nothing changed."; exit 0 ;; esac
  fi
}

command -v apt-get > /dev/null 2>&1 || fail "this installer needs an apt-based system (Debian, Ubuntu, Mint, Pop!_OS). For other systems see https://mailer.co.ke"

if [ "$action" = "uninstall" ]; then
  if ! dpkg -s "$PACKAGE" > /dev/null 2>&1; then
    say "Mocky Mail isn't installed."
    exit 0
  fi
  confirm "Remove Mocky Mail? Your mail and settings stay in ~/.local/share."
  as_root apt-get remove -y "$PACKAGE"
  say "Mocky Mail removed."
  exit 0
fi

case "$(dpkg --print-architecture)" in
  amd64) ;;
  *) fail "Mocky Mail for Linux is built for 64-bit Intel/AMD (amd64) only so far." ;;
esac

command -v curl > /dev/null 2>&1 || fail "curl is needed: sudo apt install curl"
command -v sha256sum > /dev/null 2>&1 || fail "sha256sum is needed (part of coreutils)"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

say "Checking the latest version..."
curl -fsSL "$BASE_URL/SHA256SUMS" -o "$work/SHA256SUMS" || fail "couldn't reach $BASE_URL"
# The newest amd64 .deb listed (version-sorted).
line="$(grep -E "  ${PACKAGE}_[0-9][0-9A-Za-z.+~-]*_amd64\.deb$" "$work/SHA256SUMS" | sort -t_ -k2,2V | tail -n 1)"
[ -n "$line" ] || fail "no Linux package is listed at $BASE_URL"
expected="${line%%  *}"
file="${line##*  }"
version="$(printf '%s' "$file" | cut -d_ -f2)"

installed="$(dpkg-query -W -f='${Version}' "$PACKAGE" 2>/dev/null || true)"
if [ -n "$installed" ] && [ "$installed" = "$version" ]; then
  say "Mocky Mail $version is already installed and up to date."
  exit 0
fi

if [ -n "$installed" ]; then
  say "Mocky Mail $installed is installed; updating to $version."
else
  say "Installing Mocky Mail $version."
fi
confirm "Continue?"

say "Downloading $file..."
curl -fL --progress-bar "$BASE_URL/$file" -o "$work/$file" || fail "download failed"
actual="$(sha256sum "$work/$file" | cut -d' ' -f1)"
[ "$actual" = "$expected" ] || fail "the download doesn't match its published checksum; nothing was installed. Please try again."
say "Checksum verified."

as_root apt-get install -y "$work/$file"

if [ "$dry_run" -eq 1 ]; then
  say "Dry run: nothing was installed."
else
  say "Done. Open Mocky Mail from your applications menu."
fi
