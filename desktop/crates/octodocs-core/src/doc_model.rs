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
    /// A GFM task list item. `checked` reflects the `[ ]`/`[x]` state.
    TaskListItem { checked: bool },
    /// An inline image. `path` is the relative URL (e.g. `images/photo.png`).
    /// `height` is the display height in pixels (default 300; stored in markdown title).
    Image { path: String, alt: String, height: f32 },
    /// A GFM table. `source` is the raw markdown for round-trip serialization.
    /// `headers` and `rows` are plain-text cell contents for WYSIWYG display.
    Table {
        source: String,
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
    },
}

/// A run of text with a single inline format.
#[derive(Debug, Clone)]
pub struct InlineSpan {
    pub text: String,
    pub format: InlineFormat,
    /// Non-None only when `format == InlineFormat::Link`.
    pub link_url: Option<String>,
}

impl InlineSpan {
    /// Construct a plain/formatted span (no link).
    pub fn plain(text: impl Into<String>, format: InlineFormat) -> Self {
        Self { text: text.into(), format, link_url: None }
    }
    /// Construct a link span.
    pub fn link(text: impl Into<String>, url: impl Into<String>) -> Self {
        Self { text: text.into(), format: InlineFormat::Link, link_url: Some(url.into()) }
    }
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
    /// Hyperlink — URL stored in InlineSpan.link_url.
    Link,
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
            spans: vec![InlineSpan { text: String::new(), format: InlineFormat::Plain, link_url: None }],
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

fn inlines_to_cell_markdown(inlines: &[Inline]) -> String {
    let mut s = String::new();
    for inline in inlines {
        match inline {
            Inline::Text(t) => s.push_str(t),
            Inline::Bold(t) => {
                s.push_str("**");
                s.push_str(t);
                s.push_str("**");
            }
            Inline::Italic(t) => {
                s.push('*');
                s.push_str(t);
                s.push('*');
            }
            Inline::Underline(t) => {
                s.push_str("<u>");
                s.push_str(t);
                s.push_str("</u>");
            }
            Inline::Strikethrough(t) => {
                s.push_str("~~");
                s.push_str(t);
                s.push_str("~~");
            }
            Inline::Code(t) => {
                s.push('`');
                s.push_str(t);
                s.push('`');
            }
            Inline::Link { text, url } => {
                s.push('[');
                s.push_str(text);
                s.push_str("](");
                s.push_str(url);
                s.push(')');
            }
            Inline::Image { alt, url, .. } => {
                s.push_str("![");
                s.push_str(alt);
                s.push_str("](");
                s.push_str(url);
                s.push(')');
            }
            Inline::SoftBreak => s.push(' '),
            Inline::HardBreak => s.push(' '),
        }
    }
    s
}

fn table_to_gfm_markdown(headers: &[Vec<Inline>], rows: &[Vec<Vec<Inline>>]) -> String {
    let col_count = headers.len().max(1);
    let mut md = String::new();
    // Header row
    md.push('|');
    for cell in headers {
        md.push(' ');
        md.push_str(&inlines_to_cell_markdown(cell));
        md.push_str(" |");
    }
    md.push('\n');
    // Separator row
    md.push('|');
    for _ in 0..col_count {
        md.push_str(" --- |");
    }
    md.push('\n');
    // Data rows
    for row in rows {
        md.push('|');
        for i in 0..col_count {
            let cell_text = row.get(i)
                .map(|cell| inlines_to_cell_markdown(cell))
                .unwrap_or_default();
            md.push(' ');
            md.push_str(&cell_text);
            md.push_str(" |");
        }
        md.push('\n');
    }
    md
}

fn inlines_to_spans(inlines: &[Inline]) -> Vec<InlineSpan> {
    let mut spans: Vec<InlineSpan> = Vec::new();
    for inline in inlines {
        match inline {
            Inline::Text(t) => spans.push(InlineSpan { text: t.clone(), format: InlineFormat::Plain, link_url: None }),
            Inline::Bold(t) => spans.push(InlineSpan { text: t.clone(), format: InlineFormat::Bold, link_url: None }),
            Inline::Italic(t) => spans.push(InlineSpan { text: t.clone(), format: InlineFormat::Italic, link_url: None }),
            Inline::Underline(t) => spans.push(InlineSpan { text: t.clone(), format: InlineFormat::Underline, link_url: None }),
            Inline::Strikethrough(t) => spans.push(InlineSpan { text: t.clone(), format: InlineFormat::Strikethrough, link_url: None }),
            Inline::Code(t) => spans.push(InlineSpan { text: t.clone(), format: InlineFormat::Code, link_url: None }),
            Inline::Link { text, url, .. } => spans.push(InlineSpan { text: text.clone(), format: InlineFormat::Link, link_url: Some(url.clone()) }),
            Inline::Image { alt, .. } => spans.push(InlineSpan { text: alt.clone(), format: InlineFormat::Plain, link_url: None }),
            Inline::SoftBreak => {
                if let Some(last) = spans.last_mut() {
                    last.text.push(' ');
                } else {
                    spans.push(InlineSpan { text: " ".to_string(), format: InlineFormat::Plain, link_url: None });
                }
            }
            Inline::HardBreak => {
                if let Some(last) = spans.last_mut() {
                    last.text.push('\n');
                } else {
                    spans.push(InlineSpan { text: "\n".to_string(), format: InlineFormat::Plain, link_url: None });
                }
            }
        }
    }

    if spans.is_empty() {
        spans.push(InlineSpan { text: String::new(), format: InlineFormat::Plain, link_url: None });
    }
    spans
}

fn render_node_to_doc_paragraphs(node: &RenderNode) -> Vec<DocParagraph> {
    match node {
        RenderNode::Paragraph(inlines) => {
            // Detect a standalone image paragraph (single Inline::Image)
            if inlines.len() == 1 {
                if let Inline::Image { alt, url, height } = &inlines[0] {
                    return vec![DocParagraph {
                        kind: ParagraphKind::Image { path: url.clone(), alt: alt.clone(), height: *height },
                        spans: vec![InlineSpan { text: String::new(), format: InlineFormat::Plain, link_url: None }],
                    }];
                }
            }
            let spans = inlines_to_spans(inlines);
            vec![DocParagraph { kind: ParagraphKind::Paragraph, spans }]
        }
        RenderNode::Heading { level, text } => {
            vec![DocParagraph {
                kind: ParagraphKind::Heading(*level),
                spans: vec![InlineSpan { text: text.clone(), format: InlineFormat::Plain, link_url: None }],
            }]
        }
        RenderNode::CodeBlock { lang, content } => {
            let text = content.trim_end_matches('\n').to_string();
            vec![DocParagraph {
                kind: ParagraphKind::CodeFence(lang.clone()),
                spans: vec![InlineSpan { text, format: InlineFormat::Plain, link_url: None }],
            }]
        }
        RenderNode::MermaidBlock(src) => {
            vec![DocParagraph {
                kind: ParagraphKind::Mermaid(PathBuf::new()),
                spans: vec![InlineSpan { text: src.clone(), format: InlineFormat::Plain, link_url: None }],
            }]
        }
        RenderNode::TaskListItem { checked, inlines } => {
            let spans = inlines_to_spans(inlines);
            vec![DocParagraph { kind: ParagraphKind::TaskListItem { checked: *checked }, spans }]
        }
        RenderNode::BlockQuote(nodes) => {
            let inner_spans: Vec<InlineSpan> = nodes
                .iter()
                .flat_map(render_node_to_doc_paragraphs)
                .flat_map(|p| {
                    let mut ss = p.spans;
                    ss.push(InlineSpan { text: "\n".to_string(), format: InlineFormat::Plain, link_url: None });
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
        RenderNode::Table { headers, rows } => {
            let source = table_to_gfm_markdown(headers, rows);
            let header_texts: Vec<String> = headers.iter().map(|c| inlines_to_cell_markdown(c)).collect();
            let row_texts: Vec<Vec<String>> = rows
                .iter()
                .map(|row| row.iter().map(|c| inlines_to_cell_markdown(c)).collect())
                .collect();
            vec![DocParagraph {
                kind: ParagraphKind::Table { source, headers: header_texts, rows: row_texts },
                spans: vec![InlineSpan { text: String::new(), format: InlineFormat::Plain, link_url: None }],
            }]
        }
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
    // For Link spans also require matching URLs before merging.
    let mut merged: Vec<InlineSpan> = Vec::new();
    for span in spans {
        if span.text.is_empty() {
            continue;
        }
        if let Some(last) = merged.last_mut() {
            if last.format == span.format && last.link_url == span.link_url {
                last.text.push_str(&span.text);
                continue;
            }
        }
        {
            merged.push(InlineSpan {
                text: span.text.clone(),
                format: span.format,
                link_url: span.link_url.clone(),
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
            InlineFormat::Link => {
                let url = span.link_url.as_deref().unwrap_or("");
                s.push('[');
                s.push_str(&span.text);
                s.push_str("](");
                s.push_str(url);
                s.push(')');
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
            ParagraphKind::TaskListItem { checked } => {
                let marker = if *checked { "- [x] " } else { "- [ ] " };
                format!("{}{}", marker, spans_to_markdown(&para.spans))
            }
            ParagraphKind::Image { path, alt, height } => {
                // Only store height in title when it differs from the default (300px).
                if (*height - 300.0).abs() < 1.0 {
                    format!("![{alt}]({path})")
                } else {
                    format!("![{alt}]({path} \"{height:.0}\")")  
                }
            }
            ParagraphKind::Table { source, .. } => source.clone(),
        };
        parts.push(block);
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
    fn underline_roundtrip() {
        let md = "<u>underlined</u>\n";
        let out = roundtrip(md);
        assert!(
            out.contains("<u>underlined</u>"),
            "expected underline to round-trip, got: {out}"
        );
    }

    #[test]
    fn strikethrough_roundtrip() {
        let md = "Some ~~struck~~ text\n";
        let out = roundtrip(md);
        assert!(
            out.contains("~~struck~~"),
            "expected strikethrough to round-trip, got: {out}"
        );
    }

    #[test]
    fn coalesces_adjacent_same_format_spans() {
        let paragraphs = vec![DocParagraph {
            kind: ParagraphKind::Paragraph,
            spans: vec![
                InlineSpan {
                    text: "bold".to_string(),
                    format: InlineFormat::Bold,
                    link_url: None,
                },
                InlineSpan {
                    text: "also bold".to_string(),
                    format: InlineFormat::Bold,
                    link_url: None,
                },
            ],
        }];

        let out = doc_paragraphs_to_markdown(&paragraphs);
        assert!(!out.contains("**bold****also bold**"), "got: {out}");
        assert!(out.contains("**boldalso bold**"), "got: {out}");
    }

    #[test]
    fn preserves_empty_paragraphs() {
        let paragraphs = vec![
            DocParagraph {
                kind: ParagraphKind::Paragraph,
                spans: vec![InlineSpan {
                    text: "first".to_string(),
                    format: InlineFormat::Plain,
                    link_url: None,
                }],
            },
            DocParagraph::empty(),
            DocParagraph {
                kind: ParagraphKind::Paragraph,
                spans: vec![InlineSpan {
                    text: "third".to_string(),
                    format: InlineFormat::Plain,
                    link_url: None,
                }],
            },
        ];

        let out = doc_paragraphs_to_markdown(&paragraphs);
        assert!(out.contains("first\n\n\n\nthird\n"), "got: {out}");
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

    #[test]
    fn link_roundtrip() {
        let md = "See [docs](README.md) for details\n";
        let out = roundtrip(md);
        assert!(out.contains("[docs](README.md)"), "link should round-trip, got: {out}");
    }

    #[test]
    fn link_with_http_roundtrip() {
        let md = "Visit [site](https://example.com)\n";
        let out = roundtrip(md);
        assert!(out.contains("[site](https://example.com)"), "http link should round-trip, got: {out}");
    }

    #[test]
    fn image_roundtrip() {
        let md = "![alt text](images/photo.png)\n";
        let out = roundtrip(md);
        assert!(out.contains("![alt text](images/photo.png)"), "got: {out}");
    }

    #[test]
    fn image_height_roundtrip() {
        // Non-default height is stored as a quoted numeric title.
        let md = "![alt](images/photo.png \"450\")\n";
        let out = roundtrip(md);
        assert!(
            out.contains("![alt](images/photo.png \"450\")"),
            "expected height preserved in title, got: {out}"
        );
    }

    #[test]
    fn image_default_height_not_stored_in_title() {
        // At the default 300px height the title should be omitted entirely.
        let md = "![alt](images/photo.png)\n";
        let out = roundtrip(md);
        // Must NOT contain a " "300" " suffix.
        assert!(
            !out.contains("\"300\""),
            "default height should not appear in serialised output, got: {out}"
        );
    }

    #[test]
    fn task_list_checked_roundtrip() {
        let md = "- [x] Done task\n";
        let out = roundtrip(md);
        assert!(out.contains("- [x] Done task"), "got: {out}");
    }

    #[test]
    fn task_list_unchecked_roundtrip() {
        let md = "- [ ] Todo task\n";
        let out = roundtrip(md);
        assert!(out.contains("- [ ] Todo task"), "got: {out}");
    }

    #[test]
    fn fix_link_urls_with_spaces_roundtrip() {
        // Images with spaces in filenames should survive a parse → serialize round-trip.
        let md = "![my photo](images/my photo.png)\n";
        let out = roundtrip(md);
        // After round-trip the path will have the space (the serialiser writes the
        // clean path back; spaces in the original are normalised by copy_image_to_images_dir
        // before insertion, but the parser must at least survive without panicking).
        assert!(!out.is_empty(), "round-trip produced empty output for spaced URL");
    }

    #[test]
    fn table_link_roundtrip() {
        let md = "| Name | Ref |\n| --- | --- |\n| API | [docs](README.md) |\n";
        let out = roundtrip(md);
        assert!(
            out.contains("[docs](README.md)"),
            "table link should round-trip, got: {out}"
        );
    }

    #[test]
    fn table_http_link_roundtrip() {
        let md = "| Name | URL |\n| --- | --- |\n| Site | [Open](https://example.com) |\n";
        let out = roundtrip(md);
        assert!(
            out.contains("[Open](https://example.com)"),
            "table http link should round-trip, got: {out}"
        );
    }
}
