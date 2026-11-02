//! Log-safe rendering of URLs and other page-shaped strings.
//!
//! pico.log is a plain-text file the user is routinely asked to
//! share when debugging. Page URLs routinely carry secrets in
//! their query string (OAuth codes in dev login flows, API keys,
//! session tokens) and Pico's own server URLs embed the root
//! token — so nothing URL-shaped reaches the log with its query
//! or fragment intact. [`redact_url`] keeps scheme + authority +
//! path (what a human needs to recognize the address) and
//! replaces the query with `?…` and the fragment with `#…`:
//! presence stays visible, contents never do. Userinfo is
//! dropped entirely (it can carry credentials).
//!
//! This is a pure display transformation — it never feeds
//! navigation.
//!
//! AUDIT V28 (A5-L2), a stated trade-off: the PATH is kept
//! whole, and Pico's own server URLs carry the per-process
//! root token in the path (`/r/<token>/…`). An earlier draft
//! of this module's rationale implied the token was protected
//! too; it is not, deliberately: pico.log's reader is the user
//! on their own machine (the file exists to be pasted back to
//! support), the server is loopback-only with a Host wall the
//! token alone cannot pass from another machine, and the
//! token dies with the process. Query and fragment CONTENTS —
//! where real credentials live — are always hidden.

/// Maximum characters of a redacted value kept for the log.
pub const MAX_LOG_VALUE_CHARS: usize = 300;

/// Render `url` safe for the log: no userinfo, no query
/// contents, no fragment contents. Non-URL input passes through
/// the same hiding + control-stripping + length cap (callers
/// also use this for URIs from download events and error
/// strings).
pub fn redact_url(url: &str) -> String {
    let (scheme, rest) = match url.split_once("://") {
        Some((s, r)) if !s.is_empty() && s.len() <= 32 => (s, r),
        _ => {
            // Not a schemed URL (an error string, a raw IPC
            // body): still hide anything behind ? or # — callers
            // pass arbitrary page-shaped text here, and a query's
            // contents are never needed to debug it.
            let cut = url.find(['?', '#']).unwrap_or(url.len());
            let mut out = url[..cut].to_string();
            let tail = &url[cut..];
            if tail.starts_with('?') {
                out.push_str("?\u{2026}");
                if tail.contains('#') {
                    out.push_str("#\u{2026}");
                }
            } else if tail.starts_with('#') {
                out.push_str("#\u{2026}");
            }
            return cap_and_clean(&out);
        }
    };
    // Authority ends at the first path/query/fragment delimiter.
    let auth_end = rest
        .find(|c| ['/', '?', '#'].contains(&c))
        .unwrap_or(rest.len());
    let authority = &rest[..auth_end];
    // Drop userinfo: it is `user[:pass]@` and never belongs in a
    // log (the LAST '@' separates it, per RFC 3986).
    let host_port = authority.rsplit('@').next().unwrap_or(authority);
    let tail = &rest[auth_end..];
    let path_end = tail.find(|c| ['?', '#'].contains(&c)).unwrap_or(tail.len());
    let path = &tail[..path_end];
    let suffix = &tail[path_end..];
    let mut out = String::with_capacity(url.len().min(MAX_LOG_VALUE_CHARS) + 4);
    out.push_str(&scheme.to_ascii_lowercase());
    out.push_str("://");
    out.push_str(host_port);
    out.push_str(path);
    if suffix.starts_with('?') {
        out.push_str("?\u{2026}");
        if suffix.contains('#') {
            out.push_str("#\u{2026}");
        }
    } else if suffix.starts_with('#') {
        out.push_str("#\u{2026}");
    }
    cap_and_clean(&out)
}

/// Invisible format characters that are NOT `char::is_control`
/// but still forge log structure in a viewer: U+2028/U+2029
/// (line/paragraph separators — a title containing one renders
/// as two log lines), the bidi controls U+202A–U+202E (visual
/// spoofing of what a line says), zero-width marks
/// U+200B–U+200F, word joiner / invisible operators
/// U+2060–U+2064, and the BOM U+FEFF. AUDIT V28 (A5-L6) —
/// shared by this module and the app logger's sanitize.
/// AUDIT V29 (A5-4): the set now also covers the bidi
/// ISOLATES U+2066–U+2069 (the modern replacement for the
/// covered embeddings, equally able to reorder what a log
/// line appears to say), the Arabic letter mark U+061C, the
/// soft hyphen U+00AD, U+180E, the deprecated format controls
/// U+206A–U+206F, and the interlinear annotation anchors
/// U+FFF9–U+FFFB.
/// AUDIT 0.31 (A5-L5 + A6-F8): the set now also covers
/// U+2065 (an unassigned-but-format slot in the invisible
/// operators block, rendered as nothing by viewers) and
/// the Unicode Tags range — the language tag U+E0001 and
/// the tag characters U+E0020–U+E007F, which are invisible
/// in rendering yet survive copy/paste, so a host or log
/// line carrying one looks identical to one without it.
pub fn is_invisible_format(c: char) -> bool {
    matches!(c,
        '\u{00AD}' | '\u{061C}' | '\u{180E}' | '\u{2028}' | '\u{2029}'
        | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}'
        | '\u{2060}'..='\u{2064}' | '\u{2065}' | '\u{2066}'..='\u{2069}' | '\u{206A}'..='\u{206F}'
        | '\u{FEFF}' | '\u{FFF9}'..='\u{FFFB}'
        | '\u{E0001}' | '\u{E0020}'..='\u{E007F}')
}

/// Strip control characters (ANSI escapes, CR/LF smuggling,
/// NULs) and cap the length — the log file must stay
/// one-event-per-line plain text no matter what a page or path
/// throws at it.
pub fn cap_and_clean(s: &str) -> String {
    let mut out = String::with_capacity(s.len().min(MAX_LOG_VALUE_CHARS));
    for c in s.chars().take(MAX_LOG_VALUE_CHARS) {
        if c.is_control() || is_invisible_format(c) {
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    if s.chars().count() > MAX_LOG_VALUE_CHARS {
        out.push_str("\u{2026}[truncated]");
    }
    out
}

#[cfg(test)]
mod tests_v28 {
    use super::*;

    #[test]
    fn invisible_format_characters_are_stripped() {
        // A5-L6: U+2028 forges a line break in viewers, U+202E
        // spoofs direction, zero-width marks hide content —
        // none are is_control(), all must be cleaned.
        let dirty = "title\u{2028}forged\u{202E}evil\u{200B}x\u{FEFF}";
        let clean = cap_and_clean(dirty);
        assert!(!clean.contains('\u{2028}'));
        assert!(!clean.contains('\u{202E}'));
        assert!(!clean.contains('\u{200B}'));
        assert!(!clean.contains('\u{FEFF}'));
        assert!(is_invisible_format('\u{2029}'));
        assert!(is_invisible_format('\u{2060}'));
        // A5-4 (v29): the bidi isolates and the Arabic letter
        // mark — the modern spoofing set.
        assert!(is_invisible_format('\u{2066}'));
        assert!(is_invisible_format('\u{2069}'));
        assert!(is_invisible_format('\u{061C}'));
        assert!(is_invisible_format('\u{00AD}'));
        assert!(!is_invisible_format('a'));
        assert!(
            !is_invisible_format('\u{2026}'),
            "the ellipsis marker stays"
        );
    }

    #[test]
    fn tags_range_and_u2065_are_invisible_format() {
        // AUDIT 0.31 (A5-L5): U+2065 and the Unicode Tags
        // range join the set; their immediate neighbours
        // outside the ranges do not.
        assert!(is_invisible_format('\u{2065}'));
        assert!(is_invisible_format('\u{E0001}'));
        assert!(is_invisible_format('\u{E0020}'));
        assert!(is_invisible_format('\u{E0041}'));
        assert!(is_invisible_format('\u{E007F}'));
        assert!(!is_invisible_format('\u{E0000}'));
        assert!(!is_invisible_format('\u{E0002}'));
        assert!(!is_invisible_format('\u{E001F}'));
        assert!(!is_invisible_format('\u{E0080}'));
        // And they are stripped from log output like the rest.
        let clean = cap_and_clean("a\u{2065}b\u{E0041}c");
        assert_eq!(clean, "a b c");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_show_presence_never_contents() {
        assert_eq!(
            redact_url("http://localhost:3000/login?code=SECRET&next=/x"),
            "http://localhost:3000/login?\u{2026}"
        );
        assert_eq!(
            redact_url("https://ex.test/a?x=1#y=2"),
            "https://ex.test/a?\u{2026}#\u{2026}"
        );
        assert_eq!(redact_url("https://ex.test/plain"), "https://ex.test/plain");
    }

    #[test]
    fn userinfo_never_survives() {
        let out = redact_url("http://user:hunter2@127.0.0.1:8080/x?y=1");
        assert!(!out.contains("hunter2"));
        assert!(out.contains("127.0.0.1:8080"));
    }
}
