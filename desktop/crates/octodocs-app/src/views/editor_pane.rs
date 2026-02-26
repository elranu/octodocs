use adabraka_ui::prelude::*;

use crate::app_state::AppState;

pub struct EditorPane {
    pub app_state: Entity<AppState>,
}

impl EditorPane {
    pub fn new(app_state: Entity<AppState>) -> Self {
        Self { app_state }
    }
}

impl Render for EditorPane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let editor_state = self.app_state.read(cx).full_editor_state.clone();

        div()
            .flex()
            .flex_col()
            .size_full()
            .child(
                Editor::new(&editor_state)
                    .min_lines(20)
                    .show_line_numbers(true, cx),
            )
    }
}
