use serde::{Deserialize, Serialize};
use serde_json::{from_slice, to_vec};

use crate::browser::BrowserConfig;
use crate::commands::command::CommandResult;
use crate::dom::node::DomSnapshot;
use crate::events::stream::RuntimeEvent;
use crate::session::cookies::SessionState;
use crate::transport::bridge::{AegisError, BatchRequest, BridgeEventEnvelope};

const MAGIC: [u8; 4] = *b"AEGS";
pub const PROTOCOL_VERSION: u16 = 1;
const HEADER_LEN: usize = 16;

fn is_false(value: &bool) -> bool {
    !*value
}

fn deserialize_u64ish<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    if let Some(integer) = value.as_u64() {
        return Ok(integer);
    }
    if let Some(float) = value.as_f64()
        && float.is_finite() && float >= 0.0
    {
        return Ok(float as u64);
    }
    Err(serde::de::Error::custom("expected unsigned integer"))
}

fn deserialize_option_u64ish<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    let Some(value) = value else {
        return Ok(None);
    };
    if let Some(integer) = value.as_u64() {
        return Ok(Some(integer));
    }
    if let Some(float) = value.as_f64()
        && float.is_finite() && float >= 0.0
    {
        return Ok(Some(float as u64));
    }
    Err(serde::de::Error::custom("expected optional unsigned integer"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u16)]
pub enum MessageKind {
    EnsureRuntime = 1,
    EvalJs = 2,
    SendBatch = 3,
    SnapshotDom = 4,
    InjectSession = 5,
    SnapshotSession = 6,
    DrainEvents = 7,
    Navigate = 8,
    SnapshotHostState = 9,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MessageEnvelope<T> {
    pub kind: MessageKind,
    pub payload: T,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvalJsRequest {
    pub script: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvalJsResponse {
    pub value: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NavigateRequest {
    pub url: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub capture_network_events: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DrainEventsRequest {
    #[serde(default, skip_serializing_if = "is_false")]
    pub enable_network_capture: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NavigateResponse {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<DomSnapshot>,
    #[serde(default)]
    pub events: Vec<BridgeEventEnvelope>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BatchWireResponse {
    pub batch_id: u64,
    pub results: Vec<CommandResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<DomSnapshot>,
    #[serde(default)]
    pub events: Vec<BridgeEventEnvelope>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventsResponse {
    pub events: Vec<BridgeEventEnvelope>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DownloadState {
    pub id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(default)]
    pub state: String,
    #[serde(default, deserialize_with = "deserialize_u64ish")]
    pub received_bytes: u64,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_option_u64ish"
    )]
    pub total_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percent_complete: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interrupt_reason: Option<String>,
    #[serde(default)]
    pub complete: bool,
    #[serde(default)]
    pub canceled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct HostRuntimeState {
    pub startup_complete: bool,
    pub browser_available: bool,
    #[serde(default)]
    pub active_context_id: Option<String>,
    #[serde(default)]
    pub active_browser_id: Option<i32>,
    #[serde(default)]
    pub context_id: Option<String>,
    #[serde(default)]
    pub browser_id: Option<i32>,
    #[serde(default)]
    pub request_context_available: bool,
    #[serde(default)]
    pub attached_browser_ids: Vec<i32>,
    #[serde(default)]
    pub known_context_ids: Vec<String>,
    pub page_ready: bool,
    pub renderer_ready: bool,
    #[serde(default)]
    pub runtime_installed: bool,
    pub runtime_ready: bool,
    pub load_in_progress: bool,
    pub browser_closed: bool,
    pub cancel_requested: bool,
    pub current_url: Option<String>,
    #[serde(default)]
    pub download_dir: Option<std::path::PathBuf>,
    #[serde(default)]
    pub downloads: Vec<DownloadState>,
    pub active_operation: Option<String>,
    pub active_stage: Option<String>,
}

pub fn encode_message<T: Serialize>(kind: MessageKind, payload: &T) -> Result<Vec<u8>, AegisError> {
    let body = to_vec(&MessageEnvelope { kind, payload })
        .map_err(|error| AegisError::Protocol(error.to_string()))?;

    let mut frame = Vec::with_capacity(HEADER_LEN + body.len());
    frame.extend_from_slice(&MAGIC);
    frame.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
    frame.extend_from_slice(&(kind as u16).to_le_bytes());
    frame.extend_from_slice(&(body.len() as u64).to_le_bytes());
    frame.extend_from_slice(&body);
    Ok(frame)
}

pub fn decode_message<T: for<'de> Deserialize<'de>>(
    expected_kind: MessageKind,
    bytes: &[u8],
) -> Result<T, AegisError> {
    if bytes.len() < HEADER_LEN {
        return Err(AegisError::Protocol("frame too short".into()));
    }
    if bytes[0..4] != MAGIC {
        return Err(AegisError::Protocol("bad magic".into()));
    }

    let version = u16::from_le_bytes([bytes[4], bytes[5]]);
    if version != PROTOCOL_VERSION {
        return Err(AegisError::Protocol(format!(
            "unsupported protocol version {version}"
        )));
    }

    let kind = u16::from_le_bytes([bytes[6], bytes[7]]);
    let length = u64::from_le_bytes(bytes[8..16].try_into().expect("header length")) as usize;
    if bytes.len() != HEADER_LEN + length {
        return Err(AegisError::Protocol("frame length mismatch".into()));
    }
    if kind != expected_kind as u16 {
        return Err(AegisError::Protocol(format!(
            "unexpected message kind {kind}, expected {}",
            expected_kind as u16
        )));
    }

    let payload = &bytes[HEADER_LEN..];
    let envelope: MessageEnvelope<T> =
        from_slice(payload).map_err(|error| AegisError::Protocol(error.to_string()))?;
    if envelope.kind != expected_kind {
        return Err(AegisError::Protocol("envelope kind mismatch".into()));
    }

    Ok(envelope.payload)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraceEventRecord {
    pub sequence: u64,
    pub timestamp_ms: u64,
    pub event: RuntimeEvent,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraceBatchRecord {
    pub batch_id: u64,
    pub request: BatchRequest,
    pub response: BatchWireResponse,
    #[serde(default)]
    pub emitted_events: Vec<TraceEventRecord>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraceFile {
    pub protocol_version: u16,
    pub browser_config: BrowserConfig,
    #[serde(default)]
    pub initial_session: Option<SessionState>,
    #[serde(default)]
    pub batches: Vec<TraceBatchRecord>,
}
