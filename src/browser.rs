use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BrowserMode {
    #[default]
    Headless,
    Headful,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserConfig {
    pub mode: BrowserMode,
    #[serde(default)]
    pub start_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_dir: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upload_dir: Option<PathBuf>,
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            mode: BrowserMode::Headless,
            start_url: None,
            download_dir: None,
            upload_dir: None,
        }
    }
}
