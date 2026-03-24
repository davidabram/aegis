#!/usr/bin/env python3

import argparse
import json
import os
import signal
import subprocess
import sys
import time
import urllib.request
from pathlib import Path


def http_get_json(url: str, timeout: float) -> dict:
    with urllib.request.urlopen(url, timeout=timeout) as response:
        return json.loads(response.read().decode())


def http_post_json(url: str, payload: dict, timeout: float) -> dict:
    data = json.dumps(payload).encode()
    request = urllib.request.Request(
        url,
        data=data,
        headers={"content-type": "application/json"},
    )
    with urllib.request.urlopen(request, timeout=timeout) as response:
        return json.loads(response.read().decode())


def wait_for_runtime(base_url: str, timeout_s: float) -> tuple[float, dict]:
    started = time.time()
    while time.time() - started < timeout_s:
        try:
            runtime = http_get_json(f"{base_url}/runtime", timeout=1.0)
            return time.time() - started, runtime
        except Exception:
            time.sleep(0.05)
    raise TimeoutError("runtime did not become ready in time")


def main() -> int:
    parser = argparse.ArgumentParser(description="Measure Aegis cold-start and first-command latency.")
    parser.add_argument("--addr", default="127.0.0.1:7915")
    parser.add_argument("--mode", choices=("headless", "headful"), default="headless")
    parser.add_argument("--start-url", default="https://example.com")
    parser.add_argument("--host-lib", default="native/build-xcode/Debug/libaegis_host.dylib")
    parser.add_argument(
        "--binary",
        default="target/aarch64-apple-darwin/debug/aegis",
        help="Path to the Aegis CLI binary to launch",
    )
    parser.add_argument("--timeout", type=float, default=20.0)
    parser.add_argument("--debug-log", default="/tmp/aegis-measure-startup.log")
    args = parser.parse_args()

    root = Path(__file__).resolve().parents[1]
    binary = root / args.binary
    base_url = f"http://{args.addr}"

    env = os.environ.copy()
    env["AEGIS_BUNDLED_CLI"] = "1"
    env["AEGIS_DEBUG_LOG"] = args.debug_log

    command = [
        str(binary),
        "serve",
        "--addr",
        args.addr,
        "--mode",
        args.mode,
        "--start-url",
        args.start_url,
        "--host-lib",
        args.host_lib,
    ]

    process = subprocess.Popen(
        command,
        cwd=root,
        env=env,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )

    try:
        runtime_ready_s, runtime_before = wait_for_runtime(base_url, args.timeout)

        first_command_started = time.time()
        first_execute = http_post_json(
            f"{base_url}/execute",
            {"commands": [{"type": "eval", "code": "document.title"}]},
            timeout=args.timeout,
        )
        first_command_s = time.time() - first_command_started

        runtime_after = http_get_json(f"{base_url}/runtime", timeout=1.0)

        report = {
            "addr": args.addr,
            "mode": args.mode,
            "start_url": args.start_url,
            "runtime_ready_ms": round(runtime_ready_s * 1000, 1),
            "first_command_ms": round(first_command_s * 1000, 1),
            "runtime_before": runtime_before,
            "first_execute": first_execute,
            "runtime_after": runtime_after,
            "debug_log": args.debug_log,
        }
        print(json.dumps(report, indent=2))
        return 0
    finally:
        try:
            process.terminate()
            process.wait(timeout=5)
        except Exception:
            try:
                os.kill(process.pid, signal.SIGKILL)
            except Exception:
                pass


if __name__ == "__main__":
    sys.exit(main())
