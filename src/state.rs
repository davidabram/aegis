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

    pub fn imports_dir(&self) -> PathBuf {
        self.root.join("imports")
    }

    pub fn exports_dir(&self) -> PathBuf {
        self.root.join("exports")
    }

    pub fn browser_import_dir(&self, browser: &str, profile: &str) -> PathBuf {
        self.imports_dir().join(browser).join(profile)
    }

    pub fn browser_export_dir(&self, browser: &str, profile: &str) -> PathBuf {
        self.exports_dir().join(browser).join(profile)
    }

    pub fn secrets_dir(&self) -> PathBuf {
        self.root.join("secrets")
    }

    pub fn profile_secret_dir(&self, profile: &str) -> PathBuf {
        self.secrets_dir().join("profiles").join(profile)
    }

    pub fn profile_credentials_file(&self, profile: &str) -> PathBuf {
        self.profile_secret_dir(profile).join("credentials.json")
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
