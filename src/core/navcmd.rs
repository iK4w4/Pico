//! The nav-command vocabulary (0.41.0, Workstream B) —
//! the strict protocol for every bookmarks/history
//! gesture the chrome, the start page, and the history
//! page can ask the shell to perform. It mirrors
//! `core::dlcmd` (the downloads vocabulary) exactly in
//! shape and discipline, because the two share the
//! problem: untrusted page script must never be able to
//! ask for more than its surface is allowed.
//!
//! Canonical wire body, identical on every surface:
//!
//! ```json
//! {"kind":"nav","action":"<kebab-verb>", ...args}
//! ```
//!
//! - The chrome sends it over the chrome transport (the
//!   `{v, boot, req}` envelope is stripped before this
//!   parser runs, and the body dedupes like every chrome
//!   command — see app.rs).
//! - The start page and the history page send it over
//!   their content transport as the whole IPC body.
//!
//! `parse_nav_body` is the ONLY parser. It is strict:
//! `deny_unknown_fields`, a hard size cap, the kind tag
//! must be exactly `"nav"`, and every argument is
//! validated/clamped here so downstream code trusts the
//! values. The `NAV_COMMANDS` table is the single source
//! of truth for which verbs exist and which surfaces may
//! send them; app.rs checks `spec.allows(surface)` for
//! every parsed body, and the history-page verbs ALSO
//! pass a conjunctive tab-kind + URL gate
//! ([`history_page_gate`]) before the parser runs —
//! exactly the downloads-page discipline.
//!
//! Deliberate exclusions, mirroring dlcmd's reasoning:
//! there is NO "open this URL" verb. Opening a bookmark
//! from the start page rides the EXISTING start-action
//! dispatch (`core::action`, a `service`-shaped verb
//! gated to the start tab), because a nav verb that
//! navigates would be a confused deputy the moment any
//! surface could send it. Likewise there is no
//! "bookmark this arbitrary URL" verb: `bookmark-toggle`
//! and `bookmark-query` carry NO url argument at all —
//! they act on the shell's ACTIVE tab, whose URL only
//! the shell knows.
//!
//! The one collision to know about: the content
//! script's navigation OBSERVER notes also use
//! `{"kind":"nav", ...}` (they carry `how`/`href`, never
//! `action`). Routing distinguishes them by the presence
//! of an `action` field ([`looks_like_nav_body`]
//! requires it); the observer parser remains the
//! fallback and this parser rejects note-shaped bodies
//! as unknown actions.

use serde::Deserialize;

/// The history page's canonical URL — the ONE spelling
/// the tab-kind gate compares against after
/// normalization (mirrors `dlcmd::DOWNLOADS_URL`).
pub const HISTORY_URL: &str = "pico://history";

/// Bodies larger than this are refused before parsing —
/// the largest legitimate body (a remove with a 2,048-char
/// URL) fits with room to spare.
pub const MAX_NAV_BODY_BYTES: usize = 8192;
/// A search/suggestion query's cap, in chars.
pub const MAX_NAV_QUERY_CHARS: usize = 512;
/// How many suggestion rows the chrome asks for.
pub const SUGGESTION_LIMIT: usize = 8;
/// How many recent-history tiles the start page shows.
pub const RECENT_TILES: usize = 8;

/// Which surface a nav body arrived from. Each verb
/// lists the surfaces allowed to send it in
/// [`NAV_COMMANDS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavSurface {
    /// The chrome (shell) transport.
    Chrome,
    /// The start page's content transport.
    Start,
    /// The history page's content transport.
    History,
}

/// Argument shape of one verb (the dlcmd discipline: the
/// table and the parser must agree, and a test pins it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavArgSchema {
    /// No argument keys beyond kind/action.
    None,
    /// Exactly one string key: `url`.
    Url,
    /// Exactly one string key: `query`.
    Query,
}

/// One row of the vocabulary table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NavCommandSpec {
    pub name: &'static str,
    pub args: NavArgSchema,
    pub surfaces: &'static [NavSurface],
}

impl NavCommandSpec {
    pub fn allows(&self, surface: NavSurface) -> bool {
        self.surfaces.contains(&surface)
    }
}

use NavSurface::{Chrome, History, Start};

/// THE vocabulary. Order is documentation order:
/// chrome bookmark verbs, the start page's read/remove,
/// the history page's own verbs, then suggestions.
pub const NAV_COMMANDS: &[NavCommandSpec] = &[
    // Toggle the ACTIVE tab's bookmark (the star, Ctrl+D).
    // No URL argument exists by design (see module header).
    NavCommandSpec {
        name: "bookmark-toggle",
        args: NavArgSchema::None,
        surfaces: &[Chrome],
    },
    // Ask whether the ACTIVE tab's URL is bookmarked. The
    // answer rides the next chrome snapshot (the star's
    // filled state), so this verb exists for the chrome
    // to re-ask after in-page navigations the snapshot
    // cadence might lag; it changes nothing.
    NavCommandSpec {
        name: "bookmark-query",
        args: NavArgSchema::None,
        surfaces: &[Chrome],
    },
    // Read the bookmark list (the start page asks once
    // on load; the answer is the __picoSetBookmarks
    // push). Changes nothing. Start-only: the chrome
    // never renders a bookmark list — its surfaces are
    // the star (snapshot) and the dropdown
    // (suggestions-query).
    NavCommandSpec {
        name: "bookmarks-list",
        args: NavArgSchema::None,
        surfaces: &[Start],
    },
    // Remove one bookmark by URL (the start page's ×).
    NavCommandSpec {
        name: "bookmark-remove",
        args: NavArgSchema::Url,
        surfaces: &[Start],
    },
    // Read the full history list (the history page's
    // initial fill; live pushes keep it current after).
    NavCommandSpec {
        name: "history-list",
        args: NavArgSchema::None,
        surfaces: &[History],
    },
    // Search history (the history page's search box).
    NavCommandSpec {
        name: "history-search",
        args: NavArgSchema::Query,
        surfaces: &[History],
    },
    // Remove one history entry by URL (a row's ×).
    NavCommandSpec {
        name: "history-remove",
        args: NavArgSchema::Url,
        surfaces: &[History],
    },
    // Clear the whole history (the page's Clear button;
    // the page arms a two-step confirm before sending).
    NavCommandSpec {
        name: "history-clear",
        args: NavArgSchema::None,
        surfaces: &[History],
    },
    // Address-bar suggestions for a query string. The
    // answer is pushed into the chrome as
    // __picoNavSuggestions; changes nothing.
    NavCommandSpec {
        name: "suggestions-query",
        args: NavArgSchema::Query,
        surfaces: &[Chrome],
    },
];

/// Look up a verb's spec by its wire spelling.
pub fn spec_for(name: &str) -> Option<&'static NavCommandSpec> {
    NAV_COMMANDS.iter().find(|s| s.name == name)
}

/// A parsed, validated nav command. Constructed ONLY by
/// [`parse_nav_body`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavCommand {
    BookmarkToggle,
    BookmarkQuery,
    BookmarksList,
    BookmarkRemove { url: String },
    HistoryList,
    HistorySearch { query: String },
    HistoryRemove { url: String },
    HistoryClear,
    SuggestionsQuery { query: String },
}

impl NavCommand {
    /// The wire spelling (the log line's verb).
    pub fn name(&self) -> &'static str {
        match self {
            NavCommand::BookmarkToggle => "bookmark-toggle",
            NavCommand::BookmarkQuery => "bookmark-query",
            NavCommand::BookmarksList => "bookmarks-list",
            NavCommand::BookmarkRemove { .. } => "bookmark-remove",
            NavCommand::HistoryList => "history-list",
            NavCommand::HistorySearch { .. } => "history-search",
            NavCommand::HistoryRemove { .. } => "history-remove",
            NavCommand::HistoryClear => "history-clear",
            NavCommand::SuggestionsQuery { .. } => "suggestions-query",
        }
    }
}

/// Why a nav body was refused. Each variant is a closed
/// log word (never body content), mirroring `DlDeny`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavDeny {
    /// Over the byte cap, or not JSON of the body shape.
    Malformed,
    /// The `kind` tag was not exactly `"nav"`.
    WrongKind,
    /// The action string names no verb in the table.
    UnknownAction,
    /// A required argument was missing or invalid
    /// (unrecordable URL, control characters, too long).
    BadArgs,
}

impl NavDeny {
    pub fn as_str(&self) -> &'static str {
        match self {
            NavDeny::Malformed => "malformed",
            NavDeny::WrongKind => "wrong-kind",
            NavDeny::UnknownAction => "unknown-action",
            NavDeny::BadArgs => "bad-args",
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawNav {
    kind: String,
    action: String,
    url: Option<String>,
    query: Option<String>,
}

fn valid_url_arg(raw: &str) -> Option<String> {
    // A remove target must be a URL the store could hold:
    // recordable, bounded, control-free. (Checked against
    // the store afterwards anyway; this keeps junk out of
    // the log-shaped values.)
    if crate::core::navstore::is_recordable_url(raw) {
        Some(raw.to_string())
    } else {
        None
    }
}

fn valid_query_arg(raw: &str) -> Option<String> {
    if raw.chars().count() > MAX_NAV_QUERY_CHARS || raw.chars().any(|c: char| c.is_control()) {
        return None;
    }
    Some(raw.to_string())
}

/// Parse and validate one nav body. The ONLY parser.
pub fn parse_nav_body(raw: &str) -> Result<NavCommand, NavDeny> {
    if raw.len() > MAX_NAV_BODY_BYTES {
        return Err(NavDeny::Malformed);
    }
    let parsed: RawNav = serde_json::from_str(raw).map_err(|_| NavDeny::Malformed)?;
    if parsed.kind != "nav" {
        return Err(NavDeny::WrongKind);
    }
    let spec = spec_for(&parsed.action).ok_or(NavDeny::UnknownAction)?;
    // Argument-shape enforcement against the table: a
    // verb that takes no args refuses bodies carrying
    // any, and a verb that takes one refuses the other
    // key or a missing value.
    match spec.args {
        NavArgSchema::None => {
            if parsed.url.is_some() || parsed.query.is_some() {
                return Err(NavDeny::BadArgs);
            }
        }
        NavArgSchema::Url => {
            if parsed.query.is_some() || parsed.url.is_none() {
                return Err(NavDeny::BadArgs);
            }
        }
        NavArgSchema::Query => {
            if parsed.url.is_some() || parsed.query.is_none() {
                return Err(NavDeny::BadArgs);
            }
        }
    }
    match parsed.action.as_str() {
        "bookmark-toggle" => Ok(NavCommand::BookmarkToggle),
        "bookmark-query" => Ok(NavCommand::BookmarkQuery),
        "bookmarks-list" => Ok(NavCommand::BookmarksList),
        "bookmark-remove" => Ok(NavCommand::BookmarkRemove {
            url: valid_url_arg(parsed.url.as_deref().unwrap_or("")).ok_or(NavDeny::BadArgs)?,
        }),
        "history-list" => Ok(NavCommand::HistoryList),
        "history-search" => Ok(NavCommand::HistorySearch {
            query: valid_query_arg(parsed.query.as_deref().unwrap_or(""))
                .ok_or(NavDeny::BadArgs)?,
        }),
        "history-remove" => Ok(NavCommand::HistoryRemove {
            url: valid_url_arg(parsed.url.as_deref().unwrap_or("")).ok_or(NavDeny::BadArgs)?,
        }),
        "history-clear" => Ok(NavCommand::HistoryClear),
        "suggestions-query" => Ok(NavCommand::SuggestionsQuery {
            query: valid_query_arg(parsed.query.as_deref().unwrap_or(""))
                .ok_or(NavDeny::BadArgs)?,
        }),
        _ => Err(NavDeny::UnknownAction),
    }
}

/// Cheap routing pre-filter for IPC bodies (the content
/// handler's kind dispatch): is this plausibly a nav
/// COMMAND body? Requires a top-level `action` KEY —
/// that is what separates a command from a navigation
/// OBSERVER note, which shares the `"nav"` kind but
/// carries `how`/`href` and must fall through to the
/// observer parser untouched.
///
/// AUDIT 0.41.3 (Front C F4): the check is STRUCTURAL
/// now — trim, require `{`, parse the Value, require
/// `kind == "nav"` AND an `action` key present. The
/// substring version swallowed honest bodies: an
/// observer note from a page titled exactly `action`
/// (`"title":"action"`) contained both needles and was
/// routed into the command branch (which returns
/// unconditionally, eating the note), and a
/// shield-prune report carrying an "action"-shaped
/// string did the same. A title is a VALUE, not a key;
/// only the parsed shape can tell them apart.
pub fn looks_like_nav_body(body: &str) -> bool {
    let t = body.trim();
    if !t.starts_with('{') {
        return false;
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(t) else {
        return false;
    };
    v.get("kind").and_then(|k| k.as_str()) == Some("nav")
        && v.as_object().is_some_and(|obj| obj.contains_key("action"))
}

/// The action spelling of a nav body for the log line,
/// without parsing (same discipline as dlcmd's label:
/// closed words only — the value comes from the table or
/// is `unknown`).
pub fn action_label(body: &str) -> &'static str {
    #[derive(Deserialize)]
    struct Label {
        action: Option<String>,
    }
    if let Ok(l) = serde_json::from_str::<Label>(body) {
        if let Some(a) = l.action {
            if let Some(spec) = spec_for(&a) {
                return spec.name;
            }
        }
    }
    "unknown"
}

/// Is this URL (any spelling the webview may report) the
/// history page? Query/fragment and one trailing slash
/// are not part of the page's identity — the downloads
/// gate's rule, mirrored.
pub fn is_history_url(url: &str) -> bool {
    let norm = crate::core::addr::normalize_pico_url(url);
    let base = norm.split(['?', '#']).next().unwrap_or("");
    base.trim_end_matches('/') == HISTORY_URL
}

/// The conjunctive gate for history-page bodies: the
/// SENDING tab must be a history tab AND its current URL
/// must be the history page. Either conjunct alone is
/// insufficient (the downloads gate's lesson, mirrored):
/// the flag can outlive a navigation away, and the URL
/// can be spoofed by a tab that merely loaded the
/// spelling.
pub fn history_page_gate(is_history_tab: bool, on_history_url: bool) -> bool {
    is_history_tab && on_history_url
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_complete_and_unique() {
        let mut names: Vec<&str> = NAV_COMMANDS.iter().map(|s| s.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), NAV_COMMANDS.len(), "verb names unique");
        assert_eq!(NAV_COMMANDS.len(), 9, "the 0.41.0 nav vocabulary");
        for spec in NAV_COMMANDS {
            assert_eq!(spec_for(spec.name).map(|s| s.name), Some(spec.name));
            assert!(!spec.surfaces.is_empty(), "{} has a surface", spec.name);
        }
    }

    #[test]
    fn surface_matrix_is_the_spec() {
        let allows = |n: &str, s: NavSurface| spec_for(n).unwrap().allows(s);
        // Chrome: bookmark verbs + suggestions only.
        assert!(allows("bookmark-toggle", Chrome));
        assert!(allows("bookmark-query", Chrome));
        assert!(!allows("bookmarks-list", Chrome));
        assert!(allows("suggestions-query", Chrome));
        assert!(!allows("bookmark-remove", Chrome));
        assert!(!allows("history-list", Chrome));
        assert!(!allows("history-clear", Chrome));
        // Start: list + remove only.
        assert!(allows("bookmarks-list", Start));
        assert!(allows("bookmark-remove", Start));
        assert!(!allows("bookmark-toggle", Start));
        assert!(!allows("history-list", Start));
        assert!(!allows("suggestions-query", Start));
        // History page: its own four only.
        assert!(allows("history-list", History));
        assert!(allows("history-search", History));
        assert!(allows("history-remove", History));
        assert!(allows("history-clear", History));
        assert!(!allows("history-clear", Start));
        assert!(!allows("bookmark-toggle", History));
    }

    #[test]
    fn parse_matrix() {
        assert_eq!(
            parse_nav_body(r#"{"kind":"nav","action":"bookmark-toggle"}"#),
            Ok(NavCommand::BookmarkToggle)
        );
        assert_eq!(
            parse_nav_body(
                r#"{"kind":"nav","action":"bookmark-remove","url":"https://example.com/"}"#
            ),
            Ok(NavCommand::BookmarkRemove {
                url: "https://example.com/".into()
            })
        );
        assert_eq!(
            parse_nav_body(r#"{"kind":"nav","action":"suggestions-query","query":"ex"}"#),
            Ok(NavCommand::SuggestionsQuery { query: "ex".into() })
        );
        // Strictness: unknown field refused (deny_unknown_fields).
        assert_eq!(
            parse_nav_body(r#"{"kind":"nav","action":"bookmark-toggle","x":1}"#),
            Err(NavDeny::Malformed)
        );
        // Wrong kind, unknown action.
        assert_eq!(
            parse_nav_body(r#"{"kind":"dl","action":"bookmark-toggle"}"#),
            Err(NavDeny::WrongKind)
        );
        assert_eq!(
            parse_nav_body(r#"{"kind":"nav","action":"bookmark-nuke"}"#),
            Err(NavDeny::UnknownAction)
        );
        // An observer note is NOT a command (unknown action
        // — it carries how/href; routing never sends it here).
        assert_eq!(
            parse_nav_body(r#"{"kind":"nav","how":"push","href":"https://example.com/"}"#),
            Err(NavDeny::Malformed)
        );
        // Arg shape: no-arg verb with an arg; url verb
        // without one; url verb with the wrong key.
        assert_eq!(
            parse_nav_body(r#"{"kind":"nav","action":"history-clear","url":"https://e.com/"}"#),
            Err(NavDeny::BadArgs)
        );
        assert_eq!(
            parse_nav_body(r#"{"kind":"nav","action":"history-remove"}"#),
            Err(NavDeny::BadArgs)
        );
        assert_eq!(
            parse_nav_body(r#"{"kind":"nav","action":"history-remove","query":"x"}"#),
            Err(NavDeny::BadArgs)
        );
        // Unrecordable remove targets refused (internal
        // pages can never be store rows).
        assert_eq!(
            parse_nav_body(r#"{"kind":"nav","action":"bookmark-remove","url":"pico://start"}"#),
            Err(NavDeny::BadArgs)
        );
        // Query with a control character refused.
        assert_eq!(
            parse_nav_body(
                "{\"kind\":\"nav\",\"action\":\"history-search\",\"query\":\"a\\u0001b\"}"
            ),
            Err(NavDeny::BadArgs)
        );
        // Over the byte cap.
        let big = format!(
            "{{\"kind\":\"nav\",\"action\":\"history-search\",\"query\":\"{}\"}}",
            "a".repeat(MAX_NAV_BODY_BYTES)
        );
        assert_eq!(parse_nav_body(&big), Err(NavDeny::Malformed));
    }

    #[test]
    fn routing_prefilter_separates_commands_from_notes() {
        assert!(looks_like_nav_body(
            r#"{"kind":"nav","action":"history-list"}"#
        ));
        assert!(!looks_like_nav_body(
            r#"{"kind":"nav","how":"push","href":"https://example.com/"}"#
        ));
        assert!(!looks_like_nav_body(r#"{"kind":"dl","action":"list"}"#));
        assert!(!looks_like_nav_body("not json"));
        // AUDIT 0.41.3 (Front C F4): an honest observer
        // note from a page titled exactly "action" is NOT
        // a command — the word is a title VALUE, not a
        // top-level action key (the substring prefilter
        // swallowed this note).
        assert!(!looks_like_nav_body(
            r#"{"kind":"nav","how":"push","href":"https://example.com/","title":"action"}"#
        ));
        // A shield-prune report is not a nav body either,
        // whatever its items are shaped like.
        assert!(!looks_like_nav_body(
            r#"{"kind":"shield_prune","items":[{"p":"action","n":1}]}"#
        ));
        // Whitespace variants of a real command still route.
        assert!(looks_like_nav_body(
            r#"{ "kind" : "nav", "action" : "history-list" }"#
        ));
    }

    #[test]
    fn history_gate_is_conjunctive() {
        assert!(history_page_gate(true, true));
        assert!(!history_page_gate(true, false));
        assert!(!history_page_gate(false, true));
        assert!(!history_page_gate(false, false));
    }

    #[test]
    fn history_url_spellings() {
        assert!(is_history_url("pico://history"));
        assert!(is_history_url("pico://history/"));
        assert!(is_history_url("pico://history?x=1"));
        assert!(is_history_url("https://pico.history/"));
        assert!(!is_history_url("pico://historyx"));
        assert!(!is_history_url("pico://history.evil"));
        assert!(!is_history_url("pico://downloads"));
        assert!(!is_history_url("https://example.com/"));
    }
}
