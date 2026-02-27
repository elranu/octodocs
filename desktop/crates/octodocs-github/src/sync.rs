use anyhow::{Context, Result};
use base64::Engine;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};

use crate::client;
use crate::GitHubSyncConfig;

const API_BASE: &str = "https://api.github.com";

#[derive(Debug, Deserialize)]
struct ContentShaResponse {
    sha: String,
}

#[derive(Debug, Serialize)]
struct CreateUpdateFileRequest<'a> {
    message: &'a str,
    content: String,
    branch: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    sha: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
struct FileUpdateResponse {
    commit: CommitInfo,
}

#[derive(Debug, Serialize)]
struct DeleteFileRequest<'a> {
    message: &'a str,
    sha: &'a str,
    branch: &'a str,
}

#[derive(Debug, Deserialize)]
struct CommitInfo {
    sha: String,
}

pub(crate) fn build_repo_path(config: &GitHubSyncConfig, filename: &str) -> String {
    let folder = config.folder.trim_matches('/');
    if folder.is_empty() {
        filename.to_string()
    } else {
        format!("{folder}/{filename}")
    }
}

pub fn get_file_sha(
    token: &str,
    config: &GitHubSyncConfig,
    filename: &str,
) -> Result<Option<String>> {
    let client = client::build(token)?;
    let path = build_repo_path(config, filename);
    let url = format!(
        "{API_BASE}/repos/{}/{}/contents/{}?ref={}",
        config.owner, config.repo, path, config.branch
    );

    let response = client
        .get(&url)
        .send()
        .with_context(|| format!("Failed to query existing file SHA for '{path}'"))?;

    if response.status() == StatusCode::NOT_FOUND {
        return Ok(None);
    }

    let response = response
        .error_for_status()
        .with_context(|| format!("GitHub returned error while querying SHA for '{path}'"))?;

    let content: ContentShaResponse = response
        .json()
        .with_context(|| format!("Failed to parse SHA response for '{path}'"))?;

    Ok(Some(content.sha))
}

pub fn push_file(
    token: &str,
    config: &GitHubSyncConfig,
    filename: &str,
    content: &str,
) -> Result<String> {
    let client = client::build(token)?;
    let path = build_repo_path(config, filename);
    let url = format!(
        "{API_BASE}/repos/{}/{}/contents/{}",
        config.owner, config.repo, path
    );
    let message = format!("OctoDocs: update {filename}");

    let existing_sha = get_file_sha(token, config, filename)?;

    let body = CreateUpdateFileRequest {
        message: &message,
        content: base64::engine::general_purpose::STANDARD.encode(content.as_bytes()),
        branch: &config.branch,
        sha: existing_sha.as_deref(),
    };

    let response = client
        .put(&url)
        .json(&body)
        .send()
        .with_context(|| {
            format!(
                "Failed to push '{path}' to {}/{}#{}",
                config.owner, config.repo, config.branch
            )
        })?
        .error_for_status()
        .with_context(|| {
            format!(
                "GitHub returned error while pushing '{path}'",
            )
        })?;

    let update: FileUpdateResponse = response
        .json()
        .with_context(|| format!("Failed to parse push response for '{path}'"))?;

    Ok(update.commit.sha)
}

pub fn delete_file(token: &str, config: &GitHubSyncConfig, filename: &str) -> Result<Option<String>> {
    let Some(existing_sha) = get_file_sha(token, config, filename)? else {
        return Ok(None);
    };

    let client = client::build(token)?;
    let path = build_repo_path(config, filename);
    let url = format!(
        "{API_BASE}/repos/{}/{}/contents/{}",
        config.owner, config.repo, path
    );
    let message = format!("OctoDocs: delete {filename}");

    let body = DeleteFileRequest {
        message: &message,
        sha: &existing_sha,
        branch: &config.branch,
    };

    let response = client
        .delete(&url)
        .json(&body)
        .send()
        .with_context(|| {
            format!(
                "Failed to delete '{path}' from {}/{}#{}",
                config.owner, config.repo, config.branch
            )
        })?
        .error_for_status()
        .with_context(|| {
            format!(
                "GitHub returned error while deleting '{path}'",
            )
        })?;

    let update: FileUpdateResponse = response
        .json()
        .with_context(|| format!("Failed to parse delete response for '{path}'"))?;

    Ok(Some(update.commit.sha))
}
