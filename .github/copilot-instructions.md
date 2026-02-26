# OctoDocs — AI Agent Coding Instructions

## Repository layout

```
octodocs/
├── desktop/                  ← Cargo workspace (all Rust code lives here)
│   ├── Cargo.toml            ← workspace manifest; [patch.crates-io] overrides below
│   ├── crates/
│   │   ├── octodocs-core/    ← pure Rust, no UI (document model, renderer, Mermaid)
│   │   └── octodocs-app/     ← GPUI application (views, app state, toolbar)
│   └── patches/
│       ├── adabraka-gpui/    ← vendored GPUI fork (patched for GPU alignment + APIs)
│       ├── adabraka-ui/      ← vendored component library (patched for toolbar + icon APIs)
│       └── implemented-changes-and-rationale.md  ← why each patch exists
├── docs/plans/               ← implementation plans (source of truth for architecture)
└── .github/agents/           ← reusable agent instruction files
```

## Build & run

All commands must run from `desktop/` (the Cargo workspace root):

```bash
cd desktop
cargo build -p octodocs-app          # build the app
cargo run -p octodocs-app            # run the app
cargo test -p octodocs-core          # fast unit tests (no GPUI dependency)
cargo clippy --workspace             # lint
```

Nightly Rust is required — `desktop/rust-toolchain.toml` pins the channel automatically via `rustup`.

## Critical: vendored patches

`adabraka-ui` and `adabraka-gpui` are **not** fetched from crates.io — they are overridden by `[patch.crates-io]` in `desktop/Cargo.toml` to local paths under `desktop/patches/`. **Always edit the patch source**, never the crates.io cache. Key patches:
- `patches/adabraka-gpui/src/scene.rs` + `window.rs` — GPU shader padding (prevents Vulkan crash on startup)
- `patches/adabraka-ui/src/components/editor.rs` — added `wrap_selection()` and `insert_text()` for toolbar
- `patches/adabraka-ui/src/components/icon.rs` — icon color defaults to `foreground` (was invisible on dark themes)
- `patches/adabraka-ui/src/components/editor.rs` — added `place_cursor_at_end()` for block activation

## GPUI patterns

```rust
// State is held in Entity<T> — never stored directly in views
let state = cx.new(|cx| MyStruct::new(cx));

// Notify GPUI that state changed (triggers re-render of subscribed views)
cx.notify();

// Subscribe to another entity's changes
let sub = cx.observe(&entity, |this, _, cx| { cx.notify(); });

// Downgrade to a weak handle for use in closures (avoids reference cycles)
let weak = entity.downgrade();
let _ = weak.update(cx, |state, cx| { ... });
```

Views implement `Render` and use `use_theme()` for all colors — never hardcode values.

## WYSIWYG block editor architecture

The editor is a **single-pane** model (no split pane). State lives in `app_state.rs`:

- `blocks: Vec<DocumentBlock>` — document split into top-level blocks by `Renderer::parse_blocks()`
- `active_block: Option<usize>` — `None` means all blocks are rendered; `Some(i)` means block `i` is in edit mode
- `editor_state: Entity<EditorState>` — a **single shared editor** reused for whichever block is active

On activation (`activate_block(idx)`): block source is loaded into the shared editor and `place_cursor_at_end()` is called. On any editor change, the subscription in `AppState::new()` splices back the edited source, calls `DocumentBlock::reassemble()`, and sets `dirty = true`. `BlockEditorPane` (`views/block_editor_pane.rs`) renders each block: active → `Editor` widget; inactive → `render_node()` from `preview_pane.rs`.

## Mermaid rendering pipeline

`octodocs-core::mermaid::render_png()` is the only entry point. The pipeline:

```
source → mermaid_rs_renderer::render()  →  sanitize_svg_xml()  →  usvg::Tree::from_str()
      →  resvg::render() at 2× scale  →  pixmap.save_png()  →  /tmp/octodocs-mermaid-cache/{hash}.png
```

**Never use `gpui::svg()`** for Mermaid output — GPUI converts SVGs to a monochrome alpha mask (icon-only). Use `img(Arc<Path>)` with the rasterized PNG. `sanitize_svg_xml()` must be called before passing the SVG to `usvg` to fix unescaped `"` inside `font-family` XML attributes.

## File operations

File dialogs use `rfd::FileDialog` (not a GPUI native API). `MenuBar` was removed because the vendored version does not render dropdown lists — New/Open/Save/Save As are toolbar buttons in `root.rs`.

## Icons & assets

Icons load from `desktop/crates/octodocs-app/assets/icons/`. The custom `AssetSource` in `main.rs` resolves paths relative to that directory. Use Lucide SVG icons. Reference icons via their filename stem (e.g., `"bold"` → `bold.svg`).

## Core types quick reference

| Type | Location | Purpose |
|---|---|---|
| `Document` | `octodocs-core::document` | raw content + optional `PathBuf` |
| `DocumentBlock` | `octodocs-core::renderer` | `{ source: String, node: RenderNode }` |
| `RenderNode` | `octodocs-core::renderer` | enum: `Heading`, `Paragraph`, `MermaidBlock`, … |
| `AppState` | `octodocs-app::app_state` | single Entity shared by all views |
| `BlockEditorPane` | `views/block_editor_pane.rs` | main content view (WYSIWYG) |
| `RootView` | `views/root.rs` | top-level layout: toolbar + pane + status bar |
