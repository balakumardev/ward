# Building & Signing Ward

This document covers how to produce a distributable `.dmg` of Ward, including
universal-binary (Apple Silicon + Intel) builds and Apple notarization.

The repo does **not** contain any Apple credentials. Signing + notarization
happen in the build environment that supplies the variables below; locally
Ward can be built and run unsigned with the same commands.

## 1. Prerequisites

- **macOS 11.0 (Big Sur)** or newer — bundle's `minimumSystemVersion`
- **Xcode Command Line Tools** — `xcode-select --install`
- **Rust** stable + the two Apple targets:
  ```sh
  rustup target add aarch64-apple-darwin
  rustup target add x86_64-apple-darwin
  ```
- **Node 20+** and `npm`
- **Apple Developer ID certificate** installed in the login keychain
  (for signed builds only) — see "Signing" below.

## 2. Build commands

### 2a. Local unsigned `.dmg` (current arch)

```sh
npm install
npm run tauri build
```

This is the same `npm` script you use today; Tauri reads `src-tauri/tauri.conf.json`
and produces a `.dmg` in `src-tauri/target/release/bundle/dmg/`.

### 2b. Universal binary (Apple Silicon + Intel)

For a single `.dmg` that runs on both architectures:

```bash
src-tauri/dist/sign.sh universal
# or, equivalently:
TAURI_SIGNING_IDENTITY="$APPLE_SIGNING_IDENTITY" \
  npm run tauri build -- --target universal-apple-darwin
```

Tauri 2's build invokes `lipo` to stitch the two `aarch64` / `x86_64`
binaries into one fat binary before bundling. Universal builds roughly
double compile time.

### 2c. Dev / unsigned

```sh
npm run tauri dev      # Vite + Tauri hot reload
npm run tauri build    # local `.dmg`, unsigned unless env vars set
```

## 3. Required environment variables for signing + notarization

None of these are required to **build** Ward. When unset, Tauri skips
signing and notarization, and the build still succeeds — the resulting
`.dmg` simply won't pass Gatekeeper.

| Variable | Purpose | Source |
|----------|---------|--------|
| `APPLE_SIGNING_IDENTITY` | Common name of the "Developer ID Application" certificate in the keychain. Overrides `bundle.macOS.signingIdentity` in `tauri.conf.json`. | `security find-identity -v -p codesigning` |
| `APPLE_ID` | Apple ID email used for notarization submission. | Developer account |
| `APPLE_PASSWORD` | App-specific password for the Apple ID above. | <https://appleid.apple.com/account/manage> → App-Specific Passwords |
| `APPLE_TEAM_ID` | 10-char Apple Developer Team ID. | <https://developer.apple.com/account> → Membership |
| `APPLE_PROVIDER_SHORT_NAME` | (optional) Provider short name if the Apple ID is in multiple teams. Defaults to the user's primary team. | `security find-identity -v` shows the provider |

Tauri reads all five. If `APPLE_SIGNING_IDENTITY` is unset but `APPLE_ID`,
`APPLE_PASSWORD`, and `APPLE_TEAM_ID` are, Tauri tries to infer the cert
from the team. If none are set, no signing happens.

### Setting them

**One-shot** (recommended for CI / local builds):

```bash
export APPLE_SIGNING_IDENTITY="Developer ID Application: Your Name (TEAM12345)"
export APPLE_ID="you@example.com"
export APPLE_PASSWORD="abcd-efgh-ijkl-mnop"   # app-specific, not your account password
export APPLE_TEAM_ID="TEAM12345"
src-tauri/dist/sign.sh universal
```

**Stored** (only on your developer machine): add the exports above to
`~/.zshrc` or `~/.zprofile`. **Never commit these values.**

## 4. `src-tauri/dist/sign.sh`

A wrapper around `npm run tauri build` that:
1. Verifies the required env vars are present (non-empty).
2. Validates the signing identity is actually in the login keychain.
3. Picks the target: `arm64` (default), `x64`, or `universal`.
4. Streams the env vars through to Tauri's CLI.

Source file: `src-tauri/dist/sign.sh`. It is **executable** and **safe to
commit** — no secrets are baked in. There is also
`src-tauri/dist/sign.sh.example`, a copy with the env-var portion
redacted for reference.

## 5. After the `.dmg` is built

1. The artifact lands at:
   ```
   src-tauri/target/release/bundle/dmg/Ward_<version>_<arch>.dmg
   src-tauri/target/release/bundle/macos/Ward.app
   ```
2. Smoke-test on a clean user account (`mv Ward.app /Applications`,
   launch, confirm Gatekeeper accepts it).
3. Verify notarization stapled to the binary:
   ```sh
   spctl --assess --verbose /Applications/Ward.app
   xcrun stapler validate /Applications/Ward.app
   ```
4. `stapler staple Ward.app` if the ticket did not come back from
   `altool` yet (rare — usually automatic after Tauri notarizes).

## 6. Troubleshooting

| Symptom | Cause | Fix |
|---------|-------|-----|
| `cargo: ... no such target` | Apple targets missing | `rustup target add aarch64-apple-darwin x86_64-apple-darwin` |
| Build finishes but `Ward.app` is unsigned | env vars not exported to the build process | Export the four signing vars **before** running `sign.sh`; do not chain them with `&&` after another command. |
| `xcrun notarytool ... 401 unauthorized` | Wrong `APPLE_PASSWORD` or expired | Re-issue an app-specific password at appleid.apple.com. |
| Gatekeeper rejects the `.dmg` | Notarization didn't complete | Re-run `xcrun notarytool submit`; check email for Apple's auto-rejection reasons. |
| `lipo: ... can't open input file` during universal build | Only one of the two targets was compiled | Run `rustup target add` for the missing one, then rebuild. |

## 7. Out of scope for this repo

- **Auto-updater / Squirrel / tauri-plugin-updater**: not wired. If you add
  it later, point `bundle > updater > pubkey` at the RSA public key Tauri
  generates and host the manifest under `https://releases.example.com/`.
- **Notarization for non-`dmg` targets** (App Store `.pkg`): same env vars
  apply, but the bundle target switches from `dmg` to `app` and you submit
  via Transporter rather than `notarytool`.
