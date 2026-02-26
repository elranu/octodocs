use adabraka_ui::prelude::*;
use gpui::Subscription;
use octodocs_core::{Document, DocumentBlock, Renderer};

/// Central application state — one entity shared by all views.
pub struct AppState {
    pub document: Document,
    /// The document split into top-level blocks (WYSIWYG model).
    pub blocks: Vec<DocumentBlock>,
    /// Index of the block currently open for inline editing, or `None` (all rendered).
    pub active_block: Option<usize>,
    pub dirty: bool,
    /// Shared editor entity reused for whichever block is active.
    pub editor_state: Entity<adabraka_ui::components::editor::EditorState>,
    _content_subscription: Subscription,
}

impl AppState {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let default_content = "# Welcome to OctoDocs\n\nStart typing **Markdown** here and watch the preview update live.\n\n## Features\n\n- Live preview\n- Open, save, and create `.md` files\n- Mermaid diagram support\n\n## Example\n\n> This is a blockquote.\n\n```mermaid\ngraph TD\n    A[Start] --> B[Edit Markdown]\n    B --> C[Preview updates]\n```\n";

        let editor_state = cx.new(|cx| {
            adabraka_ui::components::editor::EditorState::new(cx)
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
            editor_state,
            _content_subscription: subscription,
        }
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
        self.editor_state.update(cx, |state, cx| state.set_content("", cx));
        cx.notify();
    }

    /// Load a document from disk.
    pub fn load_document(&mut self, doc: Document, cx: &mut Context<AppState>) {
        let content = doc.content.clone();
        self.blocks = Renderer::parse_blocks(&content);
        self.active_block = None;
        self.editor_state.update(cx, |state, cx| state.set_content("", cx));
        self.document = doc;
        self.dirty = false;
        cx.notify();
    }

    /// Save to the current path (falls back to save_as if no path set).
    pub fn save(&mut self, cx: &mut Context<AppState>) {
        if self.document.path.is_some() {
            match octodocs_core::FileIo::save(&self.document) {
                Ok(_) => {
                    self.dirty = false;
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
                    cx.notify();
                }
                Err(e) => eprintln!("Save-as error: {e}"),
            }
        }
    }
}

