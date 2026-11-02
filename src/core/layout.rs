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
    let (chrome, _) = tile(inner_w, inner_h, scale);
    if !panel_open {
        return chrome;
    }
    let extra = (TELEPORT_PANEL_H * sane_scale(scale)).round().max(0.0) as u32;
    PhysRect {
        h: chrome.h.saturating_add(extra).min(inner_h),
        ..chrome
    }
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
    fn degenerate_scale_falls_back_to_1x() {
        assert_eq!(chrome_height_phys(0.0), 86);
        assert_eq!(chrome_height_phys(f64::NAN), 86);
        assert_eq!(chrome_height_phys(-2.0), 86);
        assert_eq!(solid_height_phys(0.0), 74);
        assert_eq!(feather_height_phys(f64::INFINITY), 12);
    }
}
