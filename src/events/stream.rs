use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::dom::diff::DomMutation;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventType {
    DomMutation,
    Navigation,
    Network,
    Log,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
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
    },
    Log {
        level: String,
        message: String,
        data: Option<Value>,
    },
}

impl RuntimeEvent {
    pub fn event_type(&self) -> EventType {
        match self {
            RuntimeEvent::DomMutation { .. } => EventType::DomMutation,
            RuntimeEvent::Navigation { .. } => EventType::Navigation,
            RuntimeEvent::Network { .. } => EventType::Network,
            RuntimeEvent::Log { .. } => EventType::Log,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SequencedEvent {
    pub sequence: u64,
    pub timestamp_ms: u64,
    pub event: RuntimeEvent,
}

#[derive(Debug, Clone, Default)]
pub struct EventStream {
    events: Vec<SequencedEvent>,
}

impl EventStream {
    pub fn push(&mut self, event: SequencedEvent) {
        self.events.push(event);
    }

    pub fn push_all(&mut self, events: Vec<SequencedEvent>) {
        self.events.extend(events);
    }

    pub fn read_from(&self, sequence: u64, filter: Option<EventType>) -> Vec<SequencedEvent> {
        self.events
            .iter()
            .filter(|entry| entry.sequence > sequence)
            .filter(|entry| filter.is_none_or(|kind| entry.event.event_type() == kind))
            .cloned()
            .collect()
    }

    pub fn latest_sequence(&self) -> u64 {
        self.events.last().map(|event| event.sequence).unwrap_or(0)
    }
}
