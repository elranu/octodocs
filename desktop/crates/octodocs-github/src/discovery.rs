use anyhow::{Context, Result};
use serde::Deserialize;

use crate::client;
use crate::{BranchInfo, FolderEntry, RepoInfo};

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

    branches.sort_by(|a, b| a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase()));
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
        })?
        .error_for_status()
        .with_context(|| {
            format!("GitHub API returned error for folder listing in {owner}/{repo}")
        })?;

    let value: serde_json::Value = response.json()
        .with_context(|| format!("Failed to parse folder contents"))?;

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
