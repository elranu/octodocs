use adabraka_ui::prelude::*;

use crate::app_state::AppState;

/// Full-width WYSIWYG pane built on the word-style continuous document editor.
///
/// The entire document is a single [`DocumentEditor`] surface — the cursor
/// flows freely across paragraph boundaries and markdown syntax is never
/// visible. Formatting is applied via the toolbar or keyboard shortcuts.
pub struct DocumentEditorPane {
    pub app_state: Entity<AppState>,
}

impl DocumentEditorPane {
    pub fn new(app_state: Entity<AppState>) -> Self {
        Self { app_state }
    }
}

impl Render for DocumentEditorPane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let editor_state = self.app_state.read(cx).doc_editor.clone();
        scrollable_vertical(
            div()
                .size_full()
                .child(DocumentEditor::new(&editor_state)),
        )
    }
}
