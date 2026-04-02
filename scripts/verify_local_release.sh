#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
PLATFORM="$(uname -s)"
DEFAULT_ENTITLEMENTS="${REPO_ROOT}/native/mac/aegis.entitlements"
DOCTOR_JSON=""
INSTALLED_APP=""

cd "${REPO_ROOT}"

if [[ "${AEGIS_CODESIGN_ENTITLEMENTS:-}" == "" && -f "${DEFAULT_ENTITLEMENTS}" ]]; then
  export AEGIS_CODESIGN_ENTITLEMENTS="${DEFAULT_ENTITLEMENTS}"
fi

DOCTOR_JSON="$(cargo run --quiet -- native doctor)"
INSTALLED_APP="$(AEGIS_NATIVE_DOCTOR_JSON="${DOCTOR_JSON}" python3 - <<'PY'
import json
import os
import sys

data = json.loads(os.environ["AEGIS_NATIVE_DOCTOR_JSON"])
value = data.get("canonical_install_dir")
if not isinstance(value, str):
    raise SystemExit("canonical_install_dir missing from native doctor output")
print(value)
PY
)"

echo "==> Installing local release"
cargo run --quiet -- native install

echo "==> Checking native paths"
cargo run --quiet -- native status

if [[ "$PLATFORM" == "Darwin" ]]; then
  echo "==> Verifying bundle signature"
  codesign --verify --strict --verbose=2 "${INSTALLED_APP}"

  if [[ "${AEGIS_CODESIGN_IDENTITY:-}" != "" && "${AEGIS_CODESIGN_IDENTITY:-}" != "-" ]]; then
    echo "==> Assessing bundle with Gatekeeper"
    spctl --assess --type execute --verbose=4 "${INSTALLED_APP}"
  else
    echo "==> Skipping Gatekeeper assessment for ad hoc signature"
  fi
else
  echo "==> Linux install verified at ${INSTALLED_APP}"
fi

echo "==> Running Fozzy full verification"
bash scripts/run_fozzy_full.sh

echo "==> Local release verification complete"
