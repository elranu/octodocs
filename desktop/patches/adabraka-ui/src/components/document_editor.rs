//! Word-style continuous document editor.
//!
//! One `Entity<DocumentEditorState>` owns the full document as `Vec<DocParagraph>`.
//! A single cursor and optional selection span across the whole document.
//! `DocumentEditorElement` is a custom GPUI element that shapes and paints all
//! visual lines, registers the input handler, and handles mouse clicks.

use crate::theme::use_theme;
use gpui::*;
use octodocs_core::{
    doc_paragraphs_to_markdown, DocCursor, DocParagraph, DocSelection, InlineFormat, InlineSpan,
    ParagraphKind,
};
use std::ops::Range;
use std::path::PathBuf;
use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, RwLock};

// ─────────────────────────────────────────────────────────────────────────────
// Actions
// ─────────────────────────────────────────────────────────────────────────────

actions!(
    document_editor,
    [
        MoveUp,
        MoveDown,
        MoveLeft,
        MoveRight,
        MoveToLineStart,
        MoveToLineEnd,
        MoveToDocStart,
        MoveToDocEnd,
        SelectLeft,
        SelectRight,
        SelectUp,
        SelectDown,
        SelectToLineStart,
        SelectToLineEnd,
        SelectAll,
        Backspace,
        Delete,
        Enter,
        Tab,
        Copy,
        Cut,
        Paste,
    ]
);

/// Register key bindings. Call once from `adabraka_ui::init`.
pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("up", MoveUp, Some("DocumentEditor")),
        KeyBinding::new("down", MoveDown, Some("DocumentEditor")),
        KeyBinding::new("left", MoveLeft, Some("DocumentEditor")),
        KeyBinding::new("right", MoveRight, Some("DocumentEditor")),
        KeyBinding::new("home", MoveToLineStart, Some("DocumentEditor")),
        KeyBinding::new("end", MoveToLineEnd, Some("DocumentEditor")),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-up", MoveToDocStart, Some("DocumentEditor")),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-home", MoveToDocStart, Some("DocumentEditor")),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-down", MoveToDocEnd, Some("DocumentEditor")),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-end", MoveToDocEnd, Some("DocumentEditor")),
        KeyBinding::new("shift-left", SelectLeft, Some("DocumentEditor")),
        KeyBinding::new("shift-right", SelectRight, Some("DocumentEditor")),
        KeyBinding::new("shift-up", SelectUp, Some("DocumentEditor")),
        KeyBinding::new("shift-down", SelectDown, Some("DocumentEditor")),
        KeyBinding::new("shift-home", SelectToLineStart, Some("DocumentEditor")),
        KeyBinding::new("shift-end", SelectToLineEnd, Some("DocumentEditor")),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-a", SelectAll, Some("DocumentEditor")),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-a", SelectAll, Some("DocumentEditor")),
        KeyBinding::new("backspace", Backspace, Some("DocumentEditor")),
        KeyBinding::new("delete", Delete, Some("DocumentEditor")),
        KeyBinding::new("enter", Enter, Some("DocumentEditor")),
        KeyBinding::new("tab", Tab, Some("DocumentEditor")),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-c", Copy, Some("DocumentEditor")),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-c", Copy, Some("DocumentEditor")),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-x", Cut, Some("DocumentEditor")),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-x", Cut, Some("DocumentEditor")),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-v", Paste, Some("DocumentEditor")),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-v", Paste, Some("DocumentEditor")),
    ]);
}

// ─────────────────────────────────────────────────────────────────────────────
// Visual line cache  (rebuilt each paint frame, stored in state)
// ─────────────────────────────────────────────────────────────────────────────

/// Layout info for one visual line (one `\n`-delimited subline of a paragraph).
#[derive(Debug, Clone)]
pub struct VisualLine {
    pub para_idx: usize,
    /// Character offset (within the paragraph's plain-text) of the first char.
    pub char_start: usize,
    /// Character offset (within the paragraph's plain-text) of the first char
    /// on the *next* visual line (i.e., char_start of the next line, or
    /// `para.char_count()` for the last line).  The `\n` separator itself is
    /// not counted.
    pub char_end: usize,
    /// y-coordinate of the top edge, in document (scroll-content) coordinates.
    pub top_px: f32,
    pub height_px: f32,
    pub font_size_px: f32,
    pub is_mermaid: bool,
    /// x-positions (relative to the left margin, in logical pixels) of each
    /// character's left edge.  Length = char_count + 1; the last entry is the
    /// right edge of the last character (= where a cursor after EOF would sit).
    /// Empty for mermaid blocks.
    pub glyph_xs: Vec<Pixels>,
}

// ─────────────────────────────────────────────────────────────────────────────
// DocumentEditorState
// ─────────────────────────────────────────────────────────────────────────────

pub struct DocumentEditorState {
    pub focus_handle: FocusHandle,
    pub paragraphs: Vec<DocParagraph>,
    pub cursor: DocCursor,
    pub selection: Option<DocSelection>,
    /// UTF-8 byte range (relative to the current paragraph) for IME composition.
    pub marked_range: Option<Range<usize>>,
    /// Layout cache rebuilt every paint pass.
    pub layout_cache: Vec<VisualLine>,
    /// Last known element bounds (used for `bounds_for_range` in IME).
    pub last_bounds: Option<Bounds<Pixels>>,
    /// Anchor set on mouse-down; drives click-and-drag text selection.
    pub drag_anchor: Option<DocCursor>,
    /// True once the mouse has moved with the left button held (distinguishes
    /// a drag from a simple click so we know whether to clear the selection).
    pub drag_occurred: bool,
    /// Active table cell: (row, col) where row=0 is the header row.
    /// Set when the user clicks on a table cell or navigates with Tab.
    pub table_cursor: Option<(usize, usize)>,
    /// Directory of the currently open document, used to resolve and store images.
    pub document_dir: Option<PathBuf>,
    /// Tracks an in-progress image-block resize drag: (para_idx, original_height, drag_start_y).
    pub image_resize_drag: Option<(usize, f32, f32)>,
    /// When Some, a full-size zoom overlay is shown for this image path.
    pub image_zoom: Option<PathBuf>,
    /// Index of the image paragraph the mouse is currently hovering over (for magnifier badge).
    pub hovered_image_para: Option<usize>,
    /// Index of the mermaid paragraph the mouse is currently hovering over (for magnifier badge).
    pub hovered_mermaid_para: Option<usize>,
    /// Per-paragraph total visual pixel height (text only, excl. PARA_GAP).
    /// Populated each paint pass; used by request_layout/prepaint to size the
    /// scroll area correctly after word-wrap is computed.
    pub para_visual_heights: Vec<f32>,
    /// Whether the mouse is hovering a link span (drives pointer cursor).
    pub hovered_link: bool,
    /// Set to Some(path) by a link click on a local .md file; the host app should
    /// consume this in its doc_editor observer and navigate to that document.
    pub navigate_request: Option<String>,
    /// Shared image decode cache: abs path → decoded BGRA RenderImage.
    /// Populated asynchronously by background tasks; paint reads without blocking.
    pub image_cache: Arc<RwLock<HashMap<String, Arc<RenderImage>>>>,
    /// In-flight decode keys so we don't spawn duplicate tasks.
    pub pending_images: HashSet<String>,
}

impl DocumentEditorState {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            paragraphs: vec![DocParagraph::empty()],
            cursor: DocCursor::zero(),
            selection: None,
            marked_range: None,
            layout_cache: Vec::new(),
            last_bounds: None,
            drag_anchor: None,
            drag_occurred: false,
            table_cursor: None,
            document_dir: None,
            image_resize_drag: None,
            image_zoom: None,
            hovered_image_para: None,
            hovered_mermaid_para: None,
            para_visual_heights: Vec::new(),
            hovered_link: false,
            navigate_request: None,
            image_cache: Arc::new(RwLock::new(HashMap::new())),
            pending_images: HashSet::new(),
        }
    }

    fn reset_transient_state(&mut self) {
        self.cursor = DocCursor::zero();
        self.selection = None;
        self.marked_range = None;
        self.layout_cache.clear();
        self.drag_anchor = None;
        self.image_resize_drag = None;
        self.image_zoom = None;
        self.hovered_image_para = None;
        self.hovered_mermaid_para = None;
        self.hovered_link = false;
        self.navigate_request = None;
        self.para_visual_heights.clear();
        self.drag_occurred = false;
        self.table_cursor = None;
        self.image_cache = Arc::new(RwLock::new(HashMap::new()));
        self.pending_images.clear();
        IMAGE_CACHE.with(|cache| cache.borrow_mut().clear());
        MERMAID_CACHE.with(|cache| cache.borrow_mut().clear());
        MERMAID_DIMS_CACHE.with(|cache| cache.borrow_mut().clear());
    }

    pub fn clear(&mut self) {
        self.paragraphs = vec![DocParagraph::empty()];
        self.reset_transient_state();
    }

    /// Replace the document content. Cursor resets to the beginning.
    pub fn load_document(&mut self, paragraphs: Vec<DocParagraph>, cx: &mut Context<Self>) {
        self.paragraphs = if paragraphs.is_empty() {
            vec![DocParagraph::empty()]
        } else {
            paragraphs
        };
        self.reset_transient_state();
        // Spawn background decode for every image paragraph so paint never blocks.
        let doc_dir = self.document_dir.clone();
        for para in &self.paragraphs {
            if let ParagraphKind::Image { path, .. } = &para.kind {
                let abs_path = doc_dir
                    .as_ref()
                    .map(|d| d.join(path.as_str()))
                    .unwrap_or_else(|| PathBuf::from(path.as_str()));
                let cache_key = abs_path.to_string_lossy().to_string();
                if !self.pending_images.insert(cache_key.clone()) {
                    continue; // already queued
                }
                let cache_ref = self.image_cache.clone();
                cx.spawn(async move |this, cx| {
                    let ri = cx
                        .background_executor()
                        .spawn(async move {
                            std::fs::read(&abs_path).ok()
                                .and_then(|bytes| image::load_from_memory(&bytes).ok())
                                .map(|dyn_img| {
                                    let mut rgba = dyn_img.into_rgba8();
                                    for pixel in rgba.chunks_exact_mut(4) { pixel.swap(0, 2); }
                                    Arc::new(RenderImage::new([image::Frame::new(rgba)]))
                                })
                        })
                        .await;
                    let _ = this.update(cx, |s, cx| {
                        s.pending_images.remove(&cache_key);
                        if let Some(image) = ri {
                            if let Ok(mut c) = cache_ref.write() {
                                c.insert(cache_key, image);
                            }
                        }
                        cx.notify();
                    });
                }).detach();
            }
        }
        cx.notify();
    }

    /// Serialise the rich model back to Markdown.
    pub fn to_markdown(&self) -> String {
        doc_paragraphs_to_markdown(&self.paragraphs)
    }

    // ── Helpers ──────────────────────────────────────────────────────────────

    fn num_paras(&self) -> usize {
        self.paragraphs.len()
    }

    fn para_char_count(&self, para_idx: usize) -> usize {
        self.paragraphs[para_idx].char_count()
    }

    /// Find which visual line contains the given cursor position.
    fn visual_line_for_cursor(&self, c: DocCursor) -> Option<&VisualLine> {
        self.layout_cache.iter().find(|vl| {
            vl.para_idx == c.para_idx
                && c.char_offset >= vl.char_start
                && c.char_offset <= vl.char_end
        })
    }

    // ── Span / text mutation helpers ─────────────────────────────────────────

    /// Merge adjacent spans that share the same format and drop empty spans.
    fn merge_adjacent_spans(spans: &mut Vec<InlineSpan>) {
        let mut merged: Vec<InlineSpan> = Vec::with_capacity(spans.len());
        for span in spans.drain(..) {
            if span.text.is_empty() {
                continue;
            }
            if let Some(last) = merged.last_mut() {
                if last.format == span.format && last.link_url == span.link_url {
                    last.text.push_str(&span.text);
                    continue;
                }
            }
            merged.push(span);
        }

        if merged.is_empty() {
            merged.push(InlineSpan {
                text: String::new(),
                format: InlineFormat::Plain,
                link_url: None,
            });
        }
        *spans = merged;
    }

    fn split_text_at_char(text: &str, at: usize) -> (String, String) {
        let mut before = String::new();
        let mut after = String::new();
        for (i, ch) in text.chars().enumerate() {
            if i < at {
                before.push(ch);
            } else {
                after.push(ch);
            }
        }
        (before, after)
    }

    fn split_spans_at_char(spans: &[InlineSpan], at: usize) -> (Vec<InlineSpan>, Vec<InlineSpan>) {
        let mut left: Vec<InlineSpan> = Vec::new();
        let mut right: Vec<InlineSpan> = Vec::new();

        let mut char_pos = 0usize;
        for span in spans {
            let span_len = span.text.chars().count();
            let span_start = char_pos;
            let span_end = span_start + span_len;

            if at <= span_start {
                right.push(InlineSpan {
                    text: span.text.clone(),
                    format: span.format,
                    link_url: span.link_url.clone(),
                });
            } else if at >= span_end {
                left.push(InlineSpan {
                    text: span.text.clone(),
                    format: span.format,
                    link_url: span.link_url.clone(),
                });
            } else {
                let split_at = at - span_start;
                let (before, after) = Self::split_text_at_char(&span.text, split_at);
                if !before.is_empty() {
                    left.push(InlineSpan {
                        text: before,
                        format: span.format,
                        link_url: span.link_url.clone(),
                    });
                }
                if !after.is_empty() {
                    right.push(InlineSpan {
                        text: after,
                        format: span.format,
                        link_url: span.link_url.clone(),
                    });
                }
            }
            char_pos = span_end;
        }

        Self::merge_adjacent_spans(&mut left);
        Self::merge_adjacent_spans(&mut right);
        (left, right)
    }

    /// Higher-level helper: insert `text` at `DocCursor`, return new cursor.
    fn do_insert(&mut self, at: DocCursor, text: &str) -> DocCursor {
        if text.is_empty() {
            return at;
        }

        let para = &mut self.paragraphs[at.para_idx];
        let (mut left, mut right) = Self::split_spans_at_char(&para.spans, at.char_offset);
        let insertion_format = left
            .last()
            .map(|s| s.format)
            .or_else(|| right.first().map(|s| s.format))
            .unwrap_or(InlineFormat::Plain);
        // Do not propagate Link format when typing — new characters should be Plain.
        let insertion_format = if insertion_format == InlineFormat::Link { InlineFormat::Plain } else { insertion_format };

        left.push(InlineSpan {
            text: text.to_string(),
            format: insertion_format,
            link_url: None,
        });
        left.append(&mut right);
        Self::merge_adjacent_spans(&mut left);
        para.spans = left;

        let new_char_offset = at.char_offset + text.chars().count();
        DocCursor { para_idx: at.para_idx, char_offset: new_char_offset }
    }

    /// Remove `count` chars starting at `at`, return new cursor.
    fn do_delete_chars(&mut self, at: DocCursor, count: usize) -> DocCursor {
        if count == 0 {
            return at;
        }

        let para = &mut self.paragraphs[at.para_idx];
        let (mut left, tail) = Self::split_spans_at_char(&para.spans, at.char_offset);
        let (_, mut right) = Self::split_spans_at_char(&tail, count);
        left.append(&mut right);
        Self::merge_adjacent_spans(&mut left);
        para.spans = left;
        at
    }

    /// Get selected text as a String (or empty if no selection).
    pub fn selected_text(&self) -> String {
        if let Some((tr, tc)) = self.table_cursor {
            let para_idx = self.cursor.para_idx;
            if matches!(&self.paragraphs[para_idx].kind, ParagraphKind::Table { .. }) {
                let Some(sel) = self.selection else { return String::new(); };
                let (start, end) = sel.ordered();
                if start.para_idx == para_idx && end.para_idx == para_idx {
                    let cell = self.table_cell_text(para_idx, tr, tc);
                    let s = start.char_offset.min(cell.chars().count());
                    let e = end.char_offset.min(cell.chars().count());
                    return cell.chars().skip(s).take(e.saturating_sub(s)).collect();
                }
                return String::new();
            }
        }

        let Some(sel) = self.selection else { return String::new(); };
        let (start, end) = sel.ordered();
        if start.para_idx == end.para_idx {
            let flat = self.paragraphs[start.para_idx].plain_text();
            flat.chars().skip(start.char_offset).take(end.char_offset - start.char_offset).collect()
        } else {
            let mut s = String::new();
            // First paragraph (partial)
            let first_flat = self.paragraphs[start.para_idx].plain_text();
            s.extend(first_flat.chars().skip(start.char_offset));
            s.push('\n');
            // Middle paragraphs (full)
            for pi in start.para_idx + 1..end.para_idx {
                s.push_str(&self.paragraphs[pi].plain_text());
                s.push('\n');
            }
            // Last paragraph (partial)
            let last_flat = self.paragraphs[end.para_idx].plain_text();
            s.extend(last_flat.chars().take(end.char_offset));
            s
        }
    }

    /// Insert (or replace selection with) a hyperlink span.
    pub fn insert_link(&mut self, text: String, url: String, cx: &mut Context<Self>) {
        // Table cells are stored as plain strings (not inline spans), so insert
        // markdown link syntax directly into the active cell.
        if let Some((tr, tc)) = self.table_cursor {
            let para_idx = self.cursor.para_idx;
            if let ParagraphKind::Table { headers, rows, source } = &mut self.paragraphs[para_idx].kind {
                let insert_text = if text.is_empty() { url.clone() } else { text };
                let link_md = format!("[{insert_text}]({url})");
                let cell: &mut String = if tr == 0 {
                    &mut headers[tc]
                } else {
                    &mut rows[tr - 1][tc]
                };
                let (start_off, end_off) = if let Some(sel) = self.selection {
                    let (start, end) = sel.ordered();
                    if start.para_idx == para_idx && end.para_idx == para_idx {
                        (start.char_offset, end.char_offset)
                    } else {
                        (self.cursor.char_offset, self.cursor.char_offset)
                    }
                } else {
                    (self.cursor.char_offset, self.cursor.char_offset)
                };
                let char_count = cell.chars().count();
                let safe_start = start_off.min(char_count);
                let safe_end = end_off.min(char_count);
                if safe_start < safe_end {
                    let byte_s = cell.char_indices().nth(safe_start).map(|(i, _)| i).unwrap_or(cell.len());
                    let byte_e = cell.char_indices().nth(safe_end).map(|(i, _)| i).unwrap_or(cell.len());
                    cell.drain(byte_s..byte_e);
                }
                let insert_off = safe_start.min(cell.chars().count());
                let byte_off = cell
                    .char_indices()
                    .nth(insert_off)
                    .map(|(i, _)| i)
                    .unwrap_or(cell.len());
                cell.insert_str(byte_off, &link_md);
                *source = gfm_rebuild_from_strings(headers, rows);
                self.cursor.char_offset = insert_off + link_md.chars().count();
                self.selection = None;
                cx.notify();
                return;
            }
        }

        // Delete any active selection first (including multi-paragraph) so the
        // link reliably replaces whatever is selected.
        if self.selection.is_some() {
            self.cursor = self.delete_selection(cx);
        }
        let at = self.cursor;
        let para = &mut self.paragraphs[at.para_idx];
        let (mut left, mut right) = Self::split_spans_at_char(&para.spans, at.char_offset);
        left.push(InlineSpan { text: text.clone(), format: InlineFormat::Link, link_url: Some(url) });
        left.append(&mut right);
        Self::merge_adjacent_spans(&mut left);
        para.spans = left;
        self.cursor = DocCursor {
            para_idx: at.para_idx,
            char_offset: at.char_offset + text.chars().count(),
        };
        cx.notify();
    }

    /// Return the link URL for the span that contains the character at `char_offset - 1`
    /// (i.e., the character the user clicked on or is positioned just after).
    pub fn link_url_at_offset(&self, para_idx: usize, char_offset: usize) -> Option<String> {
        let para = self.paragraphs.get(para_idx)?;
        // char_offset is a gap position; the clicked char is at offset-1 (or 0 at start).
        let check = if char_offset > 0 { char_offset - 1 } else { 0 };
        let mut pos = 0usize;
        for span in &para.spans {
            let len = span.text.chars().count();
            if check >= pos && check < pos + len.max(1) {
                return if span.format == InlineFormat::Link { span.link_url.clone() } else { None };
            }
            pos += len;
        }
        None
    }

    /// If the word immediately before the cursor looks like a URL
    /// (starts with `http://` or `https://`), convert it in-place to a Link span.
    /// Called automatically by `insert_text` when the user types a space or Enter.
    fn try_autolink(&mut self) {
        let para_idx = self.cursor.para_idx;
        if !matches!(
            self.paragraphs[para_idx].kind,
            ParagraphKind::Paragraph | ParagraphKind::Heading(_)
        ) {
            return;
        }
        let flat = self.paragraphs[para_idx].plain_text();
        let chars_before: Vec<char> = flat.chars().take(self.cursor.char_offset).collect();
        if chars_before.is_empty() {
            return;
        }
        // Position after the last whitespace character = start of the last word.
        let word_start = chars_before
            .iter()
            .rposition(|&c| c == ' ' || c == '\n' || c == '\t')
            .map(|i| i + 1)
            .unwrap_or(0);
        if word_start >= chars_before.len() {
            return;
        }
        let word: String = chars_before[word_start..].iter().collect();
        if !word.starts_with("http://") && !word.starts_with("https://") {
            return;
        }
        // Skip if the span at word_start is already a Link.
        {
            let mut pos = 0usize;
            for span in &self.paragraphs[para_idx].spans {
                let len = span.text.chars().count();
                if word_start < pos + len {
                    if span.format == InlineFormat::Link {
                        return;
                    }
                    break;
                }
                pos += len;
            }
        }
        // Replace [word_start, cursor.char_offset) with a Link span.
        let url = word.clone();
        let para = &mut self.paragraphs[para_idx];
        let (left_all, right) = Self::split_spans_at_char(&para.spans, self.cursor.char_offset);
        let (before_word, _) = Self::split_spans_at_char(&left_all, word_start);
        let mut new_spans = before_word;
        new_spans.push(InlineSpan { text: url.clone(), format: InlineFormat::Link, link_url: Some(url) });
        new_spans.extend(right);
        Self::merge_adjacent_spans(&mut new_spans);
        para.spans = new_spans;
        // cursor.char_offset stays the same — total char count did not change.
    }

    /// Delete the current selection, return new cursor position (at selection start).
    fn delete_selection(&mut self, cx: &mut Context<Self>) -> DocCursor {
        let Some(sel) = self.selection.take() else { return self.cursor; };
        let (start, end) = sel.ordered();

        if let Some((tr, tc)) = self.table_cursor {
            let para_idx = self.cursor.para_idx;
            if start.para_idx == para_idx
                && end.para_idx == para_idx
                && matches!(&self.paragraphs[para_idx].kind, ParagraphKind::Table { .. })
            {
                if let ParagraphKind::Table { headers, rows, source } = &mut self.paragraphs[para_idx].kind {
                    let cell: &mut String = if tr == 0 { &mut headers[tc] } else { &mut rows[tr - 1][tc] };
                    let char_count = cell.chars().count();
                    let safe_start = start.char_offset.min(char_count);
                    let safe_end = end.char_offset.min(char_count);
                    if safe_start < safe_end {
                        let byte_s = cell.char_indices().nth(safe_start).map(|(i, _)| i).unwrap_or(cell.len());
                        let byte_e = cell.char_indices().nth(safe_end).map(|(i, _)| i).unwrap_or(cell.len());
                        cell.drain(byte_s..byte_e);
                        *source = gfm_rebuild_from_strings(headers, rows);
                    }
                    cx.notify();
                    return DocCursor { para_idx, char_offset: safe_start };
                }
            }
        }

        if start.para_idx == end.para_idx {
            let count = end.char_offset - start.char_offset;
            return self.do_delete_chars(start, count);
        }

        // Multi-paragraph selection: collapse by joining edges
        // Truncate the start paragraph at the selection start
        {
            let (mut start_left, _) =
                Self::split_spans_at_char(&self.paragraphs[start.para_idx].spans, start.char_offset);
            let (_, mut end_right) =
                Self::split_spans_at_char(&self.paragraphs[end.para_idx].spans, end.char_offset);
            start_left.append(&mut end_right);
            Self::merge_adjacent_spans(&mut start_left);
            self.paragraphs[start.para_idx].spans = start_left;
        }
        // Remove the in-between and end paragraphs
        self.paragraphs.drain(start.para_idx + 1..=end.para_idx);
        cx.notify();
        start
    }

    // ── Cursor movement ───────────────────────────────────────────────────────

    pub fn move_left(&mut self, _: &MoveLeft, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(sel) = self.selection.take() {
            self.cursor = sel.ordered().0;
        } else if self.cursor.char_offset > 0 {
            self.cursor.char_offset -= 1;
        } else if self.cursor.para_idx > 0 {
            self.cursor.para_idx -= 1;
            self.cursor.char_offset = self.para_char_count(self.cursor.para_idx);
        }
        cx.notify();
    }

    pub fn move_right(&mut self, _: &MoveRight, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(sel) = self.selection.take() {
            self.cursor = sel.ordered().1;
        } else {
            let len = self.para_char_count(self.cursor.para_idx);
            if self.cursor.char_offset < len {
                self.cursor.char_offset += 1;
            } else if self.cursor.para_idx + 1 < self.num_paras() {
                self.cursor.para_idx += 1;
                self.cursor.char_offset = 0;
            }
        }
        cx.notify();
    }

    pub fn move_up(&mut self, _: &MoveUp, _: &mut Window, cx: &mut Context<Self>) {
        self.selection = None;
        if let Some(cur_vl) = self.visual_line_for_cursor(self.cursor) {
            let idx_in_cache = self.layout_cache.iter().position(|vl| {
                vl.para_idx == cur_vl.para_idx && vl.char_start == cur_vl.char_start
            });
            if let Some(idx) = idx_in_cache {
                if idx > 0 {
                    let prev = &self.layout_cache[idx - 1];
                    let col = self.cursor.char_offset.saturating_sub(cur_vl.char_start);
                    let new_offset = (prev.char_start + col).min(prev.char_end);
                    self.cursor = DocCursor { para_idx: prev.para_idx, char_offset: new_offset };
                }
            }
        } else if self.cursor.para_idx > 0 {
            self.cursor.para_idx -= 1;
            self.cursor.char_offset = self.para_char_count(self.cursor.para_idx);
        }
        cx.notify();
    }

    pub fn move_down(&mut self, _: &MoveDown, _: &mut Window, cx: &mut Context<Self>) {
        self.selection = None;
        if let Some(cur_vl) = self.visual_line_for_cursor(self.cursor) {
            let idx_in_cache = self.layout_cache.iter().position(|vl| {
                vl.para_idx == cur_vl.para_idx && vl.char_start == cur_vl.char_start
            });
            if let Some(idx) = idx_in_cache {
                if idx + 1 < self.layout_cache.len() {
                    let next = &self.layout_cache[idx + 1];
                    let col = self.cursor.char_offset.saturating_sub(cur_vl.char_start);
                    let new_offset = (next.char_start + col).min(next.char_end);
                    self.cursor = DocCursor { para_idx: next.para_idx, char_offset: new_offset };
                }
            }
        } else if self.cursor.para_idx + 1 < self.num_paras() {
            self.cursor.para_idx += 1;
            self.cursor.char_offset = 0;
        }
        cx.notify();
    }

    pub fn move_to_line_start(&mut self, _: &MoveToLineStart, _: &mut Window, cx: &mut Context<Self>) {
        self.selection = None;
        if let Some(vl) = self.visual_line_for_cursor(self.cursor) {
            self.cursor.char_offset = vl.char_start;
        } else {
            self.cursor.char_offset = 0;
        }
        cx.notify();
    }

    pub fn move_to_line_end(&mut self, _: &MoveToLineEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.selection = None;
        if let Some(vl) = self.visual_line_for_cursor(self.cursor).cloned() {
            self.cursor.char_offset = vl.char_end;
        } else {
            self.cursor.char_offset = self.para_char_count(self.cursor.para_idx);
        }
        cx.notify();
    }

    pub fn move_to_doc_start(&mut self, _: &MoveToDocStart, _: &mut Window, cx: &mut Context<Self>) {
        self.selection = None;
        self.cursor = DocCursor::zero();
        cx.notify();
    }

    pub fn move_to_doc_end(&mut self, _: &MoveToDocEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.selection = None;
        let last = self.num_paras() - 1;
        self.cursor = DocCursor { para_idx: last, char_offset: self.para_char_count(last) };
        cx.notify();
    }

    // ── Select variants ───────────────────────────────────────────────────────

    fn start_sel_if_needed(&mut self) {
        if self.selection.is_none() {
            self.selection = Some(DocSelection { anchor: self.cursor, focus: self.cursor });
        }
    }

    fn extend_sel(&mut self) {
        if let Some(ref mut sel) = self.selection {
            sel.focus = self.cursor;
        }
    }

    pub fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.start_sel_if_needed();
        if self.cursor.char_offset > 0 {
            self.cursor.char_offset -= 1;
        } else if self.cursor.para_idx > 0 {
            self.cursor.para_idx -= 1;
            self.cursor.char_offset = self.para_char_count(self.cursor.para_idx);
        }
        self.extend_sel();
        cx.notify();
    }

    pub fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.start_sel_if_needed();
        let len = self.para_char_count(self.cursor.para_idx);
        if self.cursor.char_offset < len {
            self.cursor.char_offset += 1;
        } else if self.cursor.para_idx + 1 < self.num_paras() {
            self.cursor.para_idx += 1;
            self.cursor.char_offset = 0;
        }
        self.extend_sel();
        cx.notify();
    }

    pub fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        self.start_sel_if_needed();
        if let Some(cur_vl) = self.visual_line_for_cursor(self.cursor) {
            let idx_in_cache = self.layout_cache.iter().position(|vl| {
                vl.para_idx == cur_vl.para_idx && vl.char_start == cur_vl.char_start
            });
            if let Some(idx) = idx_in_cache {
                if idx > 0 {
                    let prev = &self.layout_cache[idx - 1];
                    let col = self.cursor.char_offset.saturating_sub(cur_vl.char_start);
                    let new_offset = (prev.char_start + col).min(prev.char_end);
                    self.cursor = DocCursor { para_idx: prev.para_idx, char_offset: new_offset };
                }
            }
        }
        self.extend_sel();
        cx.notify();
    }

    pub fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        self.start_sel_if_needed();
        if let Some(cur_vl) = self.visual_line_for_cursor(self.cursor) {
            let idx_in_cache = self.layout_cache.iter().position(|vl| {
                vl.para_idx == cur_vl.para_idx && vl.char_start == cur_vl.char_start
            });
            if let Some(idx) = idx_in_cache {
                if idx + 1 < self.layout_cache.len() {
                    let next = &self.layout_cache[idx + 1];
                    let col = self.cursor.char_offset.saturating_sub(cur_vl.char_start);
                    let new_offset = (next.char_start + col).min(next.char_end);
                    self.cursor = DocCursor { para_idx: next.para_idx, char_offset: new_offset };
                }
            }
        }
        self.extend_sel();
        cx.notify();
    }

    pub fn select_to_line_start(&mut self, _: &SelectToLineStart, _: &mut Window, cx: &mut Context<Self>) {
        self.start_sel_if_needed();
        if let Some(vl) = self.visual_line_for_cursor(self.cursor) {
            self.cursor.char_offset = vl.char_start;
        } else {
            self.cursor.char_offset = 0;
        }
        self.extend_sel();
        cx.notify();
    }

    pub fn select_to_line_end(&mut self, _: &SelectToLineEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.start_sel_if_needed();
        if let Some(vl) = self.visual_line_for_cursor(self.cursor).cloned() {
            self.cursor.char_offset = vl.char_end;
        } else {
            self.cursor.char_offset = self.para_char_count(self.cursor.para_idx);
        }
        self.extend_sel();
        cx.notify();
    }

    pub fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        let last = self.num_paras() - 1;
        let end = DocCursor { para_idx: last, char_offset: self.para_char_count(last) };
        self.selection = Some(DocSelection { anchor: DocCursor::zero(), focus: end });
        self.cursor = end;
        cx.notify();
    }

    // ── Text mutation ─────────────────────────────────────────────────────────

    pub fn insert_text(&mut self, text: &str, cx: &mut Context<Self>) {
        // ── Table cell editing ─────────────────────────────────────────────────
        if let Some((tr, tc)) = self.table_cursor {
            let para_idx = self.cursor.para_idx;
            if matches!(&self.paragraphs[para_idx].kind, ParagraphKind::Table { .. }) {
                if text == "\n" {
                    // Enter: add a new row after the current row
                    let (_, row_count) = match &self.paragraphs[para_idx].kind {
                        ParagraphKind::Table { headers, rows, .. } => (headers.len(), rows.len()),
                        _ => unreachable!(),
                    };
                    self.table_add_row_at(para_idx, tr.min(row_count));
                    self.table_cursor = Some((tr + 1, 0));
                    self.cursor.char_offset = 0;
                } else {
                    let offset = self.cursor.char_offset;
                    if let ParagraphKind::Table { headers, rows, source } = &mut self.paragraphs[para_idx].kind {
                        let cell: &mut String = if tr == 0 { &mut headers[tc] } else { &mut rows[tr - 1][tc] };
                        let char_count = cell.chars().count();
                        let safe_off = offset.min(char_count);
                        let byte_off = cell.char_indices().nth(safe_off).map(|(i, _)| i).unwrap_or(cell.len());
                        cell.insert_str(byte_off, text);
                        *source = gfm_rebuild_from_strings(headers, rows);
                        self.cursor.char_offset = safe_off + text.chars().count();
                    }
                }
                cx.notify();
                return;
            }
        }
        // ────────────────────────────────────────────────────────────────────────
        let had_selection = self.selection.is_some();
        if let Some(_) = self.selection {
            let at = self.delete_selection(cx);
            self.cursor = at;
        }

        // URL auto-linking: when the user types a space or Enter without an active
        // selection, check whether the word just before the cursor is a bare URL
        // and convert it to a Link span automatically.
        if !had_selection && (text == " " || text == "\n") {
            self.try_autolink();
        }

        if text == "\n" {
            // Enter: split paragraph at cursor
            let cur_para_idx = self.cursor.para_idx;
            let split_at = self.cursor.char_offset;
            let (head, tail) =
                Self::split_spans_at_char(&self.paragraphs[cur_para_idx].spans, split_at);
            self.paragraphs[cur_para_idx].spans = head;
            let new_para = DocParagraph {
                kind: ParagraphKind::Paragraph,
                spans: tail,
            };
            self.paragraphs.insert(cur_para_idx + 1, new_para);
            self.cursor = DocCursor { para_idx: cur_para_idx + 1, char_offset: 0 };
        } else {
            self.cursor = self.do_insert(self.cursor, text);
        }
        cx.notify();
    }

    pub fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        // ── Table cell backspace ───────────────────────────────────────────────
        if let Some((tr, tc)) = self.table_cursor {
            let para_idx = self.cursor.para_idx;
            if matches!(&self.paragraphs[para_idx].kind, ParagraphKind::Table { .. }) {
                if self.cursor.char_offset > 0 {
                    let offset = self.cursor.char_offset;
                    if let ParagraphKind::Table { headers, rows, source } = &mut self.paragraphs[para_idx].kind {
                        let cell: &mut String = if tr == 0 { &mut headers[tc] } else { &mut rows[tr - 1][tc] };
                        let del_at = offset - 1;
                        let byte_range = {
                            let mut iter = cell.char_indices();
                            let start = iter.nth(del_at).map(|(i, _)| i).unwrap_or(cell.len());
                            let end = iter.next().map(|(i, _)| i).unwrap_or(cell.len());
                            start..end
                        };
                        cell.drain(byte_range);
                        *source = gfm_rebuild_from_strings(headers, rows);
                        self.cursor.char_offset = del_at;
                    }
                }
                cx.notify();
                return;
            }
        }
        // ────────────────────────────────────────────────────────────────────────
        if self.selection.is_some() {
            let at = self.delete_selection(cx);
            self.cursor = at;
            cx.notify();
            return;
        }

        if self.cursor.char_offset > 0 {
            let at = DocCursor { para_idx: self.cursor.para_idx, char_offset: self.cursor.char_offset - 1 };
            self.do_delete_chars(at, 1);
            self.cursor = at;
        } else if self.cursor.para_idx > 0 {
            // Merge with previous paragraph
            let prev_idx = self.cursor.para_idx - 1;
            let prev_len = self.para_char_count(prev_idx);
            let mut merged = self.paragraphs[prev_idx].spans.clone();
            merged.extend(self.paragraphs[self.cursor.para_idx].spans.clone());
            Self::merge_adjacent_spans(&mut merged);
            self.paragraphs[prev_idx].spans = merged;
            self.paragraphs.remove(self.cursor.para_idx);
            self.cursor = DocCursor { para_idx: prev_idx, char_offset: prev_len };
        }
        cx.notify();
    }

    pub fn delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        if self.selection.is_some() {
            let at = self.delete_selection(cx);
            self.cursor = at;
            cx.notify();
            return;
        }

        let len = self.para_char_count(self.cursor.para_idx);
        if self.cursor.char_offset < len {
            self.do_delete_chars(self.cursor, 1);
        } else if self.cursor.para_idx + 1 < self.num_paras() {
            // Merge with next paragraph
            let mut merged = self.paragraphs[self.cursor.para_idx].spans.clone();
            merged.extend(self.paragraphs[self.cursor.para_idx + 1].spans.clone());
            Self::merge_adjacent_spans(&mut merged);
            self.paragraphs[self.cursor.para_idx].spans = merged;
            self.paragraphs.remove(self.cursor.para_idx + 1);
        }
        cx.notify();
    }

    pub fn enter(&mut self, _: &Enter, _: &mut Window, cx: &mut Context<Self>) {
        self.insert_text("\n", cx);
    }

    pub fn tab(&mut self, _: &Tab, _: &mut Window, cx: &mut Context<Self>) {
        // ── Table: navigate between cells ─────────────────────────────────────
        if let Some((tr, tc)) = self.table_cursor {
            let para_idx = self.cursor.para_idx;
            let (col_count, row_count) = match &self.paragraphs[para_idx].kind {
                ParagraphKind::Table { headers, rows, .. } => (headers.len(), rows.len()),
                _ => { self.insert_text("    ", cx); return; }
            };
            let next_tc = tc + 1;
            if next_tc < col_count {
                // Move to next cell in the same row
                let cell_len = self.table_cell_text(para_idx, tr, next_tc).chars().count();
                self.table_cursor = Some((tr, next_tc));
                self.cursor.char_offset = cell_len;
            } else {
                let next_tr = tr + 1;
                if next_tr <= row_count {
                    // First cell of the next row
                    let cell_len = self.table_cell_text(para_idx, next_tr, 0).chars().count();
                    self.table_cursor = Some((next_tr, 0));
                    self.cursor.char_offset = cell_len;
                } else {
                    // Past the last row: add a new one
                    self.table_add_row_at(para_idx, row_count);
                    self.table_cursor = Some((row_count + 1, 0));
                    self.cursor.char_offset = 0;
                }
            }
            cx.notify();
            return;
        }
        // ────────────────────────────────────────────────────────────────────────
        self.insert_text("    ", cx);
    }

    pub fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        let text = self.selected_text();
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    pub fn cut(&mut self, _: &Cut, _: &mut Window, cx: &mut Context<Self>) {
        let text = self.selected_text();
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            let at = self.delete_selection(cx);
            self.cursor = at;
            cx.notify();
        }
    }

    pub fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(item) = cx.read_from_clipboard() {
            // Check for an image in the clipboard first.
            for entry in item.entries() {
                if let ClipboardEntry::Image(img) = entry {
                    let ext = match img.format {
                        ImageFormat::Jpeg => "jpg",
                        ImageFormat::Gif => "gif",
                        ImageFormat::Webp => "webp",
                        _ => "png",
                    };
                    let images_dir = self.images_dir();
                    if std::fs::create_dir_all(&images_dir).is_ok() {
                        let ts = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis();
                        let filename = format!("clipboard-{ts}.{ext}");
                        let dest = images_dir.join(&filename);
                        if std::fs::write(&dest, &img.bytes).is_ok() {
                            let rel = format!("images/{filename}");
                            self.insert_image_at_cursor(rel, "image".to_string(), cx);
                            return;
                        }
                    }
                    break;
                }
            }
            // Fall back to text paste.
            if let Some(text) = item.text() {
                if self.selection.is_some() {
                    let at = self.delete_selection(cx);
                    self.cursor = at;
                }
                // Insert line by line
                let lines: Vec<&str> = text.split('\n').collect();
                for (i, line) in lines.iter().enumerate() {
                    if i > 0 { self.insert_text("\n", cx); }
                    if !line.is_empty() { self.insert_text(line, cx); }
                }
            }
        }
    }

    // ── Formatting ────────────────────────────────────────────────────────────

    /// Toggle bold on the current selection.
    pub fn toggle_bold(&mut self, cx: &mut Context<Self>) {
        self.toggle_format(InlineFormat::Bold, cx);
    }

    pub fn toggle_italic(&mut self, cx: &mut Context<Self>) {
        self.toggle_format(InlineFormat::Italic, cx);
    }

    pub fn toggle_underline(&mut self, cx: &mut Context<Self>) {
        self.toggle_format(InlineFormat::Underline, cx);
    }

    pub fn toggle_strikethrough(&mut self, cx: &mut Context<Self>) {
        self.toggle_format(InlineFormat::Strikethrough, cx);
    }

    pub fn toggle_code(&mut self, cx: &mut Context<Self>) {
        self.toggle_format(InlineFormat::Code, cx);
    }

    /// Toggle the current paragraph between `TaskListItem { checked: false }` and `Paragraph`.
    pub fn toggle_task_list_item(&mut self, cx: &mut Context<Self>) {
        let para = &mut self.paragraphs[self.cursor.para_idx];
        para.kind = match &para.kind {
            ParagraphKind::TaskListItem { .. } => ParagraphKind::Paragraph,
            _ => ParagraphKind::TaskListItem { checked: false },
        };
        cx.notify();
    }

    /// Insert an image paragraph after the current cursor position.
    pub fn insert_image_at_cursor(&mut self, path: String, alt: String, cx: &mut Context<Self>) {
        let para = DocParagraph {
            kind: ParagraphKind::Image { path: path.clone(), alt, height: IMAGE_BLOCK_DEFAULT_HEIGHT },
            spans: vec![InlineSpan { text: String::new(), format: InlineFormat::Plain, link_url: None }],
        };
        let insert_at = self.cursor.para_idx + 1;
        self.paragraphs.insert(insert_at, para);
        self.cursor = DocCursor { para_idx: insert_at, char_offset: 0 };

        // Kick off background decode for the newly inserted image.
        let abs_path = self.document_dir
            .as_ref()
            .map(|d| d.join(&path))
            .unwrap_or_else(|| PathBuf::from(&path));
        let cache_key = abs_path.to_string_lossy().to_string();
        if self.pending_images.insert(cache_key.clone()) {
            let cache_ref = self.image_cache.clone();
            cx.spawn(async move |this, cx| {
                let ri = cx
                    .background_executor()
                    .spawn(async move {
                        std::fs::read(&abs_path).ok()
                            .and_then(|bytes| image::load_from_memory(&bytes).ok())
                            .map(|dyn_img| {
                                let mut rgba = dyn_img.into_rgba8();
                                for pixel in rgba.chunks_exact_mut(4) { pixel.swap(0, 2); }
                                Arc::new(RenderImage::new([image::Frame::new(rgba)]))
                            })
                    })
                    .await;
                let _ = this.update(cx, |s, cx| {
                    s.pending_images.remove(&cache_key);
                    if let Some(image) = ri {
                        if let Ok(mut c) = cache_ref.write() {
                            c.insert(cache_key, image);
                        }
                    }
                    cx.notify();
                });
            }).detach();
        }

        cx.notify();
    }

    /// Return the images directory for the current document, falling back to a temp dir.
    fn images_dir(&self) -> PathBuf {
        self.document_dir
            .as_ref()
            .map(|d| d.join("images"))
            .unwrap_or_else(|| std::env::temp_dir().join("octodocs").join("images"))
    }

    /// Flip the `checked` state of a `TaskListItem` paragraph (called when user
    /// clicks on the checkbox area in the WYSIWYG view).
    pub fn toggle_checked_at_para(&mut self, para_idx: usize, cx: &mut Context<Self>) {
        if let ParagraphKind::TaskListItem { checked } = &mut self.paragraphs[para_idx].kind {
            *checked = !*checked;
            cx.notify();
        }
    }

    fn toggle_format(&mut self, fmt: InlineFormat, cx: &mut Context<Self>) {
        let Some(sel) = self.selection else {
            // No text selected — do nothing.
            return;
        };

        let (start, end) = sel.ordered();
        if start.para_idx != end.para_idx {
            cx.notify();
            return; // Cross-paragraph formatting out of scope for MVP
        }

        let para = &mut self.paragraphs[start.para_idx];
        let flat = para.plain_text();

        // Determine whether to apply or clear formatting uniformly across the
        // selected range.
        let mut has_non_fmt = false;
        let mut probe_char_pos = 0usize;
        for span in &para.spans {
            let span_chars = span.text.chars().count();
            let span_start = probe_char_pos;
            let span_end = span_start + span_chars;
            probe_char_pos = span_end;

            let overlap_start = span_start.max(start.char_offset);
            let overlap_end = span_end.min(end.char_offset);
            if overlap_start < overlap_end && span.format != fmt {
                has_non_fmt = true;
                break;
            }
        }

        // Rebuild spans: split at start/end boundaries and apply one target
        // format across the full selected range.
        let mut new_spans: Vec<InlineSpan> = Vec::new();
        let mut char_pos = 0usize;

        for span in para.spans.clone() {
            let span_chars = span.text.chars().count();
            let span_start = char_pos;
            let span_end = span_start + span_chars;

            let sel_start = start.char_offset;
            let sel_end = end.char_offset;

            // Part before the selection
            if span_start < sel_start && span_end > 0 {
                let end_clip = sel_start.min(span_end);
                if end_clip > span_start {
                    let text: String = flat.chars().skip(span_start).take(end_clip - span_start).collect();
                    if !text.is_empty() {
                        new_spans.push(InlineSpan { text, format: span.format, link_url: span.link_url.clone() });
                    }
                }
            }

            // Part within the selection
            let overlap_start = span_start.max(sel_start);
            let overlap_end = span_end.min(sel_end);
            if overlap_start < overlap_end {
                let text: String = flat.chars().skip(overlap_start).take(overlap_end - overlap_start).collect();
                if !text.is_empty() {
                    let new_fmt = if has_non_fmt { fmt } else { InlineFormat::Plain };
                    new_spans.push(InlineSpan { text, format: new_fmt, link_url: None });
                }
            }

            // Part after the selection
            if span_end > sel_end && span_start < span_end {
                let start_clip = sel_end.max(span_start);
                if start_clip < span_end {
                    let text: String = flat.chars().skip(start_clip).take(span_end - start_clip).collect();
                    if !text.is_empty() {
                        new_spans.push(InlineSpan { text, format: span.format, link_url: span.link_url.clone() });
                    }
                }
            }

            char_pos = span_end;
        }

        para.spans = coalesce_spans(new_spans);
        self.cursor = end;
        self.selection = None;
        cx.notify();
    }

    /// Change the block type of the current paragraph.
    pub fn set_paragraph_kind(&mut self, kind: ParagraphKind, cx: &mut Context<Self>) {
        self.paragraphs[self.cursor.para_idx].kind = kind;
        cx.notify();
    }

    /// Insert a blank 3-column / 2-row GFM table after the current paragraph.
    pub fn insert_table(&mut self, cx: &mut Context<Self>) {
        let source = "| Column 1 | Column 2 | Column 3 |\n| --- | --- | --- |\n| Cell | Cell | Cell |\n| Cell | Cell | Cell |\n".to_string();
        let headers = vec!["Column 1".to_string(), "Column 2".to_string(), "Column 3".to_string()];
        let rows = vec![
            vec!["Cell".to_string(), "Cell".to_string(), "Cell".to_string()],
            vec!["Cell".to_string(), "Cell".to_string(), "Cell".to_string()],
        ];
        let para = DocParagraph {
            kind: ParagraphKind::Table { source, headers, rows },
            spans: vec![InlineSpan { text: String::new(), format: InlineFormat::Plain, link_url: None }],
        };
        let insert_at = self.cursor.para_idx + 1;
        self.paragraphs.insert(insert_at, para);
        self.cursor = DocCursor { para_idx: insert_at, char_offset: 0 };
        cx.notify();
    }

    // ── Table cell helpers ─────────────────────────────────────────────────────

    /// Return a copy of the text in table cell (row, col).
    /// row=0 is the header row; row≥1 is data row (row-1).
    fn table_cell_text(&self, para_idx: usize, row: usize, col: usize) -> String {
        match &self.paragraphs[para_idx].kind {
            ParagraphKind::Table { headers, rows, .. } => {
                if row == 0 {
                    headers.get(col).cloned().unwrap_or_default()
                } else {
                    rows.get(row - 1).and_then(|r| r.get(col)).cloned().unwrap_or_default()
                }
            }
            _ => String::new(),
        }
    }

    /// Insert an empty data row at `data_idx` (0-indexed within rows).
    fn table_add_row_at(&mut self, para_idx: usize, data_idx: usize) {
        if let ParagraphKind::Table { headers, rows, source } = &mut self.paragraphs[para_idx].kind {
            let col_count = headers.len();
            rows.insert(data_idx.min(rows.len()), vec![String::new(); col_count]);
            *source = gfm_rebuild_from_strings(headers, rows);
        }
    }

    /// Add a data row after the active row (or at the bottom if no cell is active).
    pub fn add_table_row(&mut self, cx: &mut Context<Self>) {
        let para_idx = self.cursor.para_idx;
        if !matches!(&self.paragraphs[para_idx].kind, ParagraphKind::Table { .. }) {
            return;
        }
        let row_count = match &self.paragraphs[para_idx].kind {
            ParagraphKind::Table { rows, .. } => rows.len(),
            _ => return,
        };
        let insert_at = match self.table_cursor {
            Some((tr, _)) => tr.min(row_count),
            None => row_count,
        };
        self.table_add_row_at(para_idx, insert_at);
        self.table_cursor = Some((insert_at + 1, 0));
        self.cursor.char_offset = 0;
        cx.notify();
    }

    /// Remove the currently active data row. No-op if the header is active.
    pub fn remove_table_row(&mut self, cx: &mut Context<Self>) {
        let para_idx = self.cursor.para_idx;
        if let Some((tr, _)) = self.table_cursor {
            if tr == 0 { return; }
            if let ParagraphKind::Table { rows, headers, source } = &mut self.paragraphs[para_idx].kind {
                let data_idx = tr - 1;
                if data_idx < rows.len() {
                    rows.remove(data_idx);
                    *source = gfm_rebuild_from_strings(headers, rows);
                    let new_tr = if rows.is_empty() { 0 } else { (data_idx + 1).min(rows.len()) };
                    self.table_cursor = Some((new_tr, 0));
                    self.cursor.char_offset = 0;
                }
            }
        }
        cx.notify();
    }

    /// Add a column to the right of the active column (or at the end).
    pub fn add_table_column(&mut self, cx: &mut Context<Self>) {
        let para_idx = self.cursor.para_idx;
        if let ParagraphKind::Table { headers, rows, source } = &mut self.paragraphs[para_idx].kind {
            let insert_at = match self.table_cursor {
                Some((_, tc)) => (tc + 1).min(headers.len()),
                None => headers.len(),
            };
            headers.insert(insert_at, format!("Column {}", headers.len() + 1));
            for row in rows.iter_mut() {
                row.insert(insert_at.min(row.len()), String::new());
            }
            *source = gfm_rebuild_from_strings(headers, rows);
        }
        cx.notify();
    }

    /// Map a click position to the (row, col) of the table cell under the cursor,
    /// or None if the click is not inside any table bounds.
    pub fn table_cell_from_point(&self, point: Point<Pixels>, bounds: Bounds<Pixels>) -> Option<(usize, usize)> {
        let left = bounds.left() + px(LEFT_MARGIN);
        let vl = self.layout_cache.iter().rev().find(|vl|
            bounds.top() + px(vl.top_px) <= point.y
        )?;
        let para_idx = vl.para_idx;
        if let ParagraphKind::Table { headers, rows, .. } = &self.paragraphs[para_idx].kind {
            let col_count = headers.len().max(1);
            let table_top = bounds.top() + px(vl.top_px);
            let table_bottom = table_top + px(vl.height_px);
            if point.y < table_top || point.y > table_bottom { return None; }
            let table_w = bounds.size.width - px(LEFT_MARGIN * 2.0);
            let col_w_f32: f32 = table_w / px(col_count as f32);
            let rel_x: Pixels = (point.x - left).max(px(0.0));
            let col = ((rel_x / px(col_w_f32)) as usize).min(col_count - 1);
            let rel_y: Pixels = point.y - table_top;
            let row = ((rel_y / px(TABLE_ROW_H)) as usize).min(rows.len());
            Some((row, col))
        } else {
            None
        }
    }

    fn table_cursor_from_point(&self, point: Point<Pixels>, bounds: Bounds<Pixels>) -> Option<DocCursor> {
        let (row, col) = self.table_cell_from_point(point, bounds)?;
        let left = bounds.left() + px(LEFT_MARGIN);
        let vl = self.layout_cache.iter().rev().find(|vl|
            bounds.top() + px(vl.top_px) <= point.y
        )?;
        let para_idx = vl.para_idx;
        let ParagraphKind::Table { headers, .. } = &self.paragraphs[para_idx].kind else {
            return None;
        };

        let col_count = headers.len().max(1);
        let table_w = bounds.size.width - px(LEFT_MARGIN * 2.0);
        let col_w_f32: f32 = table_w / px(col_count as f32);
        let cell_x = left + px(col_w_f32 * col as f32);
        let text_x_start = cell_x + px(TABLE_CELL_PAD_X);
        let text_x_end = cell_x + px(col_w_f32 - TABLE_CELL_PAD_X);
        let rel = ((point.x - text_x_start) / (text_x_end - text_x_start).max(px(1.0))).clamp(0.0, 1.0);

        let cell = self.table_cell_text(para_idx, row, col);
        let len = cell.chars().count();
        let char_offset = ((len as f32) * rel).round() as usize;
        Some(DocCursor { para_idx, char_offset: char_offset.min(len) })
    }

    // ── Mouse ─────────────────────────────────────────────────────────────────

    /// Map a screen-space click position to the nearest document cursor position.
    /// Uses the per-line `glyph_xs` cache populated each paint pass for accurate
    /// sub-character hit testing without needing live `ShapedLine` objects.
    pub fn cursor_from_point(&self, point: Point<Pixels>, bounds: Bounds<Pixels>) -> DocCursor {
        if let Some(c) = self.table_cursor_from_point(point, bounds) {
            return c;
        }

        let relative_x = (point.x - (bounds.left() + px(LEFT_MARGIN))).max(px(0.0));

        // Find the last visual line whose top edge is at or above the click y.
        let vl = self.layout_cache.iter().rev().find(|vl|
            bounds.top() + px(vl.top_px) <= point.y
        );

        if let Some(vl) = vl {
            if vl.is_mermaid {
                return DocCursor { para_idx: vl.para_idx, char_offset: 0 };
            }
            let char_col = find_closest_char_col(relative_x, &vl.glyph_xs);
            return DocCursor { para_idx: vl.para_idx, char_offset: vl.char_start + char_col };
        }

        // Click above all content — go to start of document.
        DocCursor::zero()
    }

    // ── IME helpers ───────────────────────────────────────────────────────────

    /// Flat document content for IME (within the current paragraph).
    fn current_para_content(&self) -> String {
        self.paragraphs[self.cursor.para_idx].plain_text()
    }

    fn char_offset_to_utf16_in_para(&self, char_off: usize) -> usize {
        self.char_offset_to_utf16_for_para(self.cursor.para_idx, char_off)
    }

    fn char_offset_to_utf16_for_para(&self, para_idx: usize, char_off: usize) -> usize {
        let text = self.paragraphs[para_idx].plain_text();
        let mut u16 = 0usize;
        let mut chars_seen = 0usize;
        for ch in text.chars() {
            if chars_seen >= char_off { break; }
            u16 += ch.len_utf16();
            chars_seen += 1;
        }
        u16
    }

    fn utf16_to_char_offset_in_para(&self, utf16_off: usize) -> usize {
        let text = self.current_para_content();
        let mut char_count = 0usize;
        let mut u16_count = 0usize;
        for ch in text.chars() {
            if u16_count >= utf16_off { break; }
            u16_count += ch.len_utf16();
            char_count += 1;
        }
        char_count
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Focusable + EntityInputHandler
// ─────────────────────────────────────────────────────────────────────────────

impl Focusable for DocumentEditorState {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EntityInputHandler for DocumentEditorState {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        // When inside a table cell, report the cell text instead of paragraph text.
        if let Some((tr, tc)) = self.table_cursor {
            let para_idx = self.cursor.para_idx;
            if matches!(&self.paragraphs[para_idx].kind, ParagraphKind::Table { .. }) {
                let cell_text = self.table_cell_text(para_idx, tr, tc);
                let start = utf16_to_char_offset_in_str(&cell_text, range_utf16.start);
                let end = utf16_to_char_offset_in_str(&cell_text, range_utf16.end);
                let ac_start = char_offset_to_utf16_in_str(&cell_text, start);
                let ac_end = char_offset_to_utf16_in_str(&cell_text, end);
                *actual_range = Some(ac_start..ac_end);
                let result: String = cell_text.chars().skip(start).take(end.saturating_sub(start)).collect();
                return Some(result);
            }
        }
        let start = self.utf16_to_char_offset_in_para(range_utf16.start);
        let end = self.utf16_to_char_offset_in_para(range_utf16.end);
        let actual_start = self.char_offset_to_utf16_in_para(start);
        let actual_end = self.char_offset_to_utf16_in_para(end);
        *actual_range = Some(actual_start..actual_end);

        let text = self.current_para_content();
        let result: String = text.chars().skip(start).take(end.saturating_sub(start)).collect();
        Some(result)
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        // When inside a table cell, report cursor position within cell text.
        if let Some((tr, tc)) = self.table_cursor {
            let para_idx = self.cursor.para_idx;
            if matches!(&self.paragraphs[para_idx].kind, ParagraphKind::Table { .. }) {
                let cell_text = self.table_cell_text(para_idx, tr, tc);
                let off = self.cursor.char_offset.min(cell_text.chars().count());
                let u16_off = char_offset_to_utf16_in_str(&cell_text, off);
                return Some(UTF16Selection { range: u16_off..u16_off, reversed: false });
            }
        }
        let cursor_u16 = self.char_offset_to_utf16_in_para(self.cursor.char_offset);
        if let Some(sel) = &self.selection {
            let (start, end) = sel.ordered();
            // Only report if selection is within the current paragraph
            if start.para_idx == self.cursor.para_idx && end.para_idx == self.cursor.para_idx {
                let s_u16 = self.char_offset_to_utf16_in_para(start.char_offset);
                let e_u16 = self.char_offset_to_utf16_in_para(end.char_offset);
                return Some(UTF16Selection {
                    range: s_u16..e_u16,
                    reversed: sel.anchor > sel.focus,
                });
            }
        }
        Some(UTF16Selection { range: cursor_u16..cursor_u16, reversed: false })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        let mr = self.marked_range.as_ref()?;
        let start = bytes_to_chars(&self.current_para_content(), mr.start);
        let end = bytes_to_chars(&self.current_para_content(), mr.end);
        let s_u16 = self.char_offset_to_utf16_in_para(start);
        let e_u16 = self.char_offset_to_utf16_in_para(end);
        Some(s_u16..e_u16)
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // ── Table cell: route input into the active cell ──────────────────────
        if let Some((tr, tc)) = self.table_cursor {
            let para_idx = self.cursor.para_idx;
            if matches!(&self.paragraphs[para_idx].kind, ParagraphKind::Table { .. }) {
                if !new_text.is_empty() {
                    let offset = self.cursor.char_offset;
                    if let ParagraphKind::Table { headers, rows, source } = &mut self.paragraphs[para_idx].kind {
                        let cell: &mut String = if tr == 0 { &mut headers[tc] } else { &mut rows[tr - 1][tc] };
                        let char_count = cell.chars().count();
                        let safe_off = offset.min(char_count);
                        // If range_utf16 specifies a delete range (e.g. IME replacement), apply it
                        let (del_start_char, del_end_char) = if let Some(ref r) = range_utf16 {
                            let s = utf16_to_char_offset_in_str(cell, r.start);
                            let e = utf16_to_char_offset_in_str(cell, r.end);
                            (s, e)
                        } else {
                            (safe_off, safe_off)
                        };
                        if del_start_char < del_end_char {
                            let byte_s = cell.char_indices().nth(del_start_char).map(|(i,_)| i).unwrap_or(cell.len());
                            let byte_e = cell.char_indices().nth(del_end_char).map(|(i,_)| i).unwrap_or(cell.len());
                            cell.drain(byte_s..byte_e);
                        }
                        let insert_char_off = del_start_char.min(cell.chars().count());
                        let byte_off = cell.char_indices().nth(insert_char_off).map(|(i,_)| i).unwrap_or(cell.len());
                        cell.insert_str(byte_off, new_text);
                        *source = gfm_rebuild_from_strings(headers, rows);
                        self.cursor.char_offset = insert_char_off + new_text.chars().count();
                    }
                }
                self.marked_range = None;
                cx.notify();
                return;
            }
        }
        // ── Normal paragraph text ─────────────────────────────────────────────
        // Resolve the range to delete
        let range = range_utf16.map(|r| {
            let s = self.utf16_to_char_offset_in_para(r.start);
            let e = self.utf16_to_char_offset_in_para(r.end);
            s..e
        }).or_else(|| {
            self.marked_range.as_ref().map(|mr| {
                let s = bytes_to_chars(&self.current_para_content(), mr.start);
                let e = bytes_to_chars(&self.current_para_content(), mr.end);
                s..e
            })
        }).unwrap_or_else(|| self.cursor.char_offset..self.cursor.char_offset);

        // Delete the range
        if range.start < range.end {
            let del_at = DocCursor { para_idx: self.cursor.para_idx, char_offset: range.start };
            self.do_delete_chars(del_at, range.end - range.start);
            self.cursor = del_at;
        }

        // Insert new text
        if !new_text.is_empty() {
            self.cursor = self.do_insert(self.cursor, new_text);
        }

        self.marked_range = None;
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // ── Table cell: IME mark/replace in active cell ───────────────────────
        if let Some((tr, tc)) = self.table_cursor {
            let para_idx = self.cursor.para_idx;
            if matches!(&self.paragraphs[para_idx].kind, ParagraphKind::Table { .. }) {
                let offset = self.cursor.char_offset;
                if let ParagraphKind::Table { headers, rows, source } = &mut self.paragraphs[para_idx].kind {
                    let cell: &mut String = if tr == 0 { &mut headers[tc] } else { &mut rows[tr - 1][tc] };
                    let char_count = cell.chars().count();
                    let safe_off = offset.min(char_count);
                    let (del_start, del_end) = if let Some(ref r) = range_utf16 {
                        let s = utf16_to_char_offset_in_str(cell, r.start);
                        let e = utf16_to_char_offset_in_str(cell, r.end);
                        (s, e)
                    } else {
                        (safe_off, safe_off)
                    };
                    if del_start < del_end {
                        let byte_s = cell.char_indices().nth(del_start).map(|(i,_)| i).unwrap_or(cell.len());
                        let byte_e = cell.char_indices().nth(del_end).map(|(i,_)| i).unwrap_or(cell.len());
                        cell.drain(byte_s..byte_e);
                    }
                    let insert_off = del_start.min(cell.chars().count());
                    let byte_off = cell.char_indices().nth(insert_off).map(|(i,_)| i).unwrap_or(cell.len());
                    let insert_byte_start = byte_off;
                    if !new_text.is_empty() {
                        cell.insert_str(byte_off, new_text);
                        self.cursor.char_offset = insert_off + new_text.chars().count();
                    }
                    *source = gfm_rebuild_from_strings(headers, rows);
                    let mark_byte_end = insert_byte_start + new_text.len();
                    self.marked_range = Some(insert_byte_start..mark_byte_end);
                    if let Some(r) = new_selected_range_utf16 {
                        // best-effort: adjust cursor offset within new_text
                        let adj = utf16_to_char_offset_in_str(new_text, r.end.saturating_sub(r.start));
                        self.cursor.char_offset = insert_off + adj;
                    }
                }
                cx.notify();
                return;
            }
        }
        // ── Normal paragraph text ─────────────────────────────────────────────
        let range = range_utf16.map(|r| {
            let s = self.utf16_to_char_offset_in_para(r.start);
            let e = self.utf16_to_char_offset_in_para(r.end);
            s..e
        }).unwrap_or(self.cursor.char_offset..self.cursor.char_offset);

        if range.start < range.end {
            let del_at = DocCursor { para_idx: self.cursor.para_idx, char_offset: range.start };
            self.do_delete_chars(del_at, range.end - range.start);
            self.cursor = del_at;
        }

        let insert_at = self.cursor;
        let text = self.current_para_content();
        let insert_byte_start: usize = text
            .char_indices()
            .nth(insert_at.char_offset)
            .map(|(b, _)| b)
            .unwrap_or(text.len());

        if !new_text.is_empty() {
            self.cursor = self.do_insert(self.cursor, new_text);
        }

        // Mark the newly inserted range
        let mark_byte_end = insert_byte_start + new_text.len();
        self.marked_range = Some(insert_byte_start..mark_byte_end);

        if let Some(new_sel_utf16) = new_selected_range_utf16 {
            let s = self.utf16_to_char_offset_in_para(new_sel_utf16.start);
            let e = self.utf16_to_char_offset_in_para(new_sel_utf16.end);
            let anchor = DocCursor { para_idx: self.cursor.para_idx, char_offset: insert_at.char_offset + s };
            let focus = DocCursor { para_idx: self.cursor.para_idx, char_offset: insert_at.char_offset + e };
            self.selection = Some(DocSelection { anchor, focus });
            self.cursor = focus;
        }

        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        _bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        self.last_bounds
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let bounds = self.last_bounds?;
        let cursor = self.cursor_from_point(point, bounds);
        Some(self.char_offset_to_utf16_for_para(cursor.para_idx, cursor.char_offset))
    }
}

impl Render for DocumentEditorState {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        DocumentEditorElement { state: cx.entity() }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Layout constants
// ─────────────────────────────────────────────────────────────────────────────

const LEFT_MARGIN: f32 = 48.0;
const TOP_PADDING: f32 = 24.0;
const PARA_GAP: f32 = 8.0;
const VIEWPORT_OVERSCAN_PX: f32 = 300.0;
/// Fallback height used before a mermaid diagram has been rendered for the first time.
const MERMAID_HEIGHT_LOADING: f32 = 120.0;
/// Maximum display height for a mermaid diagram (prevents extremely tall diagrams).
const MAX_MERMAID_HEIGHT: f32 = 600.0;
/// Minimum display height so even tiny diagrams have a visible block.
const MIN_MERMAID_HEIGHT: f32 = 80.0;
const IMAGE_BLOCK_DEFAULT_HEIGHT: f32 = 300.0;

// Per-process image decode cache: path string → decoded BGRA RenderImage.
// Using thread_local avoids any GPUI state entanglement; images are decoded
// once on the first paint frame and served from memory thereafter.
thread_local! {
    static IMAGE_CACHE: RefCell<std::collections::HashMap<String, Arc<RenderImage>>> =
        RefCell::new(std::collections::HashMap::new());
    /// Mermaid source hash → rendered RenderImage (Ok) or failed flag (Err).
    static MERMAID_CACHE: RefCell<std::collections::HashMap<u64, Result<Arc<RenderImage>, ()>>> =
        RefCell::new(std::collections::HashMap::new());
    /// Mermaid source hash → natural (logical) dimensions (width, height) in SVG pixels.
    static MERMAID_DIMS_CACHE: RefCell<std::collections::HashMap<u64, (f32, f32)>> =
        RefCell::new(std::collections::HashMap::new());
}

/// Compute a stable u64 hash for a mermaid source string.
/// Uses the same seed as preview_pane so both share the same /tmp PNG + .dims files.
fn mermaid_source_hash(source: &str) -> u64 {
    let mut h = DefaultHasher::new();
    "mermaid-cache-v6".hash(&mut h);
    source.hash(&mut h);
    h.finish()
}

/// Return the proportional display height for a mermaid block given the container width.
/// Falls back to MERMAID_HEIGHT_LOADING if dimensions are not yet known.
fn mermaid_display_height(key: u64, block_w: f32) -> f32 {
    let dims = MERMAID_DIMS_CACHE.with(|c| c.borrow().get(&key).copied());
    if let Some((nw, nh)) = dims {
        if nw > 0.0 {
            let h = nh * (block_w / nw);
            return h.clamp(MIN_MERMAID_HEIGHT, MAX_MERMAID_HEIGHT);
        }
    }
    MERMAID_HEIGHT_LOADING
}
const TABLE_ROW_H: f32 = 28.0;
const TABLE_CELL_PAD_X: f32 = 8.0;
const TABLE_CELL_PAD_Y: f32 = 5.0;

/// Regenerate GFM markdown source from plain-string headers + rows.
fn gfm_rebuild_from_strings(headers: &[String], rows: &[Vec<String>]) -> String {
    let mut out = String::new();
    out.push('|');
    for h in headers { out.push(' '); out.push_str(h); out.push_str(" |"); }
    out.push('\n');
    out.push('|');
    for _ in headers { out.push_str(" --- |"); }
    out.push('\n');
    for row in rows {
        out.push('|');
        for ci in 0..headers.len() {
            let cell = row.get(ci).map(|s| s.as_str()).unwrap_or("");
            out.push(' '); out.push_str(cell); out.push_str(" |");
        }
        out.push('\n');
    }
    out
}

/// Parse a simple markdown link that occupies the full cell text: `[label](url)`.
fn parse_markdown_link_exact(text: &str) -> Option<(String, String)> {
    let t = text.trim();
    if !t.starts_with('[') || !t.ends_with(')') {
        return None;
    }
    let close_bracket = t.find("](")?;
    if close_bracket < 1 || close_bracket + 2 >= t.len() {
        return None;
    }
    let label = &t[1..close_bracket];
    let url = &t[(close_bracket + 2)..(t.len() - 1)];
    if label.is_empty() || url.is_empty() {
        return None;
    }
    Some((label.to_string(), url.to_string()))
}

/// Total pixel height for a table paragraph (header row + data rows + border).
fn table_height(headers: &[String], rows: &[Vec<String>]) -> f32 {
    let _ = headers;
    TABLE_ROW_H * (1 + rows.len()) as f32 + 2.0
}

/// Open a URL or local file path using the system default handler.
fn open_url(url: &str) {
    #[cfg(target_os = "linux")]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(url).spawn();
    // Use rundll32 instead of `cmd /C start` to avoid shell-metacharacter injection.
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler", url])
        .spawn();
    // Silence unused-variable warning on unsupported platforms
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    let _ = url;
}

fn para_font_size(kind: &ParagraphKind) -> f32 {
    match kind {
        ParagraphKind::Heading(1) => 26.0,
        ParagraphKind::Heading(2) => 21.0,
        ParagraphKind::Heading(3) => 18.0,
        ParagraphKind::CodeFence(_) => 13.0,
        ParagraphKind::Table { .. } => 13.0,
        _ => 15.0,
    }
}

fn para_line_height(kind: &ParagraphKind) -> f32 {
    match kind {
        ParagraphKind::Heading(1) => 38.0,
        ParagraphKind::Heading(2) => 32.0,
        ParagraphKind::Heading(3) => 28.0,
        ParagraphKind::CodeFence(_) => 20.0,
        ParagraphKind::Table { .. } => TABLE_ROW_H,
        _ => 24.0,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Byte / char helpers
// ─────────────────────────────────────────────────────────────────────────────

fn chars_to_bytes(text: &str, char_offset: usize) -> usize {
    text.char_indices()
        .nth(char_offset)
        .map(|(b, _)| b)
        .unwrap_or(text.len())
}

fn coalesce_spans(spans: Vec<InlineSpan>) -> Vec<InlineSpan> {
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
        merged.push(span);
    }

    if merged.is_empty() {
        vec![InlineSpan { text: String::new(), format: InlineFormat::Plain, link_url: None }]
    } else {
        merged
    }
}

fn bytes_to_chars(text: &str, byte_offset: usize) -> usize {
    let safe_offset = byte_offset.min(text.len());
    text[..safe_offset].chars().count()
}

/// Convert a UTF-16 code-unit offset to a Unicode char offset within `text`.
fn utf16_to_char_offset_in_str(text: &str, utf16_off: usize) -> usize {
    let mut u16_count = 0usize;
    for (char_idx, ch) in text.chars().enumerate() {
        if u16_count >= utf16_off { return char_idx; }
        u16_count += ch.len_utf16();
    }
    text.chars().count()
}

/// Convert a Unicode char offset to a UTF-16 code-unit offset within `text`.
fn char_offset_to_utf16_in_str(text: &str, char_off: usize) -> usize {
    let mut u16_count = 0usize;
    for (idx, ch) in text.chars().enumerate() {
        if idx >= char_off { break; }
        u16_count += ch.len_utf16();
    }
    u16_count
}

/// Given a click x-position (relative to the left margin) and the precomputed
/// glyph x-positions for a visual line, return the character column (0-based)
/// where a cursor should be placed.
///
/// `xs[i]` is the left-edge x of char `i`; `xs[char_count]` is the right edge
/// of the last char.  Returns a column clamped to `[0, char_count]`.
fn find_closest_char_col(x: Pixels, xs: &[Pixels]) -> usize {
    if xs.is_empty() { return 0; }
    // Find the first entry whose left-edge is strictly to the right of x.
    match xs.iter().position(|&ex| ex > x) {
        // x is past all glyph edges → cursor after last char.
        None => xs.len().saturating_sub(1),
        // x is before the first glyph → cursor before first char.
        Some(0) => 0,
        // x falls between xs[i-1] and xs[i]; snap to the closer side.
        Some(i) => {
            if x - xs[i - 1] <= xs[i] - x { i - 1 } else { i }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// TextRun builder
// ─────────────────────────────────────────────────────────────────────────────

fn span_font(format: InlineFormat, base_font: &Font, mono_font_family: &SharedString) -> Font {
    match format {
        InlineFormat::Plain => base_font.clone(),
        InlineFormat::Bold => Font {
            weight: FontWeight::BOLD,
            ..base_font.clone()
        },
        InlineFormat::Italic => Font {
            style: FontStyle::Italic,
            ..base_font.clone()
        },
        InlineFormat::Underline | InlineFormat::Strikethrough => base_font.clone(),
        InlineFormat::Code => Font {
            family: mono_font_family.clone(),
            weight: FontWeight::NORMAL,
            style: FontStyle::Normal,
            ..base_font.clone()
        },
        InlineFormat::Link => base_font.clone(),
    }
}

/// Build (line_text, text_runs) for the slice [char_start, char_end) within `spans`.
fn spans_to_text_runs(
    spans: &[InlineSpan],
    char_start: usize,
    char_end: usize,
    base_font: &Font,
    base_color: Hsla,
    emphasis_color: Hsla,
    code_color: Hsla,
    link_color: Hsla,
    mono_font_family: &SharedString,
) -> (String, Vec<TextRun>) {
    let mut line_text = String::new();
    let mut runs: Vec<TextRun> = Vec::new();
    let mut pos = 0usize;

    for span in spans {
        let span_char_len = span.text.chars().count();
        let span_start = pos;
        let span_end = span_start + span_char_len;

        pos = span_end;

        if span_end <= char_start || span_start >= char_end {
            // No overlap
            continue;
        }

        let overlap_start = span_start.max(char_start);
        let overlap_end = span_end.min(char_end);

        let rel_start = overlap_start - span_start;
        let run_text: String = span.text
            .chars()
            .skip(rel_start)
            .take(overlap_end - overlap_start)
            .collect();

        if run_text.is_empty() { continue; }

        let byte_len = run_text.len();
        let font = span_font(span.format, base_font, mono_font_family);
        let color = match span.format {
            InlineFormat::Code => code_color,
            InlineFormat::Bold | InlineFormat::Italic | InlineFormat::Underline | InlineFormat::Strikethrough => emphasis_color,
            InlineFormat::Link => link_color,
            InlineFormat::Plain => base_color,
        };

        let underline = match span.format {
            InlineFormat::Underline => Some(UnderlineStyle {
                thickness: px(1.0),
                color: Some(emphasis_color),
                wavy: false,
            }),
            InlineFormat::Link => Some(UnderlineStyle {
                thickness: px(1.0),
                color: Some(link_color),
                wavy: false,
            }),
            _ => None,
        };

        let strikethrough = match span.format {
            InlineFormat::Strikethrough => Some(StrikethroughStyle {
                thickness: px(1.0),
                color: Some(emphasis_color),
            }),
            _ => None,
        };

        runs.push(TextRun {
            len: byte_len,
            font,
            color,
            background_color: None,
            underline,
            strikethrough,
        });
        line_text.push_str(&run_text);
    }

    if runs.is_empty() {
        // Emit a zero-width transparent run so shape_line never panics
        runs.push(TextRun {
            len: 0,
            font: base_font.clone(),
            color: base_color,
            background_color: None,
            underline: None,
            strikethrough: None,
        });
    }

    (line_text, runs)
}

// ─────────────────────────────────────────────────────────────────────────────
// DocumentEditorElement
// ─────────────────────────────────────────────────────────────────────────────

struct DocumentEditorElement {
    state: Entity<DocumentEditorState>,
}

struct EditorPrepaintState;

impl IntoElement for DocumentEditorElement {
    type Element = Self;
    fn into_element(self) -> Self::Element { self }
}

impl Element for DocumentEditorElement {
    type RequestLayoutState = ();
    type PrepaintState = EditorPrepaintState;

    fn id(&self) -> Option<ElementId> { None }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> { None }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let state = self.state.read(cx);
        let mut total_h = TOP_PADDING;

        for (para_idx, para) in state.paragraphs.iter().enumerate() {
            let lh = para_line_height(&para.kind);
            match &para.kind {
                ParagraphKind::Mermaid(_) => {
                    let source = para.plain_text();
                    let key = mermaid_source_hash(&source);
                    let approx_w = state.last_bounds
                        .map(|b| f32::from(b.size.width) - LEFT_MARGIN * 2.0)
                        .unwrap_or(760.0);
                    total_h += mermaid_display_height(key, approx_w) + PARA_GAP;
                }
                ParagraphKind::Image { height, .. } => {
                    total_h += height + PARA_GAP;
                }
                ParagraphKind::Table { headers, rows, .. } => {
                    total_h += table_height(headers, rows) + PARA_GAP;
                }
                _ => {
                    // Count visual lines by counting '\n's in plain text;
                    // if we have cached wrapped heights from a previous paint
                    // pass, use those instead for accurate wrapping scroll size.
                    let text = para.plain_text();
                    let line_count = (text.chars().filter(|&c| c == '\n').count() + 1).max(1);
                    let cached_h = state.para_visual_heights.get(para_idx).copied();
                    total_h += cached_h.unwrap_or(lh * line_count as f32) + PARA_GAP;
                }
            }
        }
        total_h += TOP_PADDING; // bottom padding

        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = px(total_h).into();

        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let state = self.state.read(cx);
        let mut total_h = TOP_PADDING;
        for (para_idx, para) in state.paragraphs.iter().enumerate() {
            let lh = para_line_height(&para.kind);
            match &para.kind {
                ParagraphKind::Mermaid(_) => {
                    let source = para.plain_text();
                    let key = mermaid_source_hash(&source);
                    let approx_w = state.last_bounds
                        .map(|b| f32::from(b.size.width) - LEFT_MARGIN * 2.0)
                        .unwrap_or(760.0);
                    total_h += mermaid_display_height(key, approx_w) + PARA_GAP;
                }
                ParagraphKind::Image { height, .. } => total_h += height + PARA_GAP,
                ParagraphKind::Table { headers, rows, .. } => {
                    total_h += table_height(headers, rows) + PARA_GAP;
                }
                _ => {
                    let text = para.plain_text();
                    let line_count = (text.chars().filter(|&c| c == '\n').count() + 1).max(1);
                    let cached_h = state.para_visual_heights.get(para_idx).copied();
                    total_h += cached_h.unwrap_or(lh * line_count as f32) + PARA_GAP;
                }
            }
        }

        let _ = total_h;
        EditorPrepaintState
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.state.read(cx).focus_handle.clone();

        // Set pointer cursor when hovering a link span (must be called during paint).
        let hovered_link = self.state.read(cx).hovered_link;
        window.set_window_cursor_style(if hovered_link {
            CursorStyle::PointingHand
        } else {
            CursorStyle::default()
        });

        // Register input handler (must happen in paint)
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.state.clone()),
            cx,
        );

        self.state.update(cx, |s, _| { s.last_bounds = Some(bounds); });

        let theme = use_theme();
        let text_style = window.text_style();
        // Ensure the editor uses the design-system body family (Inter), which
        // has embedded bold faces. Relying on window default style can resolve
        // to a family with no visible weight/style variants.
        let mut base_font = text_style.font();
        base_font.family = theme.tokens.font_family.clone().into();
        base_font.weight = FontWeight::NORMAL;
        base_font.style = FontStyle::Normal;
        let mono_family: SharedString = theme.tokens.font_mono.clone().into();
        let emphasis_color = theme.tokens.primary;
        let code_color = theme.tokens.primary.opacity(0.85);
        let link_color = gpui::hsla(200.0 / 360.0, 0.95, 0.72, 1.0);
        let selection_color = theme.tokens.primary.opacity(0.22);
        let caret_color = theme.tokens.primary;
        let viewport = window.viewport_size();
        let viewport_top = px(0.0);
        let viewport_bottom = px(f32::from(viewport.height));
        let overscan = px(VIEWPORT_OVERSCAN_PX);

        let (
            paragraphs,
            cursor,
            selection,
            table_cursor,
            cached_para_heights,
            doc_dir,
            hovered_image_para,
            hovered_mermaid_para,
        ) = {
            let s = self.state.read(cx);
            (
                s.paragraphs.clone(),
                s.cursor,
                s.selection,
                s.table_cursor,
                s.para_visual_heights.clone(),
                s.document_dir.clone(),
                s.hovered_image_para,
                s.hovered_mermaid_para,
            )
        };

        let mut current_y: f32 = TOP_PADDING;
        let left: Pixels = bounds.left() + px(LEFT_MARGIN);
        let block_w = bounds.size.width - px(LEFT_MARGIN * 2.0);
        let mut visual_lines: Vec<VisualLine> = Vec::new();
        let mut line_layouts: Vec<Option<ShapedLine>> = Vec::new();
        // Per-paragraph visual heights accumulated this frame.
        let mut para_visual_heights: Vec<f32> = Vec::new();
        let is_in_viewport = |top_doc_y: f32, block_h: f32| {
            let top_screen = bounds.top() + px(top_doc_y);
            let bottom_screen = top_screen + px(block_h.max(1.0));
            bottom_screen >= viewport_top - overscan && top_screen <= viewport_bottom + overscan
        };

        for (para_idx, para) in paragraphs.iter().enumerate() {
            let kind = &para.kind;
            let font_size_px = para_font_size(kind);
            let line_height = para_line_height(kind);
            let font_size = px(font_size_px);
            let lh_px = px(line_height);
            let fallback_para_h = cached_para_heights
                .get(para_idx)
                .copied()
                .unwrap_or_else(|| {
                    let text = para.plain_text();
                    let line_count = (text.chars().filter(|&c| c == '\n').count() + 1).max(1);
                    line_height * line_count as f32
                })
                .max(line_height);

            let para_color = match kind {
                ParagraphKind::Heading(_) => theme.tokens.foreground,
                ParagraphKind::BlockQuote => theme.tokens.muted_foreground,
                _ => theme.tokens.foreground,
            };

            match kind {
                ParagraphKind::Mermaid(_) => {
                    if !is_in_viewport(current_y, fallback_para_h) {
                        visual_lines.push(VisualLine {
                            para_idx,
                            char_start: 0,
                            char_end: para.char_count(),
                            top_px: current_y,
                            height_px: fallback_para_h,
                            font_size_px,
                            is_mermaid: true,
                            glyph_xs: vec![],
                        });
                        line_layouts.push(None);
                        para_visual_heights.push(fallback_para_h);
                        current_y += fallback_para_h + PARA_GAP;
                        continue;
                    }

                    // The actual mermaid source is stored in para.spans, not in the PathBuf field.
                    let mermaid_source = para.plain_text();
                    let top_screen = bounds.top() + px(current_y);
                    let block_w = bounds.size.width - px(LEFT_MARGIN * 2.0);
                    let key = mermaid_source_hash(&mermaid_source);

                    // Render to PNG once, cache the decoded RenderImage.
                    let render_image: Option<Arc<RenderImage>> = MERMAID_CACHE.with(|cache| {
                        let mut map = cache.borrow_mut();
                        if let Some(entry) = map.get(&key) {
                            return entry.as_ref().ok().cloned();
                        }
                        // Generate PNG to temp cache dir.
                        let cache_dir = std::env::temp_dir().join("octodocs-mermaid-cache");
                        let _ = std::fs::create_dir_all(&cache_dir);
                        let png_path = cache_dir.join(format!("{key}.png"));
                        let dims_path = cache_dir.join(format!("{key}.dims"));
                        // Load or render the PNG, and always populate the dims cache.
                        let result = if png_path.exists() {
                            // Read dims sidecar if present.
                            if let Ok(raw) = std::fs::read_to_string(&dims_path) {
                                let mut parts = raw.split_whitespace();
                                if let (Some(w), Some(h)) = (parts.next(), parts.next()) {
                                    if let (Ok(w), Ok(h)) = (w.parse::<f32>(), h.parse::<f32>()) {
                                        MERMAID_DIMS_CACHE.with(|c| c.borrow_mut().insert(key, (w, h)));
                                    }
                                }
                            }
                            std::fs::read(&png_path).ok()
                        } else {
                            match octodocs_core::mermaid::render_png(&mermaid_source, &png_path) {
                                Ok((nw, nh)) => {
                                    MERMAID_DIMS_CACHE.with(|c| c.borrow_mut().insert(key, (nw, nh)));
                                    let _ = std::fs::write(&dims_path, format!("{nw} {nh}"));
                                    std::fs::read(&png_path).ok()
                                }
                                Err(_) => None,
                            }
                        };
                        let ri = result
                            .and_then(|bytes| image::load_from_memory(&bytes).ok())
                            .map(|dyn_img| {
                                let mut rgba = dyn_img.into_rgba8();
                                // GPUI requires BGRA: swap R and B channels.
                                for pixel in rgba.chunks_exact_mut(4) { pixel.swap(0, 2); }
                                Arc::new(RenderImage::new([image::Frame::new(rgba)]))
                            });
                        let out = ri.clone();
                        map.insert(key, ri.ok_or(()));
                        out
                    });

                    if let Some(ri) = render_image {
                        let img_size = ri.size(0);
                        let iw = img_size.width.0 as f32;
                        let ih = img_size.height.0 as f32;
                        // Compute proportional display height from natural dims.
                        // Use the RenderImage pixel aspect ratio as fallback if dims cache isn't populated yet.
                        let display_h = mermaid_display_height(key, f32::from(block_w));
                        if iw > 0.0 && ih > 0.0 {
                            // Fit the image into the block proportionally (no letterbox padding).
                            let display_w = f32::from(block_w);
                            let scale = (display_w / iw).min(display_h / ih);
                            let dw = iw * scale;
                            let dh = ih * scale;
                            let ox = (display_w - dw) / 2.0;
                            let oy = (display_h - dh) / 2.0;
                            let paint_bounds = Bounds::new(
                                point(left + px(ox), top_screen + px(oy)),
                                size(px(dw), px(dh)),
                            );
                            let _ = window.paint_image(paint_bounds, gpui::Corners::default(), ri, 0, false);
                        }
                        // Magnifier badge in top-right corner when hovered.
                        let is_hovered = hovered_mermaid_para == Some(para_idx);
                        if is_hovered {
                            let badge_size = 32.0_f32;
                            let badge_x = left + block_w - px(badge_size + 8.0);
                            let badge_y = top_screen + px(8.0);
                            let badge_bounds = Bounds::new(
                                point(badge_x, badge_y),
                                size(px(badge_size), px(badge_size)),
                            );
                            window.paint_quad(gpui::PaintQuad {
                                bounds: badge_bounds,
                                corner_radii: gpui::Corners::all(px(badge_size / 2.0)),
                                background: gpui::rgba(0x000000cc).into(),
                                border_widths: gpui::Edges::all(px(0.0)),
                                border_color: gpui::transparent_black(),
                                border_style: gpui::BorderStyle::Solid,
                                continuous_corners: false,
                            });
                            let icon = "\u{1f50d}"; // 🔍
                            let icon_run = TextRun {
                                len: icon.len(),
                                font: base_font.clone(),
                                color: gpui::white(),
                                background_color: None,
                                underline: None,
                                strikethrough: None,
                            };
                            let shaped = window.text_system().shape_line(
                                icon.into(), px(16.0), &[icon_run], None,
                            );
                            let _ = shaped.paint(
                                point(badge_x + px(8.0), badge_y + px(8.0)),
                                px(24.0), window, cx,
                            );
                        }
                        visual_lines.push(VisualLine {
                            para_idx,
                            char_start: 0,
                            char_end: 0,
                            top_px: current_y,
                            height_px: display_h,
                            font_size_px,
                            is_mermaid: true,
                            glyph_xs: vec![],
                        });
                        line_layouts.push(None);
                        para_visual_heights.push(display_h);
                        current_y += display_h + PARA_GAP;
                    } else {
                        // Fallback placeholder when rendering failed.
                        let display_h = MERMAID_HEIGHT_LOADING;
                        let mermaid_bounds = Bounds::new(
                            point(left, top_screen),
                            size(block_w, px(display_h)),
                        );
                        window.paint_quad(fill(mermaid_bounds, theme.tokens.muted.opacity(0.3)));
                        let label_run = TextRun {
                            len: "Mermaid diagram".len(),
                            font: base_font.clone(),
                            color: theme.tokens.muted_foreground,
                            background_color: None,
                            underline: None,
                            strikethrough: None,
                        };
                        let shaped = window.text_system().shape_line(
                            "Mermaid diagram".into(),
                            px(13.0),
                            &[label_run],
                            None,
                        );
                        let _ = shaped.paint(
                            point(left, top_screen + px(display_h / 2.0 - 8.0)),
                            px(20.0),
                            window,
                            cx,
                        );
                        visual_lines.push(VisualLine {
                            para_idx,
                            char_start: 0,
                            char_end: 0,
                            top_px: current_y,
                            height_px: display_h,
                            font_size_px,
                            is_mermaid: true,
                            glyph_xs: vec![],
                        });
                        line_layouts.push(None);
                        para_visual_heights.push(display_h);
                        current_y += display_h + PARA_GAP;
                    }
                    continue;
                }
                ParagraphKind::Image { path, height, .. } => {
                    let img_height = *height;
                    if !is_in_viewport(current_y, img_height) {
                        visual_lines.push(VisualLine {
                            para_idx,
                            char_start: 0,
                            char_end: para.char_count(),
                            top_px: current_y,
                            height_px: img_height,
                            font_size_px,
                            is_mermaid: false,
                            glyph_xs: vec![],
                        });
                        line_layouts.push(None);
                        para_visual_heights.push(img_height);
                        current_y += img_height + PARA_GAP;
                        continue;
                    }

                    let abs_path = doc_dir
                        .as_ref()
                        .map(|d| d.join(path.as_str()))
                        .unwrap_or_else(|| PathBuf::from(path.as_str()));
                    let cache_key = abs_path.to_string_lossy().to_string();

                    // Read from the async-populated cache; never decode on the UI thread.
                    let render_image: Option<Arc<RenderImage>> = self.state.read(cx)
                        .image_cache
                        .read()
                        .ok()
                        .and_then(|c| c.get(&cache_key).cloned());

                    let top_screen = bounds.top() + px(current_y);
                    let block_w = bounds.size.width - px(LEFT_MARGIN * 2.0);

                    if let Some(ri) = render_image {
                        // Letterbox fit inside the block.
                        let img_size = ri.size(0);
                        let iw = img_size.width.0 as f32;
                        let ih = img_size.height.0 as f32;
                        if iw > 0.0 && ih > 0.0 {
                            let scale = (f32::from(block_w) / iw).min(img_height / ih);
                            let dw = iw * scale;
                            let dh = ih * scale;
                            let ox = (f32::from(block_w) - dw) / 2.0;
                            let oy = (img_height - dh) / 2.0;
                            let paint_bounds = Bounds::new(
                                point(left + px(ox), top_screen + px(oy)),
                                size(px(dw), px(dh)),
                            );
                            let _ = window.paint_image(paint_bounds, gpui::Corners::default(), ri, 0, false);
                        }
                    } else {
                        // Placeholder while file is missing/loading.
                        window.paint_quad(gpui::PaintQuad {
                            bounds: Bounds::new(point(left, top_screen), size(block_w, px(img_height))),
                            corner_radii: gpui::Corners::all(px(4.0)),
                            background: theme.tokens.muted.opacity(0.25).into(),
                            border_widths: gpui::Edges::all(px(1.0)),
                            border_color: theme.tokens.border,
                            border_style: gpui::BorderStyle::Solid,
                            continuous_corners: false,
                        });
                        let filename = std::path::Path::new(path.as_str())
                            .file_name().map(|f| f.to_string_lossy().to_string())
                            .unwrap_or_else(|| path.clone());
                        let label = format!("\u{1f5bc}  {filename}");
                        let label_run = TextRun { len: label.len(), font: base_font.clone(),
                            color: theme.tokens.muted_foreground, background_color: None,
                            underline: None, strikethrough: None };
                        let shaped = window.text_system().shape_line(label.into(), px(13.0), &[label_run], None);
                        let _ = shaped.paint(point(left + px(8.0), top_screen + px(img_height / 2.0 - 8.0)), px(20.0), window, cx);
                    }

                    // Resize handle bar at the bottom of the block.
                    let handle_y = top_screen + px(img_height - 5.0);
                    let handle_cx = left + block_w * 0.5;
                    window.paint_quad(fill(
                        Bounds::new(point(handle_cx - px(24.0), handle_y), size(px(48.0), px(4.0))),
                        theme.tokens.border.opacity(0.55),
                    ));

                    // Magnifier badge in top-right corner when hovered.
                    let is_hovered = hovered_image_para == Some(para_idx);
                    if is_hovered {
                        let badge_size = 32.0_f32;
                        let badge_x = left + block_w - px(badge_size + 8.0);
                        let badge_y = top_screen + px(8.0);
                        let badge_bounds = Bounds::new(
                            point(badge_x, badge_y),
                            size(px(badge_size), px(badge_size)),
                        );
                        window.paint_quad(gpui::PaintQuad {
                            bounds: badge_bounds,
                            corner_radii: gpui::Corners::all(px(badge_size / 2.0)),
                            background: gpui::rgba(0x000000cc).into(),
                            border_widths: gpui::Edges::all(px(0.0)),
                            border_color: gpui::transparent_black(),
                            border_style: gpui::BorderStyle::Solid,
                            continuous_corners: false,
                        });
                        let icon = "\u{1f50d}"; // 🔍
                        let icon_run = TextRun {
                            len: icon.len(),
                            font: base_font.clone(),
                            color: gpui::white(),
                            background_color: None,
                            underline: None,
                            strikethrough: None,
                        };
                        let shaped = window.text_system().shape_line(
                            icon.into(), px(16.0), &[icon_run], None,
                        );
                        let _ = shaped.paint(
                            point(badge_x + px(8.0), badge_y + px(8.0)),
                            px(24.0), window, cx,
                        );
                    }

                    visual_lines.push(VisualLine {
                        para_idx,
                        char_start: 0,
                        char_end: 0,
                        top_px: current_y,
                        height_px: img_height,
                        font_size_px,
                        is_mermaid: false,
                        glyph_xs: vec![],
                    });
                    line_layouts.push(None);
                    para_visual_heights.push(img_height);
                    current_y += img_height + PARA_GAP;
                    continue;
                }
                ParagraphKind::Table { headers, rows, .. } => {
                    let th = table_height(headers, rows);
                    if !is_in_viewport(current_y, th) {
                        visual_lines.push(VisualLine {
                            para_idx,
                            char_start: 0,
                            char_end: para.char_count(),
                            top_px: current_y,
                            height_px: th,
                            font_size_px,
                            is_mermaid: false,
                            glyph_xs: vec![],
                        });
                        line_layouts.push(None);
                        para_visual_heights.push(th);
                        current_y += th + PARA_GAP;
                        continue;
                    }
                    let table_w = bounds.size.width - px(LEFT_MARGIN * 2.0);
                    let col_count = headers.len().max(1);
                    // col_w_f32: raw f32 for arithmetic (Pixels/Pixels = f32)
                    let col_w_f32: f32 = table_w / px(col_count as f32);
                    let top_screen = bounds.top() + px(current_y);

                    // Background
                    let table_bounds = Bounds::new(
                        point(left, top_screen),
                        size(table_w, px(th)),
                    );
                    window.paint_quad(fill(table_bounds, theme.tokens.muted.opacity(0.08)));

                    // Horizontal separator after header
                    let sep_y = top_screen + px(TABLE_ROW_H);
                    let sep = Bounds::new(
                        point(left, sep_y),
                        size(table_w, px(1.0)),
                    );
                    window.paint_quad(fill(sep, theme.tokens.border));

                    // Outer border
                    for (bx, by, bw, bh) in [
                        (left, top_screen, table_w, px(1.0)),                        // top
                        (left, top_screen + px(th - 1.0), table_w, px(1.0)),          // bottom
                        (left, top_screen, px(1.0), px(th)),                           // left
                        (left + table_w - px(1.0), top_screen, px(1.0), px(th)),       // right
                    ] {
                        window.paint_quad(fill(Bounds::new(point(bx, by), size(bw, bh)), theme.tokens.border));
                    }

                    // Vertical column separators
                    for ci in 1..headers.len() {
                        let cx_px = left + px(col_w_f32 * ci as f32);
                        let vert = Bounds::new(point(cx_px, top_screen), size(px(1.0), px(th)));
                        window.paint_quad(fill(vert, theme.tokens.border));
                    }

                    // Paint cell text
                    let cell_font_size = px(13.0);
                    let cell_lh = px(TABLE_ROW_H);

                    let mut paint_cells = |cells: &[String], row_top: Pixels, bold: bool, row_index: usize, window: &mut Window| {
                        let mut header_font = base_font.clone();
                        if bold { header_font.weight = gpui::FontWeight::BOLD; }
                        for (ci, cell_text) in cells.iter().enumerate() {
                            let cell_x = left + px(col_w_f32 * ci as f32 + TABLE_CELL_PAD_X);
                            let cell_y = row_top + px(TABLE_CELL_PAD_Y);
                            let cell_content_w = (col_w_f32 - TABLE_CELL_PAD_X * 2.0).max(4.0);
                            let (render_text, render_is_link) = if let Some((label, _url)) = parse_markdown_link_exact(cell_text) {
                                (label, true)
                            } else {
                                (cell_text.clone(), false)
                            };
                            // Clip text to column width
                            let max_chars = (cell_content_w / 7.5).max(4.0) as usize;
                            let display: String = render_text.chars().take(max_chars.max(4)).collect();
                            if display.is_empty() { continue; }
                            let underline = if render_is_link {
                                Some(UnderlineStyle {
                                    thickness: px(1.0),
                                    color: Some(link_color),
                                    wavy: false,
                                })
                            } else {
                                None
                            };
                            let run = TextRun {
                                len: display.len(),
                                font: header_font.clone(),
                                color: if render_is_link { link_color } else { theme.tokens.foreground },
                                background_color: None,
                                underline,
                                strikethrough: None,
                            };
                            let shaped = window.text_system().shape_line(
                                display.clone().into(),
                                cell_font_size,
                                &[run],
                                None,
                            );

                            if let Some(sel) = selection {
                                if let Some((tc_tr, tc_tc)) = table_cursor {
                                    if cursor.para_idx == para_idx && tc_tr == row_index && tc_tc == ci {
                                        let (sel_start, sel_end) = sel.ordered();
                                        let raw_len = cell_text.chars().count();
                                        let s = sel_start.char_offset.min(raw_len);
                                        let e = sel_end.char_offset.min(raw_len);
                                        if e > s {
                                            let raw_prefix_s: String = cell_text.chars().take(s).collect();
                                            let raw_prefix_e: String = cell_text.chars().take(e).collect();
                                            let x1 = shaped.x_for_index(raw_prefix_s.len());
                                            let x2 = shaped.x_for_index(raw_prefix_e.len());
                                            let width = (x2 - x1).max(px(1.0));
                                            window.paint_quad(fill(
                                                Bounds::new(
                                                    point(cell_x + x1, row_top + px(2.0)),
                                                    size(width, px(TABLE_ROW_H - 4.0)),
                                                ),
                                                selection_color,
                                            ));
                                        }
                                    }
                                }
                            }
                            let _ = shaped.paint(point(cell_x, cell_y), cell_lh, window, cx);
                        }
                    };

                    // Header row (bold)
                    paint_cells(headers, top_screen, true, 0, window);
                    // Data rows
                    for (ri, row) in rows.iter().enumerate() {
                        let row_top = top_screen + px(TABLE_ROW_H * (ri + 1) as f32);
                        // Alternate row bg
                        if ri % 2 == 1 {
                            let row_bg = Bounds::new(
                                point(left + px(1.0), row_top),
                                size(table_w - px(2.0), px(TABLE_ROW_H)),
                            );
                            window.paint_quad(fill(row_bg, theme.tokens.muted.opacity(0.15)));
                        }
                        paint_cells(row, row_top, false, ri + 1, window);
                    }

                    // ── Active cell highlight + text cursor ─────────────────
                    if let Some((tc_tr, tc_tc)) = table_cursor {
                        if cursor.para_idx == para_idx && tc_tc < col_count {
                            let cell_x = left + px(col_w_f32 * tc_tc as f32);
                            let cell_y = top_screen + px(TABLE_ROW_H * tc_tr as f32);
                            let cell_w = px(col_w_f32);
                            let cell_h = px(TABLE_ROW_H);
                            // Highlight fill
                            window.paint_quad(fill(
                                Bounds::new(point(cell_x, cell_y), size(cell_w, cell_h)),
                                theme.tokens.primary.opacity(0.12),
                            ));
                            // Highlight border (4 sides)
                            for (bx, by, bw, bh) in [
                                (cell_x, cell_y, cell_w, px(1.5)),
                                (cell_x, cell_y + cell_h - px(1.5), cell_w, px(1.5)),
                                (cell_x, cell_y, px(1.5), cell_h),
                                (cell_x + cell_w - px(1.5), cell_y, px(1.5), cell_h),
                            ] {
                                window.paint_quad(fill(Bounds::new(point(bx, by), size(bw, bh)), theme.tokens.primary));
                            }
                            // Text cursor caret inside the active cell
                            if focus_handle.is_focused(window) {
                                let cell_text: String = if tc_tr == 0 {
                                    headers.get(tc_tc).cloned().unwrap_or_default()
                                } else {
                                    rows.get(tc_tr - 1).and_then(|r| r.get(tc_tc)).cloned().unwrap_or_default()
                                };
                                let caret_col = cursor.char_offset.min(cell_text.chars().count());
                                let prefix: String = cell_text.chars().take(caret_col).collect();
                                let prefix_w = if prefix.is_empty() {
                                    px(0.0)
                                } else {
                                    let prefix_bytes = prefix.len();
                                    let run = TextRun {
                                        len: prefix_bytes,
                                        font: base_font.clone(),
                                        color: theme.tokens.foreground,
                                        background_color: None,
                                        underline: None,
                                        strikethrough: None,
                                    };
                                    let shaped = window.text_system().shape_line(
                                        prefix.into(),
                                        cell_font_size,
                                        &[run],
                                        None,
                                    );
                                    shaped.x_for_index(prefix_bytes)
                                };
                                let caret_x = cell_x + px(TABLE_CELL_PAD_X) + prefix_w;
                                let caret_y2 = cell_y + px(TABLE_CELL_PAD_Y);
                                window.paint_quad(fill(
                                    Bounds::new(
                                        point(caret_x, caret_y2),
                                        size(px(2.0), px(TABLE_ROW_H - TABLE_CELL_PAD_Y * 2.0)),
                                    ),
                                    caret_color,
                                ));
                            }
                        }
                    }

                    visual_lines.push(VisualLine {
                        para_idx,
                        char_start: 0,
                        char_end: 0,
                        top_px: current_y,
                        height_px: th,
                        font_size_px,
                        is_mermaid: false,
                        glyph_xs: vec![],
                    });
                    line_layouts.push(None);
                    para_visual_heights.push(th);
                    current_y += th + PARA_GAP;
                    continue;
                }
                _ => {}
            }

            if !is_in_viewport(current_y, fallback_para_h) {
                visual_lines.push(VisualLine {
                    para_idx,
                    char_start: 0,
                    char_end: para.char_count(),
                    top_px: current_y,
                    height_px: fallback_para_h,
                    font_size_px,
                    is_mermaid: false,
                    glyph_xs: vec![px(0.0)],
                });
                line_layouts.push(None);
                para_visual_heights.push(fallback_para_h);
                current_y += fallback_para_h + PARA_GAP;
                continue;
            }

            // For code fences, draw a background rect first
            if let ParagraphKind::CodeFence(_) = kind {
                let text = para.plain_text();
                let sublines = text.split('\n').count().max(1);
                let total_height = line_height * sublines as f32 + 8.0;
                let code_bg = Bounds::new(
                    point(left - px(8.0), bounds.top() + px(current_y - 4.0)),
                    size(bounds.size.width - px((LEFT_MARGIN - 8.0) * 2.0), px(total_height)),
                );
                window.paint_quad(fill(code_bg, theme.tokens.muted.opacity(0.3)));
            }

            // Blockquote left bar
            if let ParagraphKind::BlockQuote = kind {
                let text = para.plain_text();
                let sublines = text.split('\n').count().max(1);
                let total_height = line_height * sublines as f32;
                let bar = Bounds::new(
                    point(left - px(12.0), bounds.top() + px(current_y)),
                    size(px(3.0), px(total_height)),
                );
                window.paint_quad(fill(bar, theme.tokens.border));
            }

            // Task list item checkbox
            if let ParagraphKind::TaskListItem { checked } = kind {
                let cb_size = px(14.0);
                let cb_x = left - px(28.0);
                let cb_y = bounds.top() + px(current_y + (line_height - 14.0) / 2.0);
                let cb_bounds = Bounds::new(point(cb_x, cb_y), size(cb_size, cb_size));
                // Draw checkbox border
                window.paint_quad(
                    gpui::PaintQuad {
                        bounds: cb_bounds,
                        corner_radii: gpui::Corners::all(px(2.0)),
                        background: if *checked {
                            theme.tokens.accent.into()
                        } else {
                            gpui::Hsla::transparent_black().into()
                        },
                        border_widths: gpui::Edges::all(px(1.5)),
                        border_color: if *checked {
                            theme.tokens.accent
                        } else {
                            theme.tokens.border
                        },
                        border_style: gpui::BorderStyle::Solid,
                        continuous_corners: false,
                    },
                );
                // Draw check mark for checked state (two paint_quad strokes)
                if *checked {
                    // Horizontal part of checkmark (bottom stroke)
                    let h_stroke = Bounds::new(
                        point(cb_x + px(3.0), cb_y + px(8.0)),
                        size(px(4.0), px(2.0)),
                    );
                    window.paint_quad(fill(h_stroke, gpui::white()));
                    // Diagonal ascending part
                    let v_stroke = Bounds::new(
                        point(cb_x + px(6.0), cb_y + px(4.0)),
                        size(px(2.0), px(6.0)),
                    );
                    window.paint_quad(fill(v_stroke, gpui::white()));
                }
            }

            // Split paragraph text at '\n' to get visual sub-lines, then
            // word-wrap each sub-line to fit within block_w.
            let flat = para.plain_text();
            let sublines: Vec<&str> = flat.split('\n').collect();
            let mut char_start = 0usize;
            let mut para_h = 0.0f32;

            for subline in &sublines {
                let char_count = subline.chars().count();
                let char_end = char_start + char_count;
                let line_y_screen = bounds.top() + px(current_y);

                if subline.is_empty() {
                    // Empty line — cursor sits at x=0 relative to margin.
                    visual_lines.push(VisualLine {
                        para_idx,
                        char_start,
                        char_end,
                        top_px: current_y,
                        height_px: line_height,
                        font_size_px,
                        is_mermaid: false,
                        glyph_xs: vec![px(0.0)],
                    });
                    line_layouts.push(None);
                    current_y += line_height;
                    para_h += line_height;
                } else {
                    let (line_text, runs) = spans_to_text_runs(
                        &para.spans,
                        char_start,
                        char_end,
                        &base_font,
                        para_color,
                        emphasis_color,
                        code_color,
                        link_color,
                        &mono_family,
                    );
                    let total_run_bytes: usize = runs.iter().map(|r| r.len).sum();

                    if !line_text.is_empty() && total_run_bytes > 0 {
                        // Use shape_text with word-wrap so long lines break visually.
                        let wrap_result = window.text_system().shape_text(
                            line_text.clone().into(),
                            font_size,
                            &runs,
                            Some(block_w),
                            None,
                        );

                        let wrapped = wrap_result.ok().and_then(|mut v| v.drain(..).next());

                        if let Some(wl) = wrapped {
                            // Paint the full wrapped block in one call.
                            let _ = wl.paint(
                                point(left, line_y_screen),
                                lh_px,
                                gpui::TextAlign::Left,
                                None,
                                window,
                                cx,
                            );

                            // Split into per-visual-row VisualLine entries using wrap
                            // boundaries from the layout so cursor and selection work.
                            let unwrapped = &wl.unwrapped_layout;

                            // Collect byte offsets where each row ends.
                            let mut row_end_bytes: Vec<usize> = wl
                                .wrap_boundaries
                                .iter()
                                .map(|wb| {
                                    unwrapped
                                        .runs
                                        .get(wb.run_ix)
                                        .and_then(|r| r.glyphs.get(wb.glyph_ix))
                                        .map(|g| g.index)
                                        .unwrap_or(line_text.len())
                                })
                                .collect();
                            row_end_bytes.push(line_text.len());

                            let num_rows = row_end_bytes.len();
                            let mut row_start_byte = 0usize;
                            let mut row_char_start = char_start;

                            for (row_idx, &row_end_byte) in row_end_bytes.iter().enumerate() {
                                let row_text = &line_text[row_start_byte..row_end_byte];
                                let row_char_count = row_text.chars().count();
                                let row_char_end = row_char_start + row_char_count;

                                // x base in the unwrapped layout for this visual row.
                                let row_base_x = if row_start_byte == 0 {
                                    px(0.0)
                                } else {
                                    unwrapped.x_for_index(row_start_byte)
                                };

                                // glyph_xs: char-indexed positions relative to row start.
                                let gc = row_char_end - row_char_start;
                                let row_glyph_xs: Vec<Pixels> = (0..=gc)
                                    .map(|ci| {
                                        let byte =
                                            row_start_byte + chars_to_bytes(row_text, ci);
                                        unwrapped.x_for_index(byte) - row_base_x
                                    })
                                    .collect();

                                visual_lines.push(VisualLine {
                                    para_idx,
                                    char_start: row_char_start,
                                    char_end: row_char_end,
                                    top_px: current_y + row_idx as f32 * line_height,
                                    height_px: line_height,
                                    font_size_px,
                                    is_mermaid: false,
                                    glyph_xs: row_glyph_xs,
                                });
                                line_layouts.push(None);

                                row_start_byte = row_end_byte;
                                row_char_start = row_char_end;
                            }

                            let total_h = num_rows as f32 * line_height;
                            current_y += total_h;
                            para_h += total_h;
                        } else {
                            // shape_text failed — fallback: render as single unshaped line.
                            visual_lines.push(VisualLine {
                                para_idx,
                                char_start,
                                char_end,
                                top_px: current_y,
                                height_px: line_height,
                                font_size_px,
                                is_mermaid: false,
                                glyph_xs: vec![px(0.0)],
                            });
                            line_layouts.push(None);
                            current_y += line_height;
                            para_h += line_height;
                        }
                    } else {
                        // Zero-length or zero-run line.
                        visual_lines.push(VisualLine {
                            para_idx,
                            char_start,
                            char_end,
                            top_px: current_y,
                            height_px: line_height,
                            font_size_px,
                            is_mermaid: false,
                            glyph_xs: vec![px(0.0)],
                        });
                        line_layouts.push(None);
                        current_y += line_height;
                        para_h += line_height;
                    }
                }

                char_start = char_end + 1;
            }

            para_visual_heights.push(para_h);
            current_y += PARA_GAP;
        }

        // Store layout cache and per-paragraph visual heights in state.
        self.state.update(cx, |s, _| {
            s.layout_cache = visual_lines.clone();
            s.para_visual_heights = para_visual_heights;
        });

        // ── Selection highlight ──────────────────────────────────────────────
        if let Some(sel) = selection {
            let (sel_start, sel_end) = sel.ordered();
            for (_, vl) in visual_lines.iter().enumerate() {
                if vl.is_mermaid { continue; }
                let vl_para = vl.para_idx;
                let vl_cs = vl.char_start;
                let vl_ce = vl.char_end;

                let sel_s_in_vl = if sel_start.para_idx == vl_para {
                    if sel_start.char_offset > vl_ce { continue; }
                    sel_start.char_offset.max(vl_cs)
                } else if sel_start.para_idx < vl_para {
                    vl_cs
                } else {
                    continue;
                };

                let sel_e_in_vl = if sel_end.para_idx == vl_para {
                    if sel_end.char_offset < vl_cs { continue; }
                    sel_end.char_offset.min(vl_ce)
                } else if sel_end.para_idx > vl_para {
                    vl_ce
                } else {
                    continue;
                };

                let line_y_screen = bounds.top() + px(vl.top_px);
                let (sel_x_start, sel_width) = {
                    let col_s = sel_s_in_vl - vl_cs;
                    let col_e = sel_e_in_vl - vl_cs;
                    let x_s = vl.glyph_xs.get(col_s).copied().unwrap_or(px(0.0));
                    let x_e = vl.glyph_xs.get(col_e).copied().unwrap_or(x_s);
                    (left + x_s, x_e - x_s)
                };

                if sel_width > px(0.0) {
                    window.paint_quad(fill(
                        Bounds::new(
                            point(sel_x_start, line_y_screen),
                            size(sel_width, px(vl.height_px)),
                        ),
                        selection_color,
                    ));
                }
            }
        }

        // ── Cursor caret ─────────────────────────────────────────────────────
        if focus_handle.is_focused(window) {
            let cur_vl = visual_lines.iter().enumerate().find(|(_, vl)| {
                vl.para_idx == cursor.para_idx
                    && cursor.char_offset >= vl.char_start
                    && cursor.char_offset <= vl.char_end
            });

            if let Some((_, vl)) = cur_vl {
                // Tables draw their own cell caret in the table paint block above.
                if !matches!(&paragraphs[vl.para_idx].kind, ParagraphKind::Table { .. }) {
                let col = cursor.char_offset - vl.char_start;
                let cursor_x = {
                    vl.glyph_xs.get(col).copied()
                        .map(|x| left + x)
                        .unwrap_or(left)
                };
                let cursor_y_screen = bounds.top() + px(vl.top_px);

                window.paint_quad(fill(
                    Bounds::new(
                        point(cursor_x, cursor_y_screen),
                        size(px(2.0), px(vl.height_px)),
                    ),
                    caret_color,
                ));
                } // end table guard
            }
        }
    }
}


// ─────────────────────────────────────────────────────────────────────────────
// DocumentEditor — public RenderOnce wrapper
// ─────────────────────────────────────────────────────────────────────────────

/// The public component. Renders the full document as a scrollable rich editor.
#[derive(IntoElement)]
pub struct DocumentEditor {
    state: Entity<DocumentEditorState>,
    show_border: bool,
}

impl DocumentEditor {
    pub fn new(state: &Entity<DocumentEditorState>) -> Self {
        Self { state: state.clone(), show_border: false }
    }

    pub fn show_border(mut self, show: bool) -> Self {
        self.show_border = show;
        self
    }
}

impl RenderOnce for DocumentEditor {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = use_theme();
        let focus_handle = self.state.read(cx).focus_handle.clone();

        let state = self.state.clone();
        let state_move = self.state.clone();
        let state_up = self.state.clone();

        let mut base = div()
            .id(("document-editor", self.state.entity_id()))
            .key_context("DocumentEditor")
            .track_focus(&focus_handle)
            .size_full()
            .bg(theme.tokens.background)
            .cursor_text()
            .on_action(window.listener_for(&self.state, DocumentEditorState::move_left))
            .on_action(window.listener_for(&self.state, DocumentEditorState::move_right))
            .on_action(window.listener_for(&self.state, DocumentEditorState::move_up))
            .on_action(window.listener_for(&self.state, DocumentEditorState::move_down))
            .on_action(window.listener_for(&self.state, DocumentEditorState::move_to_line_start))
            .on_action(window.listener_for(&self.state, DocumentEditorState::move_to_line_end))
            .on_action(window.listener_for(&self.state, DocumentEditorState::move_to_doc_start))
            .on_action(window.listener_for(&self.state, DocumentEditorState::move_to_doc_end))
            .on_action(window.listener_for(&self.state, DocumentEditorState::select_left))
            .on_action(window.listener_for(&self.state, DocumentEditorState::select_right))
            .on_action(window.listener_for(&self.state, DocumentEditorState::select_up))
            .on_action(window.listener_for(&self.state, DocumentEditorState::select_down))
            .on_action(window.listener_for(&self.state, DocumentEditorState::select_to_line_start))
            .on_action(window.listener_for(&self.state, DocumentEditorState::select_to_line_end))
            .on_action(window.listener_for(&self.state, DocumentEditorState::select_all))
            .on_action(window.listener_for(&self.state, DocumentEditorState::backspace))
            .on_action(window.listener_for(&self.state, DocumentEditorState::delete))
            .on_action(window.listener_for(&self.state, DocumentEditorState::enter))
            .on_action(window.listener_for(&self.state, DocumentEditorState::tab))
            .on_action(window.listener_for(&self.state, DocumentEditorState::copy))
            .on_action(window.listener_for(&self.state, DocumentEditorState::cut))
            .on_action(window.listener_for(&self.state, DocumentEditorState::paste))
            .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, window: &mut Window, cx: &mut App| {
                let bounds = state.read(cx).last_bounds.unwrap_or_default();
                let new_cursor = state.read(cx).cursor_from_point(event.position, bounds);
                let new_table_cell = state.read(cx).table_cell_from_point(event.position, bounds);
                let focus = state.read(cx).focus_handle.clone();

                // Detect click on task list item checkbox (drawn in the left margin).
                // Checkbox area: [left - 28, left - 8] x-range.
                let cb_left = bounds.left() + px(LEFT_MARGIN - 28.0);
                let cb_right = bounds.left() + px(LEFT_MARGIN - 8.0);
                let in_checkbox_zone = event.position.x >= cb_left && event.position.x <= cb_right;
                if in_checkbox_zone {
                    let para_idx = new_cursor.para_idx;
                    let is_task = state.read(cx).paragraphs.get(para_idx)
                        .map(|p| matches!(p.kind, ParagraphKind::TaskListItem { .. }))
                        .unwrap_or(false);
                    if is_task {
                        state.update(cx, |s, cx| {
                            s.toggle_checked_at_para(para_idx, cx);
                        });
                        window.focus(&focus);
                        return;
                    }
                }

                // Detect click on the image resize handle (4px bar at image block bottom).
                let resize_info: Option<(usize, f32, f32)> = {
                    let s = state.read(cx);
                    let mut found = None;
                    for vl in &s.layout_cache {
                        let Some(para) = s.paragraphs.get(vl.para_idx) else { continue; };
                        if let ParagraphKind::Image { height, .. } = &para.kind {
                            let block_bottom = bounds.top() + px(vl.top_px + vl.height_px);
                            if event.position.y >= block_bottom - px(12.0) && event.position.y <= block_bottom + px(4.0) {
                                found = Some((vl.para_idx, *height, event.position.y.into()));
                                break;
                            }
                        }
                    }
                    found
                };
                if let Some((para_idx, orig_h, start_y)) = resize_info {
                    state.update(cx, |s, cx| {
                        s.image_resize_drag = Some((para_idx, orig_h, start_y));
                        cx.notify();
                    });
                    window.focus(&focus);
                    return;
                }

                // Detect single click on the magnifier badge (top-right corner of image or mermaid).
                {
                    let s = state.read(cx);
                    let block_w = bounds.size.width - px(LEFT_MARGIN * 2.0);
                    let left = bounds.left() + px(LEFT_MARGIN);
                    let badge_size = 32.0_f32;
                    for vl in &s.layout_cache {
                        let Some(para) = s.paragraphs.get(vl.para_idx) else { continue; };
                        let top_screen = bounds.top() + px(vl.top_px);
                        let badge_x = left + block_w - px(badge_size + 8.0);
                        let badge_y = top_screen + px(8.0);
                        let in_badge = event.position.x >= badge_x
                            && event.position.x <= badge_x + px(badge_size)
                            && event.position.y >= badge_y
                            && event.position.y <= badge_y + px(badge_size);
                        if !in_badge { continue; }
                        match &para.kind {
                            ParagraphKind::Image { path, .. } => {
                                if let Some(ref dir) = s.document_dir {
                                    let img_path = dir.join(path.as_str());
                                    let _ = s;
                                    state.update(cx, |st, cx| {
                                        st.image_zoom = Some(img_path);
                                        cx.notify();
                                    });
                                    window.focus(&focus);
                                    return;
                                }
                            }
                            ParagraphKind::Mermaid(_) => {
                                let source = para.plain_text();
                                let key = mermaid_source_hash(&source);
                                let png_path = std::env::temp_dir()
                                    .join("octodocs-mermaid-cache")
                                    .join(format!("{key}.png"));
                                if png_path.exists() {
                                    let _ = s;
                                    state.update(cx, |st, cx| {
                                        st.image_zoom = Some(png_path);
                                        cx.notify();
                                    });
                                    window.focus(&focus);
                                    return;
                                }
                            }
                            _ => {}
                        }
                        break;
                    }
                }

                // Detect double-click on an image or mermaid block → show full-size zoom overlay.
                if event.click_count >= 2 {
                    let zoom_path: Option<PathBuf> = {
                        let s = state.read(cx);
                        let mut found = None;
                        for vl in &s.layout_cache {
                            let Some(para) = s.paragraphs.get(vl.para_idx) else { continue; };
                            let block_top = bounds.top() + px(vl.top_px);
                            let block_bottom = block_top + px(vl.height_px);
                            if event.position.y < block_top || event.position.y > block_bottom {
                                continue;
                            }
                            match &para.kind {
                                ParagraphKind::Image { path, .. } => {
                                    if let Some(ref dir) = s.document_dir {
                                        found = Some(dir.join(path));
                                    }
                                }
                                ParagraphKind::Mermaid(_) => {
                                    let source = para.plain_text();
                                    let key = mermaid_source_hash(&source);
                                    let png_path = std::env::temp_dir()
                                        .join("octodocs-mermaid-cache")
                                        .join(format!("{key}.png"));
                                    if png_path.exists() {
                                        found = Some(png_path);
                                    }
                                }
                                _ => {}
                            }
                            break;
                        }
                        found
                    };
                    if let Some(img_path) = zoom_path {
                        state.update(cx, |s, cx| {
                            s.image_zoom = Some(img_path);
                            cx.notify();
                        });
                        window.focus(&focus);
                        return;
                    }
                }

                state.update(cx, |s, cx| {
                    s.cursor = new_cursor;
                    s.table_cursor = new_table_cell;
                    // Do not clear selection here. A plain click will clear it on mouse up;
                    // a drag will set drag_occurred=true and keep the selection.
                    s.drag_anchor = Some(new_cursor);
                    s.drag_occurred = false;
                    cx.notify();
                });
                window.focus(&focus);
            })
            .on_mouse_move(move |event: &MouseMoveEvent, _window: &mut Window, cx: &mut App| {
                // Guard on drag_anchor rather than event.pressed_button: pressed_button
                // detection can be unreliable on some Linux compositors.
                // drag_anchor is only Some when on_mouse_down already fired, so this
                // is definitively a left-button drag gesture.

                // Handle image resize drag.
                let resize_drag = state_move.read(cx).image_resize_drag;
                if let Some((para_idx, orig_h, start_y)) = resize_drag {
                    let delta: f32 = f32::from(event.position.y) - start_y;
                    let new_h = (orig_h + delta).max(60.0);
                    state_move.update(cx, |s, cx| {
                        if let Some(para) = s.paragraphs.get_mut(para_idx) {
                            if let ParagraphKind::Image { ref mut height, .. } = para.kind {
                                *height = new_h;
                            }
                        }
                        cx.notify();
                    });
                    return;
                }

                // Track link hover for pointer cursor.
                {
                    let new_hovered_link = {
                        let s = state_move.read(cx);
                        if !s.layout_cache.is_empty() {
                            let b = s.last_bounds.unwrap_or_default();
                            let hover_cursor = s.cursor_from_point(event.position, b);
                            s.link_url_at_offset(hover_cursor.para_idx, hover_cursor.char_offset).is_some()
                        } else {
                            false
                        }
                    };
                    let prev_link = state_move.read(cx).hovered_link;
                    if prev_link != new_hovered_link {
                        state_move.update(cx, |s, cx| {
                            s.hovered_link = new_hovered_link;
                            cx.notify();
                        });
                    }
                }

                // Track which image/mermaid block (if any) the cursor is hovering over.
                {
                    let (new_image_hovered, new_mermaid_hovered) = {
                        let s = state_move.read(cx);
                        let b = s.last_bounds.unwrap_or_default();
                        let mut img_found = None;
                        let mut mer_found = None;
                        for vl in &s.layout_cache {
                            let top = b.top() + px(vl.top_px);
                            let bot = top + px(vl.height_px);
                            if event.position.y < top || event.position.y > bot { continue; }
                            if let Some(para) = s.paragraphs.get(vl.para_idx) {
                                match &para.kind {
                                    ParagraphKind::Image { .. } => { img_found = Some(vl.para_idx); break; }
                                    ParagraphKind::Mermaid(_) => { mer_found = Some(vl.para_idx); break; }
                                    _ => {}
                                }
                            }
                        }
                        (img_found, mer_found)
                    };
                    let (prev_img, prev_mer) = {
                        let s = state_move.read(cx);
                        (s.hovered_image_para, s.hovered_mermaid_para)
                    };
                    if prev_img != new_image_hovered || prev_mer != new_mermaid_hovered {
                        state_move.update(cx, |s, cx| {
                            s.hovered_image_para = new_image_hovered;
                            s.hovered_mermaid_para = new_mermaid_hovered;
                            cx.notify();
                        });
                        // Only short-circuit when not dragging text; during a drag,
                        // we still need to update the selection in this same event.
                        let drag_anchor = state_move.read(cx).drag_anchor;
                        if drag_anchor.is_none() {
                            return;
                        }
                    }
                }

                let (bounds, drag_anchor) = {
                    let s = state_move.read(cx);
                    (s.last_bounds.unwrap_or_default(), s.drag_anchor)
                };
                if let Some(anchor) = drag_anchor {
                    let focus_cursor = state_move.read(cx).cursor_from_point(event.position, bounds);
                    let focus_table_cell = state_move.read(cx).table_cell_from_point(event.position, bounds);
                    state_move.update(cx, |s, cx| {
                        s.drag_occurred = true;
                        s.cursor = focus_cursor;
                        if focus_table_cell.is_some() {
                            s.table_cursor = focus_table_cell;
                        }
                        s.selection = if anchor == focus_cursor {
                            None
                        } else {
                            Some(DocSelection { anchor, focus: focus_cursor })
                        };
                        cx.notify();
                    });
                }
            })
            .on_mouse_up(MouseButton::Left, move |event: &MouseUpEvent, _window: &mut Window, cx: &mut App| {
                let _ = event;
                state_up.update(cx, |s, cx| {
                    // Clear image resize drag on mouse up.
                    if s.image_resize_drag.take().is_some() {
                        cx.notify();
                        return;
                    }
                    // Plain click (no drag): open link if the cursor landed on a Link span.
                    if !s.drag_occurred {
                        if let Some((tr, tc)) = s.table_cursor {
                            let para_idx = s.cursor.para_idx;
                            if matches!(&s.paragraphs[para_idx].kind, ParagraphKind::Table { .. }) {
                                let cell = s.table_cell_text(para_idx, tr, tc);
                                if let Some((_label, url)) = parse_markdown_link_exact(&cell) {
                                    if url.starts_with("http://") || url.starts_with("https://") {
                                        open_url(&url);
                                    } else if let Some(dir) = s.document_dir.as_ref() {
                                        let resolved = dir.join(&url).to_string_lossy().into_owned();
                                        s.navigate_request = Some(resolved);
                                    } else if std::path::Path::new(&url).is_absolute() {
                                        s.navigate_request = Some(url);
                                    }
                                }
                            }
                        }
                        if let Some(url) = s.link_url_at_offset(s.cursor.para_idx, s.cursor.char_offset) {
                            if url.starts_with("http://") || url.starts_with("https://") {
                                open_url(&url);
                            } else {
                                // Local .md link — ask the host app to navigate.
                                // Only resolve when a base directory is known; otherwise
                                // require an absolute path to avoid CWD-relative surprises.
                                if let Some(dir) = s.document_dir.as_ref() {
                                    let resolved = dir.join(&url).to_string_lossy().into_owned();
                                    s.navigate_request = Some(resolved);
                                } else if std::path::Path::new(&url).is_absolute() {
                                    s.navigate_request = Some(url);
                                }
                            }
                        }
                    }
                    // Plain click clears selection; completed drag preserves it.
                    if !s.drag_occurred {
                        s.selection = None;
                    }
                    s.drag_anchor = None;
                    s.drag_occurred = false;
                    cx.notify();
                });
            });

        // ── File drag-and-drop ─────────────────────────────────────────────
        let state_drop = self.state.clone();
        base = base.on_drop::<ExternalPaths>(move |paths: &ExternalPaths, _window: &mut Window, cx: &mut App| {
            const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg"];
            let doc_dir_opt = state_drop.read(cx).document_dir.clone();
            for src in paths.paths() {
                let ext_lc = src.extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.to_lowercase())
                    .unwrap_or_default();
                if !IMAGE_EXTS.contains(&ext_lc.as_str()) {
                    continue;
                }
                let images_dir = match &doc_dir_opt {
                    Some(d) => d.join("images"),
                    None => std::env::temp_dir().join("octodocs").join("images"),
                };
                if std::fs::create_dir_all(&images_dir).is_err() {
                    continue;
                }
                let Some(filename) = src.file_name() else { continue; };
                let safe_name = filename.to_string_lossy().replace(' ', "_");
                let dest = images_dir.join(&safe_name);
                if std::fs::copy(src, &dest).is_err() {
                    continue;
                }
                let rel = format!("images/{}", safe_name);
                let alt = src.file_stem()
                    .map(|s| s.to_string_lossy().replace(' ', "_"))
                    .unwrap_or_default();
                let _ = state_drop.update(cx, |s, cx| s.insert_image_at_cursor(rel, alt, cx));
            }
        });

        if self.show_border {
            base = base.border_1().border_color(theme.tokens.border).rounded(theme.tokens.radius_md);
        }

        base.child(DocumentEditorElement { state: self.state.clone() })
    }
}
