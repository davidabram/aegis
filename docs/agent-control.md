# Aegis Agent Control Guide

This is the practical control surface for agents.

The production rule is simple:

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
- `--user-data-dir <path>`
- `--host-lib <path>`

On macOS, runtime-backed CLI commands are re-execed through the bundled app path automatically.

## Fast Start

Inspect native paths:

```bash
cargo run -- native paths
```

Start the API server:

```bash
cargo run -- \
  --host-lib ./native/build-xcode/Debug/libaegis_host.dylib \
  --mode headful \
  serve --addr 127.0.0.1:7878
```

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
cargo run -- trace replay traces/run.json
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

### Snapshot DOM

```bash
curl http://127.0.0.1:7878/dom
```

### Events

```bash
curl 'http://127.0.0.1:7878/events?since=0'
```

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
3. snapshot the DOM
4. locate target node IDs from the snapshot
5. execute `click` / `set_value` / `eval`
6. read incremental events with `since=<latest_sequence>`
7. snapshot session if login or state matters
8. enable traces for runs you may need to replay

## Constraints

- CLI runtime commands require `--host-lib`
- native macOS builds require the local CEF SDK under `third_party/cef/...`
- the published GitHub repo intentionally excludes the vendored CEF binary payload
