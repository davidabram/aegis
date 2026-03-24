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
        let stored: StoredSessionProfile = match serde_json::from_slice(&bytes) {
            Ok(stored) => stored,
            Err(_) => {
                let default = default_session_payload();
                AegisStatePaths::detect()?.repair_json_file(
                    &self.info.path,
                    &default,
                    "session profile",
                )?;
                StoredSessionProfile {
                    version: PROFILE_VERSION,
                    session: SessionState::default(),
                }
            }
        };
        if stored.version != PROFILE_VERSION {
            let default = default_session_payload();
            AegisStatePaths::detect()?.repair_json_file(
                &self.info.path,
                &default,
                "session profile",
            )?;
            return Ok(Some(SessionState::default()));
        }
        if stored.session.validate().is_err() {
            let default = default_session_payload();
            AegisStatePaths::detect()?.repair_json_file(
                &self.info.path,
                &default,
                "session profile",
            )?;
            return Ok(Some(SessionState::default()));
        }
        Ok(Some(stored.session))
    }

    pub fn save(&self, session: &SessionState) -> Result<PathBuf, String> {
        session
            .validate()
            .map_err(|error| format!("invalid session profile data: {error}"))?;
        let parent =
            self.info.path.parent().ok_or_else(|| {
                format!("invalid session profile path {}", self.info.path.display())
            })?;
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

fn default_session_payload() -> serde_json::Value {
    serde_json::json!({
        "version": PROFILE_VERSION,
        "session": {
            "cookies": [],
            "local_storage": {},
            "session_storage": {},
            "network_overrides": []
        }
    })
}

#[cfg(test)]
mod tests {
    use super::SessionProfileStore;
    use crate::state::aegis_test_env_lock;
    use std::fs;

    #[test]
    fn load_repairs_corrupt_session_profile() {
        let _guard = aegis_test_env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let temp = tempfile::tempdir().unwrap();
        unsafe {
            std::env::set_var("AEGIS_HOME", temp.path());
        }
        let store = SessionProfileStore::new("default").unwrap();
        fs::write(store.info().path.clone(), b"{bad-json").unwrap();
        let session = store.load().unwrap().unwrap();
        assert!(session.cookies.is_empty());
        let backups = fs::read_dir(store.info().path.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with("session.json.corrupt."))
            .count();
        assert_eq!(backups, 1);
        unsafe {
            std::env::remove_var("AEGIS_HOME");
        }
    }
}
