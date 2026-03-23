# Aegis

Aegis is an automated agent browser engine for deterministic, host-backed web execution.

The production automation path is one persistent runtime behind one local control plane:

- start `serve`
- control the runtime over the HTTP API
- run either `headless` or `headful` against that same persistent session

It combines:

- a Rust control plane for navigation, execution, sessions, DOM snapshots, events, and traces
- a native macOS embedded-browser runtime backed by CEF
- a headful standalone browser app with a native Cocoa chrome shell
- deterministic trace recording and replay for testing and regression control
- a local HTTP API for persistent agent control

## What Aegis Does

Aegis is built for agent workflows that need a real browser engine instead of a mock DOM.

Core capabilities:

- navigate and execute commands against a live browser runtime
- run in `headless` or `headful` mode
- capture canonical DOM snapshots
- persist session state
- emit ordered runtime events
- record and replay deterministic traces
- expose one persistent runtime over a local API server
- launch a native standalone macOS browser shell

## Agent Guide

For the practical control surface agents should use, see:

- [docs/agent-control.md](/Users/deepsaint/Desktop/aegis/docs/agent-control.md)

That guide covers:

- the production `serve` control model
- HTTP API routes
- command payloads
- session and event semantics
- trace recording and replay
- recommended agent control flow

## Repository Layout

- `src/`: Rust runtime, CLI, API server, transport bridge, trace system, session handling
- `native/`: native macOS host, standalone app, Cocoa browser shell, CEF integration
- `assets/`: embedded runtime assets injected into the browser
- `tests/`: Rust and scenario tests
- `third_party/`: local native dependencies that are not intended to be published to GitHub

## CLI Surface

The main binary is `aegis`.

Top-level commands:

- `serve`: run the local HTTP API
- `trace`: replay deterministic traces
- `native`: inspect, configure, and build the native macOS runtime

Global runtime controls:

- `--mode headless|headful`
- `--start-url <url>`
- `--user-data-dir <path>`
- `--host-lib <path>`

## Production Control Path

For production automation, treat `serve` plus the HTTP API as the single correct control surface.

That means:

- one persistent runtime per controlled browser session
- no relaunch-per-command bridge behavior
- the same control model for `headless` and `headful`
- state, DOM, events, and trace capture all flowing through the same API session

Recommended startup:

```bash
cargo run -- \
  --host-lib ./native/build-xcode/Debug/libaegis_host.dylib \
  --mode headful \
  serve --addr 127.0.0.1:7878
```

Then drive the runtime over:

- `POST /navigate`
- `POST /execute`
- `GET /dom`
- `GET /events`
- `GET/POST /session`

## Native Browser Engine

The native browser runtime is a macOS CEF-backed engine with two main surfaces:

- embedded runtime host library for agent/runtime control
- standalone `aegis_native.app` for live headful browsing with a native Cocoa shell

The standalone app now uses:

- a native `NSWindow` / `NSView` host container
- native browser chrome controls
- explicit app-owned CEF profile/cache paths
- headful browsing through the embedded CEF runtime

## Native Dependency Provisioning

The macOS native build expects the CEF SDK at:

- `third_party/cef/cef_binary_146.0.6+g68649e2+chromium-146.0.7680.154_macosarm64`

That SDK is intentionally not tracked in git because the binary payload exceeds GitHub file-size limits. Provision it locally before running native configure/build steps.

## Development

Rust tests:

```bash
cargo test
```

CLI help:

```bash
cargo run -- --help
```

Native status / configure / build:

```bash
cargo run -- native status
cargo run -- native configure
cargo run -- native build
```

Run the local API server:

```bash
cargo run -- \
  --host-lib ./native/build-xcode/Debug/libaegis_host.dylib \
  --mode headful \
  serve --addr 127.0.0.1:7878
```

Native macOS app build:

```bash
xcodebuild -project native/build-xcode/aegis_native.xcodeproj \
  -scheme aegis_native \
  -configuration Debug \
  -arch arm64 \
  build
```

## Testing Model

Aegis is designed around deterministic execution and replayable artifacts.

Testing surfaces in this repo include:

- Rust unit/integration tests in `tests/`
- deterministic trace recording and replay in the runtime
- scenario-oriented validation through Fozzy flows and recorded traces

## Current State

What is working in the codebase today:

- Rust control-plane runtime and CLI
- local HTTP API service for persistent runtime control
- deterministic trace record/replay flow
- native macOS CEF-backed standalone app
- headful browser window hosting with native Cocoa chrome

## Notes

- The published GitHub repository intentionally excludes local CEF SDK payloads.
- If native build artifacts are needed locally, keep them under ignored paths such as `third_party/cef/`, `native/build/`, and `native/build-xcode/`.
