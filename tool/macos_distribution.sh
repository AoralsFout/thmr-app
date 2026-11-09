#!/bin/bash
set -euo pipefail
app=build/macos/Build/Products/Release/THMR.app
if [ -n "${MACOS_CERTIFICATE_BASE64:-}" ]; then
keychain="$RUNNER_TEMP/thmr-signing.keychain-db"
password=$(openssl rand -hex 24)
trap 'security delete-keychain "$keychain" >/dev/null 2>&1 || true; rm -f "$RUNNER_TEMP/thmr-signing.p12" "$RUNNER_TEMP/thmr-apple-api.p8"' EXIT
python3 - <<'PY'
import base64, os
from pathlib import Path
target = Path(os.environ['RUNNER_TEMP'])
for name in ['MACOS_CODESIGN_IDENTITY', 'APPLE_API_KEY_P8', 'APPLE_API_KEY_ID', 'APPLE_API_ISSUER_ID']:
    if not os.environ.get(name):
        raise SystemExit('Missing macOS distribution secret: ' + name)
(target / 'thmr-signing.p12').write_bytes(base64.b64decode(os.environ['MACOS_CERTIFICATE_BASE64'], validate=True))
(target / 'thmr-apple-api.p8').write_text(os.environ['APPLE_API_KEY_P8'])
for name in ['thmr-signing.p12', 'thmr-apple-api.p8']:
    (target / name).chmod(0o600)
PY
security create-keychain -p "$password" "$keychain"
security unlock-keychain -p "$password" "$keychain"
security import "$RUNNER_TEMP/thmr-signing.p12" -k "$keychain" -P "$MACOS_CERTIFICATE_PASSWORD" -T /usr/bin/codesign
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$password" "$keychain" >/dev/null
python3 tool/macos_app.py sign "$app" --identity "$MACOS_CODESIGN_IDENTITY" --keychain "$keychain"
ditto -c -k --sequesterRsrc --keepParent "$app" "$RUNNER_TEMP/thmr-notarize.zip"
xcrun notarytool submit "$RUNNER_TEMP/thmr-notarize.zip" --wait \
  --key "$RUNNER_TEMP/thmr-apple-api.p8" --key-id "$APPLE_API_KEY_ID" --issuer "$APPLE_API_ISSUER_ID"
xcrun stapler staple "$app"
python3 tool/macos_app.py verify "$app"
else
  echo 'Developer ID not configured: macOS artifacts use local signing and are not notarized.'
  # Replace Xcode-injected entitlements and stale signatures on BOTH slices.
  python3 tool/macos_app.py sign "$app"
fi


# Stage a Flutter-free host application from the same signed audio frameworks.
python3 tool/package_desktop.py --platform macos --bundle "$app" --terminal-only --stage-only
host=build/terminal/macos/THMRHost.app
cli=build/terminal/macos/bin/thmr
if [ -n "${MACOS_CERTIFICATE_BASE64:-}" ]; then
  python3 - "$host" "$MACOS_CODESIGN_IDENTITY" "$keychain" <<'PY'
import sys
from pathlib import Path
sys.path.insert(0, 'tool')
from macos_app import sign_app
sign_app(Path(sys.argv[1]), sys.argv[2], sys.argv[3])
PY
  codesign --force --options runtime --timestamp --sign "$MACOS_CODESIGN_IDENTITY" --keychain "$keychain" "$cli"
  ditto -c -k --sequesterRsrc --keepParent build/terminal/macos "$RUNNER_TEMP/thmr-terminal-notarize.zip"
  xcrun notarytool submit "$RUNNER_TEMP/thmr-terminal-notarize.zip" --wait \
    --key "$RUNNER_TEMP/thmr-apple-api.p8" --key-id "$APPLE_API_KEY_ID" --issuer "$APPLE_API_ISSUER_ID"
  xcrun stapler staple "$host"
else
  python3 - "$host" <<'PY'
import sys
from pathlib import Path
sys.path.insert(0, 'tool')
from macos_app import sign_app
sign_app(Path(sys.argv[1]))
PY
  codesign --force --timestamp=none --sign - "$cli"
fi
codesign --verify --deep --strict "$host"
python3 tool/package_desktop.py --platform macos --archive-only
THMR_NO_SYSTEM_MEDIA=1 python3 tool/test_unified_host.py --bin-dir "$app/Contents/MacOS"

# Both distributions come from the same app, after signing and stapling.
ditto -c -k --sequesterRsrc --keepParent "$app" thmr-app-macos-universal.zip
python3 tool/verify_macos_zip.py thmr-app-macos-universal.zip
bash tool/package_macos_dmg.sh "$app"
if [ -n "${MACOS_CERTIFICATE_BASE64:-}" ]; then
  codesign --force --timestamp --sign "$MACOS_CODESIGN_IDENTITY" \
    --keychain "$keychain" thmr-app-macos-universal.dmg
  xcrun notarytool submit thmr-app-macos-universal.dmg --wait \
    --key "$RUNNER_TEMP/thmr-apple-api.p8" --key-id "$APPLE_API_KEY_ID" --issuer "$APPLE_API_ISSUER_ID"
  xcrun stapler staple thmr-app-macos-universal.dmg
  xcrun stapler validate thmr-app-macos-universal.dmg
fi
python3 tool/verify_macos_dmg.py thmr-app-macos-universal.dmg
