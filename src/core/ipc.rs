//! Typed JSON IPC protocol between the chrome page, content
//! pages and Rust. Protocol **version 1** (the `v` field; absent
//! means 1).
//!
//! Protocol shape:
//! - Chrome → Rust commands are JSON objects:
//!   `{"v": 1, "boot": "<random per page load>", "req": <monotonic
//!   u64>, "t": "<command>", ...fields}`. Every command carries
//!   the page's boot id and a request id; Rust dedupes on
//!   `(boot, req)` so a double-fired click can never run twice.
//!   A command announcing a newer protocol version than
//!   [`PROTOCOL_VERSION`] is rejected, never guessed at.
//! - Content → Rust navigation notes come from the injected
//!   observer script ([`NAV_OBSERVER_SCRIPT`]):
//!   `{"kind":"nav","how":"push|replace|pop|init","href":…,"title":…}`.
//! - Rust → chrome is always a full serde-serialized state
//!   snapshot (never string-formatted user text), pushed only
//!   when it changes.
//!
//! This module is pure parsing/state — fully host-testable.

use serde::Deserialize;
use std::collections::{HashSet, VecDeque};

/// The IPC protocol version this build speaks.
pub const PROTOCOL_VERSION: u64 = 1;

/// Hard caps that keep a hostile or malfunctioning page from
/// making the host allocate or scan unbounded input.
pub const MAX_COMMAND_BODY_BYTES: usize = 64 * 1024;
pub const MAX_NAV_BODY_BYTES: usize = 16 * 1024;
pub const MAX_BOOT_LEN: usize = 128;
pub const MAX_NAVIGATE_TEXT: usize = 8192;
pub const MAX_HREF_LEN: usize = 8192;
pub const MAX_TITLE_CHARS: usize = 512;

/// A social destination the user can open from Pico's branding
/// surfaces (v15). An *enum*, never a URL: the wire carries only
/// `telegram` or `youtube`, and Rust maps the variant to one of
/// the two constant URLs below. No arbitrary URL string can cross
/// IPC for this feature, and serde rejects unknown variants, so
/// a compromised or confused sender cannot turn the branding
/// links into an open-redirect primitive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SocialSite {
    Telegram,
    Youtube,
}

impl SocialSite {
    /// The constant URL this variant opens. These are the ONLY
    /// two social URLs in the program (the user's own links).
    pub fn url(self) -> &'static str {
        match self {
            SocialSite::Telegram => "https://t.me/iK4w4",
            SocialSite::Youtube => "https://www.youtube.com/@iK4w4",
        }
    }

    /// Parse the wire name (`telegram` / `youtube`) — used by the
    /// start-page action channel, which shares this enum
    /// discipline. Anything else returns `None`.
    pub fn from_wire(name: &str) -> Option<SocialSite> {
        match name {
            "telegram" => Some(SocialSite::Telegram),
            "youtube" => Some(SocialSite::Youtube),
            _ => None,
        }
    }
}

/// A command sent by the chrome page. Parsed from the `t` field.
/// Every variant is matched by the app layer's command handler —
/// `tests/addr_ipc.rs` pins the parse side, and the DOM suite
/// pins that every chrome control emits one of these.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Command {
    /// Chrome finished loading; answer with a full snapshot.
    Ready,
    NewTab,
    /// Open one of the author's social pages in a new tab.
    /// Carries only the [`SocialSite`] enum.
    Social {
        site: SocialSite,
    },
    CloseTab {
        tab: u64,
    },
    SwitchTab {
        tab: u64,
    },
    CycleTab {
        dir: i32,
    },
    Navigate {
        text: String,
    },
    Back,
    Forward,
    Reload,
    Home,
    OpenFile,
    ThemeToggle,
    /// Toggle the Shield (ad blocker) on/off. The new state is
    /// persisted; the shell re-renders from the next snapshot.
    ShieldToggle,
    /// Toggle Lite Mode (v25) on/off. Persisted like the Shield's
    /// flag; the shell re-renders from the next snapshot.
    LiteToggle,
    /// Phone Teleport (v25): ask the app to resolve the active
    /// tab's URL for the phone and answer (via the next snapshot)
    /// with the QR payload. A chrome command — the chrome page is
    /// privileged by construction, so unlike start-page actions
    /// this needs no extra gate (AUDIT V25).
    Teleport,
    /// The Teleport panel was dismissed (v25): the app restores
    /// the plain chrome bounds and drops the payload.
    TeleportClose,
    /// The panel's Copy button was used (v25): the clipboard work
    /// happens in the page; this command exists so the action is
    /// logged like every other teleport event.
    TeleportCopy,
    /// Open the theme picker panel (v27). A chrome command like
    /// Teleport's: the app opens the centered panel (closing
    /// Teleport's — the two panels are mutually exclusive) and
    /// the next snapshot carries the palette catalogue.
    Palette,
    /// The theme picker was dismissed (v27): restore the plain
    /// chrome bounds.
    PaletteClose,
    /// Apply a palette by id (v27). The id is validated against
    /// the core palette table by the app; an unknown id is
    /// logged and ignored, never persisted.
    ThemeSet {
        id: String,
    },
    WinMin,
    WinMax,
    WinDrag,
    WinClose,
}

/// A parsed chrome command with its dedupe identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope {
    pub boot: String,
    pub req: u64,
    pub cmd: Command,
}

/// The allowed object keys for one command variant
/// (AUDIT 0.31, A1-F5): the common envelope keys (`t`, `v`,
/// `boot`, `req`) plus that variant's own fields — and
/// nothing else. `None` for an unknown `t` (serde rejects
/// those too; this check runs first so the error names the
/// real problem). The [`Command`] enum cannot carry
/// `deny_unknown_fields` (it is internally tagged, where
/// serde forbids it on the enum), so the check is done by
/// hand against the parsed JSON object.
fn allowed_command_keys(t: &str) -> Option<&'static [&'static str]> {
    const COMMON: &[&str] = &["t", "v", "boot", "req"];
    const SOCIAL: &[&str] = &["t", "v", "boot", "req", "site"];
    const TAB: &[&str] = &["t", "v", "boot", "req", "tab"];
    const DIR: &[&str] = &["t", "v", "boot", "req", "dir"];
    const TEXT: &[&str] = &["t", "v", "boot", "req", "text"];
    const ID: &[&str] = &["t", "v", "boot", "req", "id"];
    match t {
        "ready" | "new_tab" | "back" | "forward" | "reload" | "home" | "open_file"
        | "theme_toggle" | "shield_toggle" | "lite_toggle" | "teleport" | "teleport_close"
        | "teleport_copy" | "palette" | "palette_close" | "win_min" | "win_max" | "win_drag"
        | "win_close" => Some(COMMON),
        "social" => Some(SOCIAL),
        "close_tab" | "switch_tab" => Some(TAB),
        "cycle_tab" => Some(DIR),
        "navigate" => Some(TEXT),
        "theme_set" => Some(ID),
        _ => None,
    }
}

/// May a same-tab navigation to `url` proceed when it
/// carries userinfo? (AUDIT 0.31, A6-F2.) A URL WITHOUT
/// userinfo is not this predicate's concern (`true`). A
/// credentialed URL is allowed ONLY when it is byte-exactly
/// the shell's one-shot expectation for that tab — the URL
/// the user themselves typed and submitted — because typed
/// credentialed URLs are the documented user's-own-act
/// exception; a page minting the same shape gets no match.
pub fn userinfo_nav_allowed(url: &str, expected: Option<&str>) -> bool {
    let has_userinfo = match url.split_once("://") {
        Some((_, rest)) => {
            let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
            authority.contains('@')
        }
        None => false,
    };
    if !has_userinfo {
        return true;
    }
    expected == Some(url)
}

/// Parse one raw IPC message body from the chrome webview.
pub fn parse_command(body: &str) -> Result<Envelope, String> {
    if body.len() > MAX_COMMAND_BODY_BYTES {
        return Err(format!("command body too large ({} bytes)", body.len()));
    }
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|e| format!("bad json: {e}"))?;
    // Protocol version: absent = 1 (the original shape). Anything
    // from the future is rejected rather than misinterpreted.
    if let Some(v) = value.get("v") {
        match v.as_u64() {
            Some(n) if n == PROTOCOL_VERSION => {}
            _ => return Err("unsupported protocol version".to_string()),
        }
    }
    let boot = value
        .get("boot")
        .and_then(|b| b.as_str())
        .ok_or("missing boot id")?
        .to_string();
    if boot.is_empty() || boot.len() > MAX_BOOT_LEN {
        return Err("boot id out of range".to_string());
    }
    let req = value
        .get("req")
        .and_then(|r| r.as_u64())
        .ok_or("missing req id")?;
    // AUDIT 0.31 (A1-F5): reject unknown fields. The report
    // parsers below deny unknown fields; commands did not —
    // the internally-tagged enum silently IGNORED extra keys,
    // so a sender could smuggle a field the handler never
    // validated (and a future variant field could be spoofed
    // into an older handler's blind spot). Check the object's
    // keys against the variant's allowed set before parsing.
    {
        let t = value.get("t").and_then(|t| t.as_str()).unwrap_or("");
        let allowed = allowed_command_keys(t).ok_or("unknown command")?;
        if let Some(obj) = value.as_object() {
            for key in obj.keys() {
                if !allowed.contains(&key.as_str()) {
                    return Err(format!("unknown field for command {t}: {key}"));
                }
            }
        }
    }
    let cmd: Command = serde_json::from_value(value).map_err(|e| format!("bad command: {e}"))?;
    if let Command::Navigate { text } = &cmd {
        if text.len() > MAX_NAVIGATE_TEXT {
            return Err("navigate text too large".to_string());
        }
    }
    if let Command::ThemeSet { id } = &cmd {
        // Palette ids are short slugs ("forest"); anything
        // longer is not a palette name, whatever it claims.
        if id.is_empty() || id.len() > 32 {
            return Err("theme id out of range".to_string());
        }
    }
    if let Command::CycleTab { dir } = &cmd {
        // The chrome only ever sends ±1 (AUDIT V28 A1-L3): any
        // other value is a bug or an attack, and feeding it raw
        // into the tab-index arithmetic could jump arbitrarily
        // (or overflow in debug builds).
        if *dir != -1 && *dir != 1 {
            return Err("cycle dir out of range".to_string());
        }
    }
    Ok(Envelope { boot, req, cmd })
}

/// Bounded dedupe set for `(boot, req)` pairs. When the chrome
/// page reloads it picks a fresh boot id, which resets the set.
///
/// AUDIT V15-1: a page that is *destroyed* (the watchdog's chrome
/// recreation) is a different case from a page that reloads.
/// Commands the dead page already queued can be delivered after
/// its replacement has booted; without retirement they would be
/// adopted as a "new boot" (clearing the live generation's
/// dedupe state), and a stale `ready` would mark the new
/// generation healthy even if it never handshakes.
/// [`Dedupe::retire_current_boot`] moves the current boot id
/// into a bounded retired set; envelopes carrying a retired boot
/// id are dropped without touching live state.
#[derive(Debug, Default)]
pub struct Dedupe {
    boot: String,
    seen: HashSet<u64>,
    order: VecDeque<u64>,
    cap: usize,
    retired: HashSet<String>,
    retired_order: VecDeque<String>,
}

/// How many destroyed chrome generations' boot ids are
/// remembered. Recreation is a once-per-session event by design
/// (the watchdog recreates once, then escalates), so this bound
/// is generous.
const MAX_RETIRED_BOOTS: usize = 16;

impl Dedupe {
    pub fn new(cap: usize) -> Self {
        Dedupe {
            boot: String::new(),
            seen: HashSet::new(),
            order: VecDeque::new(),
            cap: cap.max(16),
            retired: HashSet::new(),
            retired_order: VecDeque::new(),
        }
    }

    /// Retire the current boot id: its page is gone, and any of
    /// its still-queued commands must be ignored from now on.
    /// Called by the app layer when the watchdog destroys a
    /// chrome WebView.
    pub fn retire_current_boot(&mut self) {
        if self.boot.is_empty() {
            return;
        }
        let boot = std::mem::take(&mut self.boot);
        self.seen.clear();
        self.order.clear();
        if self.retired.insert(boot.clone()) {
            self.retired_order.push_back(boot);
        }
        while self.retired_order.len() > MAX_RETIRED_BOOTS {
            if let Some(old) = self.retired_order.pop_front() {
                self.retired.remove(&old);
            }
        }
    }

    /// `true` the first time a `(boot, req)` pair is seen,
    /// `false` for duplicates — and always `false` for envelopes
    /// from a retired (destroyed) page generation.
    ///
    /// AUDIT V28 A1-L1: a new boot id is adopted only when there
    /// is no live boot, or when the envelope is the new
    /// generation's `ready` handshake. The v27 rule adopted ANY
    /// unseen boot, so a stale command queued by a chrome page
    /// that reloaded WITHOUT the watchdog (renderer recovery,
    /// DevTools reload) flipped the live boot and cleared the
    /// live generation's seen-set — a stale command executed, and
    /// a re-delivered live command could execute twice. A genuine
    /// new generation always introduces itself with `ready`
    /// (the chrome script sends it on load), so gating adoption
    /// on it costs a real generation nothing; a non-watchdog
    /// reload's pre-ready stragglers are dropped, and its `ready`
    /// starts the new generation cleanly.
    pub fn check(&mut self, env: &Envelope) -> bool {
        if self.retired.contains(&env.boot) {
            return false;
        }
        if env.boot != self.boot {
            if self.boot.is_empty() || matches!(env.cmd, Command::Ready) {
                self.boot = env.boot.clone();
                self.seen.clear();
                self.order.clear();
            } else {
                return false;
            }
        }
        if !self.seen.insert(env.req) {
            return false;
        }
        self.order.push_back(env.req);
        while self.order.len() > self.cap {
            if let Some(old) = self.order.pop_front() {
                self.seen.remove(&old);
            }
        }
        true
    }
}

// ---------------------------------------------------------------------------
// Navigation observer notes (content webviews)
// ---------------------------------------------------------------------------

/// How a same-document navigation happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavHow {
    Init,
    Push,
    Replace,
    Pop,
}

/// A navigation note posted by the injected observer script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavNote {
    pub how: NavHow,
    pub href: String,
    pub title: String,
}

/// Parse a raw IPC message body from a content webview. `None`
/// for anything that is not a well-formed navigation note (pages
/// can post arbitrary messages to `window.ipc`; those are
/// ignored).
pub fn parse_nav_note(body: &str) -> Option<NavNote> {
    if body.len() > MAX_NAV_BODY_BYTES {
        return None;
    }
    #[derive(Deserialize)]
    struct Raw {
        kind: String,
        how: String,
        href: String,
        #[serde(default)]
        title: String,
    }
    let raw: Raw = serde_json::from_str(body).ok()?;
    if raw.kind != "nav" {
        return None;
    }
    let how = match raw.how.as_str() {
        "init" => NavHow::Init,
        "push" => NavHow::Push,
        "replace" => NavHow::Replace,
        "pop" => NavHow::Pop,
        _ => return None,
    };
    // A navigation note is an untrusted *hint* about an HTTP(S)
    // document, never a command: anything that is not a
    // well-formed HTTP(S) URL of bounded length is dropped here,
    // and the app layer additionally requires the note to stay in
    // the tab's current origin before it may move the history
    // machine.
    if raw.href.is_empty()
        || raw.href.len() > MAX_HREF_LEN
        || crate::core::addr::web_origin(&raw.href).is_none()
    {
        return None;
    }
    Some(NavNote {
        how,
        href: raw.href,
        title: raw.title.chars().take(MAX_TITLE_CHARS).collect(),
    })
}

/// Cheap pre-filter for the content IPC handler: does this
/// body claim to be a Shield prune report? Full validation is
/// [`parse_shield_prune`]'s job.
///
/// AUDIT 0.30 (A1-2): the needle is the kind ASSIGNMENT
/// (`"kind":"shield_prune"`), not the bare quoted word — a
/// nav note whose page title happens to contain the quoted
/// word "shield_prune" must not be routed into this branch
/// (the handler returns unconditionally per branch, so a
/// misroute would swallow the note). The closing quote in
/// the needle also keeps it disjoint from `shield_pruned`.
/// All runtime bodies are `JSON.stringify` output with
/// `kind` first and no spaces, so the needle costs the
/// honest path nothing.
pub fn looks_like_shield_prune(body: &str) -> bool {
    body.len() <= MAX_NAV_BODY_BYTES && body.contains("\"kind\":\"shield_prune\"")
}

/// Cheap pre-filter for lite-chosen bodies (AUDIT V29): the
/// kind assignment present, same bound (AUDIT 0.30, A1-2 —
/// see `looks_like_shield_prune`).
pub fn looks_like_lite_chosen(body: &str) -> bool {
    body.len() <= MAX_NAV_BODY_BYTES && body.contains("\"kind\":\"lite_chosen\"")
}

/// Validate a lite-chosen report (AUDIT V29):
/// `{"kind":"lite_chosen"}` — the report carries NO payload
/// beyond the fact itself (no URL, no selector, no counts), so
/// there is nothing for a hostile page to smuggle through it.
/// Like every content report it is a hint, never a command:
/// the app gives it exactly one effect — the sending tab's
/// chosen flag is set in the Lite runtime — and only when the
/// sender is a live public content tab with Lite on. Returns
/// false for anything that is not exactly this kind.
pub fn parse_lite_chosen(body: &str) -> bool {
    if body.len() > MAX_NAV_BODY_BYTES {
        return false;
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Raw {
        kind: String,
    }
    match serde_json::from_str::<Raw>(body) {
        Ok(raw) => raw.kind == "lite_chosen",
        Err(_) => false,
    }
}

/// One measured prune-payload report (0.30): the scriptlet
/// runtime's exact count of ad subtrees pruned in-page and
/// their serialized byte total. Unlike [`PruneItem`] (a
/// diagnostic hint worth only log lines), this report feeds
/// the savings book — so it is validated to the same standard
/// as the book's own gates: exact shape, unknown fields
/// denied, hard bounds, and the app re-checks the sender
/// (Shield on, live public content tab) plus a per-tab rate
/// floor before a single number moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrunedReport {
    pub bytes: u64,
    pub events: u64,
}

/// Cheap pre-filter for measured prune reports (0.30): the
/// kind assignment present, same bound as every content
/// body (AUDIT 0.30, A1-2 — see `looks_like_shield_prune`
/// for why the needle is the assignment, not the word).
pub fn looks_like_shield_pruned(body: &str) -> bool {
    body.len() <= MAX_NAV_BODY_BYTES && body.contains("\"kind\":\"shield_pruned\"")
}

/// Validate a `shield_pruned` report:
/// `{"kind":"shield_pruned","bytes":N,"events":M}` with
/// `bytes` ≤ [`crate::core::stats::MAX_PRUNE_REPORT_BYTES`]
/// and `events` ≤ [`crate::core::stats::MAX_PRUNE_REPORT_EVENTS`].
/// Deny-unknown-fields: a report that carries anything else
/// (a URL, a path, a count for another purpose) is not this
/// report. The values are payload-free aggregates — there is
/// nothing in them a page could aim at another page.
pub fn parse_shield_pruned(body: &str) -> Option<PrunedReport> {
    if body.len() > MAX_NAV_BODY_BYTES {
        return None;
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Raw {
        kind: String,
        bytes: u64,
        events: u64,
    }
    let raw: Raw = serde_json::from_str(body).ok()?;
    if raw.kind != "shield_pruned"
        || raw.bytes > crate::core::stats::MAX_PRUNE_REPORT_BYTES
        || raw.events > crate::core::stats::MAX_PRUNE_REPORT_EVENTS
    {
        return None;
    }
    Some(PrunedReport {
        bytes: raw.bytes,
        events: raw.events,
    })
}

/// One per-path prune count reported by the scriptlet payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PruneItem {
    pub path: String,
    pub count: u64,
}

/// Parse a `shield_prune` IPC body (v23): the document-start
/// scriptlet runtime reports how many ad properties it deleted,
/// per property path, so the host can log proof the layer is
/// working. This is a *diagnostic hint only*: pages can post
/// arbitrary IPC bodies, so the parsed items feed nothing but
/// capped sample log lines — never the blocked counters, never
/// any decision. Every field is bounded here: at most 16
/// items, paths at most 256 bytes with no control characters,
/// counts clamped to the payload's own ceiling.
pub fn parse_shield_prune(body: &str) -> Option<Vec<PruneItem>> {
    if body.len() > MAX_NAV_BODY_BYTES {
        return None;
    }
    #[derive(Deserialize)]
    struct RawItem {
        p: String,
        n: u64,
    }
    #[derive(Deserialize)]
    struct Raw {
        kind: String,
        items: Vec<RawItem>,
    }
    let raw: Raw = serde_json::from_str(body).ok()?;
    if raw.kind != "shield_prune" || raw.items.len() > 16 {
        return None;
    }
    let mut out = Vec::with_capacity(raw.items.len());
    for item in raw.items {
        if item.n == 0
            || item.p.is_empty()
            || item.p.len() > 256
            || item.p.chars().any(|c| c.is_control())
        {
            continue;
        }
        out.push(PruneItem {
            path: item.p,
            count: item.n.min(100_000),
        });
    }
    Some(out)
}

/// The document-created script injected into every content
/// webview. It hooks `history.pushState`/`replaceState` and
/// listens for `popstate`/`hashchange` so same-document (SPA)
/// navigations reach Rust the moment they happen — wry page-load
/// events never fire for them. Notes are posted through wry's IPC
/// channel; the receiving handler knows which tab the webview
/// belongs to, so the script itself needs no tab identity.
///
/// Defensive by construction: everything is wrapped in try/catch,
/// it bails out in subframes and on non-http(s) pages, it posts
/// nothing if `window.ipc` is unavailable, and if anything throws
/// the page is left completely untouched.
pub const NAV_OBSERVER_SCRIPT: &str = r#"
(function () {
  try {
    if (window.top !== window.self) return;
    if (location.protocol !== "http:" && location.protocol !== "https:") return;
    if (!window.ipc || typeof window.ipc.postMessage !== "function") return;
    var last = location.href;
    function post(how) {
      try {
        window.ipc.postMessage(JSON.stringify({
          kind: "nav", how: how,
          href: location.href,
          title: document.title || ""
        }));
      } catch (e) { /* IPC unavailable: stay silent */ }
    }
    function check(how) {
      var h = location.href;
      if (h !== last) { last = h; post(how); }
    }
    var origPush = history.pushState;
    var origReplace = history.replaceState;
    if (typeof origPush === "function") {
      history.pushState = function () {
        var r = origPush.apply(this, arguments);
        check("push");
        return r;
      };
    }
    if (typeof origReplace === "function") {
      history.replaceState = function () {
        var r = origReplace.apply(this, arguments);
        check("replace");
        return r;
      };
    }
    window.addEventListener("popstate", function () { check("pop"); });
    window.addEventListener("hashchange", function () { check("pop"); });
    post("init");
  } catch (e) { /* never break the page */ }
})();
"#;
