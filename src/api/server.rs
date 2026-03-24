use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;
use std::thread;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

use crate::browser::BrowserConfig;
use crate::commands::command::Command;
use crate::dom::node::DomSnapshot;
use crate::events::stream::SequencedEvent;
use crate::host::LoadedAegisClient;
use crate::runtime::executor::ExecutionReport;
use crate::session::cookies::SessionState;
use crate::transport::bridge::AegisError;

const IDLE_PUMP_INTERVAL: Duration = Duration::from_millis(10);

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
    Navigate(String, oneshot::Sender<Result<Vec<SequencedEvent>, AegisError>>),
    Execute(
        Vec<Command>,
        oneshot::Sender<Result<ExecutionReport, AegisError>>,
    ),
    SnapshotDom(oneshot::Sender<Result<DomSnapshot, AegisError>>),
    Events(u64, oneshot::Sender<Result<Vec<SequencedEvent>, AegisError>>),
    EnableTrace(PathBuf, oneshot::Sender<Result<(), AegisError>>),
    BrowserConfig(oneshot::Sender<BrowserConfig>),
}

pub async fn serve(
    addr: SocketAddr,
    host_library: PathBuf,
    browser_config: BrowserConfig,
) -> Result<(), AegisError> {
    let mut client = LoadedAegisClient::connect(host_library.clone(), browser_config.clone())?;
    let (tx, rx) = mpsc::channel::<ApiCommand>();
    let state = ApiState {
        tx,
        host_library,
    };
    let (startup_tx, startup_rx) = mpsc::channel::<Result<(), String>>();

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

    loop {
        match rx.recv_timeout(IDLE_PUMP_INTERVAL) {
            Ok(command) => match command {
                ApiCommand::InjectSession(session, reply) => {
                    let _ = reply.send(client.inject_session(session));
                }
                ApiCommand::SnapshotSession(reply) => {
                    let _ = reply.send(client.snapshot_session());
                }
                ApiCommand::Navigate(url, reply) => {
                    let result = client.navigate(url);
                    let _ = reply.send(result);
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
                    let _ = reply.send(browser_config.clone());
                }
            },
            Err(mpsc::RecvTimeoutError::Timeout) => {
                client.pump()?;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    Ok(())
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
