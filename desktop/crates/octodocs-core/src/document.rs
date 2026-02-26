use std::path::PathBuf;

/// Represents an open (or unsaved) Markdown document.
#[derive(Debug, Clone, Default)]
pub struct Document {
    /// Raw Markdown source text.
    pub content: String,
    /// Filesystem path, `None` for new unsaved files.
    pub path: Option<PathBuf>,
}

impl Document {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_content(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            path: None,
        }
    }

    /// Returns the file name (stem + extension) or "Untitled" if no path is set.
    pub fn title(&self) -> String {
        self.path
            .as_ref()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("Untitled")
            .to_string()
    }

    /// Returns `true` when the document has never been saved to disk.
    pub fn is_new(&self) -> bool {
        self.path.is_none()
    }

    /// Returns the word count of the current content.
    pub fn word_count(&self) -> usize {
        self.content
            .split_whitespace()
            .filter(|s| !s.is_empty())
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_untitled_when_no_path() {
        let doc = Document::new();
        assert_eq!(doc.title(), "Untitled");
    }

    #[test]
    fn title_from_path() {
        let doc = Document {
            content: String::new(),
            path: Some(PathBuf::from("/home/user/notes/readme.md")),
        };
        assert_eq!(doc.title(), "readme.md");
    }

    #[test]
    fn word_count_basic() {
        let doc = Document::with_content("hello world foo");
        assert_eq!(doc.word_count(), 3);
    }
}
