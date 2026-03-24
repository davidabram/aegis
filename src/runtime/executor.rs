use crate::browser::BrowserConfig;
use serde::{Deserialize, Serialize};

use crate::commands::command::{Command, CommandResult};
use crate::dom::diff::DomMutation;
use crate::dom::tree::DomTree;
use crate::events::stream::{EventStream, RuntimeEvent, SequencedEvent};
use crate::runtime::scheduler::Scheduler;
use crate::session::cookies::SessionState;
use crate::trace::recorder::TraceRecorder;
use crate::transport::bridge::{
    AegisError, BatchRequest, BatchResponse, BridgeEventEnvelope, CefBridge,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionReport {
    pub batch_id: u64,
    pub results: Vec<CommandResult>,
    pub latest_event_sequence: u64,
}

pub struct AegisRuntime {
    bridge: CefBridge,
    browser_config: BrowserConfig,
    dom: DomTree,
    events: EventStream,
    scheduler: Scheduler,
    trace_recorder: Option<TraceRecorder>,
}

impl AegisRuntime {
    pub fn new(mut bridge: CefBridge, browser_config: BrowserConfig) -> Result<Self, AegisError> {
        bridge.install_runtime()?;
        let snapshot = bridge.snapshot_dom()?;
        Ok(Self {
            bridge,
            browser_config,
            dom: DomTree::from_snapshot(snapshot),
            events: EventStream::default(),
            scheduler: Scheduler::default(),
            trace_recorder: None,
        })
    }

    pub fn execute(&mut self, commands: &[Command]) -> Result<ExecutionReport, AegisError> {
        let batch_id = self.scheduler.next_batch_id();
        let request = BatchRequest {
            batch_id,
            commands: commands.to_vec(),
        };
        let response = self.bridge.send_batch(&request)?;
        let results = response.results.clone();
        let emitted_events = self.apply_response(response.clone())?;
        self.record_trace(request, response, &emitted_events)?;

        Ok(ExecutionReport {
            batch_id,
            results,
            latest_event_sequence: self.events.latest_sequence(),
        })
    }

    pub fn navigate(&mut self, url: String) -> Result<Vec<SequencedEvent>, AegisError> {
        let response = self.bridge.navigate(&url)?;
        let request = BatchRequest {
            batch_id: self.scheduler.next_batch_id(),
            commands: Vec::new(),
        };
        let emitted_events = self.apply_response(response.clone())?;
        self.record_trace(request, response, &emitted_events)?;
        Ok(emitted_events)
    }

    fn apply_response(
        &mut self,
        response: BatchResponse,
    ) -> Result<Vec<SequencedEvent>, AegisError> {
        if let Some(snapshot) = response.snapshot.clone() {
            self.dom.replace_snapshot(snapshot);
        }

        let mut raw_events = response.events;
        raw_events.extend(self.bridge.drain_events()?);
        self.apply_dom_mutations(&raw_events);

        let events = raw_events
            .into_iter()
            .map(|event| self.sequence_event(event))
            .collect::<Vec<_>>();
        self.events.push_all(events.clone());

        Ok(events)
    }

    fn apply_dom_mutations(&mut self, events: &[BridgeEventEnvelope]) {
        let mut changes = Vec::<DomMutation>::new();
        for event in events {
            if let RuntimeEvent::DomMutation { changes: event_changes } = &event.event {
                changes.extend(event_changes.iter().cloned());
            }
        }
        if !changes.is_empty() {
            self.dom.apply_mutations(&changes);
        }
    }

    fn sequence_event(&mut self, event: BridgeEventEnvelope) -> SequencedEvent {
        SequencedEvent {
            sequence: self.scheduler.next_event_sequence(),
            timestamp_ms: self.scheduler.next_timestamp_ms(),
            event: event.event,
        }
    }

    pub fn inject_session(&mut self, session: SessionState) -> Result<(), AegisError> {
        session.validate().map_err(AegisError::InvalidSession)?;
        if let Some(recorder) = &mut self.trace_recorder {
            recorder.set_initial_session(session.clone());
            recorder.flush()?;
        }
        self.bridge.inject_session(session)
    }

    pub fn snapshot_session(&mut self) -> Result<SessionState, AegisError> {
        self.bridge.snapshot_session()
    }

    pub fn pump(&mut self) -> Result<(), AegisError> {
        self.bridge.pump()
    }

    pub fn dom_snapshot(&self) -> crate::dom::node::DomSnapshot {
        self.dom.snapshot()
    }

    pub fn event_stream(&self) -> &EventStream {
        &self.events
    }

    pub fn bridge(&self) -> &CefBridge {
        &self.bridge
    }

    pub fn enable_trace_recording(&mut self, path: impl Into<std::path::PathBuf>) {
        self.trace_recorder = Some(TraceRecorder::new(path, self.browser_config.clone()));
    }

    pub fn browser_config(&self) -> &BrowserConfig {
        &self.browser_config
    }

    fn record_trace(
        &mut self,
        request: BatchRequest,
        response: BatchResponse,
        emitted_events: &[SequencedEvent],
    ) -> Result<(), AegisError> {
        if let Some(recorder) = &mut self.trace_recorder {
            recorder.record_batch(request, response, emitted_events);
            recorder.flush()?;
        }
        Ok(())
    }
}
