use std::time::SystemTime;

use adabraka_ui::prelude::*;
use gpui::{Subscription, Task};
use octodocs_core::{Document, DocumentBlock, Renderer};
use octodocs_github::{GitHubSyncConfig, SyncStatus};

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
    /// Optional GitHub destination for autosync.
    pub github_config: Option<GitHubSyncConfig>,
    /// Runtime status of GitHub autosync.
    pub github_sync_status: SyncStatus,
    /// Whether the GitHub setup panel is open.
    pub github_panel_open: bool,
    _sync_task: Option<Task<()>>,
    _content_subscription: Subscription,
    _full_content_subscription: Subscription,
}

impl AppState {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let default_content = "# Welcome to OctoDocs\n\nStart typing **Markdown** here and watch the preview update live.\n\n## Features\n\n- Live preview\n- Open, save, and create `.md` files\n- Mermaid diagram support\n\n## Example\n\n> This is a blockquote.\n\n```mermaid\ngraph TD\n    A[Start] --> B[Edit Markdown]\n    B --> C[Preview updates]\n```\n";

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

        let blocks = Renderer::parse_blocks(default_content);

        Self {
            document: Document::with_content(default_content),
            blocks,
            active_block: None,
            dirty: false,
            view_mode: ViewMode::Wysiwyg,
            editor_state,
            full_editor_state,
            github_config: None,
            github_sync_status: SyncStatus::Idle,
            github_panel_open: false,
            _sync_task: None,
            _content_subscription: subscription,
            _full_content_subscription: full_subscription,
        }
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
        self.github_config = None;
        self.github_sync_status = SyncStatus::Idle;
        self.github_panel_open = false;
        self.editor_state.update(cx, |state, cx| state.set_content("", cx));
        self.full_editor_state.update(cx, |state, cx| state.set_content("", cx));
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
        self.github_sync_status = SyncStatus::Idle;
        cx.notify();
    }

    fn trigger_github_sync(&mut self, cx: &mut Context<AppState>) {
        let Some(config) = self.github_config.clone() else {
            self.github_sync_status = SyncStatus::Idle;
            return;
        };

        let Some(path) = self.document.path.as_ref() else {
            self.github_sync_status = SyncStatus::Failed {
                message: "Missing local document path for sync".to_string(),
            };
            cx.notify();
            return;
        };

        let Some(filename) = path.file_name().and_then(|f| f.to_str()).map(|f| f.to_string()) else {
            self.github_sync_status = SyncStatus::Failed {
                message: "Invalid local filename for sync".to_string(),
            };
            cx.notify();
            return;
        };

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
                cx.notify();
            });
        }));
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
                    self.trigger_github_sync(cx);
                    cx.notify();
                }
                Err(e) => eprintln!("Save-as error: {e}"),
            }
        }
    }
}

