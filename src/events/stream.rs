use std::collections::VecDeque;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::dom::diff::DomMutation;

const DEFAULT_MAX_RETAINED_EVENTS: usize = 10_000;

fn u64_from_value<E: serde::de::Error>(value: &Value) -> Result<u64, E> {
    if let Some(integer) = value.as_u64() {
        return Ok(integer);
    }
    if let Some(float) = value.as_f64()
        && float.is_finite()
        && float >= 0.0
    {
        return Ok(float as u64);
    }
    Err(E::custom("expected unsigned integer"))
}

fn optional_u64_from_value<E: serde::de::Error>(value: Option<&Value>) -> Result<Option<u64>, E> {
    value.map(u64_from_value).transpose()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventType {
    DomMutation,
    Navigation,
    Network,
    Download,
    WebSocket,
    Log,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkResourcePhase {
    Request,
    Response,
    Finished,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebSocketFrameDirection {
    Sent,
    Received,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RuntimeEvent {
    DomMutation {
        changes: Vec<DomMutation>,
    },
    Navigation {
        url: String,
    },
    Network {
        request_id: String,
        url: String,
        method: Option<String>,
        resource_type: Option<String>,
        phase: Option<NetworkResourcePhase>,
        status: Option<u16>,
        status_text: Option<String>,
        mime_type: Option<String>,
        from_cache: Option<bool>,
        error_text: Option<String>,
    },
    Download {
        id: u64,
        url: Option<String>,
        suggested_name: Option<String>,
        target_path: Option<String>,
        mime_type: Option<String>,
        state: String,
        received_bytes: u64,
        total_bytes: Option<u64>,
        percent_complete: Option<i32>,
        interrupt_reason: Option<String>,
        complete: bool,
        canceled: bool,
    },
    WebSocketOpen {
        request_id: String,
        url: String,
    },
    WebSocketHandshake {
        request_id: String,
        url: String,
        status: Option<u16>,
        status_text: Option<String>,
    },
    WebSocketFrame {
        request_id: String,
        url: String,
        direction: WebSocketFrameDirection,
        opcode: Option<u8>,
        mask: Option<bool>,
        payload_preview: String,
        payload_length: usize,
        truncated: bool,
    },
    WebSocketClose {
        request_id: String,
        url: String,
    },
    Log {
        level: String,
        message: String,
        data: Option<Value>,
    },
    Unknown {
        event_type: String,
        payload: Value,
    },
}

impl Serialize for RuntimeEvent {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let value = match self {
            RuntimeEvent::DomMutation { changes } => {
                serde_json::json!({ "type": "dom_mutation", "changes": changes })
            }
            RuntimeEvent::Navigation { url } => {
                serde_json::json!({ "type": "navigation", "url": url })
            }
            RuntimeEvent::Network {
                request_id,
                url,
                method,
                resource_type,
                phase,
                status,
                status_text,
                mime_type,
                from_cache,
                error_text,
            } => serde_json::json!({
                "type": "network",
                "request_id": request_id,
                "url": url,
                "method": method,
                "resource_type": resource_type,
                "phase": phase,
                "status": status,
                "status_text": status_text,
                "mime_type": mime_type,
                "from_cache": from_cache,
                "error_text": error_text,
            }),
            RuntimeEvent::Download {
                id,
                url,
                suggested_name,
                target_path,
                mime_type,
                state,
                received_bytes,
                total_bytes,
                percent_complete,
                interrupt_reason,
                complete,
                canceled,
            } => serde_json::json!({
                "type": "download",
                "id": id,
                "url": url,
                "suggested_name": suggested_name,
                "target_path": target_path,
                "mime_type": mime_type,
                "state": state,
                "received_bytes": received_bytes,
                "total_bytes": total_bytes,
                "percent_complete": percent_complete,
                "interrupt_reason": interrupt_reason,
                "complete": complete,
                "canceled": canceled,
            }),
            RuntimeEvent::WebSocketOpen { request_id, url } => {
                serde_json::json!({ "type": "websocket_open", "request_id": request_id, "url": url })
            }
            RuntimeEvent::WebSocketHandshake {
                request_id,
                url,
                status,
                status_text,
            } => serde_json::json!({
                "type": "websocket_handshake",
                "request_id": request_id,
                "url": url,
                "status": status,
                "status_text": status_text,
            }),
            RuntimeEvent::WebSocketFrame {
                request_id,
                url,
                direction,
                opcode,
                mask,
                payload_preview,
                payload_length,
                truncated,
            } => serde_json::json!({
                "type": "websocket_frame",
                "request_id": request_id,
                "url": url,
                "direction": direction,
                "opcode": opcode,
                "mask": mask,
                "payload_preview": payload_preview,
                "payload_length": payload_length,
                "truncated": truncated,
            }),
            RuntimeEvent::WebSocketClose { request_id, url } => {
                serde_json::json!({ "type": "websocket_close", "request_id": request_id, "url": url })
            }
            RuntimeEvent::Log {
                level,
                message,
                data,
            } => serde_json::json!({
                "type": "log",
                "level": level,
                "message": message,
                "data": data,
            }),
            RuntimeEvent::Unknown { payload, .. } => payload.clone(),
        };
        value.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for RuntimeEvent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let payload = Value::deserialize(deserializer)?;
        let event_type = payload
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();

        fn from_value<T: serde::de::DeserializeOwned, E: serde::de::Error>(
            payload: &Value,
        ) -> Result<T, E> {
            serde_json::from_value(payload.clone()).map_err(E::custom)
        }

        match event_type.as_str() {
            "dom_mutation" => {
                #[derive(Deserialize)]
                struct DomMutationEvent {
                    changes: Vec<DomMutation>,
                }
                let value: DomMutationEvent = from_value(&payload)?;
                Ok(RuntimeEvent::DomMutation {
                    changes: value.changes,
                })
            }
            "navigation" => {
                #[derive(Deserialize)]
                struct NavigationEvent {
                    url: String,
                }
                let value: NavigationEvent = from_value(&payload)?;
                Ok(RuntimeEvent::Navigation { url: value.url })
            }
            "network" => {
                #[derive(Deserialize)]
                struct NetworkEvent {
                    request_id: String,
                    url: String,
                    method: Option<String>,
                    resource_type: Option<String>,
                    phase: Option<NetworkResourcePhase>,
                    status: Option<u16>,
                    status_text: Option<String>,
                    mime_type: Option<String>,
                    from_cache: Option<bool>,
                    error_text: Option<String>,
                }
                let value: NetworkEvent = from_value(&payload)?;
                Ok(RuntimeEvent::Network {
                    request_id: value.request_id,
                    url: value.url,
                    method: value.method,
                    resource_type: value.resource_type,
                    phase: value.phase,
                    status: value.status,
                    status_text: value.status_text,
                    mime_type: value.mime_type,
                    from_cache: value.from_cache,
                    error_text: value.error_text,
                })
            }
            "download" => {
                #[derive(Deserialize)]
                struct DownloadEvent {
                    id: u64,
                    url: Option<String>,
                    suggested_name: Option<String>,
                    target_path: Option<String>,
                    mime_type: Option<String>,
                    state: String,
                    received_bytes: Value,
                    total_bytes: Option<Value>,
                    percent_complete: Option<i32>,
                    interrupt_reason: Option<String>,
                    complete: bool,
                    canceled: bool,
                }
                let value: DownloadEvent = from_value(&payload)?;
                Ok(RuntimeEvent::Download {
                    id: value.id,
                    url: value.url,
                    suggested_name: value.suggested_name,
                    target_path: value.target_path,
                    mime_type: value.mime_type,
                    state: value.state,
                    received_bytes: u64_from_value(&value.received_bytes)?,
                    total_bytes: optional_u64_from_value(value.total_bytes.as_ref())?,
                    percent_complete: value.percent_complete,
                    interrupt_reason: value.interrupt_reason,
                    complete: value.complete,
                    canceled: value.canceled,
                })
            }
            "websocket_open" => {
                #[derive(Deserialize)]
                struct WebSocketOpenEvent {
                    request_id: String,
                    url: String,
                }
                let value: WebSocketOpenEvent = from_value(&payload)?;
                Ok(RuntimeEvent::WebSocketOpen {
                    request_id: value.request_id,
                    url: value.url,
                })
            }
            "websocket_handshake" => {
                #[derive(Deserialize)]
                struct WebSocketHandshakeEvent {
                    request_id: String,
                    url: String,
                    status: Option<u16>,
                    status_text: Option<String>,
                }
                let value: WebSocketHandshakeEvent = from_value(&payload)?;
                Ok(RuntimeEvent::WebSocketHandshake {
                    request_id: value.request_id,
                    url: value.url,
                    status: value.status,
                    status_text: value.status_text,
                })
            }
            "websocket_frame" => {
                #[derive(Deserialize)]
                struct WebSocketFrameEvent {
                    request_id: String,
                    url: String,
                    direction: WebSocketFrameDirection,
                    opcode: Option<u8>,
                    mask: Option<bool>,
                    payload_preview: String,
                    payload_length: usize,
                    truncated: bool,
                }
                let value: WebSocketFrameEvent = from_value(&payload)?;
                Ok(RuntimeEvent::WebSocketFrame {
                    request_id: value.request_id,
                    url: value.url,
                    direction: value.direction,
                    opcode: value.opcode,
                    mask: value.mask,
                    payload_preview: value.payload_preview,
                    payload_length: value.payload_length,
                    truncated: value.truncated,
                })
            }
            "websocket_close" => {
                #[derive(Deserialize)]
                struct WebSocketCloseEvent {
                    request_id: String,
                    url: String,
                }
                let value: WebSocketCloseEvent = from_value(&payload)?;
                Ok(RuntimeEvent::WebSocketClose {
                    request_id: value.request_id,
                    url: value.url,
                })
            }
            "log" => {
                #[derive(Deserialize)]
                struct LogEvent {
                    level: String,
                    message: String,
                    data: Option<Value>,
                }
                let value: LogEvent = from_value(&payload)?;
                Ok(RuntimeEvent::Log {
                    level: value.level,
                    message: value.message,
                    data: value.data,
                })
            }
            _ => Ok(RuntimeEvent::Unknown {
                event_type,
                payload,
            }),
        }
    }
}

impl RuntimeEvent {
    pub fn event_type(&self) -> EventType {
        match self {
            RuntimeEvent::DomMutation { .. } => EventType::DomMutation,
            RuntimeEvent::Navigation { .. } => EventType::Navigation,
            RuntimeEvent::Network { .. } => EventType::Network,
            RuntimeEvent::Download { .. } => EventType::Download,
            RuntimeEvent::WebSocketOpen { .. }
            | RuntimeEvent::WebSocketHandshake { .. }
            | RuntimeEvent::WebSocketFrame { .. }
            | RuntimeEvent::WebSocketClose { .. } => EventType::WebSocket,
            RuntimeEvent::Log { .. } => EventType::Log,
            RuntimeEvent::Unknown { .. } => EventType::Log,
        }
    }

    pub fn event_name(&self) -> &str {
        match self {
            RuntimeEvent::DomMutation { .. } => "dom_mutation",
            RuntimeEvent::Navigation { .. } => "navigation",
            RuntimeEvent::Network { .. } => "network",
            RuntimeEvent::Download { .. } => "download",
            RuntimeEvent::WebSocketOpen { .. } => "websocket_open",
            RuntimeEvent::WebSocketHandshake { .. } => "websocket_handshake",
            RuntimeEvent::WebSocketFrame { .. } => "websocket_frame",
            RuntimeEvent::WebSocketClose { .. } => "websocket_close",
            RuntimeEvent::Log { .. } => "log",
            RuntimeEvent::Unknown { event_type, .. } => event_type.as_str(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SequencedEvent {
    pub sequence: u64,
    pub timestamp_ms: u64,
    pub event: RuntimeEvent,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventReadWindow {
    pub requested_since: u64,
    pub oldest_available_sequence: Option<u64>,
    pub latest_sequence: u64,
    pub gap_detected: bool,
    pub events: Vec<SequencedEvent>,
}

#[derive(Debug, Clone)]
pub struct EventStream {
    events: VecDeque<SequencedEvent>,
    max_retained: Option<usize>,
}

impl Default for EventStream {
    fn default() -> Self {
        Self::with_max_retained(DEFAULT_MAX_RETAINED_EVENTS)
    }
}

impl EventStream {
    pub fn with_max_retained(max_retained: usize) -> Self {
        Self {
            events: VecDeque::new(),
            max_retained: Some(max_retained.max(1)),
        }
    }

    pub fn unbounded() -> Self {
        Self {
            events: VecDeque::new(),
            max_retained: None,
        }
    }

    pub fn push(&mut self, event: SequencedEvent) {
        self.events.push_back(event);
        self.trim_to_limit();
    }

    pub fn push_all(&mut self, events: Vec<SequencedEvent>) {
        self.events.extend(events);
        self.trim_to_limit();
    }

    pub fn read_from(&self, sequence: u64, filter: Option<EventType>) -> EventReadWindow {
        let oldest_available_sequence = self.oldest_sequence();
        let latest_sequence = self.latest_sequence();
        let gap_detected = oldest_available_sequence
            .is_some_and(|oldest| latest_sequence > 0 && sequence.saturating_add(1) < oldest);
        let events = self
            .events
            .iter()
            .filter(|entry| entry.sequence > sequence)
            .filter(|entry| filter.is_none_or(|kind| entry.event.event_type() == kind))
            .cloned()
            .collect();
        EventReadWindow {
            requested_since: sequence,
            oldest_available_sequence,
            latest_sequence,
            gap_detected,
            events,
        }
    }

    pub fn latest_sequence(&self) -> u64 {
        self.events.back().map(|event| event.sequence).unwrap_or(0)
    }

    pub fn oldest_sequence(&self) -> Option<u64> {
        self.events.front().map(|event| event.sequence)
    }

    pub fn retained_len(&self) -> usize {
        self.events.len()
    }

    fn trim_to_limit(&mut self) {
        let Some(max_retained) = self.max_retained else {
            return;
        };
        while self.events.len() > max_retained {
            let _ = self.events.pop_front();
        }
    }
}
