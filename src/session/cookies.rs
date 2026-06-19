use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::session::storage::{NetworkOverride, StorageArea};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CookieSameSite {
    Unspecified,
    None,
    Lax,
    Strict,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_unix: Option<u64>,
    #[serde(default)]
    pub secure: bool,
    #[serde(default)]
    pub http_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub same_site: Option<CookieSameSite>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SessionState {
    #[serde(default)]
    pub cookies: Vec<Cookie>,
    #[serde(default)]
    pub local_storage: HashMap<String, String>,
    #[serde(default)]
    pub session_storage: HashMap<String, String>,
    #[serde(default)]
    pub network_overrides: Vec<NetworkOverride>,
}

impl SessionState {
    pub fn with_storage(
        mut self,
        area: StorageArea,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        match area {
            StorageArea::Local => {
                self.local_storage.insert(key.into(), value.into());
            }
            StorageArea::Session => {
                self.session_storage.insert(key.into(), value.into());
            }
        }
        self
    }

    pub fn validate(&self) -> Result<(), String> {
        for cookie in &self.cookies {
            if cookie.name.trim().is_empty() {
                return Err("cookie name must not be empty".into());
            }
            if cookie.domain.trim().is_empty() {
                return Err(format!("cookie {} must include a domain", cookie.name));
            }
        }

        for override_ in &self.network_overrides {
            if override_.header.trim().is_empty() {
                return Err("network override header must not be empty".into());
            }
        }

        Ok(())
    }
}
