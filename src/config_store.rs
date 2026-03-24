use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::browser_import::ImportedCredential;
use crate::state::AegisStatePaths;

#[derive(Debug, Clone)]
pub struct AegisConfigStore {
    paths: AegisStatePaths,
}

impl AegisConfigStore {
    pub fn detect() -> Result<Self, String> {
        Ok(Self {
            paths: AegisStatePaths::detect()?,
        })
    }

    pub fn get(&self, concern: &str) -> Result<Option<Value>, String> {
        validate_name(concern, "name")?;
        let path = self.paths.settings_file(concern);
        if !path.exists() {
            return Ok(None);
        }
        let bytes = fs::read(&path)
            .map_err(|error| format!("failed to read config {}: {error}", path.display()))?;
        let value = serde_json::from_slice(&bytes)
            .map_err(|error| format!("failed to parse config {}: {error}", path.display()))?;
        Ok(Some(value))
    }

    pub fn set(&self, concern: &str, value: &Value) -> Result<PathBuf, String> {
        validate_name(concern, "name")?;
        let path = self.paths.settings_file(concern);
        let parent = path
            .parent()
            .ok_or_else(|| format!("invalid config path {}", path.display()))?;
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed to create config directory {}: {error}",
                parent.display()
            )
        })?;
        let payload = serde_json::to_vec_pretty(value)
            .map_err(|error| format!("failed to encode config {concern}: {error}"))?;
        fs::write(&path, payload)
            .map_err(|error| format!("failed to write config {}: {error}", path.display()))?;
        Ok(path)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredProfileCredentials {
    version: u32,
    credentials: Vec<ImportedCredential>,
}

#[derive(Debug, Clone)]
pub struct AegisSecretStore {
    paths: AegisStatePaths,
}

impl AegisSecretStore {
    pub fn detect() -> Result<Self, String> {
        Ok(Self {
            paths: AegisStatePaths::detect()?,
        })
    }

    pub fn load_profile_credentials(
        &self,
        profile: &str,
    ) -> Result<Vec<ImportedCredential>, String> {
        validate_name(profile, "profile")?;
        let path = self.paths.profile_credentials_file(profile);
        if !path.exists() {
            return Ok(Vec::new());
        }
        let bytes = fs::read(&path).map_err(|error| {
            format!("failed to read credential file {}: {error}", path.display())
        })?;
        let stored: StoredProfileCredentials = serde_json::from_slice(&bytes).map_err(|error| {
            format!(
                "failed to parse credential file {}: {error}",
                path.display()
            )
        })?;
        Ok(stored.credentials)
    }

    pub fn save_profile_credentials(
        &self,
        profile: &str,
        credentials: &[ImportedCredential],
    ) -> Result<PathBuf, String> {
        validate_name(profile, "profile")?;
        let path = self.paths.profile_credentials_file(profile);
        let parent = path
            .parent()
            .ok_or_else(|| format!("invalid credential path {}", path.display()))?;
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed to create credential directory {}: {error}",
                parent.display()
            )
        })?;
        let payload = serde_json::to_vec_pretty(&StoredProfileCredentials {
            version: 1,
            credentials: credentials.to_vec(),
        })
        .map_err(|error| format!("failed to encode credential payload: {error}"))?;
        fs::write(&path, payload).map_err(|error| {
            format!(
                "failed to write credential file {}: {error}",
                path.display()
            )
        })?;
        set_owner_only_permissions(&path)?;
        Ok(path)
    }
}

fn validate_name(value: &str, label: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("{label} must not be empty"));
    }
    if !value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return Err(format!(
            "{label} {value:?} must use only letters, numbers, '.', '-', or '_'"
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn set_owner_only_permissions(path: &std::path::Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let permissions = fs::Permissions::from_mode(0o600);
    fs::set_permissions(path, permissions)
        .map_err(|error| format!("failed to secure secret file {}: {error}", path.display()))
}

#[cfg(not(unix))]
fn set_owner_only_permissions(_path: &std::path::Path) -> Result<(), String> {
    Ok(())
}
