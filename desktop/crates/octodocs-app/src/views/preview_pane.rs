use adabraka_ui::prelude::*;
use octodocs_core::{Inline, RenderNode};

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
        let nodes = self.app_state.read(cx).render_tree.0.clone();

        let content: Vec<AnyElement> = nodes
            .iter()
            .map(|node| render_node(node, &theme))
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

fn render_node(node: &RenderNode, theme: &adabraka_ui::theme::Theme) -> AnyElement {
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
            let lines: Vec<AnyElement> = source
                .lines()
                .map(|line| {
                    let preserved = line.replace(' ', "\u{00A0}");
                    code_small(preserved).into_any_element()
                })
                .collect();

            div()
                .p(px(12.0))
                .rounded(px(6.0))
                .border_1()
                .border_color(theme.tokens.primary)
                .bg(theme.tokens.muted)
                .my(px(4.0))
                .child(muted_small("⬡ Mermaid Diagram"))
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
                )
                .into_any_element()
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
                .map(|n| render_node(n, theme))
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
                        .map(|n| render_node(n, theme))
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
    }
}

fn render_inline(inline: &Inline, theme: &adabraka_ui::theme::Theme) -> AnyElement {
    match inline {
        Inline::Text(t) => body(t.clone()).into_any_element(),
        Inline::Bold(t) => Text::new(t.clone())
            .weight(gpui::FontWeight::BOLD)
            .into_any_element(),
        Inline::Italic(t) => Text::new(t.clone())
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
