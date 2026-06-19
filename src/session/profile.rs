use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::session::cookies::SessionState;
use crate::state::{
    AegisStatePaths, replace_corrupt_state_file, with_state_file_lock, write_state_file,
};

const PROFILE_VERSION: u32 = 2;

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
        with_state_file_lock(&self.info.path, || {
            let bytes = std::fs::read(&self.info.path)
                .map_err(|error| format!("failed to read session profile: {error}"))?;
            let stored: StoredSessionProfile = match serde_json::from_slice(&bytes) {
                Ok(stored) => stored,
                Err(_) => {
                    let default = default_session_payload();
                    replace_corrupt_state_file(
                        &self.info.path,
                        &serde_json::to_vec_pretty(&default).map_err(|error| {
                            format!("failed to encode default session profile: {error}")
                        })?,
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
                replace_corrupt_state_file(
                    &self.info.path,
                    &serde_json::to_vec_pretty(&default).map_err(|error| {
                        format!("failed to encode default session profile: {error}")
                    })?,
                    "session profile",
                )?;
                return Ok(Some(SessionState::default()));
            }
            if stored.session.validate().is_err() {
                let default = default_session_payload();
                replace_corrupt_state_file(
                    &self.info.path,
                    &serde_json::to_vec_pretty(&default).map_err(|error| {
                        format!("failed to encode default session profile: {error}")
                    })?,
                    "session profile",
                )?;
                return Ok(Some(SessionState::default()));
            }
            Ok(Some(stored.session))
        })
    }

    pub fn save(&self, session: &SessionState) -> Result<PathBuf, String> {
        session
            .validate()
            .map_err(|error| format!("invalid session profile data: {error}"))?;
        let payload = serde_json::to_vec_pretty(&StoredSessionProfile {
            version: PROFILE_VERSION,
            session: session.clone(),
        })
        .map_err(|error| format!("failed to encode session profile: {error}"))?;
        with_state_file_lock(&self.info.path, || {
            write_state_file(&self.info.path, &payload)
        })?;
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
    let paths = AegisStatePaths::detect()?;
    paths.ensure_profile_layout(profile)?;
    Ok(paths.session_file(profile))
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
        let temp = tempfile::tempdir().expect("temporary state dir should be created");
        unsafe {
            std::env::set_var("AEGIS_HOME", temp.path());
        }
        let store =
            SessionProfileStore::new("default").expect("session profile store should initialize");
        fs::write(store.info().path.clone(), b"{bad-json")
            .expect("corrupt session fixture should be written");
        let session = store
            .load()
            .expect("corrupt session should be repaired")
            .expect("default session should be returned");
        assert!(session.cookies.is_empty());
        let backups = fs::read_dir(
            store
                .info()
                .path
                .parent()
                .expect("session profile should have a parent directory"),
        )
        .expect("session profile directory should be readable")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("session.json.corrupt."))
        .count();
        assert_eq!(backups, 1);
        unsafe {
            std::env::remove_var("AEGIS_HOME");
        }
    }

    #[test]
    fn load_resets_legacy_session_profile_versions() {
        let _guard = aegis_test_env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let temp = tempfile::tempdir().expect("temporary state dir should be created");
        unsafe {
            std::env::set_var("AEGIS_HOME", temp.path());
        }
        let store =
            SessionProfileStore::new("brave-import").expect("session profile store should initialize");
        fs::write(
            store.info().path.clone(),
            br#"{"version":1,"session":{"cookies":[{"name":"shopify","value":"bad","domain":"accounts.shopify.com"}],"local_storage":{},"session_storage":{},"network_overrides":[]}}"#,
        )
        .expect("legacy session fixture should be written");
        let session = store
            .load()
            .expect("legacy session should be reset")
            .expect("default session should be returned");
        assert!(session.cookies.is_empty());
        unsafe {
            std::env::remove_var("AEGIS_HOME");
        }
    }
}
