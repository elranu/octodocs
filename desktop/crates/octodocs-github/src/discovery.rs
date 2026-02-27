use anyhow::{Context, Result};
use base64::Engine;
use reqwest::StatusCode;
use serde::Deserialize;

use crate::client;
use crate::{BranchInfo, FolderEntry, GitHubSyncConfig, RepoInfo};

const API_BASE: &str = "https://api.github.com";

#[derive(Debug, Deserialize)]
struct RepoOwner {
    login: String,
}

#[derive(Debug, Deserialize)]
struct RepoApi {
    name: String,
    owner: RepoOwner,
    default_branch: String,
}

#[derive(Debug, Deserialize)]
struct BranchApi {
    name: String,
}

#[derive(Debug, Deserialize)]
struct FileContentApi {
    content: Option<String>,
    encoding: Option<String>,
}

/// Parse the `rel="next"` link from the GitHub Link header for pagination.
fn parse_next_link(link_header: Option<&str>) -> Option<String> {
    let link = link_header?;
    for part in link.split(',') {
        let parts: Vec<&str> = part.split(';').map(|s| s.trim()).collect();
        if parts.len() == 2 && parts[1].contains("rel=\"next\"") {
            let url = parts[0].trim_start_matches('<').trim_end_matches('>');
            return Some(url.to_string());
        }
    }
    None
}

pub fn list_repos(token: &str) -> Result<Vec<RepoInfo>> {
    let client = client::build(token)?;
    let mut repos = Vec::new();
    let mut url = format!("{API_BASE}/user/repos?per_page=100&type=all");

    loop {
        let response = client
            .get(&url)
            .send()
            .context("Failed to list repositories")?
            .error_for_status()
            .context("GitHub API returned error")?;

        let next_link = parse_next_link(
            response.headers()
                .get("link")
                .and_then(|v| v.to_str().ok())
        );

        let page: Vec<RepoApi> = response.json()
            .context("Failed to parse repository list")?;

        for repo in page {
            repos.push(RepoInfo {
                owner: repo.owner.login,
                name: repo.name,
                default_branch: repo.default_branch,
            });
        }

        match next_link {
            Some(link) => url = link,
            None => break,
        }
    }

    repos.sort_by(|a, b| {
        let ao = a.owner.to_ascii_lowercase();
        let bo = b.owner.to_ascii_lowercase();
        ao.cmp(&bo).then_with(|| a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase()))
    });

    Ok(repos)
}

pub fn list_branches(token: &str, owner: &str, repo: &str) -> Result<Vec<BranchInfo>> {
    let client = client::build(token)?;
    let mut branches = Vec::new();
    let mut url = format!("{API_BASE}/repos/{owner}/{repo}/branches?per_page=100");

    loop {
        let response = client
            .get(&url)
            .send()
            .with_context(|| format!("Failed to list branches for {owner}/{repo}"))?
            .error_for_status()
            .with_context(|| format!("GitHub API returned error for {owner}/{repo}"))?;

        let next_link = parse_next_link(
            response.headers()
                .get("link")
                .and_then(|v| v.to_str().ok())
        );

        let page: Vec<BranchApi> = response.json()
            .with_context(|| format!("Failed to parse branch list for {owner}/{repo}"))?;

        for branch in page {
            branches.push(BranchInfo { name: branch.name });
        }

        match next_link {
            Some(link) => url = link,
            None => break,
        }
    }

    branches.sort_by_key(|a| a.name.to_ascii_lowercase());
    Ok(branches)
}

pub fn list_folder(
    token: &str,
    owner: &str,
    repo: &str,
    branch: &str,
    path: &str,
) -> Result<Vec<FolderEntry>> {
    let client = client::build(token)?;

    let clean_path = path.trim_matches('/');
    let url = if clean_path.is_empty() {
        format!("{API_BASE}/repos/{owner}/{repo}/contents?ref={branch}")
    } else {
        format!("{API_BASE}/repos/{owner}/{repo}/contents/{clean_path}?ref={branch}")
    };

    let response = client
        .get(&url)
        .send()
        .with_context(|| {
            format!(
                "Failed to list folder '{}' in {owner}/{repo}#{branch}",
                if clean_path.is_empty() { "/" } else { clean_path }
            )
        })?;

    if response.status() == StatusCode::NOT_FOUND || response.status() == StatusCode::CONFLICT {
        return Ok(vec![]);
    }

    let response = response
        .error_for_status()
        .with_context(|| {
            format!("GitHub API returned error for folder listing in {owner}/{repo}")
        })?;

    let value: serde_json::Value = response.json()
        .with_context(|| "Failed to parse folder contents".to_string())?;

    let items = value
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|item| {
            let item_type = item.get("type").and_then(|v| v.as_str())?;
            let name = item.get("name").and_then(|v| v.as_str())?.to_string();
            let path = item.get("path").and_then(|v| v.as_str())?.to_string();
            Some(FolderEntry {
                name,
                path,
                is_dir: item_type == "dir",
            })
        })
        .collect::<Vec<_>>();

    Ok(items)
}

fn fetch_file_content(
    token: &str,
    owner: &str,
    repo: &str,
    branch: &str,
    path: &str,
) -> Result<String> {
    let client = client::build(token)?;
    let clean_path = path.trim_matches('/');
    let url = format!(
        "{API_BASE}/repos/{owner}/{repo}/contents/{clean_path}?ref={branch}"
    );

    let response = client
        .get(&url)
        .send()
        .with_context(|| format!("Failed to fetch file content for '{clean_path}'"))?
        .error_for_status()
        .with_context(|| format!("GitHub API returned error while reading '{clean_path}'"))?;

    let file: FileContentApi = response
        .json()
        .with_context(|| format!("Failed to parse file content for '{clean_path}'"))?;

    let content = file
        .content
        .ok_or_else(|| anyhow::anyhow!("Missing content for '{clean_path}'"))?;
    let encoding = file.encoding.unwrap_or_default().to_ascii_lowercase();

    if encoding != "base64" {
        return Err(anyhow::anyhow!(
            "Unsupported encoding '{encoding}' for '{clean_path}'"
        ));
    }

    let normalized = content.replace('\n', "");
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(normalized.as_bytes())
        .with_context(|| format!("Failed to decode base64 for '{clean_path}'"))?;

    String::from_utf8(decoded)
        .with_context(|| format!("File '{clean_path}' is not valid UTF-8"))
}

fn collect_markdown_files_recursive(
    token: &str,
    owner: &str,
    repo: &str,
    branch: &str,
    path: &str,
    out: &mut Vec<(String, String)>,
) -> Result<()> {
    let entries = list_folder(token, owner, repo, branch, path)?;

    for entry in entries {
        if entry.is_dir {
            collect_markdown_files_recursive(
                token,
                owner,
                repo,
                branch,
                &entry.path,
                out,
            )?;
            continue;
        }

        if entry.name.to_ascii_lowercase().ends_with(".md") {
            let content = fetch_file_content(token, owner, repo, branch, &entry.path)?;
            out.push((entry.path, content));
        }
    }

    Ok(())
}

pub fn pull_markdown_files(
    token: &str,
    owner: &str,
    repo: &str,
    branch: &str,
    folder: &str,
) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    collect_markdown_files_recursive(token, owner, repo, branch, folder, &mut out)?;
    Ok(out)
}

/// Percent-encode each segment of a repo path (preserving `/` separators).
fn percent_encode_path(path: &str) -> String {
    path.split('/')
        .map(|seg| {
            seg.bytes()
                .flat_map(|b| {
                    if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                        vec![b as char]
                    } else {
                        format!("%{b:02X}").chars().collect::<Vec<_>>()
                    }
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Fetch a single file from GitHub, returning `None` if the file does not exist (404).
/// Remote content is returned as a UTF-8 string on success.
/// The path within the repo is resolved from `config.folder` + `filename`.
pub fn pull_file(
    token: &str,
    config: &GitHubSyncConfig,
    filename: &str,
) -> Result<Option<String>> {
    let repo_path = crate::sync::build_repo_path(config, filename);
    let encoded_path = percent_encode_path(&repo_path);

    let client = client::build(token)?;
    let url = format!(
        "{API_BASE}/repos/{}/{}/contents/{}?ref={}",
        config.owner, config.repo, encoded_path, config.branch
    );

    let response = client
        .get(&url)
        .send()
        .with_context(|| format!("Failed to fetch '{repo_path}' from GitHub"))?;

    if response.status() == StatusCode::NOT_FOUND {
        return Ok(None);
    }

    let response = response
        .error_for_status()
        .with_context(|| format!("GitHub returned error while pulling '{repo_path}'"))?;

    let file: FileContentApi = response
        .json()
        .with_context(|| format!("Failed to parse content response for '{repo_path}'"))?;

    let content = file
        .content
        .ok_or_else(|| anyhow::anyhow!("Missing content in response for '{repo_path}'"))?;
    let encoding = file.encoding.unwrap_or_default().to_ascii_lowercase();

    if encoding != "base64" {
        return Err(anyhow::anyhow!(
            "Unsupported encoding '{encoding}' for '{repo_path}'"
        ));
    }

    let normalized = content.replace('\n', "");
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(normalized.as_bytes())
        .with_context(|| format!("Failed to decode base64 for '{repo_path}'"))?;

    let text = String::from_utf8(decoded)
        .with_context(|| format!("File '{repo_path}' is not valid UTF-8"))?;

    Ok(Some(text))
}
