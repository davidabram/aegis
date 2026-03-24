use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;

const STATE_VERSION: u32 = 1;
const DEFAULT_PROFILE: &str = "default";

#[derive(Debug, Clone)]
pub struct AegisStatePaths {
    root: PathBuf,
}

impl AegisStatePaths {
    pub fn detect() -> Result<Self, String> {
        let root = if let Some(root) = std::env::var_os("AEGIS_HOME") {
            PathBuf::from(root)
        } else if let Some(home) = std::env::var_os("HOME") {
            Path::new(&home).join(".aegis")
        } else {
            return Err("HOME is not set".into());
        };

        let paths = Self { root };
        paths.ensure_layout()?;
        Ok(paths)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn settings_dir(&self) -> PathBuf {
        self.root.join("settings")
    }

    pub fn settings_file(&self, concern: &str) -> PathBuf {
        self.settings_dir().join(format!("{concern}.json"))
    }

    pub fn secrets_dir(&self) -> PathBuf {
        self.root.join("secrets")
    }

    pub fn profile_secret_dir(&self, profile: &str) -> PathBuf {
        self.secrets_dir().join("profiles").join(profile)
    }

    pub fn profile_secrets_file(&self, profile: &str) -> PathBuf {
        self.profile_secret_dir(profile).join("secrets.json")
    }

    pub fn profiles_dir(&self) -> PathBuf {
        self.root.join("profiles")
    }

    pub fn profile_dir(&self, profile: &str) -> PathBuf {
        self.profiles_dir().join(profile)
    }

    pub fn session_file(&self, profile: &str) -> PathBuf {
        self.profile_dir(profile).join("session.json")
    }

    pub fn runtime_dir(&self) -> PathBuf {
        self.root.join("runtime")
    }

    pub fn runtime_scope_dir(&self, scope: &str) -> PathBuf {
        self.runtime_dir().join(scope)
    }

    pub fn runtime_instances_dir(&self, scope: &str) -> PathBuf {
        self.runtime_scope_dir(scope).join("instances")
    }

    pub fn ensure_profile_layout(&self, profile: &str) -> Result<(), String> {
        validate_name(profile, "profile")?;
        self.ensure_dir(&self.profile_dir(profile))?;
        self.ensure_dir(&self.profile_secret_dir(profile))?;
        self.ensure_json_file(
            &self.session_file(profile),
            &default_session_payload(),
            "session profile",
        )?;
        self.ensure_json_file(
            &self.profile_secrets_file(profile),
            &default_secrets_payload(),
            "profile secrets",
        )?;
        Ok(())
    }

    fn ensure_layout(&self) -> Result<(), String> {
        self.ensure_dir(&self.root)?;
        self.ensure_dir(&self.settings_dir())?;
        self.ensure_dir(&self.profiles_dir())?;
        self.ensure_dir(&self.secrets_dir())?;
        self.ensure_dir(&self.secrets_dir().join("profiles"))?;
        self.ensure_dir(&self.runtime_instances_dir("serve-headless"))?;
        self.ensure_dir(&self.runtime_instances_dir("serve-headful"))?;

        self.remove_obsolete_path(&self.root.join("imports"))?;
        self.remove_obsolete_path(&self.root.join("exports"))?;
        self.remove_obsolete_profile_secret_files()?;

        self.ensure_canonical_json_file(
            &self.settings_file("agent"),
            &default_agent_settings_payload(),
            "agent settings",
            normalize_agent_settings,
        )?;
        self.ensure_canonical_json_file(
            &self.settings_file("runtime"),
            &default_runtime_settings_payload(),
            "runtime settings",
            normalize_runtime_settings,
        )?;
        self.ensure_canonical_json_file(
            &self.settings_file("credentials"),
            &default_credentials_settings_payload(),
            "credentials settings",
            normalize_credentials_settings,
        )?;
        self.ensure_profile_layout(DEFAULT_PROFILE)?;
        Ok(())
    }

    fn ensure_dir(&self, path: &Path) -> Result<(), String> {
        fs::create_dir_all(path)
            .map_err(|error| format!("failed to create directory {}: {error}", path.display()))
    }

    fn ensure_json_file(
        &self,
        path: &Path,
        default_value: &serde_json::Value,
        label: &str,
    ) -> Result<(), String> {
        let parent = path
            .parent()
            .ok_or_else(|| format!("invalid {} path {}", label, path.display()))?;
        self.ensure_dir(parent)?;

        let default_bytes = serde_json::to_vec_pretty(default_value)
            .map_err(|error| format!("failed to encode default {label}: {error}"))?;

        match fs::read(path) {
            Ok(bytes) => {
                if serde_json::from_slice::<serde_json::Value>(&bytes).is_ok() {
                    secure_state_file_if_needed(path)?;
                    return Ok(());
                }
                self.replace_corrupt_file(path, &default_bytes, label)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                write_state_file(path, &default_bytes)?;
            }
            Err(error) => {
                return Err(format!(
                    "failed to read {} {}: {error}",
                    label,
                    path.display()
                ));
            }
        }
        Ok(())
    }

    fn ensure_canonical_json_file(
        &self,
        path: &Path,
        default_value: &serde_json::Value,
        label: &str,
        normalize: fn(serde_json::Value) -> serde_json::Value,
    ) -> Result<(), String> {
        let parent = path
            .parent()
            .ok_or_else(|| format!("invalid {} path {}", label, path.display()))?;
        self.ensure_dir(parent)?;

        match fs::read(path) {
            Ok(bytes) => {
                let parsed = match serde_json::from_slice::<serde_json::Value>(&bytes) {
                    Ok(value) => value,
                    Err(_) => {
                        self.repair_json_file(path, default_value, label)?;
                        return Ok(());
                    }
                };
                let normalized = normalize(parsed);
                if normalized != default_or_existing(path)? {
                    let payload = serde_json::to_vec_pretty(&normalized)
                        .map_err(|error| format!("failed to encode normalized {label}: {error}"))?;
                    write_state_file(path, &payload)?;
                } else {
                    secure_state_file_if_needed(path)?;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.ensure_json_file(path, default_value, label)?;
            }
            Err(error) => {
                return Err(format!(
                    "failed to read {} {}: {error}",
                    label,
                    path.display()
                ));
            }
        }
        Ok(())
    }

    pub fn repair_json_file(
        &self,
        path: &Path,
        default_value: &serde_json::Value,
        label: &str,
    ) -> Result<(), String> {
        let default_bytes = serde_json::to_vec_pretty(default_value)
            .map_err(|error| format!("failed to encode default {label}: {error}"))?;
        self.replace_corrupt_file(path, &default_bytes, label)
    }

    fn replace_corrupt_file(
        &self,
        path: &Path,
        replacement: &[u8],
        label: &str,
    ) -> Result<(), String> {
        let backup = corrupt_backup_path(path);
        if path.exists() {
            fs::rename(path, &backup).map_err(|error| {
                format!(
                    "failed to quarantine corrupt {} {} to {}: {error}",
                    label,
                    path.display(),
                    backup.display()
                )
            })?;
        }
        write_state_file(path, replacement)
            .map_err(|error| format!("failed to rewrite {} {}: {error}", label, path.display()))
    }

    fn remove_obsolete_path(&self, path: &Path) -> Result<(), String> {
        if !path.exists() {
            return Ok(());
        }
        if path.is_dir() {
            fs::remove_dir_all(path).map_err(|error| {
                format!(
                    "failed to remove obsolete directory {}: {error}",
                    path.display()
                )
            })?;
        } else {
            fs::remove_file(path).map_err(|error| {
                format!("failed to remove obsolete file {}: {error}", path.display())
            })?;
        }
        Ok(())
    }

    fn remove_obsolete_profile_secret_files(&self) -> Result<(), String> {
        let profiles_dir = self.secrets_dir().join("profiles");
        if !profiles_dir.exists() {
            return Ok(());
        }
        for entry in fs::read_dir(&profiles_dir)
            .map_err(|error| format!("failed to read {}: {error}", profiles_dir.display()))?
        {
            let entry =
                entry.map_err(|error| format!("failed to read profile secret entry: {error}"))?;
            let obsolete = entry.path().join("credentials.json");
            if obsolete.exists() {
                fs::remove_file(&obsolete).map_err(|error| {
                    format!(
                        "failed to remove obsolete secret file {}: {error}",
                        obsolete.display()
                    )
                })?;
            }
        }
        Ok(())
    }
}

fn default_agent_settings_payload() -> serde_json::Value {
    json!({
        "version": STATE_VERSION,
        "default_profile": DEFAULT_PROFILE
    })
}

fn normalize_agent_settings(value: serde_json::Value) -> serde_json::Value {
    let default_profile = value
        .get("default_profile")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(DEFAULT_PROFILE);
    json!({
        "version": STATE_VERSION,
        "default_profile": default_profile
    })
}

fn normalize_runtime_settings(value: serde_json::Value) -> serde_json::Value {
    let bootstrap_page = value
        .get("bootstrap_page")
        .and_then(|value| value.as_str())
        .unwrap_or("local");
    json!({
        "version": STATE_VERSION,
        "bootstrap_page": bootstrap_page,
        "modes": {
            "headless": {
                "persistent": true
            },
            "headful": {
                "persistent": true
            }
        }
    })
}

fn default_runtime_settings_payload() -> serde_json::Value {
    json!({
        "version": STATE_VERSION,
        "bootstrap_page": "local",
        "modes": {
            "headless": {
                "persistent": true
            },
            "headful": {
                "persistent": true
            }
        }
    })
}

fn normalize_credentials_settings(value: serde_json::Value) -> serde_json::Value {
    json!({
        "version": STATE_VERSION,
        "auto_store": value
            .get("auto_store")
            .and_then(|value| value.as_bool())
            .unwrap_or(true)
    })
}

fn default_credentials_settings_payload() -> serde_json::Value {
    json!({
        "version": STATE_VERSION,
        "auto_store": true
    })
}

fn default_session_payload() -> serde_json::Value {
    json!({
        "version": STATE_VERSION,
        "session": {
            "cookies": [],
            "local_storage": {},
            "session_storage": {},
            "network_overrides": []
        }
    })
}

fn default_secrets_payload() -> serde_json::Value {
    json!({
        "version": STATE_VERSION,
        "secrets": {
            "credentials": {
                "version": STATE_VERSION,
                "entries": []
            }
        }
    })
}

fn corrupt_backup_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| "state".to_string());
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    path.with_file_name(format!("{file_name}.corrupt.{timestamp}"))
}

fn default_or_existing(path: &Path) -> Result<serde_json::Value, String> {
    let bytes = fs::read(path)
        .map_err(|error| format!("failed to read json file {}: {error}", path.display()))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("failed to parse json file {}: {error}", path.display()))
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

pub fn with_state_file_lock<T>(
    path: &Path,
    action: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let _guard = StateFileLock::acquire(path)?;
    action()
}

pub fn replace_corrupt_state_file(path: &Path, replacement: &[u8], label: &str) -> Result<(), String> {
    let backup = corrupt_backup_path(path);
    if path.exists() {
        fs::rename(path, &backup).map_err(|error| {
            format!(
                "failed to quarantine corrupt {} {} to {}: {error}",
                label,
                path.display(),
                backup.display()
            )
        })?;
    }
    write_state_file(path, replacement)
}

pub fn write_state_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("invalid state path {}", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("failed to create directory {}: {error}", parent.display()))?;

    let temp_path = temporary_write_path(path);
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&temp_path)
        .map_err(|error| format!("failed to open temp state file {}: {error}", temp_path.display()))?;

    if let Err(error) = file.write_all(bytes) {
        let _ = fs::remove_file(&temp_path);
        return Err(format!(
            "failed to write temp state file {}: {error}",
            temp_path.display()
        ));
    }
    if let Err(error) = file.sync_all() {
        let _ = fs::remove_file(&temp_path);
        return Err(format!(
            "failed to sync temp state file {}: {error}",
            temp_path.display()
        ));
    }
    if let Err(error) = secure_state_file_if_needed(&temp_path) {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }

    if let Err(error) = fs::rename(&temp_path, path) {
        let _ = fs::remove_file(&temp_path);
        return Err(format!(
            "failed to atomically replace state file {}: {error}",
            path.display()
        ));
    }

    secure_state_file_if_needed(path)?;
    sync_directory(parent)?;
    Ok(())
}

fn secure_state_file_if_needed(path: &Path) -> Result<(), String> {
    set_owner_only_permissions(path)
}

fn temporary_write_path(path: &Path) -> PathBuf {
    let pid = std::process::id();
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_micros())
        .unwrap_or(0);
    let file_name = path
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| "state".to_string());
    path.with_file_name(format!("{file_name}.tmp.{pid}.{timestamp}"))
}

fn lock_file_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| "state".to_string());
    path.with_file_name(format!("{file_name}.lock"))
}

struct StateFileLock {
    file: File,
}

impl StateFileLock {
    fn acquire(path: &Path) -> Result<Self, String> {
        let lock_path = lock_file_path(path);
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|error| format!("failed to open state lock {}: {error}", lock_path.display()))?;
        lock_file_exclusive(&file, &lock_path)?;
        Ok(Self { file })
    }
}

impl Drop for StateFileLock {
    fn drop(&mut self) {
        let _ = unlock_file(&self.file);
    }
}

#[cfg(unix)]
fn lock_file_exclusive(file: &File, path: &Path) -> Result<(), String> {
    use std::os::fd::AsRawFd;

    let status = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
    if status == 0 {
        Ok(())
    } else {
        Err(format!(
            "failed to lock state file {}: {}",
            path.display(),
            std::io::Error::last_os_error()
        ))
    }
}

#[cfg(not(unix))]
fn lock_file_exclusive(_file: &File, _path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
fn unlock_file(file: &File) -> Result<(), String> {
    use std::os::fd::AsRawFd;

    let status = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
    if status == 0 {
        Ok(())
    } else {
        Err(format!(
            "failed to unlock state file: {}",
            std::io::Error::last_os_error()
        ))
    }
}

#[cfg(not(unix))]
fn unlock_file(_file: &File) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), String> {
    let directory = File::open(path)
        .map_err(|error| format!("failed to open state directory {}: {error}", path.display()))?;
    directory
        .sync_all()
        .map_err(|error| format!("failed to sync state directory {}: {error}", path.display()))
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
fn set_owner_only_permissions(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let permissions = fs::Permissions::from_mode(0o600);
    fs::set_permissions(path, permissions)
        .map_err(|error| format!("failed to secure secret file {}: {error}", path.display()))
}

#[cfg(not(unix))]
fn set_owner_only_permissions(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
use std::sync::{Mutex, OnceLock};

#[cfg(test)]
pub(crate) fn aegis_test_env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[cfg(test)]
mod tests {
    use super::{AegisStatePaths, aegis_test_env_lock};
    use std::fs;

    #[test]
    fn detect_bootstraps_canonical_layout() {
        let _guard = aegis_test_env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let temp = tempfile::tempdir().expect("temporary state dir should be created");
        unsafe {
            std::env::set_var("AEGIS_HOME", temp.path());
        }
        let paths = AegisStatePaths::detect().expect("state layout should bootstrap");
        assert!(paths.settings_file("agent").exists());
        assert!(paths.settings_file("runtime").exists());
        assert!(paths.settings_file("credentials").exists());
        assert!(paths.session_file("default").exists());
        assert!(paths.profile_secrets_file("default").exists());
        assert!(paths.runtime_instances_dir("serve-headless").exists());
        assert!(paths.runtime_instances_dir("serve-headful").exists());
        unsafe {
            std::env::remove_var("AEGIS_HOME");
        }
    }

    #[test]
    fn detect_replaces_corrupt_defaults_and_removes_obsolete_dirs() {
        let _guard = aegis_test_env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let temp = tempfile::tempdir().expect("temporary state dir should be created");
        unsafe {
            std::env::set_var("AEGIS_HOME", temp.path());
        }
        fs::create_dir_all(temp.path().join("settings"))
            .expect("settings directory should be created");
        fs::write(temp.path().join("settings/agent.json"), b"{not-json")
            .expect("corrupt settings fixture should be written");
        fs::create_dir_all(temp.path().join("imports/old"))
            .expect("obsolete directory should be created");
        let paths = AegisStatePaths::detect().expect("state layout should repair corrupt config");
        let agent = fs::read_to_string(paths.settings_file("agent"))
            .expect("normalized agent config should be readable");
        assert!(agent.contains("\"default_profile\""));
        let backups = fs::read_dir(temp.path().join("settings"))
            .expect("settings directory should be readable")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with("agent.json.corrupt."))
            .count();
        assert_eq!(backups, 1);
        assert!(!temp.path().join("imports").exists());
        unsafe {
            std::env::remove_var("AEGIS_HOME");
        }
    }

    #[test]
    fn detect_normalizes_agent_settings_and_removes_obsolete_secret_files() {
        let _guard = aegis_test_env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let temp = tempfile::tempdir().expect("temporary state dir should be created");
        unsafe {
            std::env::set_var("AEGIS_HOME", temp.path());
        }
        let settings_dir = temp.path().join("settings");
        fs::create_dir_all(&settings_dir).expect("settings directory should be created");
        fs::write(
            settings_dir.join("agent.json"),
            br#"{"version":1,"default_profile":"work","browser_import":"brave"}"#,
        )
        .expect("agent fixture should be written");
        let obsolete_dir = temp.path().join("secrets/profiles/work");
        fs::create_dir_all(&obsolete_dir).expect("obsolete secret dir should be created");
        fs::write(obsolete_dir.join("credentials.json"), b"legacy")
            .expect("obsolete secret fixture should be written");
        let paths = AegisStatePaths::detect().expect("state layout should normalize config");
        let agent = fs::read_to_string(paths.settings_file("agent"))
            .expect("agent config should be readable");
        let credentials = fs::read_to_string(paths.settings_file("credentials"))
            .expect("credentials config should be readable");
        assert!(agent.contains("\"default_profile\": \"work\""));
        assert!(!agent.contains("browser_import"));
        assert!(credentials.contains("\"auto_store\": true"));
        assert!(!obsolete_dir.join("credentials.json").exists());
        unsafe {
            std::env::remove_var("AEGIS_HOME");
        }
    }

    #[cfg(unix)]
    #[test]
    fn detect_secures_existing_state_files() {
        use std::os::unix::fs::PermissionsExt;

        let _guard = aegis_test_env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let temp = tempfile::tempdir().expect("temporary state dir should be created");
        unsafe {
            std::env::set_var("AEGIS_HOME", temp.path());
        }
        let paths = AegisStatePaths::detect().expect("state layout should bootstrap");
        fs::set_permissions(paths.session_file("default"), fs::Permissions::from_mode(0o644))
            .expect("session permissions should be changed for test");
        fs::set_permissions(paths.settings_file("agent"), fs::Permissions::from_mode(0o644))
            .expect("settings permissions should be changed for test");

        let paths = AegisStatePaths::detect().expect("state layout should re-secure files");
        let session_mode = fs::metadata(paths.session_file("default"))
            .expect("session metadata should be readable")
            .permissions()
            .mode()
            & 0o777;
        let agent_mode = fs::metadata(paths.settings_file("agent"))
            .expect("agent metadata should be readable")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(session_mode, 0o600);
        assert_eq!(agent_mode, 0o600);

        unsafe {
            std::env::remove_var("AEGIS_HOME");
        }
    }
}
