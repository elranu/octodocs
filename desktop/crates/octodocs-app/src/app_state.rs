use std::time::{Duration, SystemTime};
use std::path::{Path, PathBuf};

use adabraka_ui::prelude::*;
use gpui::{Subscription, Task};
use octodocs_core::{DocParagraph, Document, DocumentBlock, Renderer, doc_paragraphs_to_markdown, markdown_to_doc_paragraphs};
use octodocs_github::{GitHubSyncConfig, SyncStatus};

/// Copy an external image file into `{doc_dir}/images/` and return the relative URL string.
/// Falls back to `$TMPDIR/octodocs/images/` when the document has not been saved yet.
pub fn copy_image_to_images_dir(src: &Path, doc_path: Option<&Path>) -> std::io::Result<String> {
    let images_dir = doc_images_dir(doc_path);
    std::fs::create_dir_all(&images_dir)?;
    let filename = src.file_name()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "missing filename"))?;
    // Sanitize: spaces are not valid in CommonMark URLs, replace with underscores.
    let safe_name = filename.to_string_lossy().replace(' ', "_");
    let dest = images_dir.join(&safe_name);
    std::fs::copy(src, &dest)?;
    Ok(format!("images/{}", safe_name))
}

fn doc_images_dir(doc_path: Option<&Path>) -> PathBuf {
    doc_path
        .and_then(|p| p.parent())
        .map(|d| d.to_path_buf())
        .unwrap_or_else(|| std::env::temp_dir().join("octodocs"))
        .join("images")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubSyncBinding {
    pub local_root: PathBuf,
    pub config: GitHubSyncConfig,
}

/// Which content layout the user is currently viewing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    /// Single-pane WYSIWYG block editor (Typora/Notion style).
    Wysiwyg,
    /// Side-by-side raw Markdown source + live rendered preview.
    Split,
    /// Full-width raw Markdown source only.
    Source,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostAuthAction {
    AddRepo,
}

impl ViewMode {
    /// Cycle to the next mode in order: Wysiwyg → Split → Source → Wysiwyg.
    pub fn next(self) -> Self {
        match self {
            ViewMode::Wysiwyg => ViewMode::Split,
            ViewMode::Split => ViewMode::Source,
            ViewMode::Source => ViewMode::Wysiwyg,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ViewMode::Wysiwyg => "WYSIWYG",
            ViewMode::Split => "Split",
            ViewMode::Source => "Source",
        }
    }
}

/// Tracks the progress of an in-app update initiated via the update banner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateStatus {
    /// No update in progress.
    Idle,
    /// Downloading the new installer / running install.sh.
    Downloading,
    /// Update script launched; user should restart (Linux/macOS).
    Done,
}

/// Central application state — one entity shared by all views.
pub struct AppState {
    pub document: Document,
    /// The document split into top-level blocks (for the Split/Source preview pane).
    pub blocks: Vec<DocumentBlock>,
    pub dirty: bool,
    /// Current view layout mode.
    pub view_mode: ViewMode,
    /// Word-style rich document editor (WYSIWYG mode).
    pub doc_editor: Entity<DocumentEditorState>,
    /// Full-document editor for Source and Split modes.
    pub full_editor_state: Entity<adabraka_ui::components::editor::EditorState>,
    /// GitHub sync bindings: local root folder -> remote destination.
    pub github_bindings: Vec<GitHubSyncBinding>,
    /// Runtime status of GitHub autosync.
    pub github_sync_status: SyncStatus,
    /// Last local file path that successfully synced to GitHub.
    pub last_synced_path: Option<PathBuf>,
    /// Last repository import summary shown in status bar.
    pub last_import_summary: Option<String>,
    /// Whether the GitHub sidebar is open.
    pub sidebar_open: bool,
    /// Whether the GitHub auth modal is open.
    pub auth_modal_open: bool,
    /// Whether the repository add wizard modal is open.
    pub repo_add_modal_open: bool,
    /// Which GitHub binding is currently selected in the sidebar.
    pub active_binding_idx: Option<usize>,
    /// A path waiting to be opened after unsaved-change confirmation.
    pub pending_open_path: Option<PathBuf>,
    /// Whether to show unsaved-change confirmation before opening sidebar file.
    pub show_unsaved_prompt: bool,
    /// Set when the user clicked the window close button with unsaved changes.
    /// The in-app modal will quit the app after Save/Discard when this is true.
    pub pending_window_close: bool,
    /// Action to perform after authentication succeeds.
    pub pending_post_auth_action: Option<PostAuthAction>,
    _import_summary_version: u64,
    _sync_task: Option<Task<()>>,
    _pull_task: Option<Task<()>>,
    _summary_task: Option<Task<()>>,
    _doc_editor_subscription: Subscription,
    _full_content_subscription: Subscription,
    /// Counts how many pending editor notifications were triggered by load_document
    /// (not by the user). Observers skip marking dirty while this is > 0.
    loading_doc: usize,
    /// Monotonic generation id for open-file requests.
    /// Async results from older generations are ignored.
    load_generation: u64,
    /// Latest release tag available for update (e.g. "v0.1.8"), `None` if up-to-date.
    pub update_available: Option<String>,
    /// Progress of an in-progress update.
    pub update_status: UpdateStatus,
    _update_task: Option<Task<()>>,
    /// Whether the UI is currently in dark mode.
    pub is_dark: bool,
    /// Background task that auto-saves every 60 seconds when dirty.
    _autosave_task: Option<Task<()>>,
    /// Background task that polls the system for dark/light mode changes (Linux).
    _theme_watcher_task: Option<Task<()>>,
    /// Whether the Insert Link dialog is visible.
    pub insert_link_modal_open: bool,
    /// Pre-filled text for the Insert Link dialog (copied from the current selection).
    pub insert_link_prefill_text: String,
}

impl AppState {
    fn bindings_store_path() -> Option<PathBuf> {
        let base = dirs::config_dir()
            .or_else(|| dirs::home_dir().map(|home| home.join(".config")))?;
        Some(base.join("octodocs").join("github_bindings.tsv"))
    }

    fn load_github_bindings_from_disk() -> Vec<GitHubSyncBinding> {
        let Some(path) = Self::bindings_store_path() else {
            return vec![];
        };
        let Ok(content) = std::fs::read_to_string(path) else {
            return vec![];
        };

        content
            .lines()
            .filter_map(|line| {
                let parts = line.split('\t').collect::<Vec<_>>();
                if parts.len() != 5 {
                    return None;
                }
                Some(GitHubSyncBinding {
                    local_root: PathBuf::from(parts[4]),
                    config: GitHubSyncConfig {
                        owner: parts[0].to_string(),
                        repo: parts[1].to_string(),
                        branch: parts[2].to_string(),
                        folder: parts[3].to_string(),
                    },
                })
            })
            .collect()
    }

    fn persist_github_bindings_to_disk(&self) {
        let Some(path) = Self::bindings_store_path() else {
            return;
        };
        if let Some(parent) = path.parent() {
            if std::fs::create_dir_all(parent).is_err() {
                return;
            }
        }

        let body = self
            .github_bindings
            .iter()
            .map(|binding| {
                format!(
                    "{}\t{}\t{}\t{}\t{}",
                    binding.config.owner,
                    binding.config.repo,
                    binding.config.branch,
                    binding.config.folder,
                    binding.local_root.display()
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        let _ = std::fs::write(path, body);
    }

    fn ui_state_store_path() -> Option<PathBuf> {
        let base = dirs::config_dir()
            .or_else(|| dirs::home_dir().map(|home| home.join(".config")))?;
        Some(base.join("octodocs").join("ui_state.tsv"))
    }

    fn load_ui_state_from_disk() -> (Option<usize>, Option<PathBuf>) {
        let Some(path) = Self::ui_state_store_path() else {
            return (None, None);
        };
        let Ok(content) = std::fs::read_to_string(path) else {
            return (None, None);
        };

        let mut active_binding_idx = None;
        let mut last_opened_file = None;

        for line in content.lines() {
            let mut parts = line.splitn(2, '\t');
            let key = parts.next().unwrap_or_default();
            let value = parts.next().unwrap_or_default();
            match key {
                "active_binding_idx" => {
                    active_binding_idx = value.parse::<usize>().ok();
                }
                "last_opened_file" => {
                    if !value.is_empty() {
                        last_opened_file = Some(PathBuf::from(value));
                    }
                }
                _ => {}
            }
        }

        (active_binding_idx, last_opened_file)
    }

    fn persist_ui_state_to_disk(&self) {
        let Some(path) = Self::ui_state_store_path() else {
            return;
        };
        if let Some(parent) = path.parent() {
            if std::fs::create_dir_all(parent).is_err() {
                return;
            }
        }

        let mut lines = Vec::new();
        if let Some(idx) = self.active_binding_idx {
            lines.push(format!("active_binding_idx\t{idx}"));
        }
        if let Some(doc_path) = self.document.path.as_ref() {
            lines.push(format!("last_opened_file\t{}", doc_path.display()));
        }

        let _ = std::fs::write(path, lines.join("\n"));
    }

    pub fn new_with(cx: &mut Context<Self>, initial_is_dark: bool) -> Self {
        let doc_editor = cx.new(DocumentEditorState::new);
        let full_editor_state = cx.new(|cx| {
            adabraka_ui::components::editor::EditorState::new(cx)
        });

        // When the full editor changes (Source/Split mode), sync document + blocks.
        let full_subscription = cx.observe(&full_editor_state, |this, _, cx| {
            // Skip if this notification came from load_document, not from the user.
            if this.loading_doc > 0 {
                this.loading_doc -= 1;
                return;
            }
            if this.view_mode != ViewMode::Wysiwyg {
                let content = this.full_editor_state.read(cx).content();
                this.blocks = Renderer::parse_blocks(&content);
                this.document.content = content;
                this.dirty = true;
                cx.notify();
            }
        });

        // When the doc_editor changes (WYSIWYG mode), sync markdown back to document.
        let doc_editor_sub = cx.observe(&doc_editor, |this, _, cx| {
            // Consume any in-app .md navigation request set by a link click.
            let nav = this.doc_editor.update(cx, |ed, _| ed.navigate_request.take());
            if let Some(path_str) = nav {
                let path = std::path::PathBuf::from(&path_str);
                this.open_file_from_link(path, cx);
                return;
            }

            // Skip if this notification came from load_document, not from the user.
            if this.loading_doc > 0 {
                this.loading_doc -= 1;
                // Normalize document.content to the round-trip output so that any
                // whitespace differences introduced by markdown→paragraphs→markdown
                // don't produce a false dirty flag on the next user edit.
                if this.view_mode == ViewMode::Wysiwyg {
                    this.document.content = this.doc_editor.read(cx).to_markdown();
                }
                return;
            }
            if this.view_mode == ViewMode::Wysiwyg {
                let markdown = this.doc_editor.read(cx).to_markdown();
                // Only mark dirty when the *content* actually changed.
                // UI-only state changes (hover badge, zoom overlay) also call
                // cx.notify() on doc_editor but don't alter the markdown, so
                // we must compare before writing back to avoid a false dirty flag.
                if markdown != this.document.content {
                    this.document.content = markdown;
                    this.dirty = true;
                    cx.notify();
                }
            }
        });

        let github_bindings = Self::load_github_bindings_from_disk();
        let (saved_active_binding_idx, saved_last_opened_file) = Self::load_ui_state_from_disk();

        let document = Document::new();
        // WYSIWYG is the startup default; split/source blocks are computed lazily
        // when those modes are entered. The last-opened file is restored async below.
        let blocks = vec![];

        // Populate the word-style editor with empty content on startup.
        // The actual document is loaded asynchronously after state is constructed.
        doc_editor.update(cx, |editor, cx| {
            editor.document_dir = None;
            editor.load_document(vec![], cx);
        });

        let active_binding_idx = if github_bindings.is_empty() {
            None
        } else if let Some(saved_idx) = saved_active_binding_idx {
            if saved_idx < github_bindings.len() {
                Some(saved_idx)
            } else {
                Some(github_bindings.len() - 1)
            }
        } else {
            Some(github_bindings.len() - 1)
        };

        let mut state = Self {
            document,
            blocks,
            dirty: false,
            view_mode: ViewMode::Wysiwyg,
            doc_editor,
            full_editor_state,
            github_bindings,
            github_sync_status: SyncStatus::Idle,
            last_synced_path: None,
            last_import_summary: None,
            sidebar_open: true,
            auth_modal_open: false,
            repo_add_modal_open: false,
            active_binding_idx,
            pending_open_path: None,
            show_unsaved_prompt: false,
            pending_window_close: false,
            pending_post_auth_action: None,
            // One pending notification from the startup doc_editor.update() above.
            loading_doc: 1,
            load_generation: 0,
            update_available: None,
            update_status: UpdateStatus::Idle,
            _update_task: None,
            is_dark: initial_is_dark,
            _autosave_task: None,
            _theme_watcher_task: None,
            insert_link_modal_open: false,
            insert_link_prefill_text: String::new(),
            _import_summary_version: 0,
            _sync_task: None,
            _pull_task: None,
            _summary_task: None,
            _doc_editor_subscription: doc_editor_sub,
            _full_content_subscription: full_subscription,
        };

        // Auto-save every 60 seconds when there are unsaved changes and a path exists.
        // To avoid UI stalls, disk I/O is done on the background executor:
        //   1. Capture path + content on the UI thread and clear dirty.
        //   2. Write the file off-thread.
        //   3. On success, trigger GitHub sync back on the UI thread.
        //   4. On failure, restore dirty so the next cycle retries.
        state._autosave_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .spawn(async move { std::thread::sleep(Duration::from_secs(60)); })
                    .await;

                // Step 1: capture on UI thread.
                let snapshot = this.update(cx, |state, cx| {
                    if state.dirty {
                        if let Some(path) = state.document.path.clone() {
                            let content = state.document.content.clone();
                            state.dirty = false;
                            cx.notify();
                            return Some((path, content));
                        }
                    }
                    None
                });

                let Ok(Some((path, content))) = snapshot else {
                    continue;
                };

                // Step 2: write off-thread.
                let write_path = path.clone();
                let write_content = content.clone();
                let write_result = cx
                    .background_executor()
                    .spawn(async move { std::fs::write(&write_path, write_content.as_bytes()) })
                    .await;

                // Step 3/4: back on UI thread — trigger sync or restore dirty on error.
                let _ = this.update(cx, |state, cx| {
                    match write_result {
                        Ok(()) => {
                            state.trigger_github_sync(cx);
                        }
                        Err(e) => {
                            eprintln!("Auto-save error: {e}");
                            // Restore dirty so next cycle retries.
                            state.dirty = true;
                            cx.notify();
                        }
                    }
                });
            }
        }));

        // On Linux, poll gsettings every 3 s and apply a theme switch when detected.
        #[cfg(target_os = "linux")]
        {
            state._theme_watcher_task = Some(cx.spawn(async move |this, cx| {
                let mut last_dark = initial_is_dark;
                loop {
                    cx.background_executor()
                        .spawn(async move { std::thread::sleep(Duration::from_secs(3)); })
                        .await;
                    let current = cx.background_executor()
                        .spawn(async move { crate::linux_is_dark_mode() })
                        .await;
                    if current != last_dark {
                        last_dark = current;
                        let _ = this.update(cx, |state, cx| {
                            state.is_dark = current;
                            if current {
                                install_theme(cx, Theme::dark());
                            } else {
                                install_theme(cx, Theme::light());
                            }
                            cx.notify();
                        });
                    }
                }
            }));
        }

        // Kick off background update check after 3 s so it never delays startup.
        state._update_task = Some(cx.spawn(async move |this, cx| {
            let tag = cx
                .background_executor()
                .spawn(async move {
                    std::thread::sleep(Duration::from_secs(3));
                    crate::updater::check_for_update()
                })
                .await;

            if let Some(tag) = tag {
                let _ = this.update(cx, |state, cx| {
                    state.update_available = Some(tag);
                    cx.notify();
                });
            }
        }));

        // Restore the last-opened file asynchronously so app startup is not gated
        // by file I/O + markdown parsing on the UI thread.
        if let Some(path) = saved_last_opened_file {
            state._pull_task = Some(cx.spawn(async move |this, cx| {
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        let doc = octodocs_core::FileIo::open(&path)?;
                        let content = doc.content.clone();
                        let paragraphs = markdown_to_doc_paragraphs(&content);
                        let blocks = Renderer::parse_blocks(&content);
                        let normalized = doc_paragraphs_to_markdown(&paragraphs);
                        Ok::<_, anyhow::Error>((doc, paragraphs, blocks, normalized))
                    })
                    .await;
                let _ = this.update(cx, |state, cx| match result {
                    Ok((doc, paragraphs, blocks, normalized)) => {
                        state.load_document_parsed(doc, paragraphs, blocks, normalized, cx)
                    }
                    Err(e) => eprintln!("Startup restore error: {e}"),
                });
            }));
        }

        state
    }

    pub fn set_import_summary(&mut self, message: String, cx: &mut Context<AppState>) {
        self.last_import_summary = Some(message);
        self._import_summary_version = self._import_summary_version.saturating_add(1);
        let version = self._import_summary_version;
        cx.notify();

        self._summary_task = Some(cx.spawn(async move |this, cx| {
            let _ = cx
                .background_executor()
                .spawn(async move {
                    std::thread::sleep(Duration::from_secs(6));
                })
                .await;

            let _ = this.update(cx, |state, cx| {
                if state._import_summary_version == version {
                    state.last_import_summary = None;
                    cx.notify();
                }
            });
        }));
    }

    /// Cycle to the next view mode, syncing content as needed.
    pub fn cycle_view_mode(&mut self, cx: &mut Context<AppState>) {
        let next = self.view_mode.next();
        self.set_view_mode(next, cx);
    }

    /// Switch to a specific view mode, syncing content between editor states.
    pub fn set_view_mode(&mut self, mode: ViewMode, cx: &mut Context<AppState>) {
        let was_wysiwyg = self.view_mode == ViewMode::Wysiwyg;
        let going_wysiwyg = mode == ViewMode::Wysiwyg;

        if was_wysiwyg && !going_wysiwyg {
            // Leaving WYSIWYG: serialize doc_editor → full_editor_state + blocks.
            let content = self.doc_editor.read(cx).to_markdown();
            self.document.content = content.clone();
            self.blocks = Renderer::parse_blocks(&content);
            self.full_editor_state.update(cx, |state, cx| {
                state.set_content(&content, cx);
            });
        } else if !was_wysiwyg && going_wysiwyg {
            // Returning to WYSIWYG: re-parse markdown → doc_editor.
            let content = self.full_editor_state.read(cx).content();
            self.blocks = Renderer::parse_blocks(&content);
            self.document.content = content.clone();
            let paragraphs = markdown_to_doc_paragraphs(&content);
            let doc_dir = self.document.path.as_ref().and_then(|p| p.parent()).map(|p| p.to_path_buf());
            self.doc_editor.update(cx, |editor, cx| {
                editor.document_dir = doc_dir;
                editor.load_document(paragraphs, cx);
            });
        }

        self.view_mode = mode;
        cx.notify();
    }

    /// Reset to a brand new empty document.
    pub fn new_document(&mut self, cx: &mut Context<AppState>) {
        self.document = Document::new();
        self.blocks = vec![];
        self.dirty = false;
        self.github_sync_status = SyncStatus::Idle;
        self.last_synced_path = None;
        self.last_import_summary = None;
        self.sidebar_open = false;
        self.auth_modal_open = false;
        self.repo_add_modal_open = false;
        self.active_binding_idx = None;
        self.pending_open_path = None;
        self.show_unsaved_prompt = false;
        self.pending_window_close = false;
        self.pending_post_auth_action = None;
        self.loading_doc = 0; // reset any stale counter before incrementing
        self.loading_doc += 1;
        self.doc_editor.update(cx, |editor, cx| {
            editor.document_dir = None;
            editor.load_document(vec![], cx);
        });
        self.loading_doc += 1;
        self.full_editor_state.update(cx, |state, cx| state.set_content("", cx));
        cx.notify();
    }

    pub fn toggle_sidebar(&mut self, cx: &mut Context<AppState>) {
        self.sidebar_open = !self.sidebar_open;
        cx.notify();
    }

    pub fn set_active_binding_idx(&mut self, idx: usize, cx: &mut Context<AppState>) {
        if idx < self.github_bindings.len() {
            self.active_binding_idx = Some(idx);
            self.persist_ui_state_to_disk();
            cx.notify();
        }
    }

    pub fn open_file_from_sidebar(&mut self, path: PathBuf, cx: &mut Context<AppState>) {
        // Dirty document: ask before discarding changes.
        if self.dirty {
            self.pending_open_path = Some(path);
            self.show_unsaved_prompt = true;
            cx.notify();
            return;
        }

        // Non-dirty (including re-opening the same file): pull from GitHub then load.
        self.pull_and_open_file(path, cx);
    }

    fn clear_document_for_open(&mut self, path: PathBuf, cx: &mut Context<AppState>) {
        self.document.path = Some(path);
        self.document.content.clear();
        self.blocks.clear();
        self.dirty = false;
        self.loading_doc += 1;
        self.doc_editor.update(cx, |editor, _| {
            editor.clear();
        });
        self.persist_ui_state_to_disk();
        cx.notify();
    }

    /// Open a local .md file immediately from disk without a GitHub pull.
    /// Used for in-editor link navigation where instant feedback matters.
    pub fn open_file_from_link(&mut self, path: PathBuf, cx: &mut Context<AppState>) {
        if self.dirty {
            self.pending_open_path = Some(path);
            self.show_unsaved_prompt = true;
            cx.notify();
            return;
        }
        self.clear_document_for_open(path.clone(), cx);
        self.load_generation = self.load_generation.saturating_add(1);
        let expected_gen = self.load_generation;
        // Read the file on the background executor so the UI thread is never blocked.
        self._pull_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let doc = octodocs_core::FileIo::open(&path)?;
                    let content = doc.content.clone();
                    let paragraphs = markdown_to_doc_paragraphs(&content);
                    let blocks = Renderer::parse_blocks(&content);
                    let normalized = doc_paragraphs_to_markdown(&paragraphs);
                    Ok::<_, anyhow::Error>((doc, paragraphs, blocks, normalized))
                })
                .await;
            let _ = this.update(cx, |state, cx| {
                if state.load_generation != expected_gen {
                    return;
                }
                match result {
                    Ok((doc, paragraphs, blocks, normalized)) => {
                        state.load_document_parsed(doc, paragraphs, blocks, normalized, cx)
                    }
                    Err(e) => eprintln!("Link navigation error: {e}"),
                }
            });
        }));
    }

    /// Fetch the latest version of `path` from GitHub (if a sync binding exists),
    /// overwrite the local file on success, then load the document into the editor.
    /// All error paths fall back to opening whatever is on disk — no crash, no dialog.
    /// Public so that view code can call it directly (e.g. "Open" toolbar button, Discard prompt).
    pub fn pull_and_open_file(&mut self, path: PathBuf, cx: &mut Context<AppState>) {
        self.clear_document_for_open(path.clone(), cx);
        self.load_generation = self.load_generation.saturating_add(1);
        let expected_gen = self.load_generation;

        // Resolve sync binding metadata — fall back to async local open if none.
        // Token lookup is intentionally done on the background executor to keep UI responsive.
        let (config, filename) = match self.resolve_pull_params(&path) {
            Some(params) => params,
            None => {
                // No GitHub binding — open the local file on a background thread
                // so disk I/O + markdown parsing never blocks the UI.
                self._pull_task = Some(cx.spawn(async move |this, cx| {
                    let result = cx
                        .background_executor()
                        .spawn(async move {
                            let doc = octodocs_core::FileIo::open(&path)?;
                            let content = doc.content.clone();
                            let paragraphs = markdown_to_doc_paragraphs(&content);
                            let blocks = Renderer::parse_blocks(&content);
                            let normalized = doc_paragraphs_to_markdown(&paragraphs);
                            Ok::<_, anyhow::Error>((doc, paragraphs, blocks, normalized))
                        })
                        .await;
                    let _ = this.update(cx, |state, cx| {
                        if state.load_generation != expected_gen {
                            return;
                        }
                        match result {
                        Ok((doc, paragraphs, blocks, normalized)) => {
                            state.load_document_parsed(doc, paragraphs, blocks, normalized, cx)
                        }
                        Err(e) => eprintln!("Open error: {e}"),
                        }
                    });
                }));
                return;
            }
        };

        self.github_sync_status = SyncStatus::Syncing;
        cx.notify();

        self._pull_task = Some(cx.spawn(async move |this, cx| {
            let path_fallback = path.clone();

            let result: anyhow::Result<(bool, Document, Vec<DocParagraph>, Vec<DocumentBlock>, String)> = cx
                .background_executor()
                .spawn(async move {
                    let token = octodocs_github::get_stored_token().ok().flatten();
                    let (pulled, doc) = if let Some(token) = token {
                        let pulled = octodocs_github::pull_file(&token, &config, &filename)?;
                        let doc = if let Some(content) = pulled.clone() {
                            std::fs::write(&path, content.as_bytes()).map_err(|e| {
                                anyhow::anyhow!(
                                    "Failed to write pulled content to '{}': {e}",
                                    path.display()
                                )
                            })?;
                            octodocs_core::FileIo::open(&path)?
                        } else {
                            // 404 — file not yet on GitHub; open local copy.
                            octodocs_core::FileIo::open(&path)?
                        };
                        (pulled.is_some(), doc)
                    } else {
                        // No token available — silently fall back to local open.
                        (false, octodocs_core::FileIo::open(&path)?)
                    };
                    let content = doc.content.clone();
                    let paragraphs = markdown_to_doc_paragraphs(&content);
                    let blocks = Renderer::parse_blocks(&content);
                    let normalized = doc_paragraphs_to_markdown(&paragraphs);
                    Ok((pulled, doc, paragraphs, blocks, normalized))
                })
                .await;

            let _ = this.update(cx, |state, cx| {
                if state.load_generation != expected_gen {
                    return;
                }

                match result {
                    Ok((_pulled, doc, paragraphs, blocks, normalized)) => {
                        state.github_sync_status = SyncStatus::Idle;
                        state.load_document_parsed(doc, paragraphs, blocks, normalized, cx);
                    }
                    Err(err) => {
                        state.github_sync_status = SyncStatus::Failed {
                            message: err.to_string(),
                        };
                        cx.notify();
                        // Open fallback on background thread — never block the UI.
                        state._pull_task = Some(cx.spawn(async move |this, cx| {
                            let result = cx
                                .background_executor()
                                .spawn(async move {
                                    let doc = octodocs_core::FileIo::open(&path_fallback)?;
                                    let content = doc.content.clone();
                                    let paragraphs = markdown_to_doc_paragraphs(&content);
                                    let blocks = Renderer::parse_blocks(&content);
                                    let normalized = doc_paragraphs_to_markdown(&paragraphs);
                                    Ok::<_, anyhow::Error>((doc, paragraphs, blocks, normalized))
                                })
                                .await;
                            let _ = this.update(cx, |state, cx| {
                                if state.load_generation != expected_gen {
                                    return;
                                }
                                match result {
                                    Ok((doc, paragraphs, blocks, normalized)) => {
                                        state.load_document_parsed(doc, paragraphs, blocks, normalized, cx)
                                    }
                                    Err(e) => eprintln!("Open error: {e}"),
                                }
                            });
                        }));
                    }
                }
            });
        }));
    }

    /// Resolve the GitHub pull parameters for `path`.
    /// Returns `None` if no binding or no relative path can be computed.
    fn resolve_pull_params(&self, path: &Path) -> Option<(octodocs_github::GitHubSyncConfig, String)> {
        let binding = self.find_sync_binding(path)?;
        let filename = Self::relative_sync_path(binding, path)?;
        Some((binding.config.clone(), filename))
    }

    /// Load an already-parsed document — all heavy work (parsing, normalization) was
    /// done on a background thread before this call so the UI thread is never blocked.
    pub fn load_document_parsed(
        &mut self,
        doc: Document,
        paragraphs: Vec<DocParagraph>,
        blocks: Vec<DocumentBlock>,
        normalized_content: String,
        cx: &mut Context<AppState>,
    ) {
        self.blocks = if self.view_mode == ViewMode::Wysiwyg { vec![] } else { blocks };
        let doc_dir = doc.path.as_ref().and_then(|p| p.parent()).map(|p| p.to_path_buf());
        self.loading_doc += 1;
        self.doc_editor.update(cx, |editor, cx| {
            editor.document_dir = doc_dir;
            editor.load_document(paragraphs, cx);
        });
        if self.view_mode != ViewMode::Wysiwyg {
            self.loading_doc += 1;
            let c = normalized_content.clone();
            self.full_editor_state.update(cx, |state, cx| state.set_content(&c, cx));
        }
        self.document = doc;
        // Already normalized — no to_markdown() call needed on the UI thread.
        self.document.content = normalized_content;
        self.dirty = false;
        self.persist_ui_state_to_disk();
        cx.notify();
    }

    fn trigger_github_sync(&mut self, cx: &mut Context<AppState>) {
        let Some(path) = self.document.path.as_ref() else {
            self.github_sync_status = SyncStatus::Failed {
                message: "Missing local document path for sync".to_string(),
            };
            cx.notify();
            return;
        };

        let Some(binding) = self.find_sync_binding(path) else {
            if self.github_bindings.is_empty() {
                self.github_sync_status = SyncStatus::Idle;
            } else {
                self.github_sync_status = SyncStatus::Failed {
                    message: "No GitHub sync mapping for this file path".to_string(),
                };
            }
            cx.notify();
            return;
        };

        let config = binding.config.clone();

        let Some(filename) = Self::relative_sync_path(binding, path) else {
            self.github_sync_status = SyncStatus::Failed {
                message: "Invalid local path for sync".to_string(),
            };
            cx.notify();
            return;
        };
        let syncing_local_path = path.clone();

        let token = match octodocs_github::get_stored_token() {
            Ok(Some(token)) => token,
            Ok(None) => {
                self.github_sync_status = SyncStatus::Failed {
                    message: "GitHub token not found. Please authenticate.".to_string(),
                };
                cx.notify();
                return;
            }
            Err(err) => {
                self.github_sync_status = SyncStatus::Failed {
                    message: format!("GitHub token read failed: {err}"),
                };
                cx.notify();
                return;
            }
        };

        let content = self.document.content.clone();

        // Collect local images referenced in the document to push alongside markdown
        let doc_dir = path.parent().map(|p| p.to_path_buf());
        let mut images_to_push: Vec<(String, Vec<u8>)> = Vec::new();
        if let Some(ref dir) = doc_dir {
            for rel_img in Self::extract_local_image_paths(&content) {
                let full_img = dir.join(&rel_img);
                if let Ok(bytes) = std::fs::read(&full_img) {
                    if let Some(img_sync) = Self::relative_sync_path(binding, &full_img) {
                        images_to_push.push((img_sync, bytes));
                    }
                }
            }
        }

        self.github_sync_status = SyncStatus::Syncing;
        cx.notify();

        self._sync_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let push_result = octodocs_github::push_file(&token, &config, &filename, &content);
                    if push_result.is_ok() {
                        for (img_path, img_bytes) in &images_to_push {
                            if let Err(e) = octodocs_github::push_binary_file(&token, &config, img_path, img_bytes) {
                                eprintln!("[sync] Failed to push image '{}': {e}", img_path);
                            }
                        }
                    }
                    push_result
                })
                .await;

            let _ = this.update(cx, |state, cx| {
                state.github_sync_status = match result {
                    Ok(sha) => SyncStatus::Success {
                        committed_at: SystemTime::now(),
                        sha,
                    },
                    Err(err) => SyncStatus::Failed {
                        message: err.to_string(),
                    },
                };
                if matches!(state.github_sync_status, SyncStatus::Success { .. }) {
                    state.last_synced_path = Some(syncing_local_path.clone());
                }
                cx.notify();
            });
        }));
    }

    fn extract_local_image_paths(markdown: &str) -> Vec<String> {
        let mut paths = Vec::new();
        let mut rest = markdown;
        while let Some(img_start) = rest.find("![") {
            rest = &rest[img_start + 2..];
            let Some(bracket_end) = rest.find("](") else { continue };
            rest = &rest[bracket_end + 2..];
            let url_end = rest
                .find([')', '"', ' '])
                .unwrap_or(rest.len());
            let url = rest[..url_end].trim_matches('<').trim_matches('>');
            if !url.is_empty() && !url.contains("://") {
                paths.push(url.to_string());
            }
            rest = &rest[url_end..];
        }
        paths
    }

    fn find_sync_binding(&self, doc_path: &Path) -> Option<&GitHubSyncBinding> {
        self.github_bindings
            .iter()
            .filter(|binding| doc_path.starts_with(&binding.local_root))
            .max_by_key(|binding| binding.local_root.components().count())
    }

    fn relative_sync_path(binding: &GitHubSyncBinding, doc_path: &Path) -> Option<String> {
        let rel = doc_path.strip_prefix(&binding.local_root).ok()?;
        let path = rel.to_string_lossy().replace('\\', "/");
        let trimmed = path.trim_start_matches('/').to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    }

    pub fn sync_rename_to_github(&mut self, old_path: PathBuf, new_path: PathBuf, cx: &mut Context<AppState>) {
        let Some(binding) = self.find_sync_binding(&new_path) else {
            return;
        };

        let Some(old_filename) = Self::relative_sync_path(binding, &old_path) else {
            return;
        };
        let Some(new_filename) = Self::relative_sync_path(binding, &new_path) else {
            return;
        };

        let token = match octodocs_github::get_stored_token() {
            Ok(Some(token)) => token,
            _ => return,
        };

        let config = binding.config.clone();
        let content = match std::fs::read_to_string(&new_path) {
            Ok(content) => content,
            Err(_) => return,
        };

        self.github_sync_status = SyncStatus::Syncing;
        cx.notify();

        self._sync_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let _ = octodocs_github::delete_file(&token, &config, &old_filename);
                    octodocs_github::push_file(&token, &config, &new_filename, &content)
                })
                .await;

            let _ = this.update(cx, |state, cx| {
                state.github_sync_status = match result {
                    Ok(sha) => SyncStatus::Success {
                        committed_at: SystemTime::now(),
                        sha,
                    },
                    Err(err) => SyncStatus::Failed {
                        message: err.to_string(),
                    },
                };
                if matches!(state.github_sync_status, SyncStatus::Success { .. }) {
                    state.last_synced_path = Some(new_path.clone());
                }
                cx.notify();
            });
        }));
    }

    pub fn upsert_github_binding(
        &mut self,
        local_root: PathBuf,
        config: GitHubSyncConfig,
        cx: &mut Context<AppState>,
    ) {
        if let Some(existing) = self
            .github_bindings
            .iter_mut()
            .find(|binding| binding.local_root == local_root)
        {
            existing.config = config;
        } else {
            self.github_bindings.push(GitHubSyncBinding { local_root, config });
        }
        if self.active_binding_idx.is_none() && !self.github_bindings.is_empty() {
            self.active_binding_idx = Some(0);
        }
        self.persist_github_bindings_to_disk();
        self.persist_ui_state_to_disk();
        self.github_sync_status = SyncStatus::Idle;
        cx.notify();
    }

    /// Save to the current path (falls back to save_as if no path set).
    pub fn save(&mut self, cx: &mut Context<AppState>) {
        if self.document.path.is_some() {
            match octodocs_core::FileIo::save(&self.document) {
                Ok(_) => {
                    self.dirty = false;
                    self.trigger_github_sync(cx);
                    cx.notify();
                }
                Err(e) => eprintln!("Save error: {e}"),
            }
        } else {
            self.save_as(cx);
        }
    }

    /// Prompt for a path and save.
    pub fn save_as(&mut self, cx: &mut Context<AppState>) {
        let title = self.document.title();
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Markdown", &["md", "markdown"])
            .set_file_name(&title)
            .save_file()
        {
            match octodocs_core::FileIo::save_as(&self.document, &path) {
                Ok(_) => {
                    let doc_dir = path.parent().map(|p| p.to_path_buf());
                    self.doc_editor.update(cx, |editor, _| editor.document_dir = doc_dir);
                    self.document.path = Some(path);
                    self.dirty = false;
                    self.persist_ui_state_to_disk();
                    self.trigger_github_sync(cx);
                    cx.notify();
                }
                Err(e) => eprintln!("Save-as error: {e}"),
            }
        }
    }

    /// Hide the update banner without installing.
    pub fn dismiss_update(&mut self, cx: &mut Context<AppState>) {
        self.update_available = None;
        self.update_status = UpdateStatus::Idle;
        cx.notify();
    }

    /// Begin the platform-appropriate update:
    /// - Linux/macOS: re-runs `install.sh` via the system shell in the background.
    /// - Windows: downloads the Inno Setup installer to `%TEMP%` then launches it
    ///   silently; the installer calls `CloseApplications=force` so it will
    ///   terminate us automatically.
    pub fn trigger_update(&mut self, cx: &mut Context<AppState>) {
        let Some(tag) = self.update_available.clone() else { return };

        self.update_status = UpdateStatus::Downloading;
        cx.notify();

        self._update_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { crate::updater::launch_update(&tag) })
                .await;

            match result {
                Ok(()) => {
                    #[cfg(target_os = "windows")]
                    {
                        // The Inno Setup installer is now running with /verysilent.
                        // It has CloseApplications=force so it will terminate us if
                        // we're still alive; calling exit here is just a clean shortcut.
                        std::process::exit(0);
                    }
                    #[cfg(not(target_os = "windows"))]
                    {
                        let _ = this.update(cx, |state, cx| {
                            state.update_status = UpdateStatus::Done;
                            cx.notify();
                        });
                    }
                }
                Err(err) => {
                    eprintln!("Auto-update failed: {err}");
                    let _ = this.update(cx, |state, cx| {
                        state.update_status = UpdateStatus::Idle;
                        cx.notify();
                    });
                }
            }
        }));
    }
}

