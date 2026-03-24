use crate::browser::BrowserConfig;
use crate::dom::node::DomSnapshot;
use crate::events::stream::{EventStream, RuntimeEvent, SequencedEvent};
use crate::session::cookies::SessionState;
use crate::trace::recorder::TraceRecorder;
use crate::transport::bridge::AegisError;

pub struct ReplayState {
    pub browser_config: BrowserConfig,
    pub session: Option<SessionState>,
    pub final_snapshot: DomSnapshot,
    pub events: EventStream,
}

pub fn replay_trace(path: impl Into<std::path::PathBuf>) -> Result<ReplayState, AegisError> {
    let recorder = TraceRecorder::load(path)?;
    let trace = recorder.trace();
    let mut final_snapshot = DomSnapshot::default();
    let mut events = EventStream::default();

    for batch in &trace.batches {
        if let Some(snapshot) = &batch.response.snapshot {
            final_snapshot = snapshot.clone();
        } else if batch
            .emitted_events
            .iter()
            .any(|event| matches!(event.event, RuntimeEvent::Navigation { .. }))
        {
            final_snapshot = DomSnapshot::default();
        }
        for event in &batch.emitted_events {
            events.push(SequencedEvent {
                sequence: event.sequence,
                timestamp_ms: event.timestamp_ms,
                event: event.event.clone(),
            });
        }
    }

    Ok(ReplayState {
        browser_config: trace.browser_config.clone(),
        session: trace.initial_session.clone(),
        final_snapshot,
        events,
    })
}
