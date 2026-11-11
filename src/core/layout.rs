//! Window tiling math — the single logical→physical conversion
//! site, and the geometry contract for the whole shell.
//!
//! v14 (AUDIT V14-4): v13 computed chrome/content bounds in
//! *logical* units in two places, each re-deriving logical size
//! from the physical inner size, and handed wry logical rects
//! whose independent rounding could leave the two webviews a
//! device pixel apart or overlapped — a hairline of unpainted
//! (black) window at the seam on fractional DPI scales. Now:
//! the GUI layer asks this module once, in *device pixels*, and
//! hands wry physical-pixel rects.
//!
//! v16 (header redesign): the chrome is 86 DIP and the two
//! webviews overlap *by design*: the chrome rect is
//! `[0, CHROME_H)`, the content rect starts at `SOLID_H` (74),
//! so the content's top `FEATHER_H` (12) DIP sit beneath the
//! chrome WebView, whose transparent feather strip lets the
//! header visually melt into the page (see assets/chrome.html).
//! The overlap is exact by construction at every scale: both
//! edges derive from the two constants below, rounded once,
//! here — never anywhere else. The chrome page's CSS rows MUST
//! keep summing to these constants (40 top row + 46 address row
//! = CHROME_H; solid paint stops at SOLID_H; the feather strip
//! is FEATHER_H) — the asset tests pin the CSS side of the
//! contract.

/// Total chrome height in logical pixels: the merged top row
/// (40) plus the address row (46). The chrome WebView occupies
/// exactly this much of the window's top.
pub const CHROME_H: f64 = 86.0;

/// Height of the chrome's solid (opaque) region in logical
/// pixels. The content rect starts here, so the page shows
/// through the chrome's transparent feather below this line.
pub const SOLID_H: f64 = 74.0;

/// The feather overlap in logical pixels: how much of the
/// content's top sits beneath the chrome WebView. Derived,
/// never set independently — CHROME_H − SOLID_H by
/// construction.
pub const FEATHER_H: f64 = CHROME_H - SOLID_H;

/// An axis-aligned rectangle in device (physical) pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysRect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

fn sane_scale(scale: f64) -> f64 {
    if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    }
}

/// Chrome height in device pixels for a scale factor: the
/// logical height scaled and rounded once, here.
pub fn chrome_height_phys(scale: f64) -> u32 {
    (CHROME_H * sane_scale(scale)).round().max(1.0) as u32
}

/// Content top edge in device pixels for a scale factor: where
/// the solid chrome ends and the feather overlap begins.
pub fn solid_height_phys(scale: f64) -> u32 {
    (SOLID_H * sane_scale(scale)).round().max(1.0) as u32
}

/// Feather overlap in device pixels for a scale factor.
pub fn feather_height_phys(scale: f64) -> u32 {
    chrome_height_phys(scale).saturating_sub(solid_height_phys(scale))
}

/// Extra chrome height (logical px) while the Teleport panel is
/// open. The panel is an OVERLAY: the chrome WebView's rect
/// grows downward over the content — the content rect itself is
/// never moved, the 86-DIP geometry and the feather are exactly
/// as they are with the panel closed, and closing the panel
/// restores the plain chrome rect.
///
/// v26 (AUDIT D2/D3): the panel is now a big CENTERED card over
/// a dimmed backdrop (the v25 card was 264 wide in the far right
/// corner with a 184px QR — hard to scan on an ultrawide). The
/// card needs ~478 DIP (head + 264px QR well + URL + note +
/// copy, with margins), so the extension grows to 496. The
/// region of chrome below SOLID_H paints only the backdrop and
/// the card. Two v25 defects are designed out here and pinned
/// in the chrome CSS: everything in the chrome document that
/// must stay at the window's geometry (the feather, the error
/// bar) is anchored by `top`, never by `bottom` — a
/// bottom-anchored element rides to the bottom of this GROWN
/// rect and paints hundreds of pixels too low (that was the
/// user's "black bar"). The cost, unchanged from v25 and
/// documented in AUDIT: while the panel is open, the top band
/// of the page is under the chrome HWND and does not take
/// clicks — the panel is modal by shape, and Esc/×/backdrop/
/// tab-switch/navigation dismiss it.
pub const TELEPORT_PANEL_H: f64 = 496.0;

/// Downloads popover geometry (0.32.2): a compact dropdown
/// anchored under the toolbar's right cluster instead of a
/// centered sheet. These constants are the geometry contract
/// shared with the CSS in assets/chrome.html (#dpanel and its
/// rows) — the CSS MUST use these same numbers (78/360/10/8/
/// 26/30/76/6/386/28); keep both places in sync.
pub const DL_POPOVER_TOP: f64 = 78.0;
pub const DL_POPOVER_PAD: f64 = 10.0;
pub const DL_POPOVER_HEAD_H: f64 = 26.0;
pub const DL_POPOVER_ADD_H: f64 = 30.0;
pub const DL_POPOVER_GAP: f64 = 8.0;
/// One popover row's height (0.35 anatomy): vertical padding
/// 16 + name-line 24 + gap 5 + bar 4 + gap 5 + prog-line 22
/// = 76.
pub const DL_ROW_H: f64 = 76.0;
pub const DL_ROW_ERR_H: f64 = 15.0;
pub const DL_ROW_GAP: f64 = 6.0;
pub const DL_EMPTY_H: f64 = 30.0;
pub const DL_LIST_MAX_H: f64 = 386.0;
pub const DL_POPOVER_MAX_H: f64 = 480.0;
/// The popover footer's height (0.35): the "View all" row
/// (28) that has been in the DOM since 0.33 — through 0.34
/// it was never priced into the card at all.
pub const DL_POPOVER_FOOT_H: f64 = 28.0;

/// The chrome-rect EXTENSION in DIP beyond CHROME_H needed to
/// contain the downloads popover for `rows` tasks, `error_rows`
/// of which carry an error line: the card height is
/// 2*PAD + HEAD_H + GAP + ADD_H + GAP + list_h + GAP + FOOT_H
/// (the footer is priced since 0.35; capped at
/// POPOVER_MAX_H), and the extension covers the card at
/// POPOVER_TOP plus 10 DIP of shadow slack, minus the chrome
/// height already present.
pub fn downloads_popover_h(rows: usize, error_rows: usize) -> f64 {
    let list_h = if rows == 0 {
        DL_EMPTY_H
    } else {
        let h = rows as f64 * DL_ROW_H
            + (rows - 1) as f64 * DL_ROW_GAP
            + error_rows as f64 * DL_ROW_ERR_H;
        h.min(DL_LIST_MAX_H)
    };
    let card_h = (2.0 * DL_POPOVER_PAD
        + DL_POPOVER_HEAD_H
        + DL_POPOVER_GAP
        + DL_POPOVER_ADD_H
        + DL_POPOVER_GAP
        + list_h
        + DL_POPOVER_GAP
        + DL_POPOVER_FOOT_H)
        .min(DL_POPOVER_MAX_H);
    (DL_POPOVER_TOP + card_h + 10.0 - CHROME_H).max(0.0)
}

/// Download toast geometry (0.32.3): the "download
/// started / blocked" pill hangs at the same anchor as the
/// popover (top 78, right 12 — under the toolbar's right
/// cluster). The numbers are the contract shared with the
/// CSS in assets/chrome.html (#dl-toast): top 78, height
/// 34 (8px vertical padding around an 18px line), right 12;
/// keep both places in sync.
pub const DL_TOAST_TOP: f64 = 78.0;
pub const DL_TOAST_H: f64 = 34.0;

/// The chrome-rect EXTENSION in DIP beyond CHROME_H needed
/// to contain the toast: its anchor plus its height plus
/// the same 10 DIP of shadow slack the popover gets,
/// minus the chrome height already present.
pub fn downloads_toast_h() -> f64 {
    (DL_TOAST_TOP + DL_TOAST_H + 10.0 - CHROME_H).max(0.0)
}

/// Context menu geometry: Pico's own right-click menu is a
/// card drawn by the chrome page, anchored at the resolved
/// cursor point (see `core::ctxmenu::resolve_anchor`). These
/// constants are the geometry contract shared with the CSS
/// in assets/chrome.html (the menu card and its rows) — the
/// CSS MUST use these same numbers (236/30/9/6); keep both
/// places in sync.
pub const CTX_MENU_W: f64 = 236.0;
pub const CTX_ROW_H: f64 = 30.0;
pub const CTX_SEP_H: f64 = 9.0;
pub const CTX_MENU_PAD_Y: f64 = 6.0;

/// The context menu card's height in DIP for `rows` item
/// rows and `seps` separator rows: vertical padding top and
/// bottom, plus each row at its contracted height.
pub fn ctx_menu_h(rows: usize, seps: usize) -> f64 {
    2.0 * CTX_MENU_PAD_Y + rows as f64 * CTX_ROW_H + seps as f64 * CTX_SEP_H
}

/// Where the context menu card's top-left corner goes, in
/// window DIP, for a window of `win_w`×`win_h` and a card of
/// height `menu_h` anchored at `anchor` (the card's width
/// is the contracted [`CTX_MENU_W`]). The card opens at the
/// anchor when it fits: `x` is clamped into the window with
/// a 4-DIP margin, and `y` flips up by the card's height
/// when opening downward would cross the bottom margin,
/// then clamps the same way — on a window too small for the
/// card, both axes pin to the 4-DIP margin rather than
/// running off-screen or underflowing.
pub fn ctx_menu_pos(win_w: f64, win_h: f64, anchor: (f64, f64), menu_h: f64) -> (f64, f64) {
    let max_x = (win_w - CTX_MENU_W - 4.0).max(4.0);
    let x = anchor.0.clamp(4.0, max_x);
    let y = if anchor.1 + menu_h <= win_h - 4.0 {
        anchor.1
    } else {
        anchor.1 - menu_h
    };
    let max_y = (win_h - menu_h - 4.0).max(4.0);
    (x, y.clamp(4.0, max_y))
}

/// The chrome-rect EXTENSION in DIP beyond CHROME_H needed
/// to contain a context menu whose card top sits at `pos_y`
/// (window DIP) with height `menu_h`: the card's bottom
/// plus the same 10 DIP of shadow slack the downloads
/// popover and toast get, minus the chrome height already
/// present. A menu that opens upward into the chrome's own
/// band needs no extension at all.
pub fn ctx_menu_extra(pos_y: f64, menu_h: f64) -> f64 {
    (pos_y + menu_h + 10.0 - CHROME_H).max(0.0)
}

/// The chrome rect extended downward by `extra_dip` logical
/// pixels (scaled, clamped to the window height like the base
/// rect). With `extra_dip == 0.0` this is exactly `tile()`'s
/// chrome rect.
pub fn chrome_rect_with_extra(inner_w: u32, inner_h: u32, scale: f64, extra_dip: f64) -> PhysRect {
    let (chrome, _) = tile(inner_w, inner_h, scale);
    if extra_dip <= 0.0 {
        return chrome;
    }
    let extra = (extra_dip * sane_scale(scale)).round().max(0.0) as u32;
    PhysRect {
        h: chrome.h.saturating_add(extra).min(inner_h),
        ..chrome
    }
}

/// The chrome rect, optionally extended for the Teleport panel.
/// With `panel_open == false` this is exactly `tile()`'s chrome
/// rect; with it, the height grows by [`TELEPORT_PANEL_H`]
/// (scaled, clamped to the window height like the base rect).
pub fn chrome_rect_with_panel(
    inner_w: u32,
    inner_h: u32,
    scale: f64,
    panel_open: bool,
) -> PhysRect {
    if !panel_open {
        let (chrome, _) = tile(inner_w, inner_h, scale);
        return chrome;
    }
    chrome_rect_with_extra(inner_w, inner_h, scale, TELEPORT_PANEL_H)
}

/// Tile a window client area of `inner_w`×`inner_h` device
/// pixels into `(chrome, content)` rects. The chrome spans rows
/// `[0, chrome_h)`; the content spans `[solid_h, inner_h)` —
/// the rows `[solid_h, chrome_h)` are covered by *both* rects
/// (the feather overlap), and every other row by exactly one.
/// Both span the full width, so no window background can ever
/// show as a seam between the feather's end and the content's
/// start.
pub fn tile(inner_w: u32, inner_h: u32, scale: f64) -> (PhysRect, PhysRect) {
    let ch = chrome_height_phys(scale).min(inner_h);
    let chrome = PhysRect {
        x: 0,
        y: 0,
        w: inner_w,
        h: ch,
    };
    let top = solid_height_phys(scale).min(inner_h);
    let content = PhysRect {
        x: 0,
        y: top as i32,
        w: inner_w,
        h: inner_h - top,
    };
    (chrome, content)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCALES: [f64; 5] = [1.0, 1.25, 1.5, 1.75, 2.0];
    const SIZES: [(u32, u32); 7] = [
        (1180, 780),
        (1920, 1040),
        (2560, 1392),
        (761, 501),
        (1365, 767),
        (1023, 599),
        (801, 1001),
    ];

    #[test]
    fn geometry_constants_are_the_v22_contract() {
        assert_eq!(CHROME_H, 86.0, "total chrome slot");
        assert_eq!(SOLID_H, 74.0, "solid region / content top");
        assert_eq!(FEATHER_H, 12.0, "feather overlap is derived");
    }

    #[test]
    fn tiling_overlaps_by_exactly_the_feather_at_every_scale() {
        for sf in SCALES {
            for (w, h) in SIZES {
                let (chrome, content) = tile(w, h, sf);
                assert_eq!(chrome.x, 0);
                assert_eq!(chrome.y, 0);
                assert_eq!(chrome.w, w, "chrome width at sf {sf}");
                assert_eq!(content.x, 0);
                assert_eq!(content.w, w, "content width at sf {sf}");
                assert_eq!(
                    chrome.h,
                    (CHROME_H * sf).round() as u32,
                    "chrome height is round(86 * sf) at sf {sf}"
                );
                assert_eq!(
                    content.y as u32,
                    (SOLID_H * sf).round() as u32,
                    "content top is round(74 * sf) at sf {sf}"
                );
                let overlap = chrome.h - content.y as u32;
                assert_eq!(
                    overlap,
                    (FEATHER_H * sf).round() as u32,
                    "feather overlap is round(12 * sf) at sf {sf}"
                );
                assert_eq!(
                    content.y as u32 + content.h,
                    h,
                    "content must reach the bottom edge at sf {sf} size {w}x{h}"
                );
            }
        }
    }

    #[test]
    fn tiny_windows_clamp_without_underflow() {
        let (chrome, content) = tile(320, 60, 1.0);
        assert_eq!(chrome.h, 60);
        assert_eq!(content.y, 60);
        assert_eq!(content.h, 0);
        let (chrome, content) = tile(320, 40, 1.0);
        assert_eq!(chrome.h, 40);
        assert_eq!(content.y, 40);
        assert_eq!(content.h, 0);
        let (chrome, content) = tile(320, 0, 1.5);
        assert_eq!(chrome.h, 0);
        assert_eq!(content.y, 0);
        assert_eq!(content.h, 0);
    }

    /// AUDIT V26 D2: the invariant the user asked for, stated
    /// flatly — for every window size (incl. 2048×1152 and true
    /// ultrawides) × panel state × DSF, the CONTENT rect is one
    /// pure function of (window, scale): identical open vs
    /// closed, top at the contracted 74-DIP line, bottom exactly
    /// at the window's bottom edge (no band of window background
    /// can ever show beneath it).
    #[test]
    fn v26_content_rect_is_panel_independent_at_all_sizes() {
        const V26_SIZES: [(u32, u32); 6] = [
            (760, 500),
            (1280, 800),
            (2048, 1152),
            (2560, 1080),
            (3440, 1440),
            (1920, 1080),
        ];
        for sf in [1.0, 1.25, 1.5, 2.0] {
            for (w, h) in V26_SIZES {
                let (_, content_closed) = tile(w, h, sf);
                // chrome_rect_with_panel is the only panel-aware
                // entry point, and it returns ONLY the chrome
                // rect; the content rect comes from tile() in
                // both states by construction in app.rs. Assert
                // the pair (grown chrome, same content) here:
                let chrome_open = chrome_rect_with_panel(w, h, sf, true);
                let chrome_closed = chrome_rect_with_panel(w, h, sf, false);
                assert!(
                    chrome_open.h >= chrome_closed.h,
                    "panel only ever grows the chrome rect"
                );
                assert_eq!(
                    content_closed.y as u32,
                    (SOLID_H * sf).round() as u32,
                    "content top is the 74-DIP contract at sf {sf} size {w}x{h}"
                );
                assert_eq!(
                    content_closed.y as u32 + content_closed.h,
                    h,
                    "content reaches the window's bottom edge at sf {sf} size {w}x{h}"
                );
                assert_eq!(content_closed.x, 0);
                assert_eq!(content_closed.w, w);
            }
        }
    }

    #[test]
    fn teleport_panel_extends_chrome_without_touching_content() {
        for sf in SCALES {
            for (w, h) in SIZES {
                let (base, content) = tile(w, h, sf);
                let closed = chrome_rect_with_panel(w, h, sf, false);
                assert_eq!(closed, base, "closed panel = the v22 rect at sf {sf}");
                let open = chrome_rect_with_panel(w, h, sf, true);
                let extra = (TELEPORT_PANEL_H * sf).round() as u32;
                assert_eq!(
                    open.h,
                    (base.h + extra).min(h),
                    "open panel grows the chrome rect only, at sf {sf}"
                );
                assert_eq!(open.x, 0);
                assert_eq!(open.y, 0);
                assert_eq!(open.w, w);
                // Content geometry is the tiling's, panel or not.
                assert_eq!(content.y as u32, (SOLID_H * sf).round() as u32);
            }
        }
    }

    #[test]
    fn downloads_popover_h_closed_form_values() {
        // 0 rows: card = 20+26+8+30+8+30+8+28 = 158;
        // 78+158+10-86 = 160.
        assert_eq!(downloads_popover_h(0, 0), 160.0);
        // 1 row: list = 76; card = 204; 78+204+10-86 = 206.
        assert_eq!(downloads_popover_h(1, 0), 206.0);
        // Cap: many rows pin the list at 386 and the card at
        // POPOVER_MAX_H (480), so the extension is
        // 78+480+10-86 = 482.
        assert_eq!(downloads_popover_h(20, 0), 482.0);
        assert_eq!(downloads_popover_h(100, 100), 482.0);
        // The 0.35 constants themselves are the CSS contract.
        assert_eq!(DL_ROW_H, 76.0);
        assert_eq!(DL_POPOVER_FOOT_H, 28.0);
    }

    #[test]
    fn downloads_popover_h_error_rows_add_height() {
        let plain = downloads_popover_h(2, 0);
        let with_err = downloads_popover_h(2, 1);
        assert_eq!(with_err - plain, DL_ROW_ERR_H);
        assert_eq!(downloads_popover_h(2, 2) - plain, 2.0 * DL_ROW_ERR_H);
        // The empty state ignores error rows (there are no rows).
        assert_eq!(downloads_popover_h(0, 3), downloads_popover_h(0, 0));
    }

    #[test]
    fn downloads_toast_h_closed_form_value() {
        // 78 (anchor) + 34 (pill) + 10 (shadow slack) − 86
        // (chrome already present) = 36 DIP of extension,
        // and the constants themselves are the CSS contract.
        assert_eq!(downloads_toast_h(), 36.0);
        assert_eq!(DL_TOAST_TOP, 78.0);
        assert_eq!(DL_TOAST_H, 34.0);
    }

    #[test]
    fn chrome_rect_with_extra_scales_the_extension() {
        for sf in SCALES {
            let (base, _) = tile(1920, 1040, sf);
            let closed = chrome_rect_with_extra(1920, 1040, sf, 0.0);
            assert_eq!(closed, base, "zero extra = the plain rect at sf {sf}");
            let grown = chrome_rect_with_extra(1920, 1040, sf, downloads_popover_h(0, 0));
            let extra = (downloads_popover_h(0, 0) * sf).round() as u32;
            assert_eq!(
                grown.h,
                (base.h + extra).min(1040),
                "extra scales with the window at sf {sf}"
            );
            // chrome_rect_with_panel is the same body at
            // TELEPORT_PANEL_H.
            assert_eq!(
                chrome_rect_with_panel(1920, 1040, sf, true),
                chrome_rect_with_extra(1920, 1040, sf, TELEPORT_PANEL_H),
                "panel rect == extra rect at sf {sf}"
            );
            assert_eq!(
                chrome_rect_with_panel(1920, 1040, sf, false),
                base,
                "closed panel == extra 0 at sf {sf}"
            );
        }
    }

    #[test]
    fn ctx_menu_h_math() {
        // Padding only: 2*6.
        assert_eq!(ctx_menu_h(0, 0), 12.0);
        // The plain-page menu: 4 rows (back/forward/reload/
        // inspect) + 1 separator = 12 + 120 + 9.
        assert_eq!(ctx_menu_h(4, 1), 141.0);
        // The maximal menu: 14 rows + 4 separators.
        assert_eq!(ctx_menu_h(14, 4), 468.0);
        assert_eq!(CTX_MENU_W, 236.0);
        assert_eq!(CTX_ROW_H, 30.0);
        assert_eq!(CTX_SEP_H, 9.0);
        assert_eq!(CTX_MENU_PAD_Y, 6.0);
    }

    #[test]
    fn ctx_menu_pos_opens_at_the_anchor_when_it_fits() {
        assert_eq!(
            ctx_menu_pos(1280.0, 800.0, (100.0, 100.0), 141.0),
            (100.0, 100.0)
        );
        // An anchor inside the margins at the very corner
        // clamps up to the 4-DIP margin, never to 0.
        assert_eq!(ctx_menu_pos(1280.0, 800.0, (0.0, 0.0), 141.0), (4.0, 4.0));
    }

    #[test]
    fn ctx_menu_pos_flips_up_at_the_bottom() {
        // 750 + 141 = 891 > 800 − 4: the card opens upward,
        // its top at anchor − height.
        assert_eq!(
            ctx_menu_pos(1280.0, 800.0, (100.0, 750.0), 141.0),
            (100.0, 609.0)
        );
        // Exactly fitting (655 + 141 = 796 = 800 − 4) does
        // NOT flip.
        assert_eq!(
            ctx_menu_pos(1280.0, 800.0, (100.0, 655.0), 141.0),
            (100.0, 655.0)
        );
    }

    #[test]
    fn ctx_menu_pos_clamps_at_the_right_edge() {
        // 1280 − 236 (card) − 4 (margin) = 1040.
        assert_eq!(
            ctx_menu_pos(1280.0, 800.0, (1200.0, 100.0), 141.0),
            (1040.0, 100.0)
        );
        assert_eq!(
            ctx_menu_pos(1280.0, 800.0, (1040.0, 100.0), 141.0),
            (1040.0, 100.0)
        );
    }

    #[test]
    fn ctx_menu_pos_tiny_window_pins_to_the_margin() {
        // A window narrower than the card and shorter than
        // the menu: both axes pin to 4 rather than inverting
        // the clamp range or underflowing.
        assert_eq!(ctx_menu_pos(200.0, 100.0, (50.0, 50.0), 141.0), (4.0, 4.0));
        assert_eq!(ctx_menu_pos(0.0, 0.0, (0.0, 0.0), 141.0), (4.0, 4.0));
    }

    #[test]
    fn ctx_menu_extra_uses_the_toast_slack_convention() {
        // Bottom at 100 + 141 = 241; 241 + 10 (shadow slack)
        // − 86 (chrome already present) = 165.
        assert_eq!(ctx_menu_extra(100.0, 141.0), 165.0);
        // A menu fully inside the chrome band needs nothing.
        assert_eq!(ctx_menu_extra(4.0, 30.0), 0.0);
    }

    #[test]
    fn degenerate_scale_falls_back_to_1x() {
        assert_eq!(chrome_height_phys(0.0), 86);
        assert_eq!(chrome_height_phys(f64::NAN), 86);
        assert_eq!(chrome_height_phys(-2.0), 86);
        assert_eq!(solid_height_phys(0.0), 74);
        assert_eq!(feather_height_phys(f64::INFINITY), 12);
    }
}
