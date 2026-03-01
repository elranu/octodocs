use std::path::Path;

/// Maximum output width in pixels. Diagrams wider than this are scaled down;
/// narrower diagrams are rendered at 2× for crispness.
const MAX_WIDTH_PX: f32 = 900.0;
/// HiDPI multiplier used when the diagram's natural width is smaller than MAX_WIDTH_PX.
const HIDPI_SCALE: f32 = 2.0;

/// Render a Mermaid diagram source to a PNG file at `output`.
///
/// The PNG is rasterized at up to `HIDPI_SCALE`× density for crispness, capped so
/// the physical width never exceeds `MAX_WIDTH_PX` pixels.
///
/// Returns the **logical** display size `(width, height)` in CSS-style pixels — i.e.
/// the natural SVG size, which is what the UI should use when sizing the `<img>`.
pub fn render_png(source: &str, output: &Path) -> anyhow::Result<(f32, f32)> {
    let svg = mermaid_rs_renderer::render(source)?;
    let svg = sanitize_svg_xml(&svg);

    // Parse with usvg so we know the natural diagram dimensions.
    let mut opt = usvg::Options::default();
    opt.fontdb_mut().load_system_fonts();
    let tree = usvg::Tree::from_str(&svg, &opt)?;

    // Natural (logical) size — what the UI will display at.
    let natural_w = tree.size().width();
    let natural_h = tree.size().height();

    // Compute physical raster scale: aim for HIDPI_SCALE× but cap at MAX_WIDTH_PX.
    let scale = (MAX_WIDTH_PX / natural_w).clamp(0.5, HIDPI_SCALE);
    let out_w = ((natural_w * scale).round() as u32).max(1);
    let out_h = ((natural_h * scale).round() as u32).max(1);

    let mut pixmap = resvg::tiny_skia::Pixmap::new(out_w, out_h)
        .ok_or_else(|| anyhow::anyhow!("Failed to allocate {}×{} pixmap", out_w, out_h))?;
    pixmap.fill(resvg::tiny_skia::Color::WHITE);

    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    pixmap.save_png(output).map_err(|e| anyhow::anyhow!("PNG write failed: {e}"))?;

    Ok((natural_w, natural_h))
}

/// Fix SVG strings produced by mermaid-rs-renderer that contain unescaped
/// double quotes inside XML attribute values — e.g. font-family values like
/// `"Segoe UI"` which break strict XML parsers (usvg).
///
/// Operates on Unicode chars (not raw bytes) to correctly preserve multi-byte
/// UTF-8 text content (e.g. →, …, ✅ that appears in diagram labels).
fn sanitize_svg_xml(svg: &str) -> String {
    let chars: Vec<char> = svg.chars().collect();
    let len = chars.len();
    let mut out = String::with_capacity(svg.len());
    let mut i = 0;

    while i < len {
        let ch = chars[i];
        if ch == '=' && i + 1 < len && chars[i + 1] == '"' {
            out.push('=');
            out.push('"');
            i += 2; // skip past `="`
            // consume the attribute value until the real closing quote
            while i < len {
                let c = chars[i];
                if c == '"' {
                    // it's the closing quote if the next char is space, /, > or EOF
                    let is_closing = matches!(
                        chars.get(i + 1).copied(),
                        Some(' ') | Some('/') | Some('>') | None
                    );
                    if is_closing {
                        out.push('"');
                        i += 1;
                        break;
                    } else {
                        // inner unescaped quote — replace with single quote
                        out.push('\'');
                        i += 1;
                    }
                } else {
                    out.push(c);
                    i += 1;
                }
            }
        } else {
            out.push(ch);
            i += 1;
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::{render_png, sanitize_svg_xml};

    #[test]
    fn sanitize_fixes_unescaped_font_family_quotes() {
        let input = r#"<text font-family="Inter, ui-sans-serif, -apple-system, "Segoe UI", sans-serif" fill="red">hi</text>"#;
        let fixed = sanitize_svg_xml(input);
        assert!(
            !fixed.contains(r#""Segoe UI""#),
            "inner double-quotes should be replaced: {fixed}"
        );
        assert!(
            fixed.contains("'Segoe UI'"),
            "should use single-quotes for Segoe UI: {fixed}"
        );
    }

    #[test]
    fn sanitize_preserves_unicode_in_text_content() {
        // Mermaid labels with multi-byte UTF-8: arrow (→), ellipsis (…), emoji (✅)
        // These appeared as garbled characters before the fix (byte-by-byte processing
        // corrupted multi-byte sequences into individual Latin-1 chars).
        let input = r#"<text font-family="Inter, "Segoe UI", sans-serif">show "Syncing…" → ✅</text>"#;
        let fixed = sanitize_svg_xml(input);
        assert!(fixed.contains("'Segoe UI'"), "font quotes must be fixed: {fixed}");
        assert!(fixed.contains("Syncing…"), "ellipsis (U+2026) must survive: {fixed}");
        assert!(fixed.contains('→'), "arrow (U+2192) must survive: {fixed}");
        assert!(fixed.contains('✅'), "emoji (U+2705) must survive: {fixed}");
    }

    #[test]
    fn renders_sequence_diagram_with_unicode_labels() {
        // The actual github-sync.md diagram contains →, …, ✅ in labels.
        // Before the fix, sanitize_svg_xml corrupted these to garbled chars in the PNG.
        let input = "sequenceDiagram\n    participant App\n    participant GitHub\n    App->>GitHub: POST /git/blobs (content → blob SHA)\n    GitHub-->>App: 200 OK ✅";
        let dir = std::env::temp_dir().join("octodocs-mermaid-test");
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("test_unicode_seq.png");
        let _ = std::fs::remove_file(&out);
        let (lw, lh) = render_png(input, &out).expect("unicode sequence diagram should render");
        assert!(out.exists(), "PNG must be written");
        assert!(lw > 0.0 && lh > 0.0, "logical dimensions must be positive: {lw}×{lh}");
    }

    #[test]
    fn renders_basic_flowchart_to_png() {
        let input = "graph TD\n    A[Start] --> B[Edit Markdown]\n    B --> C[Preview updates]";
        let dir = std::env::temp_dir().join("octodocs-mermaid-test");
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("test_flowchart.png");
        let (lw, lh) = render_png(input, &out).expect("mermaid renderer should produce png");
        assert!(out.exists());
        let bytes = std::fs::read(&out).unwrap();
        // PNG magic bytes: \x89PNG
        assert_eq!(&bytes[1..4], b"PNG");
        // Logical size must be positive
        assert!(lw > 0.0 && lh > 0.0, "logical size should be positive: {lw}×{lh}");
    }
}
