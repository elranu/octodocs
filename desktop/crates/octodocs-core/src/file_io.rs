use std::fs;
use std::io;
use std::path::Path;

use crate::Document;

pub struct FileIo;

impl FileIo {
    /// Read a Markdown file from disk and return a `Document`.
    pub fn open(path: &Path) -> io::Result<Document> {
        let content = fs::read_to_string(path)?;
        Ok(Document {
            content,
            path: Some(path.to_path_buf()),
        })
    }

    /// Write the document content to its current path.
    /// Returns an error if the document has no path set.
    pub fn save(doc: &Document) -> io::Result<()> {
        let path = doc.path.as_ref().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "Document has no path — use save_as")
        })?;
        fs::write(path, &doc.content)
    }

    /// Write the document content to a new path.
    pub fn save_as(doc: &Document, path: &Path) -> io::Result<()> {
        fs::write(path, &doc.content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    // NOTE: tempfile is only needed for tests — add it as a dev-dep if you want to run these.
    // For now this is left as a doc-level example.
    #[test]
    fn round_trip() {
        let content = "# Hello\n\nWorld\n";
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(content.as_bytes()).unwrap();

        let doc = FileIo::open(file.path()).unwrap();
        assert_eq!(doc.content, content);

        let mut doc2 = doc.clone();
        doc2.content = "# Updated\n".to_string();
        FileIo::save(&doc2).unwrap();

        let doc3 = FileIo::open(file.path()).unwrap();
        assert_eq!(doc3.content, "# Updated\n");
    }
}
