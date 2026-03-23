# AEGIS Implementation Plan

This document turns the product spec into an executable engineering checklist for the v1 runtime.

## 1. Delivery Rules

- Build the runtime as a Rust crate with a production-only subsystem boundary around a native embedded-browser host.
- Keep the browser-facing contract deterministic: ordered commands, ordered events, explicit snapshots, no implicit waits.
- Use polling only as a temporary bridge fallback. The long-term contract is event-driven delivery.
- Treat session state as a first-class input/output surface, not a side feature.

## 2. Milestones

### 2.1 Foundation

- ✅ Create the `aegis` crate and top-level module layout.
- ✅ Create `PLAN.md` as the execution checklist and tracking doc.
- ✅ Define the native-host bridge boundary for embedded Chromium integration.
- ✅ Add workspace-level linting and release profiles.

### 2.2 Commands And Runtime

- ✅ Define the public command model for `click`, `set_value`, and `eval`.
- ✅ Implement batch encoding for browser dispatch.
- ✅ Implement a deterministic scheduler stamp for command batches.
- ✅ Implement the runtime executor that updates the DOM mirror from native-host command results.
- ✅ Add a higher-level client surface for runtime/session/navigation ownership.
- ✅ Add production CLI entrypoints on top of the runtime and trace surfaces.
- ✅ Add a production HTTP API service layer for remote agent control.
- [ ] Add retry-free explicit navigation wait primitives driven by events.
- [ ] Add command telemetry and latency histograms.

### 2.3 DOM State

- ✅ Define the canonical DOM node model and snapshot schema.
- ✅ Implement the in-memory DOM tree mirror.
- ✅ Implement a mutation-to-diff reducer for minimal updates.
- [ ] Add resilient node identity remapping for moved nodes.
- [ ] Add attribute/text whitelist configuration per site profile.

### 2.4 Bridge Layer

- ✅ Define a concrete native-host bridge for `eval_js`, `send_batch`, session injection, and event draining.
- ✅ Embed the v1 in-page runtime bundle at `assets/js/aegis_runtime.js`.
- ✅ Wire the bridge around an FFI-ready host API that a CEF wrapper can implement.
- ✅ Replace ad hoc string transport with a framed binary message protocol.
- ✅ Add plugin-loading for native browser host libraries.
- ✅ Add native CEF SDK installation and Xcode project generation into the repo.
- ✅ Add CLI-native status/configure/build surfaces for operator use.

### 2.5 Sessions And Network

- ✅ Define cookie, storage, and session snapshot models.
- ✅ Implement runtime-facing session injection and snapshot APIs.
- ✅ Model network header overrides for auth/session propagation.
- [ ] Add domain/path/expiry validation for cookie injection.
- [ ] Add request/response interception hooks beyond static headers.

### 2.6 Eventing And Replay

- ✅ Define typed runtime events for DOM, navigation, network, and log output.
- ✅ Implement an event stream store with filtered subscription reads.
- ✅ Persist command/event traces to disk.
- ✅ Implement deterministic replay artifacts for recorded traces.
- [ ] Integrate trace validation into CI automation.

### 2.7 Verification

- ✅ Add unit coverage for command encoding, DOM diffs, session validation, and event sequencing.
- [ ] Add host-backed runtime checks against the native browser backend.
- ✅ Add Fozzy scenarios for replay, session injection, and event determinism.
- [ ] Add benchmark coverage for batch latency and snapshot size.

## 3. Build Order

1. ✅ Bootstrap crate structure and public API surface.
2. ✅ Land the native-host bridge plus runtime executor so the command loop is architecturally correct.
3. ✅ Land DOM mirror, diffs, sessions, and event stream integration.
4. ✅ Add the native embedded-browser bridge boundary.
5. ✅ Add deterministic trace recording and replay.
6. [ ] Harden performance and production verification.

## 4. Concrete Checklist By Subsystem

### 4.1 Public Crate Surface

- ✅ Export runtime, commands, DOM, session, transport, and event modules from `src/lib.rs`.
- ✅ Provide an ergonomic `AegisRuntime` facade for agent/tool consumers.
- ✅ Add a higher-level `Client` API for navigation and tab/session orchestration.
- ✅ Add a plugin-loaded production client for native host ownership.

### 4.2 Runtime Execution

- ✅ Accept ordered batches of commands.
- ✅ Encode each batch as a browser transport payload.
- ✅ Return per-command success/error data without aborting the whole batch.
- ✅ Refresh the local DOM mirror from browser state after each batch.
- ✅ Attach monotonic batch IDs to persisted traces.

### 4.3 DOM Mirror

- ✅ Store node IDs, tags, attributes, text, and child IDs.
- ✅ Replace the mirror from a canonical browser snapshot.
- ✅ Apply reduced mutation diffs without requiring a full snapshot.
- [ ] Add local query helpers for tag/attribute lookup.

### 4.4 Sessions

- ✅ Support cookie injection before navigation.
- ✅ Support `localStorage` and `sessionStorage` snapshots and injection.
- ✅ Support network header overrides for auth bootstrap.
- [ ] Support redaction rules for sensitive session exports.

### 4.5 Events

- ✅ Store ordered runtime events with sequence numbers.
- ✅ Filter event reads by event type.
- ✅ Merge bridge events into the runtime-owned stream.
- [ ] Expose async subscriptions for long-lived agent listeners.

### 4.6 Testing And Tooling

- ✅ Add `cargo test` coverage for the production core modules.
- [ ] Add `cargo bench` or Criterion benchmarks.
- ✅ Add Fozzy traces and deterministic scenario coverage.
- [ ] Add host-backed smoke tests when the embedded browser backend lands.

### 4.7 Agent Surfaces

- ✅ Add a production CLI binary for navigation, execution, sessions, traces, and events.
- ✅ Add CLI-native build/status commands so operators can prepare the embedded browser backend from the repo.
- ✅ Add a production HTTP API service for remote agent control.
- ✅ Isolate the native browser runtime behind a single control-plane thread for API use.

## 5. Open Follow-Ups

- [ ] Spec the binary transport protocol for commands, results, snapshots, and events.
- [ ] Define the deterministic replay log format and compatibility guarantees.
- [ ] Define multi-tab process and session ownership.
- [ ] Define the Aria runtime-node adapter API.
