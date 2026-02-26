use crate::renderer::{DocumentBlock, Renderer};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SpanFormat {
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineSpan {
    pub text: String,
    pub format: SpanFormat,
}

impl InlineSpan {
    pub fn new(text: impl Into<String>, format: SpanFormat) -> Self {
        Self { text: text.into(), format }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InlineSpanKind {
    Styled(InlineSpan),
    Link { text: String, url: String },
}

impl InlineSpanKind {
    pub fn plain(text: impl Into<String>) -> Self {
        InlineSpanKind::Styled(InlineSpan::new(text, SpanFormat::default()))
    }

    pub fn text(&self) -> &str {
        match self {
            InlineSpanKind::Styled(s) => &s.text,
            InlineSpanKind::Link { text, .. } => text,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RichBlock {
    Paragraph { spans: Vec<InlineSpanKind> },
    Heading { level: u8, text: String },
    CodeFence { lang: Option<String>, lines: Vec<String> },
    MermaidBlock(String),
    List { ordered: bool, items: Vec<Vec<InlineSpanKind>> },
    ThematicBreak,
    BlockQuote(Vec<InlineSpanKind>),
}

impl RichBlock {
    pub fn to_markdown(&self) -> String {
        match self {
            RichBlock::Paragraph { spans } => {
                let mut s = String::new();
                for span in spans {
                    match span {
                        InlineSpanKind::Styled(is) => {
                            if is.format.code {
                                s.push('`');
                                s.push_str(&is.text);
                                s.push('`');
                            } else if is.format.bold && is.format.italic {
                                s.push_str("***");
                                s.push_str(&is.text);
                                s.push_str("***");
                            } else if is.format.bold {
                                s.push_str("**");
                                s.push_str(&is.text);
                                s.push_str("**");
                            } else if is.format.italic {
                                s.push('*');
                                s.push_str(&is.text);
                                s.push('*');
                            } else {
                                s.push_str(&is.text);
                            }
                        }
                        InlineSpanKind::Link { text, url } => {
                            s.push('[');
                            s.push_str(text);
                            s.push_str("](");
                            s.push_str(url);
                            s.push(')');
                        }
                    }
                }
                s.push('\n');
                s
            }
            RichBlock::Heading { level, text } => {
                format!("{} {}\n", "#".repeat(*level as usize), text)
            }
            RichBlock::CodeFence { lang, lines } => {
                let lang_str = lang.as_deref().unwrap_or("");
                let content = lines.join("\n");
                format!("```{}\n{}\n```\n", lang_str, content)
            }
            RichBlock::MermaidBlock(src) => {
                format!("```mermaid\n{}\n```\n", src)
            }
            RichBlock::List { ordered, items } => {
                let mut s = String::new();
                for (i, item) in items.iter().enumerate() {
                    let prefix = if *ordered {
                        format!("{}. ", i + 1)
                    } else {
                        "- ".to_string()
                    };
                    s.push_str(&prefix);
                    for span in item {
                        s.push_str(span.text());
                    }
                    s.push('\n');
                }
                s
            }
            RichBlock::ThematicBreak => "---\n".to_string(),
            RichBlock::BlockQuote(spans) => {
                let mut s = "> ".to_string();
                for span in spans {
                    s.push_str(span.text());
                }
                s.push('\n');
                s
            }
        }
    }

    pub fn from_document_block(block: &DocumentBlock) -> RichBlock {
        let blocks = Renderer::parse_rich_blocks(&block.source);
        blocks.into_iter().next().unwrap_or_else(|| RichBlock::Paragraph {
            spans: vec![InlineSpanKind::plain(block.source.trim_end())],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn span_format_default() {
        let f = SpanFormat::default();
        assert!(!f.bold && !f.italic && !f.code);
    }

    #[test]
    fn inline_span_kind_plain() {
        let s = InlineSpanKind::plain("hello");
        assert_eq!(s.text(), "hello");
        assert!(matches!(s, InlineSpanKind::Styled(ref is) if is.format == SpanFormat::default()));
    }

    #[test]
    fn rich_block_paragraph_to_markdown() {
        let block = RichBlock::Paragraph {
            spans: vec![
                InlineSpanKind::plain("Hello "),
                InlineSpanKind::Styled(InlineSpan::new(
                    "world",
                    SpanFormat { bold: true, ..Default::default() },
                )),
            ],
        };
        assert_eq!(block.to_markdown(), "Hello **world**\n");
    }

    #[test]
    fn rich_block_heading_to_markdown() {
        let block = RichBlock::Heading { level: 2, text: "Test".to_string() };
        assert_eq!(block.to_markdown(), "## Test\n");
    }

    #[test]
    fn rich_block_code_fence() {
        let block = RichBlock::CodeFence {
            lang: Some("rust".to_string()),
            lines: vec!["fn main() {}".to_string()],
        };
        assert!(block.to_markdown().contains("```rust"));
    }

    #[test]
    fn thematic_break() {
        assert_eq!(RichBlock::ThematicBreak.to_markdown(), "---\n");
    }
}
