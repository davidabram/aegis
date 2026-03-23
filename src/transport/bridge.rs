use std::ffi::c_void;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::ptr::NonNull;
use std::slice;
use std::process::Command as ProcessCommand;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::browser::BrowserConfig;
use crate::commands::command::{Command, CommandResult};
use crate::dom::node::DomSnapshot;
use crate::events::stream::RuntimeEvent;
use crate::native::{DEFAULT_APP_BUNDLE_PATH, prepare_bundled_cli};
use crate::session::cookies::SessionState;
use crate::transport::protocol::{
    BatchWireResponse, EvalJsRequest, EvalJsResponse, EventsResponse, MessageKind, NavigateRequest,
    NavigateResponse, decode_message, encode_message,
};

#[derive(Debug, Error)]
pub enum AegisError {
    #[error("serialization error: {0}")]
    Serialize(serde_json::Error),
    #[error("deserialization error: {0}")]
    Deserialize(serde_json::Error),
    #[error("bridge error: {0}")]
    Bridge(String),
    #[error("invalid session: {0}")]
    InvalidSession(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("utf8 error: {0}")]
    Utf8(#[from] std::str::Utf8Error),
    #[error("protocol error: {0}")]
    Protocol(String),
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostStatus {
    Ok = 0,
    Error = 1,
}

pub type HostHandle = *mut c_void;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct HostBuffer {
    pub ptr: *mut u8,
    pub len: usize,
}

impl HostBuffer {
    pub const fn empty() -> Self {
        Self {
            ptr: std::ptr::null_mut(),
            len: 0,
        }
    }
}

pub type HostApi = unsafe extern "C" fn(
    ctx: HostHandle,
    input_ptr: *const u8,
    input_len: usize,
    output: *mut HostBuffer,
) -> HostStatus;

pub type HostFree = unsafe extern "C" fn(ctx: HostHandle, buffer: HostBuffer);

#[repr(C)]
#[derive(Clone, Copy)]
pub struct HostFunctionTable {
    pub install_runtime: HostApi,
    pub eval_js: HostApi,
    pub send_batch: HostApi,
    pub snapshot_dom: HostApi,
    pub inject_session: HostApi,
    pub snapshot_session: HostApi,
    pub drain_events: HostApi,
    pub navigate: HostApi,
    pub free_buffer: HostFree,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BridgeEventEnvelope {
    pub event: RuntimeEvent,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BatchRequest {
    pub batch_id: u64,
    pub commands: Vec<Command>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BatchResponse {
    pub batch_id: u64,
    pub results: Vec<CommandResult>,
    pub snapshot: DomSnapshot,
    #[serde(default)]
    pub events: Vec<BridgeEventEnvelope>,
}

enum BridgeBackend {
    Dynamic {
        host: NonNull<c_void>,
        fns: HostFunctionTable,
    },
    #[cfg(target_os = "macos")]
    NativeApp(NativeAppBridge),
}

#[cfg(target_os = "macos")]
struct NativeAppBridge {
    app_executable: PathBuf,
    browser_config: BrowserConfig,
    last_url: Option<String>,
    session: SessionState,
}

pub struct CefBridge {
    backend: BridgeBackend,
    runtime_script_path: PathBuf,
}

impl CefBridge {
    pub fn new(host: HostHandle, fns: HostFunctionTable) -> Result<Self, AegisError> {
        let host =
            NonNull::new(host).ok_or_else(|| AegisError::Bridge("host handle is null".into()))?;
        Ok(Self {
            backend: BridgeBackend::Dynamic { host, fns },
            runtime_script_path: PathBuf::from("assets/js/aegis_runtime.js"),
        })
    }

    #[cfg(target_os = "macos")]
    pub fn new_native_app(
        workspace_root: impl AsRef<Path>,
        browser_config: BrowserConfig,
    ) -> Result<Self, AegisError> {
        let workspace_root = workspace_root.as_ref();
        let app_bundle = workspace_root.join(DEFAULT_APP_BUNDLE_PATH);
        let app_executable = if crate::native::bundle_executable(&app_bundle).exists() {
            crate::native::bundle_executable(&app_bundle)
        } else {
            let current_exe = std::env::current_exe()?;
            prepare_bundled_cli(workspace_root, current_exe)?;
            crate::native::bundle_executable(&app_bundle)
        };

        Ok(Self {
            backend: BridgeBackend::NativeApp(NativeAppBridge {
                app_executable,
                browser_config,
                last_url: None,
                session: SessionState::default(),
            }),
            runtime_script_path: PathBuf::from("assets/js/aegis_runtime.js"),
        })
    }

    pub fn install_runtime(&mut self) -> Result<(), AegisError> {
        #[cfg(target_os = "macos")]
        if matches!(self.backend, BridgeBackend::NativeApp(_)) {
            return Ok(());
        }
        let script = fs::read_to_string(&self.runtime_script_path)?;
        let payload = encode_message(MessageKind::InstallRuntime, &script)?;
        let _response = self.invoke_message(MessageKind::InstallRuntime, &payload)?;
        Ok(())
    }

    pub fn eval_js(&mut self, script: &str) -> Result<String, AegisError> {
        let payload = encode_message(
            MessageKind::EvalJs,
            &EvalJsRequest {
                script: script.into(),
            },
        )?;
        let response = self.invoke_message(MessageKind::EvalJs, &payload)?;
        let response: EvalJsResponse = decode_message(MessageKind::EvalJs, &response)?;
        String::from_utf8(response.value).map_err(|error| AegisError::Bridge(error.to_string()))
    }

    pub fn send_batch(&mut self, request: &BatchRequest) -> Result<BatchResponse, AegisError> {
        let payload = encode_message(MessageKind::SendBatch, request)?;
        let response = self.invoke_message(MessageKind::SendBatch, &payload)?;
        let response: BatchWireResponse = decode_message(MessageKind::SendBatch, &response)?;
        Ok(BatchResponse {
            batch_id: response.batch_id,
            results: response.results,
            snapshot: response.snapshot,
            events: response.events,
        })
    }

    pub fn snapshot_dom(&mut self) -> Result<DomSnapshot, AegisError> {
        let payload = encode_message(MessageKind::SnapshotDom, &())?;
        let response = self.invoke_message(MessageKind::SnapshotDom, &payload)?;
        decode_message(MessageKind::SnapshotDom, &response)
    }

    pub fn inject_session(&mut self, session: SessionState) -> Result<(), AegisError> {
        let payload = encode_message(MessageKind::InjectSession, &session)?;
        #[cfg(target_os = "macos")]
        if let BridgeBackend::NativeApp(bridge) = &mut self.backend {
            bridge.session = session.clone();
        }
        let _response = self.invoke_message(MessageKind::InjectSession, &payload)?;
        Ok(())
    }

    pub fn snapshot_session(&mut self) -> Result<SessionState, AegisError> {
        let payload = encode_message(MessageKind::SnapshotSession, &())?;
        let response = self.invoke_message(MessageKind::SnapshotSession, &payload)?;
        decode_message(MessageKind::SnapshotSession, &response)
    }

    pub fn drain_events(&mut self) -> Result<Vec<BridgeEventEnvelope>, AegisError> {
        let payload = encode_message(MessageKind::DrainEvents, &())?;
        let response = self.invoke_message(MessageKind::DrainEvents, &payload)?;
        if response.is_empty() {
            return Ok(Vec::new());
        }
        let response: EventsResponse = decode_message(MessageKind::DrainEvents, &response)?;
        Ok(response.events)
    }

    pub fn navigate(&mut self, url: &str) -> Result<BatchResponse, AegisError> {
        let payload = encode_message(MessageKind::Navigate, &NavigateRequest { url: url.into() })?;
        #[cfg(target_os = "macos")]
        if let BridgeBackend::NativeApp(bridge) = &mut self.backend {
            bridge.last_url = Some(url.to_string());
        }
        let response = self.invoke_message(MessageKind::Navigate, &payload)?;
        let response: NavigateResponse = decode_message(MessageKind::Navigate, &response)?;
        Ok(BatchResponse {
            batch_id: 0,
            results: Vec::new(),
            snapshot: response.snapshot,
            events: response.events,
        })
    }

    fn invoke_message(&mut self, kind: MessageKind, input: &[u8]) -> Result<Vec<u8>, AegisError> {
        match &mut self.backend {
            BridgeBackend::Dynamic { host, fns } => {
                let function = match kind {
                    MessageKind::InstallRuntime => fns.install_runtime,
                    MessageKind::EvalJs => fns.eval_js,
                    MessageKind::SendBatch => fns.send_batch,
                    MessageKind::SnapshotDom => fns.snapshot_dom,
                    MessageKind::InjectSession => fns.inject_session,
                    MessageKind::SnapshotSession => fns.snapshot_session,
                    MessageKind::DrainEvents => fns.drain_events,
                    MessageKind::Navigate => fns.navigate,
                };
                Self::invoke_raw(*host, *fns, function, input)
            }
            #[cfg(target_os = "macos")]
            BridgeBackend::NativeApp(bridge) => bridge.invoke(kind, input),
        }
    }

    fn invoke_raw(
        host: NonNull<c_void>,
        fns: HostFunctionTable,
        function: HostApi,
        input: &[u8],
    ) -> Result<Vec<u8>, AegisError> {
        let mut output = HostBuffer::empty();
        let status = unsafe {
            function(
                host.as_ptr(),
                input.as_ptr(),
                input.len(),
                &mut output as *mut HostBuffer,
            )
        };

        match status {
            HostStatus::Ok => Self::decode_buffer(host, fns, output),
            HostStatus::Error => {
                let message = Self::decode_buffer(host, fns, output)?;
                let message = String::from_utf8_lossy(&message).to_string();
                Err(AegisError::Bridge(message))
            }
        }
    }

    fn decode_buffer(
        host: NonNull<c_void>,
        fns: HostFunctionTable,
        buffer: HostBuffer,
    ) -> Result<Vec<u8>, AegisError> {
        if buffer.ptr.is_null() || buffer.len == 0 {
            return Ok(Vec::new());
        }

        let output = unsafe { slice::from_raw_parts(buffer.ptr.cast_const(), buffer.len) }.to_vec();
        unsafe {
            (fns.free_buffer)(host.as_ptr(), buffer);
        }
        Ok(output)
    }
}

#[cfg(target_os = "macos")]
impl NativeAppBridge {
    fn invoke(&mut self, kind: MessageKind, input: &[u8]) -> Result<Vec<u8>, AegisError> {
        let temp = std::env::temp_dir().join(format!(
            "aegis-app-{}-{}-{}",
            std::process::id(),
            kind as u16,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|error| AegisError::Bridge(error.to_string()))?
                .as_nanos()
        ));
        fs::create_dir_all(&temp)?;
        let config_path = temp.join("config.json");
        let request_path = temp.join("request.bin");
        let response_path = temp.join("response.bin");
        let error_path = temp.join("error.txt");
        let debug_log_path = temp.join("debug.log");

        let mut config = self.browser_config.clone();
        if config.start_url.is_none() {
            config.start_url = self.last_url.clone();
        }
        if config.user_data_dir.is_none() {
            config.user_data_dir = Some(temp.join("user-data").to_string_lossy().into_owned());
        }
        fs::write(&config_path, serde_json::to_vec(&config).map_err(AegisError::Serialize)?)?;
        fs::write(&request_path, input)?;

        let output = ProcessCommand::new(&self.app_executable)
            .current_dir(std::env::current_dir()?)
            .arg("--mode")
            .arg(match self.browser_config.mode {
                crate::browser::BrowserMode::Headless => "headless",
                crate::browser::BrowserMode::Headful => "headful",
            })
            .arg("--aegis-config")
            .arg(&config_path)
            .arg("--aegis-request")
            .arg(&request_path)
            .arg("--aegis-response")
            .arg(&response_path)
            .arg("--aegis-error")
            .arg(&error_path)
            .arg("--aegis-debug-log")
            .arg(&debug_log_path)
            .arg("--aegis-op")
            .arg((kind as u16).to_string())
            .output()?;

        if !output.status.success() {
            let detail = fs::read_to_string(&error_path)
                .unwrap_or_else(|_| String::from_utf8_lossy(&output.stderr).to_string());
            return Err(AegisError::Bridge(detail));
        }

        let response = fs::read(&response_path)?;
        if matches!(kind, MessageKind::SnapshotSession) {
            if let Ok(session) = decode_message::<SessionState>(MessageKind::SnapshotSession, &response)
            {
                self.session = session;
            }
        }
        Ok(response)
    }
}
