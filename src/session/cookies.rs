use std::collections::HashMap;
use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

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

    pub fn normalized(&self) -> Self {
        let now_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let mut seen = HashSet::new();
        let mut cookies = Vec::new();
        for cookie in self.cookies.iter().rev() {
            if cookie.name.trim().is_empty() || cookie.domain.trim().is_empty() {
                continue;
            }
            if cookie
                .expires_unix
                .is_some_and(|expires| expires <= now_unix)
            {
                continue;
            }
            let normalized_domain = cookie.domain.trim().trim_start_matches('.').to_lowercase();
            let normalized_path = cookie
                .path
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or("/")
                .to_string();
            let key = (
                cookie.name.trim().to_lowercase(),
                normalized_domain.clone(),
                normalized_path.clone(),
            );
            if !seen.insert(key) {
                continue;
            }
            cookies.push(Cookie {
                name: cookie.name.trim().to_string(),
                value: cookie.value.clone(),
                domain: normalized_domain,
                path: Some(normalized_path),
                expires_unix: cookie.expires_unix,
                secure: cookie.secure,
                http_only: cookie.http_only,
                same_site: cookie.same_site.clone(),
            });
        }
        cookies.reverse();

        Self {
            cookies,
            local_storage: self.local_storage.clone(),
            session_storage: self.session_storage.clone(),
            network_overrides: self.network_overrides.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Cookie, CookieSameSite, SessionState};

    #[test]
    fn normalized_deduplicates_and_drops_expired_cookies() {
        let session = SessionState {
            cookies: vec![
                Cookie {
                    name: "sid".into(),
                    value: "old".into(),
                    domain: ".Accounts.Shopify.com".into(),
                    path: Some(String::new()),
                    expires_unix: Some(1),
                    secure: true,
                    http_only: true,
                    same_site: Some(CookieSameSite::None),
                },
                Cookie {
                    name: "sid".into(),
                    value: "fresh".into(),
                    domain: "accounts.shopify.com".into(),
                    path: None,
                    expires_unix: None,
                    secure: true,
                    http_only: true,
                    same_site: Some(CookieSameSite::None),
                },
                Cookie {
                    name: "sid".into(),
                    value: "latest".into(),
                    domain: ".accounts.shopify.com".into(),
                    path: Some("/".into()),
                    expires_unix: None,
                    secure: true,
                    http_only: true,
                    same_site: Some(CookieSameSite::None),
                },
            ],
            ..SessionState::default()
        };

        let normalized = session.normalized();
        assert_eq!(normalized.cookies.len(), 1);
        assert_eq!(normalized.cookies[0].value, "latest");
        assert_eq!(normalized.cookies[0].domain, "accounts.shopify.com");
        assert_eq!(normalized.cookies[0].path.as_deref(), Some("/"));
    }
}
