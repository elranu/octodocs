use adabraka_ui::prelude::*;
use gpui::Subscription;
use octodocs_core::{Document, RenderTree, Renderer};

/// Central application state — one entity shared by all views.
pub struct AppState {
    pub document: Document,
    pub render_tree: RenderTree,
    pub dirty: bool,
    pub editor_state: Entity<adabraka_ui::components::editor::EditorState>,
    _content_subscription: Subscription,
}

impl AppState {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let default_content = "# Welcome to OctoDocs\n\nStart typing **Markdown** here and watch the preview update live.\n\n## Features\n\n- Live preview\n- Open, save, and create `.md` files\n- Mermaid diagram support\n\n## Example\n\n> This is a blockquote.\n\n```mermaid\ngraph TD\n    A[Start] --> B[Edit Markdown]\n    B --> C[Preview updates]\n```\n";

        let editor_state = cx.new(|cx| {
            adabraka_ui::components::editor::EditorState::new(cx)
        });

        editor_state.update(cx, |state, cx| {
            state.set_content(default_content, cx);
        });

        // Re-parse preview whenever the editor content changes.
        let subscription = cx.observe(&editor_state, |this, _, cx| {
            let content = this.editor_state.read(cx).content();
            this.document.content = content.clone();
            this.dirty = true;
            this.render_tree = Renderer::parse(&content);
            cx.notify();
        });

        Self {
            document: Document::with_content(default_content),
            render_tree: Renderer::parse(default_content),
            dirty: false,
            editor_state,
            _content_subscription: subscription,
        }
    }

    /// Reset to a brand new empty document.
    pub fn new_document(&mut self, cx: &mut Context<AppState>) {
        self.document = Document::new();
        self.render_tree = RenderTree(vec![]);
        self.dirty = false;
        self.editor_state.update(cx, |state, cx| {
            state.set_content("", cx);
        });
        cx.notify();
    }

    /// Load a document from disk.
    pub fn load_document(&mut self, doc: Document, cx: &mut Context<AppState>) {
        let content = doc.content.clone();
        self.render_tree = Renderer::parse(&content);
        self.editor_state.update(cx, |state, cx| {
            state.set_content(&content, cx);
        });
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
