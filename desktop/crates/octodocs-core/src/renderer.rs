use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

// ──────────────────────────────────────────────────────────────────
// Data model
// ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct RenderTree(pub Vec<RenderNode>);

/// A single top-level block in a WYSIWYG document.
/// Carries both the raw markdown source (for the inline editor) and the
/// pre-parsed node (for rendered display).
#[derive(Debug, Clone)]
pub struct DocumentBlock {
    /// Raw markdown source for this block, always ends with `\n`.
    pub source: String,
    /// Pre-parsed render node (reparsed whenever source changes).
    pub node: RenderNode,
}

impl DocumentBlock {
    /// Reconstruct the full markdown document from a slice of blocks.
    /// Blocks are joined with a blank line (each source already ends with `\n`).
    pub fn reassemble(blocks: &[DocumentBlock]) -> String {
        blocks
            .iter()
            .map(|b| b.source.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[derive(Debug, Clone)]
pub enum RenderNode {
    Heading { level: u8, text: String },
    Paragraph(Vec<Inline>),
    CodeBlock { lang: Option<String>, content: String },
    /// A Mermaid diagram — content is the raw diagram source.
    MermaidBlock(String),
    ThematicBreak,
    BlockQuote(Vec<RenderNode>),
    List { ordered: bool, items: Vec<Vec<RenderNode>> },
    /// A GFM task list item (`- [ ]` or `- [x]`).
    TaskListItem { checked: bool, inlines: Vec<Inline> },
    /// A GFM table. `headers` is the header row; `rows` are the body rows.
    /// Each cell is a list of `Inline` nodes (may have bold/italic etc.).
    Table {
        headers: Vec<Vec<Inline>>,
        rows: Vec<Vec<Vec<Inline>>>,
    },
}

#[derive(Debug, Clone)]
pub enum Inline {
    Text(String),
    Bold(String),
    Italic(String),
    Underline(String),
    Strikethrough(String),
    Code(String),
    Link { text: String, url: String },
    Image { alt: String, url: String, height: f32 },
    SoftBreak,
    HardBreak,
}

// ──────────────────────────────────────────────────────────────────
// Preprocessing helpers
// ──────────────────────────────────────────────────────────────────

/// CommonMark does not allow bare spaces inside `](url)` link/image destinations.
/// This function rewrites `](url with spaces)` → `](<url with spaces>)` so that
/// pulldown-cmark can parse them correctly.  Already angle-bracketed URLs are
/// left untouched.  Code-span / fenced-code content is skipped to avoid false
/// positives.
fn fix_link_urls_with_spaces(md: &str) -> std::borrow::Cow<'_, str> {
    // Fast path: no bareword link suspects.
    if !md.contains("](") {
        return std::borrow::Cow::Borrowed(md);
    }

    let mut out = String::with_capacity(md.len() + 32);
    let mut rest = md;
    let mut modified = false;

    while let Some(idx) = rest.find("](") {
        // Copy everything up to and including `](`
        out.push_str(&rest[..idx + 2]);
        rest = &rest[idx + 2..];

        // Skip if already angle-bracketed or looks like an empty ref `]()`
        if rest.starts_with('<') || rest.starts_with(')') {
            continue;
        }

        // Find the closing `)`. We do a naive scan — safe enough for image
        // paths that won't contain nested parentheses.
        if let Some(close) = rest.find(')') {
            let inner = &rest[..close];   // everything between `(` and `)`
            if inner.contains(' ') && !inner.starts_with('<') {
                // Separate optional quoted title at the end:
                // e.g.  `images/foo bar.png "My Title"`
                let (url_part, title_part) = split_url_title(inner);
                out.push('<');
                out.push_str(url_part.trim_end());
                out.push('>');
                if !title_part.is_empty() {
                    out.push(' ');
                    out.push_str(title_part);
                }
                modified = true;
            } else {
                out.push_str(inner);
            }
            out.push(')');
            rest = &rest[close + 1..];
        }
    }

    out.push_str(rest);
    if modified { std::borrow::Cow::Owned(out) } else { std::borrow::Cow::Borrowed(md) }
}

/// Split `url "title"` or `url 'title'` into `(url, "title")`.
/// Returns `(full, "")` when there is no title.
fn split_url_title(s: &str) -> (&'_ str, &'_ str) {
    let s = s.trim_end();
    // Check for trailing `"..."` or `'...'`
    if let Some(q) = s.rfind('"').or_else(|| s.rfind('\'')) {
        let ch = s.as_bytes()[q] as char;
        let tail = &s[q..];
        if tail.starts_with(ch) && tail.ends_with(ch) && tail.len() >= 2 {
            let before = s[..q].trim_end();
            if before.contains(' ') || before.is_empty() {
                // Looks like a real title
                return (before, tail);
            }
        }
    }
    (s, "")
}

// ──────────────────────────────────────────────────────────────────
// Renderer
// ──────────────────────────────────────────────────────────────────

pub struct Renderer;

impl Renderer {
    /// Split a markdown document into top-level `DocumentBlock`s.
    /// Each block carries its raw source text (for the inline editor) and its
    /// pre-parsed `RenderNode` (for rendered display).
    pub fn parse_blocks(markdown: &str) -> Vec<DocumentBlock> {
        let options = Options::ENABLE_TABLES
            | Options::ENABLE_FOOTNOTES
            | Options::ENABLE_STRIKETHROUGH
            | Options::ENABLE_TASKLISTS
            | Options::ENABLE_GFM;

        let mut blocks: Vec<DocumentBlock> = Vec::new();
        let mut depth: usize = 0;
        let mut block_start: usize = 0;

        for (event, range) in Parser::new_ext(markdown, options).into_offset_iter() {
            match event {
                Event::Start(_) => {
                    if depth == 0 {
                        block_start = range.start;
                    }
                    depth += 1;
                }
                Event::End(_) => {
                    depth -= 1;
                    if depth == 0 {
                        let src = markdown[block_start..range.end.min(markdown.len())]
                            .trim_end()
                            .to_string();
                        if !src.is_empty() {
                            let node = Self::parse(&src)
                                .0
                                .into_iter()
                                .next()
                                .unwrap_or(RenderNode::Paragraph(vec![]));
                            blocks.push(DocumentBlock { source: src + "\n", node });
                        }
                    }
                }
                // Thematic break (---) is a single event with no Start/End wrapper.
                Event::Rule if depth == 0 => {
                    let src = markdown[range].trim_end().to_string();
                    if !src.is_empty() {
                        blocks.push(DocumentBlock {
                            source: src + "\n",
                            node: RenderNode::ThematicBreak,
                        });
                    }
                }
                _ => {}
            }
        }

        blocks
    }

    pub fn parse(markdown: &str) -> RenderTree {
        let options = Options::ENABLE_TABLES
            | Options::ENABLE_FOOTNOTES
            | Options::ENABLE_STRIKETHROUGH
            | Options::ENABLE_TASKLISTS
            | Options::ENABLE_GFM;

        let preprocessed = fix_link_urls_with_spaces(markdown);
        let parser = Parser::new_ext(&preprocessed, options);
        let mut nodes: Vec<RenderNode> = Vec::new();

        // Simple flat state machine — good enough for v1.
        let mut in_paragraph = false;
        let mut inline_buf: Vec<Inline> = Vec::new();
        let mut in_heading: Option<u8> = None;
        let mut heading_text = String::new();
        let mut in_code_block = false;
        let mut code_lang: Option<String> = None;
        let mut code_buf = String::new();
        let mut bold = false;
        let mut italic = false;
        let mut strikethrough = false;
        let mut underline = false;
        // List / task list state
        let mut in_list_item = false;
        let mut task_list_checked: Option<bool> = None;
        // Image state
        let mut in_image = false;
        let mut image_url = String::new();
        let mut image_height: f32 = 300.0;
        let mut image_alt_buf = String::new();
        // Table state
        let mut in_table_cell = false;
        let mut table_headers: Vec<Vec<Inline>> = Vec::new();
        let mut table_rows: Vec<Vec<Vec<Inline>>> = Vec::new();
        let mut table_current_row: Vec<Vec<Inline>> = Vec::new();
        let mut table_cell_buf: Vec<Inline> = Vec::new();

        for event in parser {
            match event {
                // ── Headings ──────────────────────────────────────
                Event::Start(Tag::Heading { level, .. }) => {
                    in_heading = Some(heading_level_to_u8(level));
                    heading_text.clear();
                }
                Event::End(TagEnd::Heading(_)) => {
                    if let Some(lvl) = in_heading.take() {
                        nodes.push(RenderNode::Heading {
                            level: lvl,
                            text: heading_text.clone(),
                        });
                    }
                }

                // ── Tables ───────────────────────────────────────
                Event::Start(Tag::Table(_)) => {
                    in_table_cell = false;
                    table_headers.clear();
                    table_rows.clear();
                    table_current_row.clear();
                    table_cell_buf.clear();
                }
                Event::End(TagEnd::Table) => {
                    nodes.push(RenderNode::Table {
                        headers: table_headers.clone(),
                        rows: table_rows.clone(),
                    });
                    table_headers.clear();
                    table_rows.clear();
                }
                Event::Start(Tag::TableHead) => {
                    table_current_row.clear();
                }
                Event::End(TagEnd::TableHead) => {
                    table_headers = table_current_row.clone();
                    table_current_row.clear();
                }
                Event::Start(Tag::TableRow) => { table_current_row.clear(); }
                Event::End(TagEnd::TableRow) => {
                    table_rows.push(table_current_row.clone());
                    table_current_row.clear();
                }
                Event::Start(Tag::TableCell) => {
                    in_table_cell = true;
                    table_cell_buf.clear();
                    bold = false; italic = false; strikethrough = false; underline = false;
                }
                Event::End(TagEnd::TableCell) => {
                    in_table_cell = false;
                    table_current_row.push(table_cell_buf.clone());
                    table_cell_buf.clear();
                }

                // ── Paragraphs ────────────────────────────────────
                Event::Start(Tag::Paragraph) => {
                    in_paragraph = true;
                    inline_buf.clear();
                }
                Event::End(TagEnd::Paragraph) => {
                    in_paragraph = false;
                    if !inline_buf.is_empty() {
                        if in_list_item {
                            if let Some(checked) = task_list_checked {
                                nodes.push(RenderNode::TaskListItem { checked, inlines: inline_buf.clone() });
                            } else {
                                nodes.push(RenderNode::Paragraph(inline_buf.clone()));
                            }
                        } else {
                            nodes.push(RenderNode::Paragraph(inline_buf.clone()));
                        }
                        inline_buf.clear();
                    }
                }

                // ── List items ────────────────────────────────────
                // ── Images ───────────────────────────────────────────────────
                Event::Start(Tag::Image { dest_url, title, .. }) => {
                    in_image = true;
                    image_url = dest_url.to_string();
                    // Parse height from title (stored as integer pixel string e.g. "300")
                    image_height = title.parse::<f32>().unwrap_or(300.0);
                    image_alt_buf.clear();
                }
                Event::End(TagEnd::Image) => {
                    if in_image {
                        let inline = Inline::Image {
                            alt: image_alt_buf.clone(),
                            url: image_url.clone(),
                            height: image_height,
                        };
                        if in_paragraph {
                            inline_buf.push(inline);
                        }
                        in_image = false;
                    }
                }

                Event::Start(Tag::Item) => {
                    in_list_item = true;
                    task_list_checked = None;
                }
                Event::End(TagEnd::Item) => {
                    in_list_item = false;
                    task_list_checked = None;
                }
                Event::TaskListMarker(checked) => {
                    if in_list_item {
                        task_list_checked = Some(checked);
                    }
                }

                // ── Code blocks ───────────────────────────────────
                Event::Start(Tag::CodeBlock(kind)) => {
                    in_code_block = true;
                    code_buf.clear();
                    code_lang = match kind {
                        CodeBlockKind::Fenced(lang) => {
                            let l = lang.trim().to_string();
                            if l.is_empty() { None } else { Some(l) }
                        }
                        CodeBlockKind::Indented => None,
                    };
                }
                Event::End(TagEnd::CodeBlock) => {
                    in_code_block = false;
                    let is_mermaid = code_lang
                        .as_deref()
                        .map(|l| l.to_lowercase() == "mermaid")
                        .unwrap_or(false);

                    if is_mermaid {
                        nodes.push(RenderNode::MermaidBlock(code_buf.clone()));
                    } else {
                        nodes.push(RenderNode::CodeBlock {
                            lang: code_lang.clone(),
                            content: code_buf.clone(),
                        });
                    }
                    code_lang = None;
                }

                // ── Emphasis / Strong ─────────────────────────────
                Event::Start(Tag::Strong) => bold = true,
                Event::End(TagEnd::Strong) => bold = false,
                Event::Start(Tag::Emphasis) => italic = true,
                Event::End(TagEnd::Emphasis) => italic = false,
                Event::Start(Tag::Strikethrough) => strikethrough = true,
                Event::End(TagEnd::Strikethrough) => strikethrough = false,

                Event::Html(html) | Event::InlineHtml(html) => {
                    let token = html.trim().to_lowercase();
                    if token == "<u>" || token == "<ins>" {
                        underline = true;
                    } else if token == "</u>" || token == "</ins>" {
                        underline = false;
                    }
                }

                // ── Text ──────────────────────────────────────────
                Event::Text(text) => {
                    let s = text.into_string();
                    if in_image {
                        image_alt_buf.push_str(&s);
                    } else if in_code_block {
                        code_buf.push_str(&s);
                    } else if in_table_cell {
                        let inline = if bold {
                            Inline::Bold(s)
                        } else if italic {
                            Inline::Italic(s)
                        } else if strikethrough {
                            Inline::Strikethrough(s)
                        } else {
                            Inline::Text(s)
                        };
                        table_cell_buf.push(inline);
                    } else if in_heading.is_some() {
                        heading_text.push_str(&s);
                    } else if in_paragraph {
                        let inline = if bold {
                            Inline::Bold(s)
                        } else if italic {
                            Inline::Italic(s)
                        } else if underline {
                            Inline::Underline(s)
                        } else if strikethrough {
                            Inline::Strikethrough(s)
                        } else {
                            // pulldown-cmark does not always emit emphasis events for
                            // intraword forms like `IT*ALI*C`. Handle that common typing
                            // pattern as a fallback so WYSIWYG matches user expectation.
                            if let Some(inlines) = parse_intraword_italic_fallback(&s) {
                                inline_buf.extend(inlines);
                                continue;
                            }
                            Inline::Text(s)
                        };
                        inline_buf.push(inline);
                    }
                }

                Event::Code(text) => {
                    if in_paragraph {
                        inline_buf.push(Inline::Code(text.into_string()));
                    }
                }

                Event::SoftBreak => {
                    if in_paragraph {
                        inline_buf.push(Inline::SoftBreak);
                    }
                }
                Event::HardBreak => {
                    if in_paragraph {
                        inline_buf.push(Inline::HardBreak);
                    }
                }

                Event::Rule => nodes.push(RenderNode::ThematicBreak),

                _ => {}
            }
        }

        RenderTree(nodes)
    }
}

fn heading_level_to_u8(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn parse_intraword_italic_fallback(text: &str) -> Option<Vec<Inline>> {
    if !text.contains('*') {
        return None;
    }

    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<Inline> = Vec::new();
    let mut cursor = 0usize;

    while cursor < chars.len() {
        let open = (cursor..chars.len()).find(|&i| chars[i] == '*');
        let Some(open_idx) = open else {
            break;
        };

        if open_idx > cursor {
            out.push(Inline::Text(chars[cursor..open_idx].iter().collect()));
        }

        let close = (open_idx + 1..chars.len()).find(|&i| chars[i] == '*');
        let Some(close_idx) = close else {
            out.push(Inline::Text(chars[open_idx..].iter().collect()));
            cursor = chars.len();
            break;
        };

        if close_idx == open_idx + 1 {
            out.push(Inline::Text("**".to_string()));
            cursor = close_idx + 1;
            continue;
        }

        let italic_text: String = chars[open_idx + 1..close_idx].iter().collect();
        out.push(Inline::Italic(italic_text));
        cursor = close_idx + 1;
    }

    if cursor < chars.len() {
        out.push(Inline::Text(chars[cursor..].iter().collect()));
    }

    if out.is_empty() || (out.len() == 1 && matches!(out[0], Inline::Text(_))) {
        None
    } else {
        Some(out)
    }
}

// ──────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_heading() {
        let tree = Renderer::parse("# Hello World");
        assert!(matches!(
            &tree.0[0],
            RenderNode::Heading { level: 1, text } if text == "Hello World"
        ));
    }

    #[test]
    fn detects_mermaid_fence() {
        let md = "```mermaid\ngraph TD\n  A-->B\n```\n";
        let tree = Renderer::parse(md);
        assert!(matches!(&tree.0[0], RenderNode::MermaidBlock(_)));
    }

    #[test]
    fn non_mermaid_code_block() {
        let md = "```rust\nfn main() {}\n```\n";
        let tree = Renderer::parse(md);
        assert!(matches!(
            &tree.0[0],
            RenderNode::CodeBlock { lang: Some(l), .. } if l == "rust"
        ));
    }

    #[test]
    fn parses_paragraph_with_bold() {
        let tree = Renderer::parse("Hello **world**");
        if let RenderNode::Paragraph(inlines) = &tree.0[0] {
            assert!(inlines.iter().any(|i| matches!(i, Inline::Bold(t) if t == "world")));
        } else {
            panic!("expected paragraph");
        }
    }

    #[test]
    fn parses_paragraph_with_strikethrough() {
        let tree = Renderer::parse("Hello ~~world~~");
        if let RenderNode::Paragraph(inlines) = &tree.0[0] {
            assert!(
                inlines
                    .iter()
                    .any(|i| matches!(i, Inline::Strikethrough(t) if t == "world"))
            );
        } else {
            panic!("expected paragraph");
        }
    }

    #[test]
    fn parses_paragraph_with_underline_tag() {
        let tree = Renderer::parse("Hello <u>world</u>");
        if let RenderNode::Paragraph(inlines) = &tree.0[0] {
            assert!(
                inlines
                    .iter()
                    .any(|i| matches!(i, Inline::Underline(t) if t == "world"))
            );
        } else {
            panic!("expected paragraph");
        }
    }

    #[test]
    fn parses_intraword_italic_fallback() {
        let tree = Renderer::parse("IT*ALI*C ITALIC");
        if let RenderNode::Paragraph(inlines) = &tree.0[0] {
            assert!(
                inlines.iter().any(|i| matches!(i, Inline::Italic(t) if t == "ALI")),
                "expected intraword italic fallback, got: {:?}",
                inlines
            );
        } else {
            panic!("expected paragraph");
        }
    }

    #[test]
    fn parses_gfm_table() {
        let md = "| Name | Age |\n| --- | --- |\n| Alice | 30 |\n| Bob | 25 |\n";
        let tree = Renderer::parse(md);
        if let RenderNode::Table { headers, rows } = &tree.0[0] {
            // Two header cells
            assert_eq!(headers.len(), 2, "expected 2 header columns");
            let h0: String = headers[0].iter().filter_map(|i| if let Inline::Text(t) = i { Some(t.clone()) } else { None }).collect();
            assert_eq!(h0, "Name");
            // Two data rows
            assert_eq!(rows.len(), 2, "expected 2 data rows");
            let r0c1: String = rows[0][1].iter().filter_map(|i| if let Inline::Text(t) = i { Some(t.clone()) } else { None }).collect();
            assert_eq!(r0c1, "30");
        } else {
            panic!("expected RenderNode::Table, got: {:?}", tree.0);
        }
    }
}
