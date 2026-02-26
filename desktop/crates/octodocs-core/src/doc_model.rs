//! Rich document model for the word-style continuous editor.
//!
//! This module defines `DocParagraph` (the mutable rich model), `DocCursor`, and
//! `DocSelection`, plus conversion helpers between `Vec<DocParagraph>` and the
//! existing `RenderNode` / `DocumentBlock` types.

use std::path::PathBuf;

use crate::renderer::{Inline, RenderNode, Renderer};

// ─────────────────────────────────────────────────────────────
// Core types
// ─────────────────────────────────────────────────────────────

/// A single top-level block in the rich editor model.
#[derive(Debug, Clone)]
pub struct DocParagraph {
    pub kind: ParagraphKind,
    pub spans: Vec<InlineSpan>,
}

/// Block-level kind of a paragraph.
#[derive(Debug, Clone, PartialEq)]
pub enum ParagraphKind {
    Paragraph,
    Heading(u8),
    CodeFence(Option<String>),
    BlockQuote,
    /// A Mermaid diagram island. The path is the cached PNG (may be empty
    /// until the render task completes). The source is stored in spans[0].
    Mermaid(PathBuf),
}

/// A run of text with a single inline format.
#[derive(Debug, Clone)]
pub struct InlineSpan {
    pub text: String,
    pub format: InlineFormat,
}

/// Inline formatting variants (simplified for MVP).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InlineFormat {
    Plain,
    Bold,
    Italic,
    Underline,
    Strikethrough,
    Code,
}

/// Cursor position inside the document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DocCursor {
    /// Index into the `Vec<DocParagraph>`.
    pub para_idx: usize,
    /// Character (Unicode scalar) offset within the paragraph's flat text.
    pub char_offset: usize,
}

impl DocCursor {
    pub fn zero() -> Self {
        Self { para_idx: 0, char_offset: 0 }
    }
}

impl PartialOrd for DocCursor {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for DocCursor {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.para_idx
            .cmp(&other.para_idx)
            .then(self.char_offset.cmp(&other.char_offset))
    }
}

/// An anchor+focus selection inside the document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DocSelection {
    pub anchor: DocCursor,
    pub focus: DocCursor,
}

impl DocSelection {
    /// Return `(start, end)` in document order.
    pub fn ordered(&self) -> (DocCursor, DocCursor) {
        if self.anchor <= self.focus {
            (self.anchor, self.focus)
        } else {
            (self.focus, self.anchor)
        }
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.focus
    }
}

// ─────────────────────────────────────────────────────────────
// DocParagraph helpers
// ─────────────────────────────────────────────────────────────

impl DocParagraph {
    /// Concatenate all span texts.
    pub fn plain_text(&self) -> String {
        self.spans.iter().map(|s| s.as_str()).collect()
    }

    /// Number of Unicode scalar values in the paragraph.
    pub fn char_count(&self) -> usize {
        self.spans.iter().map(|s| s.text.chars().count()).sum()
    }

    /// New empty paragraph.
    pub fn empty() -> Self {
        Self {
            kind: ParagraphKind::Paragraph,
            spans: vec![InlineSpan { text: String::new(), format: InlineFormat::Plain }],
        }
    }
}

impl InlineSpan {
    pub fn as_str(&self) -> &str {
        &self.text
    }
}

// ─────────────────────────────────────────────────────────────
// RenderNode → DocParagraph conversion
// ─────────────────────────────────────────────────────────────

fn inlines_to_spans(inlines: &[Inline]) -> Vec<InlineSpan> {
    let mut spans: Vec<InlineSpan> = Vec::new();
    for inline in inlines {
        match inline {
            Inline::Text(t) => spans.push(InlineSpan { text: t.clone(), format: InlineFormat::Plain }),
            Inline::Bold(t) => spans.push(InlineSpan { text: t.clone(), format: InlineFormat::Bold }),
            Inline::Italic(t) => spans.push(InlineSpan { text: t.clone(), format: InlineFormat::Italic }),
            Inline::Underline(t) => spans.push(InlineSpan { text: t.clone(), format: InlineFormat::Underline }),
            Inline::Strikethrough(t) => spans.push(InlineSpan { text: t.clone(), format: InlineFormat::Strikethrough }),
            Inline::Code(t) => spans.push(InlineSpan { text: t.clone(), format: InlineFormat::Code }),
            Inline::Link { text, .. } => spans.push(InlineSpan { text: text.clone(), format: InlineFormat::Plain }),
            Inline::Image { alt, .. } => spans.push(InlineSpan { text: alt.clone(), format: InlineFormat::Plain }),
            Inline::SoftBreak => {
                if let Some(last) = spans.last_mut() {
                    last.text.push(' ');
                } else {
                    spans.push(InlineSpan { text: " ".to_string(), format: InlineFormat::Plain });
                }
            }
            Inline::HardBreak => {
                if let Some(last) = spans.last_mut() {
                    last.text.push('\n');
                } else {
                    spans.push(InlineSpan { text: "\n".to_string(), format: InlineFormat::Plain });
                }
            }
        }
    }

    if spans.is_empty() {
        spans.push(InlineSpan { text: String::new(), format: InlineFormat::Plain });
    }
    spans
}

fn render_node_to_doc_paragraphs(node: &RenderNode) -> Vec<DocParagraph> {
    match node {
        RenderNode::Paragraph(inlines) => {
            let spans = inlines_to_spans(inlines);
            vec![DocParagraph { kind: ParagraphKind::Paragraph, spans }]
        }
        RenderNode::Heading { level, text } => {
            vec![DocParagraph {
                kind: ParagraphKind::Heading(*level),
                spans: vec![InlineSpan { text: text.clone(), format: InlineFormat::Plain }],
            }]
        }
        RenderNode::CodeBlock { lang, content } => {
            let text = content.trim_end_matches('\n').to_string();
            vec![DocParagraph {
                kind: ParagraphKind::CodeFence(lang.clone()),
                spans: vec![InlineSpan { text, format: InlineFormat::Plain }],
            }]
        }
        RenderNode::MermaidBlock(src) => {
            vec![DocParagraph {
                kind: ParagraphKind::Mermaid(PathBuf::new()),
                spans: vec![InlineSpan { text: src.clone(), format: InlineFormat::Plain }],
            }]
        }
        RenderNode::BlockQuote(nodes) => {
            let inner_spans: Vec<InlineSpan> = nodes
                .iter()
                .flat_map(render_node_to_doc_paragraphs)
                .flat_map(|p| {
                    let mut ss = p.spans;
                    ss.push(InlineSpan { text: "\n".to_string(), format: InlineFormat::Plain });
                    ss
                })
                .collect();
            // Remove trailing newline span
            let mut inner_spans = inner_spans;
            if inner_spans.last().map(|s| s.text.as_str()) == Some("\n") {
                inner_spans.pop();
            }
            vec![DocParagraph { kind: ParagraphKind::BlockQuote, spans: inner_spans }]
        }
        RenderNode::List { items, .. } => {
            items
                .iter()
                .flat_map(|item_nodes| item_nodes.iter().flat_map(render_node_to_doc_paragraphs))
                .collect()
        }
        RenderNode::ThematicBreak => vec![],
    }
}

/// Convert a stream of `RenderNode`s (from `Renderer::parse()`) into `DocParagraph`s.
pub fn render_nodes_to_doc_paragraphs(nodes: &[RenderNode]) -> Vec<DocParagraph> {
    let mut result: Vec<DocParagraph> = nodes
        .iter()
        .flat_map(render_node_to_doc_paragraphs)
        .collect();
    if result.is_empty() {
        result.push(DocParagraph::empty());
    }
    result
}

/// Parse a Markdown string directly into `Vec<DocParagraph>`.
pub fn markdown_to_doc_paragraphs(markdown: &str) -> Vec<DocParagraph> {
    let nodes = Renderer::parse(markdown).0;
    render_nodes_to_doc_paragraphs(&nodes)
}

// ─────────────────────────────────────────────────────────────
// DocParagraph → Markdown serialisation
// ─────────────────────────────────────────────────────────────

fn spans_to_markdown(spans: &[InlineSpan]) -> String {
    // Coalesce adjacent spans that share the same format so serialization
    // doesn't emit redundant marker sequences like `********`.
    let mut merged: Vec<InlineSpan> = Vec::new();
    for span in spans {
        if span.text.is_empty() {
            continue;
        }
        if let Some(last) = merged.last_mut() {
            if last.format == span.format {
                last.text.push_str(&span.text);
                continue;
            }
        }
        {
            merged.push(InlineSpan {
                text: span.text.clone(),
                format: span.format,
            });
        }
    }

    let mut s = String::new();
    for span in &merged {
        match span.format {
            InlineFormat::Plain => s.push_str(&span.text),
            InlineFormat::Bold => {
                s.push_str("**");
                s.push_str(&span.text);
                s.push_str("**");
            }
            InlineFormat::Italic => {
                s.push('*');
                s.push_str(&span.text);
                s.push('*');
            }
            InlineFormat::Underline => {
                s.push_str("<u>");
                s.push_str(&span.text);
                s.push_str("</u>");
            }
            InlineFormat::Strikethrough => {
                s.push_str("~~");
                s.push_str(&span.text);
                s.push_str("~~");
            }
            InlineFormat::Code => {
                s.push('`');
                s.push_str(&span.text);
                s.push('`');
            }
        }
    }
    s
}

/// Serialise a slice of `DocParagraph`s to Markdown.
pub fn doc_paragraphs_to_markdown(paragraphs: &[DocParagraph]) -> String {
    let mut parts: Vec<String> = Vec::new();
    for para in paragraphs {
        let block = match &para.kind {
            ParagraphKind::Paragraph => {
                spans_to_markdown(&para.spans)
            }
            ParagraphKind::Heading(level) => {
                let prefix = "#".repeat(*level as usize);
                let text = para.plain_text();
                format!("{} {}", prefix, text)
            }
            ParagraphKind::CodeFence(lang) => {
                let lang_str = lang.as_deref().unwrap_or("");
                let content = para.plain_text();
                format!("```{}\n{}\n```", lang_str, content)
            }
            ParagraphKind::BlockQuote => {
                let text = spans_to_markdown(&para.spans);
                text.lines()
                    .map(|line| format!("> {}", line))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
            ParagraphKind::Mermaid(_) => {
                let src = para.plain_text();
                format!("```mermaid\n{}\n```", src)
            }
        };
        if !block.is_empty() {
            parts.push(block);
        }
    }
    let mut result = parts.join("\n\n");
    if !result.is_empty() {
        result.push('\n');
    }
    result
}

// ─────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(input: &str) -> String {
        let paras = markdown_to_doc_paragraphs(input);
        doc_paragraphs_to_markdown(&paras)
    }

    #[test]
    fn paragraph_roundtrip() {
        let md = "Hello world\n";
        let out = roundtrip(md);
        assert!(out.contains("Hello world"), "got: {out}");
    }

    #[test]
    fn heading_roundtrip() {
        let md = "# My Heading\n";
        let out = roundtrip(md);
        assert!(out.contains("# My Heading"), "got: {out}");
    }

    #[test]
    fn bold_roundtrip() {
        let md = "Some **bold** text\n";
        let out = roundtrip(md);
        assert!(out.contains("**bold**"), "got: {out}");
    }

    #[test]
    fn code_fence_roundtrip() {
        let md = "```rust\nfn main() {}\n```\n";
        let out = roundtrip(md);
        assert!(out.contains("```rust"), "got: {out}");
        assert!(out.contains("fn main()"), "got: {out}");
    }

    #[test]
    fn empty_doc_has_one_paragraph() {
        let paras = markdown_to_doc_paragraphs("");
        assert_eq!(paras.len(), 1);
    }

    #[test]
    fn char_count() {
        let mut para = DocParagraph::empty();
        para.spans[0].text = "hello".to_string();
        assert_eq!(para.char_count(), 5);
    }
}
