use adabraka_ui::prelude::*;
use gpui::MouseButton;
use octodocs_core::{DocumentBlock, Renderer};

use super::preview_pane::render_node;
use crate::app_state::AppState;
use crate::rich_block_editor::RichBlockEditor;

/// Full-width WYSIWYG pane.
///
/// Every top-level markdown block is rendered as formatted output.
/// Clicking a block switches it to an inline raw-text editor; all other
/// blocks stay rendered. Markdown syntax is **never** visible unless a
/// block is being edited.
///
/// # Visual design
///
/// Each block has a 4 px left accent bar (like blockquotes).
/// - **Inactive at rest**: transparent bar, no background.
/// - **Inactive on hover**: `border` colored bar, faint `muted` tint.
/// - **Active**: `primary` colored bar, `card` background.
///
/// Padding is identical in all states (`pl` = bar_width + content_pl) so
/// there is **no layout jump** when a block is activated or deactivated.
pub struct BlockEditorPane {
    pub app_state: Entity<AppState>,
}

impl BlockEditorPane {
    pub fn new(app_state: Entity<AppState>) -> Self {
        Self { app_state }
    }
}

impl Render for BlockEditorPane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = use_theme();

        let app = self.app_state.read(cx);
        let active = app.active_block;
        let editor_state = app.editor_state.clone();
        let active_rich_block = app.active_rich_block.clone();
        let blocks: Vec<DocumentBlock> = app.blocks.clone();
        let app_weak = self.app_state.downgrade();
        drop(app);

        // ─── Shared geometry constants ─────────────────────────────────
        // block_pl is the left padding to the RIGHT of the 4 px bar.
        let block_pl = px(14.0); // bar eats 4 px; this gives 18 px total indent
        let block_py = px(6.0);
        let block_mb = px(4.0);

        let mut block_elements: Vec<AnyElement> = Vec::new();

        for (i, block) in blocks.iter().enumerate() {
            // Adaptive editor height: 1 line per source line, minimum 1.
            let src_lines = block.source.trim_end().lines().count().max(1);

            let el: AnyElement = if active == Some(i) {
                // ── Active block ─────────────────────────────────────────
                // Primary left-bar accent + subtle card bg.
                // No box border on the container — the editor sits flush.
                let aw = app_weak.clone();
                div()
                    .w_full()
                    .border_l_4()
                    .border_color(theme.tokens.primary)
                    .pl(block_pl)
                    .py(block_py)
                    .mb(block_mb)
                    .rounded_r(px(4.0))
                    .bg(theme.tokens.card)
                    // Re-clicking an already-active block is a no-op
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        let _ = aw.update(cx, |state, cx| state.activate_block(i, cx));
                    })
                    .child(
                        if let Some(rb_state) = &active_rich_block {
                            RichBlockEditor::new(rb_state).into_any_element()
                        } else {
                            Editor::new(&editor_state)
                                .min_lines(src_lines)
                                .show_line_numbers(false, cx)
                                .into_any_element()
                        },
                    )
                    .into_any_element()
            } else {
                // ── Inactive block ────────────────────────────────────────
                // Transparent bar at rest; border-colored on hover.
                // Hover also adds a gentle background tint.
                let aw = app_weak.clone();

                let nodes = Renderer::parse(&block.source).0;
                let rendered: Vec<AnyElement> =
                    nodes.iter().map(|n| render_node(n, &theme)).collect();

                // Use opacity(0.0) to keep a transparent border that still
                // reserves the 4 px so padding never shifts.
                let bar_rest = theme.tokens.border.opacity(0.0);
                let bar_hover = theme.tokens.border;
                let bg_hover = theme.tokens.muted.opacity(0.4);

                div()
                    .w_full()
                    .border_l_4()
                    .border_color(bar_rest)
                    .pl(block_pl)
                    .py(block_py)
                    .mb(block_mb)
                    .rounded_r(px(4.0))
                    .cursor_text()
                    .hover(|s| s.border_color(bar_hover).bg(bg_hover))
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        let _ = aw.update(cx, |state, cx| state.activate_block(i, cx));
                    })
                    .children(rendered)
                    .into_any_element()
            };

            block_elements.push(el);
        }

        // ── Trailing click zone ──────────────────────────────────────────
        // Always visible so the user can append content after any block,
        // including a Mermaid diagram or other non-editable element.
        {
            let aw = app_weak.clone();
            let block_count = blocks.len();
            let hint_rest = theme.tokens.muted_foreground.opacity(0.35);
            let hint_hover = theme.tokens.muted_foreground.opacity(0.75);

            let hint = div()
                .w_full()
                .h(px(44.0))
                .flex()
                .items_center()
                // Same left indent as block content (4 px bar + 14 px padding).
                .pl(px(18.0))
                .cursor_text()
                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                    let _ = aw.update(cx, |state, cx| {
                        let last_is_empty = state
                            .blocks
                            .last()
                            .map(|b| b.source.trim().is_empty())
                            .unwrap_or(false);
                        if last_is_empty && !state.blocks.is_empty() {
                            let last = state.blocks.len() - 1;
                            state.activate_block(last, cx);
                        } else {
                            let node = octodocs_core::RenderNode::Paragraph(vec![]);
                            state.blocks.push(DocumentBlock {
                                source: "\n".to_string(),
                                node,
                            });
                            state.activate_block(block_count, cx);
                        }
                    });
                })
                .child(
                    div()
                        .text_color(hint_rest)
                        .hover(|s| s.text_color(hint_hover))
                        .child(muted_small("Click here or press Enter to add a new block…")),
                )
                .into_any_element();

            block_elements.push(hint);
        }

        scrollable_vertical(
            div()
                .flex()
                .flex_col()
                .w_full()
                .px(px(48.0))
                .py(px(32.0))
                .max_w(px(760.0))
                .mx_auto()
                .children(block_elements),
        )
    }
}
