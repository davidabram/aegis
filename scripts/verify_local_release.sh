#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
APP_BUNDLE="${HOME}/Applications/Aegis.app"
DEFAULT_ENTITLEMENTS="${REPO_ROOT}/native/mac/aegis.entitlements"

cd "${REPO_ROOT}"

if [[ "${AEGIS_CODESIGN_ENTITLEMENTS:-}" == "" && -f "${DEFAULT_ENTITLEMENTS}" ]]; then
  export AEGIS_CODESIGN_ENTITLEMENTS="${DEFAULT_ENTITLEMENTS}"
fi

echo "==> Installing local release"
cargo run --quiet -- native install

echo "==> Checking native paths"
cargo run --quiet -- native status

echo "==> Verifying bundle signature"
codesign --verify --strict --verbose=2 "${APP_BUNDLE}"

if [[ "${AEGIS_CODESIGN_IDENTITY:-}" != "" && "${AEGIS_CODESIGN_IDENTITY:-}" != "-" ]]; then
  echo "==> Assessing bundle with Gatekeeper"
  spctl --assess --type execute --verbose=4 "${APP_BUNDLE}"
else
  echo "==> Skipping Gatekeeper assessment for ad hoc signature"
fi

echo "==> Running host-backed smoke"
bash scripts/host_backed_smoke.sh

echo "==> Local release verification complete"
