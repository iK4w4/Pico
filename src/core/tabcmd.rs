//! The tab-command vocabulary (0.41.0, Workstream C) —
//! the strict protocol for every movable-tab gesture the
//! chrome can ask the shell to perform: drag-to-reorder,
//! keyboard move, and the tab context menu's
//! open-in-a-new-Pico action. It mirrors `core::navcmd`
//! (and through it `core::dlcmd`) in shape and discipline:
//!
//! ```json
//! {"kind":"tab","action":"<kebab-verb>", ...args}
//! ```
//!
//! - The chrome sends it over the chrome transport (the
//!   `{v, boot, req}` envelope is stripped before this
//!   parser runs, and the body dedupes like every chrome
//!   command — see app.rs). No other surface exists: tab
//!   order is shell state, and only the shell's own strip
//!   may ask to change it.
//! - [`parse_tab_body`] is the ONLY parser. It is strict:
//!   `deny_unknown_fields`, a hard size cap, the kind tag
//!   must be exactly `"tab"`, and every argument is
//!   validated here so downstream code trusts the values.
//!   The [`TAB_COMMANDS`] table is the single source of
//!   truth for which verbs exist; app.rs checks
//!   `spec.allows(surface)` for every parsed body.
//!
//! ORDER OWNERSHIP: the app's `Vec<Tab>` is the one source
//! of truth for tab order. The strip is a renderer — a
//! drag names a NEIGHBOUR tab (see below) and asks; the
//! app reorders its vec with [`final_from_neighbor`] and
//! re-renders from the next snapshot. The dragged tab's
//! identity never changes, so the active tab (an id, not
//! an index) stays active and each tab's WebView follows
//! its record.
//!
//! GAP (insertion) INDICES: a drop position is a gap
//! between tabs, numbered `0..=len` — gap `g` sits BEFORE
//! the tab currently at index `g` (gap `len` is after the
//! last tab). [`insertion_to_final`] converts (old index,
//! gap) to the dragged tab's final index: removing the
//! tab shifts every later gap down by one, so a gap past
//! the old index lands one lower. Gaps `from` and
//! `from + 1` are the tab's own two edges — the identity.
//!
//! NAMED NEIGHBOURS (AUDIT 0.41.3, Front B B-3): through
//! 0.41.2 the drop verb carried the bare gap (`to`). A
//! gap is a position in an order the SENDER rendered —
//! if the shell's order moved between render and drop,
//! the same gap named a different landing spot and the
//! tab landed somewhere the user never pointed at. The
//! verb now carries `before`: the id of the tab the
//! dragged tab should land immediately before (absent /
//! null = the end of the strip). Ids name the same
//! landing spot in any order, and
//! [`final_from_neighbor`] converts (ids, dragged id,
//! neighbour id) to the final index with the same
//! remove-then-insert semantics as the gap math.

use serde::Deserialize;

use crate::core::ctxmenu::CtxItem;

/// Bodies larger than this are refused before parsing —
/// the largest legitimate body (a move with two u64s)
/// fits with room to spare.
pub const MAX_TAB_BODY_BYTES: usize = 8192;

/// Which surface a tab body arrived from. Only the chrome
/// exists today; the table keeps the dlcmd/navcmd shape
/// so a future surface is a table edit, not a redesign.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabSurface {
    /// The chrome (shell) transport.
    Chrome,
}

/// Argument shape of one verb (the table and the parser
/// must agree, and a test pins it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabArgSchema {
    /// `tab` (u64) + `before` (optional neighbour tab id,
    /// u64 or null; absent/null = end of strip).
    TabBefore,
    /// `tab` (u64) + `dir` (−1 or +1).
    TabDir,
    /// `tab` (u64) only.
    TabOnly,
}

/// One row of the vocabulary table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TabCommandSpec {
    pub name: &'static str,
    pub args: TabArgSchema,
    pub surfaces: &'static [TabSurface],
}

impl TabCommandSpec {
    pub fn allows(&self, surface: TabSurface) -> bool {
        self.surfaces.contains(&surface)
    }
}

use TabSurface::Chrome;

/// THE vocabulary. Order is documentation order: the
/// drag verb, the step verb (keyboard + menu), the
/// new-Pico verb (menu).
pub const TAB_COMMANDS: &[TabCommandSpec] = &[
    // Drag-to-reorder drop: move `tab` to immediately
    // before the neighbour tab `before` (named-neighbour
    // semantics in the module header; absent/null
    // `before` = the end of the strip).
    TabCommandSpec {
        name: "move",
        args: TabArgSchema::TabBefore,
        surfaces: &[Chrome],
    },
    // Move `tab` one position left (dir −1) or right
    // (dir +1), clamped at the ends — Ctrl+Shift+PageUp /
    // PageDown (which names the ACTIVE tab) and the tab
    // menu's Move Left / Move Right rows.
    TabCommandSpec {
        name: "move-step",
        args: TabArgSchema::TabDir,
        surfaces: &[Chrome],
    },
    // The tab menu's new-window action: launch a new Pico
    // process for `tab` (see [`new_pico_plan`] for when
    // the original then closes). The tab id is the ONLY
    // argument — the URL comes from the shell's own tab
    // record, never from the body.
    TabCommandSpec {
        name: "open-in-new-pico",
        args: TabArgSchema::TabOnly,
        surfaces: &[Chrome],
    },
];

/// Look up a verb's spec by its wire spelling.
pub fn spec_for(name: &str) -> Option<&'static TabCommandSpec> {
    TAB_COMMANDS.iter().find(|s| s.name == name)
}

/// A parsed, validated tab command. Constructed ONLY by
/// [`parse_tab_body`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TabCommand {
    /// Drag-to-reorder drop (AUDIT 0.41.3): land `tab`
    /// immediately before the neighbour tab `before`;
    /// `None` = the end of the strip.
    Move {
        tab: u64,
        before: Option<u64>,
    },
    MoveStep {
        tab: u64,
        dir: i32,
    },
    OpenInNewPico {
        tab: u64,
    },
}

impl TabCommand {
    /// The wire spelling (the log line's verb).
    pub fn name(&self) -> &'static str {
        match self {
            TabCommand::Move { .. } => "move",
            TabCommand::MoveStep { .. } => "move-step",
            TabCommand::OpenInNewPico { .. } => "open-in-new-pico",
        }
    }

    /// The tab every verb acts on (the log line's target).
    pub fn tab(&self) -> u64 {
        match self {
            TabCommand::Move { tab, .. }
            | TabCommand::MoveStep { tab, .. }
            | TabCommand::OpenInNewPico { tab } => *tab,
        }
    }
}

/// Why a tab body was refused. Each variant is a closed
/// log word (never body content), mirroring `NavDeny`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabDeny {
    /// Over the byte cap, or not JSON of the body shape.
    Malformed,
    /// The `kind` tag was not exactly `"tab"`.
    WrongKind,
    /// The action string names no verb in the table.
    UnknownAction,
    /// A required argument was missing or invalid
    /// (wrong key for the verb, dir not ±1, gap absurd).
    BadArgs,
}

impl TabDeny {
    pub fn as_str(&self) -> &'static str {
        match self {
            TabDeny::Malformed => "malformed",
            TabDeny::WrongKind => "wrong-kind",
            TabDeny::UnknownAction => "unknown-action",
            TabDeny::BadArgs => "bad-args",
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTab {
    kind: String,
    action: String,
    tab: Option<u64>,
    before: Option<u64>,
    dir: Option<i64>,
}

/// Parse and validate one tab body. The ONLY parser.
pub fn parse_tab_body(raw: &str) -> Result<TabCommand, TabDeny> {
    if raw.len() > MAX_TAB_BODY_BYTES {
        return Err(TabDeny::Malformed);
    }
    let parsed: RawTab = serde_json::from_str(raw).map_err(|_| TabDeny::Malformed)?;
    if parsed.kind != "tab" {
        return Err(TabDeny::WrongKind);
    }
    let spec = spec_for(&parsed.action).ok_or(TabDeny::UnknownAction)?;
    // AUDIT 0.41.1 (Front E F5): tab ids start at 1 —
    // dlcmd's `require_id` has always refused 0, this
    // parser accepted it (the shell's id lookup then
    // missed and the command died silently downstream).
    // Same rule, same place in the pipeline.
    // AUDIT 0.41.3: the `before` neighbour is a tab id
    // too — the same rule covers it when present.
    if parsed.tab == Some(0) || parsed.before == Some(0) {
        return Err(TabDeny::BadArgs);
    }
    // Argument-shape enforcement against the table: each
    // verb refuses the other verbs' keys and a missing
    // value of its own. (`move`'s `before` is the one
    // OPTIONAL argument in the vocabulary: absent and
    // null both mean the end of the strip, and serde's
    // `Option` already collapses the two spellings.)
    match spec.args {
        TabArgSchema::TabBefore => {
            if parsed.tab.is_none() || parsed.dir.is_some() {
                return Err(TabDeny::BadArgs);
            }
        }
        TabArgSchema::TabDir => {
            if parsed.tab.is_none() || parsed.dir.is_none() || parsed.before.is_some() {
                return Err(TabDeny::BadArgs);
            }
        }
        TabArgSchema::TabOnly => {
            if parsed.tab.is_none() || parsed.before.is_some() || parsed.dir.is_some() {
                return Err(TabDeny::BadArgs);
            }
        }
    }
    match parsed.action.as_str() {
        "move" => Ok(TabCommand::Move {
            tab: parsed.tab.unwrap_or(0),
            before: parsed.before,
        }),
        "move-step" => {
            let dir = parsed.dir.unwrap_or(0);
            if dir != -1 && dir != 1 {
                return Err(TabDeny::BadArgs);
            }
            Ok(TabCommand::MoveStep {
                tab: parsed.tab.unwrap_or(0),
                dir: dir as i32,
            })
        }
        "open-in-new-pico" => Ok(TabCommand::OpenInNewPico {
            tab: parsed.tab.unwrap_or(0),
        }),
        _ => Err(TabDeny::UnknownAction),
    }
}

/// Cheap routing pre-filter for chrome bodies (the
/// handler's kind dispatch): is this plausibly a tab
/// COMMAND body? Requires the `"action"` key, like the
/// nav pre-filter — the two families are told apart by
/// their `kind` values, and a body must claim BOTH the
/// tab kind and an action to route here.
pub fn looks_like_tab_body(body: &str) -> bool {
    let t = body.trim_start();
    if !t.starts_with('{') {
        return false;
    }
    let has_kind = t.contains("\"kind\"") && t.contains("\"tab\"");
    let has_action = t.contains("\"action\"");
    has_kind && has_action
}

/// The action spelling of a tab body for the log line,
/// without parsing (closed words only — the value comes
/// from the table or is `unknown`).
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

// ---------------------------------------------------------------------------
// Reorder math (pure)
// ---------------------------------------------------------------------------

/// Convert (list length, old index, gap index) to the
/// dragged element's FINAL index. `None` when any index
/// is out of range (`from >= len`, `insert > len`).
/// Gap semantics are the module header's: gaps `from`
/// and `from + 1` are the identity.
pub fn insertion_to_final(len: usize, from: usize, insert: usize) -> Option<usize> {
    if len == 0 || from >= len || insert > len {
        return None;
    }
    Some(if insert > from { insert - 1 } else { insert })
}

/// The new order of `ids` after moving the element at
/// `from` to gap `insert` (see [`insertion_to_final`]).
/// `None` on out-of-range input; the identity order for
/// the no-op gaps.
pub fn reorder_ids(ids: &[u64], from: usize, insert: usize) -> Option<Vec<u64>> {
    let fin = insertion_to_final(ids.len(), from, insert)?;
    let mut out = ids.to_vec();
    let id = out.remove(from);
    out.insert(fin, id);
    Some(out)
}

/// The final index of the dragged tab `from_id` when it
/// must land immediately before the neighbour tab
/// `before` (`None` = the end of the strip) — AUDIT
/// 0.41.3, the named-neighbour form of
/// [`insertion_to_final`]. Derived through the gap math
/// so the two can never disagree: the neighbour's
/// CURRENT index is the gap (the end of the strip is
/// gap `len`), and remove-then-insert does the rest —
/// including the identity cases (before the dragged tab
/// itself, or before its current right neighbour).
/// `None` when the dragged id or a named neighbour id
/// is not in `ids`.
pub fn final_from_neighbor(ids: &[u64], from_id: u64, before: Option<u64>) -> Option<usize> {
    let from = ids.iter().position(|&id| id == from_id)?;
    let gap = match before {
        None => ids.len(),
        Some(neighbour) => ids.iter().position(|&id| id == neighbour)?,
    };
    insertion_to_final(ids.len(), from, gap)
}

/// The final index for a one-step move (keyboard / menu):
/// `from + dir`, CLAMPED to the list — a step at an edge
/// is a no-op (the same index back), never a wrap (tab
/// CYCLING wraps; tab MOVING must not teleport a tab to
/// the far end). `None` for an invalid dir or index.
pub fn step_final(len: usize, from: usize, dir: i32) -> Option<usize> {
    if len == 0 || from >= len || (dir != -1 && dir != 1) {
        return None;
    }
    let next = from as i64 + dir as i64;
    Some(next.clamp(0, len as i64 - 1) as usize)
}

// ---------------------------------------------------------------------------
// The new-Pico decision (pure)
// ---------------------------------------------------------------------------

/// The facts about one tab the new-Pico decision is
/// allowed to see — assembled by the app from its own
/// tab record, never from the requesting body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NewPicoFacts {
    /// A Pico internal page tab (start / downloads /
    /// history): its "URL" is a `pico://` spelling the
    /// new process's argv parser refuses by design.
    pub is_internal: bool,
    /// The tab's URL is a plain web URL (http/https with
    /// an origin) a new process can open from argv.
    pub is_web: bool,
    /// The tab is served by THIS process's local file
    /// server: its URL embeds this server's port + root
    /// token, which a new process does not have.
    pub is_local_server: bool,
    /// The URL carries userinfo (credentials): it must
    /// never cross a process boundary on a command line.
    pub has_userinfo: bool,
    /// The URL's query or fragment carries a secret-
    /// looking parameter (`?token=`, `#access_token=`,
    /// a signature): the same command-line exposure as
    /// userinfo, one spelling over (AUDIT 0.41.1,
    /// Front C F2 — through 0.41.0 only userinfo was
    /// kept off argv).
    pub has_query_secret: bool,
    /// A download capture is mid-flight on this tab (the
    /// coordinator is still awaiting its cookies): the
    /// capture's tab fate belongs to THIS process.
    pub has_pending_capture: bool,
    /// The tab's WebView failed to build: there is no
    /// live page state to "move", only a URL to retry.
    pub failed: bool,
}

/// What launching a new Pico for a tab means. The label
/// the menu shows follows the plan (see
/// [`tab_menu_items`]): only `MoveAndClose` may be
/// labelled "Move to New Window".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewPicoPlan {
    /// Plain web tab, nothing in flight: launch with the
    /// tab's URL, then close the original — a true move.
    MoveAndClose,
    /// Launch with the tab's URL, keep the original (a
    /// capture is in flight, or the tab is a failed shell
    /// whose retry state stays here).
    OpenWithUrl,
    /// Launch with NO argument (the new process opens
    /// its own start tab), keep the original — internal
    /// pages cannot be handed over by URL.
    OpenBare,
    /// Do not offer the action at all: the URL cannot
    /// leave this process (local-server token, userinfo)
    /// or names nothing openable.
    NotOffered,
}

/// Decide the new-Pico plan for one tab from its facts.
/// The order of the checks is the safety order: a URL
/// that cannot cross processes refuses outright before
/// any closable reasoning runs.
pub fn new_pico_plan(f: &NewPicoFacts) -> NewPicoPlan {
    if f.is_local_server || f.has_userinfo || f.has_query_secret {
        return NewPicoPlan::NotOffered;
    }
    if f.is_internal {
        return NewPicoPlan::OpenBare;
    }
    if !f.is_web {
        return NewPicoPlan::NotOffered;
    }
    if f.has_pending_capture || f.failed {
        return NewPicoPlan::OpenWithUrl;
    }
    NewPicoPlan::MoveAndClose
}

/// Does this URL's query or fragment carry a
/// secret-looking parameter? Matched against each
/// `key=value` parameter's key, in the query AND the
/// fragment (OAuth implicit flow puts `access_token` in
/// the fragment). A value's presence is not required —
/// the key alone marks the URL as carrying credential
/// material that must not ride a command line.
///
/// Two matches (AUDIT 0.41.3, Front B B-2 adds the
/// second): the whole key, lowercased, in the exact
/// list — or any SEGMENT of the key (split on `_` and
/// `-`, lowercased) in the segment list. The segment
/// rule is what catches the composed names real
/// services use (`refresh_token`, `client_secret`,
/// `x-amz-signature`); segment equality, never
/// substring, is what keeps `author` (one segment,
/// "author") and `design` (one segment, "design" —
/// despite its "sig" substring) OUT.
pub fn has_query_secret(url: &str) -> bool {
    const SECRET_KEYS: [&str; 14] = [
        "token",
        "access_token",
        "api_key",
        "apikey",
        "key",
        "auth",
        "password",
        "passwd",
        "secret",
        "signature",
        "sig",
        "session",
        "session_id",
        "sessionid",
    ];
    const SECRET_SEGMENTS: [&str; 13] = [
        "token",
        "secret",
        "sig",
        "signature",
        "auth",
        "code",
        "session",
        "passwd",
        "password",
        "apikey",
        "credential",
        "credentials",
        "key",
    ];
    let mut parts: Vec<&str> = Vec::new();
    if let Some(q) = url.split_once('?').map(|(_, q)| q) {
        parts.push(q.split('#').next().unwrap_or(q));
    }
    if let Some(f) = url.split_once('#').map(|(_, f)| f) {
        parts.push(f);
    }
    parts.iter().any(|part| {
        part.split('&').any(|param| {
            let key = param.split('=').next().unwrap_or(param);
            let lower = key.to_ascii_lowercase();
            SECRET_KEYS.contains(&lower.as_str())
                || lower
                    .split(['_', '-'])
                    .any(|seg| SECRET_SEGMENTS.contains(&seg))
        })
    })
}

/// Assemble a tab's new-Pico facts from its URL and the
/// shell's raw observations (AUDIT 0.41.1, Front C
/// F1+F3). Through 0.41.0 the assembly lived inline in
/// the shell, untested at behavior level — and it
/// classified local-server tabs by exact prefix match
/// only, missing the parsed alias-aware served check
/// (`core::server::is_served_url`) the Teleport path
/// uses: a tab on `http://localhost:<port>/r/<token>/…`
/// (the alias spelling of Pico's own file server) was
/// classified as plain web, offered Move to New Window,
/// and its token URL put on the new process's command
/// line while the original closed — a dead page and a
/// leaked root token. The assembly is pure core now;
/// the shell only gathers (`is_internal`, whether the
/// tab holds a server root id, whether that root
/// matches the URL exactly, this server's port, the
/// capture/failed flags).
#[allow(clippy::too_many_arguments)]
pub fn new_pico_facts_for(
    url: &str,
    is_internal: bool,
    root_id_present: bool,
    root_matches_url: bool,
    server_port: Option<u16>,
    has_pending_capture: bool,
    failed: bool,
) -> NewPicoFacts {
    NewPicoFacts {
        is_internal,
        is_web: crate::core::addr::web_origin(url).is_some(),
        is_local_server: root_id_present
            || root_matches_url
            || server_port.is_some_and(|port| crate::core::server::is_served_url(url, port)),
        has_userinfo: crate::core::ipc::url_has_userinfo(url),
        has_query_secret: has_query_secret(url),
        has_pending_capture,
        failed,
    }
}

// ---------------------------------------------------------------------------
// The tab context menu model (pure)
// ---------------------------------------------------------------------------

/// The tab menu's item ids. NOT `core::ctxmenu::CTX_IDS`:
/// that whitelist belongs to the content menu and its
/// `ctx_pick` command. Tab-menu picks cross as tab
/// commands (`move-step` / `open-in-new-pico`); these ids
/// are the chrome renderer's row names, pinned here so
/// the model and the renderer cannot drift.
pub const TAB_MENU_MOVE_LEFT: &str = "move_left";
pub const TAB_MENU_MOVE_RIGHT: &str = "move_right";
pub const TAB_MENU_OPEN_NEW_PICO: &str = "open_new_pico";

/// The tab context menu for the tab at `index` in a strip
/// of `len`, under new-Pico `plan`: Move Left / Move
/// Right (enabled only when a neighbour exists that way)
/// and — unless the plan is `NotOffered` — the new-Pico
/// row, labelled honestly for the plan: "Move to New
/// Window" only when the original will actually close;
/// "Open in New Pico" whenever it stays. Rows reuse
/// `core::ctxmenu::CtxItem` so the chrome renders both of
/// Pico's menus from one shape and `core::layout` prices
/// both with one function.
pub fn tab_menu_items(index: usize, len: usize, plan: NewPicoPlan) -> Vec<CtxItem> {
    let row = |id: &'static str, label: &'static str, enabled: bool| CtxItem {
        id,
        label,
        shortcut: None,
        enabled,
        sep: false,
    };
    let mut out = vec![
        row(TAB_MENU_MOVE_LEFT, "Move Left", index > 0),
        row(
            TAB_MENU_MOVE_RIGHT,
            "Move Right",
            len > 0 && index + 1 < len,
        ),
    ];
    match plan {
        NewPicoPlan::MoveAndClose => {
            out.push(CtxItem::sep());
            out.push(row(TAB_MENU_OPEN_NEW_PICO, "Move to New Window", true));
        }
        NewPicoPlan::OpenWithUrl | NewPicoPlan::OpenBare => {
            out.push(CtxItem::sep());
            out.push(row(TAB_MENU_OPEN_NEW_PICO, "Open in New Pico", true));
        }
        NewPicoPlan::NotOffered => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_complete_and_unique() {
        let mut names: Vec<&str> = TAB_COMMANDS.iter().map(|s| s.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), TAB_COMMANDS.len(), "verb names unique");
        assert_eq!(TAB_COMMANDS.len(), 3, "the 0.41.0 tab vocabulary");
        for spec in TAB_COMMANDS {
            assert_eq!(spec_for(spec.name).map(|s| s.name), Some(spec.name));
            assert!(spec.allows(TabSurface::Chrome), "{} is chrome", spec.name);
        }
    }

    #[test]
    fn parse_matrix() {
        // AUDIT 0.41.3: the drop verb names its neighbour
        // (`before`), never a bare gap.
        assert_eq!(
            parse_tab_body(r#"{"kind":"tab","action":"move","tab":7,"before":9}"#),
            Ok(TabCommand::Move {
                tab: 7,
                before: Some(9)
            })
        );
        // Absent and null `before` both mean the end.
        assert_eq!(
            parse_tab_body(r#"{"kind":"tab","action":"move","tab":7}"#),
            Ok(TabCommand::Move {
                tab: 7,
                before: None
            })
        );
        assert_eq!(
            parse_tab_body(r#"{"kind":"tab","action":"move","tab":7,"before":null}"#),
            Ok(TabCommand::Move {
                tab: 7,
                before: None
            })
        );
        // The old gap field is gone: `to` is an unknown
        // field now (deny_unknown_fields), and a zero
        // neighbour id is refused like a zero tab id.
        assert_eq!(
            parse_tab_body(r#"{"kind":"tab","action":"move","tab":7,"to":2}"#),
            Err(TabDeny::Malformed)
        );
        assert_eq!(
            parse_tab_body(r#"{"kind":"tab","action":"move","tab":7,"before":0}"#),
            Err(TabDeny::BadArgs)
        );
        assert_eq!(
            parse_tab_body(r#"{"kind":"tab","action":"move-step","tab":7,"dir":-1}"#),
            Ok(TabCommand::MoveStep { tab: 7, dir: -1 })
        );
        assert_eq!(
            parse_tab_body(r#"{"kind":"tab","action":"open-in-new-pico","tab":7}"#),
            Ok(TabCommand::OpenInNewPico { tab: 7 })
        );
        // Strictness: unknown field refused (deny_unknown_fields).
        assert_eq!(
            parse_tab_body(
                r#"{"kind":"tab","action":"move","tab":7,"to":2,"url":"https://e.com/"}"#
            ),
            Err(TabDeny::Malformed)
        );
        // Wrong kind, unknown action.
        assert_eq!(
            parse_tab_body(r#"{"kind":"nav","action":"move","tab":7,"before":2}"#),
            Err(TabDeny::WrongKind)
        );
        assert_eq!(
            parse_tab_body(r#"{"kind":"tab","action":"teleport","tab":7}"#),
            Err(TabDeny::UnknownAction)
        );
        // Arg shape: each verb refuses the others' keys
        // and its own missing value. (`move` alone is NOT
        // here: since 0.41.3 a missing `before` is the
        // valid end-of-strip drop, pinned above.)
        assert_eq!(
            parse_tab_body(r#"{"kind":"tab","action":"move","dir":1}"#),
            Err(TabDeny::BadArgs)
        );
        assert_eq!(
            parse_tab_body(r#"{"kind":"tab","action":"move","tab":7,"before":2,"dir":1}"#),
            Err(TabDeny::BadArgs)
        );
        assert_eq!(
            parse_tab_body(r#"{"kind":"tab","action":"move-step","tab":7,"before":1}"#),
            Err(TabDeny::BadArgs)
        );
        assert_eq!(
            parse_tab_body(r#"{"kind":"tab","action":"open-in-new-pico","tab":7,"before":1}"#),
            Err(TabDeny::BadArgs)
        );
        assert_eq!(
            parse_tab_body(r#"{"kind":"tab","action":"open-in-new-pico"}"#),
            Err(TabDeny::BadArgs)
        );
        // Dir must be exactly ±1.
        assert_eq!(
            parse_tab_body(r#"{"kind":"tab","action":"move-step","tab":7,"dir":2}"#),
            Err(TabDeny::BadArgs)
        );
        assert_eq!(
            parse_tab_body(r#"{"kind":"tab","action":"move-step","tab":7,"dir":0}"#),
            Err(TabDeny::BadArgs)
        );
        // A negative or fractional tab id is not a u64:
        // malformed at the serde layer.
        assert_eq!(
            parse_tab_body(r#"{"kind":"tab","action":"move","tab":-1,"before":2}"#),
            Err(TabDeny::Malformed)
        );
        // Over the byte cap.
        let big = format!(
            "{{\"kind\":\"tab\",\"action\":\"move\",\"tab\":7,\"before\":2,\"pad\":\"{}\"}}",
            "a".repeat(MAX_TAB_BODY_BYTES)
        );
        assert_eq!(parse_tab_body(&big), Err(TabDeny::Malformed));
    }

    #[test]
    fn routing_prefilter_requires_kind_and_action() {
        assert!(looks_like_tab_body(
            r#"{"kind":"tab","action":"move","tab":7,"before":2}"#
        ));
        assert!(!looks_like_tab_body(r#"{"kind":"tab","tab":7}"#));
        assert!(!looks_like_tab_body(
            r#"{"kind":"nav","action":"bookmark-toggle"}"#
        ));
        assert!(!looks_like_tab_body("not json"));
    }

    #[test]
    fn insertion_math_identity_and_ends() {
        // Identity: the tab's own two gaps.
        assert_eq!(insertion_to_final(4, 1, 1), Some(1));
        assert_eq!(insertion_to_final(4, 1, 2), Some(1));
        // To the very front / very end.
        assert_eq!(insertion_to_final(4, 2, 0), Some(0));
        assert_eq!(insertion_to_final(4, 1, 4), Some(3));
        assert_eq!(insertion_to_final(4, 0, 4), Some(3));
        // Forward move: gap past `from` lands one lower.
        assert_eq!(insertion_to_final(5, 1, 4), Some(3));
        // Backward move: gap at or before `from` is literal.
        assert_eq!(insertion_to_final(5, 3, 1), Some(1));
        // The only tab: both gaps are the identity.
        assert_eq!(insertion_to_final(1, 0, 0), Some(0));
        assert_eq!(insertion_to_final(1, 0, 1), Some(0));
        // Out of range.
        assert_eq!(insertion_to_final(0, 0, 0), None);
        assert_eq!(insertion_to_final(3, 3, 0), None);
        assert_eq!(insertion_to_final(3, 0, 4), None);
    }

    #[test]
    fn reorder_ids_moves_the_element_and_nothing_else() {
        let ids = [10, 20, 30, 40];
        assert_eq!(reorder_ids(&ids, 0, 3), Some(vec![20, 30, 10, 40]));
        assert_eq!(reorder_ids(&ids, 3, 0), Some(vec![40, 10, 20, 30]));
        assert_eq!(reorder_ids(&ids, 1, 4), Some(vec![10, 30, 40, 20]));
        assert_eq!(reorder_ids(&ids, 1, 1), Some(ids.to_vec()));
        assert_eq!(reorder_ids(&ids, 1, 2), Some(ids.to_vec()));
        assert_eq!(reorder_ids(&ids, 0, 4), Some(vec![20, 30, 40, 10]));
        assert_eq!(reorder_ids(&[7], 0, 1), Some(vec![7]));
        assert_eq!(reorder_ids(&ids, 4, 0), None);
        assert_eq!(reorder_ids(&ids, 0, 5), None);
        // Every (from, gap) pair is a permutation of the input.
        for from in 0..ids.len() {
            for gap in 0..=ids.len() {
                let out = reorder_ids(&ids, from, gap).unwrap();
                let mut sorted = out.clone();
                sorted.sort_unstable();
                assert_eq!(sorted, ids.to_vec(), "from={from} gap={gap}");
                assert_eq!(out[insertion_to_final(4, from, gap).unwrap()], ids[from]);
            }
        }
    }

    #[test]
    fn final_from_neighbor_matches_remove_and_insert_before() {
        // AUDIT 0.41.3: brute-force over every permutation
        // of four tabs, every dragged tab, every neighbour
        // (and the end): remove the dragged tab, insert it
        // immediately before the neighbour in the REMAINING
        // list (before itself = the identity; the end =
        // push), and compare the landing index.
        fn permutations(ids: &[u64]) -> Vec<Vec<u64>> {
            if ids.len() <= 1 {
                return vec![ids.to_vec()];
            }
            let mut out = Vec::new();
            for i in 0..ids.len() {
                let mut rest: Vec<u64> = ids.to_vec();
                let head = rest.remove(i);
                for mut tail in permutations(&rest) {
                    let mut p = vec![head];
                    p.append(&mut tail);
                    out.push(p);
                }
            }
            out
        }
        for ids in permutations(&[10, 20, 30, 40]) {
            for &from_id in &ids {
                let mut neighbours: Vec<Option<u64>> = ids.iter().map(|&id| Some(id)).collect();
                neighbours.push(None);
                for before in neighbours {
                    let want = {
                        let mut sim: Vec<u64> = ids.clone();
                        let from = sim.iter().position(|&id| id == from_id).unwrap();
                        sim.remove(from);
                        match before {
                            None => sim.push(from_id),
                            Some(b) if b == from_id => {
                                sim.insert(from, from_id);
                            }
                            Some(b) => {
                                let at = sim.iter().position(|&id| id == b).unwrap();
                                sim.insert(at, from_id);
                            }
                        }
                        sim.iter().position(|&id| id == from_id).unwrap()
                    };
                    assert_eq!(
                        final_from_neighbor(&ids, from_id, before),
                        Some(want),
                        "ids={ids:?} from={from_id} before={before:?}"
                    );
                }
            }
        }
        // Unknown ids name nothing.
        assert_eq!(final_from_neighbor(&[1, 2], 9, None), None);
        assert_eq!(final_from_neighbor(&[1, 2], 1, Some(9)), None);
        assert_eq!(final_from_neighbor(&[], 1, None), None);
        // The only tab, either neighbour spelling: index 0.
        assert_eq!(final_from_neighbor(&[7], 7, None), Some(0));
        assert_eq!(final_from_neighbor(&[7], 7, Some(7)), Some(0));
    }

    #[test]
    fn query_secret_segment_matching() {
        // AUDIT 0.41.3 (Front B B-2): composed keys block
        // on any secret SEGMENT; innocent keys whose
        // segments merely CONTAIN a secret word do not.
        for u in [
            "https://example.com/cb?refresh_token=abc",
            "https://example.com/cb?id_token=abc",
            "https://example.com/cb?client_secret=abc",
            "https://example.com/dl?x-amz-signature=abc",
            "https://example.com/cb?code=abc",
            "https://example.com/cb?CODE=abc",
            "https://example.com/cb?my-credentials=abc",
            "https://example.com/app#refresh_token=abc",
        ] {
            assert!(has_query_secret(u), "{u} must block");
        }
        for u in [
            "https://example.com/?author=abc",
            "https://example.com/?design=abc",
            "https://example.com/?monkey=abc",
            "https://example.com/?signaturex=abc",
            "https://example.com/?tokenize=abc",
        ] {
            assert!(!has_query_secret(u), "{u} must NOT block");
        }
    }

    #[test]
    fn step_final_clamps_at_the_ends_and_never_wraps() {
        assert_eq!(step_final(4, 1, -1), Some(0));
        assert_eq!(step_final(4, 1, 1), Some(2));
        assert_eq!(step_final(4, 0, -1), Some(0), "left edge: no-op");
        assert_eq!(step_final(4, 3, 1), Some(3), "right edge: no-op");
        assert_eq!(step_final(1, 0, 1), Some(0), "the only tab");
        assert_eq!(step_final(4, 1, 0), None);
        assert_eq!(step_final(4, 1, 2), None);
        assert_eq!(step_final(0, 0, 1), None);
        assert_eq!(step_final(4, 4, 1), None);
    }

    fn facts() -> NewPicoFacts {
        NewPicoFacts {
            is_internal: false,
            is_web: true,
            is_local_server: false,
            has_userinfo: false,
            has_query_secret: false,
            has_pending_capture: false,
            failed: false,
        }
    }

    #[test]
    fn new_pico_plan_matrix() {
        // The plain web tab: a true move.
        assert_eq!(new_pico_plan(&facts()), NewPicoPlan::MoveAndClose);
        // Mid-capture or failed: open, but keep the original.
        let mut f = facts();
        f.has_pending_capture = true;
        assert_eq!(new_pico_plan(&f), NewPicoPlan::OpenWithUrl);
        let mut f = facts();
        f.failed = true;
        assert_eq!(new_pico_plan(&f), NewPicoPlan::OpenWithUrl);
        // Internal pages hand over nothing: bare launch.
        let mut f = facts();
        f.is_internal = true;
        f.is_web = false;
        assert_eq!(new_pico_plan(&f), NewPicoPlan::OpenBare);
        // A local-server URL or a credentialed URL never
        // crosses — even when everything else is plain.
        let mut f = facts();
        f.is_local_server = true;
        assert_eq!(new_pico_plan(&f), NewPicoPlan::NotOffered);
        let mut f = facts();
        f.has_userinfo = true;
        assert_eq!(new_pico_plan(&f), NewPicoPlan::NotOffered);
        // Not web, not internal: nothing openable.
        let mut f = facts();
        f.is_web = false;
        assert_eq!(new_pico_plan(&f), NewPicoPlan::NotOffered);
    }

    #[test]
    fn tab_menu_items_labels_follow_the_plan() {
        let ids = |items: &[CtxItem]| items.iter().map(|i| i.id).collect::<Vec<_>>();
        // Middle tab, closable: both moves enabled, the
        // honest "Move to New Window" label.
        let items = tab_menu_items(1, 3, NewPicoPlan::MoveAndClose);
        assert_eq!(
            ids(&items),
            [
                TAB_MENU_MOVE_LEFT,
                TAB_MENU_MOVE_RIGHT,
                "",
                TAB_MENU_OPEN_NEW_PICO
            ]
        );
        assert_eq!(items[3].label, "Move to New Window");
        assert!(items.iter().all(|i| i.enabled || i.sep));
        // Keep-the-original plans: "Open in New Pico".
        for plan in [NewPicoPlan::OpenWithUrl, NewPicoPlan::OpenBare] {
            let items = tab_menu_items(1, 3, plan);
            assert_eq!(items[3].label, "Open in New Pico", "{plan:?}");
        }
        // NotOffered: no new-Pico row at all, no separator.
        let items = tab_menu_items(1, 3, NewPicoPlan::NotOffered);
        assert_eq!(ids(&items), [TAB_MENU_MOVE_LEFT, TAB_MENU_MOVE_RIGHT]);
        // Edges: the outward move disables, never vanishes.
        let items = tab_menu_items(0, 3, NewPicoPlan::MoveAndClose);
        assert!(!items[0].enabled && items[1].enabled);
        let items = tab_menu_items(2, 3, NewPicoPlan::MoveAndClose);
        assert!(items[0].enabled && !items[1].enabled);
        // The only tab: both moves disabled, open still offered.
        let items = tab_menu_items(0, 1, NewPicoPlan::MoveAndClose);
        assert!(!items[0].enabled && !items[1].enabled && items[3].enabled);
    }
}
