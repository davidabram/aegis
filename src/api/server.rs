use std::collections::HashMap;
use std::convert::Infallible;
use std::fs::OpenOptions;
use std::io::Write;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_stream::stream;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::oneshot;
use tokio::time::{sleep, timeout};

use crate::browser::BrowserConfig;
use crate::commands::command::{Command, CommandTarget};
use crate::commands::matcher::resolve_command_target as resolve_snapshot_target;
use crate::config_store::{AegisConfigStore, AegisSecretStore, CredentialInput};
use crate::dom::node::{DomNode, DomSnapshot};
use crate::events::stream::{EventReadWindow, SequencedEvent};
use crate::host::{LoadedAegisClient, RuntimeCancelHandle};
use crate::runtime::executor::{ExecutionReport, RuntimeStatus};
use crate::session::cookies::SessionState;
use crate::session::profile::{SessionProfileInfo, SessionProfileStore};
use crate::transport::bridge::AegisError;
use crate::transport::protocol::PROTOCOL_VERSION;

const HEADLESS_IDLE_PUMP_INTERVAL: Duration = Duration::from_millis(10);
const HEADFUL_IDLE_PUMP_INTERVAL: Duration = Duration::from_millis(2);
const COMMAND_TIMEOUT: Duration = Duration::from_secs(20);
const DEFAULT_EVENT_STREAM_POLL_INTERVAL: Duration = Duration::from_millis(100);
const MIN_EVENT_STREAM_POLL_INTERVAL_MS: u64 = 25;
const MAX_EVENT_STREAM_POLL_INTERVAL_MS: u64 = 1_000;
static TELEMETRY_START: OnceLock<Instant> = OnceLock::new();

#[derive(Clone)]
pub struct ApiState {
    tx: mpsc::Sender<ApiCommand>,
    cancel: Arc<Mutex<Option<RuntimeCancelHandle>>>,
    host_library: PathBuf,
    browser: BrowserConfig,
    startup: Arc<Mutex<ServeStartupMetrics>>,
    profile: SessionProfileInfo,
    diagnostics: Arc<Mutex<ServeDiagnostics>>,
}

#[derive(Clone)]
pub struct ServeRootState {
    host_library: PathBuf,
    browser: BrowserConfig,
    default_context_id: String,
    contexts: Arc<Mutex<HashMap<String, ManagedContext>>>,
}

struct ManagedContext {
    api: ApiState,
    owner_thread: Option<thread::JoinHandle<()>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ServeStartupMetrics {
    client_connect_ms: u64,
    api_bind_ms: u64,
    total_ready_ms: u64,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    version: &'static str,
    protocol_version: u16,
    control_plane_up: bool,
    runtime_state: RuntimeOperationalState,
    command_ready: bool,
    bridge_healthy: bool,
    browser_backend_healthy: bool,
    browser_process_up: bool,
    page_attached: bool,
    renderer_attached: bool,
    dom_snapshot_available: bool,
    event_decoder_ok: bool,
    active_operation: Option<OperationSnapshot>,
    last_failure: Option<FailureSnapshot>,
}

#[derive(Debug, Deserialize)]
pub struct NavigateBody {
    pub url: String,
}

#[derive(Debug, Deserialize)]
pub struct ExecuteBody {
    pub commands: Vec<Command>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContextSummary {
    pub id: String,
    pub default: bool,
    pub host_library: PathBuf,
    pub browser: BrowserConfig,
    pub profile: SessionProfileInfo,
    pub runtime_state: String,
    pub command_ready: bool,
}

#[derive(Debug, Deserialize)]
pub struct CreateContextBody {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub seed_from_context: Option<String>,
    #[serde(default)]
    pub mode: Option<crate::browser::BrowserMode>,
    #[serde(default)]
    pub start_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TraceBody {
    pub path: PathBuf,
}

#[derive(Debug, Deserialize)]
pub struct EventQuery {
    #[serde(default)]
    pub since: u64,
}

#[derive(Debug, Deserialize)]
pub struct EventStreamQuery {
    #[serde(default)]
    pub since: u64,
    #[serde(default)]
    pub poll_ms: Option<u64>,
}

enum ApiCommand {
    InjectSession(SessionState, oneshot::Sender<Result<(), AegisError>>),
    SnapshotSession(oneshot::Sender<Result<SessionState, AegisError>>),
    SaveSessionProfile(oneshot::Sender<Result<SessionProfileInfo, AegisError>>),
    LoadSessionProfile(oneshot::Sender<Result<SessionProfileInfo, AegisError>>),
    Navigate(
        String,
        oneshot::Sender<Result<Vec<SequencedEvent>, AegisError>>,
    ),
    Execute(
        Vec<Command>,
        oneshot::Sender<Result<ExecutionReport, AegisError>>,
    ),
    SnapshotDom(oneshot::Sender<Result<DomSnapshot, AegisError>>),
    Events(u64, oneshot::Sender<Result<EventReadWindow, AegisError>>),
    EnableTrace(PathBuf, oneshot::Sender<Result<(), AegisError>>),
    Shutdown(oneshot::Sender<Result<(), AegisError>>),
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum RuntimeOperationalState {
    Starting,
    Ready,
    Busy,
    Degraded,
    Cancelling,
    Wedged,
}

#[derive(Debug, Clone, Serialize)]
struct OperationSnapshot {
    id: u64,
    name: String,
    stage: String,
    started_at_ms: u64,
    elapsed_ms: u64,
    timed_out: bool,
}

#[derive(Debug, Clone, Serialize)]
struct FailureSnapshot {
    operation: String,
    stage: String,
    message: String,
    elapsed_ms: u64,
    timed_out: bool,
    restart_recommended: bool,
    first_seen_at_ms: u64,
    last_seen_at_ms: u64,
}

#[derive(Debug, Clone)]
struct ActiveOperation {
    id: u64,
    name: String,
    stage: String,
    started_at_ms: u64,
    started_at: Instant,
    timed_out: bool,
}

#[derive(Debug, Clone)]
struct ServeDiagnostics {
    runtime: RuntimeStatus,
    active_operation: Option<ActiveOperation>,
    last_failure: Option<FailureSnapshot>,
    total_operations: u64,
    successful_operations: u64,
    timed_out_operations: u64,
    next_operation_id: u64,
}

#[derive(Debug, Clone, Serialize)]
struct RuntimeDiagnosticsResponse {
    version: &'static str,
    protocol_version: u16,
    state: RuntimeOperationalState,
    control_plane_up: bool,
    command_ready: bool,
    bridge_healthy: bool,
    browser_backend_healthy: bool,
    browser_process_up: bool,
    page_attached: bool,
    renderer_attached: bool,
    dom_snapshot_available: bool,
    event_decoder_ok: bool,
    active_operation: Option<OperationSnapshot>,
    last_failure: Option<FailureSnapshot>,
    total_operations: u64,
    successful_operations: u64,
    timed_out_operations: u64,
    runtime: RuntimeStatus,
}

#[derive(Debug, Deserialize)]
struct NativeOperationError {
    kind: String,
    operation: String,
    stage: String,
    message: String,
    elapsed_ms: u64,
    timed_out: bool,
    restart_recommended: bool,
}

#[derive(Debug, Clone, Default)]
struct AutoCredentialCapture {
    username: Option<CapturedCredentialField>,
    password: Option<CapturedCredentialField>,
    origin: Option<String>,
}

#[derive(Debug, Clone)]
struct CapturedCredentialField {
    value: String,
    field_name: Option<String>,
    label: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CredentialFieldKind {
    Username,
    Password,
}

fn spawn_context_state(
    host_library: PathBuf,
    browser_config: BrowserConfig,
    profile_name: String,
) -> Result<ManagedContext, AegisError> {
    emit_telemetry(
        "context_start",
        json!({
            "browser_mode": browser_config.mode,
            "start_url": browser_config.start_url,
            "profile": profile_name,
        }),
    );
    let serve_started = std::time::Instant::now();
    let profile_store = SessionProfileStore::new(profile_name).map_err(AegisError::Bridge)?;
    let credential_settings = AegisConfigStore::detect()
        .and_then(|store| store.load_credentials_settings())
        .map_err(AegisError::Bridge)?;
    let credential_store = AegisSecretStore::detect().map_err(AegisError::Bridge)?;
    let mut credential_capture = AutoCredentialCapture::default();
    let pending_startup_session = profile_store.load().map_err(AegisError::Bridge)?;
    let (tx, rx) = mpsc::channel::<ApiCommand>();
    let startup = Arc::new(Mutex::new(ServeStartupMetrics {
        client_connect_ms: 0,
        api_bind_ms: 0,
        total_ready_ms: 0,
    }));
    let diagnostics = Arc::new(Mutex::new(ServeDiagnostics::new(default_runtime_status())));
    let cancel = Arc::new(Mutex::new(None));
    let state = ApiState {
        tx,
        cancel: cancel.clone(),
        host_library: host_library.clone(),
        browser: browser_config.clone(),
        startup: startup.clone(),
        profile: profile_store.info(),
        diagnostics: diagnostics.clone(),
    };
    let startup_host_library = state.host_library.clone();
    let idle_pump_interval = match browser_config.mode {
        crate::browser::BrowserMode::Headful => HEADFUL_IDLE_PUMP_INTERVAL,
        crate::browser::BrowserMode::Headless => HEADLESS_IDLE_PUMP_INTERVAL,
    };
    let (startup_tx, startup_rx) = mpsc::channel::<Result<ServeStartupMetrics, AegisError>>();
    let state_for_thread = state.clone();
    let shutdown_state = state_for_thread.clone();
    let owner_thread = thread::spawn(move || {
        let client_connect_started = std::time::Instant::now();
        let mut client = match LoadedAegisClient::connect(
            startup_host_library.clone(),
            browser_config.clone(),
        ) {
            Ok(client) => client,
            Err(error) => {
                let _ = startup_tx.send(Err(error));
                return;
            }
        };
        emit_telemetry(
            "serve_client_connected",
            json!({
                "latency_ms": client_connect_started.elapsed().as_millis() as u64,
                "browser_mode": browser_config.mode,
                "runtime": client.runtime_status(),
            }),
        );
        if let Ok(mut shared_cancel) = cancel.lock() {
            *shared_cancel = Some(client.cancel_handle());
        }
        let startup_metrics = ServeStartupMetrics {
            client_connect_ms: client_connect_started.elapsed().as_millis() as u64,
            api_bind_ms: 0,
            total_ready_ms: serve_started.elapsed().as_millis() as u64,
        };
        if let Ok(mut shared) = startup.lock() {
            *shared = startup_metrics.clone();
        }
        emit_telemetry(
            "serve_ready",
            json!({
                "client_connect_ms": startup_metrics.client_connect_ms,
                "api_bind_ms": startup_metrics.api_bind_ms,
                "total_ready_ms": startup_metrics.total_ready_ms,
                "browser_mode": browser_config.mode,
            }),
        );
        eprintln!(
            "Aegis context ready for profile {} ({:?}, host: {})",
            profile_store.info().profile,
            browser_config.mode,
            startup_host_library.display()
        );
        let _ = startup_tx.send(Ok(startup_metrics));

        let mut pending_startup_session = pending_startup_session;
        loop {
            if let Some(session) = pending_startup_session.take() {
                let started_at = Instant::now();
                record_operation_started(
                    &diagnostics,
                    "startup_restore_session",
                    "restoring persisted startup session",
                );
                emit_telemetry(
                    "startup_session_restore_started",
                    json!({
                        "browser_mode": browser_config.mode,
                        "profile": profile_store.info().profile,
                    }),
                );
                let result = client.inject_session(session);
                record_operation_finished(
                    &diagnostics,
                    "startup_restore_session",
                    &client,
                    &result,
                );
                emit_operation_telemetry(
                    "startup_restore_session",
                    started_at,
                    &result,
                    client.runtime_status(),
                );
                if let Err(error) = result {
                    emit_telemetry(
                        "startup_session_restore_failed",
                        json!({
                            "browser_mode": browser_config.mode,
                            "profile": profile_store.info().profile,
                            "error": error.to_string(),
                            "runtime": client.runtime_status(),
                        }),
                    );
                    break;
                }
                emit_telemetry(
                    "startup_session_restore_completed",
                    json!({
                        "browser_mode": browser_config.mode,
                        "profile": profile_store.info().profile,
                        "latency_ms": started_at.elapsed().as_millis() as u64,
                        "runtime": client.runtime_status(),
                    }),
                );
                continue;
            }

            match rx.recv_timeout(idle_pump_interval) {
                Ok(command) => match command {
                    ApiCommand::InjectSession(session, reply) => {
                        let started_at = Instant::now();
                        record_operation_started(
                            &diagnostics,
                            "inject_session",
                            "injecting session",
                        );
                        let result = client.inject_session(session.clone()).and_then(|_| {
                            profile_store
                                .save(&session)
                                .map(|_| ())
                                .map_err(AegisError::Bridge)
                        });
                        record_operation_finished(&diagnostics, "inject_session", &client, &result);
                        emit_operation_telemetry(
                            "inject_session",
                            started_at,
                            &result,
                            client.runtime_status(),
                        );
                        let _ = reply.send(result);
                    }
                    ApiCommand::SnapshotSession(reply) => {
                        let started_at = Instant::now();
                        record_operation_started(
                            &diagnostics,
                            "snapshot_session",
                            "capturing session state",
                        );
                        let result = client.snapshot_session();
                        record_operation_finished(
                            &diagnostics,
                            "snapshot_session",
                            &client,
                            &result,
                        );
                        emit_operation_telemetry(
                            "snapshot_session",
                            started_at,
                            &result,
                            client.runtime_status(),
                        );
                        let _ = reply.send(result);
                    }
                    ApiCommand::SaveSessionProfile(reply) => {
                        let started_at = Instant::now();
                        record_operation_started(
                            &diagnostics,
                            "save_session_profile",
                            "persisting session profile",
                        );
                        let result = client.snapshot_session().and_then(|session| {
                            profile_store
                                .save(&session)
                                .map(|_| profile_store.info())
                                .map_err(AegisError::Bridge)
                        });
                        record_operation_finished(
                            &diagnostics,
                            "save_session_profile",
                            &client,
                            &result,
                        );
                        emit_operation_telemetry(
                            "save_session_profile",
                            started_at,
                            &result,
                            client.runtime_status(),
                        );
                        let _ = reply.send(result);
                    }
                    ApiCommand::LoadSessionProfile(reply) => {
                        let started_at = Instant::now();
                        record_operation_started(
                            &diagnostics,
                            "load_session_profile",
                            "loading session profile",
                        );
                        let result = profile_store.load().map_err(AegisError::Bridge).and_then(
                            |maybe_session| match maybe_session {
                                Some(session) => {
                                    client.inject_session(session).map(|_| profile_store.info())
                                }
                                None => Ok(profile_store.info()),
                            },
                        );
                        record_operation_finished(
                            &diagnostics,
                            "load_session_profile",
                            &client,
                            &result,
                        );
                        emit_operation_telemetry(
                            "load_session_profile",
                            started_at,
                            &result,
                            client.runtime_status(),
                        );
                        let _ = reply.send(result);
                    }
                    ApiCommand::Navigate(url, reply) => {
                        let started_at = Instant::now();
                        record_operation_started(
                            &diagnostics,
                            "navigate",
                            &format!("navigating to {url}"),
                        );
                        credential_capture.reset_on_explicit_navigation(&url);
                        let result = client.navigate(url);
                        record_operation_finished(&diagnostics, "navigate", &client, &result);
                        emit_operation_telemetry(
                            "navigate",
                            started_at,
                            &result,
                            client.runtime_status(),
                        );
                        let _ = reply.send(result);
                    }
                    ApiCommand::Execute(commands, reply) => {
                        let started_at = Instant::now();
                        record_operation_started(
                            &diagnostics,
                            "execute",
                            "executing browser command batch",
                        );
                        let maybe_snapshot = if credential_settings.auto_store
                            && commands.iter().any(|command| {
                                matches!(command, Command::SetValue { .. } | Command::Click { .. })
                            }) {
                            match client.snapshot_dom() {
                                Ok(snapshot) => Some(snapshot),
                                Err(error) => {
                                    record_operation_failure(
                                        &diagnostics,
                                        "execute",
                                        failure_from_error(
                                            "execute",
                                            "capturing pre-execution DOM snapshot",
                                            &error,
                                        ),
                                        Some(client.runtime_status()),
                                    );
                                    emit_operation_telemetry(
                                        "execute",
                                        started_at,
                                        &Err::<ExecutionReport, AegisError>(AegisError::Bridge(
                                            error.to_string(),
                                        )),
                                        client.runtime_status(),
                                    );
                                    let _ = reply.send(Err(error));
                                    continue;
                                }
                            }
                        } else {
                            None
                        };
                        if let Some(snapshot) = maybe_snapshot.as_ref() {
                            credential_capture.capture_fields(
                                snapshot,
                                client.runtime().current_url(),
                                &commands,
                            );
                        }
                        let should_persist = credential_settings.auto_store
                            && maybe_snapshot.as_ref().is_some_and(|snapshot| {
                                credential_capture.should_persist(snapshot, &commands)
                            });
                        let persist_origin = if should_persist {
                            client.runtime().current_url().map(origin_key)
                        } else {
                            None
                        };
                        let result = client.execute(&commands).and_then(|report| {
                            if let Some(origin) = persist_origin {
                                credential_capture.persist(
                                    &credential_store,
                                    &profile_store.info().profile,
                                    &origin,
                                )?;
                            }
                            Ok(report)
                        });
                        record_operation_finished(&diagnostics, "execute", &client, &result);
                        emit_operation_telemetry(
                            "execute",
                            started_at,
                            &result,
                            client.runtime_status(),
                        );
                        let _ = reply.send(result);
                    }
                    ApiCommand::SnapshotDom(reply) => {
                        let started_at = Instant::now();
                        record_operation_started(
                            &diagnostics,
                            "snapshot_dom",
                            "capturing DOM snapshot",
                        );
                        let result = client.snapshot_dom();
                        record_operation_finished(&diagnostics, "snapshot_dom", &client, &result);
                        emit_operation_telemetry(
                            "snapshot_dom",
                            started_at,
                            &result,
                            client.runtime_status(),
                        );
                        let _ = reply.send(result);
                    }
                    ApiCommand::Events(since, reply) => {
                        let started_at = Instant::now();
                        record_operation_started(&diagnostics, "events", "draining runtime events");
                        let result = client.events_since(since);
                        record_operation_finished(&diagnostics, "events", &client, &result);
                        emit_operation_telemetry(
                            "events",
                            started_at,
                            &result,
                            client.runtime_status(),
                        );
                        let _ = reply.send(result);
                    }
                    ApiCommand::EnableTrace(path, reply) => {
                        let started_at = Instant::now();
                        record_operation_started(
                            &diagnostics,
                            "enable_trace",
                            "enabling trace recording",
                        );
                        client.enable_trace_recording(path);
                        record_operation_finished(&diagnostics, "enable_trace", &client, &Ok(()));
                        emit_operation_telemetry(
                            "enable_trace",
                            started_at,
                            &Ok(()),
                            client.runtime_status(),
                        );
                        let _ = reply.send(Ok(()));
                    }
                    ApiCommand::Shutdown(reply) => {
                        request_runtime_cancel(&shutdown_state);
                        mark_operation_cancel_requested(&diagnostics);
                        let _ = reply.send(Ok(()));
                        break;
                    }
                },
                Err(mpsc::RecvTimeoutError::Timeout) => match client.pump() {
                    Ok(()) => record_heartbeat(&diagnostics, &client),
                    Err(error) => {
                        emit_telemetry(
                            "runtime_pump_failure",
                            json!({
                                "error": error.to_string(),
                                "runtime": client.runtime_status(),
                            }),
                        );
                        record_operation_failure(
                            &diagnostics,
                            "pump",
                            failure_from_error("pump", "pumping browser event loop", &error),
                            Some(client.runtime_status()),
                        );
                        break;
                    }
                },
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }

        if let Ok(session) = client.snapshot_session() {
            let _ = profile_store.save(&session);
        }
    });

    match startup_rx.recv() {
        Ok(Ok(_metrics)) => {}
        Ok(Err(error)) => return Err(error),
        Err(error) => return Err(AegisError::Bridge(error.to_string())),
    }

    Ok(ManagedContext {
        api: state_for_thread,
        owner_thread: Some(owner_thread),
    })
}

pub async fn serve(
    addr: SocketAddr,
    host_library: PathBuf,
    browser_config: BrowserConfig,
    profile_name: String,
) -> Result<(), AegisError> {
    let default_context_id = "default".to_string();
    let default_context =
        spawn_context_state(host_library.clone(), browser_config.clone(), profile_name)?;
    if let Ok(mut startup) = default_context.api.startup.lock() {
        startup.api_bind_ms = 0;
    }
    let root = ServeRootState {
        host_library,
        browser: browser_config,
        default_context_id: default_context_id.clone(),
        contexts: Arc::new(Mutex::new(HashMap::from([(
            default_context_id,
            default_context,
        )]))),
    };

    let bind_started = Instant::now();
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|error| AegisError::Bridge(error.to_string()))?;
    if let Some(default_state) = root.default_context_state() {
        if let Ok(mut startup) = default_state.startup.lock() {
            startup.api_bind_ms = bind_started.elapsed().as_millis() as u64;
        }
    }
    eprintln!("Aegis serve ready on http://{}", addr);
    let app = router(root.clone());
    axum::serve(listener, app)
        .await
        .map_err(|error| AegisError::Bridge(error.to_string()))?;
    shutdown_all_contexts(&root)
        .await
        .map_err(|error| AegisError::Bridge(error.body.error))?;
    Ok(())
}

impl AutoCredentialCapture {
    fn capture_fields(
        &mut self,
        snapshot: &DomSnapshot,
        current_url: Option<&str>,
        commands: &[Command],
    ) {
        let current_origin = current_url.map(origin_key);
        if let (Some(existing), Some(current)) = (self.origin.as_ref(), current_origin.as_ref())
            && existing != current
        {
            self.clear();
        }
        if self.origin.is_none() {
            self.origin = current_origin;
        }

        for command in commands {
            let Command::SetValue { target, value } = command else {
                continue;
            };
            let Some(node) = resolve_command_target(snapshot, target) else {
                continue;
            };
            let Some(kind) = classify_credential_field(node) else {
                continue;
            };
            let field = CapturedCredentialField {
                value: value.clone(),
                field_name: node.attrs.get("name").cloned(),
                label: node
                    .semantic
                    .as_ref()
                    .and_then(|semantic| semantic.label.clone().or_else(|| semantic.name.clone())),
            };
            match kind {
                CredentialFieldKind::Username => self.username = Some(field),
                CredentialFieldKind::Password => self.password = Some(field),
            }
        }
    }

    fn should_persist(&self, snapshot: &DomSnapshot, commands: &[Command]) -> bool {
        self.username.is_some()
            && self.password.is_some()
            && commands.iter().any(|command| {
                let Command::Click { target } = command else {
                    return false;
                };
                resolve_command_target(snapshot, target).is_some_and(is_submit_like_node)
            })
    }

    fn persist(
        &mut self,
        store: &AegisSecretStore,
        profile: &str,
        fallback_origin: &str,
    ) -> Result<(), AegisError> {
        let Some(username) = self.username.as_ref() else {
            return Ok(());
        };
        let Some(password) = self.password.as_ref() else {
            return Ok(());
        };
        store
            .upsert_profile_credential(
                profile,
                CredentialInput {
                    origin: self
                        .origin
                        .clone()
                        .unwrap_or_else(|| fallback_origin.to_string()),
                    username: username.value.clone(),
                    password: password.value.clone(),
                    username_field: username.field_name.clone(),
                    password_field: password.field_name.clone(),
                    form_label: password.label.clone().or_else(|| username.label.clone()),
                },
            )
            .map_err(AegisError::Bridge)?;
        self.clear();
        Ok(())
    }

    fn reset_on_explicit_navigation(&mut self, url: &str) {
        let target_origin = origin_key(url);
        if self
            .origin
            .as_ref()
            .is_some_and(|origin| origin != &target_origin)
        {
            self.clear();
        }
    }

    fn clear(&mut self) {
        self.username = None;
        self.password = None;
        self.origin = None;
    }
}

fn resolve_command_target<'a>(
    snapshot: &'a DomSnapshot,
    target: &CommandTarget,
) -> Option<&'a DomNode> {
    resolve_snapshot_target(snapshot, target, None)
}

fn classify_credential_field(node: &DomNode) -> Option<CredentialFieldKind> {
    let control_type = node
        .semantic
        .as_ref()
        .and_then(|semantic| semantic.control_type.as_deref())
        .or_else(|| node.attrs.get("type").map(String::as_str))
        .unwrap_or("text");
    if includes_normalized(Some(control_type), "password") {
        return Some(CredentialFieldKind::Password);
    }
    let hint = credential_hint_text(node);
    if matches!(
        control_type,
        "email" | "text" | "searchbox" | "search" | "textbox"
    ) && (hint.contains("user")
        || hint.contains("email")
        || hint.contains("login")
        || hint.contains("account")
        || hint.contains("identifier")
        || hint.contains("member"))
    {
        return Some(CredentialFieldKind::Username);
    }
    None
}

fn is_submit_like_node(node: &DomNode) -> bool {
    let semantic = node.semantic.as_ref();
    if semantic.is_some_and(|semantic| semantic.actions.iter().any(|action| action == "submit")) {
        return true;
    }
    if semantic
        .as_ref()
        .and_then(|semantic| semantic.control_type.as_deref())
        .is_some_and(|control| matches!(control, "submit" | "button"))
    {
        return true;
    }
    let text = credential_hint_text(node);
    text.contains("sign in")
        || text.contains("log in")
        || text.contains("login")
        || text.contains("continue")
        || text.contains("submit")
}

fn credential_hint_text(node: &DomNode) -> String {
    let mut parts = Vec::new();
    if let Some(text) = node.text.as_ref() {
        parts.push(text.as_str());
    }
    for key in [
        "name",
        "type",
        "placeholder",
        "title",
        "autocomplete",
        "aria-label",
        "value",
    ] {
        if let Some(value) = node.attrs.get(key) {
            parts.push(value.as_str());
        }
    }
    if let Some(semantic) = node.semantic.as_ref() {
        if let Some(name) = semantic.name.as_ref() {
            parts.push(name.as_str());
        }
        if let Some(label) = semantic.label.as_ref() {
            parts.push(label.as_str());
        }
        if let Some(control_type) = semantic.control_type.as_ref() {
            parts.push(control_type.as_str());
        }
    }
    normalize_text(&parts.join(" "))
}

fn normalize_text(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn includes_normalized(haystack: Option<&str>, needle: &str) -> bool {
    haystack
        .map(normalize_text)
        .is_some_and(|haystack| haystack.contains(&normalize_text(needle)))
}

fn origin_key(url: &str) -> String {
    let trimmed = url.trim();
    if let Some((scheme, rest)) = trimmed.split_once("://") {
        let host = rest.split('/').next().unwrap_or(rest);
        return format!("{scheme}://{host}");
    }
    trimmed.to_string()
}

fn validate_context_name(name: &str) -> Result<(), ApiError> {
    if name.trim().is_empty() {
        return Err(ApiError::from(AegisError::Bridge(
            "context id must not be empty".into(),
        )));
    }
    if !name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return Err(ApiError::from(AegisError::Bridge(format!(
            "context id {name:?} must use only letters, numbers, '.', '-', or '_'"
        ))));
    }
    Ok(())
}

impl ServeRootState {
    fn default_context_state(&self) -> Option<ApiState> {
        self.context_state(&self.default_context_id)
    }

    fn context_state(&self, context_id: &str) -> Option<ApiState> {
        self.contexts
            .lock()
            .ok()
            .and_then(|contexts| contexts.get(context_id).map(|context| context.api.clone()))
    }

    fn insert_context(&self, context_id: String, state: ManagedContext) -> Result<(), ApiError> {
        let mut contexts = self.contexts.lock().map_err(|_| {
            ApiError::from(AegisError::Bridge("context registry lock poisoned".into()))
        })?;
        contexts.insert(context_id, state);
        Ok(())
    }

    fn remove_context(&self, context_id: &str) -> Result<Option<ManagedContext>, ApiError> {
        let mut contexts = self.contexts.lock().map_err(|_| {
            ApiError::from(AegisError::Bridge("context registry lock poisoned".into()))
        })?;
        Ok(contexts.remove(context_id))
    }

    fn list_contexts(&self) -> Result<Vec<ContextSummary>, ApiError> {
        let contexts = self.contexts.lock().map_err(|_| {
            ApiError::from(AegisError::Bridge("context registry lock poisoned".into()))
        })?;
        let mut items = contexts
            .iter()
            .map(|(id, state)| {
                let diagnostics = read_diagnostics(&state.api.diagnostics);
                ContextSummary {
                    id: id.clone(),
                    default: id == &self.default_context_id,
                    host_library: state.api.host_library.clone(),
                    browser: state.api.browser.clone(),
                    profile: state.api.profile.clone(),
                    runtime_state: serde_json::to_value(&diagnostics.state)
                        .ok()
                        .and_then(|value| value.as_str().map(ToOwned::to_owned))
                        .unwrap_or_else(|| "unknown".into()),
                    command_ready: diagnostics.command_ready,
                }
            })
            .collect::<Vec<_>>();
        items.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(items)
    }

    fn drain_contexts(&self) -> Result<Vec<(String, ManagedContext)>, ApiError> {
        let mut contexts = self.contexts.lock().map_err(|_| {
            ApiError::from(AegisError::Bridge("context registry lock poisoned".into()))
        })?;
        let drained = contexts.drain().collect::<Vec<_>>();
        Ok(drained)
    }
}

pub fn router(state: ServeRootState) -> Router {
    Router::new()
        .route("/", get(api_manifest))
        .route("/manifest", get(api_manifest))
        .route("/version", get(api_version))
        .route("/contexts", get(list_contexts).post(create_context))
        .route(
            "/contexts/:context_id",
            get(get_context).delete(delete_context),
        )
        .route("/contexts/:context_id/healthz", get(context_health))
        .route("/contexts/:context_id/readyz", get(context_readiness))
        .route("/contexts/:context_id/doctor", get(context_doctor))
        .route("/contexts/:context_id/runtime", get(context_runtime_info))
        .route(
            "/contexts/:context_id/runtime/cancel",
            post(context_cancel_runtime_operation),
        )
        .route(
            "/contexts/:context_id/session",
            post(context_inject_session).get(context_snapshot_session),
        )
        .route(
            "/contexts/:context_id/session/save",
            post(context_save_session_profile),
        )
        .route(
            "/contexts/:context_id/session/load",
            post(context_load_session_profile),
        )
        .route("/contexts/:context_id/navigate", post(context_navigate))
        .route("/contexts/:context_id/execute", post(context_execute))
        .route("/contexts/:context_id/dom", get(context_snapshot_dom))
        .route("/contexts/:context_id/events", get(context_events))
        .route(
            "/contexts/:context_id/events/live",
            get(context_events_live),
        )
        .route(
            "/contexts/:context_id/trace/enable",
            post(context_enable_trace),
        )
        .route("/healthz", get(health))
        .route("/readyz", get(readiness))
        .route("/doctor", get(doctor))
        .route("/runtime", get(runtime_info))
        .route("/runtime/cancel", post(cancel_runtime_operation))
        .route("/session", post(inject_session).get(snapshot_session))
        .route("/session/save", post(save_session_profile))
        .route("/session/load", post(load_session_profile))
        .route("/navigate", post(navigate))
        .route("/execute", post(execute))
        .route("/dom", get(snapshot_dom))
        .route("/events", get(events))
        .route("/events/live", get(events_live))
        .route("/trace/enable", post(enable_trace))
        .with_state(state)
}

fn require_default_context_state(root: &ServeRootState) -> Result<ApiState, ApiError> {
    root.default_context_state()
        .ok_or_else(|| ApiError::from(AegisError::Bridge("default context unavailable".into())))
}

fn require_context_state(root: &ServeRootState, context_id: &str) -> Result<ApiState, ApiError> {
    root.context_state(context_id)
        .ok_or_else(|| ApiError::not_found(format!("context `{context_id}` was not found")))
}

async fn shutdown_context(context_id: &str, state: &ApiState) -> Result<(), ApiError> {
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::Shutdown(reply_tx))
        .map_err(channel_error)?;
    match timeout(COMMAND_TIMEOUT, reply_rx).await {
        Ok(Ok(Ok(()))) => Ok(()),
        Ok(Ok(Err(error))) => Err(ApiError::from(error)),
        Ok(Err(error)) => Err(reply_error(error)),
        Err(_) => Err(ApiError::timeout(&format!("shutdown_context:{context_id}"))),
    }
}

fn join_context_thread(context_id: &str, mut managed: ManagedContext) -> Result<(), ApiError> {
    let Some(owner_thread) = managed.owner_thread.take() else {
        return Ok(());
    };
    owner_thread.join().map_err(|_| {
        ApiError::from(AegisError::Bridge(format!(
            "context `{context_id}` owner thread panicked during shutdown"
        )))
    })
}

async fn snapshot_context_session_state(state: &ApiState) -> Result<SessionState, ApiError> {
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::SnapshotSession(reply_tx))
        .map_err(channel_error)?;
    match timeout(COMMAND_TIMEOUT, reply_rx).await {
        Ok(Ok(Ok(session))) => Ok(session),
        Ok(Ok(Err(error))) => Err(ApiError::from(error)),
        Ok(Err(error)) => Err(reply_error(error)),
        Err(_) => Err(ApiError::timeout("snapshot_context_session_state")),
    }
}

async fn inject_context_session_state(
    state: &ApiState,
    session: SessionState,
) -> Result<(), ApiError> {
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::InjectSession(session, reply_tx))
        .map_err(channel_error)?;
    match timeout(COMMAND_TIMEOUT, reply_rx).await {
        Ok(Ok(Ok(()))) => Ok(()),
        Ok(Ok(Err(error))) => Err(ApiError::from(error)),
        Ok(Err(error)) => Err(reply_error(error)),
        Err(_) => Err(ApiError::timeout("inject_context_session_state")),
    }
}

async fn shutdown_all_contexts(root: &ServeRootState) -> Result<(), ApiError> {
    let contexts = root.drain_contexts()?;
    let mut first_error = None;
    for (context_id, managed) in contexts {
        if let Err(error) = shutdown_context(&context_id, &managed.api).await
            && first_error.is_none()
        {
            first_error = Some(error);
        }
        if let Err(error) = join_context_thread(&context_id, managed)
            && first_error.is_none()
        {
            first_error = Some(error);
        }
    }
    if let Some(error) = first_error {
        return Err(error);
    }
    Ok(())
}

async fn health(State(root): State<ServeRootState>) -> Result<Json<HealthResponse>, ApiError> {
    let state = require_default_context_state(&root)?;
    let diagnostics = read_diagnostics(&state.diagnostics);
    Ok(Json(HealthResponse {
        version: env!("CARGO_PKG_VERSION"),
        protocol_version: PROTOCOL_VERSION,
        control_plane_up: true,
        runtime_state: diagnostics.state.clone(),
        command_ready: diagnostics.command_ready,
        bridge_healthy: diagnostics.bridge_healthy,
        browser_backend_healthy: diagnostics.browser_backend_healthy,
        browser_process_up: diagnostics.browser_process_up,
        page_attached: diagnostics.page_attached,
        renderer_attached: diagnostics.renderer_attached,
        dom_snapshot_available: diagnostics.dom_snapshot_available,
        event_decoder_ok: diagnostics.event_decoder_ok,
        active_operation: diagnostics.active_operation,
        last_failure: diagnostics.last_failure,
    }))
}

#[derive(Debug, Serialize)]
struct RuntimeInfo {
    host_library: PathBuf,
    browser: BrowserConfig,
    diagnostics: RuntimeDiagnosticsResponse,
    startup: ServeStartupMetrics,
    profile: SessionProfileInfo,
}

#[derive(Debug, Serialize)]
struct ApiRouteDoc {
    method: &'static str,
    path: &'static str,
    summary: &'static str,
}

#[derive(Debug, Serialize)]
struct ApiSchemaFieldDoc {
    name: &'static str,
    kind: &'static str,
    required: bool,
    description: &'static str,
}

#[derive(Debug, Serialize)]
struct ApiCommandDoc {
    name: &'static str,
    summary: &'static str,
    targeting: &'static str,
    fields: Vec<ApiSchemaFieldDoc>,
}

#[derive(Debug, Serialize)]
struct ApiManifest {
    service: &'static str,
    version: &'static str,
    protocol_version: u16,
    discovery: &'static str,
    routes: Vec<ApiRouteDoc>,
    capabilities: Vec<&'static str>,
    commands: Vec<ApiCommandDoc>,
}

async fn api_manifest() -> Json<ApiManifest> {
    Json(ApiManifest {
        service: "aegis",
        version: env!("CARGO_PKG_VERSION"),
        protocol_version: PROTOCOL_VERSION,
        discovery: "/manifest",
        routes: vec![
            ApiRouteDoc {
                method: "GET",
                path: "/",
                summary: "JSON API discovery document",
            },
            ApiRouteDoc {
                method: "GET",
                path: "/manifest",
                summary: "JSON API discovery document",
            },
            ApiRouteDoc {
                method: "GET",
                path: "/version",
                summary: "Version and protocol metadata",
            },
            ApiRouteDoc {
                method: "GET",
                path: "/contexts",
                summary: "List all named browser contexts",
            },
            ApiRouteDoc {
                method: "POST",
                path: "/contexts",
                summary: "Create a new isolated browser context",
            },
            ApiRouteDoc {
                method: "GET",
                path: "/contexts/:context_id",
                summary: "Read one browser context summary",
            },
            ApiRouteDoc {
                method: "DELETE",
                path: "/contexts/:context_id",
                summary: "Delete one non-default browser context",
            },
            ApiRouteDoc {
                method: "GET",
                path: "/contexts/:context_id/healthz",
                summary: "Context-scoped health",
            },
            ApiRouteDoc {
                method: "GET",
                path: "/contexts/:context_id/readyz",
                summary: "Context-scoped readiness",
            },
            ApiRouteDoc {
                method: "GET",
                path: "/contexts/:context_id/doctor",
                summary: "Context-scoped diagnostics",
            },
            ApiRouteDoc {
                method: "GET",
                path: "/contexts/:context_id/runtime",
                summary: "Context runtime config and live state",
            },
            ApiRouteDoc {
                method: "POST",
                path: "/contexts/:context_id/runtime/cancel",
                summary: "Cancel the active operation in one context",
            },
            ApiRouteDoc {
                method: "GET",
                path: "/contexts/:context_id/session",
                summary: "Snapshot one context session",
            },
            ApiRouteDoc {
                method: "POST",
                path: "/contexts/:context_id/session",
                summary: "Inject session state into one context",
            },
            ApiRouteDoc {
                method: "POST",
                path: "/contexts/:context_id/session/save",
                summary: "Persist one context profile session",
            },
            ApiRouteDoc {
                method: "POST",
                path: "/contexts/:context_id/session/load",
                summary: "Load one context profile session",
            },
            ApiRouteDoc {
                method: "POST",
                path: "/contexts/:context_id/navigate",
                summary: "Navigate one context",
            },
            ApiRouteDoc {
                method: "POST",
                path: "/contexts/:context_id/execute",
                summary: "Execute commands in one context",
            },
            ApiRouteDoc {
                method: "GET",
                path: "/contexts/:context_id/dom",
                summary: "Fetch one context DOM snapshot",
            },
            ApiRouteDoc {
                method: "GET",
                path: "/contexts/:context_id/events",
                summary: "Read buffered events for one context",
            },
            ApiRouteDoc {
                method: "GET",
                path: "/contexts/:context_id/events/live",
                summary: "Stream events for one context over SSE",
            },
            ApiRouteDoc {
                method: "POST",
                path: "/contexts/:context_id/trace/enable",
                summary: "Enable trace recording for one context",
            },
            ApiRouteDoc {
                method: "GET",
                path: "/healthz",
                summary: "Control-plane and runtime health",
            },
            ApiRouteDoc {
                method: "GET",
                path: "/readyz",
                summary: "Command readiness gate",
            },
            ApiRouteDoc {
                method: "GET",
                path: "/doctor",
                summary: "Detailed runtime diagnostics",
            },
            ApiRouteDoc {
                method: "GET",
                path: "/runtime",
                summary: "Runtime config and live state",
            },
            ApiRouteDoc {
                method: "POST",
                path: "/runtime/cancel",
                summary: "Cancel the active runtime operation",
            },
            ApiRouteDoc {
                method: "POST",
                path: "/navigate",
                summary: "Navigate the active browser page",
            },
            ApiRouteDoc {
                method: "POST",
                path: "/execute",
                summary: "Execute a command batch including file upload and media diagnostics",
            },
            ApiRouteDoc {
                method: "GET",
                path: "/dom",
                summary: "Fetch a fresh DOM snapshot",
            },
            ApiRouteDoc {
                method: "GET",
                path: "/events",
                summary: "Read buffered runtime events",
            },
            ApiRouteDoc {
                method: "GET",
                path: "/events/live",
                summary: "Stream runtime events over SSE",
            },
            ApiRouteDoc {
                method: "GET",
                path: "/session",
                summary: "Snapshot the active session",
            },
            ApiRouteDoc {
                method: "POST",
                path: "/session",
                summary: "Inject session state",
            },
            ApiRouteDoc {
                method: "POST",
                path: "/session/save",
                summary: "Persist the active profile session",
            },
            ApiRouteDoc {
                method: "POST",
                path: "/session/load",
                summary: "Load the active profile session",
            },
            ApiRouteDoc {
                method: "POST",
                path: "/trace/enable",
                summary: "Enable trace recording",
            },
        ],
        capabilities: vec![
            "semantic_dom_snapshot",
            "event_stream",
            "network_event_capture",
            "first_class_file_upload",
            "media_diagnostics",
            "named_multi_context_control_plane",
            "session_snapshotting",
            "trace_recording",
        ],
        commands: vec![
            ApiCommandDoc {
                name: "click",
                summary: "Click one actionable node",
                targeting: "id or match",
                fields: vec![],
            },
            ApiCommandDoc {
                name: "hover",
                summary: "Hover one node",
                targeting: "id or match",
                fields: vec![],
            },
            ApiCommandDoc {
                name: "set_value",
                summary: "Set the value of one form control",
                targeting: "id or match",
                fields: vec![ApiSchemaFieldDoc {
                    name: "value",
                    kind: "string",
                    required: true,
                    description: "Text value to apply",
                }],
            },
            ApiCommandDoc {
                name: "set_files",
                summary: "Attach one or more local files to a file input",
                targeting: "id or match",
                fields: vec![ApiSchemaFieldDoc {
                    name: "paths",
                    kind: "string[]",
                    required: true,
                    description: "Absolute local file paths to attach",
                }],
            },
            ApiCommandDoc {
                name: "press_key",
                summary: "Dispatch keyboard interaction",
                targeting: "optional id or match",
                fields: vec![ApiSchemaFieldDoc {
                    name: "key",
                    kind: "string",
                    required: true,
                    description: "Key value such as Enter or Space",
                }],
            },
            ApiCommandDoc {
                name: "wait_for",
                summary: "Wait for URL, DOM, text, scroll, or media conditions",
                targeting: "optional id or match",
                fields: vec![
                    ApiSchemaFieldDoc {
                        name: "timeout_ms",
                        kind: "u64",
                        required: false,
                        description: "Overall wait deadline in milliseconds",
                    },
                    ApiSchemaFieldDoc {
                        name: "selector",
                        kind: "string",
                        required: false,
                        description: "CSS selector that must exist",
                    },
                ],
            },
            ApiCommandDoc {
                name: "scroll",
                summary: "Scroll the page viewport",
                targeting: "none",
                fields: vec![],
            },
            ApiCommandDoc {
                name: "drag",
                summary: "Perform pointer drag gestures, including range controls",
                targeting: "id or match",
                fields: vec![],
            },
            ApiCommandDoc {
                name: "geometry",
                summary: "Read geometry for a matched node",
                targeting: "id or match",
                fields: vec![],
            },
            ApiCommandDoc {
                name: "media_state",
                summary: "Read diagnostics for all media nodes or one matched media node",
                targeting: "optional id or match",
                fields: vec![],
            },
            ApiCommandDoc {
                name: "eval",
                summary: "Execute JavaScript and return the result",
                targeting: "none",
                fields: vec![],
            },
        ],
    })
}

#[derive(Debug, Serialize)]
struct ApiVersion {
    version: &'static str,
    protocol_version: u16,
}

async fn api_version() -> Json<ApiVersion> {
    Json(ApiVersion {
        version: env!("CARGO_PKG_VERSION"),
        protocol_version: PROTOCOL_VERSION,
    })
}

async fn list_contexts(
    State(root): State<ServeRootState>,
) -> Result<Json<Vec<ContextSummary>>, ApiError> {
    Ok(Json(root.list_contexts()?))
}

async fn get_context(
    State(root): State<ServeRootState>,
    Path(context_id): Path<String>,
) -> Result<Json<ContextSummary>, ApiError> {
    let contexts = root.list_contexts()?;
    contexts
        .into_iter()
        .find(|context| context.id == context_id)
        .map(Json)
        .ok_or_else(|| ApiError::not_found(format!("context `{context_id}` was not found")))
}

async fn create_context(
    State(root): State<ServeRootState>,
    Json(body): Json<CreateContextBody>,
) -> Result<(StatusCode, Json<ContextSummary>), ApiError> {
    let CreateContextBody {
        id,
        profile,
        seed_from_context,
        mode,
        start_url,
    } = body;
    let existing = root.list_contexts()?;
    let next_index = existing.len() + 1;
    let context_id = id
        .clone()
        .or_else(|| profile.clone())
        .unwrap_or_else(|| format!("context-{next_index}"));
    validate_context_name(&context_id)?;
    if root.context_state(&context_id).is_some() {
        return Err(ApiError::from(AegisError::Bridge(format!(
            "context `{context_id}` already exists"
        ))));
    }
    let profile = profile.unwrap_or_else(|| context_id.clone());
    let mut browser = root.browser.clone();
    if let Some(mode) = mode {
        browser.mode = mode;
    }
    if start_url.is_some() {
        browser.start_url = start_url;
    }
    let state = spawn_context_state(root.host_library.clone(), browser, profile)?;
    if let Some(source_context_id) = seed_from_context.as_deref() {
        let source_state = require_context_state(&root, source_context_id)?;
        let session = snapshot_context_session_state(&source_state).await?;
        if let Err(error) = inject_context_session_state(&state.api, session).await {
            let _ = shutdown_context(&context_id, &state.api).await;
            let _ = join_context_thread(&context_id, state);
            return Err(error);
        }
    }
    let diagnostics = read_diagnostics(&state.api.diagnostics);
    let summary = ContextSummary {
        id: context_id.clone(),
        default: false,
        host_library: state.api.host_library.clone(),
        browser: state.api.browser.clone(),
        profile: state.api.profile.clone(),
        runtime_state: serde_json::to_value(&diagnostics.state)
            .ok()
            .and_then(|value| value.as_str().map(ToOwned::to_owned))
            .unwrap_or_else(|| "unknown".into()),
        command_ready: diagnostics.command_ready,
    };
    root.insert_context(context_id, state)?;
    Ok((StatusCode::CREATED, Json(summary)))
}

async fn delete_context(
    State(root): State<ServeRootState>,
    Path(context_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    if context_id == root.default_context_id {
        return Err(ApiError::from(AegisError::Bridge(
            "default context cannot be deleted".into(),
        )));
    }
    match root.remove_context(&context_id)? {
        Some(state) => {
            shutdown_context(&context_id, &state.api).await?;
            join_context_thread(&context_id, state)?;
            Ok(StatusCode::NO_CONTENT)
        }
        None => Err(ApiError::not_found(format!(
            "context `{context_id}` was not found"
        ))),
    }
}

async fn context_health(
    State(root): State<ServeRootState>,
    Path(context_id): Path<String>,
) -> Result<Json<HealthResponse>, ApiError> {
    let state = require_context_state(&root, &context_id)?;
    let diagnostics = read_diagnostics(&state.diagnostics);
    Ok(Json(HealthResponse {
        version: env!("CARGO_PKG_VERSION"),
        protocol_version: PROTOCOL_VERSION,
        control_plane_up: true,
        runtime_state: diagnostics.state.clone(),
        command_ready: diagnostics.command_ready,
        bridge_healthy: diagnostics.bridge_healthy,
        browser_backend_healthy: diagnostics.browser_backend_healthy,
        browser_process_up: diagnostics.browser_process_up,
        page_attached: diagnostics.page_attached,
        renderer_attached: diagnostics.renderer_attached,
        dom_snapshot_available: diagnostics.dom_snapshot_available,
        event_decoder_ok: diagnostics.event_decoder_ok,
        active_operation: diagnostics.active_operation,
        last_failure: diagnostics.last_failure,
    }))
}

async fn runtime_info(State(root): State<ServeRootState>) -> Result<Json<RuntimeInfo>, ApiError> {
    let state = require_default_context_state(&root)?;
    Ok(Json(RuntimeInfo {
        host_library: state.host_library.clone(),
        browser: state.browser.clone(),
        diagnostics: read_diagnostics(&state.diagnostics),
        profile: state.profile.clone(),
        startup: state
            .startup
            .lock()
            .map(|metrics| metrics.clone())
            .unwrap_or(ServeStartupMetrics {
                client_connect_ms: 0,
                api_bind_ms: 0,
                total_ready_ms: 0,
            }),
    }))
}

async fn readiness(
    State(root): State<ServeRootState>,
) -> Result<Json<RuntimeDiagnosticsResponse>, ApiError> {
    let state = require_default_context_state(&root)?;
    let diagnostics = read_diagnostics(&state.diagnostics);
    if diagnostics.command_ready {
        Ok(Json(diagnostics))
    } else {
        Err(ApiError::readiness(diagnostics))
    }
}

async fn doctor(
    State(root): State<ServeRootState>,
) -> Result<Json<RuntimeDiagnosticsResponse>, ApiError> {
    let state = require_default_context_state(&root)?;
    Ok(Json(read_diagnostics(&state.diagnostics)))
}

async fn cancel_runtime_operation(
    State(root): State<ServeRootState>,
) -> Result<Json<RuntimeDiagnosticsResponse>, ApiError> {
    let state = require_default_context_state(&root)?;
    request_runtime_cancel(&state);
    mark_operation_cancel_requested(&state.diagnostics);
    Ok(Json(read_diagnostics(&state.diagnostics)))
}

async fn save_session_profile(
    State(root): State<ServeRootState>,
) -> Result<Json<SessionProfileInfo>, ApiError> {
    let state = require_default_context_state(&root)?;
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::SaveSessionProfile(reply_tx))
        .map_err(channel_error)?;
    let profile = await_command("save_session_profile", &state, reply_rx).await??;
    Ok(Json(profile))
}

async fn load_session_profile(
    State(root): State<ServeRootState>,
) -> Result<Json<SessionProfileInfo>, ApiError> {
    let state = require_default_context_state(&root)?;
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::LoadSessionProfile(reply_tx))
        .map_err(channel_error)?;
    let profile = await_command("load_session_profile", &state, reply_rx).await??;
    Ok(Json(profile))
}

async fn inject_session(
    State(root): State<ServeRootState>,
    Json(body): Json<SessionState>,
) -> Result<StatusCode, ApiError> {
    let state = require_default_context_state(&root)?;
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::InjectSession(body, reply_tx))
        .map_err(channel_error)?;
    await_command("inject_session", &state, reply_rx).await??;
    Ok(StatusCode::NO_CONTENT)
}

async fn snapshot_session(
    State(root): State<ServeRootState>,
) -> Result<Json<SessionState>, ApiError> {
    let state = require_default_context_state(&root)?;
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::SnapshotSession(reply_tx))
        .map_err(channel_error)?;
    Ok(Json(
        await_command("snapshot_session", &state, reply_rx).await??,
    ))
}

async fn navigate(
    State(root): State<ServeRootState>,
    Json(body): Json<NavigateBody>,
) -> Result<Json<Vec<SequencedEvent>>, ApiError> {
    let state = require_default_context_state(&root)?;
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::Navigate(body.url, reply_tx))
        .map_err(channel_error)?;
    Ok(Json(await_command("navigate", &state, reply_rx).await??))
}

async fn execute(
    State(root): State<ServeRootState>,
    Json(body): Json<ExecuteBody>,
) -> Result<Json<ExecutionReport>, ApiError> {
    let state = require_default_context_state(&root)?;
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::Execute(body.commands, reply_tx))
        .map_err(channel_error)?;
    Ok(Json(await_command("execute", &state, reply_rx).await??))
}

async fn snapshot_dom(State(root): State<ServeRootState>) -> Result<Json<DomSnapshot>, ApiError> {
    let state = require_default_context_state(&root)?;
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::SnapshotDom(reply_tx))
        .map_err(channel_error)?;
    Ok(Json(
        await_command("snapshot_dom", &state, reply_rx).await??,
    ))
}

async fn events(
    State(root): State<ServeRootState>,
    Query(query): Query<EventQuery>,
) -> Result<Json<EventReadWindow>, ApiError> {
    let state = require_default_context_state(&root)?;
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::Events(query.since, reply_tx))
        .map_err(channel_error)?;
    Ok(Json(await_command("events", &state, reply_rx).await??))
}

async fn events_live(
    State(root): State<ServeRootState>,
    Query(query): Query<EventStreamQuery>,
) -> Result<Sse<impl futures_core::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let state = require_default_context_state(&root)?;
    let poll_ms = query
        .poll_ms
        .unwrap_or(DEFAULT_EVENT_STREAM_POLL_INTERVAL.as_millis() as u64)
        .clamp(
            MIN_EVENT_STREAM_POLL_INTERVAL_MS,
            MAX_EVENT_STREAM_POLL_INTERVAL_MS,
        );
    let poll_interval = Duration::from_millis(poll_ms);
    let mut since = query.since;
    let state = state.clone();

    Ok(Sse::new(stream! {
        yield Ok(Event::default().event("ready").json_data(json!({
            "since": since,
            "poll_ms": poll_ms,
        })).unwrap_or_else(|_| Event::default().event("ready").data("ready")));

        loop {
            let (reply_tx, reply_rx) = oneshot::channel();
            if state.tx.send(ApiCommand::Events(since, reply_tx)).is_err() {
                yield Ok(Event::default().event("error").data("runtime event channel closed"));
                break;
            }

            let window = match timeout(COMMAND_TIMEOUT, reply_rx).await {
                Ok(Ok(Ok(window))) => window,
                Ok(Ok(Err(error))) => {
                    yield Ok(Event::default().event("error").data(error.to_string()));
                    break;
                }
                Ok(Err(_)) => {
                    yield Ok(Event::default().event("error").data("runtime event stream cancelled"));
                    break;
                }
                Err(_) => {
                    yield Ok(Event::default().event("error").data("runtime event stream timed out"));
                    break;
                }
            };

            since = window.latest_sequence;
            if !window.events.is_empty() || window.gap_detected {
                yield Ok(Event::default().event("runtime_events").json_data(&window).unwrap_or_else(|_| {
                    Event::default().event("runtime_events").data("serialization_error")
                }));
            }

            sleep(poll_interval).await;
        }
    })
    .keep_alive(KeepAlive::default().interval(Duration::from_secs(15)).text("keep-alive")))
}
async fn enable_trace(
    State(root): State<ServeRootState>,
    Json(body): Json<TraceBody>,
) -> Result<StatusCode, ApiError> {
    let state = require_default_context_state(&root)?;
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::EnableTrace(body.path, reply_tx))
        .map_err(channel_error)?;
    await_command("enable_trace", &state, reply_rx).await??;
    Ok(StatusCode::NO_CONTENT)
}

async fn context_runtime_info(
    State(root): State<ServeRootState>,
    Path(context_id): Path<String>,
) -> Result<Json<RuntimeInfo>, ApiError> {
    let state = require_context_state(&root, &context_id)?;
    Ok(Json(RuntimeInfo {
        host_library: state.host_library.clone(),
        browser: state.browser.clone(),
        diagnostics: read_diagnostics(&state.diagnostics),
        profile: state.profile.clone(),
        startup: state
            .startup
            .lock()
            .map(|metrics| metrics.clone())
            .unwrap_or(ServeStartupMetrics {
                client_connect_ms: 0,
                api_bind_ms: 0,
                total_ready_ms: 0,
            }),
    }))
}

async fn context_readiness(
    State(root): State<ServeRootState>,
    Path(context_id): Path<String>,
) -> Result<Json<RuntimeDiagnosticsResponse>, ApiError> {
    let state = require_context_state(&root, &context_id)?;
    let diagnostics = read_diagnostics(&state.diagnostics);
    if diagnostics.command_ready {
        Ok(Json(diagnostics))
    } else {
        Err(ApiError::readiness(diagnostics))
    }
}

async fn context_doctor(
    State(root): State<ServeRootState>,
    Path(context_id): Path<String>,
) -> Result<Json<RuntimeDiagnosticsResponse>, ApiError> {
    let state = require_context_state(&root, &context_id)?;
    Ok(Json(read_diagnostics(&state.diagnostics)))
}

async fn context_cancel_runtime_operation(
    State(root): State<ServeRootState>,
    Path(context_id): Path<String>,
) -> Result<Json<RuntimeDiagnosticsResponse>, ApiError> {
    let state = require_context_state(&root, &context_id)?;
    request_runtime_cancel(&state);
    mark_operation_cancel_requested(&state.diagnostics);
    Ok(Json(read_diagnostics(&state.diagnostics)))
}

async fn context_save_session_profile(
    State(root): State<ServeRootState>,
    Path(context_id): Path<String>,
) -> Result<Json<SessionProfileInfo>, ApiError> {
    let state = require_context_state(&root, &context_id)?;
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::SaveSessionProfile(reply_tx))
        .map_err(channel_error)?;
    let profile = await_command("save_session_profile", &state, reply_rx).await??;
    Ok(Json(profile))
}

async fn context_load_session_profile(
    State(root): State<ServeRootState>,
    Path(context_id): Path<String>,
) -> Result<Json<SessionProfileInfo>, ApiError> {
    let state = require_context_state(&root, &context_id)?;
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::LoadSessionProfile(reply_tx))
        .map_err(channel_error)?;
    let profile = await_command("load_session_profile", &state, reply_rx).await??;
    Ok(Json(profile))
}

async fn context_inject_session(
    State(root): State<ServeRootState>,
    Path(context_id): Path<String>,
    Json(body): Json<SessionState>,
) -> Result<StatusCode, ApiError> {
    let state = require_context_state(&root, &context_id)?;
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::InjectSession(body, reply_tx))
        .map_err(channel_error)?;
    await_command("inject_session", &state, reply_rx).await??;
    Ok(StatusCode::NO_CONTENT)
}

async fn context_snapshot_session(
    State(root): State<ServeRootState>,
    Path(context_id): Path<String>,
) -> Result<Json<SessionState>, ApiError> {
    let state = require_context_state(&root, &context_id)?;
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::SnapshotSession(reply_tx))
        .map_err(channel_error)?;
    Ok(Json(
        await_command("snapshot_session", &state, reply_rx).await??,
    ))
}

async fn context_navigate(
    State(root): State<ServeRootState>,
    Path(context_id): Path<String>,
    Json(body): Json<NavigateBody>,
) -> Result<Json<Vec<SequencedEvent>>, ApiError> {
    let state = require_context_state(&root, &context_id)?;
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::Navigate(body.url, reply_tx))
        .map_err(channel_error)?;
    Ok(Json(await_command("navigate", &state, reply_rx).await??))
}

async fn context_execute(
    State(root): State<ServeRootState>,
    Path(context_id): Path<String>,
    Json(body): Json<ExecuteBody>,
) -> Result<Json<ExecutionReport>, ApiError> {
    let state = require_context_state(&root, &context_id)?;
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::Execute(body.commands, reply_tx))
        .map_err(channel_error)?;
    Ok(Json(await_command("execute", &state, reply_rx).await??))
}

async fn context_snapshot_dom(
    State(root): State<ServeRootState>,
    Path(context_id): Path<String>,
) -> Result<Json<DomSnapshot>, ApiError> {
    let state = require_context_state(&root, &context_id)?;
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::SnapshotDom(reply_tx))
        .map_err(channel_error)?;
    Ok(Json(
        await_command("snapshot_dom", &state, reply_rx).await??,
    ))
}

async fn context_events(
    State(root): State<ServeRootState>,
    Path(context_id): Path<String>,
    Query(query): Query<EventQuery>,
) -> Result<Json<EventReadWindow>, ApiError> {
    let state = require_context_state(&root, &context_id)?;
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::Events(query.since, reply_tx))
        .map_err(channel_error)?;
    Ok(Json(await_command("events", &state, reply_rx).await??))
}

async fn context_events_live(
    State(root): State<ServeRootState>,
    Path(context_id): Path<String>,
    Query(query): Query<EventStreamQuery>,
) -> Result<Sse<impl futures_core::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let state = require_context_state(&root, &context_id)?;
    let poll_ms = query
        .poll_ms
        .unwrap_or(DEFAULT_EVENT_STREAM_POLL_INTERVAL.as_millis() as u64)
        .clamp(
            MIN_EVENT_STREAM_POLL_INTERVAL_MS,
            MAX_EVENT_STREAM_POLL_INTERVAL_MS,
        );
    let poll_interval = Duration::from_millis(poll_ms);
    let mut since = query.since;
    let state = state.clone();

    Ok(Sse::new(stream! {
        yield Ok(Event::default().event("ready").json_data(json!({
            "since": since,
            "poll_ms": poll_ms,
        })).unwrap_or_else(|_| Event::default().event("ready").data("ready")));

        loop {
            let (reply_tx, reply_rx) = oneshot::channel();
            if state.tx.send(ApiCommand::Events(since, reply_tx)).is_err() {
                yield Ok(Event::default().event("error").data("runtime event channel closed"));
                break;
            }

            let window = match timeout(COMMAND_TIMEOUT, reply_rx).await {
                Ok(Ok(Ok(window))) => window,
                Ok(Ok(Err(error))) => {
                    yield Ok(Event::default().event("error").data(error.to_string()));
                    break;
                }
                Ok(Err(_)) => {
                    yield Ok(Event::default().event("error").data("runtime event stream cancelled"));
                    break;
                }
                Err(_) => {
                    yield Ok(Event::default().event("error").data("runtime event stream timed out"));
                    break;
                }
            };

            since = window.latest_sequence;
            if !window.events.is_empty() || window.gap_detected {
                yield Ok(Event::default().event("runtime_events").json_data(&window).unwrap_or_else(|_| {
                    Event::default().event("runtime_events").data("serialization_error")
                }));
            }

            sleep(poll_interval).await;
        }
    })
    .keep_alive(KeepAlive::default().interval(Duration::from_secs(15)).text("keep-alive")))
}

async fn context_enable_trace(
    State(root): State<ServeRootState>,
    Path(context_id): Path<String>,
    Json(body): Json<TraceBody>,
) -> Result<StatusCode, ApiError> {
    let state = require_context_state(&root, &context_id)?;
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::EnableTrace(body.path, reply_tx))
        .map_err(channel_error)?;
    await_command("enable_trace", &state, reply_rx).await??;
    Ok(StatusCode::NO_CONTENT)
}

fn channel_error(error: mpsc::SendError<ApiCommand>) -> ApiError {
    ApiError::from(AegisError::Bridge(error.to_string()))
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    body: ApiErrorBody,
}

impl ApiError {
    fn not_found(message: String) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            body: ApiErrorBody {
                error: message,
                code: "not_found".into(),
                operation: None,
                stage: None,
                elapsed_ms: None,
                timed_out: false,
                restart_recommended: false,
            },
        }
    }

    fn timeout(operation: &str) -> Self {
        Self {
            status: StatusCode::GATEWAY_TIMEOUT,
            body: ApiErrorBody {
                error: format!(
                    "operation `{operation}` exceeded the server timeout and a runtime cancellation request was sent"
                ),
                code: "operation_cancel_requested".into(),
                operation: Some(operation.to_string()),
                stage: Some("awaiting_control_plane_reply".into()),
                elapsed_ms: Some(COMMAND_TIMEOUT.as_millis() as u64),
                timed_out: true,
                restart_recommended: false,
            },
        }
    }

    fn readiness(diagnostics: RuntimeDiagnosticsResponse) -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            body: ApiErrorBody {
                error: "runtime is not command-ready".into(),
                code: "not_ready".into(),
                operation: diagnostics
                    .active_operation
                    .as_ref()
                    .map(|op| op.name.clone()),
                stage: diagnostics
                    .active_operation
                    .as_ref()
                    .map(|op| op.stage.clone()),
                elapsed_ms: diagnostics
                    .active_operation
                    .as_ref()
                    .map(|op| op.elapsed_ms),
                timed_out: diagnostics
                    .active_operation
                    .as_ref()
                    .is_some_and(|op| op.timed_out),
                restart_recommended: diagnostics
                    .last_failure
                    .as_ref()
                    .is_some_and(|failure| failure.restart_recommended),
            },
        }
    }
}

impl From<AegisError> for ApiError {
    fn from(value: AegisError) -> Self {
        let message = value.to_string();
        if let Some(native) = parse_native_operation_error(&message) {
            return Self {
                status: if native.timed_out {
                    StatusCode::GATEWAY_TIMEOUT
                } else {
                    StatusCode::BAD_GATEWAY
                },
                body: ApiErrorBody {
                    error: native.message,
                    code: "native_operation_error".into(),
                    operation: Some(native.operation),
                    stage: Some(native.stage),
                    elapsed_ms: Some(native.elapsed_ms),
                    timed_out: native.timed_out,
                    restart_recommended: native.restart_recommended,
                },
            };
        }

        let status = match value {
            AegisError::InvalidSession(_) => StatusCode::BAD_REQUEST,
            AegisError::Serialize(_)
            | AegisError::Deserialize(_)
            | AegisError::Io(_)
            | AegisError::Utf8(_)
            | AegisError::Protocol(_)
            | AegisError::Bridge(_) => StatusCode::BAD_GATEWAY,
        };
        Self {
            status,
            body: ApiErrorBody {
                error: message,
                code: "aegis_error".into(),
                operation: None,
                stage: None,
                elapsed_ms: None,
                timed_out: false,
                restart_recommended: false,
            },
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (self.status, Json(self.body)).into_response()
    }
}

#[derive(Debug, Serialize)]
pub struct ApiErrorBody {
    pub error: String,
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
    pub timed_out: bool,
    pub restart_recommended: bool,
}

async fn await_command<T>(
    operation: &str,
    state: &ApiState,
    reply_rx: oneshot::Receiver<Result<T, AegisError>>,
) -> Result<Result<T, AegisError>, ApiError> {
    match timeout(COMMAND_TIMEOUT, reply_rx).await {
        Ok(result) => result.map_err(reply_error),
        Err(_) => {
            request_runtime_cancel(state);
            mark_operation_timeout(&state.diagnostics, operation);
            Err(ApiError::timeout(operation))
        }
    }
}

fn reply_error(error: oneshot::error::RecvError) -> ApiError {
    ApiError::from(AegisError::Bridge(error.to_string()))
}

fn emit_telemetry(event: &str, payload: serde_json::Value) {
    let Some(path) = std::env::var_os("AEGIS_DEBUG_LOG") else {
        return;
    };
    let start = TELEMETRY_START.get_or_init(Instant::now);
    let elapsed_ms = start.elapsed().as_millis() as u64;
    let line = json!({
        "source": "serve",
        "event": event,
        "elapsed_ms": elapsed_ms,
        "payload": payload,
    });
    if let Ok(mut output) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(output, "telemetry: {}", line);
    }
}

fn emit_operation_telemetry<T>(
    operation: &str,
    started_at: Instant,
    result: &Result<T, AegisError>,
    runtime: RuntimeStatus,
) {
    emit_telemetry(
        "operation_complete",
        json!({
            "operation": operation,
            "latency_ms": started_at.elapsed().as_millis() as u64,
            "ok": result.is_ok(),
            "error": result.as_ref().err().map(ToString::to_string),
            "runtime": runtime,
        }),
    );
}

fn record_operation_started(
    diagnostics: &Arc<Mutex<ServeDiagnostics>>,
    operation: &str,
    stage: &str,
) {
    if let Ok(mut diagnostics) = diagnostics.lock() {
        diagnostics.begin_operation(operation, stage);
    }
}

fn record_operation_finished<T>(
    diagnostics: &Arc<Mutex<ServeDiagnostics>>,
    operation: &str,
    client: &LoadedAegisClient,
    result: &Result<T, AegisError>,
) {
    let runtime = client.runtime_status();
    match result {
        Ok(_) => {
            if let Ok(mut diagnostics) = diagnostics.lock() {
                diagnostics.complete_success(operation, runtime);
            }
        }
        Err(error) => record_operation_failure(
            diagnostics,
            operation,
            failure_from_error(operation, "native operation failed", error),
            Some(runtime),
        ),
    }
}

fn record_operation_failure(
    diagnostics: &Arc<Mutex<ServeDiagnostics>>,
    operation: &str,
    failure: FailureSnapshot,
    runtime: Option<RuntimeStatus>,
) {
    if let Ok(mut diagnostics) = diagnostics.lock() {
        diagnostics.complete_failure(operation, failure, runtime);
    }
}

fn record_heartbeat(diagnostics: &Arc<Mutex<ServeDiagnostics>>, client: &LoadedAegisClient) {
    if let Ok(mut diagnostics) = diagnostics.lock() {
        diagnostics.record_runtime_snapshot(client.runtime_status());
    }
}

fn mark_operation_timeout(diagnostics: &Arc<Mutex<ServeDiagnostics>>, operation: &str) {
    if let Ok(mut diagnostics) = diagnostics.lock() {
        diagnostics.mark_timeout(operation, COMMAND_TIMEOUT.as_millis() as u64);
    }
}

fn mark_operation_cancel_requested(diagnostics: &Arc<Mutex<ServeDiagnostics>>) {
    if let Ok(mut diagnostics) = diagnostics.lock() {
        diagnostics.mark_cancel_requested();
    }
}

fn request_runtime_cancel(state: &ApiState) {
    if let Ok(cancel) = state.cancel.lock()
        && let Some(cancel) = cancel.as_ref()
    {
        cancel.request_cancel();
    }
}

fn read_diagnostics(diagnostics: &Arc<Mutex<ServeDiagnostics>>) -> RuntimeDiagnosticsResponse {
    diagnostics
        .lock()
        .map(|diagnostics| diagnostics.snapshot())
        .unwrap_or_else(|_| ServeDiagnostics::new(default_runtime_status()).snapshot())
}

fn default_runtime_status() -> RuntimeStatus {
    RuntimeStatus {
        bootstrapped: false,
        bootstrap_duration_ms: None,
        dom_nodes: 0,
        dom_snapshot_available: false,
        retained_event_count: 0,
        latest_event_sequence: 0,
        oldest_retained_event_sequence: None,
        current_url: None,
        current_title: None,
        document_ready_state: None,
        media: Vec::new(),
        last_dom_refresh_at_ms: None,
        last_live_state_refresh_at_ms: None,
        last_event_at_ms: None,
        last_successful_command_at_ms: None,
        last_successful_bridge_roundtrip_at_ms: None,
        host: Default::default(),
    }
}

fn failure_from_error(operation: &str, stage: &str, error: &AegisError) -> FailureSnapshot {
    if let Some(native) = parse_native_operation_error(&error.to_string()) {
        return FailureSnapshot {
            operation: native.operation,
            stage: native.stage,
            message: native.message,
            elapsed_ms: native.elapsed_ms,
            timed_out: native.timed_out,
            restart_recommended: native.restart_recommended,
            first_seen_at_ms: now_ms(),
            last_seen_at_ms: now_ms(),
        };
    }

    FailureSnapshot {
        operation: operation.to_string(),
        stage: stage.to_string(),
        message: error.to_string(),
        elapsed_ms: 0,
        timed_out: false,
        restart_recommended: false,
        first_seen_at_ms: now_ms(),
        last_seen_at_ms: now_ms(),
    }
}

fn parse_native_operation_error(message: &str) -> Option<NativeOperationError> {
    let payload = message.strip_prefix("bridge error: ").unwrap_or(message);
    let parsed: NativeOperationError = serde_json::from_str(payload).ok()?;
    (parsed.kind == "operation_error").then_some(parsed)
}

impl ServeDiagnostics {
    fn new(runtime: RuntimeStatus) -> Self {
        Self {
            runtime,
            active_operation: None,
            last_failure: None,
            total_operations: 0,
            successful_operations: 0,
            timed_out_operations: 0,
            next_operation_id: 1,
        }
    }

    fn begin_operation(&mut self, name: &str, stage: &str) {
        self.total_operations += 1;
        self.active_operation = Some(ActiveOperation {
            id: self.next_operation_id,
            name: name.to_string(),
            stage: stage.to_string(),
            started_at_ms: now_ms(),
            started_at: Instant::now(),
            timed_out: false,
        });
        self.next_operation_id += 1;
    }

    fn complete_success(&mut self, _name: &str, runtime: RuntimeStatus) {
        self.successful_operations += 1;
        self.runtime = runtime;
        self.active_operation = None;
        self.last_failure = None;
    }

    fn complete_failure(
        &mut self,
        _name: &str,
        mut failure: FailureSnapshot,
        runtime: Option<RuntimeStatus>,
    ) {
        if let Some(runtime) = runtime {
            self.runtime = runtime;
        }
        if let Some(previous) = self.last_failure.as_ref() {
            failure.first_seen_at_ms = previous.first_seen_at_ms;
        }
        self.last_failure = Some(failure);
        self.active_operation = None;
    }

    fn mark_timeout(&mut self, operation: &str, elapsed_ms: u64) {
        self.timed_out_operations += 1;
        self.runtime.host.cancel_requested = true;
        if let Some(active) = self.active_operation.as_mut() {
            active.timed_out = true;
            active.stage = "awaiting_control_plane_reply".into();
        }
        let now = now_ms();
        self.last_failure = Some(FailureSnapshot {
            operation: operation.to_string(),
            stage: "awaiting_control_plane_reply".into(),
            message: "the API timed out waiting for the runtime owner thread to reply and requested cancellation".into(),
            elapsed_ms,
            timed_out: true,
            restart_recommended: false,
            first_seen_at_ms: self
                .last_failure
                .as_ref()
                .map(|failure| failure.first_seen_at_ms)
                .unwrap_or(now),
            last_seen_at_ms: now,
        });
    }

    fn record_runtime_snapshot(&mut self, runtime: RuntimeStatus) {
        self.runtime = runtime;
        let host = &self.runtime.host;
        if self.runtime.bootstrapped
            && host.browser_available
            && host.page_ready
            && host.renderer_ready
            && host.runtime_ready
            && !host.load_in_progress
        {
            self.last_failure = None;
        }
    }

    fn mark_cancel_requested(&mut self) {
        self.runtime.host.cancel_requested = true;
        if let Some(active) = self.active_operation.as_mut() {
            active.stage = "cancelling_active_operation".into();
        }
    }

    fn snapshot(&self) -> RuntimeDiagnosticsResponse {
        let active_operation = self
            .active_operation
            .as_ref()
            .map(|operation| OperationSnapshot {
                id: operation.id,
                name: operation.name.clone(),
                stage: operation.stage.clone(),
                started_at_ms: operation.started_at_ms,
                elapsed_ms: operation.started_at.elapsed().as_millis() as u64,
                timed_out: operation.timed_out,
            });
        let host = &self.runtime.host;
        let runtime_operational = self.runtime.bootstrapped
            && host.browser_available
            && host.renderer_ready
            && host.runtime_ready;
        let state = if host.cancel_requested {
            RuntimeOperationalState::Cancelling
        } else if active_operation.as_ref().is_some_and(|op| op.timed_out) {
            RuntimeOperationalState::Wedged
        } else if !self.runtime.bootstrapped {
            RuntimeOperationalState::Starting
        } else if self
            .last_failure
            .as_ref()
            .is_some_and(|failure| failure.timed_out && failure.restart_recommended)
        {
            RuntimeOperationalState::Wedged
        } else if active_operation.is_some() || host.load_in_progress {
            RuntimeOperationalState::Busy
        } else if host.browser_closed
            || !host.browser_available
            || !host.renderer_ready
            || !host.runtime_ready
        {
            RuntimeOperationalState::Degraded
        } else if self.last_failure.is_some() {
            RuntimeOperationalState::Degraded
        } else if runtime_operational
            && self
                .runtime
                .last_successful_bridge_roundtrip_at_ms
                .is_some()
        {
            RuntimeOperationalState::Ready
        } else {
            RuntimeOperationalState::Degraded
        };
        let command_ready = matches!(state, RuntimeOperationalState::Ready);
        RuntimeDiagnosticsResponse {
            version: env!("CARGO_PKG_VERSION"),
            protocol_version: PROTOCOL_VERSION,
            state,
            control_plane_up: true,
            command_ready,
            bridge_healthy: self
                .runtime
                .last_successful_bridge_roundtrip_at_ms
                .is_some()
                && runtime_operational
                && self.last_failure.is_none(),
            browser_backend_healthy: self.runtime.bootstrapped
                && host.browser_available
                && !host.browser_closed
                && self
                    .last_failure
                    .as_ref()
                    .is_none_or(|failure| !failure.restart_recommended),
            browser_process_up: host.browser_available && !host.browser_closed,
            page_attached: host.page_ready,
            renderer_attached: host.renderer_ready,
            dom_snapshot_available: self.runtime.dom_snapshot_available,
            event_decoder_ok: self
                .last_failure
                .as_ref()
                .is_none_or(|failure| !failure.message.contains("unknown variant")),
            active_operation,
            last_failure: self.last_failure.clone(),
            total_operations: self.total_operations,
            successful_operations: self.successful_operations,
            timed_out_operations: self.timed_out_operations,
            runtime: self.runtime.clone(),
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_api_state(profile: &str) -> ApiState {
        let (tx, _rx) = mpsc::channel();
        ApiState {
            tx,
            cancel: Arc::new(Mutex::new(None)),
            host_library: PathBuf::from("/tmp/aegis_host.dylib"),
            browser: BrowserConfig::default(),
            startup: Arc::new(Mutex::new(ServeStartupMetrics {
                client_connect_ms: 0,
                api_bind_ms: 0,
                total_ready_ms: 0,
            })),
            profile: SessionProfileInfo {
                profile: profile.into(),
                path: PathBuf::from(format!("/tmp/{profile}.json")),
            },
            diagnostics: Arc::new(Mutex::new(ServeDiagnostics::new(default_runtime_status()))),
        }
    }

    fn dummy_managed_context(profile: &str) -> ManagedContext {
        ManagedContext {
            api: dummy_api_state(profile),
            owner_thread: None,
        }
    }

    #[test]
    fn validate_context_name_rejects_invalid_characters() {
        assert!(validate_context_name("guest-1").is_ok());
        assert!(validate_context_name("guest/admin").is_err());
        assert!(validate_context_name("").is_err());
    }

    #[test]
    fn list_contexts_sorts_and_marks_default() {
        let root = ServeRootState {
            host_library: PathBuf::from("/tmp/aegis_host.dylib"),
            browser: BrowserConfig::default(),
            default_context_id: "default".into(),
            contexts: Arc::new(Mutex::new(HashMap::from([
                ("guest".into(), dummy_managed_context("guest")),
                ("default".into(), dummy_managed_context("default")),
            ]))),
        };

        let contexts = root
            .list_contexts()
            .unwrap_or_else(|error| panic!("contexts should list: {error:?}"));
        assert_eq!(contexts.len(), 2);
        assert_eq!(contexts[0].id, "default");
        assert!(contexts[0].default);
        assert_eq!(contexts[0].runtime_state, "starting");
        assert!(!contexts[0].command_ready);
        assert_eq!(contexts[1].id, "guest");
        assert!(!contexts[1].default);
    }
}
