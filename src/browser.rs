use serde::{Deserialize, Serialize};

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
    #[serde(default)]
    pub user_data_dir: Option<String>,
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            mode: BrowserMode::Headless,
            start_url: None,
            user_data_dir: None,
        }
    }
}
