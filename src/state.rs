use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct AegisStatePaths {
    root: PathBuf,
}

impl AegisStatePaths {
    pub fn detect() -> Result<Self, String> {
        if let Some(root) = std::env::var_os("AEGIS_HOME") {
            return Ok(Self {
                root: PathBuf::from(root),
            });
        }
        if let Some(home) = std::env::var_os("HOME") {
            return Ok(Self {
                root: Path::new(&home).join(".aegis"),
            });
        }
        Err("HOME is not set".into())
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

    pub fn profiles_dir(&self) -> PathBuf {
        self.root.join("profiles")
    }

    pub fn profile_dir(&self, profile: &str) -> PathBuf {
        self.profiles_dir().join(profile)
    }

    pub fn session_file(&self, profile: &str) -> PathBuf {
        self.profile_dir(profile).join("session.json")
    }
}
