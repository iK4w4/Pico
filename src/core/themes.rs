//! The theme system (AUDIT V27, Workstream A; extended
//! 0.31): sixteen user-supplied palettes, ONE role mapping,
//! ONE derivation.
//!
//! Through v26 the UI carried exactly one palette (the violet
//! D8 set) hand-written into `assets/style.css` as two
//! `[data-theme]` blocks. The user then supplied NINE palettes
//! (screenshots of the @uiuxaadi stack cards, hexes read from
//! the images) and asked for all of them as switchable themes,
//! with Forest — the first card of the set — as the new
//! default theme. 0.31 added seven more cards (the light-
//! anchored set; mapping addendum below).
//!
//! A theme here is DATA, not CSS: [`PALETTES`] holds the six
//! hexes per palette in stack order, and [`derive`] turns a
//! palette + mode into the full design-token set by the single
//! documented mapping below. [`themes_css`] renders every
//! palette × mode into `[data-palette="…"][data-theme="…"]`
//! blocks, which the page assembly appends after the shared
//! stylesheet; the documents select a theme purely through
//! those two attributes. No per-theme CSS files exist, so the
//! sixteen themes cannot drift apart structurally — the purity
//! and contrast tests re-run this derivation and compare.
//!
//! ROLE MAPPING (one mapping, applied uniformly; slots are the
//! stack order of the source cards, top → bottom):
//!   slot 1  deep base  → dark surface; light-mode ink
//!   slot 2  raised     → dark raised-family / backdrop tint
//!   slot 3  mid        → fills; light accent
//!   slot 4  accent-lt  → dark accent; shadow tints
//!   slot 5  pale       → dark text; light-mode surface tint
//!   slot 6  dual       → per-palette override (see below)
//!
//! SLOT 6 is an accent-2 (orange/gold/coral/pink) in six stacks
//! and a slate in three (Forest, Violet, Bronze). The override
//! is one column of the table ([`Slot6::Accent2`] /
//! [`Slot6::Slate`]): an Accent2 slot 6 feeds `--glow-soft`
//! AND the theme's `--danger` step (a hot color the user
//! picked is a better danger than an imported red); a Slate
//! slot 6 becomes the well/inset tint, and danger falls back
//! to the standing semantic hue class (a red stepped in-family
//! for AA, the pre-v27 exception rule). `--ok` / `--warn` are
//! the same standing-hue class in every theme.
//! (AUDIT V28, A3-M2: slot 6 and slot 3 used to also be emitted
//! as the `--accent2` / `--accent-strong` tokens. Nothing in
//! any stylesheet consumed either — and accent-strong was
//! unstepped, an AA trap documented as "fills". Both tokens
//! are gone; the values live on inside the derivation.)
//!
//! 0.31 ADDENDUM — SEVEN MORE CARDS, LIGHT-ANCHORED: the user
//! supplied seven further @uiuxaadi cards (sixteen palettes
//! total). These cards differ from the v27 set in two ways:
//! they NAME their roles (Primary / Accent / Secondary /
//! Surface / Background, plus a Muted on the slate card), and
//! they are light-anchored ramps — one deep color, then a
//! vivid accent, then pale tints down to near-white. The
//! mapping into the six slots (one mapping, all seven):
//!   slot 1 = the card's Primary (deep base / light ink)
//!   slot 2 = DERIVED: mix(Primary → Accent, 0.35). The one
//!            rung the light-anchored cards don't carry (their
//!            stacks jump from Primary straight to the vivid
//!            Accent); derived from the card's own two deep
//!            colors with the same mix() the derivation uses,
//!            pinned as a literal and re-checked by a test
//!            against the recipe.
//!   slot 3 = the card's Accent (the saturated workhorse:
//!            light-mode accent/muted, stepped in-hue to AA)
//!   slot 4 = the card's Secondary (the pale in-family tint:
//!            dark-mode accent/faint — the same role slot 4
//!            plays for the v27 palettes)
//!   slot 5 = the card's Surface (dark-mode text; light-mode
//!            material base). The card's Background is NOT a
//!            slot: Pico's one-material law makes --bg ==
//!            --surface, so the card's Background/Surface
//!            pair collapses to its Surface rung.
//!   slot 6 = Slate class for all seven (no card carries a
//!            hot second accent): Mist Slate's own Muted
//!            (#C5BAC4 — the one card that ships a sixth
//!            color); for the other six, DERIVED:
//!            mix(Secondary → Surface, 0.50), a muted
//!            mid-pale in-family tone, pinned + recipe-tested
//!            like slot 2. Danger falls back to the standing
//!            red hue class, as for Forest/Violet/Bronze.
//! All hexes below were read from the seven uploaded images
//! and verified slot-by-slot against the parent's
//! transcription — the two agreed on every slot, so no
//! image-vs-transcription correction was needed. (File order
//! is not palette order: photo_7 = Deep Teal, photo_1 = Mist
//! Slate, photo_2 = Lavender, photo_3 = Royal Blue, photo_4 =
//! Tangerine, photo_5 = Amethyst, photo_6 = Rose.)
//!
//! DERIVATION (deterministic; the test suite re-runs it):
//! - DARK: surface = slot 1 verbatim (the D8 law: the base is
//!   not lifted); raised = mix(slot 1 → slot 2, 0.50); text =
//!   slot 5; muted = mix(slot 5 → slot 4, 0.30); faint/accent =
//!   slot 4. Text-class tokens (text,
//!   muted, faint, accent, danger) are stepped in HSL lightness
//!   toward white until they hold 4.5:1 on the surface (in
//!   practice only a few move, and only within their hue).
//! - LIGHT: surface = mix(slot 5 → white, 0.30); raised = one
//!   more tint step; text = slot 1; muted/accent = slot 3;
//!   faint = mix(slot 1 → surface, 0.42); text-class tokens
//!   stepped toward black to the same 4.5:1.
//! - `--accent-text` is whichever of slot 1 / slot 5 / white
//!   contrasts best on the accent.
//! - Shadow tints, focus tints, feather shade and backdrop are
//!   rgba() of palette colors at the alphas v26 pinned.
//! - `--glow` / `--glow-soft` (the v27 futuristic layer) are
//!   box-shadow values built from the theme's OWN accent (and
//!   accent-2): the glow can never be a foreign color. Light
//!   mode carries a reduced alpha — glow on a pale material
//!   reads as haze, not neon.
//!
//! RAISED PAIRINGS (AUDIT 0.31, A3-F3): text-class tokens are
//! stepped for AA on the SURFACE, not on `--raised`. The one
//! rendered pairing against `--raised` — `--text` on
//! `--raised` — does hold AA in all 32 derivations and is
//! pinned by a test. Arbitrary token-on-`--raised` pairings
//! are NOT all AA (worst known: Tangerine dark
//! danger-on-raised at 3.41:1), and only the rendered
//! pairings are pinned — future UI must check contrast
//! before pairing any token against `--raised`.

/// One sRGB color, 8-bit channels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub fn from_hex(s: &str) -> Rgb {
        // The palette table's literals are pinned by tests, so
        // a malformed literal is a bug the suite catches, never
        // a runtime case. AUDIT V28 (A3-LOW-4): the function is
        // total anyway — a short string used to panic on the
        // byte indexing; it yields black instead.
        if s.len() < 7 {
            return BLACK;
        }
        fn nib(b: &[u8], i: usize) -> u8 {
            let c = b[i] as char;
            match c {
                '0'..='9' => (c as u8) - b'0',
                'a'..='f' => (c as u8) - b'a' + 10,
                'A'..='F' => (c as u8) - b'A' + 10,
                _ => 0,
            }
        }
        let b = s.as_bytes();
        // `s` is "#RRGGBB".
        Rgb {
            r: nib(b, 1) * 16 + nib(b, 2),
            g: nib(b, 3) * 16 + nib(b, 4),
            b: nib(b, 5) * 16 + nib(b, 6),
        }
    }

    pub fn to_hex(&self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }

    /// Linear mix toward `other` by `t` (0 = self, 1 = other),
    /// per channel in sRGB space — the same "mix of two palette
    /// colors" operation the v26 hand-written CSS documented.
    pub fn mix(&self, other: Rgb, t: f64) -> Rgb {
        let t = t.clamp(0.0, 1.0);
        let ch = |a: u8, b: u8| -> u8 { (a as f64 + (b as f64 - a as f64) * t).round() as u8 };
        Rgb {
            r: ch(self.r, other.r),
            g: ch(self.g, other.g),
            b: ch(self.b, other.b),
        }
    }

    /// WCAG relative luminance.
    pub fn luminance(&self) -> f64 {
        let f = |c: u8| -> f64 {
            let s = c as f64 / 255.0;
            if s <= 0.03928 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * f(self.r) + 0.7152 * f(self.g) + 0.0722 * f(self.b)
    }

    /// WCAG contrast ratio against another color (1.0–21.0).
    pub fn contrast(&self, other: Rgb) -> f64 {
        let (a, b) = (self.luminance(), other.luminance());
        let (hi, lo) = if a >= b { (a, b) } else { (b, a) };
        (hi + 0.05) / (lo + 0.05)
    }

    fn to_hsl(self) -> (f64, f64, f64) {
        let (r, g, b) = (
            self.r as f64 / 255.0,
            self.g as f64 / 255.0,
            self.b as f64 / 255.0,
        );
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let l = (max + min) / 2.0;
        if (max - min).abs() < f64::EPSILON {
            return (0.0, 0.0, l);
        }
        let d = max - min;
        let s = if l > 0.5 {
            d / (2.0 - max - min)
        } else {
            d / (max + min)
        };
        let h = if max == r {
            60.0 * (((g - b) / d) % 6.0)
        } else if max == g {
            60.0 * ((b - r) / d + 2.0)
        } else {
            60.0 * ((r - g) / d + 4.0)
        };
        ((h + 360.0) % 360.0, s, l)
    }

    fn from_hsl(h: f64, sat: f64, l: f64) -> Rgb {
        let h = ((h % 360.0) + 360.0) % 360.0;
        let c = (1.0 - (2.0 * l - 1.0).abs()) * sat;
        let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
        let m = l - c / 2.0;
        let (r, g, b) = match h {
            v if v < 60.0 => (c, x, 0.0),
            v if v < 120.0 => (x, c, 0.0),
            v if v < 180.0 => (0.0, c, x),
            v if v < 240.0 => (0.0, x, c),
            v if v < 300.0 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let ch = |v: f64| -> u8 { ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8 };
        Rgb {
            r: ch(r),
            g: ch(g),
            b: ch(b),
        }
    }

    /// Step HSL lightness by `dl` (hue and saturation fixed —
    /// an in-family step, the only adjustment the AA discipline
    /// is allowed).
    pub fn step_lightness(&self, dl: f64) -> Rgb {
        let (h, s, l) = self.to_hsl();
        Rgb::from_hsl(h, s, (l + dl).clamp(0.0, 1.0))
    }

    /// A color of a standing hue (the semantic exception class:
    /// ok/warn/danger when the palette supplies none), at the
    /// lightness that holds `target` contrast on `bg`.
    fn standing(h: f64, s: f64, bg: Rgb, target: f64, lighten: bool) -> Rgb {
        let mut l = 0.45;
        for _ in 0..40 {
            let c = Rgb::from_hsl(h, s, l);
            if c.contrast(bg) >= target {
                return c;
            }
            l += if lighten { 0.02 } else { -0.02 };
            if !(0.02..=0.98).contains(&l) {
                break;
            }
        }
        Rgb::from_hsl(h, s, l.clamp(0.02, 0.98))
    }
}

/// Step `color` in lightness (toward white when `lighten`,
/// toward black otherwise) until it holds `target` contrast
/// on `bg`, at most 16 steps of 0.03. Returns the color
/// unchanged when it already passes — most palette colors do.
fn ensure_contrast(color: Rgb, bg: Rgb, target: f64, lighten: bool) -> Rgb {
    let mut c = color;
    for _ in 0..16 {
        if c.contrast(bg) >= target {
            return c;
        }
        c = c.step_lightness(if lighten { 0.03 } else { -0.03 });
    }
    c
}

const WHITE: Rgb = Rgb {
    r: 255,
    g: 255,
    b: 255,
};
const BLACK: Rgb = Rgb { r: 0, g: 0, b: 0 };

fn rgba(c: Rgb, alpha: f64) -> String {
    format!("rgba({}, {}, {}, {})", c.r, c.g, c.b, trim_alpha(alpha))
}

fn trim_alpha(a: f64) -> String {
    // Stable two-decimal rendering (0.55, 0.3 → "0.3"): the CSS
    // is string-compared by tests against re-derivation, so the
    // float formatting must be deterministic.
    let s = format!("{a:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// What slot 6 means in one palette (the documented override).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot6 {
    /// A hot second accent (orange/gold/coral/pink): becomes
    /// `--accent2` and the theme's `--danger` step.
    Accent2,
    /// A slate: becomes the well/inset tint; danger falls back
    /// to the standing red hue class.
    Slate,
}

/// One user-supplied palette: six hexes in stack order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub id: &'static str,
    pub name: &'static str,
    pub slots: [&'static str; 6],
    pub slot6: Slot6,
}

const fn pal(
    id: &'static str,
    name: &'static str,
    slots: [&'static str; 6],
    slot6: Slot6,
) -> Palette {
    Palette {
        id,
        name,
        slots,
        slot6,
    }
}

/// The sixteen palettes, in the order the user supplied
/// them (the v27 nine first, then the 0.31 seven). Hexes are
/// the TEXT values read from his images — they govern over
/// any pixel sampling of the screenshots. For the 0.31
/// seven, slots 2 (all) and 6 (six of seven) are derived
/// literals per the 0.31 addendum at the top of this file;
/// the recipe test pins them against the derivation's own
/// mix(), so a hand-slip in a literal fails the suite.
pub const PALETTES: &[Palette] = &[
    pal(
        "forest",
        "Forest",
        [
            "#0F1F17", "#1E3A2E", "#3E5A3F", "#6B7F67", "#C7CBB8", "#3D4337",
        ],
        Slot6::Slate,
    ),
    pal(
        "ember",
        "Ember Navy",
        [
            "#0B132B", "#1C2541", "#3A506B", "#5BC0DE", "#F4D6B0", "#D97706",
        ],
        Slot6::Accent2,
    ),
    pal(
        "sage",
        "Sage Gold",
        [
            "#0B0F0D", "#1F2A1F", "#3E5A3F", "#8CA98C", "#D9C7A3", "#B08D57",
        ],
        Slot6::Accent2,
    ),
    pal(
        "lagoon",
        "Lagoon",
        [
            "#0A0F14", "#0F3D3E", "#2E6F73", "#7FB3B8", "#EADCC8", "#C6601F",
        ],
        Slot6::Accent2,
    ),
    pal(
        "ultraviolet",
        "Ultraviolet",
        [
            "#1A0F2E", "#3B1B4F", "#7A2E6F", "#D6598F", "#F4D7D4", "#FF6B6B",
        ],
        Slot6::Accent2,
    ),
    pal(
        "indigo",
        "Indigo Amber",
        [
            "#1A1A1F", "#2D2B55", "#6A5ACD", "#A678B9", "#F0D9B5", "#F4A62A",
        ],
        Slot6::Accent2,
    ),
    pal(
        "graphite",
        "Graphite Pink",
        [
            "#121212", "#1F2937", "#334155", "#64748B", "#F1F5F9", "#EC4899",
        ],
        Slot6::Accent2,
    ),
    pal(
        "violet",
        "Violet",
        [
            "#0E0B1D", "#2A1E4D", "#5A3B8C", "#8B7CC7", "#C9C2E3", "#3A3A52",
        ],
        Slot6::Slate,
    ),
    pal(
        "bronze",
        "Bronze",
        [
            "#1A120E", "#3A2418", "#6B4A2E", "#A88462", "#D5C4AE", "#4A4A46",
        ],
        Slot6::Slate,
    ),
    // The 0.31 seven (light-anchored cards; mapping in the
    // addendum at the top of this file). Slot order here is
    // the SLOT order, not the card order: [Primary, derived
    // raised rung, Accent, Secondary, Surface, slot 6].
    pal(
        "teal",
        "Deep Teal",
        [
            "#0B3D3B", "#0E6860", "#14B8A6", "#99F6E4", "#E6FFFB", "#C0FBF0",
        ],
        Slot6::Slate,
    ),
    pal(
        "mist",
        "Mist Slate",
        [
            "#191D23", "#2F3A41", "#57707A", "#7B919C", "#989DAA", "#C5BAC4",
        ],
        Slot6::Slate,
    ),
    pal(
        "lavender",
        "Lavender",
        [
            "#0D0B2D", "#282177", "#5B4BFF", "#A78BFA", "#E6E0FF", "#C7B6FD",
        ],
        Slot6::Slate,
    ),
    pal(
        "royal",
        "Royal Blue",
        [
            "#0B1F3B", "#143779", "#2563EB", "#60A5FA", "#DBEAFE", "#9EC8FC",
        ],
        Slot6::Slate,
    ),
    pal(
        "tangerine",
        "Tangerine",
        [
            "#1F1D2B", "#6D3E1C", "#FF7A00", "#FFA94D", "#FFE4C7", "#FFC78A",
        ],
        Slot6::Slate,
    ),
    pal(
        "amethyst",
        "Amethyst",
        [
            "#5B21B6", "#6C36CC", "#8B5CF6", "#C7B3FF", "#EDE9FE", "#DACEFF",
        ],
        Slot6::Slate,
    ),
    pal(
        "rose",
        "Rose",
        [
            "#6D0D3F", "#99225F", "#EC4899", "#F9A8D4", "#FCE7F3", "#FBC8E4",
        ],
        Slot6::Slate,
    ),
];

/// The default palette for a fresh install (the user's "use
/// this as the new theme": the first card of his set). Existing
/// installs are migrated TO it on the v27 first run as well —
/// see `core::state` and AUDIT V27-A2 for the resolved rule.
pub const DEFAULT_PALETTE_ID: &str = "forest";
// AUDIT V28 (A3-LOW-4): a LEGACY_PALETTE_ID constant ("violet")
// used to sit here; nothing referenced it — the pre-v27 palette
// is simply the "violet" entry of PALETTES, one click away in
// the picker like any other.

pub fn palette_by_id(id: &str) -> Option<&'static Palette> {
    PALETTES.iter().find(|p| p.id == id)
}

pub fn is_known_palette(id: &str) -> bool {
    palette_by_id(id).is_some()
}

impl Palette {
    pub fn slot(&self, i: usize) -> Rgb {
        Rgb::from_hex(self.slots[i])
    }
}

/// The token names every derived theme defines, in emission
/// order. The page stylesheets reference exactly these (plus
/// the mode-independent radius scale and the --neu-* geometry
/// vocabulary, which live in the shared stylesheet because
/// they only compose other tokens).
pub const TOKEN_NAMES: &[&str] = &[
    "--bg",
    "--surface",
    "--raised",
    "--text",
    "--muted",
    "--faint",
    "--accent",
    "--accent-text",
    "--ok",
    "--warn",
    "--danger",
    "--on-danger",
    "--shadow-dark",
    "--shadow-light",
    "--inset-line",
    "--focus-glow",
    "--focus-outline",
    "--feather-shade",
    "--backdrop",
    "--glow",
    "--glow-soft",
];

/// Whichever candidate contrasts best on `bg`.
fn best_on(bg: Rgb, candidates: &[Rgb]) -> Rgb {
    let mut best = candidates[0];
    let mut best_ratio = 0.0;
    for &c in candidates {
        let r = c.contrast(bg);
        if r > best_ratio {
            best_ratio = r;
            best = c;
        }
    }
    best
}

/// Derive the full token set for one palette + mode. Pure and
/// deterministic — the same inputs always emit the same
/// (name, value) pairs, which is what lets the test suite pin
/// every theme from this one source.
pub fn derive(p: &Palette, dark: bool) -> Vec<(&'static str, String)> {
    let s1 = p.slot(0);
    let s2 = p.slot(1);
    let s3 = p.slot(2);
    let s4 = p.slot(3);
    let s5 = p.slot(4);
    let s6 = p.slot(5);
    let aa = 4.5;

    let (surface, raised, text, muted, faint, accent, accent2, danger, ok, warn);
    let (shadow_dark, shadow_light, inset_line, backdrop);
    let glow_alpha;
    if dark {
        surface = s1;
        raised = s1.mix(s2, 0.50);
        text = ensure_contrast(s5, surface, aa, true);
        muted = ensure_contrast(s5.mix(s4, 0.30), surface, aa, true);
        faint = ensure_contrast(s4, surface, aa, true);
        accent = ensure_contrast(s4, surface, aa, true);
        accent2 = match p.slot6 {
            Slot6::Accent2 => s6,
            Slot6::Slate => s3.mix(s4, 0.50),
        };
        danger = match p.slot6 {
            Slot6::Accent2 => ensure_contrast(s6, surface, aa, true),
            Slot6::Slate => Rgb::standing(0.0, 0.62, surface, aa, true),
        };
        ok = Rgb::standing(145.0, 0.55, surface, aa, true);
        warn = Rgb::standing(24.0, 0.85, surface, aa, true);
        shadow_dark = rgba(BLACK.mix(s1, 0.30), 0.60);
        shadow_light = rgba(s4, 0.16);
        inset_line = match p.slot6 {
            Slot6::Slate => rgba(s6, 0.55),
            Slot6::Accent2 => rgba(BLACK, 0.45),
        };
        backdrop = rgba(s1, 0.55);
        glow_alpha = 0.38;
    } else {
        surface = s5.mix(WHITE, 0.30);
        raised = surface.mix(WHITE, 0.45);
        text = ensure_contrast(s1, surface, aa, false);
        muted = ensure_contrast(s3, surface, aa, false);
        faint = ensure_contrast(s1.mix(surface, 0.42), surface, aa, false);
        accent = ensure_contrast(s3, surface, aa, false);
        accent2 = match p.slot6 {
            Slot6::Accent2 => s6,
            Slot6::Slate => s3.mix(s4, 0.50),
        };
        danger = match p.slot6 {
            Slot6::Accent2 => ensure_contrast(s6, surface, aa, false),
            Slot6::Slate => Rgb::standing(0.0, 0.62, surface, aa, false),
        };
        ok = Rgb::standing(145.0, 0.55, surface, aa, false);
        warn = Rgb::standing(24.0, 0.85, surface, aa, false);
        shadow_dark = rgba(s4, 0.50);
        shadow_light = rgba(WHITE, 0.85);
        inset_line = rgba(s3, 0.40);
        backdrop = rgba(s2, 0.38);
        glow_alpha = 0.22;
    }
    // AUDIT 0.31 (A3-F5): like --on-danger below, the best
    // candidate is stepped in its own direction until it
    // holds AA on the accent. Today every candidate already
    // passes (empirical minimum 4.82:1), so this is a no-op
    // for all 32 derivations — pinned token-for-token by
    // `accent_text_pins_all_32` — but a future palette whose
    // best candidate falls short is now stepped instead of
    // shipping below AA.
    let accent_text_cand = best_on(accent, &[s1, s5, WHITE]);
    let accent_text = ensure_contrast(
        accent_text_cand,
        accent,
        aa,
        accent_text_cand.luminance() > accent.luminance(),
    );
    // AUDIT V28 (A3-M1): the foreground for content sitting ON
    // the danger color (the close button's hover paints the
    // glyph on --danger). Chrome used a hard-coded white,
    // which fails contrast on the light danger steps (Indigo
    // dark's amber danger: 2.03:1). Derived like accent-text:
    // best palette candidate, then stepped in its own
    // direction until it holds AA on danger.
    let on_danger_cand = best_on(danger, &[s1, s5, WHITE, BLACK]);
    let on_danger = ensure_contrast(
        on_danger_cand,
        danger,
        aa,
        on_danger_cand.luminance() > danger.luminance(),
    );
    let feather = if dark {
        rgba(BLACK.mix(s1, 0.30), 0.30)
    } else {
        rgba(s1, 0.14)
    };
    let glow = format!(
        "0 0 16px 0 {}, 0 0 4px 0 {}",
        rgba(accent, glow_alpha),
        rgba(accent, glow_alpha * 0.75)
    );
    let glow_soft = format!("0 0 10px 0 {}", rgba(accent2, glow_alpha * 0.7));

    vec![
        ("--bg", surface.to_hex()),
        ("--surface", surface.to_hex()),
        ("--raised", raised.to_hex()),
        ("--text", text.to_hex()),
        ("--muted", muted.to_hex()),
        ("--faint", faint.to_hex()),
        ("--accent", accent.to_hex()),
        ("--accent-text", accent_text.to_hex()),
        ("--ok", ok.to_hex()),
        ("--warn", warn.to_hex()),
        ("--danger", danger.to_hex()),
        ("--on-danger", on_danger.to_hex()),
        ("--shadow-dark", shadow_dark),
        ("--shadow-light", shadow_light),
        ("--inset-line", inset_line),
        ("--focus-glow", rgba(accent, if dark { 0.18 } else { 0.16 })),
        (
            "--focus-outline",
            rgba(accent, if dark { 0.55 } else { 0.50 }),
        ),
        ("--feather-shade", feather),
        ("--backdrop", backdrop),
        ("--glow", glow),
        ("--glow-soft", glow_soft),
    ]
}

/// Token lookup in a derived set (tests + the app's window
/// background both read through this, never by re-deriving
/// colors in a second place).
pub fn token<'a>(tokens: &'a [(&'static str, String)], name: &str) -> Option<&'a str> {
    tokens
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, v)| v.as_str())
}

/// The derived surface color as RGBA bytes — the tao window /
/// WebView background the app paints behind the pages (AUDIT
/// V14-2's "no black frame" rule, now palette-aware).
pub fn surface_rgba(palette_id: &str, dark: bool) -> (u8, u8, u8, u8) {
    let p = palette_by_id(palette_id).unwrap_or(&PALETTES[0]);
    let tokens = derive(p, dark);
    let hex = token(&tokens, "--surface").unwrap_or("#000000");
    let c = Rgb::from_hex(hex);
    (c.r, c.g, c.b, 255)
}

/// Render every palette × mode as CSS blocks. Appended after
/// the shared stylesheet at page assembly; the documents pick
/// a block via their `data-palette` + `data-theme` attributes.
pub fn themes_css() -> String {
    let mut out = String::with_capacity(16 * 1024);
    out.push_str(
        "/* Theme token sets — GENERATED by core::themes from the sixteen\n   user-supplied palettes (AUDIT V27-A, 0.31). One derivation,\n   sixteen palettes, two modes; do not hand-edit values here,\n   they are re-derived and pinned by the theme tests. */\n",
    );
    for p in PALETTES {
        for dark in [true, false] {
            let mode = if dark { "dark" } else { "light" };
            out.push_str(&format!(
                "[data-palette=\"{}\"][data-theme=\"{}\"] {{\n",
                p.id, mode
            ));
            for (name, value) in derive(p, dark) {
                out.push_str(&format!("  {name}: {value};\n"));
            }
            out.push_str(&format!("  color-scheme: {mode};\n}}\n"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sixteen_palettes_in_user_order() {
        let ids: Vec<&str> = PALETTES.iter().map(|p| p.id).collect();
        assert_eq!(
            ids,
            [
                "forest",
                "ember",
                "sage",
                "lagoon",
                "ultraviolet",
                "indigo",
                "graphite",
                "violet",
                "bronze",
                "teal",
                "mist",
                "lavender",
                "royal",
                "tangerine",
                "amethyst",
                "rose"
            ]
        );
        assert_eq!(DEFAULT_PALETTE_ID, "forest");
    }

    #[test]
    fn palette_hexes_match_the_user_images() {
        // The TEXT hexes read from the user's sixteen images,
        // pinned verbatim — a transcription slip here recolors
        // his browser. (0.31 entries: card colors verbatim in
        // slots 1/3/4/5; slots 2 and 6 are the documented
        // derived literals — see the recipe test below.)
        let expect: &[(&str, [&str; 6])] = &[
            (
                "forest",
                [
                    "#0F1F17", "#1E3A2E", "#3E5A3F", "#6B7F67", "#C7CBB8", "#3D4337",
                ],
            ),
            (
                "ember",
                [
                    "#0B132B", "#1C2541", "#3A506B", "#5BC0DE", "#F4D6B0", "#D97706",
                ],
            ),
            (
                "sage",
                [
                    "#0B0F0D", "#1F2A1F", "#3E5A3F", "#8CA98C", "#D9C7A3", "#B08D57",
                ],
            ),
            (
                "lagoon",
                [
                    "#0A0F14", "#0F3D3E", "#2E6F73", "#7FB3B8", "#EADCC8", "#C6601F",
                ],
            ),
            (
                "ultraviolet",
                [
                    "#1A0F2E", "#3B1B4F", "#7A2E6F", "#D6598F", "#F4D7D4", "#FF6B6B",
                ],
            ),
            (
                "indigo",
                [
                    "#1A1A1F", "#2D2B55", "#6A5ACD", "#A678B9", "#F0D9B5", "#F4A62A",
                ],
            ),
            (
                "graphite",
                [
                    "#121212", "#1F2937", "#334155", "#64748B", "#F1F5F9", "#EC4899",
                ],
            ),
            (
                "violet",
                [
                    "#0E0B1D", "#2A1E4D", "#5A3B8C", "#8B7CC7", "#C9C2E3", "#3A3A52",
                ],
            ),
            (
                "bronze",
                [
                    "#1A120E", "#3A2418", "#6B4A2E", "#A88462", "#D5C4AE", "#4A4A46",
                ],
            ),
            (
                "teal",
                [
                    "#0B3D3B", "#0E6860", "#14B8A6", "#99F6E4", "#E6FFFB", "#C0FBF0",
                ],
            ),
            (
                "mist",
                [
                    "#191D23", "#2F3A41", "#57707A", "#7B919C", "#989DAA", "#C5BAC4",
                ],
            ),
            (
                "lavender",
                [
                    "#0D0B2D", "#282177", "#5B4BFF", "#A78BFA", "#E6E0FF", "#C7B6FD",
                ],
            ),
            (
                "royal",
                [
                    "#0B1F3B", "#143779", "#2563EB", "#60A5FA", "#DBEAFE", "#9EC8FC",
                ],
            ),
            (
                "tangerine",
                [
                    "#1F1D2B", "#6D3E1C", "#FF7A00", "#FFA94D", "#FFE4C7", "#FFC78A",
                ],
            ),
            (
                "amethyst",
                [
                    "#5B21B6", "#6C36CC", "#8B5CF6", "#C7B3FF", "#EDE9FE", "#DACEFF",
                ],
            ),
            (
                "rose",
                [
                    "#6D0D3F", "#99225F", "#EC4899", "#F9A8D4", "#FCE7F3", "#FBC8E4",
                ],
            ),
        ];
        for (id, slots) in expect {
            let p = palette_by_id(id).expect("palette exists");
            assert_eq!(&p.slots, slots, "{id} slots drifted");
        }
    }

    #[test]
    fn violet_dark_recovers_the_v26_material() {
        // The v26 shipped look must survive as the Violet
        // theme's dark mode: same material, same text, same
        // accent (the derivation was built to land here).
        let v = palette_by_id("violet").unwrap();
        let t = derive(v, true);
        assert_eq!(token(&t, "--bg"), Some("#0E0B1D"));
        assert_eq!(token(&t, "--surface"), Some("#0E0B1D"));
        assert_eq!(token(&t, "--text"), Some("#C9C2E3"));
        assert_eq!(token(&t, "--accent"), Some("#8B7CC7"));
        let l = derive(v, false);
        // Light text is slot 1 by the v27 role mapping (v26's
        // hand-written block used slot 2, #2A1E4D — one in-family
        // step lighter; the uniform mapping wins, AUDIT V27-A1).
        assert_eq!(token(&l, "--text"), Some("#0E0B1D"));
        assert_eq!(token(&l, "--accent"), Some("#5A3B8C"));
    }

    #[test]
    fn every_theme_defines_the_full_token_set_with_one_material() {
        for p in PALETTES {
            for dark in [true, false] {
                let t = derive(p, dark);
                let names: Vec<&str> = t.iter().map(|(n, _)| *n).collect();
                assert_eq!(names, TOKEN_NAMES, "{} dark={dark}", p.id);
                assert_eq!(
                    token(&t, "--bg"),
                    token(&t, "--surface"),
                    "{} dark={dark}: --bg must equal --surface (one material)",
                    p.id
                );
            }
        }
    }

    #[test]
    fn dark_surface_is_slot1_verbatim_light_surface_is_a_slot5_tint() {
        for p in PALETTES {
            let d = derive(p, true);
            assert_eq!(
                token(&d, "--surface"),
                Some(p.slots[0]),
                "{} dark surface must be slot 1 verbatim",
                p.id
            );
            let l = derive(p, false);
            let expect = p.slot(4).mix(WHITE, 0.30).to_hex();
            assert_eq!(
                token(&l, "--surface"),
                Some(expect.as_str()),
                "{} light surface must be the documented slot-5 tint",
                p.id
            );
        }
    }

    #[test]
    fn text_class_tokens_hold_aa_on_their_surface() {
        for p in PALETTES {
            for dark in [true, false] {
                let t = derive(p, dark);
                let surface = Rgb::from_hex(token(&t, "--surface").unwrap());
                for name in ["--text", "--muted", "--faint", "--accent", "--danger"] {
                    let c = Rgb::from_hex(token(&t, name).unwrap());
                    assert!(
                        c.contrast(surface) >= 4.5,
                        "{} dark={dark} {name} {} on {} is {:.2}:1 (< 4.5)",
                        p.id,
                        c.to_hex(),
                        surface.to_hex(),
                        c.contrast(surface)
                    );
                }
                // accent-text on the accent itself.
                let accent = Rgb::from_hex(token(&t, "--accent").unwrap());
                let at = Rgb::from_hex(token(&t, "--accent-text").unwrap());
                assert!(
                    at.contrast(accent) >= 4.5,
                    "{} dark={dark} accent-text on accent is {:.2}:1",
                    p.id,
                    at.contrast(accent)
                );
            }
        }
    }

    #[test]
    fn glow_is_built_from_the_themes_own_accent() {
        for p in PALETTES {
            for dark in [true, false] {
                let t = derive(p, dark);
                let accent = Rgb::from_hex(token(&t, "--accent").unwrap());
                let glow = token(&t, "--glow").unwrap();
                let needle = format!("rgba({}, {}, {},", accent.r, accent.g, accent.b);
                assert!(
                    glow.contains(&needle),
                    "{} dark={dark}: --glow {glow} is not built from accent {needle}",
                    p.id
                );
            }
        }
    }

    #[test]
    fn slot6_roles_resolve_as_documented() {
        for p in PALETTES {
            let d = derive(p, true);
            match p.slot6 {
                Slot6::Accent2 => {
                    // AUDIT V28 (A3-M2): --accent2 is no longer
                    // a token; slot 6's visible role is the
                    // --glow-soft tint (and the danger step).
                    let soft = token(&d, "--glow-soft").unwrap();
                    let s6 = p.slot(5);
                    assert!(
                        soft.contains(&format!("rgba({}, {}, {},", s6.r, s6.g, s6.b)),
                        "{}: glow-soft must be built from slot 6, got {soft}",
                        p.id
                    );
                }
                Slot6::Slate => {
                    let line = token(&d, "--inset-line").unwrap();
                    let s6 = p.slot(5);
                    assert!(
                        line.contains(&format!("rgba({}, {}, {},", s6.r, s6.g, s6.b)),
                        "{}: slate slot 6 must tint the inset line, got {line}",
                        p.id
                    );
                }
            }
        }
    }

    #[test]
    fn v28_contrast_ratios_match_independent_recomputation() {
        // AUDIT V28 (A3-M4): the suite's contrast tests compare
        // Rgb::contrast against itself — circular. These
        // constants were recomputed INDEPENDENTLY from the
        // shipped palette hexes (WCAG formula, by the v28
        // themes auditor), so a derivation change that moves a
        // real ratio breaks this test even if the internal
        // machinery stays self-consistent.
        let cases: [(&str, bool, &str, f64); 10] = [
            ("forest", true, "--text", 10.31),
            ("forest", true, "--accent", 4.88),
            ("violet", true, "--text", 11.35),
            ("graphite", false, "--text", 17.58),
            ("bronze", false, "--accent", 5.55),
            // 0.31: the seven light-anchored palettes, ratios
            // recomputed independently (Python WCAG) from the
            // derived token values, same discipline as above.
            ("teal", true, "--text", 11.50),
            ("mist", false, "--text", 8.73),
            ("tangerine", false, "--accent", 4.87),
            ("rose", true, "--accent", 6.51),
            ("amethyst", true, "--text", 7.57),
        ];
        for (id, dark, tok, want) in cases {
            let p = palette_by_id(id).expect("palette");
            let d = derive(p, dark);
            let surface = Rgb::from_hex(token(&d, "--surface").unwrap());
            let c = Rgb::from_hex(token(&d, tok).unwrap());
            let got = c.contrast(surface);
            assert!(
                (got - want).abs() <= 0.2,
                "{id} dark={dark} {tok}: got {got:.2}, independently computed {want}"
            );
        }
    }

    #[test]
    fn v28_every_theme_holds_aa_on_its_real_pairings() {
        // Text-class tokens on the surface, and the on-danger
        // foreground on danger (A3-M1's pairing), for all 32
        // derived themes — including the ensure_contrast cap:
        // if stepping ever runs out of steps, this fails.
        for p in PALETTES {
            for dark in [true, false] {
                let d = derive(p, dark);
                let surface = Rgb::from_hex(token(&d, "--surface").unwrap());
                for tok in [
                    "--text", "--muted", "--faint", "--accent", "--danger", "--ok", "--warn",
                ] {
                    let c = Rgb::from_hex(token(&d, tok).unwrap());
                    assert!(
                        c.contrast(surface) >= 4.5,
                        "{} dark={dark} {tok} on surface = {:.2}",
                        p.id,
                        c.contrast(surface)
                    );
                }
                let danger = Rgb::from_hex(token(&d, "--danger").unwrap());
                let on = Rgb::from_hex(token(&d, "--on-danger").unwrap());
                assert!(
                    on.contrast(danger) >= 4.5,
                    "{} dark={dark} --on-danger on --danger = {:.2}",
                    p.id,
                    on.contrast(danger)
                );
            }
        }
    }

    #[test]
    fn themes_css_covers_all_32_blocks() {
        let css = themes_css();
        for p in PALETTES {
            for mode in ["dark", "light"] {
                let sel = format!("[data-palette=\"{}\"][data-theme=\"{}\"]", p.id, mode);
                assert!(css.contains(&sel), "missing block {sel}");
            }
        }
    }

    #[test]
    fn v31_derived_slots_follow_the_documented_recipes() {
        // The 0.31 light-anchored cards carry no raised rung
        // and (Mist Slate excepted) no sixth color, so two
        // slots are derived literals. This test re-runs the
        // documented recipes from the card colors in the
        // table itself, so a literal that drifts from its
        // recipe fails here even though the hex pin above
        // would still pass.
        const NEW: [&str; 7] = [
            "teal",
            "mist",
            "lavender",
            "royal",
            "tangerine",
            "amethyst",
            "rose",
        ];
        for id in NEW {
            let p = palette_by_id(id).expect("0.31 palette exists");
            // Slot 2 = mix(Primary → Accent, 0.35).
            let want2 = p.slot(0).mix(p.slot(2), 0.35).to_hex();
            assert_eq!(p.slots[1], want2, "{id} slot 2 must be the recipe mix");
            assert_eq!(p.slot6, Slot6::Slate, "{id} slot 6 class must be Slate");
            if id == "mist" {
                // The one card with a sixth color: its own
                // Muted, verbatim.
                assert_eq!(p.slots[5], "#C5BAC4", "mist slot 6 is the card Muted");
            } else {
                // Slot 6 = mix(Secondary → Surface, 0.50).
                let want6 = p.slot(3).mix(p.slot(4), 0.50).to_hex();
                assert_eq!(p.slots[5], want6, "{id} slot 6 must be the recipe mix");
            }
        }
    }

    #[test]
    fn accent_text_pins_all_32() {
        // AUDIT 0.31 (A3-F5): the --accent-text values dumped
        // from the derivation BEFORE accent_text was routed
        // through ensure_contrast. Routing it through the
        // stepper must be a no-op today (empirical minimum
        // contrast 4.82:1), so every one of the 32 values
        // below must survive the change token-for-token.
        let expect: &[(&str, bool, &str)] = &[
            ("forest", true, "#0F1F17"),
            ("forest", false, "#FFFFFF"),
            ("ember", true, "#0B132B"),
            ("ember", false, "#FFFFFF"),
            ("sage", true, "#0B0F0D"),
            ("sage", false, "#FFFFFF"),
            ("lagoon", true, "#0A0F14"),
            ("lagoon", false, "#FFFFFF"),
            ("ultraviolet", true, "#1A0F2E"),
            ("ultraviolet", false, "#FFFFFF"),
            ("indigo", true, "#1A1A1F"),
            ("indigo", false, "#FFFFFF"),
            ("graphite", true, "#121212"),
            ("graphite", false, "#FFFFFF"),
            ("violet", true, "#0E0B1D"),
            ("violet", false, "#FFFFFF"),
            ("bronze", true, "#1A120E"),
            ("bronze", false, "#FFFFFF"),
            ("teal", true, "#0B3D3B"),
            ("teal", false, "#FFFFFF"),
            ("mist", true, "#191D23"),
            ("mist", false, "#FFFFFF"),
            ("lavender", true, "#0D0B2D"),
            ("lavender", false, "#FFFFFF"),
            ("royal", true, "#0B1F3B"),
            ("royal", false, "#FFFFFF"),
            ("tangerine", true, "#1F1D2B"),
            ("tangerine", false, "#FFFFFF"),
            ("amethyst", true, "#5B21B6"),
            ("amethyst", false, "#FFFFFF"),
            ("rose", true, "#6D0D3F"),
            ("rose", false, "#FFFFFF"),
        ];
        assert_eq!(expect.len(), 32, "the pin table must cover all 32");
        for (id, dark, want) in expect {
            let p = palette_by_id(id).expect("palette exists");
            let t = derive(p, *dark);
            assert_eq!(
                token(&t, "--accent-text"),
                Some(*want),
                "{id} dark={dark} --accent-text drifted from its pre-A3-F5 pin"
            );
        }
    }

    #[test]
    fn text_on_raised_holds_aa_for_all_32() {
        // AUDIT 0.31 (A3-F3): the RENDERED pairing of --text
        // on --raised holds AA in every derivation (observed
        // minimum ~5.22:1, Mist Slate dark). This pins only
        // that rendered pairing — arbitrary token-on---raised
        // pairings are NOT all AA; see the derivation warning
        // at the top of this file.
        for p in PALETTES {
            for dark in [true, false] {
                let t = derive(p, dark);
                let text = Rgb::from_hex(token(&t, "--text").unwrap());
                let raised = Rgb::from_hex(token(&t, "--raised").unwrap());
                assert!(
                    text.contrast(raised) >= 4.5,
                    "{} dark={dark} --text {} on --raised {} is {:.2}:1 (< 4.5)",
                    p.id,
                    text.to_hex(),
                    raised.to_hex(),
                    text.contrast(raised)
                );
            }
        }
    }
}
