#!/usr/bin/env bash
#
# sign.sh — wrap `npm run tauri build` with the Apple signing + notarization
# environment variables that Tauri 2 reads (APPLE_SIGNING_IDENTITY etc).
#
# This script is safe to commit. It contains NO secrets. Each invocation
# requires the user (or CI) to have set the env vars beforehand.
#
# Usage:
#   src-tauri/dist/sign.sh                  # default arch (host's arch)
#   src-tauri/dist/sign.sh arm64            # Apple Silicon only
#   src-tauri/dist/sign.sh x64              # Intel only
#   src-tauri/dist/sign.sh universal        # Apple Silicon + Intel via lipo
#
# Required env vars (else build succeeds unsigned):
#   APPLE_SIGNING_IDENTITY  - 'Developer ID Application: <name> (<TEAM>)'
#   APPLE_ID                - Apple ID email
#   APPLE_PASSWORD          - App-specific password (not account pwd!)
#   APPLE_TEAM_ID           - 10-char Team ID
#
# Optional:
#   APPLE_PROVIDER_SHORT_NAME - provider short name (multi-team Apple IDs)

set -euo pipefail

# ── Resolve repo root ───────────────────────────────────────────────
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
REPO_ROOT="$( cd "${SCRIPT_DIR}/../.." && pwd )"
cd "${REPO_ROOT}"

# ── Locate tauri CLI ────────────────────────────────────────────────
if ! command -v npm >/dev/null 2>&1; then
  echo "[sign.sh] npm not found in PATH" >&2
  exit 1
fi

# ── Pick target arch ─────────────────────────────────────────────────
TARGET_ARG=""
case "${1:-}" in
  universal)
    TARGET_ARG="--target universal-apple-darwin"
    ;;
  arm64|aarch64)
    # Let tauri default — it picks the host arch.
    :
    ;;
  x64|x86_64)
    TARGET_ARG="--target x86_64-apple-darwin"
    ;;
  "")
    # default — host arch
    :
    ;;
  *)
    echo "[sign.sh] Unknown target: '${1}'. Use 'universal', 'arm64', or 'x64'." >&2
    exit 2
    ;;
esac

# ── Validate signing env vars ───────────────────────────────────────
MISSING=()
for var in APPLE_SIGNING_IDENTITY APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID; do
  if [[ -z "${!var:-}" ]]; then
    MISSING+=("$var")
  fi
done

if [[ ${#MISSING[@]} -gt 0 ]]; then
  echo "[sign.sh] WARNING: these signing env vars are unset:"
  for var in "${MISSING[@]}"; do
    echo "           - $var"
  done
  echo "[sign.sh] Build will succeed but the .dmg will NOT be signed."
  echo "[sign.sh] See BUILD.md for instructions on setting them."
fi

# ── Confirm the signing identity exists in the keychain ────────────
if [[ -n "${APPLE_SIGNING_IDENTITY:-}" ]]; then
  if ! security find-identity -v -p codesigning \
        | grep -F "${APPLE_SIGNING_IDENTITY}" >/dev/null; then
    echo "[sign.sh] ERROR: APPLE_SIGNING_IDENTITY='${APPLE_SIGNING_IDENTITY}'" >&2
    echo "          not found in login keychain." >&2
    echo "          Run: security find-identity -v -p codesigning" >&2
    exit 3
  fi
  echo "[sign.sh] Signing identity present in keychain."
fi

# ── Re-export the vars so npm/t tauri's child process sees them ────
export APPLE_SIGNING_IDENTITY
export APPLE_ID
export APPLE_PASSWORD
export APPLE_TEAM_ID
export APPLE_PROVIDER_SHORT_NAME

# ── Run the tauri build ────────────────────────────────────────────
echo "[sign.sh] Running: npm run tauri build -- ${TARGET_ARG}"
exec npm run tauri build -- ${TARGET_ARG}
