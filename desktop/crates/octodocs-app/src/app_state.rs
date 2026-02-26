use std::time::SystemTime;

use adabraka_ui::prelude::*;
use gpui::{Subscription, Task};
use octodocs_core::{Document, DocumentBlock, RichBlock, Renderer};
use octodocs_github::{GitHubSyncConfig, SyncStatus};

use crate::rich_block_editor::{RichBlockState, SpanCursor, SpanFormatToggle};

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
    /// Rich block editor state for the currently active block (WYSIWYG mode).
    pub active_rich_block: Option<Entity<RichBlockState>>,
    /// Optional GitHub destination for autosync.
    pub github_config: Option<GitHubSyncConfig>,
    /// Runtime status of GitHub autosync.
    pub github_sync_status: SyncStatus,
    /// Whether the GitHub setup panel is open.
    pub github_panel_open: bool,
    _sync_task: Option<Task<()>>,
    _content_subscription: Subscription,
    _full_content_subscription: Subscription,
    _rich_block_subscription: Option<Subscription>,
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
            active_rich_block: None,
            github_config: None,
            github_sync_status: SyncStatus::Idle,
            github_panel_open: false,
            _sync_task: None,
            _content_subscription: subscription,
            _full_content_subscription: full_subscription,
            _rich_block_subscription: None,
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

        // Load into legacy editor (still used as fallback).
        let src = self.blocks[idx].source.trim_end().to_string();
        self.editor_state.update(cx, |state, cx| {
            state.set_content(&src, cx);
            state.place_cursor_at_end(cx);
        });

        // Create / refresh the rich block editor state.
        let rich_block = RichBlock::from_document_block(&self.blocks[idx]);
        let rb_state = cx.new(|cx| {
            let mut s = RichBlockState::new(cx);
            s.set_content(&rich_block, cx);
            s
        });

        // Subscribe: propagate changes back into blocks[idx].
        let sub = cx.observe(&rb_state, |this, entity, cx| {
            let split_req = entity.read(cx).split_requested;
            let merge_req = entity.read(cx).merge_prev_requested;

            if split_req.is_some() {
                entity.update(cx, |s, _| s.split_requested = None);
                if let Some(split_cursor) = split_req {
                    this.split_block_at(split_cursor, cx);
                }
                return;
            }
            if merge_req {
                entity.update(cx, |s, _| s.merge_prev_requested = false);
                this.merge_with_previous_block(cx);
                return;
            }

            // Regular content sync.
            if let Some(idx) = this.active_block {
                if idx < this.blocks.len() {
                    let block = entity.read(cx).to_rich_block();
                    let md = block.to_markdown();
                    let node = Renderer::parse(&md)
                        .0
                        .into_iter()
                        .next()
                        .unwrap_or(octodocs_core::RenderNode::Paragraph(vec![]));
                    this.blocks[idx].source = md.trim_end().to_string() + "\n";
                    this.blocks[idx].node = node;
                    this.document.content = DocumentBlock::reassemble(&this.blocks);
                    this.dirty = true;
                    cx.notify();
                }
            }
        });

        self.active_rich_block = Some(rb_state);
        self._rich_block_subscription = Some(sub);
        cx.notify();
    }

    /// Deactivate block editing — all blocks return to rendered view.
    pub fn deactivate_block(&mut self, cx: &mut Context<AppState>) {
        self.active_block = None;
        self.active_rich_block = None;
        self._rich_block_subscription = None;
        cx.notify();
    }

    /// Reset to a brand new empty document.
    pub fn new_document(&mut self, cx: &mut Context<AppState>) {
        self.document = Document::new();
        self.blocks = vec![];
        self.active_block = None;
        self.active_rich_block = None;
        self._rich_block_subscription = None;
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
        self.active_rich_block = None;
        self._rich_block_subscription = None;
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

    /// Apply a formatting toggle to the currently active rich block's selection.
    pub fn apply_format(&mut self, toggle: SpanFormatToggle, cx: &mut Context<AppState>) {
        if let Some(rb) = &self.active_rich_block {
            rb.update(cx, |state, cx| state.apply_format(toggle, cx));
        }
    }

    /// Split the active block at `cursor`, creating two blocks.
    pub fn split_block_at(&mut self, cursor: SpanCursor, cx: &mut Context<AppState>) {
        let Some(idx) = self.active_block else { return; };
        let Some(rb) = &self.active_rich_block else { return; };
        let block = rb.read(cx).to_rich_block();

        match block {
            RichBlock::Paragraph { ref spans } => {
                let split_vis = cursor.visual_offset(spans);

                let before_spans: Vec<_> = {
                    let mut result = Vec::new();
                    let mut vis = 0;
                    for span in spans {
                        let len = span.text().chars().count();
                        let span_end = vis + len;
                        if span_end <= split_vis {
                            result.push(span.clone());
                        } else if vis < split_vis {
                            let take = split_vis - vis;
                            let text: String = span.text().chars().take(take).collect();
                            match span {
                                octodocs_core::InlineSpanKind::Styled(is) => {
                                    result.push(octodocs_core::InlineSpanKind::Styled(
                                        octodocs_core::InlineSpan::new(text, is.format.clone()),
                                    ));
                                }
                                octodocs_core::InlineSpanKind::Link { url, .. } => {
                                    result.push(octodocs_core::InlineSpanKind::Link {
                                        text,
                                        url: url.clone(),
                                    });
                                }
                            }
                        }
                        vis += len;
                    }
                    result
                };

                let after_spans: Vec<_> = {
                    let mut result = Vec::new();
                    let mut vis = 0;
                    for span in spans {
                        let len = span.text().chars().count();
                        let span_start = vis;
                        let span_end = vis + len;
                        if span_start >= split_vis {
                            result.push(span.clone());
                        } else if span_end > split_vis {
                            let skip = split_vis - span_start;
                            let text: String = span.text().chars().skip(skip).collect();
                            match span {
                                octodocs_core::InlineSpanKind::Styled(is) => {
                                    result.push(octodocs_core::InlineSpanKind::Styled(
                                        octodocs_core::InlineSpan::new(text, is.format.clone()),
                                    ));
                                }
                                octodocs_core::InlineSpanKind::Link { url, .. } => {
                                    result.push(octodocs_core::InlineSpanKind::Link {
                                        text,
                                        url: url.clone(),
                                    });
                                }
                            }
                        }
                        vis += len;
                    }
                    result
                };

                let first_md = RichBlock::Paragraph {
                    spans: if before_spans.is_empty() {
                        vec![octodocs_core::InlineSpanKind::plain("")]
                    } else {
                        before_spans
                    },
                }
                .to_markdown();
                let second_md = RichBlock::Paragraph {
                    spans: if after_spans.is_empty() {
                        vec![octodocs_core::InlineSpanKind::plain("")]
                    } else {
                        after_spans
                    },
                }
                .to_markdown();

                let first_node = Renderer::parse(&first_md)
                    .0
                    .into_iter()
                    .next()
                    .unwrap_or(octodocs_core::RenderNode::Paragraph(vec![]));
                let second_node = Renderer::parse(&second_md)
                    .0
                    .into_iter()
                    .next()
                    .unwrap_or(octodocs_core::RenderNode::Paragraph(vec![]));

                self.active_rich_block = None;
                self._rich_block_subscription = None;
                self.active_block = None;

                self.blocks[idx] = DocumentBlock {
                    source: first_md.trim_end().to_string() + "\n",
                    node: first_node,
                };
                self.blocks.insert(
                    idx + 1,
                    DocumentBlock {
                        source: second_md.trim_end().to_string() + "\n",
                        node: second_node,
                    },
                );

                self.document.content = DocumentBlock::reassemble(&self.blocks);
                self.dirty = true;
                self.activate_block(idx + 1, cx);
            }
            _ => {
                self.active_rich_block = None;
                self._rich_block_subscription = None;
                self.active_block = None;

                let new_node = octodocs_core::RenderNode::Paragraph(vec![]);
                self.blocks.insert(
                    idx + 1,
                    DocumentBlock { source: "\n".to_string(), node: new_node },
                );
                self.document.content = DocumentBlock::reassemble(&self.blocks);
                self.dirty = true;
                self.activate_block(idx + 1, cx);
            }
        }
    }

    /// Merge the active block with the block before it (triggered by Backspace at start).
    pub fn merge_with_previous_block(&mut self, cx: &mut Context<AppState>) {
        let Some(idx) = self.active_block else { return; };
        if idx == 0 {
            return;
        }

        let Some(rb) = &self.active_rich_block else { return; };
        let curr_block = rb.read(cx).to_rich_block();
        let prev_block = RichBlock::from_document_block(&self.blocks[idx - 1]);

        let (merged_block, cursor_vis) = match (prev_block, curr_block) {
            (
                RichBlock::Paragraph { spans: mut prev_spans },
                RichBlock::Paragraph { spans: curr_spans },
            ) => {
                let merge_cursor_vis =
                    prev_spans.iter().map(|s| s.text().chars().count()).sum::<usize>();
                prev_spans.extend(curr_spans);
                if prev_spans.is_empty() {
                    prev_spans.push(octodocs_core::InlineSpanKind::plain(""));
                }
                (RichBlock::Paragraph { spans: prev_spans }, merge_cursor_vis)
            }
            _ => return,
        };

        let merged_md = merged_block.to_markdown();
        let merged_node = Renderer::parse(&merged_md)
            .0
            .into_iter()
            .next()
            .unwrap_or(octodocs_core::RenderNode::Paragraph(vec![]));

        self.active_rich_block = None;
        self._rich_block_subscription = None;
        self.active_block = None;

        self.blocks[idx - 1] = DocumentBlock {
            source: merged_md.trim_end().to_string() + "\n",
            node: merged_node,
        };
        self.blocks.remove(idx);

        self.document.content = DocumentBlock::reassemble(&self.blocks);
        self.dirty = true;

        self.activate_block(idx - 1, cx);
        if let Some(rb) = &self.active_rich_block {
            let spans = rb.read(cx).spans.clone();
            let new_cursor = SpanCursor::from_visual_offset(&spans, cursor_vis);
            rb.update(cx, |state, cx| {
                state.cursor = new_cursor;
                cx.notify();
            });
        }
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

