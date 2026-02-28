# 🏆 Agents League — Project Submission

> **Issue title:** `Project: Creative Apps - OctoDocs`
> **Label:** `submission`
> **Submit at:** https://github.com/microsoft/agentsleague/issues/new?template=project.yml

---

## Track

**Creative Apps (GitHub Copilot)**

---

## Project Name

**OctoDocs**

---

## GitHub Username

**@elranu**

---

## Repository URL

**https://github.com/elranu/octodocs**

---

## Project Description

OctoDocs is a free, native desktop Markdown editor built in Rust that **automatically commits and pushes your documents to GitHub every time you press Ctrl+S** — no terminal, no Git knowledge required.

The core idea: your notes and documents should be as reliably backed up as your code. OctoDocs bridges that gap. You write in a WYSIWYG editor (Word/Docs-style, with live rendering of headings, bold, italic, code, etc.), hit Save, and the file is immediately committed to your own GitHub repository. Every save is a real Git commit with a timestamp and full diff — giving you infinite version history for free.

Key features:
- **WYSIWYG Markdown editor** — continuous document model, no raw syntax visible while writing
- **Auto-push on every save** — background GitHub commit + push, confirmed by an inline sync badge
- **Inline Mermaid diagrams** — flowcharts and sequence diagrams rendered natively (no Node.js, no browser)
- **File explorer sidebar** — browse, create, and organize `.md` files that live in your GitHub repo
- **Multiple repositories** — connect personal notes, team wikis, and project docs to different repos
- **Native Rust performance** — instant launch, minimal RAM, runs on Linux, macOS, and Windows

Built entirely with GitHub Copilot assistance throughout the development process.

---

## Demo Video or Screenshots

> ⚠️ **TODO before submitting:** Record a short screen capture (~2 min) showing:
> 1. Opening OctoDocs → file sidebar
> 2. Writing in WYSIWYG mode (bold, headings, checklist)
> 3. Pressing Ctrl+S → "Synced to GitHub" badge appears
> 4. Opening GitHub.com to show the commit was created
> 5. A Mermaid diagram rendering inline

```
Demo Video: https://youtu.be/TODO
Screenshots: https://github.com/elranu/octodocs/tree/main/assets
Live Demo: N/A (native desktop app)
```

---

## Primary Programming Language

**Other** (Rust)

---

## Key Technologies Used

- **Rust** — entire application, zero unsafe, compiled native binary
- **GPUI** (Zed's GPU UI framework, vendored fork) — hardware-accelerated UI rendering
- **GitHub REST API** — OAuth authentication, commit creation, push
- **mermaid-rs + resvg** — server-side Mermaid SVG rendering → PNG rasterization (2× scale)
- **rfd** — native file dialogs (open/save)
- **GitHub Copilot in VS Code** — AI-assisted development throughout; used for architecture planning, code generation, and patching vendored dependencies

---

## Submission Type

**Individual**

---

## Team Members

N/A — individual submission.

---

## Submission Requirements

- [x] My project meets the track-specific challenge requirements
- [x] My repository includes a comprehensive README.md with setup instructions
- [x] My code does not contain hardcoded API keys or secrets
- [x] I have included demo materials (video or screenshots)
- [x] My project is my own work with proper attribution for any third-party code
- [x] I agree to the [Code of Conduct](https://github.com/microsoft/agentsleague/blob/main/CODE_OF_CONDUCT.md)
- [x] I have read and agree to the [Disclaimer](https://github.com/microsoft/agentsleague/blob/main/DISCLAIMER.md)
- [x] My submission does NOT contain any confidential, proprietary, or sensitive information
- [x] I confirm I have the rights to submit this content and grant the necessary licenses

---

## Quick Setup Summary

```
1. Install Rust nightly (rustup handles this automatically via rust-toolchain.toml)
2. Clone the repo:  git clone https://github.com/elranu/octodocs
3. cd octodocs/desktop
4. Run:  cargo run -p octodocs-app
5. On first launch, complete GitHub OAuth in the browser
6. Pick or create a GitHub repo — start writing
```

Full instructions: see [README.md](https://github.com/elranu/octodocs/blob/main/README.md)

---

## Technical Highlights

- **Vendored GPUI patches** — forked and patched Zed's GPUI to fix GPU shader padding that caused Vulkan crashes on startup (Linux/Nvidia). The patch adds correct alignment to scene buffers in `scene.rs`.
- **Continuous WYSIWYG document model** — implemented a Word/Docs-style single-pane editor (`DocumentEditorState`) with inline span formatting, cursor tracking, and live markdown serialization — no "preview pane" split needed.
- **Zero-dependency Mermaid rendering** — built a fully self-contained rendering pipeline: `mermaid-rs` renders SVG → `sanitize_svg_xml()` fixes unescaped XML attributes → `usvg` parses → `resvg` rasterizes at 2× scale → PNG cached to `/tmp`. No Node.js, no Chromium, no network calls.
- **GitHub auto-sync architecture** — every `Ctrl+S` triggers an async background task that creates a Git tree, commit object, and updates the branch ref via GitHub's REST API. The UI shows a live "Synced" badge without blocking the editor.
- **GitHub Copilot-driven development** — the entire codebase was built with Copilot in VS Code, including architecture decisions documented in `.github/copilot-instructions.md` and `docs/plans/`.

---

## Challenges & Learnings

**Biggest challenge: making GPUI work outside of Zed.**
GPUI is not published as a stable library — it's tightly coupled to Zed's internal build. Getting it to compile as a standalone dependency required vendoring the crate, patching GPU shader buffer alignment for Vulkan (Zed targets Metal/DirectX primarily), and fixing icon rendering so colors were visible on dark themes (the original `icon.rs` defaulted to transparent foreground).

**Key learnings:**
- GPU buffer alignment bugs are silent on some hardware but crash immediately on others — always test on Vulkan/Linux early.
- Building a WYSIWYG editor from scratch (even a simple one) is an order of magnitude harder than a source-text editor. Cursor semantics, inline span merging, and undo state all interact in non-obvious ways.
- GitHub Copilot as a pair programmer meaningfully accelerated the hard parts: it generated the Mermaid pipeline boilerplate, suggested the `sanitize_svg_xml` fix, and drafted most of the GitHub API integration code.

---

## Contact Information

> Fill in before submitting: `email@example.com`

---

## Country/Region

**Argentina**
