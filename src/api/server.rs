use std::net::SocketAddr;
use std::path::PathBuf;
use std::thread;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot};

use crate::browser::BrowserConfig;
use crate::commands::command::Command;
use crate::dom::node::DomSnapshot;
use crate::events::stream::SequencedEvent;
use crate::host::LoadedAegisClient;
use crate::runtime::executor::ExecutionReport;
use crate::session::cookies::SessionState;
use crate::transport::bridge::AegisError;

#[derive(Clone)]
pub struct ApiState {
    tx: mpsc::Sender<ApiCommand>,
    host_library: PathBuf,
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
    BrowserConfig(oneshot::Sender<BrowserConfig>),
}

pub async fn serve(
    addr: SocketAddr,
    host_library: PathBuf,
    browser_config: BrowserConfig,
) -> Result<(), AegisError> {
    let state = ApiState {
        tx: spawn_control_plane(host_library.clone(), browser_config),
        host_library,
    };

    let app = router(state);
    let listener = TcpListener::bind(addr)
        .await
        .map_err(|error| AegisError::Bridge(error.to_string()))?;
    axum::serve(listener, app)
        .await
        .map_err(|error| AegisError::Bridge(error.to_string()))
}

fn spawn_control_plane(
    host_library: PathBuf,
    browser_config: BrowserConfig,
) -> mpsc::Sender<ApiCommand> {
    let (tx, mut rx) = mpsc::channel(64);

    thread::spawn(move || {
        let config = browser_config;
        let mut client = match LoadedAegisClient::connect(host_library, config.clone()) {
            Ok(client) => client,
            Err(error) => {
                while let Some(command) = rx.blocking_recv() {
                    send_startup_error(command, &error);
                }
                return;
            }
        };

        while let Some(command) = rx.blocking_recv() {
            match command {
                ApiCommand::InjectSession(session, reply) => {
                    let _ = reply.send(client.inject_session(session));
                }
                ApiCommand::SnapshotSession(reply) => {
                    let _ = reply.send(client.snapshot_session());
                }
                ApiCommand::Navigate(url, reply) => {
                    let _ = reply.send(client.navigate(url));
                }
                ApiCommand::Execute(commands, reply) => {
                    let _ = reply.send(client.execute(&commands));
                }
                ApiCommand::SnapshotDom(reply) => {
                    let _ = reply.send(Ok(client.snapshot_dom()));
                }
                ApiCommand::Events(since, reply) => {
                    let _ = reply.send(Ok(client.events_since(since)));
                }
                ApiCommand::EnableTrace(path, reply) => {
                    client.enable_trace_recording(path);
                    let _ = reply.send(Ok(()));
                }
                ApiCommand::BrowserConfig(reply) => {
                    let _ = reply.send(config.clone());
                }
            }
        }
    });

    tx
}

fn send_startup_error(command: ApiCommand, error: &AegisError) {
    let error = clone_error(error);
    match command {
        ApiCommand::InjectSession(_, reply) => {
            let _ = reply.send(Err(error));
        }
        ApiCommand::SnapshotSession(reply) => {
            let _ = reply.send(Err(error));
        }
        ApiCommand::Navigate(_, reply) => {
            let _ = reply.send(Err(error));
        }
        ApiCommand::Execute(_, reply) => {
            let _ = reply.send(Err(error));
        }
        ApiCommand::SnapshotDom(reply) => {
            let _ = reply.send(Err(error));
        }
        ApiCommand::Events(_, reply) => {
            let _ = reply.send(Err(error));
        }
        ApiCommand::EnableTrace(_, reply) => {
            let _ = reply.send(Err(error));
        }
        ApiCommand::BrowserConfig(_) => {}
    }
}

fn clone_error(error: &AegisError) -> AegisError {
    AegisError::Bridge(error.to_string())
}

pub fn router(state: ApiState) -> Router {
    Router::new()
        .route("/healthz", get(health))
        .route("/runtime", get(runtime_info))
        .route("/session", post(inject_session).get(snapshot_session))
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
}

async fn runtime_info(State(state): State<ApiState>) -> Result<Json<RuntimeInfo>, ApiError> {
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::BrowserConfig(reply_tx))
        .await
        .map_err(channel_error)?;
    let browser = reply_rx.await.map_err(reply_error_config)?;
    Ok(Json(RuntimeInfo {
        host_library: state.host_library,
        browser,
    }))
}

async fn inject_session(
    State(state): State<ApiState>,
    Json(body): Json<SessionState>,
) -> Result<StatusCode, ApiError> {
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::InjectSession(body, reply_tx))
        .await
        .map_err(channel_error)?;
    reply_rx.await.map_err(reply_error)??;
    Ok(StatusCode::NO_CONTENT)
}

async fn snapshot_session(State(state): State<ApiState>) -> Result<Json<SessionState>, ApiError> {
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::SnapshotSession(reply_tx))
        .await
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
        .await
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
        .await
        .map_err(channel_error)?;
    Ok(Json(reply_rx.await.map_err(reply_error)??))
}

async fn snapshot_dom(State(state): State<ApiState>) -> Result<Json<DomSnapshot>, ApiError> {
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::SnapshotDom(reply_tx))
        .await
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
        .await
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
        .await
        .map_err(channel_error)?;
    reply_rx.await.map_err(reply_error)??;
    Ok(StatusCode::NO_CONTENT)
}

fn channel_error(error: mpsc::error::SendError<ApiCommand>) -> ApiError {
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
