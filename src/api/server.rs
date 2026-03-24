use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

use crate::browser::BrowserConfig;
use crate::commands::command::{Command, CommandMatcher, CommandTarget};
use crate::config_store::{AegisConfigStore, AegisSecretStore, CredentialInput};
use crate::dom::node::{DomNode, DomNodeSemantics, DomSnapshot};
use crate::events::stream::SequencedEvent;
use crate::host::LoadedAegisClient;
use crate::runtime::executor::{ExecutionReport, RuntimeStatus};
use crate::session::cookies::SessionState;
use crate::session::profile::{SessionProfileInfo, SessionProfileStore};
use crate::transport::bridge::AegisError;

const IDLE_PUMP_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Clone)]
pub struct ApiState {
    tx: mpsc::Sender<ApiCommand>,
    host_library: PathBuf,
    startup: Arc<Mutex<ServeStartupMetrics>>,
    profile: SessionProfileInfo,
}

#[derive(Debug, Clone, Serialize)]
pub struct ServeStartupMetrics {
    client_connect_ms: u64,
    api_bind_ms: u64,
    total_ready_ms: u64,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
}

#[derive(Debug, Deserialize)]
pub struct NavigateBody {
    pub url: String,
}

#[derive(Debug, Deserialize)]
pub struct ExecuteBody {
    pub commands: Vec<Command>,
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

#[derive(Debug, Serialize)]
pub struct ApiErrorBody {
    pub error: String,
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
    Events(
        u64,
        oneshot::Sender<Result<Vec<SequencedEvent>, AegisError>>,
    ),
    EnableTrace(PathBuf, oneshot::Sender<Result<(), AegisError>>),
    RuntimeInfo(oneshot::Sender<(BrowserConfig, RuntimeStatus)>),
}

#[derive(Debug, Clone)]
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

pub async fn serve(
    addr: SocketAddr,
    host_library: PathBuf,
    browser_config: BrowserConfig,
    profile_name: String,
) -> Result<(), AegisError> {
    let serve_started = std::time::Instant::now();
    let client_connect_started = std::time::Instant::now();
    let mut client = LoadedAegisClient::connect(host_library.clone(), browser_config.clone())?;
    let profile_store = SessionProfileStore::new(profile_name).map_err(AegisError::Bridge)?;
    let credential_settings = AegisConfigStore::detect()
        .and_then(|store| store.load_credentials_settings())
        .map_err(AegisError::Bridge)?;
    let credential_store = AegisSecretStore::detect().map_err(AegisError::Bridge)?;
    let mut credential_capture = AutoCredentialCapture::default();
    if let Some(session) = profile_store.load().map_err(AegisError::Bridge)? {
        client.inject_session(session)?;
    }
    let client_connect_ms = client_connect_started.elapsed().as_millis() as u64;
    let api_bind_started = std::time::Instant::now();
    let (tx, rx) = mpsc::channel::<ApiCommand>();
    let startup = Arc::new(Mutex::new(ServeStartupMetrics {
        client_connect_ms,
        api_bind_ms: 0,
        total_ready_ms: 0,
    }));
    let (startup_tx, startup_rx) = mpsc::channel::<Result<(), String>>();
    let state = ApiState {
        tx,
        host_library,
        startup: startup.clone(),
        profile: profile_store.info(),
    };
    let startup_host_library = state.host_library.clone();

    thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                let _ = startup_tx.send(Err(error.to_string()));
                return;
            }
        };

        runtime.block_on(async move {
            let listener = match tokio::net::TcpListener::bind(addr).await {
                Ok(listener) => {
                    let _ = startup_tx.send(Ok(()));
                    listener
                }
                Err(error) => {
                    let _ = startup_tx.send(Err(error.to_string()));
                    return;
                }
            };

            let app = router(state);
            let _ = axum::serve(listener, app).await;
        });
    });

    match startup_rx.recv() {
        Ok(Ok(())) => {}
        Ok(Err(error)) => return Err(AegisError::Bridge(error)),
        Err(error) => return Err(AegisError::Bridge(error.to_string())),
    }

    let startup_metrics = ServeStartupMetrics {
        client_connect_ms,
        api_bind_ms: api_bind_started.elapsed().as_millis() as u64,
        total_ready_ms: serve_started.elapsed().as_millis() as u64,
    };
    if let Ok(mut shared) = startup.lock() {
        *shared = startup_metrics;
    }

    eprintln!(
        "Aegis serve ready on http://{} ({:?}, host: {})",
        addr,
        browser_config.mode,
        startup_host_library.display()
    );

    loop {
        match rx.recv_timeout(IDLE_PUMP_INTERVAL) {
            Ok(command) => match command {
                ApiCommand::InjectSession(session, reply) => {
                    let result = client.inject_session(session.clone()).and_then(|_| {
                        profile_store
                            .save(&session)
                            .map(|_| ())
                            .map_err(AegisError::Bridge)
                    });
                    let _ = reply.send(result);
                }
                ApiCommand::SnapshotSession(reply) => {
                    let _ = reply.send(client.snapshot_session());
                }
                ApiCommand::SaveSessionProfile(reply) => {
                    let result = client.snapshot_session().and_then(|session| {
                        profile_store
                            .save(&session)
                            .map(|_| profile_store.info())
                            .map_err(AegisError::Bridge)
                    });
                    let _ = reply.send(result);
                }
                ApiCommand::LoadSessionProfile(reply) => {
                    let result = profile_store.load().map_err(AegisError::Bridge).and_then(
                        |maybe_session| match maybe_session {
                            Some(session) => {
                                client.inject_session(session).map(|_| profile_store.info())
                            }
                            None => Ok(profile_store.info()),
                        },
                    );
                    let _ = reply.send(result);
                }
                ApiCommand::Navigate(url, reply) => {
                    credential_capture.reset_on_explicit_navigation(&url);
                    let result = client.navigate(url);
                    let _ = reply.send(result);
                }
                ApiCommand::Execute(commands, reply) => {
                    let maybe_snapshot = if credential_settings.auto_store
                        && commands.iter().any(|command| {
                            matches!(command, Command::SetValue { .. } | Command::Click { .. })
                        }) {
                        Some(client.snapshot_dom()?)
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
                    let _ = reply.send(result);
                }
                ApiCommand::SnapshotDom(reply) => {
                    let _ = reply.send(client.snapshot_dom());
                }
                ApiCommand::Events(since, reply) => {
                    let _ = reply.send(client.events_since(since));
                }
                ApiCommand::EnableTrace(path, reply) => {
                    client.enable_trace_recording(path);
                    let _ = reply.send(Ok(()));
                }
                ApiCommand::RuntimeInfo(reply) => {
                    let _ = reply.send((browser_config.clone(), client.runtime_status()));
                }
            },
            Err(mpsc::RecvTimeoutError::Timeout) => {
                client.pump()?;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    if let Ok(session) = client.snapshot_session() {
        let _ = profile_store.save(&session);
    }

    Ok(())
}

impl Default for AutoCredentialCapture {
    fn default() -> Self {
        Self {
            username: None,
            password: None,
            origin: None,
        }
    }
}

impl AutoCredentialCapture {
    fn capture_fields(
        &mut self,
        snapshot: &DomSnapshot,
        current_url: Option<&str>,
        commands: &[Command],
    ) {
        let current_origin = current_url.map(origin_key);
        if let (Some(existing), Some(current)) = (self.origin.as_ref(), current_origin.as_ref()) {
            if existing != current {
                self.clear();
            }
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
    match target {
        CommandTarget::Id { id } => snapshot.nodes.iter().find(|node| node.id == *id),
        CommandTarget::Match { matcher } => snapshot
            .nodes
            .iter()
            .find(|node| node_matches_command_matcher(node, matcher)),
    }
}

fn node_matches_command_matcher(node: &DomNode, matcher: &CommandMatcher) -> bool {
    let semantic = node.semantic.as_ref().cloned().unwrap_or(DomNodeSemantics {
        role: None,
        name: None,
        label: None,
        control_type: None,
        actionable: false,
        disabled: false,
        actions: Vec::new(),
    });
    if matcher
        .role
        .as_ref()
        .is_some_and(|value| !includes_normalized(semantic.role.as_deref(), value))
    {
        return false;
    }
    if matcher
        .name
        .as_ref()
        .is_some_and(|value| !includes_normalized(semantic.name.as_deref(), value))
    {
        return false;
    }
    if matcher
        .label
        .as_ref()
        .is_some_and(|value| !includes_normalized(semantic.label.as_deref(), value))
    {
        return false;
    }
    if matcher
        .control_type
        .as_ref()
        .is_some_and(|value| !includes_normalized(semantic.control_type.as_deref(), value))
    {
        return false;
    }
    if matcher
        .tag
        .as_ref()
        .is_some_and(|value| !includes_normalized(Some(node.tag.as_str()), value))
    {
        return false;
    }
    if matcher
        .text
        .as_ref()
        .is_some_and(|value| !includes_normalized(node.text.as_deref(), value))
    {
        return false;
    }
    if matcher.placeholder.as_ref().is_some_and(|value| {
        !includes_normalized(node.attrs.get("placeholder").map(String::as_str), value)
    }) {
        return false;
    }
    if matcher.href_contains.as_ref().is_some_and(|value| {
        !includes_normalized(node.attrs.get("href").map(String::as_str), value)
    }) {
        return false;
    }
    if matcher
        .actionable
        .is_some_and(|value| semantic.actionable != value)
    {
        return false;
    }
    if matcher
        .disabled
        .is_some_and(|value| semantic.disabled != value)
    {
        return false;
    }
    true
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

pub fn router(state: ApiState) -> Router {
    Router::new()
        .route("/healthz", get(health))
        .route("/runtime", get(runtime_info))
        .route("/session", post(inject_session).get(snapshot_session))
        .route("/session/save", post(save_session_profile))
        .route("/session/load", post(load_session_profile))
        .route("/navigate", post(navigate))
        .route("/execute", post(execute))
        .route("/dom", get(snapshot_dom))
        .route("/events", get(events))
        .route("/trace/enable", post(enable_trace))
        .with_state(state)
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

#[derive(Debug, Serialize)]
struct RuntimeInfo {
    host_library: PathBuf,
    browser: BrowserConfig,
    runtime: RuntimeStatus,
    startup: ServeStartupMetrics,
    profile: SessionProfileInfo,
}

async fn runtime_info(State(state): State<ApiState>) -> Result<Json<RuntimeInfo>, ApiError> {
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::RuntimeInfo(reply_tx))
        .map_err(channel_error)?;
    let (browser, runtime) = reply_rx.await.map_err(reply_error_config)?;
    Ok(Json(RuntimeInfo {
        host_library: state.host_library,
        browser,
        runtime,
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

async fn save_session_profile(
    State(state): State<ApiState>,
) -> Result<Json<SessionProfileInfo>, ApiError> {
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::SaveSessionProfile(reply_tx))
        .map_err(channel_error)?;
    let profile = reply_rx.await.map_err(reply_error)??;
    Ok(Json(profile))
}

async fn load_session_profile(
    State(state): State<ApiState>,
) -> Result<Json<SessionProfileInfo>, ApiError> {
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::LoadSessionProfile(reply_tx))
        .map_err(channel_error)?;
    let profile = reply_rx.await.map_err(reply_error)??;
    Ok(Json(profile))
}

async fn inject_session(
    State(state): State<ApiState>,
    Json(body): Json<SessionState>,
) -> Result<StatusCode, ApiError> {
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::InjectSession(body, reply_tx))
        .map_err(channel_error)?;
    reply_rx.await.map_err(reply_error)??;
    Ok(StatusCode::NO_CONTENT)
}

async fn snapshot_session(State(state): State<ApiState>) -> Result<Json<SessionState>, ApiError> {
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::SnapshotSession(reply_tx))
        .map_err(channel_error)?;
    Ok(Json(reply_rx.await.map_err(reply_error)??))
}

async fn navigate(
    State(state): State<ApiState>,
    Json(body): Json<NavigateBody>,
) -> Result<Json<Vec<SequencedEvent>>, ApiError> {
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::Navigate(body.url, reply_tx))
        .map_err(channel_error)?;
    Ok(Json(reply_rx.await.map_err(reply_error)??))
}

async fn execute(
    State(state): State<ApiState>,
    Json(body): Json<ExecuteBody>,
) -> Result<Json<ExecutionReport>, ApiError> {
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::Execute(body.commands, reply_tx))
        .map_err(channel_error)?;
    Ok(Json(reply_rx.await.map_err(reply_error)??))
}

async fn snapshot_dom(State(state): State<ApiState>) -> Result<Json<DomSnapshot>, ApiError> {
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::SnapshotDom(reply_tx))
        .map_err(channel_error)?;
    Ok(Json(reply_rx.await.map_err(reply_error)??))
}

async fn events(
    State(state): State<ApiState>,
    Query(query): Query<EventQuery>,
) -> Result<Json<Vec<SequencedEvent>>, ApiError> {
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::Events(query.since, reply_tx))
        .map_err(channel_error)?;
    Ok(Json(reply_rx.await.map_err(reply_error)??))
}

async fn enable_trace(
    State(state): State<ApiState>,
    Json(body): Json<TraceBody>,
) -> Result<StatusCode, ApiError> {
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::EnableTrace(body.path, reply_tx))
        .map_err(channel_error)?;
    reply_rx.await.map_err(reply_error)??;
    Ok(StatusCode::NO_CONTENT)
}

fn channel_error(error: mpsc::SendError<ApiCommand>) -> ApiError {
    ApiError(AegisError::Bridge(error.to_string()))
}

fn reply_error(error: oneshot::error::RecvError) -> ApiError {
    ApiError(AegisError::Bridge(error.to_string()))
}

fn reply_error_config(error: oneshot::error::RecvError) -> ApiError {
    ApiError(AegisError::Bridge(error.to_string()))
}

struct ApiError(AegisError);

impl From<AegisError> for ApiError {
    fn from(value: AegisError) -> Self {
        Self(value)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let body = Json(ApiErrorBody {
            error: self.0.to_string(),
        });
        (StatusCode::BAD_REQUEST, body).into_response()
    }
}
