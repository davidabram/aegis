# Aegis Agent Control Guide

This is the practical control surface for agents.

The production rule is simple:

- run `aegis` with no arguments for the local human browser app
- run one persistent `serve` process
- control that runtime over the HTTP API
- use `headless` or `headful` as modes of that same runtime

CLI guidance:

- `aegis --help` gives the high-level command map and quick starts
- `aegis usage` prints the recommended production workflow
- `aegis examples` prints copy-pasteable commands for common tasks

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
- `--profile <name>`

Production state model:

- Chromium browser profiles are ephemeral and not a persistence API
- Chromium credential storage and autofill persistence are disabled in the runtime
- Aegis-owned state lives under `~/.aegis` by default, or `$AEGIS_HOME` if set
- session continuity goes through `GET /session`, `POST /session`, `POST /session/save`, and `POST /session/load`
- the active profile persists to `~/.aegis/profiles/<profile>/session.json`
- concern-specific local settings belong in `~/.aegis/settings/*.json`
- `~/.aegis/settings/credentials.json` controls Aegis-owned login capture
- Aegis-owned secrets belong in `~/.aegis/secrets/profiles/<profile>/secrets.json`
- saved browser credentials live under each profile's `secrets.credentials.entries`
- traces go through `POST /trace/enable`
- if `--start-url` is omitted, the runtime boots into a local no-network bootstrap page
- Aegis does not use Chrome/Brave Safe Storage or browser login/profile databases in the production path

For local production-like use, the canonical path is one installed release app at
`~/Applications/Aegis.app` plus its bundled CLI at
`~/Applications/Aegis.app/Contents/MacOS/aegis_cli`, fronted by the installed shell launcher at
`~/.local/bin/aegis` or `~/bin/aegis`.
Normal `aegis` usage should not rebuild or reinstall anything.

## Fast Start

Inspect native paths:

```bash
aegis native paths
```

Install or refresh the stable local release app:

```bash
./install.sh
```

Inspect or set Aegis-owned config:

```bash
aegis config get agent
aegis config set agent --json '{"default_profile":"work"}'
aegis config get credentials
aegis config set credentials --json '{"auto_store":false}'
```

Inspect or set Aegis-owned secrets:

```bash
aegis config secrets-get --profile work
aegis config secrets-set --profile work --json '{"github":{"username":"saint","password":"..."},"api_keys":{"openai":"..."}}'
```

Manage Aegis-owned saved browser credentials:

```bash
aegis config credentials-list --profile work
aegis config credentials-set --profile work --json '{"origin":"https://github.com","username":"saint","password":"...","username_field":"login","password_field":"password","form_label":"Sign in"}'
aegis config credentials-remove --profile work --origin https://github.com --username saint
aegis config credentials-clear --profile work
```

Secrets rules:

- secrets live only in `~/.aegis/secrets/...`
- Aegis never reads Chrome/Brave Safe Storage or browser login databases
- Aegis auto-stores credentials by default when it sees username/password entry followed by a submit-like click in the active profile
- users can disable that behavior in `~/.aegis/settings/credentials.json`
- users can inspect or clean up cached credentials entirely through the CLI

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
python3 scripts/measure_startup.py --mode headful --samples 5
```

The startup harness now measures the local bootstrap page by default so cold-start timings are not
inflated by external network fetches.

It also reports:

- `process_spawn_ms`
- `serve_ready_banner_ms`
- `runtime_poll_attempts`
- `runtime_ready_ms`
- `first_command_ms`

With `--samples > 1` it also prints median and max timings plus the full report for each sample.

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
- `runtime`
- `startup`
- `profile`

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

### Save Active Profile

```bash
curl -X POST http://127.0.0.1:7878/session/save
```

### Load Active Profile

```bash
curl -X POST http://127.0.0.1:7878/session/load
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

Canonical targeting rule:

- prefer semantic `match` targets for `click` and `set_value`
- use raw node `id` only for short-lived follow-up actions against a freshly materialized DOM
- on reactive pages, node ids are not a stable long-term contract

Matcher example:

```bash
curl -X POST http://127.0.0.1:7878/execute \
  -H 'content-type: application/json' \
  -d '{
    "commands": [
      {
        "type": "set_value",
        "match": {
          "control_type": "searchbox",
          "name": "Search with DuckDuckGo",
          "actionable": true
        },
        "value": "browser automation"
      },
      {
        "type": "click",
        "match": {
          "control_type": "submit",
          "name": "Search",
          "actionable": true
        }
      }
    ]
  }'
```

Supported matcher fields:

- `role`
- `name`
- `label`
- `control_type`
- `tag`
- `text`
- `placeholder`
- `href_contains`
- `actionable`
- `disabled`

Notes:
- `navigate` returns quickly with ordered navigation/events and invalidates the cached DOM tree
- `GET /dom` or a DOM-targeting command such as `click` / `set_value` repopulates the DOM snapshot on demand
- `execute` may return `"snapshot": null` for low-latency commands such as `eval` and `scroll`
- agents should treat the event stream as the incremental source of truth between full snapshots
- the most reliable loop on live sites is `navigate -> /dom -> execute(match...)`

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
