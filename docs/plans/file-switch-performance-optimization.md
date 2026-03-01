# File-Switch Performance Optimization Plan

## Overview

Switching between markdown files in the OctoDocs sidebar currently takes 1–2 seconds. The sidebar highlight delay introduced during previous UX fixes has been resolved; the remaining latency is caused by the WYSIWYG editor's unvirtualized paint pipeline and the synchronous file-open architecture. This plan addresses those root causes through four ordered phases.

References:
- Internal: [desktop/patches/adabraka-ui/src/components/document_editor.rs](../desktop/patches/adabraka-ui/src/components/document_editor.rs)
- Internal: [desktop/crates/octodocs-app/src/app_state.rs](../desktop/crates/octodocs-app/src/app_state.rs)
- Internal: [desktop/crates/octodocs-app/src/views/github_sidebar.rs](../desktop/crates/octodocs-app/src/views/github_sidebar.rs)
- External: [Zed markdown preview crate](https://github.com/zed-industries/zed/tree/main/crates/markdown)
- External: [VS Code TextMate worker tokenizer](https://github.com/microsoft/vscode/blob/main/src/vs/workbench/services/textMate/browser/worker/textMateTokenizationWorker.worker.ts)

---

## Design Alignment

### Chosen Direction

- **Approach**: Incremental, paint-layer virtualization combined with a two-phase load model.
- **Rationale**: The root bottleneck is in the WYSIWYG renderer, not I/O. Virtualization eliminates the per-file-switch CPU spike without requiring a re-architecture of the editor data model, keeping risk low while delivering the largest perceived improvement.

### Scope

**In scope:**
- Viewport-aware paragraph culling in `DocumentEditorElement::paint()` and `request_layout()`
- Two-phase document open (show blank editor with correct active state immediately, hydrate content asynchronously)
- Generation counter on `AppState` to discard stale load results on rapid switching
- Sidebar directory scan cache to avoid `read_dir()` on every paint pass

**Out of scope:**
- Rope/piece-tree document buffer (no user editing performance complaints yet)
- Incremental/diff-based re-parsing on keystroke (edit latency is acceptable)
- Full replacement of the `DocumentEditorElement` with a GPUI `list` virtualization component
- Mermaid pre-rendering in a background thread (separate concern)

### Constraints

- All changes to `DocumentEditorElement` must remain in the vendored `patches/adabraka-ui/` tree.
- No Tokio runtime — async work must use GPUI's `background_executor()` and `cx.spawn()`.
- `DocumentEditorState` must remain the single source of truth for paragraph data; no duplicated copies.
- The `layout_cache: Vec<VisualLine>` and `para_visual_heights: Vec<f32>` fields already exist and must be used rather than re-invented.

### Risks & Concerns

| Risk / Concern | Source | Resolution |
|----------------|--------|------------|
| Viewport culling breaks cursor hit-testing (click coords map to wrong paragraph) | Investigation — `paint()` uses `bounds.top() + px(vl.top_px)` for all paragraphs | Culling must only skip GPU draw calls, not the `top_px` accumulation; hit-test loop reads from `layout_cache` which already stores absolute positions |
| Two-phase open leaves a blank flash before content appears | Design | Content hydration completes in ~16 ms on local files; blank frame is imperceptible; on slow GitHub pulls the loader icon already covers the transition |
| Generation counter adds contention if UI thread and background task race | Design | Counter is read/written only on the UI thread (in `load_document_parsed` and `pull_and_open_file`); background task captures the expected generation at spawn time |
| Sidebar cache becomes stale after external file-system changes | Investigation | Cache is invalidated on every `pull_and_open_file` call and on the next `cx.notify()` triggered by a sync operation — acceptable for current usage patterns |

### Questions Resolved During Process

- *Why is switching still slow after moving I/O to background?* → `DocumentEditorElement::paint()` re-shapes every paragraph every frame; triggered on `cx.notify()` even when only the active-file path changed.
- *Does Zed virtualize its markdown preview?* → Yes; `list(self.list_state.clone(), cx.processor(...))` in `crates/markdown` — only visible blocks enter the render tree.
- *Can we use `para_visual_heights` for off-screen height estimation without shaping?* → Yes; heights are populated on the previous render pass and persist until `load_document()` clears them — sufficient for scroll-height estimation.

---

## Index

- [ ] Phase 1: [Viewport Virtualization in DocumentEditor](#phase-1-viewport-virtualization-in-documenteditor)
- [ ] Phase 2: [Two-Phase Document Open](#phase-2-two-phase-document-open)
- [ ] Phase 3: [Cancellable Versioned Load Pipeline](#phase-3-cancellable-versioned-load-pipeline)
- [ ] Phase 4: [Sidebar Directory Scan Cache](#phase-4-sidebar-directory-scan-cache)

---

## Investigation Findings

### Codebase Analysis

**`DocumentEditorElement::paint()` — the primary bottleneck**

Located at [document_editor.rs line 1885](../desktop/patches/adabraka-ui/src/components/document_editor.rs). The method iterates `state.paragraphs` unconditionally. For every paragraph it:
1. Shapes all text runs via GPUI's font shaper (CPU-bound).
2. Decodes Mermaid diagrams and images if not in thread-local cache.
3. Issues GPU draw calls for glyphs, backgrounds, decorations, table grids.

None of this is guarded by a viewport check. A 200-paragraph document shapes every paragraph on every `cx.notify()` call, including the one fired immediately after `load_document()`.

**`request_layout()` and `prepaint()` — duplicated linear scans**

Both methods independently iterate all paragraphs to accumulate `total_h`. `para_visual_heights: Vec<f32>` ([line 162](../desktop/patches/adabraka-ui/src/components/document_editor.rs#L162)) stores the last-rendered height for each paragraph index and survives across frames.

**`load_document()`**

Clears `layout_cache`, `para_visual_heights`, `IMAGE_CACHE`, `MERMAID_CACHE`, `MERMAID_DIMS_CACHE` at [line 197](../desktop/patches/adabraka-ui/src/components/document_editor.rs#L197), then calls `cx.notify()`. The notify immediately queues a full paint pass with empty caches.

**`pull_and_open_file()` — current pipeline**

```
UI click → resolve_pull_params (UI thread)
         → background_executor: [keyring + HTTP pull + fs write + markdown parse]
         → this.update: load_document_parsed → cx.notify()
         → GPUI: full paint pass (shapes ALL paragraphs)
```

The background I/O is already non-blocking; the spike is entirely in the paint pass triggered by `cx.notify()` at the end.

**`github_sidebar.rs` — `list_entries()` re-scans disk every render**

`list_entries()` calls `std::fs::read_dir()` synchronously. Because `cx.notify()` during file-open triggers a full render tree walk, the sidebar re-scans the directory as a side-effect of every file-open operation.

### External Research

**Zed (`zed-industries/zed` — `crates/markdown`)**
- Uses `const REPARSE_DEBOUNCE: Duration = Duration::from_millis(200)` and `parsing_markdown_task: Option<Task<Result<()>>>` for a cancellable background parse.
- Renders via `list(self.list_state.clone(), cx.processor(...))` — GPUI's built-in virtualizing list; only visible elements participate in layout/paint.
- Increments a generation counter; stale task results are discarded.

**VS Code (TextMate tokenizer worker)**
- `TokenCache` keyed by `(uri, version, config_hash)` — if version matches, tokenization is skipped entirely.
- Tokenizer yields after every 20 ms so the UI thread is never starved for more than one frame.
- Runs in a dedicated `textMateTokenizationWorker.worker.ts` web worker.

**Key best-practice takeaways**
- Viewport culling + cached heights = shaping work proportional to visible area, not document length.
- Generation tokens prevent stale results from corrupting state on rapid switching.
- Debounced background parsing with cancellation handles edit-path latency (Phase 3 architecture borrows this for the open path).

---

## Architecture Considerations

The system after all four phases:

```
┌──────────────────────────────────────────────────────────────────────────────┐
│  UI Thread                                                                   │
│                                                                              │
│  Click row ──► set selected_file ──► cx.notify() ──► repaint (sidebar only) │
│             ──► bump load_generation                                         │
│             ──► cx.spawn(task[gen])                                          │
│                        │                                                     │
│  Frame N:  DocumentEditorElement::paint()                                    │
│            ├── accumulate heights for ALL paragraphs using para_visual_heights│
│            │   (O(n) integer add, no shaping)                                │
│            └── shape + draw only paragraphs in [scroll_top-overscan,        │
│                                                  scroll_bottom+overscan]     │
└──────────────────────────────────────────────────────────────────────────────┘
                               │ background_executor
┌──────────────────────────────▼───────────────────────────────────────────────┐
│  Background Task[gen]                                                        │
│  ├── (optional) GitHub pull + fs write                                       │
│  ├── markdown parse → Vec<DocParagraph>                                      │
│  └── if gen == current_gen → this.update: load_document_parsed              │
└──────────────────────────────────────────────────────────────────────────────┘
```

**SOLID alignment**

| Principle | Component | Responsibility |
|-----------|-----------|----------------|
| Single Responsibility | `DocumentEditorElement::paint()` | GPU draw calls for visible paragraphs only; height accumulation for all |
| Single Responsibility | `AppState::pull_and_open_file()` | Orchestrate async file open; owns generation counter |
| Open/Closed | `load_document_parsed()` | Two-phase protocol: accepts `Option<Vec<DocParagraph>>` — `None` = clear-only phase; callers extend behavior without modifying internals |
| Liskov Substitution | `GithubSidebar::entries_cache` | Any `Vec<ExplorerEntry>` satisfies the render contract; backing store is an impl detail |
| Interface Segregation | New `LoadGeneration` type alias `u64` | Single counter type used only by open pipeline; not entangled with sync status |
| Dependency Inversion | `DocumentEditorState` accepts pre-parsed paragraphs | Paint element depends on abstract paragraph slice; parser is injected from background task |

---

## Requirements

- [ ] GPUI scroll position is readable during `paint()` — verify `ScrollHandle` or `bounds` relative to parent provides `scroll_top` value
- [ ] `para_visual_heights` persists across `cx.notify()` calls and is only cleared by `load_document()` — confirmed at [line 203](../desktop/patches/adabraka-ui/src/components/document_editor.rs#L203)
- [ ] No other consumer of `layout_cache` relies on it containing entries for off-screen paragraphs (cursor navigation uses `layout_cache.iter().find(...)` — must still work post-culling)
- [ ] `AppState` is only mutated on the UI thread — confirmed; background tasks use `this.update(cx, ...)` which marshals back to UI thread

---

## Implementation Steps

### Phase 1: Viewport Virtualization in DocumentEditor

**Goal**: Shape and draw only the paragraphs visible in the current scroll window. Expected outcome: first paint after file-open completes in <16 ms regardless of document length.

**Files**: [document_editor.rs](../desktop/patches/adabraka-ui/src/components/document_editor.rs)

- [ ] **1.1** Determine how scroll position is exposed during `paint()`. Locate the `ScrollHandle` or equivalent that feeds into `bounds`. Confirm whether `bounds.top()` already reflects scroll offset or is always the physical top-left of the component.
- [ ] **1.2** Add a `viewport_top_px: f32` and `viewport_bottom_px: f32` derived from `bounds` and the parent scroll offset inside `paint()` entry. Define `OVERSCAN_PX: f32 = 300.0` (one screen of overscan above and below).
- [ ] **1.3** In `paint()`: replace the unconditional paragraph loop with a two-stage loop:
  1. **Height accumulation pass** (all paragraphs, O(n) addition only): use `para_visual_heights[para_idx]` when available; use a default height estimate (e.g., `LINE_HEIGHT_PX * 1.5`) for paragraphs not yet in the cache. Accumulate `current_y` for every paragraph to know each one's screen position.
  2. **Shape + draw pass** (visible paragraphs only): skip shape/draw when `current_y + para_h < viewport_top_px - OVERSCAN_PX` or `current_y > viewport_bottom_px + OVERSCAN_PX`. Still update `current_y` for all to keep positions correct.
- [ ] **1.4** Update `request_layout()` to use `para_visual_heights` for the total-height estimate instead of re-shaping every paragraph. Only shape paragraphs in the visible window during layout.
- [ ] **1.5** Verify cursor hit-testing (`hit_test_position`, table hit-test) still works — these read from `layout_cache` which stores absolute `top_px`, so they are unaffected by paint-time culling as long as visible paragraphs still write into `layout_cache`.
- [ ] **1.6** Smoke-test with a 500-line markdown document: confirm scroll is fluid and cursor placement is correct after scrolling.

```
Before:  paint(200 paragraphs) → shape 200 × N lines → O(document_size)
After:   height_accum(200 paragraphs) → O(200 additions)
         shape+draw(~15 visible paragraphs) → O(viewport_size)
```

---

### Phase 2: Two-Phase Document Open

**Goal**: The sidebar reflects the new active file on the very next frame after click. The editor area shows a blank/cleared state immediately, then content appears once parsed.

**Files**: [app_state.rs](../desktop/crates/octodocs-app/src/app_state.rs), [document_editor.rs](../desktop/patches/adabraka-ui/src/components/document_editor.rs)

- [ ] **2.1** Add a `clear_document(&mut self, path: PathBuf, cx)` method to `AppState` that:
  - Sets `self.document.path = Some(path)`
  - Calls `doc_editor.update(cx, |editor, _| editor.paragraphs.clear())` (or a new `editor.clear()` helper)
  - Calls `cx.notify()`
  - Does NOT touch `full_editor_state` (handled in phase-2 hydration)
- [ ] **2.2** At the top of `pull_and_open_file()`, call `self.clear_document(path.clone(), cx)` before spawning the background task. This makes the sidebar active state update on the current frame while parse work proceeds asynchronously.
- [ ] **2.3** Add a `clear()` convenience method to `DocumentEditorState` that resets paragraphs, cursor, selection, and layout caches without triggering a notify (the caller controls notify timing).
- [ ] **2.4** Verify that `GithubSidebar` derives the active-file indicator from `app_state.document.path` (not from `selected_file`). If it does, Phase 2.1 is sufficient to update the sidebar immediately.

---

### Phase 3: Cancellable Versioned Load Pipeline

**Goal**: Rapid file switching (e.g., clicking down the tree quickly) must not corrupt editor state with a stale result from an earlier click.

**Files**: [app_state.rs](../desktop/crates/octodocs-app/src/app_state.rs)

- [ ] **3.1** Add `load_generation: u64` field to `AppState`, initialized to `0`.
- [ ] **3.2** At the start of `pull_and_open_file()`, increment `self.load_generation` and capture it as `let expected_gen = self.load_generation` before spawning the background task.
- [ ] **3.3** The background task captures `expected_gen`. Before calling `this.update(cx, |state, cx| ...)`, check: if `state.load_generation != expected_gen`, log a trace message and return without calling `load_document_parsed`. This discards stale results silently.
- [ ] **3.4** Update the GitHub pull path, the local open path, and the error fallback path — all three branches inside `pull_and_open_file()` need the generation guard.
- [ ] **3.5** Ensure `_pull_task` replacement (the `self._pull_task = Some(cx.spawn(...))` assignment) still cancels any in-flight GPUI task, which GPUI does automatically when the `Task` handle is dropped. The generation counter is a second layer of defense for tasks that have already completed their async work but not yet called `this.update`.

---

### Phase 4: Sidebar Directory Scan Cache

**Goal**: Eliminate synchronous `read_dir()` disk I/O from the render path.

**Files**: [github_sidebar.rs](../desktop/crates/octodocs-app/src/views/github_sidebar.rs)

- [ ] **4.1** Add `entries_cache: Option<Vec<ExplorerEntry>>` field to `GithubSidebar` (or the relevant tree-row struct).
- [ ] **4.2** Move `list_entries()` (or equivalent `read_dir` call) out of `render()` / `push_tree_rows()`. Instead, call it once during `GithubSidebar::new()` and cache the result in `entries_cache`.
- [ ] **4.3** Add an `invalidate_entries_cache(&mut self)` method. Call it from:
  - `open_file_from_sidebar()` after a successful file open (directory may have changed if a pull created new files)
  - GitHub sync complete callback (new files may have been pulled)
  - Any future file-save path that creates new files
- [ ] **4.4** On invalidation, schedule a background re-scan via `cx.spawn(background_executor.spawn(read_dir(...)))` that writes back to `entries_cache` and calls `cx.notify()`. Do not block the UI thread on the re-scan.
- [ ] **4.5** During the re-scan window, render from stale cache (entries may be slightly outdated for <100 ms — acceptable).

---

## Testing Strategy

- [ ] **Unit**: Add a test in `octodocs-core` that parses a 500-paragraph markdown document and measures `DocParagraph` parse time — establishes baseline for Phase 2/3 validation.
- [ ] **Manual smoke — Phase 1**: Open a long document (>200 paragraphs). Scroll to bottom. Switch to another file. Verify: (a) no visible frame stutter, (b) cursor click at the top of the new file places correctly.
- [ ] **Manual smoke — Phase 2**: Click a file in the sidebar. Verify the sidebar row highlights on the same frame as the click (no loader icon delay for local files).
- [ ] **Manual smoke — Phase 3**: Click file A, immediately click file B (within 200 ms). Verify final state shows file B's content, not file A's.
- [ ] **Manual smoke — Phase 4**: Open a folder with 100 files. Switch between files rapidly. Verify no visible freeze in the sidebar tree during switches.
- [ ] **Regression**: All existing view-mode switching (WYSIWYG / Split / Source), toolbar formatting actions, and GitHub sync should continue to function correctly after each phase.

References:
- Internal: [desktop/crates/octodocs-core/src/](../desktop/crates/octodocs-core/src/) — unit test location

---

## Potential Risks & Mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| Cursor placed in wrong paragraph after viewport culling | High | Keep height accumulation for ALL paragraphs even when skipping draw; hit-tests read from `layout_cache` not from draw order |
| `para_visual_heights` contains stale heights after line wrap changes (window resize) | Medium | Clear `para_visual_heights` on `bounds.size` change, not just on `load_document()` |
| Phase 2 blank-flash noticeable on slow machines | Low | The cleared editor shows the background color, which matches the theme; identical to what a blank document looks like — not a visible flash |
| Generation counter overflow at `u64` | Negligible | `u64::MAX` ≈ 1.8 × 10¹⁹ switches — not a concern |
| Sidebar cache stale after external editor writes a new file | Low | Users can close/re-open the sidebar pane to force refresh; a file-watcher is out of scope |

---

## Dependencies

- No new external crates required.
- GPUI scroll position API: verify availability of `ScrollHandle::offset()` or equivalent during `paint()` — if not directly accessible, derive from `bounds.origin.y` relative to the window root.
- Existing: `para_visual_heights: Vec<f32>` — already in `DocumentEditorState`; no schema change needed.
- Existing: `_pull_task: Option<Task<()>>` in `AppState` — task-drop cancellation already in place.

---

## Success Criteria

- [ ] Switching between local markdown files feels instant (<100 ms perceived latency on a mid-range laptop).
- [ ] Switching to a GitHub-bound file shows the loader icon immediately on click; content appears within the same time as today (~1 s for GitHub pull), but the UI is never frozen.
- [ ] Rapid clicking through 5 files in <500 ms reliably ends on the last clicked file with correct content.
- [ ] Scrolling a 500-paragraph document is smooth (60 fps) with no per-scroll frame spikes from off-screen paragraph shaping.
- [ ] All existing editor features (toolbar formatting, view-mode switching, GitHub sync, autosave) continue to work correctly.
