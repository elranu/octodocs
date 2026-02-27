# Word-Style Continuous Document Editor — Implementation Plan

## Overview

Replace the current block-switching WYSIWYG editor with a **single continuous editor surface** that behaves like MS Word or Google Docs: one editor owns the entire document, the cursor flows freely from line to line, markdown syntax is never visible, and formatting is applied via toolbar or shortcuts. Saving serialises the rich model back to Markdown.

References:
- Internal: [desktop/crates/octodocs-core/src/rich_block.rs](desktop/crates/octodocs-core/src/rich_block.rs) — rich model + round-trip serialiser (already implemented, keep)  
- Internal: [desktop/crates/octodocs-core/src/renderer.rs](desktop/crates/octodocs-core/src/renderer.rs) — `parse_rich_blocks()` entry point  
- Internal: [desktop/crates/octodocs-app/src/app_state.rs](desktop/crates/octodocs-app/src/app_state.rs) — current AppState  
- Internal: [desktop/crates/octodocs-app/src/views/block_editor_pane.rs](desktop/crates/octodocs-app/src/views/block_editor_pane.rs) — pane to be replaced  
- Internal: [desktop/patches/adabraka-ui/src/components/rich_block_editor.rs](desktop/patches/adabraka-ui/src/components/rich_block_editor.rs) — per-block editor (to be superseded)  
- Internal: [desktop/patches/adabraka-ui/src/components/editor.rs](desktop/patches/adabraka-ui/src/components/editor.rs) — raw-markdown editor (used in Split/Source mode, keep)  
- External: [GPUI text shaping API](https://github.com/zed-industries/zed/blob/main/crates/gpui/src/text_system.rs)  

---

## Design Alignment

### Lessons Learned (What Went Wrong Last Time)

| Problem | Root Cause | What the New Plan Does Instead |
|---|---|---|
| Runtime panic: `text argument should not contain newlines` | Called `shape_line` on full block text that contained `\n` | All text is split into visual lines before any shaping call |
| Toolbar buttons did nothing | Toolbar closures captured a stale `Entity<RichBlockState>` at construction time; when a new block was activated, captures pointed to the old entity | Single `Entity<DocumentEditorState>` lives for the document lifetime; toolbar closures capture it once and it is always correct |
| Cursor jumped to last word on block activation | `set_content` forced `cursor = end_of_text` | The new model has no "block activation"; cursor position is continuous across the whole document |
| UX still felt like Notion block switching | Each block was a separate GPUI entity with focus handle; clicking a block transferred focus visually + caused re-render transitions | Single focus handle on the document editor; no concept of "active block" in the UI layer |
| Multiline selection/cursor was wrong | `on_mouse_down` / `character_index_for_point` assumed single `ShapedLine` per block | The document editor owns a `Vec<VisualLine>` paint cache and maps (x,y) → char offset using line index |
| Architecture drift | Incremental patches to a mismatched data model accumulated in `AppState`, `RichBlockState`, and `BlockEditorPane` simultaneously | Fresh `DocumentEditorState` in the patch, fresh `DocumentEditorPane` in the app, no leftover block-switching code |

### Chosen Direction

**One editor entity — one document.** The entire document is a `Vec<DocParagraph>` (typed, rich paragraphs) inside a single `Entity<DocumentEditorState>`. A single GPUI custom element (`DocumentEditorElement`) renders every visual line, handles all keyboard/mouse/IME input, and maintains a document-level cursor.

This is structurally identical to how Zed's own `Editor` works at a high level and is the only architecture that eliminates all of the previous defects.

### Scope

**In scope:**
- New `DocumentEditorState` entity covering the whole document
- New `DocumentEditorElement` GPUI custom element
- Inline formatting: bold, italic, inline-code, link (display only)
- Block types: paragraph, heading (H1–H3), code fence, block-quote
- Mermaid blocks: read-only rendered PNG islands that the cursor skips around
- Toolbar: bold, italic, code, H1, H2 — all wired to the single editor entity
- Keyboard: character input, Backspace, Delete, Enter (split paragraph), Home, End, arrows, Ctrl/Cmd+A
- Markdown → rich model on open; rich model → Markdown on save
- IME support via `EntityInputHandler`
- Scroll: a wrapping `div` with `overflow_y_scroll` is sufficient for MVP

**Out of scope:**
- Undo/redo history
- Drag reorder of paragraphs
- Table editing
- Nested formatting (bold + italic simultaneously)
- Collaborative editing

---

## Index

- [x] Phase 1: [Erase Old Block-Editor Code](#phase-1-erase-old-block-editor-code)
- [x] Phase 2: [DocumentParagraph Model in Core](#phase-2-documentparagraph-model-in-core)
- [x] Phase 3: [DocumentEditorState — Cursor & Editing Engine](#phase-3-documenteditorstate--cursor--editing-engine)
- [x] Phase 4: [DocumentEditorElement — Paint Pass](#phase-4-documenteditorelement--paint-pass)
- [x] Phase 5: [AppState & Toolbar Wiring](#phase-5-appstate--toolbar-wiring)
- [x] Phase 6: [Smoke Tests & Polish](#phase-6-smoke-tests--polish)

---

## Execution Update — 2026-02-26

### Delivered in this execution pass

- Resolved selection + toolbar interaction regressions in the single-surface editor so toolbar formatting now acts on the intended selected range.
- Fixed markdown span fragmentation by coalescing adjacent same-format spans before serialization, preventing noisy source output sequences.
- Added additional inline formatting support:
  - Underline
  - Strikethrough
- Wired new toolbar actions for underline and strikethrough in the desktop app toolbar.
- Updated preview rendering to support underline and strikethrough inline nodes.
- Added icon assets used by toolbar buttons:
  - `underline.svg`
  - `strikethrough.svg`

### Important implementation note

On this Linux environment, bold/italic visual face differentiation from font weight/style alone may not be reliably perceptible even when markdown state updates correctly. The editor now applies explicit visual emphasis for formatted runs so WYSIWYG feedback is immediate and deterministic.

### Technique document

See [docs/word-style-formatting-technique.md](../word-style-formatting-technique.md) for the debugging method, rendering strategy, and serialization safeguards used.

---

## Investigation Findings

### What Exists and Can Be Reused

| Artifact | Location | Verdict |
|---|---|---|
| `RichBlock` / `InlineSpan` / `SpanFormat` types | `octodocs-core/src/rich_block.rs` | **Keep** — good model, keep as the serialisation layer |
| `parse_rich_blocks()` | `octodocs-core/src/renderer.rs` | **Keep** |
| `RichBlock::to_markdown()` | `octodocs-core/src/rich_block.rs` | **Keep** |
| `EditorState` raw-text editor | `adabraka-ui/src/components/editor.rs` | **Keep for Source/Split modes only** |
| `RichBlockState` / `RichBlockEditor` | `adabraka-ui/src/components/rich_block_editor.rs` | **Delete** — superseded |
| `BlockEditorPane` / `active_block` / `active_rich_block` | `app_state.rs`, `block_editor_pane.rs` | **Delete** — superseded |

### GPUI Constraints (Confirmed)

- `text_system().shape_line(text, ...)` — **panics if text contains `\n`**. All input must be split at `\n` before shaping.
- `ShapedLine::x_for_index(byte)` — byte offset into the *shaped string* (which equals the UTF-8 line text). Use `char_to_byte_offset` helper before calling.
- `ShapedLine::closest_index_for_x(px)` — returns a byte offset, not a char offset. Must convert back.
- GPUI `Pixels` type does **not** expose `.0`. Use `/` operator for division: `y / line_height`.
- `EntityInputHandler` maps via UTF-16 ranges — IME ranges must convert through `char_to_utf16` / `utf16_to_char` helpers (already proven-working in current code).
- A custom element must implement `Element` (not `RenderOnce`) to access `prepaint` / `paint` lifecycle.
- `window.handle_input(&focus_handle, ElementInputHandler::new(bounds, entity), cx)` must be called inside `paint`, not `prepaint`.

### Architecture Reference: How Zed's Editor Handles This

Zed's `Editor` (`crates/editor/`) stores content as a `Buffer` with a rope. Cursor is a `DisplayPoint` resolved against a `DisplayMap` that applies fold/wrap transforms. Its custom element (`EditorElement`) owns all layout state per frame.

For our scope, a simpler flat model is sufficient:
- Document = `Vec<DocParagraph>` (no rope needed at this scale)
- Cursor = `DocCursor { para_idx, char_offset }` 
- Per-frame layout cache = `Vec<VisualLine>` (one entry per shaped line, including wrapped lines)

---

## Architecture

### Data Model

```
DocumentEditorState
├── paragraphs: Vec<DocParagraph>          ← document content
├── cursor: DocCursor                       ← { para_idx, char_offset }
├── selection: Option<DocSelection>         ← { anchor: DocCursor, focus: DocCursor }
├── focus_handle: FocusHandle
├── marked_range: Option<Range<usize>>      ← IME compose range (UTF-16)
└── layout_cache: Vec<VisualLine>           ← rebuilt each paint frame
```

```
DocParagraph
├── kind: ParagraphKind                     ← Paragraph | Heading(u8) | CodeFence | BlockQuote | Mermaid
└── spans: Vec<InlineSpanKind>              ← reuse from octodocs-core
```

```
VisualLine  (owned by element per frame, not persisted)
├── para_idx: usize
├── char_start: usize                       ← char offset within paragraph where this visual line starts
├── shaped: ShapedLine                      ← output of shape_line()
├── top_px: Pixels                          ← y coordinate of top edge in painted area
├── height_px: Pixels
└── is_mermaid: bool                        ← skip cursor hit-testing
```

### Cursor Model

```
DocCursor { para_idx: usize, char_offset: usize }
```

Arrow keys traverse: char_offset ± 1 within a paragraph, then para_idx ± 1 at boundaries. The `VisualLine` cache is the authoritative mapping from `DocCursor` → screen pixel.

### Component Boundaries (SOLID)

| Component | Package | Single Responsibility |
|---|---|---|
| `DocParagraph` / `DocCursor` | `octodocs-core` | Data contracts only — no rendering |
| `DocumentEditorState` | `adabraka-ui` patch | State mutation, cursor movement, text insertion, formatting |
| `DocumentEditorElement` | `adabraka-ui` patch | Paint, hit-testing, input handler registration |
| `DocumentEditorPane` | `octodocs-app` | GPUI view that hosts the element inside a scroll container |
| `AppState` | `octodocs-app` | Holds `Entity<DocumentEditorState>`, wires open/save/toolbar callbacks |

---

## Implementation Steps

### Phase 1: Erase Old Block-Editor Code

- [ ] Delete `desktop/patches/adabraka-ui/src/components/rich_block_editor.rs`
- [ ] Remove the `rich_block_editor` module from `adabraka-ui/src/components/mod.rs` and `prelude.rs`
- [ ] Remove `rich_block_editor::init(cx)` call from `adabraka-ui/src/lib.rs`
- [ ] Remove `active_block`, `active_rich_block`, split/merge/nav handlers from `AppState`
- [ ] Remove `BlockEditorPane` and its file (`views/block_editor_pane.rs`)
- [ ] Confirm `cargo check -p octodocs-app` passes with stubs where needed

### Phase 2: DocumentParagraph Model in Core

- [ ] Add `DocParagraph` and `DocCursor` types to `octodocs-core/src/rich_block.rs` (or new file `doc_model.rs`)

  ```
  DocParagraph { kind: ParagraphKind, spans: Vec<InlineSpanKind> }
  ParagraphKind: Paragraph | Heading(u8) | CodeFence(Option<String>) | BlockQuote | Mermaid(PathBuf)
  DocCursor { para_idx: usize, char_offset: usize }
  DocSelection { anchor: DocCursor, focus: DocCursor }
  ```

- [ ] Add `fn rich_blocks_to_doc_paragraphs(blocks: &[RichBlock]) -> Vec<DocParagraph>` — flattens `RichBlock::List` items into individual `Paragraph` entries with bullet prefix spans; maps 1:1 for all other variants
- [ ] Add `fn doc_paragraphs_to_rich_blocks(paragraphs: &[DocParagraph]) -> Vec<RichBlock>` — inverse; rejoins bullet paragraphs into `RichBlock::List`
- [ ] Export new types from `octodocs-core/src/lib.rs`
- [ ] Add unit tests for both conversion functions

### Phase 3: DocumentEditorState — Cursor & Editing Engine

Create `desktop/patches/adabraka-ui/src/components/document_editor.rs`.

- [ ] Define `DocumentEditorState` struct with fields listed in Architecture section
- [ ] `impl DocumentEditorState` — state constructor `new(cx)`, `load_document(paragraphs, cx)` (replaces old `set_content` — sets cursor to `{0, 0}`)
- [ ] Cursor navigation methods:
  - `move_left / move_right` — char step; cross paragraph boundary at edges
  - `move_up / move_down` — delegate to `VisualLine` cache for line-aware vertical movement
  - `move_to_line_start / move_to_line_end`
  - All `select_*` variants (extend selection while moving)
- [ ] Text mutation methods:
  - `insert_text(s: &str)` — inserts at cursor, replaces selection if active
  - `backspace()` — delete char before cursor or merge two paragraphs at para boundary
  - `delete()` — delete char after cursor
  - `enter()` — split paragraph at cursor, create new `Paragraph` entry
- [ ] Formatting methods (called by toolbar):
  - `toggle_bold()`, `toggle_italic()`, `toggle_code()` — operate on selection range across any number of spans; use same split-merge span algorithm already implemented in `rich_block_editor.rs`
  - `set_paragraph_kind(kind: ParagraphKind)` — changes the `kind` field of current paragraph
- [ ] `impl EntityInputHandler for DocumentEditorState` — UTF-16 IME contract (port working logic from current `rich_block_editor.rs`)
- [ ] `impl Focusable for DocumentEditorState`
- [ ] Define `actions!` and `cx.bind_keys` for keyboard shortcuts
- [ ] Wire `DocumentEditorState::to_rich_blocks()` → calls `doc_paragraphs_to_rich_blocks`

### Phase 4: DocumentEditorElement — Paint Pass

Still in `document_editor.rs`, implement `struct DocumentEditorElement { state: Entity<DocumentEditorState> }`.

- [ ] Implement `Element`:
  - `request_layout` — returns `LayoutId` for a block that fills available width; height determined by content
  - `prepaint` — computes `Vec<VisualLine>` for current paragraphs:
    - For each `DocParagraph`:
      - Skip Mermaid paragraphs (emit a `VisualLine { is_mermaid: true }` placeholder with fixed height)
      - Split span text at `\n` chars; shape each sub-line individually with `window.text_system().shape_line(...)`
      - Store `VisualLine { para_idx, char_start, shaped, top_px, height_px }`
    - Store `Vec<VisualLine>` into state via `cx.update` so cursor movement can use it
  - `paint` — for each `VisualLine`:
    - Call `shaped.paint(origin, line_height, window, cx)` for normal lines
    - Render Mermaid PNG via `img(path)` for Mermaid placeholders
    - Paint selection highlight rects across selected visual lines
    - Paint cursor caret using `shaped.x_for_index(byte)` on the focused line
    - Call `window.handle_input(&focus_handle, ElementInputHandler::new(bounds, state), cx)` once per frame
- [ ] Implement `on_mouse_down`:
  - Compute `(x, y)` relative to editor top-left
  - Binary-search `layout_cache` by `top_px` to find the visual line under the click
  - Use `shaped.closest_index_for_x(x)` → byte → `byte_to_char_offset` → `DocCursor`
  - Set `state.cursor`; clear selection
- [ ] Expose `DocumentEditor` wrapper (`RenderOnce`) with `show_border` option and all action listeners, mirrors `RichBlockEditor` shape  
- [ ] Export `DocumentEditorState`, `DocumentEditor` from `adabraka-ui` prelude

### Phase 5: AppState & Toolbar Wiring

- [ ] Add `doc_editor: Entity<DocumentEditorState>` to `AppState` (created once in `AppState::new`, lives for the application lifetime)
- [ ] Remove the old `active_block` / `active_rich_block` / `blocks` / `full_editor_state` split — replace with:
  - `doc_editor` — always-active rich editor entity
  - `raw_editor` — `Entity<EditorState>` for Source / Split modes (keep as-is)
- [ ] `AppState::load_document(doc: Document, cx)`:
  1. Call `Renderer::parse_rich_blocks(&doc.content)` → `Vec<RichBlock>`
  2. Call `rich_blocks_to_doc_paragraphs` → `Vec<DocParagraph>`
  3. Call `doc_editor.update(cx, |e, cx| e.load_document(paragraphs, cx))`
  4. Also populate `raw_editor` with raw source for Split/Source mode
- [ ] `AppState::serialize_document(cx)`:
  - Read `doc_editor.read(cx).to_rich_blocks()` → call `save` pipeline
- [ ] Wire auto-save subscription: subscribe to `doc_editor` changes → serialize → set `dirty = true`
- [ ] Replace `BlockEditorPane` with new `DocumentEditorPane`:
  - Renders `DocumentEditor::new(&app_state.read(cx).doc_editor)` inside a `div().overflow_y_scroll()`
- [ ] Toolbar — **all closures capture `app_weak: WeakEntity<AppState>`**; at click time:
  ```
  aw.update(cx, |state, cx| {
      state.doc_editor.update(cx, |editor, cx| editor.toggle_bold(cx))
  })
  ```
  This is the permanent fix for the stale-handle class of bugs.
- [ ] `RootView` — remove editor-weak capture; use only `app_weak` for all toolbar closures
- [ ] Confirm `ViewMode::Wysiwyg` renders `DocumentEditorPane`; Split/Source render `EditorState`-based views unchanged

### Phase 6: Smoke Tests & Polish

- [ ] `cargo test -p octodocs-core` — conversion round-trips pass
- [ ] `cargo check -p octodocs-app` — no errors
- [ ] Manual smoke tests:
  - Open a markdown file → renders without panic
  - Type characters → appear inline
  - Enter → splits paragraph, cursor moves to new paragraph
  - Backspace at start → merges with previous paragraph
  - Bold button → selected text becomes visually bold
  - Save → file on disk is valid Markdown
  - Arrow keys → cursor moves correctly across paragraph boundaries
  - Click mid-sentence → cursor placed at click position (not end)

---

## Requirements

- [ ] Nightly Rust toolchain (already present via `rust-toolchain.toml`)
- [ ] `adabraka-ui` patch Cargo.toml must keep `octodocs-core` as a local dependency (already added in previous iteration)
- [ ] No new external crate dependencies needed

---

## Testing Strategy

- [ ] Unit tests in `octodocs-core`: `rich_blocks_to_doc_paragraphs` ↔ `doc_paragraphs_to_rich_blocks` round trip for each `ParagraphKind`
- [ ] Unit tests for span split-merge algorithm on selections that span multiple spans with mixed formatting
- [ ] Manual integration tests listed under Phase 6 smoke tests — GPUI has no headless test harness for custom elements

References:
- Internal: [desktop/crates/octodocs-core/src/rich_block.rs](desktop/crates/octodocs-core/src/rich_block.rs) — existing tests as template  

---

## Potential Risks & Mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| `shape_line` panics if any sub-line string is empty | High | Guard: if split produces an empty string, substitute a single space for shaping only; store original empty string in model |
| `VisualLine` cache is stale if paragraphs mutate between `prepaint` and `paint` | Medium | Cache is rebuilt every `prepaint`; never read from state during `paint` — use only the frame-local `Vec<VisualLine>` |
| Word-wrap: a paragraph wider than the editor needs multiple visual lines per paragraph | High | During `prepaint`, measure each logical line against available width; use `ShapedLine::width()` vs bounds width to decide if truncation wrapping is needed. MVP: disable word wrap (clipping), mark as Phase 7 |
| IME marked-range spans paragraph boundary | Low | Restrict IME input to within a single paragraph (standard OS behaviour anyway) |
| Mermaid PNG not yet rendered when document loads | Low | Show a placeholder grey rect; Mermaid render is already async via `render_png()`; swap in image when task completes |

---

## Dependencies

No new Cargo dependencies. Existing:
- `gpui` (via `adabraka-gpui` patch)
- `adabraka-ui` (via local patch)
- `octodocs-core` (local)
- `pulldown-cmark` (already used in core for parsing)

---

## Success Criteria

- [x] Opening any Markdown file shows rendered rich text with no visible `**`, `#`, or backtick syntax
- [x] The user can type, delete, and press Enter without any runtime panic
- [x] Bold / Italic / Code toolbar buttons apply formatting to the selected range
- [x] Save writes a valid, well-formed Markdown file
- [x] The cursor follows the mouse click to the correct character position
- [x] The editor feels like one continuous document with no visible block boundaries or activation clicks

---

## Execution Notes — Implementation Completed

### Key Implementation Decisions

- **`doc_model.rs` in octodocs-core**: All 16 unit tests pass. `markdown_to_doc_paragraphs` and `doc_paragraphs_to_markdown` are exported from the crate root.
- **`document_editor.rs` in adabraka-ui patches**: ~1587 lines. Full `DocumentEditorState` (cursor engine, IME, all keyboard actions) + `DocumentEditorElement` (GPUI custom element with `request_layout`/`prepaint`/`paint`) + `DocumentEditor` (public `RenderOnce` wrapper).
- **Cargo patch mechanism**: Added `octodocs-core` as a path dependency to `patches/adabraka-ui/Cargo.toml`.
- **`Pixels` field `.0` is private** — all arithmetic uses operator overloads: `bounds.left() + px(x)`.
- **`shape_line` returns `ShapedLine` directly** (not `Result`). Never call with strings containing `\n`.
- **`BlockEditorPane` deleted** — `views/block_editor_pane.rs` removed; `AppState` no longer has `blocks` activation logic (`active_block`, `editor_state`, `_content_subscription`, `activate_block()`, `deactivate_block()`).
- **`blocks: Vec<DocumentBlock>` kept** for `PreviewPane` in Split mode.
- **Toolbar closures** now call `state.doc_editor.update(cx, |editor, cx| editor.toggle_bold(cx))` etc. `ParagraphKind` imported from `octodocs_core` in `root.rs`.
- **`cargo check -p octodocs-app`**: 0 errors, 1 unrelated warning (`std::sync::Arc` in `audio_player.rs`).
