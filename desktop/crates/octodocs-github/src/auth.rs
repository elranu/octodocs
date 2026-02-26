use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use dirs::config_dir;
use keyring::Entry;
use reqwest::blocking::Client;
use serde::Deserialize;

const DEVICE_CODE_URL: &str = "https://github.com/login/device/code";
const ACCESS_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
const KEYRING_SERVICE: &str = "octodocs";
const KEYRING_USER: &str = "github-token";

#[derive(Debug, Clone)]
pub struct DeviceFlowHandle {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub interval_secs: u64,
    pub expires_at: Instant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PollResult {
    Pending,
    Token(String),
}

pub trait TokenStore {
    fn get_token(&self) -> Result<Option<String>>;
    fn set_token(&self, token: &str) -> Result<()>;
    fn clear_token(&self) -> Result<()>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct KeyringStore;

impl TokenStore for KeyringStore {
    fn get_token(&self) -> Result<Option<String>> {
        let entry = Entry::new(KEYRING_SERVICE, KEYRING_USER)?;
        match entry.get_password() {
            Ok(token) => Ok(Some(token)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    fn set_token(&self, token: &str) -> Result<()> {
        let entry = Entry::new(KEYRING_SERVICE, KEYRING_USER)?;
        entry.set_password(token)?;
        Ok(())
    }

    fn clear_token(&self) -> Result<()> {
        let entry = Entry::new(KEYRING_SERVICE, KEYRING_USER)?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(err.into()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct FileStore {
    path: PathBuf,
}

impl Default for FileStore {
    fn default() -> Self {
        let mut base = config_dir().unwrap_or_else(|| PathBuf::from("."));
        base.push("octodocs");
        base.push("token");
        Self { path: base }
    }
}

impl FileStore {
    pub fn with_path(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }
}

impl TokenStore for FileStore {
    fn get_token(&self) -> Result<Option<String>> {
        if !self.path.exists() {
            return Ok(None);
        }
        let token = std::fs::read_to_string(&self.path)
            .with_context(|| format!("Failed reading token file {}", self.path.display()))?;
        let trimmed = token.trim().to_string();
        if trimmed.is_empty() {
            Ok(None)
        } else {
            Ok(Some(trimmed))
        }
    }

    fn set_token(&self, token: &str) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("Failed creating token directory {}", parent.display())
            })?;
        }
        std::fs::write(&self.path, token)
            .with_context(|| format!("Failed writing token file {}", self.path.display()))?;
        Ok(())
    }

    fn clear_token(&self) -> Result<()> {
        if self.path.exists() {
            std::fs::remove_file(&self.path).with_context(|| {
                format!("Failed removing token file {}", self.path.display())
            })?;
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct AccessTokenResponse {
    access_token: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OAuthErrorResponse {
    error: Option<String>,
    error_description: Option<String>,
}

pub fn start_device_flow(client_id: &str, scopes: &[&str]) -> Result<DeviceFlowHandle> {
    let client = Client::new();
    let scope = scopes.join(" ");

    let response = client
        .post(DEVICE_CODE_URL)
        .header("Accept", "application/json")
        .header("User-Agent", "octodocs-app")
        .form(&[
            ("client_id", client_id),
            ("scope", scope.as_str()),
        ])
        .send()
        .context("Failed to call GitHub device code endpoint")?;

    let status = response.status();
    let body = response
        .text()
        .context("Failed to read GitHub device code response")?;

    if !status.is_success() {
        if let Ok(err) = serde_json::from_str::<OAuthErrorResponse>(&body) {
            let code = err.error.unwrap_or_else(|| "unknown_error".to_string());
            let desc = err
                .error_description
                .unwrap_or_else(|| "No description provided".to_string());
            return Err(anyhow::anyhow!(
                "GitHub device code endpoint returned {status}: {code} ({desc})"
            ));
        }
        return Err(anyhow::anyhow!(
            "GitHub device code endpoint returned {status}: {body}"
        ));
    }

    let response = serde_json::from_str::<DeviceCodeResponse>(&body)
        .context("Failed to parse device code response")?;

    Ok(DeviceFlowHandle {
        device_code: response.device_code,
        user_code: response.user_code,
        verification_uri: response.verification_uri,
        interval_secs: response.interval.unwrap_or(5),
        expires_at: Instant::now() + Duration::from_secs(response.expires_in),
    })
}

pub fn poll_for_token(client_id: &str, handle: &DeviceFlowHandle) -> Result<PollResult> {
    let client = Client::new();

    let response = client
        .post(ACCESS_TOKEN_URL)
        .header("Accept", "application/json")
        .header("User-Agent", "octodocs-app")
        .form(&[
            ("client_id", client_id),
            ("device_code", handle.device_code.as_str()),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ])
        .send()
        .context("Failed to call GitHub access token endpoint")?;

    let status = response.status();
    let body = response
        .text()
        .context("Failed to read GitHub access token response")?;

    if !status.is_success() {
        if let Ok(err) = serde_json::from_str::<OAuthErrorResponse>(&body) {
            let code = err.error.unwrap_or_else(|| "unknown_error".to_string());
            let desc = err
                .error_description
                .unwrap_or_else(|| "No description provided".to_string());
            return Err(anyhow::anyhow!(
                "GitHub access token endpoint returned {status}: {code} ({desc})"
            ));
        }
        return Err(anyhow::anyhow!(
            "GitHub access token endpoint returned {status}: {body}"
        ));
    }

    let response = serde_json::from_str::<AccessTokenResponse>(&body)
        .context("Failed to parse access token response")?;

    if let Some(token) = response.access_token {
        return Ok(PollResult::Token(token));
    }

    match response.error.as_deref() {
        Some("authorization_pending") | Some("slow_down") => Ok(PollResult::Pending),
        Some("expired_token") => Err(anyhow::anyhow!("Device code expired")),
        Some(other) => Err(anyhow::anyhow!("GitHub OAuth error: {other}")),
        None => Err(anyhow::anyhow!("GitHub OAuth response missing token and error")),
    }
}

pub fn wait_for_token(client_id: &str, handle: &DeviceFlowHandle) -> Result<String> {
    loop {
        if Instant::now() >= handle.expires_at {
            return Err(anyhow::anyhow!("Device authorization window expired"));
        }

        match poll_for_token(client_id, handle)? {
            PollResult::Token(token) => return Ok(token),
            PollResult::Pending => std::thread::sleep(Duration::from_secs(handle.interval_secs)),
        }
    }
}

pub fn store_token(token: &str) -> Result<()> {
    let keyring = KeyringStore;
    let file = FileStore::default();

    let keyring_result = keyring.set_token(token);
    let file_result = file.set_token(token);

    match (keyring_result, file_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Ok(()), Err(_)) => Ok(()),
        (Err(_), Ok(())) => Ok(()),
        (Err(keyring_err), Err(file_err)) => Err(anyhow::anyhow!(
            "Failed to store token in keyring ({keyring_err}) and file ({file_err})"
        )),
    }
}

pub fn get_stored_token() -> Result<Option<String>> {
    let keyring = KeyringStore;
    match keyring.get_token() {
        Ok(Some(token)) => Ok(Some(token)),
        Ok(None) => FileStore::default().get_token(),
        Err(_) => FileStore::default().get_token(),
    }
}

pub fn clear_stored_token() -> Result<()> {
    let keyring = KeyringStore;
    let mut first_err: Option<anyhow::Error> = None;

    if let Err(err) = keyring.clear_token() {
        first_err = Some(err);
    }

    if let Err(err) = FileStore::default().clear_token() {
        if first_err.is_none() {
            first_err = Some(err);
        }
    }

    if let Some(err) = first_err {
        return Err(err);
    }

    Ok(())
}
