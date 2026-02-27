# CI Multi-Platform Builds & Git Pull on Open Implementation Plan

## Overview

Two independent features:

1. **CI/CD builds** — GitHub Actions workflows that compile OctoDocs for Linux (primary), macOS, and Windows (both experimental), upload release binaries to GitHub Releases, and update the README with per-OS install instructions.
2. **Git Pull on file open** — When a user clicks a file in the sidebar, fetch the latest version from GitHub before loading it into the editor. All merge conflicts are resolved automatically by the "remote wins" strategy (last commit wins).

References:
- Internal: [desktop/crates/octodocs-github/src/sync.rs](desktop/crates/octodocs-github/src/sync.rs), [desktop/crates/octodocs-github/src/discovery.rs](desktop/crates/octodocs-github/src/discovery.rs), [desktop/crates/octodocs-app/src/app_state.rs](desktop/crates/octodocs-app/src/app_state.rs)
- External: [GPUI](https://www.gpui.rs/), [Zed CI reference](https://github.com/zed-industries/zed/blob/main/.github/workflows/release.yml), [Zed Linux deps script](https://github.com/zed-industries/zed/blob/main/script/linux)

---

## Design Alignment

### Chosen Direction

- **CI**: GitHub Actions with per-platform parallel jobs, triggered on `v*` tag push. Linux is the primary/blocking target; macOS and Windows are best-effort (non-blocking on release). No code signing, notarization, or sccache in v1.
- **Git Pull**: Pull-on-open via the GitHub Contents API (same transport already used for push). Remote content unconditionally overwrites local on open. No conflict UI in v1.

### Scope

**In scope:**
- GitHub Actions workflow: lint + test + build for Linux x86_64, macOS aarch64/x86_64, Windows x86_64
- Release artifacts uploaded to GitHub Releases on tag push
- `cargo clippy --workspace` as a required gate
- Per-OS README install section with pre-built binary download instructions
- New `pub fn pull_file(...)` in `octodocs-github`
- Async pull triggered in `open_file_from_sidebar` in `AppState`
- Sync status badge updated during pull (reuses `SyncStatus::Syncing` / `Success` / `Failed`)
- "Remote wins" conflict resolution: remote content overwrites local file on disk, then loads into editor

**Out of scope:**
- Code signing / notarization (macOS Gatekeeper, Windows SmartScreen) — deferred; **no free option exists** for either platform (macOS: $99/year Apple Developer Program; Windows: $200–500/year Authenticode cert or ~$10–50/month Azure Trusted Signing, US/Canada entities only). Users will see OS warnings and must manually allow the binary. README must document the workaround for each OS.
- DMG packaging for macOS — deferred (plain `.tar.gz` or direct `.app` bundle)
- Windows installer (`.msi`/NSIS) — deferred
- Incremental pull / diff-based merge — deferred
- Pull-on-background-timer / polling — deferred
- `cargo-bundle` or `tauri`-based packaging — deferred

### Constraints
- GPUI uses **Metal** on macOS and **DirectX 12** on Windows; neither has been tested by the team. macOS/Windows jobs must not block Linux releases.
- GPUI requires **Nightly Rust** (pinned via `desktop/rust-toolchain.toml`). All CI jobs must use `rustup override` / toolchain file.
- Windows build requires MSVC toolchain (`windows-latest` runner with `msvc` target); vendored `adabraka-gpui` patches may contain Linux/Metal-only assumptions.
- The pull feature must be fully non-blocking — if GitHub is unreachable or the file is not found remotely, the local copy opens normally with no error message to the user beyond the status badge.

### Risks & Concerns

| Risk / Concern | Source | Resolution |
|----------------|--------|------------|
| macOS Metal or Windows DX12 compilation fails on first CI run | Investigation (untested platforms) | Jobs are `continue-on-error: true` and non-blocking on releases; failures are visible in Actions |
| `adabraka-gpui` Vulkan padding patch causes DX12 compile errors on Windows | Investigation | Patch is in `scene.rs`/`window.rs`; likely conditional via `#[cfg(target_os)]`; CI will reveal issues |
| Pull-on-open introduces perceptible latency on slow connections | User input | Pull runs as a background task; editor opens with local content immediately while pull is in flight (progressive pattern) |
| GitHub API rate limit hit when many files opened quickly | Investigation | Contents API has 5,000 req/h with token, which is sufficient for normal use |
| Two concurrent tasks (pull + in-flight push) race on the same file | Investigation | `_sync_task` is a single `Option<Task<()>>`; new pull task will supercede the old one, same as existing push logic |

### Questions Resolved During Process
- **Should Windows block releases?** → No. Neither macOS nor Windows has been tested. Both are experimental/informational in v1.
- **Is a DMG or installer needed for v1?** → No. A plain binary tar.gz is sufficient for the first release; packaging deferred.
- **Should pull replace the local file before or after load?** → Before: write to disk first, then `FileIo::open` loads it (consistent with existing disk-first architecture).
- **What if the file doesn't exist on GitHub yet?** → 404 response → skip pull, open locally. This covers newly-created local files not yet pushed.

---

## Index

- [x] Phase 1: [GitHub Actions CI Workflow](#phase-1-github-actions-ci-workflow)
- [x] Phase 2: [Release Artifact Packaging](#phase-2-release-artifact-packaging)
- [x] Phase 3: [README Install Instructions](#phase-3-readme-install-instructions)
- [x] Phase 4: [Pull File API in octodocs-github](#phase-4-pull-file-api-in-octodocs-github)
- [x] Phase 5: [Pull on Open in AppState](#phase-5-pull-on-open-in-appstate)

---

## Investigation Findings

### Codebase Analysis

**Existing push/sync pattern** ([app_state.rs](desktop/crates/octodocs-app/src/app_state.rs)):
- `trigger_github_sync` spawns `cx.spawn(async move |this, cx| { cx.background_executor().spawn(...) })` — the pull implementation must follow the exact same async pattern.
- `_sync_task: Option<Task<()>>` is the single task slot; assigning a new task cancels the previous one (GPUI semantics).
- `SyncStatus` enum (`Idle` / `Syncing` / `Success` / `Failed`) is already shown in the status bar — the pull will reuse these variants.

**File open hook** ([app_state.rs#L406](desktop/crates/octodocs-app/src/app_state.rs#L406)):
- `open_file_from_sidebar(path, cx)` is the single entry point for all sidebar clicks.
- It reads from disk via `octodocs_core::FileIo::open(&path)` then calls `load_document`.
- Pull must be injected here: resolve binding → spawn pull task → on success write to disk → then `load_document`.

**GitHub Contents API** ([discovery.rs](desktop/crates/octodocs-github/src/discovery.rs)):
- `fetch_file_content` (currently `fn`, private) does exactly what's needed: GET `/repos/{owner}/{repo}/contents/{path}?ref={branch}`, base64-decodes, returns `String`.
- It returns a 404 via `StatusCode::NOT_FOUND` — same handling as `get_file_sha` in `sync.rs`.
- Needs to be promoted to `pub fn pull_file(token, config, filename) -> Result<Option<String>>` returning `None` on 404.

**CI starting point**: No `.github/` directory exists in the repo. All workflows are created from scratch.

### External Research

**GPUI platform matrix** (from Zed source and `gpui.rs`):
- Linux: Vulkan (via `ash`) + Wayland/X11. Needs: `libvulkan1`, `libwayland-dev`, `libxkbcommon-x11-dev`, `libfontconfig-dev`, `libasound2-dev`, `libxcb1-dev`, `libssl-dev`.
- macOS: Metal. No extra system deps; requires macOS 12+ runner. `macos-latest` (GitHub-hosted) = macOS 14 Sonoma on Apple Silicon.
- Windows: DirectX 12. Requires `windows-latest` runner (Windows Server 2022) with MSVC; no extra system installs.

**Zed CI pattern** (reviewed at [release.yml](https://github.com/zed-industries/zed/blob/main/.github/workflows/release.yml)):
- Uses per-platform parallel jobs with `needs:` dependency graph: tests → clippy → bundle → upload.
- Uses `namespacelabs/nscloud-cache-action` for Rust cache (Namespace-specific). We use the standard `Swatinem/rust-cache` action instead.
- Bundles Linux as `.tar.gz`, macOS as `.dmg` (with codesign + notarytool), Windows as `.exe` (with Azure signing). We skip signing entirely for v1.
- `cargo nextest` for tests — we can use standard `cargo test` to avoid extra install steps.

**Rust cache for GitHub Actions**: `Swatinem/rust-cache@v2` caches `~/.cargo/registry`, `~/.cargo/git`, and `target/` keyed on `Cargo.lock` + OS + toolchain. Significantly reduces build times.

References:
- [Zed script/linux](https://github.com/zed-industries/zed/blob/main/script/linux) — authoritative Linux dep list
- [Swatinem/rust-cache](https://github.com/Swatinem/rust-cache) — standard Rust caching action
- [actions/upload-artifact](https://github.com/actions/upload-artifact) — artifact upload
- [softprops/action-gh-release](https://github.com/softprops/action-gh-release) — GitHub Release upload

---

## Architecture Considerations

### CI Workflow Architecture

```
Trigger: push tag v*  OR  push to main/builds
│
├── job: clippy (ubuntu-latest)          [blocking]
│     └── cargo clippy --workspace
│
├── job: test (ubuntu-latest)            [blocking]
│     └── sudo apt install deps
│         cargo test -p octodocs-core
│
├── job: build-linux (ubuntu-latest)     [needs: clippy, test] [blocking]
│     └── sudo apt install deps
│         cargo build --release -p octodocs-app
│         tar.gz → upload artifact
│
├── job: build-macos (macos-latest)      [continue-on-error: true]
│     └── cargo build --release -p octodocs-app
│         tar.gz → upload artifact
│
└── job: build-windows (windows-latest)  [continue-on-error: true]
      └── cargo build --release -p octodocs-app
          zip → upload artifact
          
On tag push only:
└── job: release (needs: build-linux)    [blocking on Linux only]
      └── softprops/action-gh-release
          upload: linux tar.gz + any available macOS/Windows artifacts
```

### Git Pull on Open — Sequence Diagram

```
User clicks file in sidebar
         │
         ▼
open_file_from_sidebar(path, cx)
         │
         ├─ dirty? → show unsaved prompt (unchanged)
         │
         ▼
find_sync_binding(path)
         │
    No binding ──────────────────────────────────────► FileIo::open → load_document
         │
    Binding found
         │
         ▼
get_stored_token()
         │
    No token ────────────────────────────────────────► FileIo::open → load_document
         │
    Token found
         │
         ▼
github_sync_status = Syncing  →  cx.notify()
         │
         ▼
cx.spawn → background_executor.spawn
    octodocs_github::pull_file(token, config, filename)
         │
         ├─ Ok(None) [404, file not on GitHub yet]
         │       └──► FileIo::open → load_document
         │             github_sync_status = Idle
         │
         ├─ Ok(Some(content))
         │       └──► std::fs::write(path, content)  [overwrite local]
         │             FileIo::open → load_document
         │             github_sync_status = Success { sha, committed_at }
         │
         └─ Err(e)
                 └──► FileIo::open → load_document  [open stale local copy]
                       github_sync_status = Failed { message }
```

### SOLID Checklist

- **Single Responsibility**: `pull_file` in `octodocs-github` only fetches; `AppState::pull_and_open_file` only orchestrates; `load_document` only loads. Each function has one job.
- **Open/Closed**: `open_file_from_sidebar` is extended by delegating to `pull_and_open_file`; no existing callers or existing logic modified.
- **Liskov Substitution**: `pull_file` returns `Result<Option<String>>` — the same contract as `get_file_sha`, consistent with the existing library surface.
- **Interface Segregation**: The new `pull_file` function is a small, focused addition to `lib.rs`'s public surface. It does not modify any existing exports.
- **Dependency Inversion**: `AppState` depends on the `octodocs_github` crate (abstract module), not on raw HTTP details. Transport is encapsulated in `client::build(token)`.

---

## Requirements

- [ ] GitHub repository must have Actions enabled (it does — `builds` branch is active).
- [ ] `GITHUB_TOKEN` secret is automatically provided by Actions — no manual secret needed for release uploads with `softprops/action-gh-release`.
- [ ] Linux runner needs `sudo` rights to install apt packages (standard on `ubuntu-latest`).
- [ ] macOS runner: `macos-latest` = macOS 14 (arm64). For x86_64 builds: use `macos-13`.
- [ ] Windows runner: `windows-latest` with MSVC — Rust default target is `x86_64-pc-windows-msvc`.
- [ ] Nightly Rust: `rustup toolchain install nightly` + `rustup override set nightly` per job, or rely on `rust-toolchain.toml` being picked up by `rustup` after install.

---

## Implementation Steps

### Phase 1: GitHub Actions CI Workflow

- [ ] Create `.github/workflows/ci.yml` — triggers on push to `main` and `builds` branches, and on all PRs. Jobs: `clippy` + `test` (Linux only).
- [ ] Create `.github/workflows/release.yml` — triggers on `push: tags: ['v*']`. Jobs: `clippy`, `test`, `build-linux` (blocking), `build-macos`, `build-windows` (both `continue-on-error: true`), `release`.
- [ ] Linux job steps: `actions/checkout`, `Swatinem/rust-cache`, `sudo apt-get install` (Vulkan + Wayland + fontconfig + libxcb + libxkbcommon + libssl + libasound2), `rustup toolchain install nightly`, `cargo build --release -p octodocs-app`, create `octodocs-linux-x86_64.tar.gz` with the binary, `actions/upload-artifact`.
- [ ] macOS job steps: `actions/checkout`, `Swatinem/rust-cache`, `rustup toolchain install nightly`, `cargo build --release -p octodocs-app` (Metal backend — no extra system deps needed), create `octodocs-macos-aarch64.tar.gz`, `actions/upload-artifact`.
- [ ] Windows job steps: `actions/checkout`, `Swatinem/rust-cache`, `rustup toolchain install nightly`, `cargo build --release -p octodocs-app`, create a zip of the `.exe`, `actions/upload-artifact`. Use `shell: pwsh` for all run steps on Windows.
- [ ] `release` job: `needs: [build-linux]`, uses `softprops/action-gh-release@v2` to create a GitHub Release and upload all available artifacts. Use `if: always()` for downloading macOS/Windows artifacts so missing ones don't block the release.

### Phase 2: Release Artifact Packaging

- [ ] Linux: `tar -czf octodocs-linux-x86_64.tar.gz -C target/release octodocs-app` — single binary, no extra assets needed for v1.
- [ ] macOS: same plain tar of the binary. Note in Release description that macOS users need to run `xattr -cr ./octodocs-app` after download, or go to System Settings → Privacy & Security → "Open Anyway" (no Gatekeeper bypass in v1 — no free signing option exists).
- [ ] Windows: `Compress-Archive -Path target\release\octodocs-app.exe -DestinationPath octodocs-windows-x86_64.zip`. Note in Release description that Windows users will see a SmartScreen "Windows protected your PC" warning and must click "More info → Run anyway" (no Authenticode signing in v1 — no free signing option exists).
- [ ] Add release body template to workflow describing download instructions per OS.

### Phase 3: README Install Instructions

- [ ] Replace the current "Requirements / Build & Run" section in [README.md](README.md) with three sub-sections: **Linux**, **macOS**, **Windows**.
- [ ] Each section: (a) "Download pre-built binary" — links to latest GitHub Release artifact; (b) "Build from source" — `cargo build --release` with OS-specific system dep commands.
- [ ] Linux from-source deps: `sudo apt install libxcb1-dev libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libvulkan-dev vulkan-validationlayers libsecret-1-dev libfontconfig-dev libasound2-dev libssl-dev`.
- [ ] macOS from-source: no extra system deps (Xcode command line tools + `rustup` only). Add note: pre-built binary will trigger Gatekeeper; run `xattr -cr ./octodocs-app` or allow via System Settings → Privacy & Security.
- [ ] Windows from-source: no extra system deps beyond `rustup` with nightly (MSVC toolchain auto-installed). Add note: pre-built `.exe` will show SmartScreen warning; click "More info → Run anyway".
- [ ] Update "Current Status → Planned" section to mark macOS/Windows builds as in-progress.

### Phase 4: Pull File API in octodocs-github

- [ ] In [discovery.rs](desktop/crates/octodocs-github/src/discovery.rs): rename `fetch_file_content` → keep it as the private implementation; add `pub fn pull_file(token: &str, config: &GitHubSyncConfig, filename: &str) -> Result<Option<String>>` that calls into the existing logic and maps 404 to `Ok(None)`.
- [ ] The function resolves the full path via `build_repo_path(config, filename)` (function defined in `sync.rs` — either duplicate the trivial logic or move it to a shared `util.rs` module).
- [ ] In [lib.rs](desktop/crates/octodocs-github/src/lib.rs): add `pub use discovery::pull_file;` to the crate's public surface.
- [ ] The function signature mirrors `push_file`: takes `(token, config, filename)` and returns a `Result`. On 404: `Ok(None)`. On success: `Ok(Some(content_string))`.

### Phase 5: Pull on Open in AppState

- [ ] In [app_state.rs](desktop/crates/octodocs-app/src/app_state.rs): add method `fn pull_and_open_file(&mut self, path: PathBuf, cx: &mut Context<AppState>)`.
- [ ] Inside `pull_and_open_file`: resolve binding and token using the same guard pattern as `trigger_github_sync`. If either is missing, fall through to `FileIo::open` + `load_document` (graceful degradation).
- [ ] Spawn a background task (same `cx.spawn` + `background_executor.spawn` pattern as `trigger_github_sync`) that calls `octodocs_github::pull_file`.
- [ ] On `Ok(Some(content))`: `std::fs::write(&path, &content)` to update the local file, then call `load_document(FileIo::open(&path)?)` inside the task callback.
- [ ] On `Ok(None)`: open locally, set `github_sync_status = SyncStatus::Idle`.
- [ ] On `Err(e)`: open locally from disk, set `github_sync_status = SyncStatus::Failed { message }`.
- [ ] Update `open_file_from_sidebar` to call `self.pull_and_open_file(path, cx)` in place of the direct `FileIo::open` calls (for the non-dirty, non-reload paths). The dirty-check + unsaved prompt path is unchanged.

---

## Testing Strategy

- [ ] **Unit — `pull_file` API**: add a test in `octodocs-github` that mocks a 404 response (using `httpmock` or similar) and asserts `Ok(None)` is returned.
- [ ] **Integration — pull on open**: manual testing with a real GitHub repo: (a) edit a file directly on GitHub, then open it in OctoDocs → verify remote content appears; (b) open a file with no GitHub binding → verify local content loads without error.
- [ ] **CI smoke tests**: verify all three platform builds produce an artifact in GitHub Actions. macOS and Windows failures are expected initially and are tracked in the release notes.
- [ ] **Edge cases**: file created locally but not yet pushed (404 → local load), no network (request error → local load), token expired (auth error → local load + Failed status).

References:
- Internal: [desktop/crates/octodocs-core](desktop/crates/octodocs-core) — existing test patterns
- External: [httpmock](https://docs.rs/httpmock), [mockito](https://docs.rs/mockito)

---

## Potential Risks & Mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| `adabraka-gpui` Vulkan patches cause Windows DX12 compile errors | Medium | `continue-on-error: true` on Windows job; investigate patches if it fails |
| macOS binary blocked by Gatekeeper quarantine | Medium | Document `xattr -cr ./octodocs-app` or System Settings → "Open Anyway" in README; paid signing ($99/yr Apple Developer) deferred to v2 |
| Windows SmartScreen blocks unsigned `.exe` | Medium | Document "More info → Run anyway" in README; paid Authenticode cert (~$200–500/yr) or Azure Trusted Signing (~$10–50/mo, US/CA only) deferred to v2 |
| Pull task races with in-flight push task | Low | Both use the same `_sync_task` slot; new assignment cancels previous; worst case: stale content briefly shown, corrected on next open |
| `std::fs::write` fails (permissions, disk full) | Low | Return `Err` → fall through to local load path, set `Failed` status |

---

## Dependencies

**New Rust dependencies**: None. `pull_file` uses the existing `reqwest` client from `client::build`.

**New GitHub Actions dependencies**:
- `Swatinem/rust-cache@v2` — Rust build cache
- `softprops/action-gh-release@v2` — Release creation and asset upload

References:
- [Swatinem/rust-cache](https://github.com/Swatinem/rust-cache)
- [softprops/action-gh-release](https://github.com/softprops/action-gh-release)

---

## Success Criteria

- [ ] A push to a `v*` tag triggers Actions, builds the Linux binary, and produces a downloadable artifact in GitHub Releases automatically.
- [ ] macOS and Windows jobs run in parallel and their success/failure status is visible in the Actions UI without blocking the Linux release.
- [ ] README has clear "Download" links and per-OS "Build from source" instructions.
- [ ] Opening a file that was edited directly on GitHub loads the latest remote content into the editor (verified manually).
- [ ] Opening a file with no internet connection or no GitHub binding opens the local copy with no crash and no error dialog (only the status badge shows `Failed` if applicable).
