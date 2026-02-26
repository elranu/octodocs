use std::time::{Duration, SystemTime};
use std::path::{Path, PathBuf};

use adabraka_ui::prelude::*;
use gpui::{Subscription, Task};
use octodocs_core::{Document, DocumentBlock, Renderer};
use octodocs_github::{GitHubSyncConfig, SyncStatus};

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

/// Central application state — one entity shared by all views.
pub struct AppState {
    pub document: Document,
    /// The document split into top-level blocks (WYSIWYG model).
    pub blocks: Vec<DocumentBlock>,
    /// Index of the block currently open for inline editing, or `None` (all rendered).
    pub active_block: Option<usize>,
    pub dirty: bool,
    /// Current view layout mode.
    pub view_mode: ViewMode,
    /// Shared editor entity reused for whichever block is active (WYSIWYG mode).
    pub editor_state: Entity<adabraka_ui::components::editor::EditorState>,
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
    /// Action to perform after authentication succeeds.
    pub pending_post_auth_action: Option<PostAuthAction>,
    _import_summary_version: u64,
    _sync_task: Option<Task<()>>,
    _summary_task: Option<Task<()>>,
    _content_subscription: Subscription,
    _full_content_subscription: Subscription,
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

    pub fn new(cx: &mut Context<Self>) -> Self {
        let editor_state = cx.new(|cx| {
            adabraka_ui::components::editor::EditorState::new(cx)
        });
        let full_editor_state = cx.new(|cx| {
            adabraka_ui::components::editor::EditorState::new(cx)
        });

        // When the full editor changes (Source/Split mode), sync document + blocks.
        let full_subscription = cx.observe(&full_editor_state, |this, _, cx| {
            if this.view_mode != ViewMode::Wysiwyg {
                let content = this.full_editor_state.read(cx).content();
                this.blocks = Renderer::parse_blocks(&content);
                this.document.content = content;
                this.dirty = true;
                cx.notify();
            }
        });

        // When the block editor changes, sync back to the active block.
        let subscription = cx.observe(&editor_state, |this, _, cx| {
            if let Some(idx) = this.active_block {
                if idx < this.blocks.len() {
                    let content = this.editor_state.read(cx).content();
                    let node = Renderer::parse(&content)
                        .0
                        .into_iter()
                        .next()
                        .unwrap_or(octodocs_core::RenderNode::Paragraph(vec![]));
                    this.blocks[idx].source = content.trim_end().to_string() + "\n";
                    this.blocks[idx].node = node;
                    this.document.content = DocumentBlock::reassemble(&this.blocks);
                    this.dirty = true;
                    cx.notify();
                }
            }
        });

        let github_bindings = Self::load_github_bindings_from_disk();
        let (saved_active_binding_idx, saved_last_opened_file) = Self::load_ui_state_from_disk();

        let mut document = Document::new();
        if let Some(path) = saved_last_opened_file {
            if let Ok(doc) = octodocs_core::FileIo::open(&path) {
                document = doc;
            }
        }
        let blocks = Renderer::parse_blocks(&document.content);

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

        Self {
            document,
            blocks,
            active_block: None,
            dirty: false,
            view_mode: ViewMode::Wysiwyg,
            editor_state,
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
            pending_post_auth_action: None,
            _import_summary_version: 0,
            _sync_task: None,
            _summary_task: None,
            _content_subscription: subscription,
            _full_content_subscription: full_subscription,
        }
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
            // Leaving WYSIWYG: load full document into the full editor.
            let content = self.document.content.clone();
            self.active_block = None;
            self.full_editor_state.update(cx, |state, cx| {
                state.set_content(&content, cx);
            });
        } else if !was_wysiwyg && going_wysiwyg {
            // Returning to WYSIWYG: re-parse blocks from full editor content.
            let content = self.full_editor_state.read(cx).content();
            self.blocks = Renderer::parse_blocks(&content);
            self.document.content = content;
            self.active_block = None;
        }

        self.view_mode = mode;
        cx.notify();
    }

    /// Activate inline editing for block at `idx`.
    pub fn activate_block(&mut self, idx: usize, cx: &mut Context<AppState>) {
        if idx >= self.blocks.len() { return; }

        // If this block is already active, don't re-create the editor state —
        // just re-focus it so the cursor reappears after an outside click.
        if self.active_block == Some(idx) {
            if let Some(rb) = &self.active_rich_block {
                rb.update(cx, |s, cx| {
                    s.needs_focus = true;
                    cx.notify();
                });
            }
            return;
        }

        self.active_block = Some(idx);
        let src = self.blocks[idx].source.trim_end().to_string();
        self.editor_state.update(cx, |state, cx| {
            state.set_content(&src, cx);
            state.place_cursor_at_end(cx);
        });
        cx.notify();
    }

    /// Deactivate block editing — all blocks return to rendered view.
    pub fn deactivate_block(&mut self, cx: &mut Context<AppState>) {
        self.active_block = None;
        cx.notify();
    }

    /// Reset to a brand new empty document.
    pub fn new_document(&mut self, cx: &mut Context<AppState>) {
        self.document = Document::new();
        self.blocks = vec![];
        self.active_block = None;
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
        self.pending_post_auth_action = None;
        self.editor_state.update(cx, |state, cx| state.set_content("", cx));
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
        if self.document.path.as_ref() == Some(&path) || !self.dirty {
            match octodocs_core::FileIo::open(&path) {
                Ok(doc) => self.load_document(doc, cx),
                Err(err) => eprintln!("Open error: {err}"),
            }
            return;
        }

        if self.document.path.is_some() {
            self.save(cx);
            match octodocs_core::FileIo::open(&path) {
                Ok(doc) => self.load_document(doc, cx),
                Err(err) => eprintln!("Open error: {err}"),
            }
            return;
        }

        self.pending_open_path = Some(path);
        self.show_unsaved_prompt = true;
        cx.notify();
    }

    /// Load a document from disk.
    pub fn load_document(&mut self, doc: Document, cx: &mut Context<AppState>) {
        let content = doc.content.clone();
        self.blocks = Renderer::parse_blocks(&content);
        self.active_block = None;
        self.editor_state.update(cx, |state, cx| state.set_content("", cx));
        // If currently in Source/Split, populate the full editor too.
        if self.view_mode != ViewMode::Wysiwyg {
            let c = content.clone();
            self.full_editor_state.update(cx, |state, cx| state.set_content(&c, cx));
        }
        self.document = doc;
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

        self.github_sync_status = SyncStatus::Syncing;
        cx.notify();

        self._sync_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { octodocs_github::push_file(&token, &config, &filename, &content) })
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

    pub fn clear_github_bindings(&mut self, cx: &mut Context<AppState>) {
        self.github_bindings.clear();
        self.active_binding_idx = None;
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
}

