use adabraka_ui::prelude::*;
use octodocs_core::{Inline, RenderNode};
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::Arc;

use crate::app_state::AppState;

pub struct PreviewPane {
    pub app_state: Entity<AppState>,
}

impl PreviewPane {
    pub fn new(app_state: Entity<AppState>) -> Self {
        Self { app_state }
    }
}

impl Render for PreviewPane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = use_theme();
        let state = self.app_state.read(cx);
        let doc_dir = state.document.path
            .as_ref()
            .and_then(|p| p.parent())
            .map(|p| p.to_path_buf());
        let nodes: Vec<octodocs_core::RenderNode> = state
            .blocks
            .iter()
            .map(|b| b.node.clone())
            .collect();
        let content: Vec<AnyElement> = nodes
            .iter()
            .map(|node| render_node(node, &theme, doc_dir.as_deref()))
            .collect();

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(theme.tokens.background)
            .child(scrollable_vertical(
                div()
                    .flex()
                    .flex_col()
                    .size_full()
                    .p(px(24.0))
                    .gap(px(8.0))
                    .children(content),
            ))
    }
}

pub fn render_node(node: &RenderNode, theme: &adabraka_ui::theme::Theme, doc_dir: Option<&std::path::Path>) -> AnyElement {
    match node {
        RenderNode::Heading { level, text } => {
            let element: AnyElement = match level {
                1 => h1(text.clone()).into_any_element(),
                2 => h2(text.clone()).into_any_element(),
                3 => h3(text.clone()).into_any_element(),
                4 => h4(text.clone()).into_any_element(),
                _ => body(text.clone()).into_any_element(),
            };
            element
        }

        RenderNode::Paragraph(inlines) => {
            // Standalone image paragraph
            if inlines.len() == 1 {
                if let Inline::Image { alt, url, .. } = &inlines[0] {
                    let full_path = doc_dir
                        .map(|d| d.join(url))
                        .filter(|p| p.exists())
                        .unwrap_or_else(|| std::path::PathBuf::from(url));
                    return div()
                        .my(px(8.0))
                        .child(
                            img(Arc::<std::path::Path>::from(full_path))
                                .max_w_full()
                        )
                        .when(!alt.is_empty(), |d| d.child(
                            div().text_color(theme.tokens.muted_foreground)
                                .text_sm()
                                .child(alt.clone()),
                        ))
                        .into_any_element();
                }
            }
            let spans: Vec<AnyElement> = inlines
                .iter()
                .map(|inline| render_inline(inline, theme))
                .collect();

            div()
                .flex()
                .flex_wrap()
                .gap(px(2.0))
                .children(spans)
                .into_any_element()
        }

        RenderNode::CodeBlock { lang: _, content } => div()
            .p(px(12.0))
            .rounded(px(6.0))
            .bg(theme.tokens.muted)
            .my(px(4.0))
            .child(code(content.clone()))
            .into_any_element(),

        RenderNode::MermaidBlock(source) => {
            match ensure_mermaid_png_path(source) {
                Ok((png_path, logical_w, _logical_h)) => div()
                    .p(px(12.0))
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(theme.tokens.primary)
                    .bg(theme.tokens.card)
                    .my(px(4.0))
                    .child(muted_small("⬡ Mermaid Diagram"))
                    .child(
                        // Display at the diagram's natural (logical) width.
                        // The PNG is rasterized at 2× so it stays crisp on HiDPI.
                        img(Arc::<std::path::Path>::from(png_path.clone()))
                            .w(px(logical_w))
                            .mt(px(8.0)),
                    )
                    .into_any_element(),
                Err(error) => render_mermaid_fallback(source, theme, Some(error.to_string())),
            }
        }

        RenderNode::ThematicBreak => div()
            .w_full()
            .h(px(1.0))
            .bg(theme.tokens.border)
            .my(px(8.0))
            .into_any_element(),

        RenderNode::BlockQuote(children) => {
            let child_els: Vec<AnyElement> = children
                .iter()
                .map(|n| render_node(n, theme, doc_dir))
                .collect();

            div()
                .border_l_4()
                .border_color(theme.tokens.primary)
                .pl(px(16.0))
                .my(px(4.0))
                .children(child_els)
                .into_any_element()
        }

        RenderNode::List { ordered, items } => {
            let list_items: Vec<AnyElement> = items
                .iter()
                .enumerate()
                .map(|(i, item_nodes)| {
                    let bullet = if *ordered {
                        format!("{}.", i + 1)
                    } else {
                        "•".to_string()
                    };
                    let inner: Vec<AnyElement> = item_nodes
                        .iter()
                        .map(|n| render_node(n, theme, doc_dir))
                        .collect();

                    div()
                        .flex()
                        .gap(px(8.0))
                        .child(body_small(bullet))
                        .children(inner)
                        .into_any_element()
                })
                .collect();

            div()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .my(px(4.0))
                .children(list_items)
                .into_any_element()
        }

        RenderNode::TaskListItem { checked, inlines } => {
            let spans: Vec<AnyElement> = inlines
                .iter()
                .map(|inline| render_inline(inline, theme))
                .collect();
            let checkbox = if *checked { "\u{2611}" } else { "\u{2610}" };
            div()
                .flex()
                .gap(px(8.0))
                .my(px(2.0))
                .child(
                    div()
                        .text_color(if *checked {
                            theme.tokens.accent
                        } else {
                            theme.tokens.muted_foreground
                        })
                        .child(body_small(checkbox.to_string())),
                )
                .children(spans)
                .into_any_element()
        }

        RenderNode::Table { headers, rows } => {
            let col_count = headers.len().max(1);

            // Header row
            let header_cells: Vec<AnyElement> = headers
                .iter()
                .map(|cell| {
                    let content: Vec<AnyElement> = cell
                        .iter()
                        .map(|i| render_inline(i, theme))
                        .collect();
                    div()
                        .flex_1()
                        .px(px(10.0))
                        .py(px(7.0))
                        .border_r_1()
                        .border_color(theme.tokens.border)
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .flex()
                        .flex_wrap()
                        .gap(px(2.0))
                        .children(content)
                        .into_any_element()
                })
                .collect();

            let header_row = div()
                .flex()
                .bg(theme.tokens.muted)
                .border_b_1()
                .border_color(theme.tokens.border)
                .children(header_cells)
                .into_any_element();

            // Data rows
            let data_rows: Vec<AnyElement> = rows
                .iter()
                .enumerate()
                .map(|(row_i, row)| {
                    let cells: Vec<AnyElement> = (0..col_count)
                        .map(|ci| {
                            let cell_inlines =
                                row.get(ci).map(|v| v.as_slice()).unwrap_or(&[]);
                            let content: Vec<AnyElement> = cell_inlines
                                .iter()
                                .map(|inline| render_inline(inline, theme))
                                .collect();
                            div()
                                .flex_1()
                                .px(px(10.0))
                                .py(px(6.0))
                                .border_r_1()
                                .border_color(theme.tokens.border)
                                .flex()
                                .flex_wrap()
                                .gap(px(2.0))
                                .children(content)
                                .into_any_element()
                        })
                        .collect();
                    div()
                        .flex()
                        .when(row_i % 2 == 1, |d| d.bg(theme.tokens.muted.opacity(0.2)))
                        .border_b_1()
                        .border_color(theme.tokens.border)
                        .children(cells)
                        .into_any_element()
                })
                .collect();

            div()
                .border_1()
                .border_color(theme.tokens.border)
                .rounded(px(4.0))
                .my(px(8.0))
                .overflow_hidden()
                .child(header_row)
                .children(data_rows)
                .into_any_element()
        }
    }
}

fn render_mermaid_fallback(
    source: &str,
    theme: &adabraka_ui::theme::Theme,
    error: Option<String>,
) -> AnyElement {
    let lines: Vec<AnyElement> = source
        .lines()
        .map(|line| {
            let preserved = line.replace(' ', "\u{00A0}");
            code_small(preserved).into_any_element()
        })
        .collect();

    let mut container = div()
        .p(px(12.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(theme.tokens.primary)
        .bg(theme.tokens.muted)
        .my(px(4.0))
        .child(muted_small("⬡ Mermaid Diagram (fallback)"))
        .child(
            div()
                .mt(px(6.0))
                .p(px(8.0))
                .rounded(px(4.0))
                .bg(theme.tokens.card)
                .flex()
                .flex_col()
                .gap(px(1.0))
                .children(lines),
        );

    if let Some(error) = error {
        container = container.child(
            div()
                .mt(px(8.0))
                .child(muted_small(format!("Render error: {error}"))),
        );
    }

    container.into_any_element()
}

/// Returns `(path_to_png, logical_width, logical_height)`.
/// On cache hit the `.dims` sidecar is read so we avoid re-rendering.
fn ensure_mermaid_png_path(source: &str) -> anyhow::Result<(PathBuf, f32, f32)> {
    let mut hasher = DefaultHasher::new();
    "mermaid-cache-v5".hash(&mut hasher);
    source.hash(&mut hasher);
    let hash = hasher.finish();

    let cache_dir = std::env::temp_dir().join("octodocs-mermaid-cache");
    fs::create_dir_all(&cache_dir)?;

    let png_path: PathBuf = cache_dir.join(format!("{hash}.png"));
    let dims_path: PathBuf = cache_dir.join(format!("{hash}.dims"));

    if png_path.exists() && dims_path.exists() {
        let raw = fs::read_to_string(&dims_path)?;
        let mut parts = raw.split_whitespace();
        let w: f32 = parts.next().ok_or_else(|| anyhow::anyhow!("bad dims"))?.parse()?;
        let h: f32 = parts.next().ok_or_else(|| anyhow::anyhow!("bad dims"))?.parse()?;
        return Ok((png_path, w, h));
    }

    let (lw, lh) = octodocs_core::mermaid::render_png(source, &png_path)?;
    fs::write(&dims_path, format!("{lw} {lh}"))?;
    Ok((png_path, lw, lh))
}

fn render_inline(inline: &Inline, theme: &adabraka_ui::theme::Theme) -> AnyElement {
    match inline {
        Inline::Text(t) => body(t.clone()).into_any_element(),
        Inline::Bold(t) => Text::new(t.clone())
            .weight(gpui::FontWeight::BOLD)
            .into_any_element(),
        Inline::Italic(t) => Text::new(t.clone())
            .italic()
            .into_any_element(),
        Inline::Underline(t) => Text::new(t.clone())
            .underline()
            .into_any_element(),
        Inline::Strikethrough(t) => Text::new(t.clone())
            .strikethrough()
            .into_any_element(),
        Inline::Code(t) => code_small(t.clone()).into_any_element(),
        Inline::Link { text, .. } => Text::new(text.clone())
            .color(theme.tokens.primary)
            .underline()
            .into_any_element(),
        Inline::Image { alt, .. } => muted_small(format!("[img: {}]", alt)).into_any_element(),
        Inline::SoftBreak | Inline::HardBreak => div().w(px(4.0)).into_any_element(),
    }
}
