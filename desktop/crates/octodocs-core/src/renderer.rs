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
    Image { alt: String, url: String },
    SoftBreak,
    HardBreak,
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

        let parser = Parser::new_ext(markdown, options);
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

                // ── Paragraphs ────────────────────────────────────
                Event::Start(Tag::Paragraph) => {
                    in_paragraph = true;
                    inline_buf.clear();
                }
                Event::End(TagEnd::Paragraph) => {
                    in_paragraph = false;
                    if !inline_buf.is_empty() {
                        nodes.push(RenderNode::Paragraph(inline_buf.clone()));
                        inline_buf.clear();
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

                Event::Html(html) => {
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
                    if in_code_block {
                        code_buf.push_str(&s);
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
}
