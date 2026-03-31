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
bash ./install.sh

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

if [[ "$PLATFORM" == "Linux" ]]; then
  echo "==> Checking dashboard bootstrap"
  timeout 20s "${INSTALLED_APP}/bin/aegis_cli" --mode headful serve --addr 127.0.0.1:7878 &
  SERVER_PID=$!
  trap 'kill ${SERVER_PID} >/dev/null 2>&1 || true' EXIT
  sleep 5
  curl --fail --silent http://127.0.0.1:7878/healthz >/dev/null
  curl --fail --silent http://127.0.0.1:7878/ui/bootstrap >/dev/null
  curl --fail --silent http://127.0.0.1:7878/ >/dev/null
  kill ${SERVER_PID} >/dev/null 2>&1 || true
  wait ${SERVER_PID} >/dev/null 2>&1 || true
  trap - EXIT
fi

echo "==> Running Fozzy full verification"
bash scripts/run_fozzy_full.sh

echo "==> Local release verification complete"
