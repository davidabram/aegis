use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::session::cookies::SessionState;
use crate::state::AegisStatePaths;

const PROFILE_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredSessionProfile {
    version: u32,
    session: SessionState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionProfileInfo {
    pub profile: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct SessionProfileStore {
    info: SessionProfileInfo,
}

impl SessionProfileStore {
    pub fn new(profile: impl Into<String>) -> Result<Self, String> {
        let profile = profile.into();
        validate_profile_name(&profile)?;
        Ok(Self {
            info: SessionProfileInfo {
                path: profile_path(&profile)?,
                profile,
            },
        })
    }

    pub fn info(&self) -> SessionProfileInfo {
        self.info.clone()
    }

    pub fn load(&self) -> Result<Option<SessionState>, String> {
        if !self.info.path.exists() {
            return Ok(None);
        }
        let bytes = fs::read(&self.info.path)
            .map_err(|error| format!("failed to read session profile: {error}"))?;
        let stored: StoredSessionProfile = serde_json::from_slice(&bytes)
            .map_err(|error| format!("failed to parse session profile: {error}"))?;
        if stored.version != PROFILE_VERSION {
            return Err(format!(
                "unsupported session profile version {} at {}",
                stored.version,
                self.info.path.display()
            ));
        }
        stored
            .session
            .validate()
            .map_err(|error| format!("invalid stored session profile: {error}"))?;
        Ok(Some(stored.session))
    }

    pub fn save(&self, session: &SessionState) -> Result<PathBuf, String> {
        session
            .validate()
            .map_err(|error| format!("invalid session profile data: {error}"))?;
        let parent = self
            .info
            .path
            .parent()
            .ok_or_else(|| format!("invalid session profile path {}", self.info.path.display()))?;
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create session profile directory: {error}"))?;
        let payload = serde_json::to_vec_pretty(&StoredSessionProfile {
            version: PROFILE_VERSION,
            session: session.clone(),
        })
        .map_err(|error| format!("failed to encode session profile: {error}"))?;
        fs::write(&self.info.path, payload)
            .map_err(|error| format!("failed to write session profile: {error}"))?;
        Ok(self.info.path.clone())
    }
}

fn validate_profile_name(profile: &str) -> Result<(), String> {
    if profile.trim().is_empty() {
        return Err("profile name must not be empty".into());
    }
    if !profile
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return Err(format!(
            "profile name {profile:?} must use only letters, numbers, '.', '-', or '_'"
        ));
    }
    Ok(())
}

fn profile_path(profile: &str) -> Result<PathBuf, String> {
    Ok(AegisStatePaths::detect()?.session_file(profile))
}
