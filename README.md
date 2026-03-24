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
- semantic matcher-based control targeting for reactive sites
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
- use `./install.sh` as the canonical one-shot local install path

Runtime state rules:

- Chromium browser profiles are not a persistence API
- Chromium credential storage and autofill persistence are disabled in the production runtime
- persistent agent state lives under `~/.aegis` by default, or `$AEGIS_HOME` if set
- session persistence goes through `GET /session`, `POST /session`, `POST /session/save`, and `POST /session/load`
- `--profile <name>` selects `~/.aegis/profiles/<name>/session.json`
- `~/.aegis/settings/*.json` is the canonical home for concern-specific local settings
- `~/.aegis/settings/credentials.json` controls Aegis-owned login capture behavior
- `~/.aegis/secrets/profiles/<profile>/secrets.json` is the canonical home for Aegis-owned saved secrets
- saved browser credentials live under each profile's `secrets.credentials.entries`
- trace persistence goes through `POST /trace/enable`
- if `--start-url` is omitted, the runtime boots into a local no-network bootstrap page
- the canonical control style is semantic `match` targeting for `click` and `set_value`, not long-lived raw DOM ids
- Aegis does not use Chrome/Brave Safe Storage, browser login DBs, or external keychain-backed browser profile storage anywhere in the production path

## CLI Surface

The main binary is `aegis`.

Human-use shortcut:

- `aegis` with no arguments opens the local headful Aegis app
- `aegis ...` with arguments uses the installed bundled CLI at `~/Applications/Aegis.app/Contents/MacOS/aegis_cli`

Top-level commands:

- `serve`
- `usage`
- `examples`
- `config get`
- `config set`
- `config secrets-get`
- `config secrets-set`
- `config credentials-list`
- `config credentials-set`
- `config credentials-remove`
- `config credentials-clear`
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
- `--profile <name>`

Built-in CLI guidance:

- `aegis --help` gives the high-level command map plus quick starts
- `aegis usage` prints the recommended production workflow
- `aegis examples` prints copy-pasteable commands for common tasks

## Install

One-shot local install:

```bash
./install.sh
```

What it does:

- builds the release binary
- installs `~/Applications/Aegis.app`
- installs the bundled CLI at `~/Applications/Aegis.app/Contents/MacOS/aegis_cli`
- bootstraps and verifies the canonical `~/.aegis` state tree

The older helper path now delegates to the same installer:

```bash
./scripts/install_local_release.sh
```

## Config And Secrets

Inspect or set Aegis-owned config:

```bash
aegis config get agent
aegis config set agent --json '{"default_profile":"work"}'
aegis config get credentials
aegis config set credentials --json '{"auto_store":false}'
```

Inspect or set Aegis-owned per-profile secrets:

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
- Aegis does not read or write Chrome/Brave cookies, login databases, or Safe Storage entries
- Aegis auto-stores credentials by default when it sees username/password entry followed by a submit-like click in the active profile
- users can disable that behavior in `~/.aegis/settings/credentials.json`
- users can inspect and clean up cached credentials through the CLI without touching browser-managed storage

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

The response also includes:

- `profile.profile`
- `profile.path`

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

`eval.code` should be a JavaScript expression such as `document.title`.
Aegis also normalizes a leading `return ...;` if you accidentally send one.

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

### `POST /session/save`

Persist the current runtime session to the active profile file:

```bash
curl -X POST http://127.0.0.1:7878/session/save
```

### `POST /session/load`

Reload the active profile file into the live runtime:

```bash
curl -X POST http://127.0.0.1:7878/session/load
```

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
python3 scripts/measure_startup.py --mode headful --samples 5
```

The report includes:

- `process_spawn_ms`
- `serve_ready_banner_ms`
- `runtime_poll_attempts`
- `runtime_ready_ms`
- `first_command_ms`
- `runtime_before`
- `first_execute`
- `runtime_after`

When `--samples` is greater than `1`, the harness prints median and max timings plus the full per-sample reports.

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
