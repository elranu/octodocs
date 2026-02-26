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
    let scale = (MAX_WIDTH_PX / natural_w).min(HIDPI_SCALE).max(0.5);
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
fn sanitize_svg_xml(svg: &str) -> String {
    // Replace inner double-quoted font names like "Segoe UI" with single-quoted ones.
    // We scan for `="`  which starts an XML attribute value, then within the value
    // we replace any `"` that does NOT immediately precede [ />space] with `'`.
    let mut out = String::with_capacity(svg.len());
    let bytes = svg.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        let ch = bytes[i] as char;
        if ch == '=' && i + 1 < len && bytes[i + 1] == b'"' {
            out.push('=');
            out.push('"');
            i += 2; // skip past `="`
            // now consume the attribute value until the real closing quote
            while i < len {
                let b = bytes[i];
                if b == b'"' {
                    // determine if this is the closing quote:
                    // it's closing if the next non-consumed char is space, /, > or EOF
                    let next = bytes.get(i + 1).copied();
                    let is_closing = matches!(next, Some(b' ') | Some(b'/') | Some(b'>') | None);
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
                    out.push(b as char);
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
