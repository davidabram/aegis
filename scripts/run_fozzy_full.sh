#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="${ROOT_DIR}/.fozzy/aegis"
SCENARIOS=(
  "tests/aegis_core.fozzy.json"
  "tests/aegis_host_backed.fozzy.json"
)

cd "${ROOT_DIR}"
mkdir -p "${OUT_DIR}"

echo "[fozzy] capturing environment"
fozzy env --json > "${OUT_DIR}/env.json"

echo "[fozzy] mapping suite coverage"
fozzy map suites --root . --scenario-root tests --profile pedantic --json \
  > "${OUT_DIR}/map.suites.json"

for scenario in "${SCENARIOS[@]}"; do
  name="$(basename "${scenario}" .fozzy.json)"
  echo "[fozzy] validating ${scenario}"
  fozzy validate "${scenario}" --json > "${OUT_DIR}/${name}.validate.json"
done

for scenario in "${SCENARIOS[@]}"; do
  name="$(basename "${scenario}" .fozzy.json)"
  out_json="${OUT_DIR}/${name}.run.json"
  echo "[fozzy] running ${scenario}"
  fozzy run "${scenario}" \
    --json \
    --proc-backend host \
    --http-backend host \
    --fs-backend host > "${out_json}"
done

echo "[fozzy] full gate passed"
