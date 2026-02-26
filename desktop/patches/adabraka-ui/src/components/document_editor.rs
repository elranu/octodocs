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
        }
    }

    /// Replace the document content. Cursor resets to the beginning.
    pub fn load_document(&mut self, paragraphs: Vec<DocParagraph>, cx: &mut Context<Self>) {
        self.paragraphs = if paragraphs.is_empty() {
            vec![DocParagraph::empty()]
        } else {
            paragraphs
        };
        self.cursor = DocCursor::zero();
        self.selection = None;
        self.marked_range = None;
        self.layout_cache.clear();
        self.drag_anchor = None;
        self.drag_occurred = false;
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

    fn clamped_cursor(&self, c: DocCursor) -> DocCursor {
        let para_idx = c.para_idx.min(self.num_paras().saturating_sub(1));
        let char_offset = c.char_offset.min(self.para_char_count(para_idx));
        DocCursor { para_idx, char_offset }
    }

    /// Flat char offset of a DocCursor within the whole document
    /// (paragraphs separated by one `\n`).
    fn cursor_to_flat_offset(&self, c: DocCursor) -> usize {
        let mut off = 0;
        for i in 0..c.para_idx {
            off += self.para_char_count(i) + 1; // +1 for the '\n' separator
        }
        off + c.char_offset
    }

    /// Convert a document-flat char offset back to `DocCursor`.
    fn flat_offset_to_cursor(&self, mut off: usize) -> DocCursor {
        for (i, para) in self.paragraphs.iter().enumerate() {
            let len = para.char_count();
            if off <= len {
                return DocCursor { para_idx: i, char_offset: off };
            }
            off -= len + 1;
        }
        let last = self.num_paras() - 1;
        DocCursor { para_idx: last, char_offset: self.para_char_count(last) }
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

    /// Return the plain-text char at the given position (or None at boundaries).
    fn char_before_cursor(&self, c: DocCursor) -> Option<char> {
        if c.char_offset == 0 { return None; }
        self.paragraphs[c.para_idx]
            .plain_text()
            .chars()
            .nth(c.char_offset - 1)
    }

    fn char_after_cursor(&self, c: DocCursor) -> Option<char> {
        self.paragraphs[c.para_idx]
            .plain_text()
            .chars()
            .nth(c.char_offset)
    }

    /// Rebuild paragraph spans from a new flat text, trying to preserve
    /// existing span formatting wherever possible. For simplicity this
    /// implementation writes all text as plain spans; a smarter approach would
    /// splice only the changed range. For the MVP this is sufficient.
    fn rebuild_spans_from_text(para: &mut DocParagraph, new_text: String) {
        // Preserve the first span's format as the "current format"
        let fmt = para.spans.first().map(|s| s.format).unwrap_or(InlineFormat::Plain);
        para.spans = vec![InlineSpan { text: new_text, format: fmt }];
    }

    /// Higher-level helper: insert `text` at `DocCursor`, return new cursor.
    fn do_insert(&mut self, at: DocCursor, text: &str) -> DocCursor {
        let para = &mut self.paragraphs[at.para_idx];
        let flat = para.plain_text();
        let byte_pos: usize = flat.char_indices().nth(at.char_offset).map(|(b, _)| b).unwrap_or(flat.len());
        let mut new_flat = flat.clone();
        new_flat.insert_str(byte_pos, text);
        let new_char_offset = at.char_offset + text.chars().count();
        Self::rebuild_spans_from_text(para, new_flat);
        DocCursor { para_idx: at.para_idx, char_offset: new_char_offset }
    }

    /// Remove `count` chars starting at `at`, return new cursor.
    fn do_delete_chars(&mut self, at: DocCursor, count: usize) -> DocCursor {
        let para = &mut self.paragraphs[at.para_idx];
        let flat = para.plain_text();
        let byte_start = flat.char_indices().nth(at.char_offset).map(|(b, _)| b).unwrap_or(flat.len());
        let end_offset = (at.char_offset + count).min(flat.chars().count());
        let byte_end = flat.char_indices().nth(end_offset).map(|(b, _)| b).unwrap_or(flat.len());
        let mut new_flat = flat.clone();
        new_flat.replace_range(byte_start..byte_end, "");
        Self::rebuild_spans_from_text(para, new_flat);
        at
    }

    /// Get selected text as a String (or empty if no selection).
    fn selected_text(&self) -> String {
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

    /// Delete the current selection, return new cursor position (at selection start).
    fn delete_selection(&mut self, cx: &mut Context<Self>) -> DocCursor {
        let Some(sel) = self.selection.take() else { return self.cursor; };
        let (start, end) = sel.ordered();

        if start.para_idx == end.para_idx {
            let count = end.char_offset - start.char_offset;
            return self.do_delete_chars(start, count);
        }

        // Multi-paragraph selection: collapse by joining edges
        let tail: String = {
            let flat = self.paragraphs[end.para_idx].plain_text();
            flat.chars().skip(end.char_offset).collect()
        };
        // Truncate the start paragraph at the selection start
        {
            let flat = self.paragraphs[start.para_idx].plain_text();
            let new_text: String = flat.chars().take(start.char_offset).chain(tail.chars()).collect();
            Self::rebuild_spans_from_text(&mut self.paragraphs[start.para_idx], new_text);
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
        if let Some(_) = self.selection {
            let at = self.delete_selection(cx);
            self.cursor = at;
        }

        if text == "\n" {
            // Enter: split paragraph at cursor
            let cur_para_idx = self.cursor.para_idx;
            let split_at = self.cursor.char_offset;
            let flat = self.paragraphs[cur_para_idx].plain_text();
            let (head, tail): (String, String) = {
                let mut h = String::new();
                let mut t = String::new();
                for (i, ch) in flat.chars().enumerate() {
                    if i < split_at { h.push(ch); } else { t.push(ch); }
                }
                (h, t)
            };
            let fmt = self.paragraphs[cur_para_idx].spans.first()
                .map(|s| s.format).unwrap_or(InlineFormat::Plain);
            Self::rebuild_spans_from_text(&mut self.paragraphs[cur_para_idx], head);
            let new_para = DocParagraph {
                kind: ParagraphKind::Paragraph,
                spans: vec![InlineSpan { text: tail, format: fmt }],
            };
            self.paragraphs.insert(cur_para_idx + 1, new_para);
            self.cursor = DocCursor { para_idx: cur_para_idx + 1, char_offset: 0 };
        } else {
            self.cursor = self.do_insert(self.cursor, text);
        }
        cx.notify();
    }

    pub fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
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
            let current_flat = self.paragraphs[self.cursor.para_idx].plain_text();
            let prev_flat = self.paragraphs[prev_idx].plain_text();
            let merged = format!("{}{}", prev_flat, current_flat);
            Self::rebuild_spans_from_text(&mut self.paragraphs[prev_idx], merged);
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
            let next_flat = self.paragraphs[self.cursor.para_idx + 1].plain_text();
            let cur_flat = self.paragraphs[self.cursor.para_idx].plain_text();
            let merged = format!("{}{}", cur_flat, next_flat);
            Self::rebuild_spans_from_text(&mut self.paragraphs[self.cursor.para_idx], merged);
            self.paragraphs.remove(self.cursor.para_idx + 1);
        }
        cx.notify();
    }

    pub fn enter(&mut self, _: &Enter, _: &mut Window, cx: &mut Context<Self>) {
        self.insert_text("\n", cx);
    }

    pub fn tab(&mut self, _: &Tab, _: &mut Window, cx: &mut Context<Self>) {
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

    /// Toggle bold on the current selection (or the current paragraph if no selection).
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

        // Rebuild spans: split at start/end boundaries and toggle format in range
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
                        new_spans.push(InlineSpan { text, format: span.format });
                    }
                }
            }

            // Part within the selection
            let overlap_start = span_start.max(sel_start);
            let overlap_end = span_end.min(sel_end);
            if overlap_start < overlap_end {
                let text: String = flat.chars().skip(overlap_start).take(overlap_end - overlap_start).collect();
                if !text.is_empty() {
                    let new_fmt = if span.format == fmt { InlineFormat::Plain } else { fmt };
                    new_spans.push(InlineSpan { text, format: new_fmt });
                }
            }

            // Part after the selection
            if span_end > sel_end && span_start < span_end {
                let start_clip = sel_end.max(span_start);
                if start_clip < span_end {
                    let text: String = flat.chars().skip(start_clip).take(span_end - start_clip).collect();
                    if !text.is_empty() {
                        new_spans.push(InlineSpan { text, format: span.format });
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

    // ── Mouse ─────────────────────────────────────────────────────────────────

    /// Map a screen-space click position to the nearest document cursor position.
    /// Uses the per-line `glyph_xs` cache populated each paint pass for accurate
    /// sub-character hit testing without needing live `ShapedLine` objects.
    pub fn cursor_from_point(&self, point: Point<Pixels>, bounds: Bounds<Pixels>) -> DocCursor {
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
        let last = self.num_paras() - 1;
        DocCursor { para_idx: last, char_offset: self.para_char_count(last) }
    }

    // ── IME helpers ───────────────────────────────────────────────────────────

    /// Flat document content for IME (within the current paragraph).
    fn current_para_content(&self) -> String {
        self.paragraphs[self.cursor.para_idx].plain_text()
    }

    fn char_offset_to_utf16_in_para(&self, char_off: usize) -> usize {
        let text = self.current_para_content();
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
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.char_offset_to_utf16_in_para(self.cursor.char_offset))
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
const MERMAID_HEIGHT: f32 = 180.0;

fn para_font_size(kind: &ParagraphKind) -> f32 {
    match kind {
        ParagraphKind::Heading(1) => 26.0,
        ParagraphKind::Heading(2) => 21.0,
        ParagraphKind::Heading(3) => 18.0,
        ParagraphKind::CodeFence(_) => 13.0,
        _ => 15.0,
    }
}

fn para_line_height(kind: &ParagraphKind) -> f32 {
    match kind {
        ParagraphKind::Heading(1) => 38.0,
        ParagraphKind::Heading(2) => 32.0,
        ParagraphKind::Heading(3) => 28.0,
        ParagraphKind::CodeFence(_) => 20.0,
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
            if last.format == span.format {
                last.text.push_str(&span.text);
                continue;
            }
        }
        merged.push(span);
    }

    if merged.is_empty() {
        vec![InlineSpan { text: String::new(), format: InlineFormat::Plain }]
    } else {
        merged
    }
}

fn bytes_to_chars(text: &str, byte_offset: usize) -> usize {
    let safe_offset = byte_offset.min(text.len());
    text[..safe_offset].chars().count()
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
            // Use oblique as a robust fallback when no dedicated italic face
            // is available for the active family on Linux.
            style: FontStyle::Oblique,
            ..base_font.clone()
        },
        InlineFormat::Underline | InlineFormat::Strikethrough => base_font.clone(),
        InlineFormat::Code => Font {
            family: mono_font_family.clone(),
            weight: FontWeight::NORMAL,
            style: FontStyle::Normal,
            ..base_font.clone()
        },
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
            InlineFormat::Code => hsla(0.55, 0.7, 0.5, 1.0),
            InlineFormat::Bold | InlineFormat::Italic | InlineFormat::Underline | InlineFormat::Strikethrough => emphasis_color,
            InlineFormat::Plain => base_color,
        };

        let underline = match span.format {
            InlineFormat::Italic => Some(UnderlineStyle {
                thickness: px(1.0),
                color: Some(emphasis_color),
                wavy: true,
            }),
            InlineFormat::Underline => Some(UnderlineStyle {
                thickness: px(1.0),
                color: Some(emphasis_color),
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

struct EditorPrepaintState {
    /// Total content height (for request_layout sizing).
    content_height: Pixels,
}

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

        for para in &state.paragraphs {
            let lh = para_line_height(&para.kind);
            match &para.kind {
                ParagraphKind::Mermaid(_) => {
                    total_h += MERMAID_HEIGHT + PARA_GAP;
                }
                _ => {
                    // Count visual lines by counting '\n's in plain text
                    let text = para.plain_text();
                    let line_count = (text.chars().filter(|&c| c == '\n').count() + 1).max(1);
                    total_h += lh * line_count as f32 + PARA_GAP;
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
        for para in &state.paragraphs {
            let lh = para_line_height(&para.kind);
            match &para.kind {
                ParagraphKind::Mermaid(_) => total_h += MERMAID_HEIGHT + PARA_GAP,
                _ => {
                    let text = para.plain_text();
                    let line_count = (text.chars().filter(|&c| c == '\n').count() + 1).max(1);
                    total_h += lh * line_count as f32 + PARA_GAP;
                }
            }
        }

        EditorPrepaintState { content_height: px(total_h) }
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

        let (paragraphs, cursor, selection) = {
            let s = self.state.read(cx);
            (s.paragraphs.clone(), s.cursor, s.selection)
        };

        let mut current_y: f32 = TOP_PADDING;
        let left: Pixels = bounds.left() + px(LEFT_MARGIN);
        let mut visual_lines: Vec<VisualLine> = Vec::new();
        let mut line_layouts: Vec<Option<ShapedLine>> = Vec::new();

        for (para_idx, para) in paragraphs.iter().enumerate() {
            let kind = &para.kind;
            let font_size_px = para_font_size(kind);
            let line_height = para_line_height(kind);
            let font_size = px(font_size_px);
            let lh_px = px(line_height);

            let para_color = match kind {
                ParagraphKind::Heading(_) => theme.tokens.foreground,
                ParagraphKind::BlockQuote => theme.tokens.muted_foreground,
                _ => theme.tokens.foreground,
            };

            match kind {
                ParagraphKind::Mermaid(_) => {
                    // Paint a placeholder rectangle for mermaid blocks
                    let top_screen = bounds.top() + px(current_y);
                    let mermaid_bounds = Bounds::new(
                        point(left, top_screen),
                        size(bounds.size.width - px(LEFT_MARGIN * 2.0), px(MERMAID_HEIGHT)),
                    );
                    window.paint_quad(fill(mermaid_bounds, theme.tokens.muted.opacity(0.3)));
                    // Label
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
                        point(left, top_screen + px(MERMAID_HEIGHT / 2.0 - 8.0)),
                        px(20.0),
                        window,
                        cx,
                    );
                    visual_lines.push(VisualLine {
                        para_idx,
                        char_start: 0,
                        char_end: 0,
                        top_px: current_y,
                        height_px: MERMAID_HEIGHT,
                        font_size_px,
                        is_mermaid: true,
                        glyph_xs: vec![],
                    });
                    line_layouts.push(None);
                    current_y += MERMAID_HEIGHT + PARA_GAP;
                    continue;
                }
                _ => {}
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

            // Split paragraph text at '\n' to get visual sub-lines
            let flat = para.plain_text();
            let sublines: Vec<&str> = flat.split('\n').collect();
            let mut char_start = 0usize;

            for subline in &sublines {
                let char_count = subline.chars().count();
                let char_end = char_start + char_count;
                let line_y_screen = bounds.top() + px(current_y);

                // Shape the line and compute per-character x-positions for
                // accurate mouse hit-testing (stored in VisualLine::glyph_xs).
                let glyph_xs: Vec<Pixels>;
                let shaped_line: Option<ShapedLine>;

                if subline.is_empty() {
                    // Empty line — cursor sits at x=0 relative to margin.
                    glyph_xs = vec![px(0.0)];
                    shaped_line = None;
                } else {
                    let (line_text, runs) = spans_to_text_runs(
                        &para.spans,
                        char_start,
                        char_end,
                        &base_font,
                        para_color,
                        emphasis_color,
                        &mono_family,
                    );
                    let total_run_bytes: usize = runs.iter().map(|r| r.len).sum();
                    if !line_text.is_empty() && total_run_bytes > 0 {
                        let shaped = window.text_system().shape_line(
                            line_text.clone().into(),
                            font_size,
                            &runs,
                            None,
                        );
                        // glyph_xs[i] = left-edge x of char i (relative to left margin).
                        // glyph_xs[char_count] = right-edge of last char (cursor-after-end).
                        let char_count = char_end - char_start;
                        glyph_xs = (0..=char_count)
                            .map(|ci| shaped.x_for_index(chars_to_bytes(subline, ci)))
                            .collect();
                        let _ = shaped.paint(point(left, line_y_screen), lh_px, window, cx);
                        shaped_line = Some(shaped);
                    } else {
                        glyph_xs = vec![px(0.0)];
                        shaped_line = None;
                    }
                }

                visual_lines.push(VisualLine {
                    para_idx,
                    char_start,
                    char_end,
                    top_px: current_y,
                    height_px: line_height,
                    font_size_px,
                    is_mermaid: false,
                    glyph_xs,
                });
                line_layouts.push(shaped_line);

                char_start = char_end + 1;
                current_y += line_height;
            }

            current_y += PARA_GAP;
        }

        // Store layout cache in state for cursor/mouse use
        self.state.update(cx, |s, _| {
            s.layout_cache = visual_lines.clone();
        });

        // ── Selection highlight ──────────────────────────────────────────────
        if let Some(sel) = selection {
            let (sel_start, sel_end) = sel.ordered();
            for (vl_idx, vl) in visual_lines.iter().enumerate() {
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
                let (sel_x_start, sel_width) = if let Some(Some(shaped)) = line_layouts.get(vl_idx) {
                    let col_s = sel_s_in_vl - vl_cs;
                    let col_e = sel_e_in_vl - vl_cs;
                    let vl_text: String = paragraphs[vl_para].plain_text()
                        .chars().skip(vl_cs).take(vl_ce - vl_cs).collect();
                    let byte_s = chars_to_bytes(&vl_text, col_s);
                    let byte_e = chars_to_bytes(&vl_text, col_e);
                    let x_s = shaped.x_for_index(byte_s);
                    let x_e = shaped.x_for_index(byte_e);
                    (left + x_s, x_e - x_s)
                } else {
                    (left, px(0.0))
                };

                if sel_width > px(0.0) {
                    window.paint_quad(fill(
                        Bounds::new(
                            point(sel_x_start, line_y_screen),
                            size(sel_width, px(vl.height_px)),
                        ),
                        rgba(0x4477ff35),
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

            if let Some((vl_idx, vl)) = cur_vl {
                let col = cursor.char_offset - vl.char_start;
                let cursor_x = if let Some(Some(shaped)) = line_layouts.get(vl_idx) {
                    let vl_text: String = paragraphs[vl.para_idx].plain_text()
                        .chars().skip(vl.char_start).take(vl.char_end - vl.char_start).collect();
                    let byte_col = chars_to_bytes(&vl_text, col);
                    left + shaped.x_for_index(byte_col)
                } else {
                    left
                };
                let cursor_y_screen = bounds.top() + px(vl.top_px);

                window.paint_quad(fill(
                    Bounds::new(
                        point(cursor_x, cursor_y_screen),
                        size(px(2.0), px(vl.height_px)),
                    ),
                    rgb(0x0099ff),
                ));
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
                let focus = state.read(cx).focus_handle.clone();
                state.update(cx, |s, cx| {
                    s.cursor = new_cursor;
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
                let (bounds, drag_anchor) = {
                    let s = state_move.read(cx);
                    (s.last_bounds.unwrap_or_default(), s.drag_anchor)
                };
                if let Some(anchor) = drag_anchor {
                    let focus_cursor = state_move.read(cx).cursor_from_point(event.position, bounds);
                    state_move.update(cx, |s, cx| {
                        s.drag_occurred = true;
                        s.cursor = focus_cursor;
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
                    // Plain click clears selection; completed drag preserves it.
                    if !s.drag_occurred {
                        s.selection = None;
                    }
                    s.drag_anchor = None;
                    s.drag_occurred = false;
                    cx.notify();
                });
            });

        if self.show_border {
            base = base.border_1().border_color(theme.tokens.border).rounded(theme.tokens.radius_md);
        }

        base.child(DocumentEditorElement { state: self.state.clone() })
    }
}
