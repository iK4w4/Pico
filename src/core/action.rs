//! Start-page action dispatch — the SINGLE decision point for
//! every start-page gesture (AUDIT V17-1, carried into v22
//! unchanged in semantics).
//!
//! The start page offers four privileged gestures: open a recent
//! file, open the file picker, open a detected local service, and
//! open one of the author's social pages. The design rules:
//!
//! - **Two transports, one dispatch.** Clicks travel primarily as
//!   `window.ipc` messages and fall back to legacy `pico://…`
//!   anchor navigations when `window.ipc` is unavailable. Both
//!   transports parse into the same [`RawAction`] and are decided
//!   by the same [`dispatch`] — there is no second gate.
//! - **The gate is explicit state.** The caller supplies a
//!   [`Requester`] (is this webview the start tab? what page is
//!   it showing right now?) and [`dispatch`] requires BOTH. A
//!   start tab that has navigated away, and any other tab
//!   regardless of what it shows, are denied — and the gate runs
//!   BEFORE transport parsing, so a hostile page cannot probe
//!   the parser through a denied request.
//! - **Pure and total.** No I/O, no panics; every input produces
//!   a typed [`Decision`], and the app layer logs each one —
//!   silent action death is the historical bug being defended
//!   against, so the decision IS the log line.
//! - **Social destinations are enum-only** ([`SocialSite`]): no
//!   URL for them ever crosses either transport.
//!
//! v22 adds [`is_action_url`] here (it lived as an ad-hoc helper
//! in v21's app layer): the new-window router needs the same
//! classification, and classifications belong with the dispatch.
//!
//! v24 adds the recents-management gestures through the SAME
//! dispatch (AUDIT V24-2): `recent-remove` (per-row ×),
//! `recents-clear`, and `recents-visibility` (hide/show the
//! section). Same gate, same transports, same one-log-line law —
//! new action kinds, no new gate.
//!
//! 0.30 adds `stats-reset` (the savings band's Reset control)
//! the same way (AUDIT 0.30): zeroing the user's own statistics
//! is a privileged gesture exactly like clearing recents, so
//! it rides this dispatch and no other path exists.

use std::path::PathBuf;

use serde::Deserialize;

use crate::core::addr::{normalize_pico_url, web_origin, START_URL};
use crate::core::ipc::SocialSite;

/// Hard cap on an action IPC body. The largest legitimate body is
/// a service-open carrying one localhost URL.
pub const MAX_ACTION_BODY_BYTES: usize = 4096;
/// Hard cap on a service URL carried by an action.
pub const MAX_SERVICE_URL_BYTES: usize = 2048;

/// How an action request arrived.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionSource {
    /// An anchor navigation to a `pico://…` action URL (legacy
    /// fallback transport), or a web URL clicked on the start
    /// page (a service card whose IPC path was unavailable).
    Url(String),
    /// A `window.ipc` message body from the start page's script.
    Ipc(String),
}

/// Who is asking. Assembled by the app layer from its own tab
/// state — never from anything the requesting page supplied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requester {
    /// True when the requesting webview is the start tab.
    pub is_start_tab: bool,
    /// The requesting tab's current URL as tracked by the app
    /// (normalized `pico://` form).
    pub current_url: String,
}

/// An action request after transport parsing, before the
/// semantic checks (index range, service-URL allowlist).
#[derive(Debug, Clone, PartialEq, Eq)]
enum RawAction {
    PickFile,
    Recent(String),
    Social(String),
    Service(String),
    /// v24: remove the recent at this (stringly) index.
    RecentRemove(String),
    /// v24: empty the recents list.
    RecentsClear,
    /// v24: set the Recents section's persisted visibility.
    RecentsVisibility(bool),
    /// 0.30: zero the savings book (band's Reset control).
    StatsReset,
}

/// Why an action was denied. Each variant is one log word; the
/// set is closed so a denial is always classifiable from
/// pico.log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DenyReason {
    NotStartTab,
    NotOnStartPage,
    Malformed,
    UnknownAction,
    RecentOutOfRange,
    BadServiceUrl,
    UnknownSocialSite,
}

impl DenyReason {
    /// Stable log spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            DenyReason::NotStartTab => "not-start-tab",
            DenyReason::NotOnStartPage => "not-on-start-page",
            DenyReason::Malformed => "malformed",
            DenyReason::UnknownAction => "unknown-action",
            DenyReason::RecentOutOfRange => "recent-out-of-range",
            DenyReason::BadServiceUrl => "bad-service-url",
            DenyReason::UnknownSocialSite => "unknown-social-site",
        }
    }
}

/// The one decision every start-page action funnels into. The
/// app layer executes exactly this and nothing else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    OpenRecent(PathBuf),
    OpenPicker,
    OpenService(String),
    OpenSocial(SocialSite),
    /// v24: remove the recent at this index (range-checked
    /// against the same list snapshot the page rendered from).
    RemoveRecent(usize),
    /// v24: empty the recents list.
    ClearRecents,
    /// v24: persist the Recents section's visibility.
    SetRecentsHidden(bool),
    /// 0.30: zero the savings book (lifetime, session, ledger).
    ResetStats,
    Deny(DenyReason),
}

/// True when `url` names a start-page ACTION (as opposed to a
/// document): the pick-file, recent, and social families, in
/// either the canonical `pico://` spelling or wry's folded
/// `https://pico.…` spelling. Used by the navigation handlers and
/// the new-window router: an action URL must only ever be decided
/// by [`dispatch`], never loaded or tab-opened directly.
pub fn is_action_url(url: &str) -> bool {
    let u = normalize_pico_url(url);
    let base = u.split(['?', '#']).next().unwrap_or("");
    let base = base.strip_suffix('/').unwrap_or(base);
    base == "pico://pick-file"
        || base == "pico://recent"
        || base.starts_with("pico://recent/")
        || base == "pico://social"
        || base.starts_with("pico://social/")
        || base == "pico://recent-remove"
        || base.starts_with("pico://recent-remove/")
        || base == "pico://recents-clear"
        || base == "pico://recents-visibility"
        || base.starts_with("pico://recents-visibility/")
        || base == "pico://stats-reset"
}

/// Best-effort action-kind label for the log line, derived from
/// the source alone (used even when the decision is a denial).
pub fn kind_label(source: &ActionSource) -> &'static str {
    match source {
        ActionSource::Url(u) => {
            let u = normalize_pico_url(u);
            if u.starts_with("pico://pick-") {
                "pick-file"
            } else if u.starts_with("pico://recent-remove") {
                "recent-remove"
            } else if u.starts_with("pico://recents-clear") {
                "recents-clear"
            } else if u.starts_with("pico://recents-visibility") {
                "recents-visibility"
            } else if u.starts_with("pico://stats-reset") {
                "stats-reset"
            } else if u.starts_with("pico://recent/") {
                "recent"
            } else if u.starts_with("pico://social/") {
                "social"
            } else if web_origin(&u).is_some() {
                "service"
            } else {
                "unknown"
            }
        }
        ActionSource::Ipc(body) => {
            // Substring scan is enough for a label; the real parse
            // happens in dispatch and its result is what is logged
            // as the decision.
            if body.contains("\"pick-file\"") {
                "pick-file"
            } else if body.contains("\"recent-remove\"") {
                "recent-remove"
            } else if body.contains("\"recents-clear\"") {
                "recents-clear"
            } else if body.contains("\"recents-visibility\"") {
                "recents-visibility"
            } else if body.contains("\"stats-reset\"") {
                "stats-reset"
            } else if body.contains("\"recent\"") {
                "recent"
            } else if body.contains("\"social\"") {
                "social"
            } else if body.contains("\"service\"") {
                "service"
            } else {
                "unknown"
            }
        }
    }
}

/// True when an IPC body *claims* to be a start-page action (its
/// `kind` field is `"action"`). The content IPC handler routes
/// such bodies to [`dispatch`] instead of the navigation-note
/// parser; the two families are disjoint by `kind`, and anything
/// misrouted is still decided by the gate.
pub fn looks_like_action(body: &str) -> bool {
    if body.len() > MAX_ACTION_BODY_BYTES {
        return false;
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(body) else {
        return false;
    };
    v.get("kind").and_then(|k| k.as_str()) == Some("action")
}

#[derive(Deserialize)]
struct IpcAction {
    kind: String,
    action: String,
    #[serde(default)]
    index: Option<serde_json::Value>,
    #[serde(default)]
    site: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    hidden: Option<bool>,
}

/// The `index` field shared by `recent` and `recent-remove`: a
/// JSON number, or a short string carrying one.
fn ipc_index(raw: &IpcAction) -> Result<String, DenyReason> {
    let idx = raw.index.clone().ok_or(DenyReason::Malformed)?;
    match idx.as_u64() {
        Some(n) => Ok(n.to_string()),
        // A string index is accepted too — the range and
        // parse checks downstream are identical either way.
        None => match idx.as_str() {
            Some(s) if !s.is_empty() && s.len() <= 20 => Ok(s.to_string()),
            _ => Err(DenyReason::Malformed),
        },
    }
}

fn parse_ipc(body: &str) -> Result<RawAction, DenyReason> {
    if body.len() > MAX_ACTION_BODY_BYTES {
        return Err(DenyReason::Malformed);
    }
    let raw: IpcAction = serde_json::from_str(body).map_err(|_| DenyReason::Malformed)?;
    if raw.kind != "action" {
        return Err(DenyReason::Malformed);
    }
    match raw.action.as_str() {
        "pick-file" => Ok(RawAction::PickFile),
        "recent" => Ok(RawAction::Recent(ipc_index(&raw)?)),
        "recent-remove" => Ok(RawAction::RecentRemove(ipc_index(&raw)?)),
        "recents-clear" => Ok(RawAction::RecentsClear),
        "stats-reset" => Ok(RawAction::StatsReset),
        "recents-visibility" => Ok(RawAction::RecentsVisibility(
            raw.hidden.ok_or(DenyReason::Malformed)?,
        )),
        "social" => Ok(RawAction::Social(raw.site.ok_or(DenyReason::Malformed)?)),
        "service" => Ok(RawAction::Service(raw.url.ok_or(DenyReason::Malformed)?)),
        _ => Err(DenyReason::UnknownAction),
    }
}

fn parse_url(raw: &str) -> Result<RawAction, DenyReason> {
    let url = normalize_pico_url(raw);
    let base = url.split(['?', '#']).next().unwrap_or("");
    let base = base.strip_suffix('/').unwrap_or(base);
    if base == "pico://pick-file" {
        return Ok(RawAction::PickFile);
    }
    if base == "pico://recents-clear" {
        return Ok(RawAction::RecentsClear);
    }
    if base == "pico://stats-reset" {
        return Ok(RawAction::StatsReset);
    }
    if base == "pico://recent"
        || base == "pico://social"
        || base == "pico://recent-remove"
        || base == "pico://recents-visibility"
    {
        // The family is named but its argument segment is missing
        // entirely — malformed, not merely unknown.
        return Err(DenyReason::Malformed);
    }
    if let Some(rest) = base.strip_prefix("pico://recent-remove/") {
        let seg = rest.split('/').next().unwrap_or("");
        if seg.is_empty() {
            return Err(DenyReason::Malformed);
        }
        return Ok(RawAction::RecentRemove(seg.to_string()));
    }
    if let Some(rest) = base.strip_prefix("pico://recents-visibility/") {
        let seg = rest.split('/').next().unwrap_or("");
        return match seg {
            "hide" => Ok(RawAction::RecentsVisibility(true)),
            "show" => Ok(RawAction::RecentsVisibility(false)),
            _ => Err(DenyReason::Malformed),
        };
    }
    if let Some(rest) = base.strip_prefix("pico://recent/") {
        let seg = rest.split('/').next().unwrap_or("");
        if seg.is_empty() {
            return Err(DenyReason::Malformed);
        }
        return Ok(RawAction::Recent(seg.to_string()));
    }
    if let Some(rest) = base.strip_prefix("pico://social/") {
        let seg = rest.split('/').next().unwrap_or("");
        if seg.is_empty() {
            return Err(DenyReason::Malformed);
        }
        return Ok(RawAction::Social(seg.to_string()));
    }
    if url.starts_with("pico://") {
        // Any other internal URL is not an action (the start and
        // chrome documents are served by the protocol handler and
        // never decided here).
        return Err(DenyReason::UnknownAction);
    }
    if web_origin(&url).is_some() {
        return Ok(RawAction::Service(url));
    }
    Err(DenyReason::Malformed)
}

/// Decide one start-page action. Pure: the same inputs always
/// give the same decision; `tests/action_tests.rs` pins the full
/// requester × action grid for both transports.
pub fn dispatch(source: &ActionSource, requester: &Requester, recents: &[String]) -> Decision {
    // The gate comes first and is conjunctive: start tab AND
    // start page. Transport parsing of a denied request never
    // runs, so a hostile page cannot probe the parser through
    // this path.
    if !requester.is_start_tab {
        return Decision::Deny(DenyReason::NotStartTab);
    }
    let current = normalize_pico_url(&requester.current_url);
    if current != START_URL && current != "pico://start/" {
        return Decision::Deny(DenyReason::NotOnStartPage);
    }
    let raw = match source {
        ActionSource::Url(u) => parse_url(u),
        ActionSource::Ipc(b) => parse_ipc(b),
    };
    let raw = match raw {
        Ok(r) => r,
        Err(reason) => return Decision::Deny(reason),
    };
    match raw {
        RawAction::PickFile => Decision::OpenPicker,
        RawAction::Recent(seg) => {
            let Ok(idx) = seg.parse::<usize>() else {
                return Decision::Deny(DenyReason::Malformed);
            };
            match recents.get(idx) {
                Some(p) => Decision::OpenRecent(PathBuf::from(p)),
                None => Decision::Deny(DenyReason::RecentOutOfRange),
            }
        }
        RawAction::RecentRemove(seg) => {
            let Ok(idx) = seg.parse::<usize>() else {
                return Decision::Deny(DenyReason::Malformed);
            };
            if idx < recents.len() {
                Decision::RemoveRecent(idx)
            } else {
                Decision::Deny(DenyReason::RecentOutOfRange)
            }
        }
        RawAction::RecentsClear => Decision::ClearRecents,
        RawAction::StatsReset => Decision::ResetStats,
        RawAction::RecentsVisibility(hidden) => Decision::SetRecentsHidden(hidden),
        RawAction::Social(name) => match SocialSite::from_wire(&name) {
            Some(site) => Decision::OpenSocial(site),
            None => Decision::Deny(DenyReason::UnknownSocialSite),
        },
        RawAction::Service(url) => {
            if url.len() <= MAX_SERVICE_URL_BYTES && web_origin(&url).is_some() {
                Decision::OpenService(url)
            } else {
                Decision::Deny(DenyReason::BadServiceUrl)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_url_classification() {
        assert!(is_action_url("pico://pick-file"));
        assert!(is_action_url("pico://recent/3"));
        assert!(is_action_url("pico://social/youtube"));
        assert!(is_action_url("pico://recent-remove/1"));
        assert!(is_action_url("pico://recents-clear"));
        assert!(is_action_url("pico://recents-visibility/hide"));
        assert!(is_action_url("pico://stats-reset"));
        assert!(is_action_url("https://pico.recent/3"));
        assert!(!is_action_url("pico://start"));
        assert!(!is_action_url("pico://chrome"));
        assert!(!is_action_url("https://example.com/recent/3"));
    }
}
