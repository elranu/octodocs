# GitHub Sync — Implementation Plan

## Overview

Add seamless, zero-configuration GitHub backup to OctoDocs. A non-technical user logs in once with their GitHub account (OAuth Device Flow — no redirect, no terminal), picks a repository, branch, and folder, and from that point on **every Save automatically pushes the current file** to that location on GitHub using the GitHub Contents REST API.

There is **no local git repository**, no `git` binary required, and no merge conflicts exposed to the user. The entire flow is hidden behind a small sync badge and a one-time setup panel.

References:
- Internal: [desktop/crates/octodocs-app/src/app_state.rs](../desktop/crates/octodocs-app/src/app_state.rs)
- Internal: [desktop/crates/octodocs-app/src/views/root.rs](../desktop/crates/octodocs-app/src/views/root.rs)
- Internal: [desktop/crates/octodocs-core/src/document.rs](../desktop/crates/octodocs-core/src/document.rs)
- External: [GitHub Device Flow docs](https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/authorizing-oauth-apps#device-flow)
- External: [GitHub Contents API](https://docs.github.com/en/rest/repos/contents)
- External: [octocrab crate](https://docs.rs/octocrab)
- External: [keyring crate](https://docs.rs/keyring)

---

## Design Alignment

### Chosen Direction

- **Approach:** GitHub Contents REST API via `octocrab` (no local git, no `git2-rs`)
- **Rationale:** Writing/updating a single file requires one HTTP PUT call. No local clone, no disk footprint, no libgit2 C dependency. Users never encounter git terminology. `octocrab` is the most ergonomic GitHub API client in the Rust ecosystem.

### Scope

**In scope:**
- GitHub OAuth Device Flow login and token persistence (keyring)
- Repo, branch, and folder picker (one-time setup per document)
- Auto-push on every Save (Ctrl+S / Save button)
- Sync status badge in the toolbar (Idle / Syncing / ✓ / ✗ with timestamp)
- GitHub sync config persisted alongside the document (`.octodocs` sidecar file)
- Logout action

**Out of scope:**
- Pulling/fetching changes from GitHub back into the document
- Conflict resolution or merge UI
- Support for non-GitHub hosts (GitLab, Bitbucket)
- Fine-grained commit history browsing
- Branch creation from within the app

### Constraints
- **No local git binary required** — pure HTTP API
- **No Node.js** — consistent with Mermaid rendering decision
- **Async operations must not block the UI thread** — GPUI's `cx.background_executor().spawn()` is the bridge
- Token must survive app restarts (platform keyring)
- GitHub OAuth App client ID must be bundled at compile time (env var `GITHUB_CLIENT_ID`)

### Risks & Concerns

| Risk / Concern | Source | Resolution |
|---|---|---|
| GitHub rate limit (5 000 req/hr authenticated) | Investigation | Each save = 2 calls (get SHA + PUT). At 2 500 saves/hr this is impossible to hit in practice. |
| File deletion on GitHub if local file is renamed | Investigation | Out-of-scope for v1; document that sync is additive only. |
| `keyring` unavailable on minimal Linux installs (no Secret Service daemon) | Investigation | Fall back to `~/.config/octodocs/token` (obfuscated, documented as insecure fallback). |
| GPUI asyncmodel — octocrab is `tokio` async; GPUI runs its own async executor | Investigation | Use `cx.background_executor().spawn()` for the async task, then post result back via weak entity update. GPUI's executor is tokio-compatible. |
| `GITHUB_CLIENT_ID` baked into binary | User | Document need for a GitHub OAuth App registration. For open-source, the client ID is non-secret; only the client secret matters (Device Flow does not need a client secret). |

### Questions Resolved During Process

- *Should we use git2-rs or REST API?* → REST API. Simpler auth, no local disk, no C dep, one HTTP call per save.
- *Where to store GitHub sync config?* → `.octodocs` JSON sidecar next to the `.md` file (or `~/.config/octodocs/default.json` for untitled docs), so the link between document and GitHub destination travels with the file.
- *One GitHub target per document or global?* → Per-document. Each `.md` file can have its own repo/branch/folder target.

---

## Current Implementation Status (2026-02-26)

- ✅ Phases 1–6 complete — auth, discovery, push-on-save, UI panel, sync badge all working
- ✅ Phase 7 replaced: sidecar config replaced by TSV-based binding persistence (`~/.config/octodocs/github_bindings.tsv`)
- ✅ Multi-repo support: multiple bindings with folder-level granularity
- ✅ Token storage: keyring + file fallback (`~/.config/octodocs/token`)
- ✅ Subfolder-aware sync: `relative_sync_path()` preserves directory structure
- ✅ Rename sync: old file deleted + new file pushed on GitHub
- ✅ Recursive initial import via `pull_markdown_files()`
- ⚠️ Phase 8 (automated tests) not yet implemented

---

## Index

- [x] Phase 1: [New `octodocs-github` Crate](#phase-1-new-octodocs-github-crate)
- [x] Phase 2: [OAuth Device Flow & Token Persistence](#phase-2-oauth-device-flow--token-persistence)
- [x] Phase 3: [GitHub API — Repo/Branch/Folder Discovery](#phase-3-github-api--repobranch-folder-discovery)
- [x] Phase 4: [Auto-Push on Save](#phase-4-auto-push-on-save)
- [x] Phase 5: [UI — GitHub Setup Panel](#phase-5-ui--github-setup-panel)
- [x] Phase 6: [UI — Sync Status Badge](#phase-6-ui--sync-status-badge)
- [ ] Phase 7: [Sidecar Config Persistence](#phase-7-sidecar-config-persistence)
- [ ] Phase 8: [Testing](#phase-8-testing)

---

## Investigation Findings

### Codebase Analysis

**Current save flow** ([app_state.rs](../desktop/crates/octodocs-app/src/app_state.rs)):
- `AppState::save(cx)` calls `FileIo::save(&self.document)`, sets `dirty = false`, calls `cx.notify()`.
- Save is synchronous and on the main thread.
- The GitHub push must be **fire-and-forget** from the main thread — dispatched to the background executor, result posted back asynchronously.

**Async bridge pattern in GPUI** (from adabraka-gpui source):
```
cx.background_executor().spawn(async move {
    // tokio-compatible async work
    let result = client.push_file(...).await;
    // post result back
    weak_entity.update(cx, |state, _| state.github_sync_status = ...);
});
```

**Integration points:**
- `AppState::save(cx)` — add GitHub push trigger after successful local save
- `AppState::load_document(doc, cx)` — load `.octodocs` sidecar when opening a file
- `RootView::new(cx)` — create `GitHubPanel` entity and connect to toolbar
- `root.rs` toolbar — add GitHub icon button that opens the setup panel
- StatusBar (via `root.rs`) — add sync status badge on the right side

**No existing async patterns** in the codebase yet — this feature introduces the first use of `cx.background_executor()`.

### External Research

**`octocrab` crate (v0.44):**
- Authenticated client: `Octocrab::builder().personal_token(token).build()`
- List user repos: `octocrab.current().list_repos_for_authenticated_user().send().await`
- List branches: `octocrab.repos(owner, repo).list_branches().send().await`
- List folder contents: `octocrab.repos(owner, repo).get_content().path("folder/").r#ref("branch").send().await`
- Create/update file: `octocrab.repos(owner, repo).update_file(path, message, content, sha).branch(branch).send().await` — the `sha` is the existing file's blob SHA (required for updates, `None` for new files)
- The Contents API handles base64 encoding automatically via octocrab's `content_items` builder

**GitHub OAuth Device Flow:**
- Endpoint: `POST https://github.com/login/device/code` → returns `device_code`, `user_code`, `verification_uri`, `expires_in`, `interval`
- User visits `https://github.com/login/device` and enters `user_code`
- App polls `POST https://github.com/login/oauth/access_token` with `device_code`
- Scopes needed: `repo` (full repo read/write) — sufficient for listing repos, branches, contents, and pushing
- Crate `gh-device-flow` implements this flow for GitHub specifically, with polling + backoff
- No client secret needed for Device Flow — only `client_id` is required

**`keyring` crate (v3):**
- `Entry::new("octodocs", "github-token")` → `entry.set_password(&token)` / `entry.get_password()`
- On Linux uses `org.freedesktop.secrets` (GNOME Keyring / KWallet); on macOS Keychain; on Windows Credential Manager
- Returns `keyring::Error::NoEntry` when first-run (no token stored yet)
- Recommended fallback: create `~/.config/octodocs/credentials.json` (documented as plain-text, user warned)

**Async in GPUI:**
- `cx.background_executor()` returns a `BackgroundExecutor` that wraps the tokio runtime GPUI already runs
- `executor.spawn(future)` returns a `Task<T>` — the task runs concurrently but the result must be posted back to the entity via a weak handle update
- Tasks must be stored (not dropped) to avoid cancellation: `self._sync_task: Option<Task<()>>`

---

## Design Alternatives

### Option A: GitHub Contents API (no local git)

**Description:** Use `octocrab` to call `PUT /repos/{owner}/{repo}/contents/{path}` directly. Get the current file's blob SHA first (needed by GitHub to detect conflicts), then send the new content. Two network calls per save total.

**Pros:**
- No local state to manage (no `.git` directory)
- No `libgit2` C dependency (keeps binary slim)
- Works on any OS with no extra tools
- Auth is a simple Bearer header

**Cons:**
- File history is flat commits (no amend, no squash)
- Binary files require base64 encoding (not an issue for `.md`)
- If two devices save simultaneously, second push will fail with SHA mismatch (v1 limitation — documented)

**Complexity:** Low  
**Risk Level:** Low

### Option B: git2-rs (shallow clone + commit + push)

**Description:** When sync target is configured, clone the target branch to a temp directory, write the file, stage, commit, and push. Clean up the temp clone afterwards.

**Pros:**
- Full git semantics (could support merges in future)
- Commit messages can include author metadata

**Cons:**
- Requires `libgit2` C dependency (~10 MB added to binary)
- Clone is slow even shallow (300–800ms vs ~80ms for API)
- Temp directory management and cleanup needed
- SSH/HTTPS auth callback setup for libgit2 is significantly more complex
- No added value for non-technical users who never see the git history

**Complexity:** High  
**Risk Level:** Medium

### Recommendation

**Chosen Approach:** Option A — GitHub Contents API

The simpler approach is correct for the stated goal. The user never needs to understand branches, SHAs, or clones. Every save just "uploads" the file. Option B adds engineering complexity with no visible benefit for the target user.

---

## Requirements

- [ ] Register a GitHub OAuth App (Settings → Developer Settings → OAuth Apps) with name "OctoDocs" and set callback URL to `http://localhost` (unused for Device Flow)
- [ ] Store the `client_id` of that app in the build environment as `GITHUB_CLIENT_ID` (not secret for Device Flow)
- [ ] Add `tokio` as a dependency to `octodocs-github` with `rt-multi-thread` and `macros` features
- [ ] `keyring` system dependency: Linux requires `libsecret-1-dev` (`sudo apt install libsecret-1-dev`)

References:
- External: [Registering a GitHub OAuth App](https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/creating-an-oauth-app)
- External: [keyring Linux setup](https://docs.rs/keyring/latest/keyring/#linux)

---

## Architecture Considerations

### New crate: `octodocs-github`

All GitHub-related logic lives in a new workspace crate. It has **no GPUI or adabraka-ui dependency** — it is pure async Rust callable from the GPUI app layer.

```
desktop/
└── crates/
    └── octodocs-github/       ← NEW crate
        ├── Cargo.toml
        └── src/
            ├── lib.rs
            ├── auth.rs        ← Device Flow OAuth, keyring storage
            ├── client.rs      ← Octocrab wrapper, token injection
            ├── discovery.rs   ← list repos / branches / folders
            ├── sync.rs        ← push file to branch/folder
            └── config.rs      ← GitHubSyncConfig type, sidecar serialization
```

### Key types (interfaces)

```
GitHubSyncConfig {
    owner:  String,        // "octocat"
    repo:   String,        // "my-notes"
    branch: String,        // "main"
    folder: String,        // "journal/2026" — empty string = repo root
}

SyncStatus {
    Idle,
    Syncing,
    Success { committed_at: SystemTime, sha: String },
    Failed  { message: String },
}

RepoInfo    { owner: String, name: String, default_branch: String }
BranchInfo  { name: String }
FolderEntry { name: String, path: String, is_dir: bool }
```

### SOLID Checklist

**Single Responsibility:**
- `auth.rs` — OAuth device flow + token read/write; nothing else
- `client.rs` — builds authenticated `Octocrab` instance from stored token; no business logic
- `discovery.rs` — reads repo/branch/folder listings; read-only, no mutations
- `sync.rs` — writes exactly one file to GitHub; no auth, no discovery
- `config.rs` — serializes/deserializes `GitHubSyncConfig` from `.octodocs` JSON file
- `AppState` (app crate) — orchestrates: calls sync, holds `SyncStatus`, notifies views

**Open/Closed:**
- `GitHubSyncConfig` can be extended with new fields (e.g., `commit_message_template`) without changing `sync.rs`
- Auth strategy is behind a trait; a PAT (personal access token) path could replace Device Flow without touching callers

**Liskov Substitution:**
- `trait TokenStore { fn get() -> Result<String>; fn set(token: &str) -> Result<()>; }` — `KeyringStore` and `FileStore` (fallback) both satisfy this

**Interface Segregation:**
- `discovery.rs` only exposes read functions; `sync.rs` only exposes write function; consumers only import what they need

**Dependency Inversion:**
- `AppState` holds `Arc<dyn TokenStore>` and calls `octodocs_github::sync::push_file(config, token, path, content)` — never touches `octocrab` directly

### Component interaction diagram

```mermaid
sequenceDiagram
    actor User
    participant Toolbar
    participant AppState
    participant GitHubPanel
    participant octodocs-github

    User->>Toolbar: clicks GitHub icon
    Toolbar->>AppState: open_github_panel()
    AppState->>GitHubPanel: show()

    alt Not authenticated
        GitHubPanel->>octodocs_github::auth: start_device_flow()
        octodocs_github::auth-->>GitHubPanel: { user_code, verification_uri }
        GitHubPanel->>User: shows "Go to github.com/login/device\nEnter: XXXX-XXXX"
        User->>GitHub: enters code in browser
        octodocs_github::auth->>GitHubPanel: token received
        octodocs_github::auth->>keyring: store token
    end

    GitHubPanel->>octodocs_github::discovery: list_repos(token)
    octodocs_github::discovery-->>GitHubPanel: Vec<RepoInfo>
    User->>GitHubPanel: selects repo → branch → folder
    GitHubPanel->>AppState: set_github_config(GitHubSyncConfig)
    AppState->>config.rs: write .octodocs sidecar
```

```mermaid
sequenceDiagram
    actor User
    participant Toolbar (Save)
    participant AppState
    participant BackgroundExecutor
    participant octodocs-github::sync

    User->>Toolbar (Save): Ctrl+S
    Toolbar (Save)->>AppState: save(cx)
    AppState->>FileIo: save to disk
    AppState->>AppState: github_sync_status = Syncing
    AppState->>BackgroundExecutor: spawn(push_file_task)
    BackgroundExecutor->>octodocs_github::sync: get_file_sha(config, path)
    octodocs_github::sync-->>BackgroundExecutor: Option<String> (existing SHA)
    BackgroundExecutor->>octodocs_github::sync: put_file(config, path, content, sha)
    octodocs_github::sync-->>BackgroundExecutor: Result<CommitSha>
    BackgroundExecutor->>AppState: update sync_status (weak handle)
    AppState->>User: sync badge → ✓ "Saved 12:34"
```

---

## Implementation Steps

### Phase 1: New `octodocs-github` Crate

- [ ] Create `desktop/crates/octodocs-github/` with `Cargo.toml` declaring dependencies: `octocrab`, `tokio`, `serde`, `serde_json`, `anyhow`, `base64`
- [ ] Add `octodocs-github` to workspace `members` in `desktop/Cargo.toml`
- [ ] Add `octodocs-github` as a dependency of `octodocs-app`
- [ ] Define all public types in `lib.rs`: `GitHubSyncConfig`, `SyncStatus`, `RepoInfo`, `BranchInfo`, `FolderEntry`

### Phase 2: OAuth Device Flow & Token Persistence

- [ ] Add `gh-device-flow` (or implement manually with `reqwest`) and `keyring = "3"` to `octodocs-github/Cargo.toml`
- [ ] Implement `auth::start_device_flow(client_id) -> DeviceFlowHandle` — calls `POST https://github.com/login/device/code`, returns `{ user_code, verification_uri, device_code, interval }`
- [ ] Implement `auth::poll_for_token(handle) -> impl Stream<Item = PollResult>` — polls `POST https://github.com/login/oauth/access_token` every `interval` seconds, yields `Pending` or `Token(String)`
- [ ] Implement `KeyringStore::set(token)` and `KeyringStore::get() -> Option<String>` using `keyring::Entry::new("octodocs", "github-token")`
- [ ] Implement `FileStore` fallback for systems without a keyring daemon — writes to `~/.config/octodocs/token` (plain text, user-warned via UI label)
- [ ] Expose `auth::get_stored_token() -> Option<String>` — tries keyring first, then file fallback

### Phase 3: GitHub API — Repo/Branch/Folder Discovery

- [ ] Implement `client::build(token: &str) -> Octocrab` — creates authenticated instance
- [ ] Implement `discovery::list_repos(token) -> anyhow::Result<Vec<RepoInfo>>` — calls `octocrab.current().list_repos_for_authenticated_user().type_("all").send().await`, maps to `RepoInfo`
- [ ] Implement `discovery::list_branches(token, owner, repo) -> anyhow::Result<Vec<BranchInfo>>`
- [ ] Implement `discovery::list_folder(token, owner, repo, branch, path) -> anyhow::Result<Vec<FolderEntry>>` — calls `get_content().path(path).r#ref(branch).send()`, filters to directories + recurse marker
- [ ] All functions are `async fn` returning `anyhow::Result`

### Phase 4: Auto-Push on Save

- [x] Implement `sync::get_file_sha(token, config, filename) -> anyhow::Result<Option<String>>` — calls `GET /repos/{owner}/{repo}/contents/{folder}/{filename}?ref={branch}`, extracts `sha` field; returns `None` for 404
- [x] Implement `sync::push_file(token, config, filename, content) -> anyhow::Result<String>` — calls `update_file(path, message, content_base64, sha)`, returns new commit SHA
  - Commit message template: `"OctoDocs: update {filename}"` with UTC timestamp
- [x] In `AppState::save(cx)`:
  1. Call `FileIo::save` (existing)
  2. If `self.github_config.is_some()` and token is loaded:
     - Set `self.github_sync_status = SyncStatus::Syncing` + `cx.notify()`
     - Spawn `cx.spawn()` capturing weak entity + config + token + content
     - On success: weak update to `SyncStatus::Success { ... }`
     - On failure: weak update to `SyncStatus::Failed { message }`
- [x] Store the `Task<()>` in `AppState::_sync_task: Option<Task<()>>` to prevent premature cancellation

### Phase 5: UI — GitHub Setup Panel

- [x] Create `views/github_panel.rs` — a modal-style overlay view or side panel, shown/hidden via `AppState::github_panel_open: bool`
- [x] Panel states (rendered as a vertical flow):
  - **Unauthenticated**: "Connect to GitHub" button → triggers device flow → shows code card with URL + `user_code` + progress spinner → on success transitions to Authenticated
  - **Authenticated**: avatar + username display, "Disconnect" button
  - **Repo selector**: searchable list of `RepoInfo` items (populated via `discovery::list_repos`)
  - **Branch selector**: dropdown of `BranchInfo` items for selected repo
  - **Folder selector**: nested expandable tree of `FolderEntry` items (lazy-loaded per expand)
  - **Confirm button**: "Sync this document here" → calls `AppState::set_github_config`
- [x] Loading states: spinner placeholder while async lists are fetching
- [x] All async calls are dispatched via `cx.spawn()` with weak entity updates

### Phase 6: UI — Sync Status Badge

- [x] Add a `SyncBadge` component (inline in `root.rs` or a small struct) rendered in the toolbar right section
- [x] States:
  | `SyncStatus` | Badge display |
  |---|---|
  | `Idle` (no config) | GitHub icon, muted color, tooltip "Not connected to GitHub" |
  | `Idle` (config set) | GitHub icon, normal color, tooltip "Synced: {repo}/{branch}" |
  | `Syncing` | spinner icon, accent color |
  | `Success` | checkmark icon, success color, tooltip "Last synced {time}" |
  | `Failed` | warning icon, error color, tooltip "Sync failed: {message}" |
- [x] Clicking the badge opens the GitHub setup panel (same as toolbar button)

### Phase 7: Sidecar Config Persistence

**Status: REPLACED** — The original plan called for `.octodocs` JSON sidecar files next to each `.md` file. This was replaced by a centralized TSV binding file at `~/.config/octodocs/github_bindings.tsv` which maps `local_root` → `owner/repo/branch/folder`. This approach is simpler and avoids littering the user's workspace with sidecar files.

- [x] Binding persistence via `github_bindings.tsv` (implemented in `app_state.rs`)
- [x] Bindings survive app restart
- [x] Multiple bindings supported (one per local folder)
- [x] UI state persistence via `ui_state.tsv` (sidebar open/closed, active binding index)

### Phase 8: Testing

- [ ] Unit tests for `config.rs`: serialize/deserialize `GitHubSyncConfig`, round-trip through sidecar file
- [ ] Unit tests for `auth.rs` token store: mock keyring, assert store/retrieve
- [ ] Integration test (with mock HTTP server via `wiremock`): `push_file` sends correct base64 content and SHA, handles 404 (new file) and 200 (existing file) branches
- [ ] Integration test: `list_repos`, `list_branches`, `list_folder` parse octocrab responses correctly
- [ ] Manual smoke test: authenticate with a test GitHub account, connect a public repo, save document, verify commit appears in GitHub

---

## Testing Strategy

- [ ] `cargo test -p octodocs-github` — no GPUI, no UI required
- [ ] `wiremock` crate for mocking GitHub API responses in integration tests
- [ ] `tempfile` crate for sidecar file tests
- [ ] Manual end-to-end: full OAuth flow → repo selection → save → verify commit on GitHub web

References:
- External: [wiremock-rs](https://docs.rs/wiremock)
- Internal: [desktop/crates/octodocs-core/src/](../desktop/crates/octodocs-core/src/) — pattern for unit tests

---

## Potential Risks & Mitigations

| Risk | Impact | Mitigation |
|---|---|---|
| Simultaneous save from two devices → SHA mismatch → API 409 | Low | Catch 409, show "Sync conflict" in badge. Out-of-scope to resolve automatically in v1. |
| `keyring` not available (headless/CI Linux) | Medium | `FileStore` fallback with a UI warning label |
| Device Flow `user_code` expires (15 min) | Low | Show countdown timer; re-start flow button |
| `GITHUB_CLIENT_ID` env var not set at compile time | Medium | `env!()` macro will fail at compile time with a clear error message |
| Long repo lists (1000+ repos) pagination | Low | `octocrab` supports `.per_page(100).page(n)` — implement pagination with "Load more" button |

---

## Dependencies

**New crates (`octodocs-github/Cargo.toml`):**
- `octocrab = "0.44"` — GitHub API client ([docs](https://docs.rs/octocrab))
- `keyring = "3"` — platform credential storage ([docs](https://docs.rs/keyring))
- `gh-device-flow = "0.3"` — GitHub OAuth device flow ([crate](https://github.com/jakewilkins/gh-device-flow))
- `tokio = { version = "1", features = ["rt-multi-thread", "macros"] }` — async runtime
- `serde = { version = "1", features = ["derive"] }` — config serialization
- `serde_json = "1"` — sidecar file format
- `base64 = "0.22"` — encoding file content for GitHub API
- `anyhow = "1"` — error handling (already used in workspace)

**Test-only crates:**
- `wiremock = "0.6"` — HTTP mock server for integration tests

**System (Linux):**
- `libsecret-1-dev` — required by the `keyring` crate on Linux: `sudo apt install libsecret-1-dev`

**External setup (one-time):**
- GitHub OAuth App registration (free); yields `GITHUB_CLIENT_ID` build env var

---

## Success Criteria

- [x] User can authenticate with GitHub in-app with no terminal and no redirect URL
- [x] User can select any repo, branch, and folder they have write access to from a picker UI
- [x] Every Save (Ctrl+S) automatically pushes the document to the configured GitHub location
- [x] A commit appears in the GitHub repository after each save, with a clear commit message
- [x] Sync target config survives app restart (TSV bindings + keyring)
- [x] The sync badge accurately reflects the current sync state (5 contextual states)
- [ ] `cargo test -p octodocs-github` passes with no GPUI or UI dependency
- [x] App window remains responsive during sync (no UI freeze)
