# Implemented Changes and Rationale

This document summarizes the key implementation changes made in OctoDocs and explains why each one was necessary.

## 1) GPU shader alignment fix (runtime crash prevention)

**What changed**
- Added shader padding fields in GPUI patched sources:
  - `desktop/patches/adabraka-gpui/src/scene.rs`
  - `desktop/patches/adabraka-gpui/src/window.rs`

**Why**
- The app crashed at runtime due to CPU struct layout not matching WGSL alignment rules.
- Adding explicit padding ensures CPU-side data matches GPU-side shader expectations, preventing Vulkan/WGSL panic on startup.

---

## 2) Editor API extension for toolbar actions

**What changed**
- Exposed `insert_text` as public and added `wrap_selection` in:
  - `desktop/patches/adabraka-ui/src/components/editor.rs`

**Why**
- Toolbar formatting actions (bold, italic, inline code, headings) needed to edit selection/cursor text.
- Without these APIs, buttons could be wired visually but could not perform document transformations.

---

## 3) Icon default color fix (visibility on dark backgrounds)

**What changed**
- Changed icon fallback color from `primary` to `foreground` in:
  - `desktop/patches/adabraka-ui/src/components/icon.rs`

**Why**
- On some theme combinations, icons rendered black on black.
- Using `foreground` guarantees contrast consistency with active theme tokens.

---

## 4) File actions moved to toolbar (workaround for upstream menu limitation)

**What changed**
- Implemented New/Open/Save/Save As as working toolbar buttons in:
  - `desktop/crates/octodocs-app/src/views/root.rs`

**Why**
- The vendored MenuBar implementation toggled active state but did not render an actual dropdown list.
- Moving file actions to toolbar delivered reliable functionality immediately.

---

## 5) Icon asset path correction

**What changed**
- Added icon assets under crate-local path used by app assets:
  - `desktop/crates/octodocs-app/assets/icons/`

**Why**
- Asset loading is relative to the app crate manifest base.
- Icons placed only in `desktop/assets/icons/` were not being resolved in runtime for this binary.

---

## 6) Startup theme now follows system appearance

**What changed**
- Window initialization now detects system appearance and applies light/dark at startup:
  - `desktop/crates/octodocs-app/src/main.rs`
- Root view receives initial dark/light state for consistent theme toggle behavior:
  - `desktop/crates/octodocs-app/src/views/root.rs`

**Why**
- The app previously always forced dark theme.
- This now matches user/system preference on launch.

---

## 7) Mermaid preview formatting improvement

**What changed**
- Mermaid source preview now renders line-by-line and preserves indentation:
  - `desktop/crates/octodocs-app/src/views/preview_pane.rs`

**Why**
- Single text rendering collapsed line formatting/spacing in practice.
- Line-by-line rendering keeps diagram source readable and faithful.

> Note: current implementation displays Mermaid source block (formatted), not SVG diagram rendering yet.

---

## 8) App warning cleanup (dead code removal)

**What changed**
- Removed unused fields/enum/methods from app state:
  - `desktop/crates/octodocs-app/src/app_state.rs`

**Why**
- Eliminated dead code warnings in app crate builds.
- Reduced noise during development and improved maintainability.

---

## 9) Dependency strategy migrated to small-repo model

**What changed**
- Removed full vendored source replacement config:
  - deleted `desktop/.cargo/config.toml`
- Added targeted local crate overrides:
  - `desktop/Cargo.toml` with `[patch.crates-io]` for:
    - `adabraka-ui`
    - `adabraka-gpui`
- Kept only patched crates under:
  - `desktop/patches/`

**Why**
- Full `vendor/` caused very large commit size.
- This keeps repository small while preserving required local fixes.

---

## 10) Git ignore hygiene

**What changed**
- Created/updated root ignore rules in:
  - `.gitignore`
- Added ignores for build artifacts and local files (`target`, logs, screenshot patterns, etc.).

**Why**
- Prevent accidental commits of generated files.
- Keep history focused on source and intentional assets.

---

## Current summary

- App launches and runs.
- Editor + live preview are functional.
- Toolbar actions and file actions work.
- Theme follows system preference at startup.
- Repository is now configured for small, maintainable commits.

---

## 11) macOS build fix — core-graphics version unification

**What changed**
- Bumped `core-graphics` dependency in `adabraka-gpui/Cargo.toml` from `"0.24"` to `"0.25"` for the macOS target.
- `zed-font-kit/Cargo.toml` was already on `"0.25"` (no change needed there).
- Updated `Cargo.lock` via `cargo fetch` so `adabraka-gpui` resolves to `core-graphics 0.25.0`.

**Why**
- `adabraka-gpui`'s `text_system.rs` creates `CGFont`/`CGContext` objects via its own `core_graphics` crate dependency.
- `zed-font-kit` (the font rendering backend) also depends on `core-graphics` but at version `0.25`.
- `core-text 21.1.0` (another adabraka-gpui dep) also requires `core-graphics ^0.25`.
- Because both `0.24` and `0.25` were in the dependency graph, Rust treated `CGFont` from one as a different type from the other — producing E0308 mismatched type errors at the two cross-crate call sites in `text_system.rs` (lines 200 and 418).
- `cocoa 0.26.1` still requires `core-graphics ^0.24` (unchanged); it continues to use `0.24.0` for its own internal types, which never interact with `text_system.rs` types.
- After the bump, `text_system.rs` creates `CGFont`/`CGContext` from `0.25.0`, matching `zed-font-kit`'s expectation, resolving both errors.
