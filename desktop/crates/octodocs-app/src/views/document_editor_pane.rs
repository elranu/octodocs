use adabraka_ui::prelude::*;
use std::{path::Path, sync::Arc};

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
        let zoom_path = editor_state.read(cx).image_zoom.clone();
        let close_weak = editor_state.downgrade();

        let editor_scroll = scrollable_vertical(
            div()
                .size_full()
                .child(DocumentEditor::new(&editor_state)),
        );

        let mut root = div().relative().size_full().child(editor_scroll);

        if let Some(img_path) = zoom_path {
            let arc_path: Arc<Path> = img_path.into();
            root = root.child(
                div()
                    .absolute()
                    .inset_0()
                    .bg(gpui::hsla(0.0, 0.0, 0.0, 0.88))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                        let _ = close_weak.update(cx, |s, cx| {
                            s.image_zoom = None;
                            cx.notify();
                        });
                    })
                    .child(
                        img(arc_path)
                            .max_w(relative(0.95))
                            .max_h(relative(0.95)),
                    ),
            );
        }

        root
    }
}
