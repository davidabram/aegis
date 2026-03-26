#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="${ROOT_DIR}/.fozzy/aegis"
CORE_SCENARIO="tests/aegis_core.fozzy.json"
HOST_SCENARIO="tests/aegis_host_backed.fozzy.json"
FUZZ_SCENARIO="tests/aegis_native_server_main_state_config_executor_host_memory_fuzz.fozzy.json"
EXPLORE_SCENARIO="tests/aegis_native_server_main_state_config_executor_explore.fozzy.json"
SHRINK_SCENARIO="tests/aegis_native_server_main_state_config_executor_fail_shrink.fozzy.json"
CORE_TRACE="${OUT_DIR}/core.det.trace.fozzy"
EXPLORE_TRACE="${OUT_DIR}/explore.trace.fozzy"
FAIL_TRACE="${OUT_DIR}/fail.trace.fozzy"
MIN_TRACE="${OUT_DIR}/fail.min.fozzy"

cd "${ROOT_DIR}"
mkdir -p "${OUT_DIR}"

run_json() {
  local output_path="$1"
  shift
  "$@" --json > "${output_path}"
}

echo "[fozzy] env"
run_json "${OUT_DIR}/env.json" fozzy env

echo "[fozzy] usage"
fozzy usage > "${OUT_DIR}/usage.txt"

echo "[fozzy] version"
run_json "${OUT_DIR}/version.json" fozzy version

echo "[fozzy] suite map"
run_json "${OUT_DIR}/map.suites.json" \
  fozzy map suites --root . --scenario-root tests --profile pedantic

for scenario in \
  "${CORE_SCENARIO}" \
  "${HOST_SCENARIO}" \
  "${FUZZ_SCENARIO}" \
  "${SHRINK_SCENARIO}"; do
  name="$(basename "${scenario}" .fozzy.json)"
  echo "[fozzy] validate ${scenario}"
  run_json "${OUT_DIR}/${name}.validate.json" fozzy validate "${scenario}"
done

echo "[fozzy] deterministic tests"
run_json "${OUT_DIR}/test.det.json" \
  fozzy test "${CORE_SCENARIO}" "${FUZZ_SCENARIO}" --det

echo "[fozzy] deterministic anchor trace"
run_json "${OUT_DIR}/core.run.json" \
  fozzy run "${CORE_SCENARIO}" --det --record "${CORE_TRACE}" --record-collision overwrite

echo "[fozzy] verify/replay/ci core trace"
run_json "${OUT_DIR}/core.trace.verify.json" fozzy trace verify "${CORE_TRACE}"
run_json "${OUT_DIR}/core.replay.json" fozzy replay "${CORE_TRACE}"
run_json "${OUT_DIR}/core.ci.json" fozzy ci "${CORE_TRACE}"

echo "[fozzy] host-backed run"
run_json "${OUT_DIR}/host.run.json" \
  fozzy run "${HOST_SCENARIO}" \
    --proc-backend host \
    --http-backend host \
    --fs-backend host

echo "[fozzy] memory/report coverage run"
run_json "${OUT_DIR}/fuzz-signal.run.json" \
  fozzy run "${FUZZ_SCENARIO}" \
    --proc-backend host \
    --http-backend host \
    --fs-backend host \
    --mem-track \
    --mem-artifacts

echo "[fozzy] report/memory/artifacts on deterministic trace"
run_json "${OUT_DIR}/core.report.json" fozzy report show "${CORE_TRACE}" --format json
run_json "${OUT_DIR}/core.memory.top.json" fozzy memory top "${CORE_TRACE}"
run_json "${OUT_DIR}/core.artifacts.ls.json" fozzy artifacts ls "${CORE_TRACE}"

echo "[fozzy] distributed explore"
run_json "${OUT_DIR}/explore.json" \
  fozzy explore "${EXPLORE_SCENARIO}" \
    --schedule fifo \
    --steps 50 \
    --nodes 3 \
    --record "${EXPLORE_TRACE}" \
    --record-collision overwrite
run_json "${OUT_DIR}/explore.trace.verify.json" fozzy trace verify "${EXPLORE_TRACE}"

echo "[fozzy] fail+shrink path"
if fozzy run "${SHRINK_SCENARIO}" --record "${FAIL_TRACE}" --record-collision overwrite --json \
  > "${OUT_DIR}/fail.run.json"; then
  echo "[fozzy] expected ${SHRINK_SCENARIO} to fail" >&2
  exit 1
fi
run_json "${OUT_DIR}/fail.trace.verify.json" fozzy trace verify "${FAIL_TRACE}"
if fozzy replay "${FAIL_TRACE}" --json > "${OUT_DIR}/fail.replay.json"; then
  echo "[fozzy] expected replay of failing trace to fail" >&2
  exit 1
fi
if fozzy shrink "${FAIL_TRACE}" --out "${MIN_TRACE}" --json > "${OUT_DIR}/fail.shrink.json"; then
  :
elif [[ -f "${MIN_TRACE}" ]]; then
  :
else
  echo "[fozzy] shrink did not produce ${MIN_TRACE}" >&2
  exit 1
fi
run_json "${OUT_DIR}/fail.min.verify.json" fozzy trace verify "${MIN_TRACE}"

echo "[fozzy] full gate passed"
