use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use aes::Aes128;
use cbc::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit, block_padding::Pkcs7};
use pbkdf2::pbkdf2_hmac;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use tempfile::tempdir;

use crate::config_store::AegisSecretStore;
use crate::session::cookies::{Cookie, SessionState};
use crate::session::profile::SessionProfileStore;
use crate::state::AegisStatePaths;

type Aes128CbcDec = cbc::Decryptor<Aes128>;
type Aes128CbcEnc = cbc::Encryptor<Aes128>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserKind {
    Chrome,
    Brave,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserProfileInfo {
    pub browser: BrowserKind,
    pub profile: String,
    pub display_name: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportedCredential {
    pub origin_url: String,
    pub action_url: Option<String>,
    pub username: String,
    pub password: String,
    pub signon_realm: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserImportReport {
    pub browser: BrowserKind,
    pub source_profile: String,
    pub target_profile: String,
    pub imported_cookies: usize,
    pub imported_credentials: usize,
    pub session_path: PathBuf,
    pub credentials_path: PathBuf,
    pub bookmarks_path: Option<PathBuf>,
    pub preferences_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserExportReport {
    pub browser: BrowserKind,
    pub source_profile: String,
    pub target_profile: String,
    pub exported_cookies: usize,
    pub exported_credentials: usize,
    pub manifest_path: PathBuf,
    pub credentials_csv_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BrowserImportManifest {
    browser: BrowserKind,
    source_profile: String,
    target_profile: String,
    cookies: usize,
    credentials: usize,
    session_path: PathBuf,
    credentials_path: PathBuf,
    bookmarks_path: Option<PathBuf>,
    preferences_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BrowserExportManifest {
    browser: BrowserKind,
    source_profile: String,
    target_profile: String,
    cookies: usize,
    credentials: usize,
    credentials_csv_path: PathBuf,
}

pub fn list_browser_profiles(browser: BrowserKind) -> Result<Vec<BrowserProfileInfo>, String> {
    let local_state_path = browser_root(browser).join("Local State");
    let bytes = fs::read(&local_state_path)
        .map_err(|error| format!("failed to read {}: {error}", local_state_path.display()))?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("failed to parse {}: {error}", local_state_path.display()))?;

    let mut profiles = Vec::new();
    if let Some(info_cache) = value
        .get("profile")
        .and_then(|profile| profile.get("info_cache"))
        .and_then(|cache| cache.as_object())
    {
        for (profile, info) in info_cache {
            let display_name = info
                .get("name")
                .and_then(|name| name.as_str())
                .unwrap_or(profile)
                .to_string();
            profiles.push(BrowserProfileInfo {
                browser,
                profile: profile.clone(),
                display_name,
                path: browser_root(browser).join(profile),
            });
        }
    }
    profiles.sort_by(|left, right| left.profile.cmp(&right.profile));
    Ok(profiles)
}

pub fn import_browser_profile(
    browser: BrowserKind,
    source_profile: &str,
    target_profile: &str,
) -> Result<BrowserImportReport, String> {
    let source_dir = browser_root(browser).join(source_profile);
    if !source_dir.exists() {
        return Err(format!(
            "browser profile {} not found at {}",
            source_profile,
            source_dir.display()
        ));
    }

    let key = derive_browser_key(browser)?;
    let temp_dir = tempdir().map_err(|error| format!("failed to create temp dir: {error}"))?;
    let cookies_path = copy_if_exists(&source_dir.join("Cookies"), temp_dir.path())?;
    let logins_path = copy_if_exists(&source_dir.join("Login Data"), temp_dir.path())?;

    let cookies = if let Some(path) = cookies_path.as_ref() {
        read_cookies(path, &key)?
    } else {
        Vec::new()
    };
    let credentials = if let Some(path) = logins_path.as_ref() {
        read_credentials(path, &key)?
    } else {
        Vec::new()
    };

    let mut session = SessionProfileStore::new(target_profile)?
        .load()?
        .unwrap_or_default();
    let imported_cookie_count = cookies.len();
    merge_cookies(&mut session, cookies);
    let session_path = SessionProfileStore::new(target_profile)?.save(&session)?;

    let secret_store = AegisSecretStore::detect()?;
    let merged_credentials = merge_credentials(
        secret_store.load_profile_credentials(target_profile)?,
        credentials.clone(),
    );
    let credentials_path =
        secret_store.save_profile_credentials(target_profile, &merged_credentials)?;

    let state_paths = AegisStatePaths::detect()?;
    let import_dir = state_paths.browser_import_dir(browser.slug(), source_profile);
    fs::create_dir_all(&import_dir).map_err(|error| {
        format!(
            "failed to create import directory {}: {error}",
            import_dir.display()
        )
    })?;

    let preferences_path = copy_json_artifact(
        &source_dir.join("Preferences"),
        &import_dir,
        "preferences.json",
    )?;
    let bookmarks_path =
        copy_json_artifact(&source_dir.join("Bookmarks"), &import_dir, "bookmarks.json")?;

    let manifest_path = import_dir.join("manifest.json");
    let manifest = BrowserImportManifest {
        browser,
        source_profile: source_profile.to_string(),
        target_profile: target_profile.to_string(),
        cookies: imported_cookie_count,
        credentials: credentials.len(),
        session_path: session_path.clone(),
        credentials_path: credentials_path.clone(),
        bookmarks_path: bookmarks_path.clone(),
        preferences_path: preferences_path.clone(),
    };
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest)
            .map_err(|error| format!("failed to encode import manifest: {error}"))?,
    )
    .map_err(|error| {
        format!(
            "failed to write import manifest {}: {error}",
            manifest_path.display()
        )
    })?;

    Ok(BrowserImportReport {
        browser,
        source_profile: source_profile.to_string(),
        target_profile: target_profile.to_string(),
        imported_cookies: imported_cookie_count,
        imported_credentials: credentials.len(),
        session_path,
        credentials_path,
        bookmarks_path,
        preferences_path,
    })
}

pub fn export_browser_profile(
    browser: BrowserKind,
    source_profile: &str,
    target_profile: &str,
) -> Result<BrowserExportReport, String> {
    ensure_browser_closed(browser)?;

    let browser_dir = browser_root(browser).join(target_profile);
    if !browser_dir.exists() {
        return Err(format!(
            "browser profile {} not found at {}",
            target_profile,
            browser_dir.display()
        ));
    }

    let state_paths = AegisStatePaths::detect()?;
    let export_dir = state_paths.browser_export_dir(browser.slug(), target_profile);
    fs::create_dir_all(&export_dir).map_err(|error| {
        format!(
            "failed to create export directory {}: {error}",
            export_dir.display()
        )
    })?;

    let session = SessionProfileStore::new(source_profile)?
        .load()?
        .unwrap_or_default();
    let secret_store = AegisSecretStore::detect()?;
    let credentials = secret_store.load_profile_credentials(source_profile)?;
    let key = derive_browser_key(browser)?;

    write_cookies_to_browser(&browser_dir.join("Cookies"), &session.cookies, &key)?;
    write_credentials_to_browser(&browser_dir.join("Login Data"), &credentials, &key)?;

    let credentials_csv_path = export_dir.join("passwords.csv");
    fs::write(&credentials_csv_path, encode_password_csv(&credentials)).map_err(|error| {
        format!(
            "failed to write browser export csv {}: {error}",
            credentials_csv_path.display()
        )
    })?;

    let manifest_path = export_dir.join("manifest.json");
    let manifest = BrowserExportManifest {
        browser,
        source_profile: source_profile.to_string(),
        target_profile: target_profile.to_string(),
        cookies: session.cookies.len(),
        credentials: credentials.len(),
        credentials_csv_path: credentials_csv_path.clone(),
    };
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest)
            .map_err(|error| format!("failed to encode export manifest: {error}"))?,
    )
    .map_err(|error| {
        format!(
            "failed to write export manifest {}: {error}",
            manifest_path.display()
        )
    })?;

    Ok(BrowserExportReport {
        browser,
        source_profile: source_profile.to_string(),
        target_profile: target_profile.to_string(),
        exported_cookies: session.cookies.len(),
        exported_credentials: credentials.len(),
        manifest_path,
        credentials_csv_path,
    })
}

fn browser_root(browser: BrowserKind) -> PathBuf {
    let home = std::env::var_os("HOME").unwrap_or_default();
    let base = Path::new(&home).join("Library/Application Support");
    match browser {
        BrowserKind::Chrome => base.join("Google/Chrome"),
        BrowserKind::Brave => base.join("BraveSoftware/Brave-Browser"),
    }
}

impl BrowserKind {
    pub fn slug(self) -> &'static str {
        match self {
            BrowserKind::Chrome => "chrome",
            BrowserKind::Brave => "brave",
        }
    }

    fn process_name(self) -> &'static str {
        match self {
            BrowserKind::Chrome => "Google Chrome",
            BrowserKind::Brave => "Brave Browser",
        }
    }

    fn safe_storage_service(self) -> &'static str {
        match self {
            BrowserKind::Chrome => "Chrome Safe Storage",
            BrowserKind::Brave => "Brave Safe Storage",
        }
    }

    fn safe_storage_account(self) -> &'static str {
        match self {
            BrowserKind::Chrome => "Chrome",
            BrowserKind::Brave => "Brave",
        }
    }
}

fn ensure_browser_closed(browser: BrowserKind) -> Result<(), String> {
    let output = Command::new("pgrep")
        .args(["-x", browser.process_name()])
        .output()
        .map_err(|error| {
            format!(
                "failed to check running {} process: {error}",
                browser.process_name()
            )
        })?;
    if output.status.success() && !output.stdout.is_empty() {
        return Err(format!(
            "{} must be fully closed before exporting into its profile database",
            browser.process_name()
        ));
    }
    Ok(())
}

fn derive_browser_key(browser: BrowserKind) -> Result<[u8; 16], String> {
    let password = Command::new("security")
        .args([
            "find-generic-password",
            "-w",
            "-s",
            browser.safe_storage_service(),
            "-a",
            browser.safe_storage_account(),
        ])
        .output()
        .map_err(|error| format!("failed to invoke security CLI: {error}"))?;
    if !password.status.success() {
        return Err(format!(
            "failed to read {} from keychain: {}",
            browser.safe_storage_service(),
            String::from_utf8_lossy(&password.stderr).trim()
        ));
    }

    let mut key = [0u8; 16];
    let passphrase = String::from_utf8(password.stdout)
        .map_err(|error| format!("keychain returned invalid utf-8 password: {error}"))?;
    pbkdf2_hmac::<Sha1>(
        passphrase.trim_end().as_bytes(),
        b"saltysalt",
        1003,
        &mut key,
    );
    Ok(key)
}

fn copy_if_exists(path: &Path, temp_root: &Path) -> Result<Option<PathBuf>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let file_name = path
        .file_name()
        .ok_or_else(|| format!("invalid source path {}", path.display()))?;
    let copy_path = temp_root.join(file_name);
    fs::copy(path, &copy_path)
        .map_err(|error| format!("failed to copy {}: {error}", path.display()))?;
    Ok(Some(copy_path))
}

fn copy_json_artifact(
    path: &Path,
    import_dir: &Path,
    file_name: &str,
) -> Result<Option<PathBuf>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let target = import_dir.join(file_name);
    fs::copy(path, &target)
        .map_err(|error| format!("failed to copy {}: {error}", path.display()))?;
    Ok(Some(target))
}

fn read_cookies(path: &Path, key: &[u8; 16]) -> Result<Vec<Cookie>, String> {
    let connection = Connection::open(path)
        .map_err(|error| format!("failed to open cookies db {}: {error}", path.display()))?;
    let mut statement = connection
        .prepare(
            "SELECT host_key, name, value, encrypted_value, path, expires_utc, is_secure, is_httponly \
             FROM cookies",
        )
        .map_err(|error| format!("failed to prepare cookies query: {error}"))?;
    let rows = statement
        .query_map([], |row| {
            let value: String = row.get(2)?;
            let encrypted_value: Vec<u8> = row.get(3)?;
            let cookie_value = if !value.is_empty() {
                value
            } else {
                decrypt_chromium_secret(&encrypted_value, key).unwrap_or_default()
            };
            Ok(Cookie {
                name: row.get(1)?,
                value: cookie_value,
                domain: row.get(0)?,
                path: Some(row.get(4)?),
                expires_unix: chrome_time_to_unix(row.get::<_, i64>(5)?),
                secure: row.get::<_, i64>(6)? != 0,
                http_only: row.get::<_, i64>(7)? != 0,
            })
        })
        .map_err(|error| format!("failed to read cookies: {error}"))?;

    let mut cookies = Vec::new();
    for row in rows {
        cookies.push(row.map_err(|error| format!("failed to map cookie row: {error}"))?);
    }
    Ok(cookies)
}

fn read_credentials(path: &Path, key: &[u8; 16]) -> Result<Vec<ImportedCredential>, String> {
    let connection = Connection::open(path)
        .map_err(|error| format!("failed to open login db {}: {error}", path.display()))?;
    let mut statement = connection
        .prepare(
            "SELECT origin_url, action_url, username_value, password_value, signon_realm \
             FROM logins WHERE blacklisted_by_user = 0",
        )
        .map_err(|error| format!("failed to prepare login query: {error}"))?;
    let rows = statement
        .query_map([], |row| {
            let encrypted_password: Vec<u8> = row.get(3)?;
            Ok(ImportedCredential {
                origin_url: row.get(0)?,
                action_url: row.get::<_, Option<String>>(1)?,
                username: row.get(2)?,
                password: decrypt_chromium_secret(&encrypted_password, key).unwrap_or_default(),
                signon_realm: row.get(4)?,
            })
        })
        .map_err(|error| format!("failed to read login rows: {error}"))?;

    let mut credentials = Vec::new();
    for row in rows {
        let credential = row.map_err(|error| format!("failed to map login row: {error}"))?;
        if !credential.username.is_empty() || !credential.password.is_empty() {
            credentials.push(credential);
        }
    }
    Ok(credentials)
}

fn write_cookies_to_browser(path: &Path, cookies: &[Cookie], key: &[u8; 16]) -> Result<(), String> {
    let mut connection = Connection::open(path).map_err(|error| {
        format!(
            "failed to open browser cookies db {}: {error}",
            path.display()
        )
    })?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("failed to start cookie transaction: {error}"))?;
    let now = current_chrome_timestamp()?;
    for cookie in cookies {
        let domain = cookie.domain.trim();
        if domain.is_empty() || cookie.name.trim().is_empty() {
            continue;
        }
        let cookie_path = cookie.path.clone().unwrap_or_else(|| "/".to_string());
        let expires_utc = cookie.expires_unix.map(unix_to_chrome_time).unwrap_or(0);
        let has_expires = i64::from(cookie.expires_unix.is_some());
        let is_persistent = has_expires;
        let top_frame_site_key = String::new();
        let source_scheme = if cookie.secure { 2 } else { 1 };
        let source_port = if cookie.secure { 443 } else { 80 };
        let encrypted_value = encrypt_chromium_secret(&cookie.value, key)?;
        transaction
            .execute(
                "INSERT INTO cookies (
                    creation_utc, host_key, top_frame_site_key, name, value, encrypted_value,
                    path, expires_utc, is_secure, is_httponly, last_access_utc, has_expires,
                    is_persistent, priority, samesite, source_scheme, source_port, last_update_utc,
                    source_type, has_cross_site_ancestor
                 ) VALUES (?1, ?2, ?3, ?4, '', ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 1, -1, ?13, ?14, ?15, 1, 0)
                 ON CONFLICT(host_key, top_frame_site_key, has_cross_site_ancestor, name, path, source_scheme, source_port)
                 DO UPDATE SET
                    value=excluded.value,
                    encrypted_value=excluded.encrypted_value,
                    expires_utc=excluded.expires_utc,
                    is_secure=excluded.is_secure,
                    is_httponly=excluded.is_httponly,
                    last_access_utc=excluded.last_access_utc,
                    has_expires=excluded.has_expires,
                    is_persistent=excluded.is_persistent,
                    last_update_utc=excluded.last_update_utc",
                params![
                    now,
                    domain,
                    top_frame_site_key,
                    cookie.name,
                    encrypted_value,
                    cookie_path,
                    expires_utc,
                    i64::from(cookie.secure),
                    i64::from(cookie.http_only),
                    now,
                    has_expires,
                    is_persistent,
                    source_scheme,
                    source_port,
                    now,
                ],
            )
            .map_err(|error| format!("failed to write cookie {} for {}: {error}", cookie.name, domain))?;
    }
    transaction
        .commit()
        .map_err(|error| format!("failed to commit cookie export: {error}"))?;
    Ok(())
}

fn write_credentials_to_browser(
    path: &Path,
    credentials: &[ImportedCredential],
    key: &[u8; 16],
) -> Result<(), String> {
    let mut connection = Connection::open(path).map_err(|error| {
        format!(
            "failed to open browser login db {}: {error}",
            path.display()
        )
    })?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("failed to start login transaction: {error}"))?;
    let now = current_chrome_timestamp()?;
    for credential in credentials {
        if credential.origin_url.trim().is_empty() || credential.signon_realm.trim().is_empty() {
            continue;
        }
        let encrypted_password = encrypt_chromium_secret(&credential.password, key)?;
        transaction
            .execute(
                "INSERT INTO logins (
                    origin_url, action_url, username_element, username_value, password_element,
                    password_value, submit_element, signon_realm, date_created,
                    blacklisted_by_user, scheme, password_type, times_used, date_last_used,
                    date_password_modified, date_last_filled
                ) VALUES (?1, ?2, 'username', ?3, 'password', ?4, '', ?5, ?6, 0, 0, 0, 0, 0, ?7, 0)
                ON CONFLICT(origin_url, username_element, username_value, password_element, signon_realm)
                DO UPDATE SET
                    action_url=excluded.action_url,
                    password_value=excluded.password_value,
                    date_password_modified=excluded.date_password_modified",
                params![
                    credential.origin_url,
                    credential.action_url,
                    credential.username,
                    encrypted_password,
                    credential.signon_realm,
                    now,
                    now,
                ],
            )
            .map_err(|error| {
                format!(
                    "failed to write credential for {} / {}: {error}",
                    credential.signon_realm, credential.username
                )
            })?;
    }
    transaction
        .commit()
        .map_err(|error| format!("failed to commit browser credential export: {error}"))?;
    Ok(())
}

fn encrypt_chromium_secret(value: &str, key: &[u8; 16]) -> Result<Vec<u8>, String> {
    let iv = [0x20u8; 16];
    let ciphertext = Aes128CbcEnc::new(key.into(), (&iv).into())
        .encrypt_padded_vec_mut::<Pkcs7>(value.as_bytes());
    let mut encrypted = b"v10".to_vec();
    encrypted.extend_from_slice(&ciphertext);
    Ok(encrypted)
}

fn decrypt_chromium_secret(encrypted: &[u8], key: &[u8; 16]) -> Result<String, String> {
    if encrypted.is_empty() {
        return Ok(String::new());
    }
    let ciphertext = encrypted
        .strip_prefix(b"v10")
        .or_else(|| encrypted.strip_prefix(b"v11"))
        .unwrap_or(encrypted);
    let iv = [0x20u8; 16];
    let plaintext = Aes128CbcDec::new(key.into(), (&iv).into())
        .decrypt_padded_vec_mut::<Pkcs7>(ciphertext)
        .map_err(|error| format!("failed to decrypt browser secret: {error}"))?;
    String::from_utf8(plaintext)
        .map_err(|error| format!("browser secret is not valid utf-8: {error}"))
}

fn current_chrome_timestamp() -> Result<i64, String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock is before unix epoch: {error}"))?;
    Ok(unix_to_chrome_time(now.as_secs()))
}

fn chrome_time_to_unix(value: i64) -> Option<u64> {
    if value <= 0 {
        return None;
    }
    let micros = value.checked_sub(11_644_473_600_000_000)?;
    Some((micros / 1_000_000) as u64)
}

fn unix_to_chrome_time(value: u64) -> i64 {
    (value as i64 * 1_000_000) + 11_644_473_600_000_000
}

fn merge_cookies(session: &mut SessionState, imported: Vec<Cookie>) {
    let mut cookies = BTreeMap::<(String, String, String), Cookie>::new();
    for cookie in session.cookies.drain(..) {
        let path = cookie.path.clone().unwrap_or_else(|| "/".into());
        cookies.insert((cookie.domain.clone(), cookie.name.clone(), path), cookie);
    }
    for cookie in imported {
        let path = cookie.path.clone().unwrap_or_else(|| "/".into());
        cookies.insert((cookie.domain.clone(), cookie.name.clone(), path), cookie);
    }
    session.cookies = cookies.into_values().collect();
}

fn merge_credentials(
    existing: Vec<ImportedCredential>,
    imported: Vec<ImportedCredential>,
) -> Vec<ImportedCredential> {
    let mut merged = BTreeMap::<(String, String, String), ImportedCredential>::new();
    for credential in existing.into_iter().chain(imported) {
        merged.insert(
            (
                credential.signon_realm.clone(),
                credential.origin_url.clone(),
                credential.username.clone(),
            ),
            credential,
        );
    }
    merged.into_values().collect()
}

fn encode_password_csv(credentials: &[ImportedCredential]) -> String {
    let mut csv = String::from("name,url,username,password,note\n");
    for credential in credentials {
        let name = credential.signon_realm.clone();
        let url = credential
            .action_url
            .clone()
            .unwrap_or_else(|| credential.origin_url.clone());
        csv.push_str(&csv_cell(&name));
        csv.push(',');
        csv.push_str(&csv_cell(&url));
        csv.push(',');
        csv.push_str(&csv_cell(&credential.username));
        csv.push(',');
        csv.push_str(&csv_cell(&credential.password));
        csv.push(',');
        csv.push_str(&csv_cell(""));
        csv.push('\n');
    }
    csv
}

fn csv_cell(value: &str) -> String {
    let escaped = value.replace('"', "\"\"");
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod tests {
    use super::{
        ImportedCredential, csv_cell, decrypt_chromium_secret, encode_password_csv,
        encrypt_chromium_secret, merge_credentials,
    };

    #[test]
    fn chromium_secret_round_trip() {
        let key = [7u8; 16];
        let encrypted = encrypt_chromium_secret("secret-value", &key).unwrap();
        assert!(encrypted.starts_with(b"v10"));
        let decrypted = decrypt_chromium_secret(&encrypted, &key).unwrap();
        assert_eq!(decrypted, "secret-value");
    }

    #[test]
    fn merge_credentials_overwrites_same_identity() {
        let existing = vec![ImportedCredential {
            origin_url: "https://example.com/login".into(),
            action_url: Some("https://example.com/session".into()),
            username: "saint".into(),
            password: "old".into(),
            signon_realm: "https://example.com/".into(),
        }];
        let imported = vec![ImportedCredential {
            origin_url: "https://example.com/login".into(),
            action_url: Some("https://example.com/session".into()),
            username: "saint".into(),
            password: "new".into(),
            signon_realm: "https://example.com/".into(),
        }];
        let merged = merge_credentials(existing, imported);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].password, "new");
    }

    #[test]
    fn password_csv_matches_browser_import_shape() {
        let csv = encode_password_csv(&[ImportedCredential {
            origin_url: "https://example.com/login".into(),
            action_url: Some("https://example.com/session".into()),
            username: "saint".into(),
            password: "pw".into(),
            signon_realm: "Example".into(),
        }]);
        assert!(csv.starts_with("name,url,username,password,note\n"));
        assert!(csv.contains(&csv_cell("Example")));
        assert!(csv.contains(&csv_cell("https://example.com/session")));
    }
}
