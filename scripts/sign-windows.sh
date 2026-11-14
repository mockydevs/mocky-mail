#!/usr/bin/env bash
# Sign a Windows file (the app .exe, the installer and its uninstaller) with
# Mocky Digital's code-signing certificate. Tauri calls this once per file
# when the build is run with `--config src-tauri/tauri.sign.conf.json`.
#
# It fails loudly when no certificate is configured, so a signed release is
# never published unsigned by accident. Choose one method:
#
#   SSL.com eSigner (cloud key, works on Linux):
#     CODESIGNTOOL_DIR   folder holding CodeSignTool.sh
#     ES_USERNAME, ES_PASSWORD, ES_CREDENTIAL_ID, ES_TOTP_SECRET
#
#   A certificate on a PKCS#12 file or token readable by osslsigncode:
#     WINDOWS_SIGN_PFX, WINDOWS_SIGN_PFX_PASSWORD
#
# Never commit these values; load them from a private env file.
set -euo pipefail

file="${1:?usage: sign-windows.sh <file>}"
timestamp="${WINDOWS_SIGN_TIMESTAMP_URL:-http://timestamp.sectigo.com}"
description="Mocky Mail"
url="https://mailer.co.ke"

if [[ -n "${CODESIGNTOOL_DIR:-}" ]]; then
  : "${ES_USERNAME:?}" "${ES_PASSWORD:?}" "${ES_CREDENTIAL_ID:?}" "${ES_TOTP_SECRET:?}"
  out="$(mktemp -d)"
  trap 'rm -rf "$out"' EXIT
  (cd "$CODESIGNTOOL_DIR" && ./CodeSignTool.sh sign \
    -username="$ES_USERNAME" -password="$ES_PASSWORD" \
    -credential_id="$ES_CREDENTIAL_ID" -totp_secret="$ES_TOTP_SECRET" \
    -input_file_path="$file" -output_dir_path="$out")
  mv -f "$out/$(basename "$file")" "$file"
elif [[ -n "${WINDOWS_SIGN_PFX:-}" ]]; then
  : "${WINDOWS_SIGN_PFX_PASSWORD:?}"
  signed="$file.signed"
  osslsigncode sign -pkcs12 "$WINDOWS_SIGN_PFX" -readpass <(printf '%s' "$WINDOWS_SIGN_PFX_PASSWORD") \
    -h sha256 -n "$description" -i "$url" -ts "$timestamp" \
    -in "$file" -out "$signed"
  mv -f "$signed" "$file"
else
  echo "sign-windows.sh: no code-signing certificate configured; refusing to build an unsigned release." >&2
  echo "Set CODESIGNTOOL_DIR and ES_* (SSL.com eSigner) or WINDOWS_SIGN_PFX (see scripts/sign-windows.sh)." >&2
  exit 1
fi

# Fail closed: a release only counts as signed if the signature and its
# timestamp check out against the system's trusted roots.
command -v osslsigncode >/dev/null || { echo "sign-windows.sh: osslsigncode is needed to verify the signature" >&2; exit 1; }
roots="${WINDOWS_SIGN_CA_FILE:-/etc/ssl/certs/ca-certificates.crt}"
if ! osslsigncode verify -CAfile "$roots" -TSA-CAfile "$roots" -in "$file" >/dev/null; then
  echo "sign-windows.sh: signature check failed for $file" >&2
  exit 1
fi
echo "Signed $(basename "$file")"
