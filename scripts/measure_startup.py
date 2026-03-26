#!/usr/bin/env python3

import argparse
import json
import os
import signal
import socket
import subprocess
import sys
import threading
import time
import urllib.request
from pathlib import Path
from statistics import median

IS_MACOS = sys.platform == "darwin"


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


def wait_for_runtime(base_url: str, timeout_s: float) -> tuple[float, dict, int]:
    started = time.time()
    attempts = 0
    while time.time() - started < timeout_s:
        attempts += 1
        try:
            runtime = http_get_json(f"{base_url}/runtime", timeout=1.0)
            return time.time() - started, runtime, attempts
        except Exception:
            time.sleep(0.05)
    raise TimeoutError("runtime did not become ready in time")


def watch_ready_banner(stream, started_at: float, result: dict) -> None:
    if stream is None:
        return
    try:
        for line in iter(stream.readline, ""):
            if "Aegis serve ready on http://" in line:
                result["serve_ready_banner_ms"] = round((time.time() - started_at) * 1000, 1)
                result["serve_ready_banner_line"] = line.strip()
                break
    finally:
        try:
            stream.close()
        except Exception:
            pass


def ensure_port_free(host: str, port: int) -> None:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.settimeout(0.25)
        if sock.connect_ex((host, port)) != 0:
            return

    subprocess.run(
        ["zsh", "-lc", f"lsof -ti tcp:{port} | xargs -r kill -9"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    time.sleep(0.2)


def main() -> int:
    parser = argparse.ArgumentParser(description="Measure Aegis cold-start and first-command latency.")
    parser.add_argument("--addr", default="127.0.0.1:7915")
    parser.add_argument("--mode", choices=("headless", "headful"), default="headless")
    parser.add_argument(
        "--start-url",
        default="data:text/html,%3C!doctype%20html%3E%3Chtml%3E%3Chead%3E%3Cmeta%20charset%3D%22utf-8%22%3E%3Ctitle%3EAegis%20Bootstrap%3C%2Ftitle%3E%3C%2Fhead%3E%3Cbody%3E%3C%2Fbody%3E%3C%2Fhtml%3E",
    )
    parser.add_argument(
        "--host-lib",
        default=(
            "native/build/macos/Release/libaegis_host.dylib"
            if IS_MACOS
            else "native/build/linux/release/libaegis_host.so"
        ),
    )
    parser.add_argument("--timeout", type=float, default=20.0)
    parser.add_argument("--debug-log", default="/tmp/aegis-measure-startup.log")
    parser.add_argument("--samples", type=int, default=1)
    args = parser.parse_args()

    root = Path(__file__).resolve().parents[1]
    installed_cli = (
        (
            Path.home()
            / "Applications"
            / "Aegis.app"
            / "Contents"
            / "MacOS"
            / "aegis_cli"
        )
        if IS_MACOS
        else Path.home() / ".local" / "share" / "aegis" / "Aegis" / "bin" / "aegis_cli"
    )
    binary = installed_cli if installed_cli.exists() else root / "target" / "release" / "aegis"
    base_url = f"http://{args.addr}"
    host, port_text = args.addr.rsplit(":", 1)
    ensure_port_free(host, int(port_text))

    env = os.environ.copy()
    env["AEGIS_DEBUG_LOG"] = args.debug_log
    env["AEGIS_WORKSPACE_ROOT"] = str(root)
    env["AEGIS_BUNDLED_CLI"] = "1"

    command = [
        str(binary),
        "--mode",
        args.mode,
        "--start-url",
        args.start_url,
        "--host-lib",
        args.host_lib,
        "serve",
        "--addr",
        args.addr,
    ]

    def run_sample(sample_index: int) -> dict:
        debug_log = args.debug_log
        if args.samples > 1:
            debug_path = Path(args.debug_log)
            debug_log = str(
                debug_path.with_name(
                    f"{debug_path.stem}-{sample_index + 1}{debug_path.suffix}"
                )
            )

        sample_env = env.copy()
        sample_env["AEGIS_DEBUG_LOG"] = debug_log

        launch_started_at = time.time()
        process = subprocess.Popen(
            command,
            cwd=root,
            env=sample_env,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
            text=True,
            bufsize=1,
        )
        started_at = time.time()
        banner_info: dict[str, object] = {}
        banner_thread = threading.Thread(
            target=watch_ready_banner,
            args=(process.stderr, started_at, banner_info),
            daemon=True,
        )
        banner_thread.start()

        try:
            runtime_ready_s, runtime_before, runtime_attempts = wait_for_runtime(base_url, args.timeout)

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
                "pid": process.pid,
                "process_spawn_ms": round((started_at - launch_started_at) * 1000, 1),
                "runtime_ready_ms": round(runtime_ready_s * 1000, 1),
                "runtime_poll_attempts": runtime_attempts,
                "first_command_ms": round(first_command_s * 1000, 1),
                "runtime_before": runtime_before,
                "first_execute": first_execute,
                "runtime_after": runtime_after,
                "debug_log": debug_log,
            }
            report.update(banner_info)
            return report
        finally:
            try:
                process.terminate()
                process.wait(timeout=5)
            except Exception:
                try:
                    os.kill(process.pid, signal.SIGKILL)
                except Exception:
                    pass
            banner_thread.join(timeout=0.2)

    if args.samples == 1:
        print(json.dumps(run_sample(0), indent=2))
        return 0

    samples = [run_sample(i) for i in range(args.samples)]
    summary = {
        "samples": args.samples,
        "mode": args.mode,
        "addr": args.addr,
        "median_process_spawn_ms": round(median(sample["process_spawn_ms"] for sample in samples), 1),
        "median_runtime_ready_ms": round(median(sample["runtime_ready_ms"] for sample in samples), 1),
        "median_first_command_ms": round(median(sample["first_command_ms"] for sample in samples), 1),
        "max_runtime_ready_ms": round(max(sample["runtime_ready_ms"] for sample in samples), 1),
        "max_first_command_ms": round(max(sample["first_command_ms"] for sample in samples), 1),
        "sample_reports": samples,
    }
    print(json.dumps(summary, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
