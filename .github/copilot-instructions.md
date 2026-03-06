# OctoDocs — AI Agent Coding Instructions

> **CRITICAL — Release discipline:** Never run `git push`, `git tag`, or
> trigger a release without **explicit user approval**. Prepare commits locally
> (version bump, Cargo.lock, code changes) and stop to ask the user before
> pushing or creating a tag.

## Repository layout

```
octodocs/
├── desktop/                  ← Cargo workspace (all Rust code lives here)
│   ├── Cargo.toml            ← workspace manifest; [patch.crates-io] overrides below
│   ├── crates/
│   │   ├── octodocs-core/    ← pure Rust, no UI (document model, renderer, Mermaid)
│   │   ├── octodocs-github/  ← GitHub API client (auth, sync, pull, discovery)
│   │   └── octodocs-app/     ← GPUI application (views, app state, toolbar)
│   └── patches/
│       ├── adabraka-gpui/    ← vendored GPUI fork (patched for GPU alignment + APIs)
│       ├── adabraka-ui/      ← vendored component library (patched for toolbar + icon APIs)
│       └── implemented-changes-and-rationale.md  ← why each patch exists
├── install.sh                ← Linux (XDG .desktop + icon) and macOS (.app bundle) installer
│                               also: --version / --update flags on the installed wrapper
├── desktop/resources/windows/octodocs.iss  ← Inno Setup script → OctoDocs-Setup-x86_64.exe
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
- `patches/adabraka-ui/src/components/document_editor.rs` — continuous WYSIWYG document editor state + renderer
- `patches/adabraka-ui/src/components/icon.rs` — icon color defaults to `foreground` (was invisible on dark themes)
- `patches/adabraka-ui/src/components/editor.rs` — source editor improvements used in Source/Split modes

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

## WYSIWYG continuous editor architecture

The editor is a **single-pane continuous document** model (Word/Docs-style). State lives in `app_state.rs`:

- `doc_editor: Entity<DocumentEditorState>` — one entity for the full WYSIWYG document
- `full_editor_state: Entity<EditorState>` — raw markdown editor used for Source/Split modes

`DocumentEditorState` owns document paragraphs, cursor, selection, and layout cache. Toolbar actions in `root.rs` call `doc_editor.update(...)` directly (`toggle_bold`, `toggle_italic`, `toggle_underline`, `toggle_strikethrough`, `toggle_code`, heading changes).

On editor changes, the subscription in `AppState::new()` serializes `doc_editor` back to markdown (`to_markdown()`), updates `document.content`, and marks `dirty = true`.

The main WYSIWYG view is `DocumentEditorPane` (`views/document_editor_pane.rs`), which hosts `DocumentEditor` from `patches/adabraka-ui/src/components/document_editor.rs`.

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

## GitHub sync (`octodocs-github`)

`octodocs-github` is a blocking HTTP crate (uses `reqwest` blocking + `rustls-tls`). Key public API:

```rust
octodocs_github::get_stored_token()           // reads PAT from OS keyring
octodocs_github::pull_file(&token, &config, &filename)  // fetch latest from GitHub
octodocs_github::sync_file(&token, &config, &filename, content, images)  // push to GitHub
octodocs_github::discover_repos(&token)       // list accessible repos + branches
```

`GitHubSyncConfig` (owner, repo, branch, folder) is stored in `~/.config/octodocs/github_bindings.tsv`. Auth uses a GitHub PAT stored in the OS keyring via `keyring`. All sync/pull calls run on the `background_executor` so they never block the UI thread.

## Auto-update (`updater.rs`)

`crates/octodocs-app/src/updater.rs` contains the entire update logic:

- `check_for_update()` — blocking call; hits `https://api.github.com/repos/elranu/octodocs/releases/latest`, compares `tag_name` against `env!("CARGO_PKG_VERSION")`, returns `Some(tag)` when newer.
- `launch_update(tag)` — platform-specific:
  - **Linux/macOS**: spawns `sh -c "curl -fsSL install.sh | sh"` in the background.
  - **Windows**: downloads `OctoDocs-Setup-x86_64.exe` to `%TEMP%`, launches with `/verysilent /update=true`, then `std::process::exit(0)`.

`AppState` runs `check_for_update()` 3 s after startup (background executor). If a newer version exists, `update_available: Option<String>` is set and a blue banner appears between the toolbar and the editor. Actions: **Update now** → `trigger_update()`, **×** → `dismiss_update()`.

## Installers & release convention

- **Linux/macOS**: `install.sh` — one-curl install. Installs binary to `~/.local/share/octodocs/`, creates XDG `.desktop` entry + icon, registers a wrapper at `~/.local/bin/octodocs` supporting `--version` / `--update`.
- **Windows**: `desktop/resources/windows/octodocs.iss` — Inno Setup script compiled by `ISCC.exe` (pre-installed on `windows-2022` GitHub Actions runners). Produces `OctoDocs-Setup-x86_64.exe` with wizard UI, Start Menu, optional Desktop shortcut, optional PATH, `.md`/`.markdown` association, uninstaller. `PrivilegesRequired=lowest` (no admin needed).
- **Release CI** (`.github/workflows/release.yml`): triggered by `v*` tags. Builds all three platforms, packages Linux as `.tar.gz`, macOS as `.dmg`, Windows as installer `.exe` + portable `.exe`, then creates a GitHub release.
- **Versioning**: `desktop/crates/octodocs-app/Cargo.toml` `version` field **must match the git tag** (e.g. tag `v0.1.9` ↔ `version = "0.1.9"`). `env!("CARGO_PKG_VERSION")` is used for the status bar display and update comparison — bump Cargo.toml before tagging.

## Core types quick reference

| Type | Location | Purpose |
|---|---|---|
| `Document` | `octodocs-core::document` | raw content + optional `PathBuf` |
| `DocParagraph` | `octodocs-core::doc_model` | continuous editor paragraph + inline spans |
| `InlineFormat` | `octodocs-core::doc_model` | `Plain`, `Bold`, `Italic`, `Underline`, `Strikethrough`, `Code` |
| `RenderNode` | `octodocs-core::renderer` | enum: `Heading`, `Paragraph`, `MermaidBlock`, … |
| `AppState` | `octodocs-app::app_state` | single Entity shared by all views |
| `ViewMode` | `octodocs-app::app_state` | `Wysiwyg` / `Split` / `Source` |
| `UpdateStatus` | `octodocs-app::app_state` | `Idle` / `Downloading` / `Done` |
| `GitHubSyncBinding` | `octodocs-app::app_state` | local root path + `GitHubSyncConfig` |
| `GitHubSyncConfig` | `octodocs-github` | owner, repo, branch, folder |
| `SyncStatus` | `octodocs-github` | `Idle` / `Syncing` / `Success` / `Failed` |
| `DocumentEditorState` | `adabraka-ui::components::document_editor` | WYSIWYG state + editing operations |
| `DocumentEditorPane` | `views/document_editor_pane.rs` | main content view (WYSIWYG) |
| `RootView` | `views/root.rs` | top-level layout: toolbar + pane + status bar |
