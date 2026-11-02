//! Asset-integrity tests (AUDIT V14-1): the FINAL assembled chrome
//! and start-page HTML — the exact bytes the runtime serves through
//! the `pico://` protocol — must be complete, self-contained pages.
//! v13's black chrome shipped because nothing ever asserted on the
//! assembled output; these tests exist so a broken assembly fails
//! the build instead of failing silently on the user's screen.

use pico_core::core::assets::{
    assemble_chrome_html, assemble_start_html, extract_inline_scripts, CHROME_TEMPLATE, CSS_TOKEN,
    INIT_TOKEN, REQUIRED_CHROME_IDS, SHARED_CSS,
};

#[test]
fn chrome_final_html_is_complete() {
    let html = assemble_chrome_html();
    assert!(
        html.len() > 10_000,
        "chrome page suspiciously small: {} bytes",
        html.len()
    );
    assert!(!html.contains(CSS_TOKEN), "CSS token survived assembly");
    assert!(!html.contains("/*__"), "an assembler token survived");
    assert!(!html.contains("{{"), "a template token survived");
    assert!(html.contains("--bg:"), "shared CSS variables missing");
    assert!(
        html.trim_end().ends_with("</html>"),
        "page must end with </html>"
    );
    for id in REQUIRED_CHROME_IDS {
        assert!(
            html.contains(&format!("id=\"{id}\"")),
            "required chrome control #{id} missing from the final HTML"
        );
    }
    // The self-diagnosis machinery must be in the shipped page.
    assert!(html.contains("__picoGotSnapshot"), "snapshot flag missing");
    assert!(
        html.contains("didn&rsquo;t connect") || html.contains("didn't connect"),
        "shell warning text missing"
    );
    // Static skeleton: the version badge is correct BEFORE any Rust
    // message arrives.
    assert!(
        html.contains(">0.31<"),
        "static version badge must say 0.31"
    );
}

#[test]
fn start_final_html_is_complete() {
    let html = assemble_start_html("{\"marker\":true}");
    assert!(html.len() > 5_000, "start page suspiciously small");
    assert!(!html.contains(CSS_TOKEN), "CSS token survived assembly");
    assert!(!html.contains(INIT_TOKEN), "init token survived assembly");
    assert!(html.contains("--bg:"), "shared CSS variables missing");
    assert!(html.trim_end().ends_with("</html>"));
    for id in ["services", "services-empty", "recents", "others"] {
        assert!(
            html.contains(&format!("id=\"{id}\"")),
            "start page element #{id} missing"
        );
    }
    assert!(html.contains("\"marker\":true"), "init JSON not embedded");
    assert!(html.contains(">0.31<"), "start page must say 0.31");
    // Script-breakout neutralization is part of assembly: a hostile
    // title inside the init JSON cannot close the script element.
    let hostile = assemble_start_html("{\"t\":\"x</script><script>alert(1)</script>\"}");
    assert!(
        hostile.contains("<\\/script>"),
        "init JSON must be neutralized"
    );
    assert_eq!(
        hostile.matches("</script>").count(),
        1,
        "only the page's own closing script tag may appear literally"
    );
}

/// Syntax-check every inline script with node's parser when node is
/// available. (node --check validates syntax only; behavior is
/// covered by the DOM test in scripts/dom_test.mjs when jsdom is
/// installed, and by these structural assertions otherwise.)
#[test]
fn inline_scripts_parse_under_node() {
    let pages = [
        ("chrome", assemble_chrome_html()),
        ("start", assemble_start_html("{}")),
    ];
    let node = which_node();
    for (name, html) in pages {
        let scripts = extract_inline_scripts(&html);
        assert!(!scripts.is_empty(), "{name} page has no inline script");
        let Some(node) = &node else { continue };
        for (i, script) in scripts.iter().enumerate() {
            let path =
                std::env::temp_dir().join(format!("pico-v20-{name}-{i}-{}.js", std::process::id()));
            std::fs::write(&path, script).expect("write temp script");
            let out = std::process::Command::new(node)
                .arg("--check")
                .arg(&path)
                .output()
                .expect("run node --check");
            let _ = std::fs::remove_file(&path);
            assert!(
                out.status.success(),
                "node --check failed for {name} script {i}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }
}

/// v15 branding contract: the author's mark is on both pages, the
/// social chips are wired by enum name only, and the two destination
/// URLs appear NOWHERE in either page — they live exclusively as
/// constants in Rust (`SocialSite::url`), so no page content can
/// retarget them.
#[test]
fn branding_is_present_and_enum_wired() {
    let chrome = assemble_chrome_html();
    // Titlebar: wordmark + muted "by @iK4w4" text-button.
    assert!(
        chrome.contains("by @iK4w4"),
        "chrome titlebar must carry the byline"
    );
    assert!(
        chrome.contains("id=\"brand-by\""),
        "chrome byline button missing"
    );
    assert!(
        chrome.contains("Telegram — @iK4w4"),
        "byline tooltip must name Telegram"
    );
    // The chrome page sends the enum command for Telegram.
    assert!(
        chrome.contains("\"social\"") && chrome.contains("\"telegram\""),
        "chrome byline must send the social/telegram enum command"
    );

    let start = assemble_start_html("{}");
    // Hero heading + byline row + footer line.
    assert!(start.contains("Pico <span class=\"v\">0.31</span>"));
    assert!(start.contains("by @iK4w4"), "start byline text missing");
    assert!(
        start.contains("Pico 0.31 · by @iK4w4"),
        "start footer must read 'Pico 0.31 · by @iK4w4'"
    );
    // Chips: action-URL hrefs carrying ONLY the enum name, plus the
    // data-social attributes the DOM test drives.
    assert!(start.contains("href=\"pico://social/telegram\""));
    assert!(start.contains("href=\"pico://social/youtube\""));
    assert!(start.contains("data-social=\"telegram\""));
    assert!(start.contains("data-social=\"youtube\""));
    // Chip glyphs are inline SVG — no <img>, no external references.
    assert!(
        !start.contains("<img"),
        "start page must not use <img> for brand glyphs"
    );
    assert!(
        !chrome.contains("<img"),
        "chrome page must not contain <img> markup"
    );
    // The destination URLs live in Rust only — never in the pages.
    for page in [&chrome, &start] {
        assert!(
            !page.contains("https://t.me"),
            "Telegram URL must not appear in a page"
        );
        assert!(
            !page.contains("youtube.com"),
            "YouTube URL must not appear in a page"
        );
        assert!(
            !page.contains("http://") && !page.contains("https://"),
            "pages must not reference external URLs at all"
        );
    }
}

/// v16 header contract: the merged single-row header and the
/// feather, as assembled — and the CSS side of the geometry must
/// agree with the single source of truth in `core::layout` (the
/// dual-height-definition class is what sank the C# v9 chrome).
#[test]
fn header_structure_is_the_v20_contract() {
    use pico_core::core::layout::{CHROME_H, FEATHER_H, SOLID_H};
    let html = assemble_chrome_html();

    // No separate titlebar row, no dedicated drag div.
    assert!(
        !html.contains("id=\"titlebar\""),
        "the standalone titlebar row must be gone"
    );
    assert!(
        !html.contains("id=\"drag\""),
        "the dedicated drag div must be gone (the top row drags)"
    );
    // The merged structure exists.
    for id in ["shell", "topbar", "feather"] {
        assert!(
            html.contains(&format!("id=\"{id}\"")),
            "merged header element #{id} missing"
        );
    }
    // DOM order: brand block (with the byline) BEFORE the tab strip;
    // window controls inside the same top row (before the navbar).
    let pos = |needle: &str| {
        html.find(needle)
            .unwrap_or_else(|| panic!("{needle} missing from chrome html"))
    };
    assert!(
        pos("id=\"brand-by\"") < pos("id=\"tabstrip\""),
        "brand block must precede the tab strip"
    );
    assert!(
        pos("id=\"topbar\"") < pos("id=\"win-close\"")
            && pos("id=\"win-close\"") < pos("id=\"navbar\""),
        "window controls must live in the top row"
    );
    assert!(
        pos("id=\"newtab\"") < pos("id=\"navbar\""),
        "the + button rides the tab strip in the top row"
    );
    // Geometry agreement with core::layout — the CSS numbers are
    // derived from the constants here, never re-typed blindly.
    assert!(
        html.contains(&format!("height: {}px", CHROME_H as u32)),
        "chrome CSS must size the shell to CHROME_H ({} DIP)",
        CHROME_H as u32
    );
    assert!(
        html.contains(&format!("height: {}px", FEATHER_H as u32)),
        "feather strip must be FEATHER_H ({} DIP) tall",
        FEATHER_H as u32
    );
    // The address row's solid paint stops at SOLID_H: the stop sits
    // (SOLID_H − top row 40) DIP into the navbar's gradient.
    assert!(
        html.contains(&format!("{}px", SOLID_H as u32 - 40)),
        "navbar solid stop must sit at SOLID_H − 40 ({} DIP)",
        SOLID_H as u32 - 40
    );
    // The feather is a real gradient melt (surface color → nothing),
    // with the baked shadow layer. v19: the shade comes from the
    // --feather-shade token (per-theme); the v18 hardcoded black
    // rgba must never come back (AUDIT D7).
    assert!(
        html.contains("linear-gradient(to bottom, var(--surface), transparent)"),
        "feather fade gradient missing"
    );
    assert!(
        html.contains("linear-gradient(to bottom, var(--feather-shade), transparent 78%)"),
        "feather shade layer must be built from the --feather-shade token"
    );
    assert!(
        !html.contains("rgba(0, 0, 0, 0.22)"),
        "the hardcoded v18 feather shade must not return"
    );
    // The body is transparent (the controller is too — app.rs), so
    // the window's themed background is the failure color.
    assert!(
        html.contains("background: transparent"),
        "chrome body must be transparent for the feather"
    );
    // The tab strip can actually shrink (flex min-width floor off),
    // so brand + controls are never pushed off at 30 tabs.
    assert!(
        html.contains("min-width: 0; height: 40px"),
        "tab strip must opt out of the flex content floor"
    );
    // shellwarn overlays the TOP of the chrome — it must never
    // reflow the 86 DIP geometry.
    assert!(
        html.contains("right: 0; top: 0; z-index: 99"),
        "shellwarn must be pinned to the top as an overlay"
    );
    // v20 ONE SLAB (AUDIT N3): the top row paints the same
    // material token the navbar's solid zone and the feather melt
    // use, and the active tab is attached by the clipped-shadow +
    // concave-fillet mechanism — not by a fill or a border.
    let style = chrome_style_block();
    assert!(
        rule_body(&style, "#topbar").contains("background: var(--surface)"),
        "top row must paint the one material (var(--surface))"
    );
    assert!(
        rule_body(&style, ".tab::before").contains("clip-path: inset(-18px -18px 0 -18px)"),
        "the tab shadow layer must be clipped at the tab's bottom edge"
    );
    assert!(
        style.contains("radial-gradient(circle at 0 0")
            && style.contains("radial-gradient(circle at 100% 0"),
        "the active tab must carry both concave fillet gradients"
    );
}

/// Extract the chrome template's own `<style>` block (the part
/// after the shared-CSS injection token).
fn chrome_style_block() -> String {
    let t = CHROME_TEMPLATE;
    let start = t
        .find("<style>")
        .expect("chrome template has a style block")
        + "<style>".len();
    let end = t.find("</style>").expect("style block closes");
    t[start..end].to_string()
}

/// Remove /* ... */ comments so prose can never trip the scanners.
fn strip_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(s) = rest.find("/*") {
        out.push_str(&rest[..s]);
        match rest[s..].find("*/") {
            Some(e) => rest = &rest[s + e + 2..],
            None => break,
        }
    }
    out.push_str(rest);
    out
}

/// The declaration body of the FIRST rule matching `selector`.
fn rule_body(css: &str, selector: &str) -> String {
    let clean = strip_comments(css);
    let at = clean
        .find(selector)
        .unwrap_or_else(|| panic!("selector {selector} missing"));
    let open = clean[at..].find('{').expect("rule opens") + at;
    let close = clean[open..].find('}').expect("rule closes") + open;
    clean[open + 1..close].to_string()
}

/// The declaration body of one generated theme block in the
/// ASSEMBLED page CSS (v27: theme tokens are generated by
/// core::themes and appended at assembly, so the blocks live
/// in the assembled output, not in the raw style.css).
fn theme_block(palette: &str, mode: &str) -> String {
    let css = assemble_chrome_html();
    let marker = format!("[data-palette=\"{palette}\"][data-theme=\"{mode}\"]");
    let at = css
        .find(&marker)
        .unwrap_or_else(|| panic!("theme block {marker} missing"));
    let open = css[at..].find('{').expect("theme block opens") + at;
    let close = css[open..].find("\n}").expect("theme block closes") + open;
    css[open + 1..close].to_string()
}

/// The alpha component of an `rgba(...)` token value in `block`.
/// Searches for the token's DEFINITION (`--token:`) — since v20 a
/// theme block also contains var() REFERENCES to the token (e.g.
/// --neu-focus references --focus-glow) before the definition.
fn rgba_alpha(block: &str, token: &str) -> f64 {
    let definition = format!("{token}:");
    let at = block
        .find(&definition)
        .unwrap_or_else(|| panic!("{token} definition missing"));
    let rest = &block[at..];
    let open = rest.find("rgba(").expect("token value must be rgba()");
    let close = rest.find(')').expect("rgba() closes");
    rest[open + 5..close]
        .rsplit(',')
        .next()
        .expect("alpha component")
        .trim()
        .parse::<f64>()
        .expect("alpha parses")
}

/// Every px literal (and bare 0) in a declaration value.
fn px_literals(value: &str) -> Vec<f64> {
    let chars: Vec<char> = value.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit()
            || (chars[i] == '.' && i + 1 < chars.len() && chars[i + 1].is_ascii_digit())
        {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            let num: f64 = chars[start..i]
                .iter()
                .collect::<String>()
                .parse()
                .unwrap_or(f64::NAN);
            let unit: String = chars[i..].iter().take(2).collect();
            let bare = !chars
                .get(i)
                .map(|c| c.is_alphabetic() || *c == '%')
                .unwrap_or(false);
            if unit == "px" || (num == 0.0 && bare) {
                out.push(num);
            }
        } else {
            i += 1;
        }
    }
    out
}

/// v19 alignment contract (AUDIT D3): every margin/padding/gap in
/// the chrome CSS is drawn from the spacing scale
/// {0, 2, 4, 6, 8, 12, 16}. The single documented exception —
/// #navbar's geometry-derived vertical padding — is pinned
/// exactly, neutralized, and the scan proceeds without it.
#[test]
fn chrome_spacing_stays_on_the_scale() {
    const SCALE: [f64; 7] = [0.0, 2.0, 4.0, 6.0, 8.0, 12.0, 16.0];
    let mut css = strip_comments(&chrome_style_block());
    assert!(
        css.contains("padding: 1px 12px 13px;"),
        "the #navbar geometry padding changed shape (AUDIT D9 pins it)"
    );
    css = css.replace("padding: 1px 12px 13px;", "padding: 12px;");
    let props = [
        "margin",
        "margin-top",
        "margin-right",
        "margin-bottom",
        "margin-left",
        "padding",
        "padding-top",
        "padding-right",
        "padding-bottom",
        "padding-left",
        "gap",
        "row-gap",
        "column-gap",
    ];
    let mut checked = 0usize;
    for chunk in css.split([';', '{', '}']) {
        let Some((prop, value)) = chunk.split_once(':') else {
            continue;
        };
        if !props.contains(&prop.trim()) {
            continue;
        }
        for n in px_literals(value) {
            checked += 1;
            assert!(
                SCALE.contains(&n),
                "spacing value {n}px in '{}' is outside the v19 scale {SCALE:?}",
                chunk.trim()
            );
        }
    }
    assert!(checked >= 20, "spacing scan saw too little ({checked})");
}

/// v19 radius contract (AUDIT D4): border-radius values in the
/// chrome come only from the shared --r-* tokens (or 50% for true
/// circles, or 0), and the tokens themselves hold the scale.
#[test]
fn chrome_radii_come_from_the_token_scale() {
    let css = strip_comments(&chrome_style_block());
    let mut checked = 0usize;
    for chunk in css.split([';', '{', '}']) {
        let Some((prop, value)) = chunk.split_once(':') else {
            continue;
        };
        if prop.trim() != "border-radius" {
            continue;
        }
        checked += 1;
        for tok in value.split_whitespace() {
            assert!(
                tok.starts_with("var(--r-") || tok == "50%" || tok == "0" || tok == "inherit",
                "border-radius token '{tok}' is outside the radius system \
                 (var(--r-*) tokens, 50%, 0, or 'inherit' — the v20 tab \
                 shadow layer inherits its tab's radius by design)"
            );
        }
    }
    assert!(checked >= 8, "radius scan saw too little ({checked})");
    // v24 consciously re-pins --r-lg/--r-xl at +2px (AUDIT
    // V24-3 modern touch: softer cards/inputs); the small radii
    // — and therefore the chrome geometry — are unchanged.
    for (name, val) in [
        ("--r-sm", "6px"),
        ("--r-md", "8px"),
        ("--r-lg", "14px"),
        ("--r-xl", "18px"),
        ("--r-pill", "999px"),
    ] {
        assert!(
            SHARED_CSS.contains(&format!("{name}: {val}")),
            "radius token {name} missing or changed"
        );
    }
}

/// v20 focus contract (AUDIT v20 N4): the address well has NO
/// border and focus draws NO outline and NO border — focus adds
/// ONLY the --neu-focus glow (accent at <= 0.18 alpha) layered
/// over the unchanged --neu-inset pair. The input itself never
/// paints a ring (render pass 1 caught the shared :focus-visible
/// shadow doing exactly that inside the pill).
#[test]
fn address_focus_is_a_shadow_only_glow() {
    let html = assemble_chrome_html();
    assert!(
        !html.contains("outline: 2px solid var(--accent)"),
        "the v18 full-alpha focus hoop must not return"
    );
    assert!(
        !html.contains("border: 1px solid transparent"),
        "the v19 pre-allocated focus border must not return"
    );
    let style = chrome_style_block();
    let wrap = rule_body(&style, "#addr-wrap");
    assert!(
        !wrap.contains("border:") && !wrap.contains("border-color"),
        "the address well must carry no border declaration, got: {wrap}"
    );
    assert!(
        wrap.contains("transition: box-shadow 120ms ease"),
        "addr focus must cross-fade its shadow in 120ms"
    );
    let focus = rule_body(&style, "#addr-wrap:focus-within");
    // v27 re-pin (AUDIT V27-A3): the focus layer gains the
    // theme's --glow on top of the v20 inset+focus pair — the
    // address bar is one of the approved glow sites.
    assert!(
        focus.contains("box-shadow: var(--neu-inset), var(--neu-focus), var(--glow)"),
        "focus must layer the glow over the resting inset pair, got: {focus}"
    );
    assert!(
        !focus.contains("border"),
        "focus must not introduce a border, got: {focus}"
    );
    // The input never draws its own focus ring — the wrap glow is
    // the address bar's only focus indication.
    let input_focus = rule_body(&style, "#addr:focus-visible");
    assert!(
        input_focus.contains("box-shadow: none"),
        "the address input must suppress its own focus ring, got: {input_focus}"
    );
    for p in pico_core::core::themes::PALETTES {
        for mode in ["light", "dark"] {
            let alpha = rgba_alpha(&theme_block(p.id, mode), "--focus-glow");
            assert!(
                alpha <= 0.18 + f64::EPSILON,
                "{} {mode} focus glow alpha {alpha} exceeds the 0.18 bound",
                p.id
            );
        }
    }
}

/// v19 control contract (AUDIT D2/D4): icon buttons share one
/// 30x30 box and the --r-md radius; caption buttons are 38 wide;
/// no per-button glyph-size overrides survive; both rows' content
/// starts on the shared 12 DIP page inset.
#[test]
fn chrome_controls_share_one_box_and_glyph_size() {
    let style = chrome_style_block();
    for sel in [".nbtn", "#newtab", ".tab .x"] {
        let body = rule_body(&style, sel);
        assert!(
            body.contains("width: 30px") && body.contains("height: 30px"),
            "{sel} must be a uniform 30x30 icon button"
        );
    }
    for sel in [".nbtn", "#newtab"] {
        assert!(
            rule_body(&style, sel).contains("border-radius: var(--r-md)"),
            "{sel} must use the --r-md radius"
        );
    }
    assert!(
        rule_body(&style, ".wbtn").contains("width: 38px"),
        "caption buttons are uniformly 38 DIP wide"
    );
    assert!(
        !style.contains(".wbtn svg") && !style.contains(".x svg"),
        "per-button glyph size overrides (13px/11px) must not return"
    );
    assert!(
        rule_body(&style, "#topbar").contains("padding: 0 0 0 12px"),
        "top row content must start on the 12 DIP page inset"
    );
}

/// The value of one `--token: value;` declaration inside a
/// theme block (trimmed, without the trailing semicolon).
fn token_value(block: &str, token: &str) -> String {
    let at = block
        .find(token)
        .unwrap_or_else(|| panic!("{token} missing from theme block"));
    let rest = &block[at + token.len()..];
    let end = rest.find(';').expect("token declaration ends");
    rest[..end].trim().to_string()
}

/// v20 theme parity, generalized by v27 (AUDIT V27-A): EVERY
/// palette × mode block defines the full derived token set
/// and the ONE MATERIAL rule holds in each: --bg and
/// --surface are the same color, so page, slab and elements
/// are one material. The --neu-* shadow GEOMETRY moved to the
/// shared :root (it only composes the per-theme tints), and
/// is asserted there once.
#[test]
fn both_themes_define_the_v20_token_set() {
    for p in pico_core::core::themes::PALETTES {
        for mode in ["light", "dark"] {
            let block = theme_block(p.id, mode);
            for name in pico_core::core::themes::TOKEN_NAMES {
                let tok = format!("{name}:");
                assert!(block.contains(&tok), "{} {mode} is missing {tok}", p.id);
            }
            assert_eq!(
                token_value(&block, "--bg:"),
                token_value(&block, "--surface:"),
                "{} {mode}: --bg and --surface must be ONE material color",
                p.id
            );
        }
    }
    for tok in [
        "--neu-raised:",
        "--neu-raised-sm:",
        "--neu-inset:",
        "--neu-pressed:",
        "--neu-focus:",
        "--neu-focus-ring:",
    ] {
        assert!(
            SHARED_CSS.contains(tok),
            "shared :root is missing the {tok} geometry token"
        );
    }
    // The close hover resolves through the token, never a literal.
    assert!(
        assemble_chrome_html().contains("background: var(--danger)"),
        "close hover must use the --danger token"
    );
}

fn which_node() -> Option<std::path::PathBuf> {
    for cand in ["/usr/bin/node", "/usr/local/bin/node"] {
        let p = std::path::PathBuf::from(cand);
        if p.exists() {
            return Some(p);
        }
    }
    // Fall back to PATH lookup.
    let out = std::process::Command::new("node")
        .arg("--version")
        .output()
        .ok()?;
    if out.status.success() {
        Some(std::path::PathBuf::from("node"))
    } else {
        None
    }
}

/// Extract the start template's own `<style>` block.
fn start_style_block() -> String {
    let t = pico_core::core::assets::START_TEMPLATE;
    let start = t.find("<style>").expect("start template has a style block") + "<style>".len();
    let end = t.find("</style>").expect("style block closes");
    t[start..end].to_string()
}

/// Every CSS source that ships: (name, comment-stripped text).
/// The shared CSS is scanned raw (it holds the token definitions;
/// the scans below only look at `box-shadow` / `border` /
/// `outline` DECLARATIONS, never at custom-property definitions).
fn all_css_sources() -> Vec<(&'static str, String)> {
    vec![
        ("shared", strip_comments(SHARED_CSS)),
        ("chrome", strip_comments(&chrome_style_block())),
        ("start", strip_comments(&start_style_block())),
    ]
}

/// Iterate (property, value) declaration pairs in a CSS text,
/// using the same brace/semicolon chunking as the spacing scan.
fn declarations(css: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for chunk in css.split([';', '{', '}']) {
        let Some((prop, value)) = chunk.split_once(':') else {
            continue;
        };
        out.push((prop.trim().to_string(), value.trim().to_string()));
    }
    out
}

/// Cut the FIRST rule matching `selector` out of a CSS text
/// (selector through its closing brace). Returns the input
/// unchanged when the selector is absent.
fn remove_rule(css: &str, selector: &str) -> String {
    let Some(at) = css.find(selector) else {
        return css.to_string();
    };
    let open = css[at..].find('{').map(|i| at + i).expect("rule opens");
    let close = css[open..]
        .find('}')
        .map(|i| open + i)
        .expect("rule closes");
    format!("{}{}", &css[..at], &css[close + 1..])
}

/// v20 no-borders contract (AUDIT N5): in the neumorphic material,
/// NO border draws a shape. Every border declaration in the
/// shipped CSS must be the `none` reset — with ONE allowlisted
/// exception: the loading spinner's ring (.tab .spin), a
/// functional progress indicator whose border IS its rendering,
/// removed here by exact selector before the scan.
#[test]
fn no_border_draws_a_shape_anywhere() {
    for (name, css) in all_css_sources() {
        // Remove the allowlisted spinner rule (exact selector).
        if css.contains(".tab .spin") {
            assert!(
                rule_body(&css, ".tab .spin").contains("border: 2px solid"),
                "the spinner exception must be the rule we think it is"
            );
        }
        let css = remove_rule(&css, ".tab .spin");
        for (prop, value) in declarations(&css) {
            let is_border = prop == "border"
                || (prop.starts_with("border-")
                    && !prop.starts_with("border-radius")
                    && prop != "border-image");
            if !is_border {
                continue;
            }
            assert!(
                value == "none" || value == "0",
                "{name}: border declaration '{prop}: {value}' draws a shape — \
                 v20 controls are shaped by shadow only (AUDIT N5)"
            );
        }
    }
}

/// v20 shadow-vocabulary contract (AUDIT N2): EVERY box-shadow
/// declaration in the shipped CSS is composed solely of --neu-*
/// token references (or is `none`). A raw shadow literal — the
/// way v19's pages each hand-rolled their own pairs — fails here.
/// v27 amendment (AUDIT V27-A3): the two GLOW tokens are the
/// second sanctioned shadow vocabulary — the accent layer the
/// glow contract sites compose over the neumorphic base.
#[test]
fn every_box_shadow_references_a_neu_token() {
    let mut checked = 0usize;
    for (name, css) in all_css_sources() {
        for (prop, value) in declarations(&css) {
            if prop != "box-shadow" {
                continue;
            }
            checked += 1;
            if value == "none" {
                continue;
            }
            for term in value.split(',') {
                let term = term.trim();
                assert!(
                    (term.starts_with("var(--neu-")
                        || term == "var(--glow)"
                        || term == "var(--glow-soft)")
                        && term.ends_with(')'),
                    "{name}: box-shadow term '{term}' is not a sanctioned token reference"
                );
            }
        }
    }
    assert!(
        checked >= 15,
        "shadow scan saw too few declarations ({checked})"
    );
}

/// v20 focus contract, global half (AUDIT N4): no `outline`
/// declaration anywhere carries a visible style — focus is
/// drawn by the shadow system only — and no `outline-offset`
/// survives from the outline era.
#[test]
fn focus_treatment_uses_no_outline() {
    for (name, css) in all_css_sources() {
        for (prop, value) in declarations(&css) {
            if prop == "outline" {
                assert_eq!(
                    value, "none",
                    "{name}: outline must be none in v20, got '{value}'"
                );
            }
            assert!(
                prop != "outline-offset",
                "{name}: outline-offset is an outline-era leftover"
            );
        }
    }
}

/// v24 icon assets (AUDIT V24-1): the runtime RGBA is exactly
/// 64x64x4; the .ico the resource compiler consumed is a real
/// multi-frame ICO of PNG-compressed images; and the .rc that
/// produced the checked-in .res names ONLY an icon — no
/// manifest, no version info, nothing else to give an
/// activation context.
#[test]
fn v24_icon_assets_are_well_formed() {
    use pico_core::core::assets::{ICON_RGBA_64, ICON_SIZE};
    assert_eq!(ICON_SIZE, 64);
    assert_eq!(
        ICON_RGBA_64.len(),
        64 * 64 * 4,
        "runtime icon must be raw 64x64 RGBA"
    );
    // The mark is not a blank tile: some pixels are opaque.
    assert!(
        ICON_RGBA_64.chunks(4).any(|px| px[3] == 255),
        "icon has no opaque pixels"
    );
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let ico = std::fs::read(root.join("assets/pico.ico")).expect("pico.ico checked in");
    assert_eq!(&ico[0..4], &[0, 0, 1, 0], "ICO magic (reserved, type=icon)");
    let count = u16::from_le_bytes([ico[4], ico[5]]) as usize;
    assert_eq!(count, 8, "ICO must carry 8 frames (16..256)");
    // The first frame's data is a PNG (all frames are).
    // ICONDIRENTRY: 4 one-byte fields, 2 u16s, then
    // dwBytesInRes (+8) and dwImageOffset (+12).
    let size = u32::from_le_bytes([ico[6 + 8], ico[6 + 9], ico[6 + 10], ico[6 + 11]]);
    let off = u32::from_le_bytes([ico[6 + 12], ico[6 + 13], ico[6 + 14], ico[6 + 15]]) as usize;
    assert!(size > 8 && off + (size as usize) <= ico.len());
    assert_eq!(
        &ico[off..off + 4],
        &[0x89, b'P', b'N', b'G'],
        "ICO frames must be PNG-compressed"
    );
    let rc = std::fs::read_to_string(root.join("assets/pico.rc")).expect("pico.rc checked in");
    assert!(rc.contains("ICON"), "the .rc must declare the icon");
    assert!(
        !rc.contains("MANIFEST") && !rc.contains("VERSIONINFO") && !rc.contains("RT_MANIFEST"),
        "the .rc must contain NOTHING but the icon (AUDIT V24-1)"
    );
}

/// v24 start-page structure (AUDIT V24-2/3): the reorganized
/// placement — compact hero, services first, recents (with its
/// management controls) next, the drop hint attached to the
/// file-opening cluster, and the byline demoted to the footer.
#[test]
fn start_page_v24_layout_and_recents_controls() {
    let html = assemble_start_html("{}");
    let at = |needle: &str| {
        html.find(needle)
            .unwrap_or_else(|| panic!("{needle} missing"))
    };
    let hero = at("class=\"hero\"");
    let services = at("id=\"services\"");
    let recents = at("id=\"recents\"");
    let footer = at("class=\"footer\"");
    assert!(
        hero < services && services < recents && recents < footer,
        "section order must be hero < services < recents < footer"
    );
    // The drop hint rides with the file-opening cluster (after
    // the recents list), not in the hero.
    let hint = at("or drop an HTML file anywhere");
    assert!(
        hint > recents && hint < footer,
        "drop hint must sit with the recents/actions cluster"
    );
    assert!(
        !html[hero..services].contains("drop an HTML file"),
        "the hero must not carry the drop hint anymore"
    );
    // The byline lives in the footer zone now.
    let byline = at("class=\"byline\"");
    assert!(
        byline > footer,
        "the byline must be inside the footer zone (v24)"
    );
    // Recents management controls exist, with the exact legacy
    // action-URL hrefs the fallback transport dispatches on.
    for id in [
        "recents-head",
        "recents-count",
        "recents-vis",
        "recents-clear",
    ] {
        assert!(
            html.contains(&format!("id=\"{id}\"")),
            "recents control #{id} missing"
        );
    }
    assert!(html.contains("href=\"pico://recents-visibility/hide\""));
    assert!(html.contains("href=\"pico://recents-clear\""));
    // The page posts the three new action kinds.
    for kind in ["recent-remove", "recents-clear", "recents-visibility"] {
        assert!(
            html.contains(kind),
            "start page must post the {kind} action"
        );
    }
    assert!(
        html.contains("__picoSetRecentsHidden"),
        "hidden-state push hook missing"
    );
}

// ---- AUDIT V26 (D8): the user-supplied palette ----

/// The six user-supplied hexes ARE the two materials: dark is
/// built on the near-black violet, light is its mirror on the
/// pale lavender, and the accents are the palette's own steps.
/// These pins make a future theme edit argue with the test
/// suite before it argues with the user's screenshot.
///
/// v27 re-pin (AUDIT V27-A4): under the ONE derivation, violet
/// dark still recovers v26's exact material/text/accent —
/// but the light surface is now the documented derivation of
/// the same palette (slot5 lifted toward white 0.30 →
/// #D9D4EB, and light text is slot1 #0E0B1D per the uniform
/// mapping; v26's hand-set #C9C2E3 base / #2A1E4D text were
/// the theme's own exception). Nothing else about v26's
/// look changed: accents, shadows and geometry carry over.
#[test]
fn v26_palette_materials_and_accents() {
    let dark = theme_block("violet", "dark");
    assert_eq!(token_value(&dark, "--bg:"), "#0E0B1D");
    assert_eq!(token_value(&dark, "--surface:"), "#0E0B1D");
    assert_eq!(token_value(&dark, "--text:"), "#C9C2E3");
    assert_eq!(token_value(&dark, "--accent:"), "#8B7CC7");
    let light = theme_block("violet", "light");
    assert_eq!(token_value(&light, "--bg:"), "#D9D4EB");
    assert_eq!(token_value(&light, "--surface:"), "#D9D4EB");
    assert_eq!(token_value(&light, "--text:"), "#0E0B1D");
    assert_eq!(token_value(&light, "--accent:"), "#5A3B8C");
}

/// Derivation equality (AUDIT V27-A5): every generated block
/// in the shipped pages carries EXACTLY the values
/// core::themes::derive computes — the derivation is re-run
/// here from the table, so a hand-edit to a shipped value (or
/// a derivation change that skips the pages) fails loudly.
/// This is the per-theme purity law: a shipped color is a
/// palette hex or a documented derivation OF one, by
/// construction.
#[test]
fn generated_blocks_equal_the_derivation() {
    for p in pico_core::core::themes::PALETTES {
        for (mode, dark) in [("dark", true), ("light", false)] {
            let block = theme_block(p.id, mode);
            for (name, value) in pico_core::core::themes::derive(p, dark) {
                assert_eq!(
                    token_value(&block, &format!("{name}:")),
                    value,
                    "{} {mode} {name} differs from the derivation",
                    p.id
                );
            }
        }
    }
}

/// Palette purity: the OLD base/accent hexes must never
/// reappear in the shipped style assets (style.css is injected
/// into both pages, so scanning it plus the two page sources
/// covers every shipped style). v27 generalizes the scan
/// (AUDIT V27-A5): every hex literal in the shipped styles
/// must be a palette slot, a derived value, or one of the
/// functional constants below (QR matrix, the native-failure
/// strip red, and the two service-brand hover tints) — no
/// other color may sneak in as a literal.
#[test]
fn v26_palette_purity_no_old_hexes() {
    let chrome = assemble_chrome_html();
    let start = assemble_start_html("{}");
    for (name, text) in [
        ("shared css", SHARED_CSS),
        ("chrome", chrome.as_str()),
        ("start", start.as_str()),
    ] {
        let lower = text.to_lowercase();
        for old in ["#212429", "#e6e9ef", "#2e5f8a", "#93b8e6", "#dde3ec"] {
            assert!(
                !lower.contains(old),
                "{name} still carries the old palette hex {old}"
            );
        }
    }
    // Generalized literal scan.
    let mut allowed: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for p in pico_core::core::themes::PALETTES {
        for s in p.slots {
            allowed.insert(s.to_lowercase());
        }
        for dark in [true, false] {
            for (_, v) in pico_core::core::themes::derive(p, dark) {
                let vl = v.to_lowercase();
                if vl.starts_with('#') {
                    allowed.insert(vl);
                }
            }
        }
    }
    for c in [
        "#ffffff", "#fff", "#000000", "#b3261e", "#229ed9", "#ff0033",
    ] {
        allowed.insert(c.to_string());
    }
    // Scan DECLARATION VALUES only — selectors are full of id
    // references (#addr, #back) that are not colors, and
    // comments are stripped first (a comment's colon otherwise
    // glues the next selector into a fake "value").
    for (name, css) in [
        ("chrome", chrome_style_block()),
        ("start", start_style_block()),
    ] {
        for (_, value) in declarations(&strip_comments(&css)) {
            let lower = value.to_lowercase();
            let bytes = lower.as_bytes();
            let mut i = 0;
            while i < bytes.len() {
                if bytes[i] == b'#' {
                    let mut j = i + 1;
                    while j < bytes.len() && (bytes[j] as char).is_ascii_hexdigit() {
                        j += 1;
                    }
                    let lit = &lower[i..j];
                    if lit.len() == 4 || lit.len() == 7 {
                        assert!(
                            allowed.contains(lit),
                            "{name} carries unaccounted color literal {lit}"
                        );
                    }
                    i = j;
                } else {
                    i += 1;
                }
            }
        }
    }
}

/// Glow contract (AUDIT V27-A3): the --glow layer appears at
/// EXACTLY the approved sites and nowhere else — the address
/// bar when focused, the active tab's top edge, the brand
/// mark, icon buttons on hover, the filter/teleport/palette
/// buttons in their active state, the picker cards, the
/// start page's primary cards on hover, and (0.30) the
/// savings band's tiles on hover. Resting surfaces never
/// glow (that would kill the material).
#[test]
fn glow_lives_only_at_the_approved_sites() {
    let style = chrome_style_block();
    for sel in [
        "#addr-wrap:focus-within",
        ".nbtn:hover",
        "#shield[aria-pressed=\"true\"]",
        "#lite[aria-pressed=\"true\"]",
        "#teleport[aria-expanded=\"true\"]",
        "#palette[aria-expanded=\"true\"]",
        ".pcard:hover",
        ".pcard[aria-pressed=\"true\"]",
    ] {
        let body = rule_body(&style, sel);
        assert!(
            body.contains("var(--glow"),
            "approved glow site {sel} lost its glow, got: {body}"
        );
    }
    for sel in [".brand .dot", ".tab[aria-selected=\"true\"]::before"] {
        let body = rule_body(&style, sel);
        assert!(
            body.contains("var(--glow-soft"),
            "approved soft-glow site {sel} lost its glow, got: {body}"
        );
    }
    let start = start_style_block();
    assert!(
        rule_body(&start, ".card:hover").contains("var(--glow)"),
        "start cards must glow on hover"
    );
    // 0.30: the savings tiles hover like the service cards —
    // a twelfth approved site, added deliberately with the
    // band (AUDIT 0.30).
    assert!(
        rule_body(&start, ".stat-tile:hover").contains("var(--glow)"),
        "savings tiles must glow on hover"
    );
    // Total census across both pages' local styles: the sites
    // above and nothing else. (The generated theme blocks
    // DEFINE --glow; they never reference var(--glow).)
    let total = style.matches("var(--glow").count() + start.matches("var(--glow").count();
    assert_eq!(
        total, 12,
        "glow reference count drifted from the approved set (10 chrome + 2 start)"
    );
}

/// Rearrangement contract (AUDIT V27-B): the address row's
/// controls stand in the v27 cluster order — navigation left
/// of the bar, filters right of it, send/appearance last —
/// separated by groove markers, and the counter badges
/// collapse before anything can overflow at min width.
#[test]
fn navbar_clusters_stand_in_v27_order() {
    let html = assemble_chrome_html();
    let nav_at = html.find("id=\"navbar\"").expect("navbar exists");
    let nav = &html[nav_at..];
    let order = [
        "id=\"back\"",
        "id=\"fwd\"",
        "id=\"reload\"",
        "id=\"home\"",
        "id=\"openfile\"",
        "id=\"addr\"",
        "id=\"shield\"",
        "id=\"lite\"",
        "id=\"teleport\"",
        "id=\"palette\"",
        "id=\"theme\"",
    ];
    let mut last = 0usize;
    for (i, id) in order.iter().enumerate() {
        let at = nav
            .find(id)
            .unwrap_or_else(|| panic!("{id} missing from navbar"));
        assert!(
            i == 0 || at > last,
            "{id} stands out of the v27 cluster order"
        );
        last = at;
    }
    assert_eq!(
        nav.matches("class=\"vsep\"").count(),
        3,
        "the navbar needs its three groove separators"
    );
    let style = chrome_style_block();
    assert!(
        style.contains("@media (max-width: 900px)")
            && style.contains(".badges .badge { display: none !important; }"),
        "counter badges must collapse to icon-only before overflow"
    );
    assert!(
        style.contains("@media (max-width: 820px)"),
        "the brand byline yields at min width"
    );
}
