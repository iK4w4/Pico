//! Pico's own context menu — the pure item model and the
//! anchor resolution for the shell-drawn replacement of
//! WebView2's stock menu.
//!
//! The stock menu is unusable for Pico: it is unthemed, it
//! carries entries Pico has no business offering ("Save as",
//! "Print", "More tools", "Send tab to your devices"), and
//! none of its verbs route through the shell. So the content
//! WebView's stock menu is suppressed in the app layer and
//! this module decides, purely, what Pico's menu contains:
//! a fixed table of fourteen ids ([`CTX_IDS`]), fixed labels
//! and shortcuts, and groups that appear only when the right-
//! click target carries what they act on. There are simply
//! no other ids in the table, so the model is incapable of
//! emitting a stock item — the whitelist test below pins
//! that against a maximal target.
//!
//! Everything here is pure and host-testable; the app layer
//! only reports the WebView2 target facts (kind, URLs,
//! selection/editable/clipboard state, history gates) and
//! executes the id the user picks, which crosses back over
//! IPC as a `ctx_pick` command validated against
//! [`CTX_IDS`] in `core::ipc`.

/// What the right-click landed on, as reported by WebView2's
/// `ContextMenuRequested` event (`ContextMenuTargetKind`).
/// The wire values are WebView2's enum ordinals; anything
/// unrecognised is a plain page — the least capable kind —
/// never a guess at a richer one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CtxKind {
    Page,
    Image,
    SelectedText,
    Audio,
    Video,
}

impl CtxKind {
    /// Map a WebView2 `ContextMenuTargetKind` ordinal:
    /// 0 = Page, 1 = Image, 2 = SelectedText, 3 = Audio,
    /// 4 = Video; anything else (a future kind, a bad cast)
    /// degrades to Page.
    pub fn from_webview2(v: i32) -> CtxKind {
        match v {
            0 => CtxKind::Page,
            1 => CtxKind::Image,
            2 => CtxKind::SelectedText,
            3 => CtxKind::Audio,
            4 => CtxKind::Video,
            _ => CtxKind::Page,
        }
    }
}

/// The facts about one right-click target that the menu
/// model is allowed to see. URLs are the raw strings
/// WebView2 reports; emptiness is judged here (trimmed),
/// never by the caller, so a whitespace-only "link" cannot
/// conjure a link group in one place and not another.
#[derive(Debug, Clone, PartialEq)]
pub struct CtxTarget {
    pub kind: CtxKind,
    pub page_url: String,
    pub link_url: Option<String>,
    pub source_url: Option<String>,
    pub has_selection: bool,
    pub is_editable: bool,
    pub has_clipboard_text: bool,
}

/// One row of Pico's context menu. A separator is a row too
/// (`sep == true`), with empty id/label and no shortcut —
/// the renderer draws it as a rule, and it can never be
/// picked (its id is not in [`CTX_IDS`], so a forged
/// `ctx_pick` naming it is rejected at the IPC gate).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CtxItem {
    pub id: &'static str,
    pub label: &'static str,
    pub shortcut: Option<&'static str>,
    pub enabled: bool,
    pub sep: bool,
}

impl CtxItem {
    /// The separator row: the one constructor for a `sep`
    /// item, so every separator in every menu has exactly
    /// the same shape.
    pub fn sep() -> CtxItem {
        CtxItem {
            id: "",
            label: "",
            shortcut: None,
            enabled: false,
            sep: true,
        }
    }
}

/// The fourteen ids Pico's context menu can ever emit, in
/// menu order. This is also the parse whitelist for the
/// `ctx_pick` IPC command (`core::ipc` validates against
/// it): a pick naming anything else — a stock-menu verb, a
/// separator, a typo — is rejected before the app sees it.
pub const CTX_IDS: &[&str] = &[
    "back",
    "forward",
    "reload",
    "open_link",
    "copy_link",
    "download_link",
    "open_image",
    "copy_image",
    "download_image",
    "copy",
    "cut",
    "paste",
    "select_all",
    "inspect",
];

/// The fixed label and shortcut for one id in [`CTX_IDS`].
/// Labels and shortcuts live in exactly one place — the
/// renderer in the chrome page prints what it is given and
/// never re-derives either, so the two cannot drift.
fn label_shortcut(id: &str) -> (&'static str, Option<&'static str>) {
    match id {
        "back" => ("Back", Some("Alt+Left")),
        "forward" => ("Forward", Some("Alt+Right")),
        "reload" => ("Reload", Some("Ctrl+R")),
        "open_link" => ("Open Link in New Tab", None),
        "copy_link" => ("Copy Link", None),
        "download_link" => ("Download Link", None),
        "open_image" => ("Open Image in New Tab", None),
        "copy_image" => ("Copy Image Address", None),
        "download_image" => ("Download Image", None),
        "copy" => ("Copy", None),
        "cut" => ("Cut", None),
        "paste" => ("Paste", None),
        "select_all" => ("Select All", None),
        "inspect" => ("Inspect", None),
        _ => ("", None),
    }
}

/// One enabled-state-stamped row for `id`. Callers pass only
/// ids from [`CTX_IDS`]; the debug assertion keeps a typo in
/// this module loud in tests instead of a blank row in the
/// shipped menu.
fn item(id: &'static str, enabled: bool) -> CtxItem {
    debug_assert!(CTX_IDS.contains(&id), "ctx item id {id} not in CTX_IDS");
    let (label, shortcut) = label_shortcut(id);
    CtxItem {
        id,
        label,
        shortcut,
        enabled,
        sep: false,
    }
}

/// Does an optional URL carry anything once trimmed? The
/// group gates below all use this one test.
fn present(url: &Option<String>) -> bool {
    url.as_deref().is_some_and(|u| !u.trim().is_empty())
}

/// The pure item model for one right-click: the ordered rows
/// (separators included) Pico's menu shows for `target`,
/// with the history gates `can_back` / `can_forward` applied
/// to the nav group.
///
/// Group order is fixed — nav, link, image, edit, inspect —
/// and a group appears only when its precondition holds:
/// - nav: always; back/forward enabled by the gates,
///   reload always enabled.
/// - link: `link_url` present (Some, non-empty trimmed).
/// - image: kind is Image and `source_url` present.
/// - edit: in an editable field, cut/copy gated on a
///   selection, paste gated on clipboard text, select-all
///   always enabled; outside one, a lone Copy when there
///   is a selection, and nothing when there is not.
/// - inspect: always, always last, always enabled.
///
/// Audio and Video targets get nav + inspect only this
/// round: no source group is modelled for them yet, and a
/// media element's incidental link/selection state must not
/// leak other groups in. Consecutive present groups are
/// separated by exactly one separator row, and a separator
/// always precedes inspect.
pub fn menu_items(target: &CtxTarget, can_back: bool, can_forward: bool) -> Vec<CtxItem> {
    let mut groups: Vec<Vec<CtxItem>> = Vec::new();
    groups.push(vec![
        item("back", can_back),
        item("forward", can_forward),
        item("reload", true),
    ]);
    if !matches!(target.kind, CtxKind::Audio | CtxKind::Video) {
        if present(&target.link_url) {
            groups.push(vec![
                item("open_link", true),
                item("copy_link", true),
                item("download_link", true),
            ]);
        }
        if target.kind == CtxKind::Image && present(&target.source_url) {
            groups.push(vec![
                item("open_image", true),
                item("copy_image", true),
                item("download_image", true),
            ]);
        }
        if target.is_editable {
            groups.push(vec![
                item("cut", target.has_selection),
                item("copy", target.has_selection),
                item("paste", target.has_clipboard_text),
                item("select_all", true),
            ]);
        } else if target.has_selection {
            groups.push(vec![item("copy", true)]);
        }
    }
    let mut out = Vec::new();
    for (i, group) in groups.into_iter().enumerate() {
        if i > 0 {
            out.push(CtxItem::sep());
        }
        out.extend(group);
    }
    out.push(CtxItem::sep());
    out.push(item("inspect", true));
    out
}

/// The layout module's sane-scale rule, restated for the
/// anchor math: a non-finite or non-positive scale is 1.0.
fn sane_scale(scale: f64) -> f64 {
    if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    }
}

/// How close (in DIP) a candidate anchor must be to the real
/// cursor to be believed as the coordinate space WebView2
/// actually reported in. Generous enough for rounding and
/// the menu's own shadow margin, far smaller than the
/// distance between the candidate spaces at any real scale.
const ANCHOR_SNAP_DIP: f64 = 48.0;

/// Resolve a WebView2 `ContextMenuRequested` location into a
/// window-DIP point the chrome menu can be anchored at.
///
/// The event's Location coordinate space is not documented
/// tightly enough to trust blindly — it may be relative to
/// the WebView in DIP, relative to the WebView in physical
/// pixels, or relative to the window in DIP — so the three
/// candidate readings are computed and resolved against the
/// real cursor position (window-DIP) when one is available:
/// - A: `(loc.x, content_top + loc.y)` — view-relative DIP;
/// - B: `(loc.x/scale, content_top + loc.y/scale)` —
///   view-relative physical;
/// - C: `(loc.x, loc.y)` — window-relative DIP.
///
/// If `cursor` is Some and the closest candidate is within
/// [`ANCHOR_SNAP_DIP`], that candidate wins; otherwise —
/// no cursor, or none of the readings is anywhere near it —
/// A is returned: view-relative DIP is the documented-most
/// reading and the safe default. Ties go to the earliest
/// candidate (A before B before C), which at scale 1.0 —
/// where A and B coincide — is A by construction. The menu
/// position itself is then clamped/flipped by
/// `core::layout::ctx_menu_pos`.
pub fn resolve_anchor(
    location: (f64, f64),
    scale: f64,
    content_top_dip: f64,
    cursor: Option<(f64, f64)>,
) -> (f64, f64) {
    let scale = sane_scale(scale);
    let a = (location.0, content_top_dip + location.1);
    let b = (location.0 / scale, content_top_dip + location.1 / scale);
    let c = location;
    if let Some(cur) = cursor {
        let mut best = a;
        let mut best_d = f64::INFINITY;
        for cand in [a, b, c] {
            let d = (cand.0 - cur.0).hypot(cand.1 - cur.1);
            if d < best_d {
                best_d = d;
                best = cand;
            }
        }
        if best_d <= ANCHOR_SNAP_DIP {
            return best;
        }
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> CtxTarget {
        CtxTarget {
            kind: CtxKind::Page,
            page_url: "https://example.com/".to_string(),
            link_url: None,
            source_url: None,
            has_selection: false,
            is_editable: false,
            has_clipboard_text: false,
        }
    }

    fn ids(items: &[CtxItem]) -> Vec<&str> {
        items.iter().map(|i| i.id).collect()
    }

    fn find<'a>(items: &'a [CtxItem], id: &str) -> &'a CtxItem {
        items.iter().find(|i| i.id == id).expect("id present")
    }

    #[test]
    fn from_webview2_maps_known_kinds_and_defaults_to_page() {
        assert_eq!(CtxKind::from_webview2(0), CtxKind::Page);
        assert_eq!(CtxKind::from_webview2(1), CtxKind::Image);
        assert_eq!(CtxKind::from_webview2(2), CtxKind::SelectedText);
        assert_eq!(CtxKind::from_webview2(3), CtxKind::Audio);
        assert_eq!(CtxKind::from_webview2(4), CtxKind::Video);
        for other in [-1, 5, 99, i32::MAX] {
            assert_eq!(
                CtxKind::from_webview2(other),
                CtxKind::Page,
                "unknown ordinal {other} degrades to Page"
            );
        }
    }

    #[test]
    fn sep_item_has_the_fixed_separator_shape() {
        let s = CtxItem::sep();
        assert_eq!(s.id, "");
        assert_eq!(s.label, "");
        assert_eq!(s.shortcut, None);
        assert!(!s.enabled);
        assert!(s.sep);
        assert!(!CTX_IDS.contains(&s.id), "a separator is never pickable");
    }

    #[test]
    fn plain_page_is_nav_sep_inspect_with_history_gates() {
        // No history behind, history ahead: back disabled,
        // forward enabled, reload and inspect always enabled.
        let items = menu_items(&base(), false, true);
        assert_eq!(ids(&items), ["back", "forward", "reload", "", "inspect"]);
        assert!(!find(&items, "back").enabled);
        assert!(find(&items, "forward").enabled);
        assert!(find(&items, "reload").enabled);
        assert!(find(&items, "inspect").enabled);
        // The labels and shortcuts are the frozen contract.
        assert_eq!(find(&items, "back").label, "Back");
        assert_eq!(find(&items, "back").shortcut, Some("Alt+Left"));
        assert_eq!(find(&items, "forward").label, "Forward");
        assert_eq!(find(&items, "forward").shortcut, Some("Alt+Right"));
        assert_eq!(find(&items, "reload").label, "Reload");
        assert_eq!(find(&items, "reload").shortcut, Some("Ctrl+R"));
        assert_eq!(find(&items, "inspect").label, "Inspect");
        assert_eq!(find(&items, "inspect").shortcut, None);
        // The gates follow the arguments, not the target.
        let items = menu_items(&base(), true, false);
        assert!(find(&items, "back").enabled);
        assert!(!find(&items, "forward").enabled);
    }

    #[test]
    fn page_with_link_adds_the_link_group() {
        let mut t = base();
        t.link_url = Some("https://example.com/docs".to_string());
        let items = menu_items(&t, true, true);
        assert_eq!(
            ids(&items),
            [
                "back",
                "forward",
                "reload",
                "",
                "open_link",
                "copy_link",
                "download_link",
                "",
                "inspect"
            ]
        );
        for (id, label) in [
            ("open_link", "Open Link in New Tab"),
            ("copy_link", "Copy Link"),
            ("download_link", "Download Link"),
        ] {
            let it = find(&items, id);
            assert_eq!(it.label, label);
            assert_eq!(it.shortcut, None);
            assert!(it.enabled);
        }
    }

    #[test]
    fn image_adds_the_image_group() {
        let mut t = base();
        t.kind = CtxKind::Image;
        t.source_url = Some("https://example.com/cat.png".to_string());
        let items = menu_items(&t, true, false);
        assert_eq!(
            ids(&items),
            [
                "back",
                "forward",
                "reload",
                "",
                "open_image",
                "copy_image",
                "download_image",
                "",
                "inspect"
            ]
        );
        for (id, label) in [
            ("open_image", "Open Image in New Tab"),
            ("copy_image", "Copy Image Address"),
            ("download_image", "Download Image"),
        ] {
            let it = find(&items, id);
            assert_eq!(it.label, label);
            assert!(it.enabled);
        }
        // An Image kind WITHOUT a source URL gets no group.
        let mut t = base();
        t.kind = CtxKind::Image;
        let items = menu_items(&t, true, false);
        assert_eq!(ids(&items), ["back", "forward", "reload", "", "inspect"]);
        // A source URL on a non-Image kind gets no group either.
        let mut t = base();
        t.source_url = Some("https://example.com/cat.png".to_string());
        let items = menu_items(&t, true, false);
        assert_eq!(ids(&items), ["back", "forward", "reload", "", "inspect"]);
    }

    #[test]
    fn image_with_link_orders_link_group_before_image_group() {
        let mut t = base();
        t.kind = CtxKind::Image;
        t.link_url = Some("https://example.com/post".to_string());
        t.source_url = Some("https://example.com/cat.png".to_string());
        let items = menu_items(&t, true, true);
        assert_eq!(
            ids(&items),
            [
                "back",
                "forward",
                "reload",
                "",
                "open_link",
                "copy_link",
                "download_link",
                "",
                "open_image",
                "copy_image",
                "download_image",
                "",
                "inspect"
            ]
        );
    }

    #[test]
    fn selection_on_page_adds_a_copy_only_edit_group() {
        let mut t = base();
        t.kind = CtxKind::SelectedText;
        t.has_selection = true;
        let items = menu_items(&t, true, true);
        assert_eq!(
            ids(&items),
            ["back", "forward", "reload", "", "copy", "", "inspect"]
        );
        let copy = find(&items, "copy");
        assert_eq!(copy.label, "Copy");
        assert!(copy.enabled);
        // No selection outside an editable field: no edit
        // group at all (the plain-page test pins the shape).
        let items = menu_items(&base(), true, true);
        assert!(!ids(&items).contains(&"copy"));
    }

    #[test]
    fn editable_with_selection_enables_cut_copy_and_gates_paste_on_clipboard() {
        let mut t = base();
        t.is_editable = true;
        t.has_selection = true;
        t.has_clipboard_text = true;
        let items = menu_items(&t, true, true);
        assert_eq!(
            ids(&items),
            [
                "back",
                "forward",
                "reload",
                "",
                "cut",
                "copy",
                "paste",
                "select_all",
                "",
                "inspect"
            ]
        );
        for id in ["cut", "copy", "paste", "select_all"] {
            assert!(find(&items, id).enabled, "{id} enabled");
        }
        assert_eq!(find(&items, "cut").label, "Cut");
        assert_eq!(find(&items, "paste").label, "Paste");
        assert_eq!(find(&items, "select_all").label, "Select All");
    }

    #[test]
    fn editable_without_selection_disables_cut_and_copy_only() {
        let mut t = base();
        t.is_editable = true;
        t.has_selection = false;
        t.has_clipboard_text = true;
        let items = menu_items(&t, true, true);
        assert!(!find(&items, "cut").enabled);
        assert!(!find(&items, "copy").enabled);
        assert!(find(&items, "paste").enabled);
        assert!(find(&items, "select_all").enabled);
    }

    #[test]
    fn editable_without_clipboard_text_disables_paste() {
        let mut t = base();
        t.is_editable = true;
        t.has_selection = true;
        t.has_clipboard_text = false;
        let items = menu_items(&t, true, true);
        assert!(!find(&items, "paste").enabled, "nothing to paste");
        assert!(find(&items, "cut").enabled);
        assert!(find(&items, "copy").enabled);
        assert!(find(&items, "select_all").enabled);
    }

    #[test]
    fn audio_and_video_are_nav_and_inspect_only() {
        for kind in [CtxKind::Audio, CtxKind::Video] {
            // Even a maximally-loaded media target — link,
            // source, selection, editable, clipboard — leaks
            // no other group in this round.
            let mut t = base();
            t.kind = kind;
            t.link_url = Some("https://example.com/x".to_string());
            t.source_url = Some("https://example.com/m.mp4".to_string());
            t.has_selection = true;
            t.is_editable = true;
            t.has_clipboard_text = true;
            let items = menu_items(&t, true, true);
            assert_eq!(
                ids(&items),
                ["back", "forward", "reload", "", "inspect"],
                "{kind:?} gets nav + inspect only"
            );
        }
    }

    #[test]
    fn empty_or_blank_link_is_treated_as_absent() {
        for link in ["", "   ", "\t"] {
            let mut t = base();
            t.link_url = Some(link.to_string());
            let items = menu_items(&t, true, true);
            assert_eq!(
                ids(&items),
                ["back", "forward", "reload", "", "inspect"],
                "link {link:?} is absent"
            );
        }
        // The same rule governs the image source URL.
        let mut t = base();
        t.kind = CtxKind::Image;
        t.source_url = Some("  ".to_string());
        let items = menu_items(&t, true, true);
        assert_eq!(ids(&items), ["back", "forward", "reload", "", "inspect"]);
    }

    #[test]
    fn maximal_menu_stays_inside_the_whitelist() {
        // Every group present at once: the full fourteen-id
        // menu. Every emitted non-separator id is in CTX_IDS
        // (there is no other id the model can produce), and
        // no label carries a stock-menu phrase.
        let mut t = base();
        t.kind = CtxKind::Image;
        t.link_url = Some("https://example.com/post".to_string());
        t.source_url = Some("https://example.com/cat.png".to_string());
        t.has_selection = true;
        t.is_editable = true;
        t.has_clipboard_text = true;
        let items = menu_items(&t, true, true);
        assert_eq!(
            ids(&items),
            [
                "back",
                "forward",
                "reload",
                "",
                "open_link",
                "copy_link",
                "download_link",
                "",
                "open_image",
                "copy_image",
                "download_image",
                "",
                "cut",
                "copy",
                "paste",
                "select_all",
                "",
                "inspect"
            ]
        );
        for it in &items {
            if it.sep {
                assert_eq!(*it, CtxItem::sep(), "separators are uniform");
                continue;
            }
            assert!(CTX_IDS.contains(&it.id), "id {} is whitelisted", it.id);
            for banned in ["Save as", "Print", "More tools", "Send tab"] {
                assert!(
                    !it.label.contains(banned),
                    "label {:?} must not carry stock phrase {banned:?}",
                    it.label
                );
            }
        }
        // Exactly one separator between consecutive groups,
        // never two in a row, never first or last.
        assert!(!items.first().expect("non-empty").sep);
        assert!(!items.last().expect("non-empty").sep);
        for pair in items.windows(2) {
            assert!(!(pair[0].sep && pair[1].sep), "no double separators");
        }
    }

    // -------------------------------------------------------
    // resolve_anchor: location (200, 40) under a 74-DIP
    // content top. A = (200, 114); at scale 2, B = (100, 94);
    // C = (200, 40).
    // -------------------------------------------------------

    #[test]
    fn resolve_anchor_at_scale_1_the_view_relative_reading_wins() {
        // A and B coincide at scale 1.0; the cursor sits on
        // them, far from C.
        let got = resolve_anchor((200.0, 40.0), 1.0, 74.0, Some((202.0, 112.0)));
        assert_eq!(got, (200.0, 114.0));
    }

    #[test]
    fn resolve_anchor_at_scale_2_picks_the_physical_candidate_near_the_cursor() {
        let got = resolve_anchor((200.0, 40.0), 2.0, 74.0, Some((98.0, 96.0)));
        assert_eq!(got, (100.0, 94.0), "B: view-relative physical");
    }

    #[test]
    fn resolve_anchor_picks_the_window_relative_candidate_near_the_cursor() {
        let got = resolve_anchor((200.0, 40.0), 2.0, 74.0, Some((203.0, 38.0)));
        assert_eq!(got, (200.0, 40.0), "C: window-relative DIP");
    }

    #[test]
    fn resolve_anchor_far_cursor_falls_back_to_a() {
        // The cursor is >48 DIP from every candidate: none of
        // the readings is believable, so A stands.
        let got = resolve_anchor((200.0, 40.0), 2.0, 74.0, Some((900.0, 900.0)));
        assert_eq!(got, (200.0, 114.0));
    }

    #[test]
    fn resolve_anchor_without_a_cursor_is_a() {
        assert_eq!(
            resolve_anchor((200.0, 40.0), 2.0, 74.0, None),
            (200.0, 114.0)
        );
        assert_eq!(
            resolve_anchor((200.0, 40.0), 1.0, 74.0, None),
            (200.0, 114.0)
        );
    }

    #[test]
    fn resolve_anchor_degenerate_scale_is_1x() {
        for scale in [0.0, -2.0, f64::NAN, f64::INFINITY] {
            assert_eq!(
                resolve_anchor((200.0, 40.0), scale, 74.0, None),
                (200.0, 114.0),
                "scale {scale} falls back to 1.0"
            );
        }
    }
}
