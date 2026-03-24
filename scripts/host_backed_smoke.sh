#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HOST_LIB="${ROOT_DIR}/native/build-xcode/Release/libaegis_host.dylib"
ADDR="${AEGIS_SMOKE_ADDR:-127.0.0.1:7881}"
BASE_URL="http://${ADDR}"
TRACE_PATH="${ROOT_DIR}/.fozzy/host_backed_trace.fozzy"
PROFILE="host-backed-smoke"

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
  rm -rf "${TMP_DIR}"
}
trap cleanup EXIT

cd "${ROOT_DIR}"
cargo run --quiet -- \
  --host-lib "${HOST_LIB}" \
  --mode headless \
  --profile "${PROFILE}" \
  serve --addr "${ADDR}" >"${SERVER_LOG}" 2>&1 &
SERVER_PID=$!

python3 - <<'PY' "${BASE_URL}" "${SERVER_LOG}"
import json, sys, time, urllib.request, urllib.error

base_url = sys.argv[1]
server_log = sys.argv[2]
deadline = time.time() + 45
last_error = None

while time.time() < deadline:
    try:
        with urllib.request.urlopen(base_url + "/healthz", timeout=2) as response:
            if response.status == 200:
                sys.exit(0)
    except Exception as exc:
        last_error = exc
        time.sleep(0.5)

try:
    with open(server_log, "r", encoding="utf-8") as handle:
        print(handle.read(), file=sys.stderr)
except FileNotFoundError:
    pass

raise SystemExit(f"server failed to become ready: {last_error}")
PY

python3 - <<'PY' "${BASE_URL}" "${TRACE_PATH}"
import json, sys, urllib.parse, urllib.request

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

html = """<!doctype html>
<html>
  <head><title>Aegis Smoke</title></head>
  <body>
    <label for="email">Email</label>
    <input id="email" name="email" type="email" placeholder="Email address" />
    <button id="submit" type="button" onclick="document.title = document.getElementById('email').value">Save</button>
  </body>
</html>"""
data_url = "data:text/html," + urllib.parse.quote(html, safe="")

navigate_events = request("POST", "/navigate", {"url": data_url})
assert any(event["event"]["type"] == "navigation" for event in navigate_events), navigate_events

runtime = request("GET", "/runtime")
assert runtime["diagnostics"]["command_ready"] is True, runtime
assert runtime["diagnostics"]["browser_backend_healthy"] is True, runtime

dom = request("GET", "/dom")
assert dom["nodes"], dom
assert any(node.get("tag") == "input" for node in dom["nodes"]), dom

report = request("POST", "/execute", {
    "commands": [
        {
            "type": "set_value",
            "match": {
                "control_type": "email",
                "placeholder": "Email address"
            },
            "value": "smoke@example.com"
        },
        {
            "type": "click",
            "match": {
                "role": "button",
                "name": "Save"
            }
        },
        {
            "type": "eval",
            "code": "document.title"
        }
    ]
})
assert report["results"][-1]["ok"] is True, report
assert report["results"][-1]["value"] == "smoke@example.com", report

events_window = request("GET", "/events?since=0")
assert events_window["gap_detected"] is False, events_window
assert events_window["latest_sequence"] >= 1, events_window

session = request("GET", "/session")
assert "cookies" in session, session

doctor = request("GET", "/doctor")
assert doctor["command_ready"] is True, doctor
assert doctor["bridge_healthy"] is True, doctor
assert doctor["browser_backend_healthy"] is True, doctor
PY
