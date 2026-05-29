#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HOST_LIB="${AEGIS_SMOKE_HOST_LIB:-}"
ADDR="${AEGIS_SMOKE_ADDR:-127.0.0.1:7881}"
BASE_URL="http://${ADDR}"
TRACE_PATH="${ROOT_DIR}/.fozzy/host_backed_trace.fozzy"
PROFILE="${AEGIS_SMOKE_PROFILE:-host-backed-smoke}"
MODE="${AEGIS_SMOKE_MODE:-headless}"

DOCTOR_JSON="$(cargo run --quiet -- native doctor)"
INSTALLED_HOST_LIB="$(AEGIS_NATIVE_DOCTOR_JSON="${DOCTOR_JSON}" python3 - <<'PY'
import json
import os
import sys

data = json.loads(os.environ["AEGIS_NATIVE_DOCTOR_JSON"])
value = data.get("canonical_install_host_library")
if isinstance(value, str):
    print(value)
PY
)"
WORKSPACE_HOST_LIB="$(AEGIS_NATIVE_DOCTOR_JSON="${DOCTOR_JSON}" python3 - <<'PY'
import json
import os
import sys

data = json.loads(os.environ["AEGIS_NATIVE_DOCTOR_JSON"])
value = data.get("workspace_host_library")
if not isinstance(value, str):
    raise SystemExit("workspace_host_library missing from native doctor output")
print(value)
PY
)"

WORKSPACE_APP_EXECUTABLE="$(AEGIS_NATIVE_DOCTOR_JSON="${DOCTOR_JSON}" python3 - <<'PY'
import json
import os
import sys

data = json.loads(os.environ["AEGIS_NATIVE_DOCTOR_JSON"])
value = data.get("workspace_app_executable")
if not isinstance(value, str):
    raise SystemExit("workspace_app_executable missing from native doctor output")
print(value)
PY
)"
WORKSPACE_APP_EXECUTABLE_PRESENT="$(AEGIS_NATIVE_DOCTOR_JSON="${DOCTOR_JSON}" python3 - <<'PY'
import json
import os
import sys

data = json.loads(os.environ["AEGIS_NATIVE_DOCTOR_JSON"])
value = data.get("workspace_app_executable_present")
if not isinstance(value, bool):
    raise SystemExit("workspace_app_executable_present missing from native doctor output")
print("true" if value else "false")
PY
)"
WORKSPACE_APP_DIR="$(dirname "$(dirname "${WORKSPACE_APP_EXECUTABLE}")")"

if [[ -z "${HOST_LIB}" ]]; then
  if [[ -f "${WORKSPACE_HOST_LIB}" ]]; then
    if [[ "${WORKSPACE_APP_EXECUTABLE_PRESENT}" != "true" ]]; then
      cargo run --quiet -- native build --configuration release >/dev/null
      DOCTOR_JSON="$(cargo run --quiet -- native doctor)"
      WORKSPACE_HOST_LIB="$(AEGIS_NATIVE_DOCTOR_JSON="${DOCTOR_JSON}" python3 - <<'PY'
import json
import os
import sys

data = json.loads(os.environ["AEGIS_NATIVE_DOCTOR_JSON"])
print(data["workspace_host_library"])
PY
)"
    fi
    HOST_LIB="${WORKSPACE_HOST_LIB}"
  elif [[ -n "${INSTALLED_HOST_LIB}" && -f "${INSTALLED_HOST_LIB}" ]]; then
    HOST_LIB="${INSTALLED_HOST_LIB}"
  else
    HOST_LIB="${WORKSPACE_HOST_LIB}"
  fi
fi

if [[ ! -f "${HOST_LIB}" ]]; then
  echo "missing host library at ${HOST_LIB}" >&2
  exit 1
fi

TMP_DIR="$(mktemp -d)"
SERVER_LOG="${TMP_DIR}/server.log"

cleanup() {
  if [[ -n "${SERVER_PID:-}" ]] && kill -0 "${SERVER_PID}" 2>/dev/null; then
    kill "${SERVER_PID}" 2>/dev/null || true
    for _ in $(seq 1 20); do
      if ! kill -0 "${SERVER_PID}" 2>/dev/null; then
        break
      fi
      sleep 0.5
    done
    kill -9 "${SERVER_PID}" 2>/dev/null || true
    wait "${SERVER_PID}" 2>/dev/null || true
  fi
  pkill -f "${WORKSPACE_APP_DIR}/Contents/Frameworks/aegis_native Helper" 2>/dev/null || true
  pkill -f "${WORKSPACE_APP_DIR}/Contents/MacOS/aegis_native" 2>/dev/null || true
  rm -rf "${TMP_DIR}"
}
trap cleanup EXIT

pkill -f "${WORKSPACE_APP_DIR}/Contents/Frameworks/aegis_native Helper" 2>/dev/null || true
pkill -f "${WORKSPACE_APP_DIR}/Contents/MacOS/aegis_native" 2>/dev/null || true
sleep 1

cd "${ROOT_DIR}"
mkdir -p "$(dirname "${TRACE_PATH}")"
wait_for_server() {
python3 - <<'PY' "${BASE_URL}" "${SERVER_LOG}"
import json, sys, time, urllib.request, urllib.error

base_url = sys.argv[1]
server_log = sys.argv[2]
deadline = time.time() + 45
last_error = None

while time.time() < deadline:
    try:
        with urllib.request.urlopen(base_url + "/healthz", timeout=2) as response:
            if response.status != 200:
                time.sleep(0.5)
                continue
            health = json.loads(response.read().decode("utf-8"))
            if health["command_ready"] is True and health["bridge_healthy"] is True:
                sys.exit(0)
    except Exception as exc:
        last_error = exc
        time.sleep(0.5)

try:
    with open(server_log, "r", encoding="utf-8") as handle:
        print(handle.read(), file=sys.stderr)
except FileNotFoundError:
    pass

raise SystemExit(f"server failed to become command-ready: {last_error}")
PY
}

SERVER_READY=0
for attempt in 1 2 3; do
  cargo run --quiet -- \
    --host-lib "${HOST_LIB}" \
    --mode "${MODE}" \
    --profile "${PROFILE}-${attempt}" \
    serve --addr "${ADDR}" >"${SERVER_LOG}" 2>&1 &
  SERVER_PID=$!

  if wait_for_server; then
    SERVER_READY=1
    break
  fi

  if [[ -n "${SERVER_PID:-}" ]] && kill -0 "${SERVER_PID}" 2>/dev/null; then
    kill "${SERVER_PID}" 2>/dev/null || true
    wait "${SERVER_PID}" 2>/dev/null || true
  fi
  pkill -f "${WORKSPACE_APP_DIR}/Contents/Frameworks/aegis_native Helper" 2>/dev/null || true
  pkill -f "${WORKSPACE_APP_DIR}/Contents/MacOS/aegis_native" 2>/dev/null || true
  sleep 1
done

if [[ "${SERVER_READY}" != "1" ]]; then
  exit 1
fi

python3 - <<'PY' "${BASE_URL}" "${TRACE_PATH}"
import base64, io, json, sys, urllib.parse, urllib.request, wave

base_url = sys.argv[1]
trace_path = sys.argv[2]

def request(method, path, payload=None):
    data = None
    headers = {}
    if payload is not None:
        data = json.dumps(payload).encode("utf-8")
        headers["content-type"] = "application/json"
    req = urllib.request.Request(base_url + path, data=data, headers=headers, method=method)
    with urllib.request.urlopen(req, timeout=10) as response:
        body = response.read()
        if not body:
            return None
        return json.loads(body.decode("utf-8"))

request("POST", "/trace/enable", {"path": trace_path})

wav_buffer = io.BytesIO()
with wave.open(wav_buffer, "wb") as wav_file:
    wav_file.setnchannels(1)
    wav_file.setsampwidth(2)
    wav_file.setframerate(8000)
    wav_file.writeframes(b"\x00\x00" * 8000)
audio_data_url = "data:audio/wav;base64," + base64.b64encode(wav_buffer.getvalue()).decode("ascii")

html = """<!doctype html>
<html>
  <head><title>Aegis Smoke</title></head>
  <body>
    <label for="search">Search</label>
    <input id="search" name="search" type="search" placeholder="Search docs" aria-label="Search docs" />
    <ul>
      <li><a id="primary-link" href="#result" onclick="document.title = 'Result opened'; document.getElementById('status').textContent = 'Result opened'; return false;">Open result</a></li>
      <li>Open result</li>
    </ul>
    <div id="status" role="status">Idle</div>
    <label
      id="marker-option"
      data-testid="marker-option"
      for="marker-type-static"
      onclick="document.getElementById('status').textContent = 'Marker selected';"
      style="display:inline-block;padding:8px;border:1px solid #888;cursor:pointer;"
    >Static marker</label>
    <input id="marker-type-static" type="radio" name="marker-type" value="static" />
    <audio
      id="media-probe"
      data-testid="media-probe"
      tabindex="0"
      controls
      muted
      src="{audio_data_url}"
    ></audio>
    <button
      id="submit"
      type="button"
      onmouseover="document.getElementById('status').textContent = 'Hover ready'"
      onclick="document.title = document.getElementById('search').value; document.getElementById('status').textContent = 'Saved';"
    >Save</button>
  </body>
</html>""".format(audio_data_url=audio_data_url)
data_url = "data:text/html," + urllib.parse.quote(html, safe="")

navigate_events = request("POST", "/navigate", {"url": data_url})
assert any(event["event"]["type"] == "navigation" for event in navigate_events), navigate_events

import time

def wait_for_runtime(timeout_s=10):
    deadline = time.time() + timeout_s
    last = None
    while time.time() < deadline:
        last = request("GET", "/runtime")
        host = last["diagnostics"]["runtime"]["host"]
        if (
            last["diagnostics"]["command_ready"] is True
            and last["diagnostics"]["browser_backend_healthy"] is True
            and host["browser_available"] is True
            and host["renderer_ready"] is True
            and last["diagnostics"]["runtime"]["current_title"] == "Aegis Smoke"
        ):
            return last
        time.sleep(0.1)
    raise AssertionError(last)

runtime = wait_for_runtime()
assert runtime["diagnostics"]["runtime"]["host"]["runtime_ready"] is True, runtime

dom = request("GET", "/dom")
assert dom["nodes"], dom
assert any(node.get("tag") == "input" for node in dom["nodes"]), dom

report = request("POST", "/execute", {
    "commands": [
        {
            "type": "hover",
            "match": {
                "role": "button",
                "name": "Save",
                "actionable": True,
                "exact": True
            }
        },
        {
            "type": "set_value",
            "match": {
                "control_type": "searchbox",
                "placeholder": "Search docs",
                "actionable": True
            },
            "value": "smoke@example.com"
        },
        {
            "type": "press_key",
            "key": "Tab"
        },
        {
            "type": "press_key",
            "key": "Enter",
            "target": {
                "match": {
                    "role": "button",
                    "name": "Save",
                    "actionable": True
                }
            }
        },
        {
            "type": "wait_for",
            "title_contains": "smoke@example.com",
            "text": "Saved",
            "ready_state": "complete",
            "timeout_ms": 2000,
            "poll_interval_ms": 25
        },
        {
            "type": "click",
            "match": {
                "selector": "[data-testid='marker-option']",
                "test_id": "marker-option"
            }
        },
        {
            "type": "wait_for",
            "text": "Marker selected",
            "timeout_ms": 2000,
            "poll_interval_ms": 25
        },
        {
            "type": "press_key",
            "key": "Space",
            "target": {
                "match": {
                    "selector": "[data-testid='media-probe']",
                    "test_id": "media-probe",
                    "tag": "audio"
                }
            }
        },
        {
            "type": "wait_for",
            "media_ready_state_at_least": 1,
            "media_duration_known": True,
            "timeout_ms": 4000,
            "poll_interval_ms": 25
        },
        {
            "type": "click",
            "match": {
                "role": "link",
                "name": "Open result",
                "actionable": True,
                "exact": True
            }
        },
        {
            "type": "wait_for",
            "title_contains": "Result opened",
            "text": "Result opened",
            "timeout_ms": 2000,
            "poll_interval_ms": 25
        },
        {
            "type": "eval",
            "code": "({ title: document.title, status: document.getElementById('status').textContent })"
        }
    ]
})
assert report["results"][-1]["ok"] is True, report
assert report["results"][0]["value"]["hovered"] > 0, report
assert report["results"][0]["value"]["matcher_debug"]["candidate_count"] >= 1, report
assert report["results"][1]["value"]["id"] > 0, report
assert report["results"][3]["value"]["triggered_submit"] is False, report
assert report["results"][4]["ok"] is True, report
assert report["results"][5]["value"]["clicked"] > 0, report
assert report["results"][6]["ok"] is True, report
assert report["results"][7]["value"]["media_toggled"] is True, report
assert report["results"][8]["ok"] is True, report
assert report["results"][9]["value"]["navigation_changed"] is False, report
assert report["results"][10]["ok"] is True, report
assert report["results"][-1]["value"]["title"] == "Result opened", report
assert report["results"][-1]["value"]["status"] == "Result opened", report

events_window = request("GET", "/events?since=0")
assert events_window["gap_detected"] is False, events_window
assert events_window["latest_sequence"] >= 1, events_window

session = request("GET", "/session")
assert "cookies" in session, session

doctor = request("GET", "/doctor")
assert doctor["command_ready"] is True, doctor
assert doctor["bridge_healthy"] is True, doctor
assert doctor["browser_backend_healthy"] is True, doctor
assert doctor["runtime"]["current_title"] == "Result opened", doctor
assert doctor["runtime"]["document_ready_state"] in {"interactive", "complete"}, doctor
assert doctor["runtime"]["host"]["browser_available"] is True, doctor
assert doctor["runtime"]["host"]["renderer_ready"] is True, doctor
assert doctor["runtime"]["host"]["cancel_requested"] is False, doctor
assert len(doctor["runtime"]["media"]) >= 1, doctor
assert doctor["runtime"]["media"][0]["play_attempts"] >= 1, doctor
assert doctor["runtime"]["media"][0]["loaded_metadata_count"] >= 1, doctor
assert doctor["runtime"]["media"][0]["duration"] is not None, doctor
assert doctor["runtime"]["media"][0]["last_event"] is not None, doctor
PY
