//! Embedded page assembly — the ONLY way chrome/start HTML is
//! built.
//!
//! v14 moved assembly out of the GUI layer (where v13 did runtime
//! `replacen` surgery in `app.rs` and nothing asserted on the
//! bytes actually served) into pure core functions of the
//! embedded templates. The asset-integrity tests therefore
//! exercise the exact strings the runtime serves: a template
//! edit that drops the shared CSS, loses a control id, or leaves
//! a placeholder behind fails `cargo test`, not the user's first
//! launch. v22 keeps that discipline verbatim.

/// The raw chrome template (assets/chrome.html).
pub const CHROME_TEMPLATE: &str = include_str!("../../assets/chrome.html");
/// The raw start-page template (assets/start.html).
pub const START_TEMPLATE: &str = include_str!("../../assets/start.html");
/// The design system shared by both pages (assets/style.css).
pub const SHARED_CSS: &str = include_str!("../../assets/style.css");

/// The runtime window icon: raw 64×64 RGBA drawn by
/// `scripts/gen_icon.py` (AUDIT V24-1) — the same artwork the
/// exe's compiler-built `.ico` carries. The app hands these
/// bytes to tao at window creation, so the taskbar mark does
/// not depend on the resource section at all.
pub const ICON_RGBA_64: &[u8] = include_bytes!("../../assets/icon_64.rgba");
/// Icon edge length in pixels ([`ICON_RGBA_64`] is square).
pub const ICON_SIZE: u32 = 64;

/// Token in both templates where the shared CSS is injected.
pub const CSS_TOKEN: &str = "/*__SHARED_CSS__*/";
/// Token in the start template where the initial-data JSON is
/// injected.
pub const INIT_TOKEN: &str = "__PICO_INIT_JSON__";

/// Control ids the chrome page must contain for the shell to be
/// operable. The asset tests assert every one survives assembly.
pub const REQUIRED_CHROME_IDS: &[&str] = &[
    // The merged header: one top row (brand + tab strip + window
    // controls) and the feather strip — no separate titlebar row,
    // no dedicated drag div (the row's non-control areas drag).
    "shell",
    "topbar",
    "feather",
    "tabstrip",
    "newtab",
    "navbar",
    "back",
    "fwd",
    "reload",
    "home",
    "addr",
    "addr-wrap",
    "openfile",
    "theme",
    "win-min",
    "win-max",
    "win-close",
    "svc-badge",
    "dl-badge",
    "errbar",
    "shellwarn",
    "ver",
    // Branding: the brand block's "by @iK4w4" text-button (opens
    // the author's Telegram via the enum-only Social command).
    "brand-by",
    // The Shield toggle + its blocked-count chip.
    "shield",
    "shield-badge",
    // Lite toggle + chip (v25), Teleport button + panel (v25/v26).
    "lite",
    "lite-badge",
    "teleport",
    "tpanel",
    "tp-backdrop",
    // Teleport panel internals (AUDIT V28 A1-I2: the asset census
    // now covers the whole panel, not just its shell).
    "tp-close",
    "tp-url",
    "tp-qr",
    "tp-note",
    "tp-copy",
    // Theme picker button + panel (v27).
    "palette",
    "ppanel",
    "pp-grid",
    "pp-close",
    "pp-backdrop",
];

/// The full shared stylesheet as served: the hand-written
/// design system plus the GENERATED theme token blocks
/// (core::themes — sixteen palettes × two modes, AUDIT V27-A + 0.31).
/// Computed once; both pages inject exactly these bytes.
pub fn shared_css_full() -> &'static str {
    static FULL: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    FULL.get_or_init(|| format!("{SHARED_CSS}\n{}", crate::core::themes::themes_css()))
}

/// Build the chrome document: template + shared CSS. Pure and
/// total — the templates are compile-time constants, so this
/// cannot fail; if a future edit removes the CSS token the
/// substitution silently no-ops and the integrity tests (which
/// require the `--bg:` definitions in the output) fail instead.
pub fn assemble_chrome_html() -> String {
    CHROME_TEMPLATE.replacen(CSS_TOKEN, shared_css_full(), 1)
}

/// Build the start-page document: template + shared CSS +
/// initial data. `init_json` is a serialized JSON object; any
/// `</` sequence inside it is neutralized here so a hostile
/// local page title cannot break out of the `<script>` element
/// the JSON lives in.
pub fn assemble_start_html(init_json: &str) -> String {
    let safe_json = init_json.replace("</", "<\\/");
    START_TEMPLATE
        .replacen(CSS_TOKEN, shared_css_full(), 1)
        .replacen(INIT_TOKEN, &safe_json, 1)
}

/// Extract the bodies of inline `<script>` elements (those
/// without a `src` attribute) from an HTML document. Used by the
/// asset tests to syntax-check the exact scripts the runtime
/// serves via `node --check`. Byte-based scanning; script bodies
/// in our templates never contain the literal `</script` except
/// at the real end tag.
pub fn extract_inline_scripts(html: &str) -> Vec<String> {
    let bytes = html.as_bytes();
    let mut out = Vec::new();
    let mut pos = 0usize;
    while let Some(open) = find_ci(&bytes[pos..], b"<script") {
        let open = pos + open;
        let Some(gt_rel) = bytes[open..].iter().position(|&b| b == b'>') else {
            break;
        };
        let tag_end = open + gt_rel + 1;
        let tag = &html[open..tag_end];
        if tag.to_ascii_lowercase().contains("src=") {
            pos = tag_end;
            continue;
        }
        let Some(close_rel) = find_ci(&bytes[tag_end..], b"</script") else {
            break;
        };
        let close = tag_end + close_rel;
        out.push(html[tag_end..close].to_string());
        pos = close + 9;
    }
    out
}

/// ASCII case-insensitive substring search on bytes.
fn find_ci(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    (0..=haystack.len() - needle.len()).find(|&i| {
        haystack[i..i + needle.len()]
            .iter()
            .zip(needle.iter())
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chrome_assembly_is_complete() {
        let html = assemble_chrome_html();
        assert!(html.len() > 10_000, "chrome html too small: {}", html.len());
        assert!(!html.contains(CSS_TOKEN), "CSS token survived assembly");
        assert!(html.contains("--bg:"), "shared CSS variables missing");
        assert!(html.trim_end().ends_with("</html>"));
        for id in REQUIRED_CHROME_IDS {
            assert!(
                html.contains(&format!("id=\"{id}\"")),
                "chrome is missing control id {id}"
            );
        }
    }

    #[test]
    fn start_assembly_is_complete_and_escapes_script_breakout() {
        let html = assemble_start_html("{\"theme\":\"light\",\"x\":\"</script>\"}");
        assert!(html.len() > 5_000, "start html too small: {}", html.len());
        assert!(!html.contains(CSS_TOKEN), "CSS token survived assembly");
        assert!(!html.contains(INIT_TOKEN), "init token survived assembly");
        assert!(html.contains("--bg:"), "shared CSS variables missing");
        assert!(
            html.contains("<\\/script>"),
            "init JSON was not neutralized"
        );
        assert!(html.trim_end().ends_with("</html>"));
        for id in ["services", "services-empty", "recents", "others"] {
            assert!(
                html.contains(&format!("id=\"{id}\"")),
                "start page is missing id {id}"
            );
        }
    }

    #[test]
    fn script_extraction_finds_inline_bodies() {
        let chrome = assemble_chrome_html();
        let scripts = extract_inline_scripts(&chrome);
        assert_eq!(scripts.len(), 1, "chrome ships exactly one inline script");
        assert!(scripts[0].contains("__pico"), "chrome renderer missing");
        let start = assemble_start_html("{}");
        let scripts = extract_inline_scripts(&start);
        assert_eq!(
            scripts.len(),
            1,
            "start page ships exactly one inline script"
        );
        assert!(scripts[0].contains("renderServices"));
    }
}
