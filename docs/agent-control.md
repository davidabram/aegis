# Aegis Agent Control Guide

This is the practical control surface for agents.

The production rule is simple:

- run `aegis` with no arguments for the local human browser app
- run one persistent `serve` process
- control that runtime over the HTTP API
- use `headless` or `headful` as modes of that same runtime

## Mental Model

Aegis exposes one browser runtime behind one control plane.

The agent loop is:

1. start the runtime
2. navigate
3. inspect DOM or events
4. execute commands
5. persist or restore session state
6. optionally enable trace recording for replay

The runtime exposes one production control plane:

- the local HTTP API backed by a persistent `serve` process

## Runtime Modes

- `headless`: automation without a visible window
- `headful`: automation with a visible browser window

Global flags:

- `--mode headless|headful`
- `--start-url <url>`
- `--host-lib <path>`

Production state model:

- runtime browser profiles are instance-local and not a persistence API
- session continuity goes through `GET/POST /session`
- traces go through `POST /trace/enable`
- if `--start-url` is omitted, the runtime boots into a local no-network bootstrap page

On macOS, runtime-backed CLI commands are re-execed through the bundled app path automatically.

For local production-like use, the canonical path is one installed release app at
`~/Applications/Aegis.app` plus its bundled CLI at
`~/Applications/Aegis.app/Contents/MacOS/aegis_cli`.
Normal `aegis` usage should not rebuild or reinstall anything.

## Fast Start

Inspect native paths:

```bash
aegis native paths
```

Install or refresh the stable local release app:

```bash
cargo build --release
aegis-bin native install
```

Start the API server:

```bash
aegis \
  --host-lib ./native/build-xcode/Release/libaegis_host.dylib \
  --mode headful \
  serve --addr 127.0.0.1:7878
```

Measure cold-start and first-command latency:

```bash
python3 scripts/measure_startup.py --mode headful
```

The startup harness now measures the local bootstrap page by default so cold-start timings are not
inflated by external network fetches.

It also reports:

- `process_spawn_ms`
- `serve_ready_banner_ms`
- `runtime_poll_attempts`
- `runtime_ready_ms`
- `first_command_ms`

## Session Shape

`SessionState` shape:

```json
{
  "cookies": [
    {
      "name": "sid",
      "value": "abc",
      "domain": ".example.com",
      "path": "/",
      "expires_unix": 1735689600,
      "secure": true,
      "http_only": true
    }
  ],
  "local_storage": {
    "theme": "dark"
  },
  "session_storage": {
    "flow": "checkout"
  },
  "network_overrides": [
    {
      "header": "x-agent-mode",
      "value": "test"
    }
  ]
}
```

Validation rules:

- cookie names must not be empty
- cookie domains must not be empty
- network override headers must not be empty

Event stream semantics:

- events are ordered
- each event has a monotonically increasing `sequence`
- `--since <n>` returns events with `sequence > n`

Runtime event types:

- `dom_mutation`
- `navigation`
- `network`
- `log`

## Traces

Replay a recorded trace:

```bash
aegis trace replay traces/run.json
```

Use trace recording for:

- deterministic regression capture
- replayable agent runs
- debugging browser/runtime mismatches

## HTTP API

Base address defaults to `http://127.0.0.1:7878`.

### Health

```bash
curl http://127.0.0.1:7878/healthz
```

### Runtime Info

```bash
curl http://127.0.0.1:7878/runtime
```

Returns:

- `host_library`
- `browser`

### Inject Session

```bash
curl -X POST http://127.0.0.1:7878/session \
  -H 'content-type: application/json' \
  -d @session.json
```

### Snapshot Session

```bash
curl http://127.0.0.1:7878/session
```

### Navigate

```bash
curl -X POST http://127.0.0.1:7878/navigate \
  -H 'content-type: application/json' \
  -d '{"url":"https://example.com"}'
```

### Execute

```bash
curl -X POST http://127.0.0.1:7878/execute \
  -H 'content-type: application/json' \
  -d '{
    "commands": [
      {"type":"eval","code":"document.title"}
    ]
  }'
```

Notes:
- `navigate` returns quickly with ordered navigation/events and invalidates the cached DOM tree
- `GET /dom` or a node-ID command such as `click` / `set_value` repopulates the DOM snapshot on demand
- `execute` may return `"snapshot": null` for low-latency commands such as `eval` and `scroll`
- agents should treat the event stream as the incremental source of truth between full snapshots

### Snapshot DOM

```bash
curl http://127.0.0.1:7878/dom
```

### Events

```bash
curl 'http://127.0.0.1:7878/events?since=0'
```

Notes:
- `GET /events` drains pending native/browser events into the runtime stream before it responds

### Enable Trace

```bash
curl -X POST http://127.0.0.1:7878/trace/enable \
  -H 'content-type: application/json' \
  -d '{"path":"traces/run.json"}'
```

## Recommended Agent Pattern

For robust control, use this sequence:

1. start in `headless` for unattended tasks or `headful` for live debugging
2. navigate to the target URL
3. use `eval` / `scroll` immediately if you do not need node IDs yet
4. call `GET /dom` when you need a fresh structural view of the page
5. locate target node IDs from the snapshot
6. execute `click` / `set_value` / `eval`
   `scroll` is also available as a first-class command for viewport movement without ad hoc JS
7. read incremental events with `since=<latest_sequence>`
8. snapshot session if login or state matters
9. enable traces for runs you may need to replay

## Constraints

- `serve` defaults to the release host library if it exists
- the canonical local command path uses the installed bundled CLI, not an on-demand rebuild
- native macOS builds require the local CEF SDK under `third_party/cef/...`
- the published GitHub repo intentionally excludes the vendored CEF binary payload
- local ad hoc signing can reduce repeated local trust noise, but it does not bypass macOS
  Automation, Accessibility, or other privacy approvals
