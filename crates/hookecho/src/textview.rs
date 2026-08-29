//! NWS text products, rendered as typeset HTML instead of a wall of monospace.
//!
//! An AFD or a hurricane discussion is prose that was hard-wrapped to 68 columns for a teletype in
//! 1985. Every reader since has had to look at it that way: fixed pitch, a line break every 68
//! characters whether or not a sentence ended there, and no visible difference between a section
//! header and the paragraph under it. It is legible; it is not readable.
//!
//! So the same text goes through a webview — the platform's, not one we ship. Prose blocks get
//! unwrapped back into real paragraphs and set in the system's UI face; the parts where the column
//! alignment is the content (wind tables, coordinate summaries) stay monospace and stay exactly as
//! the forecaster spaced them. The egui monospace view stays: it is the web build's only option and
//! the fallback anywhere the handoff fails.
//!
//! The document is local and inert — a `Content-Security-Policy` of `default-src 'none'`, no
//! script, no network, no navigation. The text is remote input, so it is HTML-escaped on the way
//! in and nothing in it is ever turned into a link.
//!
//! ponytail: `wry` was the plan and is not what shipped. On Linux its webview is webkit2gtk inside
//! a GTK window driven by a GTK main loop; eframe is winit, one event loop, one main thread, and
//! the two do not share it — parenting a wry view into our window there needs a second process,
//! and packaging needs gtk3 + webkit2gtk as hard runtime dependencies of a radar app, to read a
//! forecast discussion. The browser the user already has renders the same bundled template with no
//! dependency at all. Android is the case that genuinely needs its own view, and gets one.

/// Turn a plain-text product into a self-contained HTML document.
///
/// `title` and `issued` head the page; `text` is the raw product.
pub fn document(title: &str, issued: &str, text: &str) -> String {
    format!(
        "<!DOCTYPE html>\n<html lang=\"en\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'\">\
<title>{}</title><style>{}</style></head><body>\
<header><h1>{}</h1><p class=\"issued\">{}</p></header><main>{}</main></body></html>\n",
        esc(title),
        CSS,
        esc(title),
        esc(issued),
        body(text)
    )
}

const CSS: &str = "\
:root { color-scheme: light dark; --fg: #16181d; --dim: #5c6373; --bg: #fbfbfd; --rule: #d9dce4; }
@media (prefers-color-scheme: dark) { :root { --fg: #e6e8ee; --dim: #969db0; --bg: #14161b; --rule: #2c313b; } }
body { margin: 0 auto; padding: 2rem 1.25rem 5rem; max-width: 42rem; background: var(--bg); color: var(--fg);
  font: 1.0625rem/1.6 system-ui, -apple-system, 'Segoe UI', Roboto, sans-serif; }
header { border-bottom: 1px solid var(--rule); margin-bottom: 1.5rem; }
h1 { font-size: 1.5rem; margin: 0 0 .25rem; }
.issued { color: var(--dim); margin: 0 0 1rem; font-size: .9375rem; }
h2 { font-size: 1.0625rem; letter-spacing: .04em; text-transform: none; margin: 2rem 0 .5rem; }
p { margin: 0 0 1rem; }
pre { font: .875rem/1.45 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; overflow-x: auto;
  margin: 0 0 1rem; white-space: pre; }
hr { border: 0; border-top: 1px solid var(--rule); margin: 2rem 0; }
";

/// Render the product's blocks. Blank lines separate blocks; what a block becomes depends on what
/// it looks like, because the products carry no markup to ask.
fn body(text: &str) -> String {
    let mut out = String::new();
    for block in text.replace('\r', "").split("\n\n") {
        let lines: Vec<&str> = block.lines().filter(|l| !l.trim().is_empty()).collect();
        if lines.is_empty() {
            continue;
        }
        // `&&` and `$$` are the products' own section and end-of-product markers.
        if lines.iter().all(|l| matches!(l.trim(), "&&" | "$$")) {
            out.push_str("<hr>");
            continue;
        }
        if lines.len() == 1 {
            if let Some(h) = heading(lines[0]) {
                out.push_str(&format!("<h2>{}</h2>", esc(&h)));
                continue;
            }
        }
        if prose(&lines) {
            out.push_str(&format!("<p>{}</p>", esc(&unwrap_lines(&lines))));
        } else {
            // The spacing is the content here: a wind table, a coordinate summary, a hazard list.
            out.push_str(&format!("<pre>{}</pre>", esc(lines.join("\n").trim_end())));
        }
    }
    out
}

/// A section header, if the line is one. AFDs mark them with a leading dot and trailing ellipsis
/// (`.SHORT TERM...`); advisories just shout a short all-caps line.
fn heading(line: &str) -> Option<String> {
    let t = line.trim();
    if t.is_empty() || t.len() > 72 {
        return None;
    }
    let dotted = t.starts_with('.') && t.len() > 1 && !t.starts_with("..");
    let shouted = t.chars().any(|c| c.is_ascii_uppercase())
        && !t.chars().any(|c| c.is_ascii_lowercase())
        && t.len() <= 48;
    if !dotted && !shouted {
        return None;
    }
    // A shouted line that reads like a sentence or a data line is not a header.
    if !dotted && (t.contains("...") || t.ends_with('.')) {
        return None;
    }
    Some(
        t.trim_start_matches('.')
            .trim_end_matches('.')
            .trim()
            .to_string(),
    )
}

/// Was this block hard-wrapped prose? Every line but the last runs near the 68-column margin,
/// which is what a wrapper does and what a human writing a table never does.
fn prose(lines: &[&str]) -> bool {
    lines.len() >= 2
        && lines[..lines.len() - 1]
            .iter()
            .all(|l| l.trim_end().len() >= 55 && !l.starts_with(' '))
        && !lines.iter().any(|l| l.trim().contains("  "))
}

/// Undo the teletype wrap: the line breaks were never in the sentences.
fn unwrap_lines(lines: &[&str]) -> String {
    lines.iter().map(|l| l.trim()).collect::<Vec<_>>().join(" ")
}

fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

/// Hand `text` to a real renderer: Android's WebView, or the desktop's default browser reading a
/// file we just wrote. `Err` means the caller keeps showing its own monospace view, which it was
/// showing anyway.
pub fn open(title: &str, issued: &str, text: &str) -> Result<(), String> {
    let html = document(title, issued, text);
    #[cfg(target_os = "android")]
    {
        crate::platform::open_textview(title, &html)
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = html;
        Err("no reader on the web build".into())
    }
    #[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
    {
        let dir = crate::paths::cache_dir().ok_or("no cache directory")?;
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        // One file per product kind, overwritten: a reader tab that is already open on the last
        // AFD should show this one, and a cache directory should not fill with discussions.
        let name: String = title
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        let path = dir.join(format!("{name}.html"));
        std::fs::write(&path, html).map_err(|e| e.to_string())?;
        crate::platform::open_url(&format!("file://{}", path.display()))
    }
}

/// Is there a reader to hand off to? The web build has no file to open and no activity to start.
pub fn available() -> bool {
    !cfg!(target_arch = "wasm32")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapped_prose_becomes_one_paragraph() {
        let text =
            "A line of forecaster prose that runs right out to the teletype margin and then\n\
                    keeps going onto a second line before it finally stops.";
        let html = body(text);
        assert!(html.starts_with("<p>"), "{html}");
        assert!(html.contains("margin and then keeps going"), "{html}");
    }

    #[test]
    fn aligned_blocks_keep_their_spacing() {
        let text = "LOCATION...25.1N  79.2W\nMAXIMUM SUSTAINED WINDS...75 MPH";
        assert!(body(text).contains("<pre>"));
        assert!(body(text).contains("25.1N  79.2W"), "spacing survives");
    }

    #[test]
    fn headers_and_separators_are_marked_up() {
        assert_eq!(heading(".SHORT TERM...").as_deref(), Some("SHORT TERM"));
        assert_eq!(heading("SYNOPSIS").as_deref(), Some("SYNOPSIS"));
        // A shouted sentence is not a header, and neither is ordinary prose.
        assert_eq!(heading("THE STORM IS EXPECTED TO WEAKEN."), None);
        assert_eq!(heading("Some ordinary prose."), None);
        assert!(body("SYNOPSIS\n\n&&").contains("<h2>SYNOPSIS</h2>"));
        assert!(body("&&").contains("<hr>"));
    }

    #[test]
    fn the_document_is_inert_and_escaped() {
        let doc = document("AFD OUN", "2026-08-25", "<script>alert(1)</script> & more");
        assert!(doc.contains("default-src 'none'"));
        assert!(!doc.contains("<script>"), "{doc}");
        assert!(doc.contains("&lt;script&gt;") && doc.contains("&amp; more"));
    }
}
