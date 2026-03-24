use crate::browser::BrowserConfig;
use crate::commands::command::Command;
use crate::events::stream::{EventType, SequencedEvent};
use crate::runtime::executor::{AegisRuntime, ExecutionReport};
use crate::session::cookies::SessionState;
use crate::transport::bridge::{AegisError, CefBridge};

pub struct AegisClient {
    runtime: AegisRuntime,
}

impl AegisClient {
    pub fn connect(bridge: CefBridge, browser_config: BrowserConfig) -> Result<Self, AegisError> {
        Ok(Self {
            runtime: AegisRuntime::new(bridge, browser_config)?,
        })
    }

    pub fn navigate(&mut self, url: impl Into<String>) -> Result<Vec<SequencedEvent>, AegisError> {
        self.runtime.navigate(url.into())
    }

    pub fn execute(&mut self, commands: &[Command]) -> Result<ExecutionReport, AegisError> {
        self.runtime.execute(commands)
    }

    pub fn inject_session(&mut self, session: SessionState) -> Result<(), AegisError> {
        self.runtime.inject_session(session)
    }

    pub fn pump(&mut self) -> Result<(), AegisError> {
        self.runtime.pump()
    }

    pub fn navigation_events_since(&self, sequence: u64) -> Vec<SequencedEvent> {
        self.runtime
            .event_stream()
            .read_from(sequence, Some(EventType::Navigation))
    }

    pub fn runtime(&self) -> &AegisRuntime {
        &self.runtime
    }

    pub fn runtime_mut(&mut self) -> &mut AegisRuntime {
        &mut self.runtime
    }

    pub fn browser_config(&self) -> &BrowserConfig {
        self.runtime.browser_config()
    }
}
