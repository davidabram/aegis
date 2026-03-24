# Aegis

Aegis is an agentic web browser.

It gives agents one persistent browser runtime, one local control plane, and one correct production path:

- run `aegis` for the local headful browser app
- start `aegis serve`
- control the browser over the local HTTP API
- run in `headless` or `headful` mode against the same session

## What It Is

Aegis is for agent workflows that need a real browser engine, not a fake DOM.

Core capabilities:

- real browser navigation
- live command execution against the page
- headless and headful control
- DOM snapshots
- ordered runtime events
- session import and export
- deterministic traces
- replayable browser runs

The browser engine is a macOS CEF-backed runtime with a native Cocoa host.

## Production Model

There is one supported runtime model:

- one persistent `serve` process
- one local HTTP API
- one browser session per runtime

There is no production per-command relaunch path.

Local release rule:

- install one stable local release app at `~/Applications/Aegis.app`
- use its bundled CLI as the canonical runtime entrypoint
- do not rebuild or reinstall during normal `aegis` usage
- refresh artifacts explicitly with `cargo build --release && aegis-bin native install`

Runtime state rules:

- browser profiles are instance-local
- session persistence goes through `GET/POST /session`
- trace persistence goes through `POST /trace/enable`
- if `--start-url` is omitted, the runtime boots into a local no-network bootstrap page

## CLI Surface

The main binary is `aegis`.

Human-use shortcut:

- `aegis` with no arguments opens the local headful Aegis app
- `aegis ...` with arguments uses the installed bundled CLI at `~/Applications/Aegis.app/Contents/MacOS/aegis_cli`

Top-level commands:

- `serve`
- `trace replay`
- `native status`
- `native configure`
- `native build`
- `native install`
- `native paths`

Global runtime flags:

- `--mode headless|headful`
- `--start-url <url>`
- `--host-lib <path>`

## Start A Runtime

```bash
aegis \
  --host-lib ./native/build-xcode/Release/libaegis_host.dylib \
  --mode headful \
  serve --addr 127.0.0.1:7878
```

For live agent debugging:

- use `--mode headful`

For unattended execution:

- use `--mode headless`

If `--start-url` is omitted, Aegis starts on the local bootstrap page and only pays network cost
when the agent explicitly navigates.

## HTTP API

Base address defaults to `http://127.0.0.1:7878`.

Core routes:

- `GET /healthz`
- `GET /runtime`
- `POST /navigate`
- `POST /execute`
- `GET /dom`
- `GET /events`
- `GET /session`
- `POST /session`
- `POST /trace/enable`

### `GET /runtime`

Returns:

- `host_library`
- `browser`
- `runtime`

The `runtime` object includes:

- `bootstrapped`
- `bootstrap_duration_ms`
- `dom_nodes`
- `latest_event_sequence`

### `POST /navigate`

Navigate the live browser session:

```bash
curl -X POST http://127.0.0.1:7878/navigate \
  -H 'content-type: application/json' \
  -d '{"url":"https://example.com"}'
```

### `POST /execute`

Run commands against the current page:

```bash
curl -X POST http://127.0.0.1:7878/execute \
  -H 'content-type: application/json' \
  -d '{
    "commands": [
      {"type":"eval","code":"document.title"}
    ]
  }'
```

Supported command types:

- `eval`
- `click`
- `set_value`
- `scroll`

Execution model:

- `navigate` returns ordered navigation/events quickly and invalidates the cached DOM tree
- `GET /dom` or a node-ID command such as `click` / `set_value` materializes a fresh DOM snapshot on demand
- `execute` can omit the snapshot for low-latency commands like `eval` and `scroll`
- incremental state flows through `GET /events`

### `GET /dom`

Return the current DOM snapshot:

```bash
curl http://127.0.0.1:7878/dom
```

### `GET /events`

Read ordered runtime events:

```bash
curl 'http://127.0.0.1:7878/events?since=0'
```

Event stream semantics:

- events are ordered
- `sequence` is monotonically increasing
- `GET /events` drains pending native/browser events into the runtime stream before it responds
- `since=<n>` returns events with `sequence > n`

Runtime event types:

- `dom_mutation`
- `navigation`
- `network`
- `log`

### `GET/POST /session`

Import or export session state:

- cookies
- local storage
- session storage
- network overrides

### `POST /trace/enable`

Enable deterministic trace capture:

```bash
curl -X POST http://127.0.0.1:7878/trace/enable \
  -H 'content-type: application/json' \
  -d '{"path":"traces/run.json"}'
```

Replay later with:

```bash
aegis trace replay traces/run.json
```

## Agent Loop

The canonical control loop is:

1. start `serve`
2. check `GET /runtime`
3. `POST /navigate`
4. inspect `GET /dom` or `GET /events`
5. `POST /execute`
6. continue from `GET /events?since=<latest_sequence>`
7. persist state with `GET /session` if needed
8. enable traces for important runs

## Startup Measurement

Measure cold-start and first-command latency with the real bundled runtime path:

```bash
python3 scripts/measure_startup.py --mode headful
```

The report includes:

- `runtime_ready_ms`
- `first_command_ms`
- `runtime_before`
- `first_execute`
- `runtime_after`

## Native Runtime

Aegis uses a local native host library:

- `native/build-xcode/Release/libaegis_host.dylib`

Native helper commands:

```bash
aegis native status
aegis native configure
aegis native build
aegis native install
aegis native paths
```

Install a stable local release app bundle:

```bash
cargo build --release
aegis-bin native install
```

That installs a locally ad hoc signed app at `~/Applications/Aegis.app`, clears quarantine
attributes, and gives the runtime a stable local app path without requiring a paid Apple
Developer account.

## Local Signing Limits

Without a paid Apple Developer account, Aegis can still do a lot locally:

- build release binaries
- use one stable installed app path
- ad hoc sign the local app bundle
- clear quarantine attributes on that local bundle

What local-only setup cannot bypass:

- macOS Automation / Accessibility / similar privacy approvals
- the benefits of Developer ID signing and notarization

Those system approvals still require one user approval path in macOS.

## Dependencies

The macOS native build expects the local CEF SDK at:

- `third_party/cef/cef_binary_146.0.6+g68649e2+chromium-146.0.7680.154_macosarm64`

That binary payload is intentionally not tracked in Git.

## Docs

The practical agent guide lives at:

- [docs/agent-control.md](/Users/deepsaint/Desktop/aegis/docs/agent-control.md)
