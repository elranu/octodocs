use std::time::SystemTime;

use serde::{Deserialize, Serialize};

pub mod auth;
pub mod client;
pub mod discovery;
pub mod sync;
pub use auth::{
    clear_stored_token, get_stored_token, poll_for_token, start_device_flow, store_token,
    wait_for_token, DeviceFlowHandle, FileStore, KeyringStore, PollResult, TokenStore,
};
pub use client::build as build_client;
pub use discovery::{list_branches, list_folder, list_repos, pull_file, pull_markdown_files};
pub use sync::{delete_file, get_file_sha, push_file};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubSyncConfig {
    pub owner: String,
    pub repo: String,
    pub branch: String,
    pub folder: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncStatus {
    Idle,
    Syncing,
    Success { committed_at: SystemTime, sha: String },
    Failed { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoInfo {
    pub owner: String,
    pub name: String,
    pub default_branch: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchInfo {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
}
