use adabraka_ui::prelude::*;
use gpui::*;
use octodocs_core::{InlineSpan, InlineSpanKind, RichBlock, SpanFormat};
use std::ops::Range;

actions!(
    rich_block_editor,
    [
        MoveLeft,
        MoveRight,
        MoveToLineStart,
        MoveToLineEnd,
        SelectLeft,
        SelectRight,
        SelectAll,
        Backspace,
        Delete,
        Enter,
    ]
);

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("left", MoveLeft, Some("RichBlockEditor")),
        KeyBinding::new("right", MoveRight, Some("RichBlockEditor")),
        KeyBinding::new("home", MoveToLineStart, Some("RichBlockEditor")),
        KeyBinding::new("end", MoveToLineEnd, Some("RichBlockEditor")),
        KeyBinding::new("shift-left", SelectLeft, Some("RichBlockEditor")),
        KeyBinding::new("shift-right", SelectRight, Some("RichBlockEditor")),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-a", SelectAll, Some("RichBlockEditor")),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-a", SelectAll, Some("RichBlockEditor")),
        KeyBinding::new("backspace", Backspace, Some("RichBlockEditor")),
        KeyBinding::new("delete", Delete, Some("RichBlockEditor")),
        KeyBinding::new("enter", Enter, Some("RichBlockEditor")),
    ]);
}

/// Position within a span list: which span + byte offset within that span.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SpanCursor {
    pub span_idx: usize,
    pub char_offset: usize,
}

impl SpanCursor {
    pub fn visual_offset(&self, spans: &[InlineSpanKind]) -> usize {
        let mut offset = 0;
        for (i, span) in spans.iter().enumerate() {
            if i == self.span_idx {
                offset += self.char_offset;
                break;
            }
            offset += span.text().chars().count();
        }
        offset
    }

    pub fn from_visual_offset(spans: &[InlineSpanKind], mut offset: usize) -> SpanCursor {
        if spans.is_empty() {
            return SpanCursor::default();
        }
        for (i, span) in spans.iter().enumerate() {
            let len = span.text().chars().count();
            if offset <= len {
                return SpanCursor { span_idx: i, char_offset: offset };
            }
            offset -= len;
        }
        let last = spans.len() - 1;
        SpanCursor { span_idx: last, char_offset: spans[last].text().chars().count() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpanSelection {
    pub anchor: SpanCursor,
    pub focus: SpanCursor,
}

impl SpanSelection {
    fn ordered(&self, spans: &[InlineSpanKind]) -> (SpanCursor, SpanCursor) {
        let a = self.anchor.visual_offset(spans);
        let b = self.focus.visual_offset(spans);
        if a <= b { (self.anchor, self.focus) } else { (self.focus, self.anchor) }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpanFormatToggle {
    Bold,
    Italic,
    Code,
}

pub struct RichBlockState {
    pub focus_handle: FocusHandle,
    pub spans: Vec<InlineSpanKind>,
    heading_level: Option<u8>,
    pub cursor: SpanCursor,
    pub selection: Option<SpanSelection>,
    marked_range: Option<Range<usize>>,
    pub last_bounds: Option<Bounds<Pixels>>,
    pub last_shaped: Option<ShapedLine>,
    pub font_size: Pixels,
    pub split_requested: Option<SpanCursor>,
    pub merge_prev_requested: bool,
}

impl RichBlockState {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            spans: vec![InlineSpanKind::plain("")],
            heading_level: None,
            cursor: SpanCursor::default(),
            selection: None,
            marked_range: None,
            last_bounds: None,
            last_shaped: None,
            font_size: px(16.0),
            split_requested: None,
            merge_prev_requested: false,
        }
    }

    pub fn set_content(&mut self, block: &RichBlock, cx: &mut Context<Self>) {
        match block {
            RichBlock::Paragraph { spans } => {
                self.spans = if spans.is_empty() {
                    vec![InlineSpanKind::plain("")]
                } else {
                    spans.clone()
                };
                self.heading_level = None;
            }
            RichBlock::Heading { level, text } => {
                self.spans = vec![InlineSpanKind::plain(text.as_str())];
                self.heading_level = Some(*level);
            }
            _ => {
                let md = block.to_markdown();
                self.spans = vec![InlineSpanKind::plain(md.trim_end())];
                self.heading_level = None;
            }
        }
        self.cursor_to_end();
        self.selection = None;
        self.split_requested = None;
        self.merge_prev_requested = false;
        cx.notify();
    }

    pub fn to_rich_block(&self) -> RichBlock {
        if let Some(level) = self.heading_level {
            return RichBlock::Heading { level, text: self.visible_text() };
        }
        RichBlock::Paragraph { spans: self.spans.clone() }
    }

    pub fn cursor_to_end(&mut self) {
        if self.spans.is_empty() {
            self.cursor = SpanCursor::default();
        } else {
            let last = self.spans.len() - 1;
            self.cursor = SpanCursor {
                span_idx: last,
                char_offset: self.spans[last].text().chars().count(),
            };
        }
    }

    pub fn visible_text(&self) -> String {
        self.spans.iter().map(|s| s.text()).collect()
    }

    pub fn total_char_len(&self) -> usize {
        self.spans.iter().map(|s| s.text().chars().count()).sum()
    }

    fn ensure_nonempty(&mut self) {
        if self.spans.is_empty() {
            self.spans.push(InlineSpanKind::plain(""));
        }
    }

    // ── Keyboard actions ──────────────────────────────────────────

    pub fn move_left(&mut self, _: &MoveLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.selection = None;
        if self.cursor.char_offset > 0 {
            self.cursor.char_offset -= 1;
        } else if self.cursor.span_idx > 0 {
            self.cursor.span_idx -= 1;
            self.cursor.char_offset = self.spans[self.cursor.span_idx].text().chars().count();
        }
        cx.notify();
    }

    pub fn move_right(&mut self, _: &MoveRight, _: &mut Window, cx: &mut Context<Self>) {
        self.selection = None;
        if self.spans.is_empty() {
            return;
        }
        let span_len = self.spans[self.cursor.span_idx].text().chars().count();
        if self.cursor.char_offset < span_len {
            self.cursor.char_offset += 1;
        } else if self.cursor.span_idx + 1 < self.spans.len() {
            self.cursor.span_idx += 1;
            self.cursor.char_offset = 0;
        }
        cx.notify();
    }

    pub fn move_to_line_start(&mut self, _: &MoveToLineStart, _: &mut Window, cx: &mut Context<Self>) {
        self.cursor = SpanCursor::default();
        self.selection = None;
        cx.notify();
    }

    pub fn move_to_line_end(&mut self, _: &MoveToLineEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.cursor_to_end();
        self.selection = None;
        cx.notify();
    }

    pub fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        let anchor = self.selection.map(|s| s.anchor).unwrap_or(self.cursor);
        if self.cursor.char_offset > 0 {
            self.cursor.char_offset -= 1;
        } else if self.cursor.span_idx > 0 {
            self.cursor.span_idx -= 1;
            self.cursor.char_offset = self.spans[self.cursor.span_idx].text().chars().count();
        }
        if anchor != self.cursor {
            self.selection = Some(SpanSelection { anchor, focus: self.cursor });
        } else {
            self.selection = None;
        }
        cx.notify();
    }

    pub fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        let anchor = self.selection.map(|s| s.anchor).unwrap_or(self.cursor);
        if self.spans.is_empty() {
            return;
        }
        let span_len = self.spans[self.cursor.span_idx].text().chars().count();
        if self.cursor.char_offset < span_len {
            self.cursor.char_offset += 1;
        } else if self.cursor.span_idx + 1 < self.spans.len() {
            self.cursor.span_idx += 1;
            self.cursor.char_offset = 0;
        }
        if anchor != self.cursor {
            self.selection = Some(SpanSelection { anchor, focus: self.cursor });
        } else {
            self.selection = None;
        }
        cx.notify();
    }

    pub fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        if self.spans.is_empty() {
            return;
        }
        let last = self.spans.len() - 1;
        let anchor = SpanCursor { span_idx: 0, char_offset: 0 };
        let focus = SpanCursor {
            span_idx: last,
            char_offset: self.spans[last].text().chars().count(),
        };
        self.cursor = focus;
        self.selection = Some(SpanSelection { anchor, focus });
        cx.notify();
    }

    pub fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(sel) = self.selection.take() {
            self.delete_selection_range(sel);
            self.merge_adjacent_same_format();
            self.ensure_nonempty();
            cx.notify();
            return;
        }
        if self.cursor.span_idx == 0 && self.cursor.char_offset == 0 {
            self.merge_prev_requested = true;
            cx.notify();
            return;
        }
        if self.cursor.char_offset > 0 {
            self.cursor.char_offset -= 1;
            self.delete_char_at_cursor();
        } else if self.cursor.span_idx > 0 {
            self.cursor.span_idx -= 1;
            let prev_len = self.spans[self.cursor.span_idx].text().chars().count();
            self.cursor.char_offset = prev_len.saturating_sub(1);
            self.delete_char_at_cursor();
        }
        self.merge_adjacent_same_format();
        self.ensure_nonempty();
        cx.notify();
    }

    pub fn delete_forward(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(sel) = self.selection.take() {
            self.delete_selection_range(sel);
            self.merge_adjacent_same_format();
            self.ensure_nonempty();
            cx.notify();
            return;
        }
        if self.spans.is_empty() {
            return;
        }
        let span_len = self.spans[self.cursor.span_idx].text().chars().count();
        if self.cursor.char_offset < span_len {
            self.delete_char_forward_at_cursor();
        }
        self.merge_adjacent_same_format();
        self.ensure_nonempty();
        cx.notify();
    }

    pub fn enter(&mut self, _: &Enter, _: &mut Window, cx: &mut Context<Self>) {
        self.split_requested = Some(self.cursor);
        cx.notify();
    }

    fn delete_char_at_cursor(&mut self) {
        if self.spans.is_empty() {
            return;
        }
        let idx = self.cursor.span_idx;
        let char_offset = self.cursor.char_offset;
        match &mut self.spans[idx] {
            InlineSpanKind::Styled(ref mut is) => {
                let mut chars: Vec<char> = is.text.chars().collect();
                if char_offset < chars.len() {
                    chars.remove(char_offset);
                    is.text = chars.into_iter().collect();
                }
            }
            InlineSpanKind::Link { text, .. } => {
                let mut chars: Vec<char> = text.chars().collect();
                if char_offset < chars.len() {
                    chars.remove(char_offset);
                    *text = chars.into_iter().collect();
                }
            }
        }
    }

    fn delete_char_forward_at_cursor(&mut self) {
        if self.spans.is_empty() {
            return;
        }
        let idx = self.cursor.span_idx;
        let char_offset = self.cursor.char_offset;
        match &mut self.spans[idx] {
            InlineSpanKind::Styled(ref mut is) => {
                let mut chars: Vec<char> = is.text.chars().collect();
                if char_offset < chars.len() {
                    chars.remove(char_offset);
                    is.text = chars.into_iter().collect();
                }
            }
            InlineSpanKind::Link { text, .. } => {
                let mut chars: Vec<char> = text.chars().collect();
                if char_offset < chars.len() {
                    chars.remove(char_offset);
                    *text = chars.into_iter().collect();
                }
            }
        }
    }

    fn delete_selection_range(&mut self, sel: SpanSelection) {
        if self.spans.is_empty() {
            return;
        }
        let (start, end) = sel.ordered(&self.spans);
        let start_vis = start.visual_offset(&self.spans);
        let end_vis = end.visual_offset(&self.spans);
        if start_vis >= end_vis {
            return;
        }

        let mut new_spans: Vec<InlineSpanKind> = Vec::new();
        let mut vis = 0usize;

        let old_spans = std::mem::take(&mut self.spans);
        for span in old_spans {
            let span_chars: Vec<char> = span.text().chars().collect();
            let len = span_chars.len();
            let span_start = vis;
            vis += len;

            let before_end = start_vis.saturating_sub(span_start).min(len);
            let after_start = end_vis.saturating_sub(span_start).min(len);

            let before: String = span_chars[..before_end].iter().collect();
            let after: String = span_chars[after_start..].iter().collect();
            let combined = before + &after;

            if !combined.is_empty() {
                match &span {
                    InlineSpanKind::Styled(is) => {
                        new_spans.push(InlineSpanKind::Styled(InlineSpan::new(
                            combined,
                            is.format.clone(),
                        )));
                    }
                    InlineSpanKind::Link { url, .. } => {
                        new_spans.push(InlineSpanKind::Link { text: combined, url: url.clone() });
                    }
                }
            }
        }

        self.spans = new_spans;
        self.cursor = start;
    }

    fn merge_adjacent_same_format(&mut self) {
        if self.spans.len() <= 1 {
            return;
        }
        let cursor_vis = self.cursor.visual_offset(&self.spans);
        let old_spans = std::mem::take(&mut self.spans);
        let mut merged: Vec<InlineSpanKind> = Vec::new();

        for span in old_spans {
            let can_merge = if let Some(InlineSpanKind::Styled(last)) = merged.last() {
                if let InlineSpanKind::Styled(next) = &span {
                    last.format == next.format
                } else {
                    false
                }
            } else {
                false
            };

            if can_merge {
                if let Some(InlineSpanKind::Styled(ref mut last)) = merged.last_mut() {
                    last.text.push_str(span.text());
                }
            } else {
                merged.push(span);
            }
        }

        self.spans = merged;
        self.cursor = SpanCursor::from_visual_offset(&self.spans, cursor_vis);
    }

    fn format_at_visual(&self, vis: usize) -> SpanFormat {
        let mut offset = 0;
        for span in &self.spans {
            let len = span.text().chars().count();
            if offset + len > vis {
                if let InlineSpanKind::Styled(is) = span {
                    return is.format.clone();
                }
                return SpanFormat::default();
            }
            offset += len;
        }
        SpanFormat::default()
    }

    pub fn apply_format(&mut self, toggle: SpanFormatToggle, cx: &mut Context<Self>) {
        let sel = match self.selection {
            Some(s) => s,
            None => {
                if self.spans.is_empty() {
                    return;
                }
                let idx = self.cursor.span_idx.min(self.spans.len() - 1);
                if let InlineSpanKind::Styled(ref mut is) = self.spans[idx] {
                    match toggle {
                        SpanFormatToggle::Bold => is.format.bold = !is.format.bold,
                        SpanFormatToggle::Italic => is.format.italic = !is.format.italic,
                        SpanFormatToggle::Code => is.format.code = !is.format.code,
                    }
                }
                cx.notify();
                return;
            }
        };

        if self.spans.is_empty() {
            return;
        }

        let cursor_vis = self.cursor.visual_offset(&self.spans);
        let (start, end) = sel.ordered(&self.spans);
        let start_vis = start.visual_offset(&self.spans);
        let end_vis = end.visual_offset(&self.spans);
        if start_vis == end_vis {
            return;
        }

        let all_have_format = self.check_all_spans_have_format(start_vis, end_vis, toggle);
        let new_val = !all_have_format;

        self.apply_format_to_range(start_vis, end_vis, toggle, new_val);
        self.merge_adjacent_same_format();
        self.cursor = SpanCursor::from_visual_offset(&self.spans, cursor_vis);
        cx.notify();
    }

    fn check_all_spans_have_format(
        &self,
        start_vis: usize,
        end_vis: usize,
        toggle: SpanFormatToggle,
    ) -> bool {
        let mut vis = 0;
        for span in &self.spans {
            let len = span.text().chars().count();
            let span_end = vis + len;
            if span_end > start_vis && vis < end_vis {
                match span {
                    InlineSpanKind::Styled(is) => {
                        let has = match toggle {
                            SpanFormatToggle::Bold => is.format.bold,
                            SpanFormatToggle::Italic => is.format.italic,
                            SpanFormatToggle::Code => is.format.code,
                        };
                        if !has {
                            return false;
                        }
                    }
                    InlineSpanKind::Link { .. } => return false,
                }
            }
            vis += len;
        }
        true
    }

    fn apply_format_to_range(
        &mut self,
        start_vis: usize,
        end_vis: usize,
        toggle: SpanFormatToggle,
        new_val: bool,
    ) {
        let mut new_spans: Vec<InlineSpanKind> = Vec::new();
        let mut vis = 0;

        let old_spans = std::mem::take(&mut self.spans);
        for span in old_spans {
            let chars: Vec<char> = span.text().chars().collect();
            let len = chars.len();
            let span_start = vis;
            let span_end = vis + len;
            vis += len;

            if span_end <= start_vis || span_start >= end_vis {
                new_spans.push(span);
                continue;
            }

            match span {
                InlineSpanKind::Styled(is) => {
                    if span_start < start_vis {
                        let before: String = chars[..start_vis - span_start].iter().collect();
                        new_spans.push(InlineSpanKind::Styled(InlineSpan::new(
                            before,
                            is.format.clone(),
                        )));
                    }
                    let sel_start = start_vis.saturating_sub(span_start);
                    let sel_end = end_vis.saturating_sub(span_start).min(len);
                    let selected: String = chars[sel_start..sel_end].iter().collect();
                    let mut new_fmt = is.format.clone();
                    match toggle {
                        SpanFormatToggle::Bold => new_fmt.bold = new_val,
                        SpanFormatToggle::Italic => new_fmt.italic = new_val,
                        SpanFormatToggle::Code => new_fmt.code = new_val,
                    }
                    new_spans.push(InlineSpanKind::Styled(InlineSpan::new(selected, new_fmt)));
                    if span_end > end_vis {
                        let after: String = chars[end_vis - span_start..].iter().collect();
                        new_spans.push(InlineSpanKind::Styled(InlineSpan::new(
                            after,
                            is.format.clone(),
                        )));
                    }
                }
                InlineSpanKind::Link { text, url } => {
                    new_spans.push(InlineSpanKind::Link { text, url });
                }
            }
        }

        self.spans = new_spans;
    }

    pub fn insert_text_at_cursor(&mut self, text: &str, cx: &mut Context<Self>) {
        if let Some(sel) = self.selection.take() {
            self.delete_selection_range(sel);
        }
        self.ensure_nonempty();

        let idx = self.cursor.span_idx.min(self.spans.len() - 1);
        let char_offset = self.cursor.char_offset;

        match &mut self.spans[idx] {
            InlineSpanKind::Styled(ref mut is) => {
                let mut chars: Vec<char> = is.text.chars().collect();
                let insert_at = char_offset.min(chars.len());
                for (i, c) in text.chars().enumerate() {
                    chars.insert(insert_at + i, c);
                }
                is.text = chars.into_iter().collect();
            }
            InlineSpanKind::Link { text: link_text, .. } => {
                let mut chars: Vec<char> = link_text.chars().collect();
                let insert_at = char_offset.min(chars.len());
                for (i, c) in text.chars().enumerate() {
                    chars.insert(insert_at + i, c);
                }
                *link_text = chars.into_iter().collect();
            }
        }

        self.cursor.char_offset += text.chars().count();
        cx.notify();
    }

    // ── UTF-16 helpers ────────────────────────────────────────────

    fn char_offset_to_utf16(&self, char_offset: usize) -> usize {
        self.visible_text().chars().take(char_offset).map(|c| c.len_utf16()).sum()
    }

    fn utf16_to_char_offset(&self, utf16: usize) -> usize {
        let text = self.visible_text();
        let mut count = 0usize;
        let mut char_idx = 0usize;
        for c in text.chars() {
            if count >= utf16 {
                break;
            }
            count += c.len_utf16();
            char_idx += 1;
        }
        char_idx
    }
}

impl Focusable for RichBlockState {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for RichBlockState {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}

impl EntityInputHandler for RichBlockState {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let text = self.visible_text();
        let start_char = self.utf16_to_char_offset(range_utf16.start);
        let end_char = self.utf16_to_char_offset(range_utf16.end);
        let actual_start = self.char_offset_to_utf16(start_char);
        let actual_end = self.char_offset_to_utf16(end_char);
        *actual_range = Some(actual_start..actual_end);
        let result: String = text.chars().skip(start_char).take(end_char - start_char).collect();
        Some(result)
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        if let Some(sel) = &self.selection {
            let (start, end) = sel.ordered(&self.spans);
            let start_utf16 = self.char_offset_to_utf16(start.visual_offset(&self.spans));
            let end_utf16 = self.char_offset_to_utf16(end.visual_offset(&self.spans));
            Some(UTF16Selection { range: start_utf16..end_utf16, reversed: false })
        } else {
            let cursor_vis = self.cursor.visual_offset(&self.spans);
            let cursor_utf16 = self.char_offset_to_utf16(cursor_vis);
            Some(UTF16Selection { range: cursor_utf16..cursor_utf16, reversed: false })
        }
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked_range.as_ref().map(|r| {
            let start = self.char_offset_to_utf16(r.start);
            let end = self.char_offset_to_utf16(r.end);
            start..end
        })
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
        let range_chars = range_utf16
            .map(|r| self.utf16_to_char_offset(r.start)..self.utf16_to_char_offset(r.end))
            .or_else(|| self.marked_range.clone())
            .or_else(|| {
                if let Some(sel) = &self.selection {
                    let (start, end) = sel.ordered(&self.spans);
                    Some(
                        start.visual_offset(&self.spans)..end.visual_offset(&self.spans),
                    )
                } else {
                    let c = self.cursor.visual_offset(&self.spans);
                    Some(c..c)
                }
            });

        if let Some(range) = range_chars {
            if range.start != range.end {
                let start_cursor =
                    SpanCursor::from_visual_offset(&self.spans, range.start);
                let end_cursor = SpanCursor::from_visual_offset(&self.spans, range.end);
                self.delete_selection_range(SpanSelection {
                    anchor: start_cursor,
                    focus: end_cursor,
                });
            }
        }

        if !new_text.is_empty() {
            self.insert_text_at_cursor(new_text, cx);
        }

        self.marked_range = None;
        self.merge_adjacent_same_format();
        self.ensure_nonempty();
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_marked_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range_chars = range_utf16
            .map(|r| self.utf16_to_char_offset(r.start)..self.utf16_to_char_offset(r.end))
            .unwrap_or_else(|| {
                let c = self.cursor.visual_offset(&self.spans);
                c..c
            });

        if range_chars.start != range_chars.end {
            let start_cursor = SpanCursor::from_visual_offset(&self.spans, range_chars.start);
            let end_cursor = SpanCursor::from_visual_offset(&self.spans, range_chars.end);
            self.delete_selection_range(SpanSelection {
                anchor: start_cursor,
                focus: end_cursor,
            });
        }

        let insert_start_vis = self.cursor.visual_offset(&self.spans);

        if !new_text.is_empty() {
            self.insert_text_at_cursor(new_text, cx);
        }

        if !new_text.is_empty() {
            if let Some(new_mr_utf16) = new_marked_range_utf16 {
                // new_mr_utf16 is relative to new_text, convert within new_text
                let start = insert_start_vis + utf16_to_char_offset_in(new_text, new_mr_utf16.start);
                let end = insert_start_vis + utf16_to_char_offset_in(new_text, new_mr_utf16.end);
                self.marked_range = Some(start..end);
            } else {
                let end_vis = insert_start_vis + new_text.chars().count();
                self.marked_range = Some(insert_start_vis..end_vis);
            }
        }

        self.merge_adjacent_same_format();
        self.ensure_nonempty();
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
        None
    }
}

// ── RichBlockElement ──────────────────────────────────────────────────────

pub struct RichBlockElement {
    pub state: Entity<RichBlockState>,
    pub font_size: Pixels,
}

impl IntoElement for RichBlockElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for RichBlockElement {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let font_size = self.font_size;
        let line_height = font_size * 1.6;
        let mut style = gpui::Style::default();
        style.size.width = relative(1.0).into();
        style.size.height = line_height.into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let theme = use_theme();
        let focus_handle = self.state.read(cx).focus_handle.clone();
        let font_size = self.font_size;
        let line_height = font_size * 1.6;

        self.state.update(cx, |state, _| {
            state.last_bounds = Some(bounds);
        });

        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.state.clone()),
            cx,
        );

        let (spans, cursor, selection) = {
            let state = self.state.read(cx);
            (state.spans.clone(), state.cursor, state.selection)
        };

        let full_text: String = spans.iter().map(|s| s.text()).collect();

        if full_text.is_empty() {
            if focus_handle.is_focused(window) {
                window.paint_quad(fill(
                    Bounds::new(bounds.origin, size(px(2.0), line_height)),
                    theme.tokens.primary,
                ));
            }
            return;
        }

        let text_style = window.text_style();
        let mut text_runs: Vec<TextRun> = Vec::new();

        for span in &spans {
            let byte_len = span.text().len();
            if byte_len == 0 {
                continue;
            }
            match span {
                InlineSpanKind::Styled(is) => {
                    let mut font = text_style.font();
                    if is.format.bold {
                        font.weight = FontWeight::BOLD;
                    }
                    if is.format.italic {
                        font.style = FontStyle::Italic;
                    }
                    if is.format.code {
                        font.family = theme.tokens.font_mono.clone();
                    }
                    let bg = if is.format.code {
                        Some(theme.tokens.muted.opacity(0.6))
                    } else {
                        None
                    };
                    text_runs.push(TextRun {
                        len: byte_len,
                        font,
                        color: theme.tokens.foreground,
                        background_color: bg,
                        underline: None,
                        strikethrough: None,
                    });
                }
                InlineSpanKind::Link { .. } => {
                    text_runs.push(TextRun {
                        len: byte_len,
                        font: text_style.font(),
                        color: theme.tokens.primary,
                        background_color: None,
                        underline: Some(UnderlineStyle {
                            thickness: px(1.0),
                            color: None,
                            wavy: false,
                        }),
                        strikethrough: None,
                    });
                }
            }
        }

        if text_runs.is_empty() {
            return;
        }

        let shaped = window.text_system().shape_line(
            full_text.clone().into(),
            font_size,
            &text_runs,
            None,
        );

        // Paint selection highlight
        if let Some(sel) = selection {
            let (start, end) = sel.ordered(&spans);
            let start_vis = start.visual_offset(&spans);
            let end_vis = end.visual_offset(&spans);
            let start_byte = char_offset_to_byte_offset(&full_text, start_vis);
            let end_byte = char_offset_to_byte_offset(&full_text, end_vis);
            let x_start = shaped.x_for_index(start_byte);
            let x_end = shaped.x_for_index(end_byte);
            window.paint_quad(fill(
                Bounds::new(
                    point(bounds.origin.x + x_start, bounds.origin.y),
                    size(x_end - x_start, line_height),
                ),
                theme.tokens.primary.opacity(0.25),
            ));
        }

        let _ = shaped.paint(bounds.origin, line_height, window, cx);

        self.state.update(cx, |state, _| {
            state.last_shaped = Some(shaped.clone());
        });

        // Paint cursor
        if focus_handle.is_focused(window) {
            let cursor_vis = cursor.visual_offset(&spans);
            let cursor_byte = char_offset_to_byte_offset(&full_text, cursor_vis);
            let cursor_x = shaped.x_for_index(cursor_byte);
            window.paint_quad(fill(
                Bounds::new(
                    point(bounds.origin.x + cursor_x, bounds.origin.y),
                    size(px(2.0), line_height),
                ),
                theme.tokens.primary,
            ));
        }

        // Mouse click → cursor placement
        let state_entity = self.state.clone();
        let bounds_copy = bounds;
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble && bounds_copy.contains(&event.position) {
                let (opt_shaped, focus_handle) = {
                    let state = state_entity.read(cx);
                    (state.last_shaped.clone(), state.focus_handle.clone())
                };
                if let Some(shaped) = opt_shaped {
                    let full_text: String =
                        state_entity.read(cx).spans.iter().map(|s| s.text()).collect();
                    let rel_x = event.position.x - bounds_copy.origin.x;
                    let byte_idx = shaped.closest_index_for_x(rel_x);
                    let char_idx = byte_offset_to_char_offset(&full_text, byte_idx);
                    let new_cursor =
                        SpanCursor::from_visual_offset(&state_entity.read(cx).spans, char_idx);
                    state_entity.update(cx, |state, cx| {
                        state.cursor = new_cursor;
                        state.selection = None;
                        cx.notify();
                    });
                }
                window.focus(&focus_handle);
            }
        });
    }
}

fn char_offset_to_byte_offset(text: &str, char_offset: usize) -> usize {
    text.char_indices().nth(char_offset).map(|(b, _)| b).unwrap_or(text.len())
}

fn byte_offset_to_char_offset(text: &str, byte_offset: usize) -> usize {
    text.char_indices().take_while(|(b, _)| *b < byte_offset).count()
}

/// Convert a UTF-16 offset within `text` to a char (Unicode scalar) offset.
fn utf16_to_char_offset_in(text: &str, utf16: usize) -> usize {
    let mut count = 0usize;
    let mut char_idx = 0usize;
    for c in text.chars() {
        if count >= utf16 {
            break;
        }
        count += c.len_utf16();
        char_idx += 1;
    }
    char_idx
}

// ── RichBlockEditor (div wrapper) ────────────────────────────────────────

#[derive(IntoElement)]
pub struct RichBlockEditor {
    state: Entity<RichBlockState>,
}

impl RichBlockEditor {
    pub fn new(state: &Entity<RichBlockState>) -> Self {
        Self { state: state.clone() }
    }
}

impl RenderOnce for RichBlockEditor {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = self.state.clone();
        div()
            .id(("rich-block-editor", state.entity_id()))
            .key_context("RichBlockEditor")
            .track_focus(&state.read(cx).focus_handle(cx))
            .w_full()
            .min_h(px(28.0))
            .on_action(window.listener_for(&state, RichBlockState::move_left))
            .on_action(window.listener_for(&state, RichBlockState::move_right))
            .on_action(window.listener_for(&state, RichBlockState::move_to_line_start))
            .on_action(window.listener_for(&state, RichBlockState::move_to_line_end))
            .on_action(window.listener_for(&state, RichBlockState::select_left))
            .on_action(window.listener_for(&state, RichBlockState::select_right))
            .on_action(window.listener_for(&state, RichBlockState::select_all))
            .on_action(window.listener_for(&state, RichBlockState::backspace))
            .on_action(window.listener_for(&state, RichBlockState::delete_forward))
            .on_action(window.listener_for(&state, RichBlockState::enter))
            .child(RichBlockElement { state, font_size: px(16.0) })
    }
}
