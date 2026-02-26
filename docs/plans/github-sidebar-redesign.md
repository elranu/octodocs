# GitHub Sidebar Redesign Implementation Plan

## Overview

Replace the current GitHub overlay modal with a **toggleable right sidebar** that contains a repo selector and a local file explorer. Authentication becomes a focused toolbar-triggered modal. Adding new repo mappings opens a dedicated wizard modal. The sidebar gives a persistent, spatial view of the GitHub-connected file tree.

References:
- Internal: [desktop/crates/octodocs-app/src/views/root.rs](desktop/crates/octodocs-app/src/views/root.rs)
- Internal: [desktop/crates/octodocs-app/src/views/github_panel.rs](desktop/crates/octodocs-app/src/views/github_panel.rs)
- Internal: [desktop/crates/octodocs-app/src/app_state.rs](desktop/crates/octodocs-app/src/app_state.rs)

---

## Design Alignment

### Chosen Direction

- **Approach**: Right toggleable sidebar + two separate modals (auth + repo-add)
- **Rationale**: Clean separation of concerns — sidebar is ambient/spatial, auth is a one-time event, repo-add is a wizard. This mirrors VS Code's Source Control sidebar pattern.

### Scope

**In scope:**
- New `GithubSidebar` view (repo dropdown + file explorer)
- `GithubAuthModal` — stripped down from current panel, auth only
- `RepoAddModal` — extracted repo/branch/folder wizard, triggered from sidebar "+" 
- File explorer: browse `.md` files and folders under the binding's `local_root`
- Create new `.md` file or folder inline in the explorer
- Click `.md` file to open it (with save-guard: auto-save if path exists, prompt if not)
- Independent sidebar toggle button in toolbar
- GitHub toolbar button opens `GithubAuthModal` only
- "+" in sidebar triggers auth first if no token, then repo-add wizard

**Out of scope:**
- Tabs / multi-document buffers
- Recursive file watcher (no inotify for now — explorer refreshes on open/create)
- Renaming or deleting files from the sidebar
- Sync status per individual file in explorer (future)

### Constraints
- No new crate dependencies — use `std::fs` for directory reading
- GPUI rendering model: state changes drive re-render; no async FS calls needed for local listing
- Must not break existing `trigger_github_sync` or `save` flows in `AppState`

### Risks & Concerns

| Risk / Concern | Source | Resolution |
|---|---|---|
| Long file trees overflow sidebar | Investigation | Scrollable `uniform_list` or simple scrollable `div` |
| "+" clicked without auth races with auth modal | User input | Auth modal sets a `pending_action = AddRepo` flag; on auth success, auto-triggers repo-add |
| `rfd::FileDialog` blocks UI thread | Investigation | Already handled; keep same pattern (call from event handler, not `cx.spawn`) |
| Sidebar width narrows editor uncomfortably | Investigation | Fixed 260px sidebar; content area shrinks but remains functional |

### Questions Resolved During Process
- Should clicking a file open it or just show status? → **Open in main editor**
- Should switching auto-save? → **Auto-save if file has path; prompt Save/Discard if unsaved new doc**
- Should sidebar be always-visible or toggleable? → **Toggleable, with its own independent toolbar button**
- Should GitHub toolbar button open sidebar or auth? → **GitHub button = auth modal only; sidebar has its own toggle**

---

## Current Implementation Status (2026-02-26)

- ✅ All 7 phases complete and building clean
- ✅ Sidebar toggle redesigned: vertical rail (closed) + collapse button in sidebar header (open)
- ✅ First-run onboarding: forces GitHub auth + repo selection (no example markdown)
- ✅ Initial markdown import: recursively pulls all `.md` files from GitHub on first repo mapping
- ✅ Import summary with auto-hide (6 seconds) in status bar
- ✅ Subfolder-aware sync paths (files in subdirectories preserve relative paths in GitHub)
- ✅ Contextual sync status badge (5 states: Not configured / Unsynced changes / Ready to sync / Synced / Synced other file)
- ✅ File rename triggers remote rename on GitHub
- ✅ `make reset-state` for clean-state testing

---

## Index

- [x] Phase 1: [AppState Extensions](#phase-1-appstate-extensions)
- [x] Phase 2: [GithubAuthModal (stripped panel)](#phase-2-githubauthmodal-stripped-panel)
- [x] Phase 3: [RepoAddModal (wizard extracted from panel)](#phase-3-repoaddmodal-wizard-extracted-from-panel)
- [x] Phase 4: [GithubSidebar — Repo Selector](#phase-4-githubsidebar--repo-selector)
- [x] Phase 5: [GithubSidebar — File Explorer](#phase-5-githubsidebar--file-explorer)
- [x] Phase 6: [Root Layout Integration](#phase-6-root-layout-integration)
- [x] Phase 7: [File Open Guard (Save/Discard prompt)](#phase-7-file-open-guard-savediscard-prompt)
- [x] Phase 8: [Post-Plan Enhancements](#phase-8-post-plan-enhancements)

---

## Investigation Findings

### Codebase Analysis

**Current `GitHubPanel` responsibilities (detected in [github_panel.rs](desktop/crates/octodocs-app/src/views/github_panel.rs)):**
- Authentication: `Loading → Unauthenticated → DeviceFlow → RepoSelect`
- Repo/branch/folder wizard: `RepoSelect → BranchSelect → FolderSelect → Confirm`
- Local folder picker + default path logic
- All rendered as a single floating overlay injected via `.when(github_panel_open, ...)`

**Current `AppState` fields relevant to this plan ([app_state.rs](desktop/crates/octodocs-app/src/app_state.rs)):**
- `github_bindings: Vec<GitHubSyncBinding>` — the multi-repo mappings
- `github_panel_open: bool` — will be split into separate flags
- `dirty: bool` and `document.path: Option<PathBuf>` — used for save-guard logic
- `load_document(doc, cx)` — the existing method for loading a file into the editor

**Root layout ([root.rs](desktop/crates/octodocs-app/src/views/root.rs)):**
- Uses flex column: toolbar → content area → status bar
- Content area is a single `AnyElement` swapped per `ViewMode`
- GitHub panel injected as overlay via `.when()` at the end of the root div
- Sidebar will require the content area to become a flex-row: editor | sidebar

**Existing icon assets ([assets/icons/](desktop/crates/octodocs-app/assets/icons/)):**
- Available: `folder.svg`, `file-plus.svg`, `chevron-right.svg`, `chevron-down.svg`, `check.svg`, `cloud.svg`
- Needed additions: `sidebar.svg` (or reuse existing), `plus.svg`, `file.svg`

**`adabraka-ui` components confirmed available:**
- `Button`, `IconButton`, `Icon`, `Spinner`, `Input`, `InputState` — all used today
- No native tree/collapsible component — file tree built from nested `div`s manually

### External Research

No external dependencies required. The file explorer uses `std::fs::read_dir` (synchronous, acceptable for local FS at this scale). GPUI's flex layout handles the sidebar panel width natively.

---

## Architecture Considerations

### Layout After Redesign

```
┌─ Toolbar ─────────────────────────────────────────────────────────────┐
│ [new] [open] [save] [save-as] │ [bold] [italic] [h1] [h2] [code]     │
│ [theme] │ [github (auth)] [sidebar-toggle]                            │
├── GitHub Sidebar (260px) ───┬─ Content Area ──────────────────────────┤
│  ┌ Repo ──────────────── ┐  │                                         │
│  │ [repo dropdown    ▾] [+] │  BlockEditorPane / EditorPane /         │
│  └──────────────────────┘  │  Split view                             │
│  ┌ Files ─────────────── ┐  │                                         │
│  │ 📁 docs/              │  │                                         │
│  │   📄 intro.md         │  │                                         │
│  │   📄 guide.md         │  │                                         │
│  │ 📄 README.md          │  │                                         │
│  │ ─────────────────     │  │                                         │
│  │ [+ New File] [+ Folder]  │                                         │
│  └──────────────────────┘  │                                         │
├─ Status Bar ────────────────┴────────────────────────────────────────-┤
```

### New Files

| File | Responsibility |
|---|---|
| `views/github_auth_modal.rs` | Auth only: Unauthenticated → DeviceFlow → success callback |
| `views/repo_add_modal.rs` | Repo/branch/folder wizard → local folder pick → upsert binding |
| `views/github_sidebar.rs` | Sidebar shell: repo selector + file explorer |

### Modified Files

| File | Change |
|---|---|
| `app_state.rs` | Add `sidebar_open`, `auth_modal_open`, `repo_add_modal_open`, `pending_open_path`, `active_binding_idx` |
| `views/root.rs` | New layout with sidebar; two new toolbar buttons; wire up new views |
| `views/mod.rs` | Export new modules |
| `views/github_panel.rs` | **Delete** — replaced by the three new files above |

### SOLID Checklist

- **Single Responsibility**: `GithubAuthModal` only authenticates. `RepoAddModal` only configures a binding. `GithubSidebar` only browses/navigates. `AppState` owns file switching logic.
- **Open/Closed**: Auth success triggers a callback (`on_authenticated: Box<dyn Fn(String, &mut App)>`) so the modal is reusable from both toolbar button and "+" flow.
- **Liskov Substitution**: Both `GithubAuthModal` and `RepoAddModal` render as overlay modals using the same `ModalDialog` wrapper — interchangeable in the overlay slot.
- **Interface Segregation**: `GithubSidebar` only reads `github_bindings` and `active_binding_idx` from `AppState`; it does not touch `dirty` or `document.path`.
- **Dependency Inversion**: `GithubSidebar` holds a `WeakEntity<AppState>` and calls `app_state.update(...)` for all mutations — it never owns state.

---

## Requirements

- [x] New Lucide SVG icons added to `assets/icons/`: `plus.svg`, `file.svg`, `panel-left.svg`
- [x] `views/mod.rs` updated to declare new modules
- [x] All existing `github_panel_open` references updated across `root.rs` and `app_state.rs`

---

## Implementation Steps

### Phase 1: AppState Extensions

Add the following fields to `AppState` in [app_state.rs](desktop/crates/octodocs-app/src/app_state.rs):

- [x] `sidebar_open: bool` — controls sidebar visibility (default `false`)
- [x] `auth_modal_open: bool` — controls `GithubAuthModal` overlay
- [x] `repo_add_modal_open: bool` — controls `RepoAddModal` overlay
- [x] `active_binding_idx: Option<usize>` — which binding the sidebar is showing (default: `None`, auto-selects first binding if present)
- [x] `pending_open_path: Option<PathBuf>` — stores a path waiting to be opened after user confirms save/discard
- [x] Remove `github_panel_open: bool`
- [x] Add method `open_file_from_sidebar(path, cx)` — encapsulates the save-guard logic (see Phase 7)
- [x] Add method `toggle_sidebar(cx)` — flips `sidebar_open`, notifies

### Phase 2: GithubAuthModal (stripped panel)

New file: `views/github_auth_modal.rs`

**State machine (simplified from current `PanelState`):**
```
Loading → Unauthenticated → DeviceFlow { user_code, verification_uri } → Done
```
- `Done` is never rendered — the modal calls an `on_authenticated` callback and closes itself
- Struct holds: `on_authenticated: Box<dyn Fn(String, &mut Context<GithubAuthModal>)>` (receives the token string)

- [x] Extract `Loading`, `Unauthenticated`, `DeviceFlow`, `Error` rendering from current `github_panel.rs`
- [x] Keep `start_auth`, `poll_for_auth` methods verbatim
- [x] On success: invoke `on_authenticated(token, cx)` then set `app_state.auth_modal_open = false`
- [x] Render as a centered `ModalDialog` overlay (same as today's panel)
- [x] "Close" / "✕" button sets `app_state.auth_modal_open = false`

### Phase 3: RepoAddModal (wizard extracted from panel)

New file: `views/repo_add_modal.rs`

**State machine:**
```
RepoSelect → BranchSelect → FolderSelect → Confirm { config, local_root }
```

- [x] Extract `RepoSelect`, `BranchSelect`, `FolderSelect`, `Confirm` rendering and handlers from `github_panel.rs` verbatim
- [x] Struct accepts `auth_token: String` at construction time (passed from sidebar or from auth modal callback)
- [x] Struct holds: `repo_search_input: Entity<InputState>`, `selected_local_root: Option<PathBuf>`, `state: WizardState`
- [x] On `apply_config()`: call `app_state.upsert_github_binding(local_root, config, cx)`, set `active_binding_idx` to the new binding, close modal (`repo_add_modal_open = false`)
- [x] "✕" / Cancel button sets `repo_add_modal_open = false`
- [x] Keep `default_local_root_for_repo`, `choose_local_root`, `use_default_local_root` helpers

### Phase 4: GithubSidebar — Repo Selector

New file: `views/github_sidebar.rs`

**Repo selector section (top of sidebar):**

- [x] Read `app_state.github_bindings` and `app_state.active_binding_idx`
- [x] Render a styled clickable dropdown showing `{owner}/{repo}:{branch}` of the active binding
  - No native `<select>` — render a button that toggles an inline popover list of bindings
  - Each row: click → update `active_binding_idx` → close popover
- [x] "+" `IconButton` at right of the selector row:
  - If `get_stored_token()` returns `Some` → set `repo_add_modal_open = true` with token
  - If no token → set `auth_modal_open = true`; store `pending_action = AddRepo` on `AppState` so auth success auto-triggers repo-add
- [x] Add `pending_post_auth_action: Option<PostAuthAction>` to `AppState`:
  ```
  enum PostAuthAction { AddRepo }
  ```
  After `GithubAuthModal` calls `on_authenticated`, check this flag and open `RepoAddModal` if set
- [x] If `github_bindings` is empty: show empty state — "No repos connected" + large "+" button

### Phase 5: GithubSidebar — File Explorer

**File explorer section (below repo selector):**

- [x] `FileExplorer` is a sub-struct or inline render function within `GithubSidebar`
- [x] Reads `local_root` from `github_bindings[active_binding_idx]`
- [x] Uses `std::fs::read_dir` to list directory contents synchronously (acceptable: local FS)
- [x] Filters: show only items where `is_dir() == true` OR `extension() == "md"`
- [x] State per instance: `expanded_dirs: HashSet<PathBuf>` — which folders are open
- [x] **Rendering rules**:
  - Folder row: `chevron-right` (collapsed) / `chevron-down` (expanded) + `folder.svg` + name; click → toggle expanded
  - File row: `file.svg` + `name.md`; click → call `app_state.open_file_from_sidebar(abs_path, cx)`
  - Indent via `pl(px(N * 12.0))` per depth level
- [x] **Create new file**: clicking "+ New File" shows an inline `Input` at the current folder level; on Enter → `std::fs::write(path, "")` + refresh
- [x] **Create folder**: clicking "+ Folder" shows inline `Input`; on Enter → `std::fs::create_dir(path)` + refresh
- [x] **Refresh**: re-read from disk after any create action (`cx.notify()` will re-render and re-read)

### Phase 6: Root Layout Integration

Changes to [root.rs](desktop/crates/octodocs-app/src/views/root.rs):

- [x] Add `github_sidebar: Entity<GithubSidebar>`, `github_auth_modal: Entity<GithubAuthModal>`, `repo_add_modal: Entity<RepoAddModal>` fields
- [x] Remove `github_panel: Entity<GitHubPanel>`
- [x] Toolbar: replace existing `github` button with two buttons:
  - `github` icon → toggles `auth_modal_open` (and calls `auth_modal.init(cx)`)
  - `panel-left` icon → calls `app_state.toggle_sidebar(cx)`
- [x] Content area: change from single `AnyElement` to flex-row:
  ```
  [sidebar (w: 260px, conditional)] | [editor content (flex-grow)]
  ```
  Sidebar shown when `app_state.sidebar_open` (always show shell when toggled, even if empty)
- [x] Overlays at the end of root div:
  ```rust
  .when(auth_modal_open, |this| this.child(self.github_auth_modal.clone()))
  .when(repo_add_modal_open, |this| this.child(self.repo_add_modal.clone()))
  ```
- [x] Status bar sync badge: unchanged — still reads `github_sync_status` and `github_bindings`
- [x] Subscribe to `AppState` in `RootView` to re-render sidebar on `sidebar_open` change (already subscribed via `github_panel_subscription` — rename/reuse)

### Phase 7: File Open Guard (Save/Discard prompt)

Add `open_file_from_sidebar(path: PathBuf, cx)` to `AppState`:

**Decision tree:**

```
open_file_from_sidebar(path)
  ├─ dirty == false OR path == document.path
  │     └─> load_document(FileIo::open(path), cx)
  ├─ dirty == true AND document.path.is_some()
  │     └─> save(cx) → load_document(FileIo::open(path), cx)
  └─ dirty == true AND document.path.is_none()
        └─> set pending_open_path = Some(path)
            set show_unsaved_prompt = true, cx.notify()
```

- [x] Add `show_unsaved_prompt: bool` to `AppState`
- [x] In `root.rs`, render an inline confirmation dialog when `show_unsaved_prompt`:
  - "You have unsaved changes. Save before opening?"
  - **Save** → `save_as(cx)` → clear `show_unsaved_prompt` → open `pending_open_path`
  - **Discard** → clear `dirty`; open `pending_open_path`; clear `show_unsaved_prompt`
  - **Cancel** → clear `pending_open_path`; clear `show_unsaved_prompt`
- [x] Use existing `ModalDialog` from `adabraka_ui::components::confirm_dialog`

---

## Testing Strategy

- [ ] Manual: open app with no bindings → sidebar toggle shows empty state with "+"
- [ ] Manual: click "+" without auth → auth modal appears; complete auth → repo-add wizard opens automatically
- [ ] Manual: add a repo binding → sidebar shows repo in dropdown + local folder contents
- [ ] Manual: click `.md` file in explorer → file opens in editor
- [ ] Manual: click `.md` with unsaved new doc → prompt appears; Save path → save-as dialog → file opens; Discard → file opens without saving
- [ ] Manual: click `.md` with unsaved but previously-saved doc → auto-saves → file opens
- [ ] Manual: create new file in explorer → appears in list; create folder → appears as expandable
- [ ] Manual: multiple bindings → repo dropdown lists all; switching changes file tree
- [ ] Manual: GitHub toolbar button (auth icon) → opens auth modal regardless of sidebar state
- [x] Build: `cargo build -p octodocs-app` with zero errors after each phase

---

### Phase 8: Post-Plan Enhancements

Features added after the original plan was fully implemented:

- [x] **Sidebar toggle redesign**: Removed toggle from toolbar. When sidebar is closed, a vertical rail (34px) appears on the left with a panel-left icon. When open, the collapse button is inside the sidebar header (top-right, next to "+").
- [x] **First-run onboarding**: No example markdown on launch. App forces GitHub auth + repo selection before allowing editing.
- [x] **Initial markdown import**: When "Enable Sync" is clicked in the repo-add wizard, all `.md` files are recursively pulled from the selected GitHub folder via `pull_markdown_files()` in `octodocs-github::discovery`.
- [x] **Import summary**: Status bar shows "Imported N markdown files" after import, auto-hides after 6 seconds with a versioned timer guard.
- [x] **Subfolder-aware sync**: `relative_sync_path()` helper preserves directory structure (e.g., `docs/guide.md` syncs to `folder/docs/guide.md`, not `folder/guide.md`).
- [x] **Contextual sync status badge**: Status bar badge shows 5 states based on current file, dirty flag, and sync history: "Not configured", "Unsynced changes", "Ready to sync", "Synced", "Synced (other file)".
- [x] **Rename sync**: File rename in sidebar triggers `sync_rename_to_github()` (delete old + push new on GitHub).
- [x] **`make reset-state`**: Clears keyring token, file token, bindings TSV, and UI state TSV for clean testing.

---

## Dependencies

- No new crate dependencies (beyond `octodocs-github` which was added in the sync plan)
- SVG icons added to `assets/icons/`: `plus.svg`, `file.svg`, `panel-left.svg`, `cloud-off.svg`, `alert-circle.svg`, `refresh-cw.svg`, `log-out.svg`, `x.svg` (Lucide)

---

## Success Criteria

- [x] GitHub toolbar button opens auth modal (or repo-add if already authenticated)
- [x] Sidebar toggle: vertical rail when closed, collapse button in header when open
- [x] Repo dropdown lists all `github_bindings`; "+" adds a new one (auth-gated)
- [x] File explorer shows all `.md` files and folders under the active binding's `local_root`
- [x] Clicking a `.md` opens it in the editor with correct save-guard behavior
- [x] Creating a file/folder from explorer writes to disk and appears in explorer
- [x] No regressions: existing save, sync, view mode, and auth flows still work
- [x] `cargo build -p octodocs-app` succeeds with zero errors
- [x] First-run onboarding forces auth + repo selection
- [x] Initial import pulls existing `.md` files from GitHub
- [x] Subfolder sync preserves directory structure in GitHub repo
