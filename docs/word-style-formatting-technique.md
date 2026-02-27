# Word-Style Formatting Technique (WYSIWYG)

## Purpose

Document the technique used to make inline formatting reliable in the word-style editor when markdown state updates correctly but visual emphasis is inconsistent across platforms.

## Problem Pattern

Observed behavior:
- Toolbar actions produced correct markdown in source mode (for example `**text**`),
- but the WYSIWYG surface did not always show visible bold/italic emphasis.

This indicates a **render-path visibility issue**, not a state-mutation issue.

## Technique Summary

1. **Prove state correctness first**
   - Verify selection range is preserved through mouse-up and toolbar click.
   - Verify `toggle_format` executes with expected `(start, end)`.
   - Verify serialized markdown reflects the expected format markers.

2. **Separate data correctness from visual correctness**
   - If source markdown is correct, treat remaining failures as rendering/face-resolution issues.

3. **Use deterministic visual emphasis in render runs**
   - Keep semantic inline format in model (`InlineFormat`).
   - In `spans_to_text_runs`, apply explicit render emphasis for formatted runs (for this environment, via emphasis color), so feedback is always visible regardless of font backend differences.

4. **Keep markdown output stable**
   - Coalesce adjacent spans with identical `InlineFormat` before serialization.
   - This prevents repeated delimiters and noisy source output after multiple toggle operations.

5. **Round-trip additional formats explicitly**
   - Extend core model with `Underline` and `Strikethrough`.
   - Parse and serialize these formats in core conversion paths.
   - Wire toolbar actions and preview rendering consistently.

## Concrete Implementation Points

### 1) Editor interaction correctness

File:
- [desktop/patches/adabraka-ui/src/components/document_editor.rs](../desktop/patches/adabraka-ui/src/components/document_editor.rs)

Key idea:
- Formatting logic must run only on a real selection range and preserve expected cursor/selection transitions.

### 2) Rendering emphasis for formatted runs

File:
- [desktop/patches/adabraka-ui/src/components/document_editor.rs](../desktop/patches/adabraka-ui/src/components/document_editor.rs)

Key idea:
- Build `TextRun` values from inline spans.
- For formatted runs (`Bold`, `Italic`, `Underline`, `Strikethrough`), use visible emphasis styling in run attributes so WYSIWYG always reflects formatting state.

### 3) Serialization normalization

File:
- [desktop/crates/octodocs-core/src/doc_model.rs](../desktop/crates/octodocs-core/src/doc_model.rs)

Key idea:
- Merge adjacent equal-format spans before markdown serialization.
- Then serialize once per merged span (for example `**merged text**` instead of split repeated markers).

### 4) Parsing and round-trip extensions

File:
- [desktop/crates/octodocs-core/src/renderer.rs](../desktop/crates/octodocs-core/src/renderer.rs)

Key idea:
- Extend inline parse state for additional formats (`Strikethrough`, simple underline tag handling).
- Map parsed nodes into doc model formats and back to markdown.

### 5) Toolbar and preview parity

Files:
- [desktop/crates/octodocs-app/src/views/root.rs](../desktop/crates/octodocs-app/src/views/root.rs)
- [desktop/crates/octodocs-app/src/views/preview_pane.rs](../desktop/crates/octodocs-app/src/views/preview_pane.rs)

Key idea:
- Every format exposed in WYSIWYG toolbar must have corresponding preview rendering and core round-trip support.

## Why This Works

- It isolates correctness layers:
  - Selection + mutation correctness,
  - Serialization correctness,
  - render visibility correctness.
- It avoids overfitting to platform font-face behavior by making formatted state visually explicit in the editor render layer.
- It keeps markdown authoritative and stable.

## Tradeoff

- Render emphasis can be stronger than pure typographic weight/style on some platforms.
- This is intentional to guarantee usability where font-face resolution does not produce clear visual contrast.

## Validation Checklist

- Select range -> click toolbar format -> visible change in WYSIWYG.
- Switch to source mode -> expected markdown markers present.
- Repeated toggles do not generate delimiter noise.
- Reopen document -> formatting round-trips and renders consistently.
