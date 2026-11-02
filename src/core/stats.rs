//! Savings statistics (0.30): what the Shield and Lite
//! prevented, shown on the start page's savings band — Brave's
//! New Tab trio (items blocked, estimated bandwidth saved,
//! estimated time saved) under Pico's honesty discipline.
//!
//! WHAT IS EXACT AND WHAT IS ESTIMATED — the distinction is
//! the feature:
//!
//! - **Requests blocked** are EXACT integers. A block exists
//!   only as a `filterpipe::decide` verdict, and the request
//!   handler records it exactly once, after the neutered
//!   response has actually been delivered (the v22-9 law: a
//!   failed neuter records nothing). Attribution travels with
//!   the verdict, so double-counting is impossible by the same
//!   construction that made it impossible in the pipeline.
//! - **Bytes and milliseconds saved** are CALIBRATED
//!   ESTIMATES, always displayed with `≈`. A blocked request's
//!   true size is unknowable without fetching it (fetching
//!   would defeat the block), so each block is credited the
//!   measured average for its resource type from the RESPONSE
//!   LEDGER below — Pico's own allowed traffic, observed on
//!   this machine, on this connection. Until a type has
//!   [`MIN_SAMPLES`] of the user's own samples, a documented
//!   conservative built-in default stands in (see
//!   [`DEFAULT_BYTES`] / [`DEFAULT_DURATION_MS`]).
//! - **In-page stripped payload** (Shield only) is measured
//!   by the runtime itself and kept conceptually separate:
//!   the scriptlet runtime reports the serialized size of
//!   the VALUES it removes from ad subtrees it prunes, over
//!   the validated `shield_pruned` IPC. Those bytes WERE
//!   downloaded (inside the JSON the prune cleaned), so
//!   they are displayed as their own sub-line and never
//!   added into the ≈ estimate. What the figure IS —
//!   stated precisely (AUDIT 0.30.1, correcting 0.30's
//!   "exact / byte for byte" wording, which overstated it):
//!   the sum of removed VALUE bytes, counted once per
//!   removal operation. It is not a wire byte count: keys
//!   and separators are excluded, so it understates the
//!   removed span (a fixture subtree whose removal cut 26
//!   wire bytes is credited 14); replace-family rules
//!   rewrite rather than remove and are not measured at
//!   all; and a fresh parse of the same body is a fresh
//!   set of removals, counted again — the layered hooks
//!   are idempotent over ONE object (a second pass finds
//!   nothing left to delete), but a re-parse creates new
//!   objects and new removals. An "event" is one removed
//!   subtree: a terminal wildcard that removes twelve
//!   children counts twelve.
//!
//! THE LEDGER learns from allowed responses only:
//! `WebResourceResponseReceived` reports each response's
//! `Content-Length` (parsed defensively; missing/invalid/zero
//! teaches nothing and is skipped — chunked responses
//! therefore never calibrate bytes) and its duration, timed
//! from the `WebResourceRequested` dispatch. Duration
//! correlation keys on the request object's identity token
//! supplied by the app layer; **if a response cannot be
//! correlated, its bytes still calibrate** (bucketed by the
//! response's own Content-Type) and only the duration sample
//! is skipped — the ledger never invents a duration. Samples
//! are sanity-capped ([`MAX_SAMPLE_BYTES`],
//! [`MAX_SAMPLE_DURATION_MS`]) and the correlation map is
//! capped and expiring ([`PendingTimings`]), so this layer
//! cannot leak memory or be poisoned by one absurd response.
//!
//! Document/Subdocument/WebSocket/Ping blocks are COUNTED but
//! credited 0 bytes / 0 ms: a document block is a navigation
//! refused (rare `$document` rules), not data waste, and
//! sockets/pings carry no response body to calibrate against.
//! Loopback is structurally absent: `decide` allows local
//! traffic before either engine runs, so no local block can
//! ever be recorded (pinned by the reconciliation test).
//!
//! Persistence: the lifetime half rides `PersistState`
//! (schema-v3 structural default, sanitized on load like
//! every other field). The in-memory book is authoritative;
//! the app mirrors it into state at its normal save points,
//! best-effort — a failed save never breaks counting
//! (the v29 C1 latch discipline).

use crate::core::filterpipe::OriginClass;
use crate::core::shield::ResourceType;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

/// Resource types in ledger order — one slot per
/// [`ResourceType`] variant. The mapping is a match, not a
/// cast, so a future variant reorder cannot silently
/// re-bucket the ledger.
pub const TYPE_COUNT: usize = 11;

pub fn type_index(rtype: ResourceType) -> usize {
    match rtype {
        ResourceType::Document => 0,
        ResourceType::Subdocument => 1,
        ResourceType::Script => 2,
        ResourceType::Image => 3,
        ResourceType::Stylesheet => 4,
        ResourceType::Xhr => 5,
        ResourceType::Media => 6,
        ResourceType::Font => 7,
        ResourceType::WebSocket => 8,
        ResourceType::Ping => 9,
        ResourceType::Other => 10,
    }
}

/// Types whose blocks are counted but never credited bytes or
/// time (see the module header for the reasoning).
fn credit_excluded(rtype: ResourceType) -> bool {
    matches!(
        rtype,
        ResourceType::Document
            | ResourceType::Subdocument
            | ResourceType::WebSocket
            | ResourceType::Ping
    )
}

/// Built-in per-type byte defaults, credited until the ledger
/// holds [`MIN_SAMPLES`] of the user's own byte samples for
/// the type. Conservative mid-points of published web-traffic
/// averages (HTTP Archive page-weight reports put the median
/// image near 50–100 KB, scripts near 30–90 KB, stylesheets
/// near 15–20 KB, webfonts near 40–100 KB, XHR payloads near
/// 10–30 KB; a single streamed media segment near 400 KB).
/// Pico deliberately credits the LOW end while uncalibrated:
/// the number must never flatter itself. Excluded types are 0.
pub const DEFAULT_BYTES: [u64; TYPE_COUNT] = [
    0,       // Document — excluded
    0,       // Subdocument — excluded
    30_000,  // Script
    50_000,  // Image
    15_000,  // Stylesheet
    20_000,  // Xhr
    400_000, // Media
    40_000,  // Font
    0,       // WebSocket — excluded
    0,       // Ping — excluded
    10_000,  // Other
];

/// Built-in per-type duration defaults (milliseconds),
/// credited until the ledger holds [`MIN_SAMPLES`] of the
/// user's own duration samples for the type. Conservative
/// slow-link figures: roughly one round trip plus transfer of
/// the default byte size on a ~300 kbps-class connection —
/// the link Pico's user actually browses on. Excluded types
/// are 0.
pub const DEFAULT_DURATION_MS: [u64; TYPE_COUNT] = [
    0,    // Document — excluded
    0,    // Subdocument — excluded
    600,  // Script
    800,  // Image
    400,  // Stylesheet
    500,  // Xhr
    2500, // Media
    700,  // Font
    0,    // WebSocket — excluded
    0,    // Ping — excluded
    400,  // Other
];

/// Samples of the user's own traffic a type needs before its
/// measured average replaces the built-in default.
pub const MIN_SAMPLES: u64 = 20;

/// One response's Content-Length above this is not a sample,
/// it is a parsing artifact or a hostile header (1 TiB).
pub const MAX_SAMPLE_BYTES: u64 = 1 << 40;
/// One response's duration above this is a hung request, not
/// a calibration point (10 minutes).
pub const MAX_SAMPLE_DURATION_MS: u64 = 600_000;

/// Largest `shield_pruned` report the IPC accepts (16 MiB of
/// pruned payload in one flush is beyond any real page).
pub const MAX_PRUNE_REPORT_BYTES: u64 = 16_777_216;
/// Largest prune-event count in one report.
pub const MAX_PRUNE_REPORT_EVENTS: u64 = 100_000;
/// Minimum spacing between accepted prune reports per tab:
/// the real runtime flushes every 2.5 s, so 1 s of headroom
/// admits every honest flush and starves a forged flood.
pub const PRUNE_MIN_INTERVAL_MS: u64 = 1_000;
/// Session ceiling on accepted prune bytes (1 TiB): a page
/// forging reports around the clock still cannot push the
/// lifetime figure past a bound the reset control cures.
pub const PRUNE_SESSION_BYTE_CAP: u64 = 1 << 40;

/// Session ceiling for accepted prune EVENTS (AUDIT 0.30,
/// A1-3: bytes had a session cap, events did not). Far past
/// honest reach (a thousand maximum-size reports), it exists
/// so a forged-report flood is bounded in both dimensions.
pub const PRUNE_SESSION_EVENT_CAP: u64 = 100_000_000;
/// Session log lines for accepted prune reports.
pub const PRUNE_LOG_SAMPLE: usize = 20;

/// One resource type's calibration: what Pico's own allowed
/// responses of this type actually weighed and how long they
/// took. Byte and duration samples are counted separately —
/// a chunked response teaches duration but not bytes, and a
/// correlation miss teaches bytes but not duration.
///
/// Every field carries a serde default (AUDIT 0.30.1): a
/// stats object missing any one field — a hand-trimmed or
/// partially-written state file — must load with that field
/// at zero, not fail the whole state parse and take every
/// other setting down with it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct TypeLedger {
    pub total_bytes: u64,
    pub byte_samples: u64,
    pub total_duration_ms: u64,
    pub duration_samples: u64,
}

impl TypeLedger {
    /// The measured average byte size, once calibrated.
    /// (MIN_SAMPLES ≥ 1, so the gate alone keeps the divisor
    /// non-zero.)
    pub fn avg_bytes(&self) -> Option<u64> {
        if self.byte_samples >= MIN_SAMPLES {
            Some(self.total_bytes / self.byte_samples)
        } else {
            None
        }
    }

    /// The measured average duration, once calibrated.
    pub fn avg_duration_ms(&self) -> Option<u64> {
        if self.duration_samples >= MIN_SAMPLES {
            Some(self.total_duration_ms / self.duration_samples)
        } else {
            None
        }
    }

    fn sanitized(mut self) -> Self {
        // A hand-edited ledger must not survive with figures
        // its own sample counts cannot support: totals are
        // capped at samples × the per-sample ceilings (and at
        // zero when there are no samples at all).
        //
        // AUDIT 0.30 (A5): sample counts are additionally
        // scaled down past a billion — a count no honest book
        // reaches — with the totals scaled PROPORTIONALLY
        // (u128), so the measured average survives the cap;
        // and total_bytes takes the same absolute 2^62 cap
        // the engine figures carry, so no crafted product of
        // samples × ceiling can saturate a persisted figure.
        const SAMPLES_CAP: u64 = 1 << 30;
        if self.byte_samples > SAMPLES_CAP {
            self.total_bytes = ((self.total_bytes as u128 * SAMPLES_CAP as u128)
                / self.byte_samples as u128) as u64;
            self.byte_samples = SAMPLES_CAP;
        }
        if self.duration_samples > SAMPLES_CAP {
            self.total_duration_ms = ((self.total_duration_ms as u128 * SAMPLES_CAP as u128)
                / self.duration_samples as u128) as u64;
            self.duration_samples = SAMPLES_CAP;
        }
        self.total_bytes = self
            .total_bytes
            .min(self.byte_samples.saturating_mul(MAX_SAMPLE_BYTES))
            .min(1 << 62);
        self.total_duration_ms = self
            .total_duration_ms
            .min(self.duration_samples.saturating_mul(MAX_SAMPLE_DURATION_MS));
        self
    }
}

/// One engine's accumulated figures. `blocked` is exact;
/// `est_bytes` / `est_duration_ms` are sums of per-block
/// credits (each an average or a documented default — see the
/// module header); `pruned_*` are the Shield's in-page removal
/// figures (always zero for Lite) — see the module header for
/// exactly what they count. Field-level serde defaults, like
/// [`TypeLedger`]'s (AUDIT 0.30.1).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct EngineStats {
    pub blocked: u64,
    pub est_bytes: u64,
    pub est_duration_ms: u64,
    pub pruned_events: u64,
    pub pruned_bytes: u64,
}

impl EngineStats {
    fn sanitized(mut self) -> Self {
        // Ceilings that keep a corrupt file from rendering
        // nonsense while never touching a plausible lifetime:
        // 2^62 of anything is ~146 years of heavy browsing.
        const CAP: u64 = 1 << 62;
        self.blocked = self.blocked.min(CAP);
        self.est_bytes = self.est_bytes.min(CAP);
        self.est_duration_ms = self.est_duration_ms.min(CAP);
        self.pruned_events = self.pruned_events.min(CAP);
        self.pruned_bytes = self.pruned_bytes.min(CAP);
        self
    }
}

/// The persisted lifetime savings: both engines' figures plus
/// the calibration ledger they were credited from. Rides
/// `PersistState` under a serde default, so every pre-0.30
/// state file loads with a zero book (the app then seeds the
/// Shield's blocked count from the legacy `blocked_total`,
/// once — see `App` startup and AUDIT 0.30).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Savings {
    pub shield: EngineStats,
    pub lite: EngineStats,
    pub ledger: [TypeLedger; TYPE_COUNT],
}

// ---------------------------------------------------------------------------
// Tolerant deserialization (AUDIT 0.31, A5-M1)
// ---------------------------------------------------------------------------
//
// Through 0.30.1 these three types derived Deserialize over
// plain u64 fields and a fixed `[TypeLedger; 11]` array, so
// ONE malformed value in the stats subtree — a hand-edited
// `"blocked": -1`, a float, a string, a ledger array of the
// wrong length, a literal `null` — failed the WHOLE
// PersistState parse and took every other setting down with
// it. The impls below deserialize through serde_json::Value
// and sanitize per field: null / non-object → default;
// a scalar that is not a non-negative integer (negative,
// float, string, bool) → that field's default (0); a ledger
// array of any length → truncated or zero-padded to exactly
// TYPE_COUNT. Serialization is the unchanged derived shape
// (same field names, same layout), so every existing state
// file loads byte-identically under the new reader.

fn tolerant_u64(v: &serde_json::Value) -> u64 {
    // Value::as_u64 is Some ONLY for a non-negative integer
    // that fits u64 — exactly the tolerant rule.
    v.as_u64().unwrap_or(0)
}

fn type_ledger_from_value(v: &serde_json::Value) -> TypeLedger {
    let Some(obj) = v.as_object() else {
        return TypeLedger::default();
    };
    let get = |k: &str| obj.get(k).map(tolerant_u64).unwrap_or(0);
    TypeLedger {
        total_bytes: get("total_bytes"),
        byte_samples: get("byte_samples"),
        total_duration_ms: get("total_duration_ms"),
        duration_samples: get("duration_samples"),
    }
}

fn engine_stats_from_value(v: &serde_json::Value) -> EngineStats {
    let Some(obj) = v.as_object() else {
        return EngineStats::default();
    };
    let get = |k: &str| obj.get(k).map(tolerant_u64).unwrap_or(0);
    EngineStats {
        blocked: get("blocked"),
        est_bytes: get("est_bytes"),
        est_duration_ms: get("est_duration_ms"),
        pruned_events: get("pruned_events"),
        pruned_bytes: get("pruned_bytes"),
    }
}

fn savings_from_value(v: &serde_json::Value) -> Savings {
    let Some(obj) = v.as_object() else {
        return Savings::default();
    };
    let mut out = Savings::default();
    if let Some(shield) = obj.get("shield") {
        out.shield = engine_stats_from_value(shield);
    }
    if let Some(lite) = obj.get("lite") {
        out.lite = engine_stats_from_value(lite);
    }
    if let Some(arr) = obj.get("ledger").and_then(|l| l.as_array()) {
        for (i, entry) in arr.iter().take(TYPE_COUNT).enumerate() {
            out.ledger[i] = type_ledger_from_value(entry);
        }
    }
    out
}

impl<'de> Deserialize<'de> for TypeLedger {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let v = serde_json::Value::deserialize(deserializer)?;
        Ok(type_ledger_from_value(&v))
    }
}

impl<'de> Deserialize<'de> for EngineStats {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let v = serde_json::Value::deserialize(deserializer)?;
        Ok(engine_stats_from_value(&v))
    }
}

impl<'de> Deserialize<'de> for Savings {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let v = serde_json::Value::deserialize(deserializer)?;
        Ok(savings_from_value(&v))
    }
}

impl Savings {
    pub fn sanitized(mut self) -> Self {
        self.shield = self.shield.sanitized();
        self.lite = self.lite.sanitized();
        for slot in &mut self.ledger {
            *slot = slot.sanitized();
        }
        self
    }

    pub fn is_zero(&self) -> bool {
        self.shield == EngineStats::default()
            && self.lite == EngineStats::default()
            && self.ledger.iter().all(|l| *l == TypeLedger::default())
    }
}

/// Which engine a verdict belongs to. Attribution is decided
/// by `filterpipe::decide` and nowhere else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    Shield,
    Lite,
}

/// The bytes one blocked request of this type is credited:
/// the ledger's measured average once calibrated, else the
/// documented default; 0 for the excluded types.
pub fn credit_bytes(ledger: &[TypeLedger; TYPE_COUNT], rtype: ResourceType) -> u64 {
    if credit_excluded(rtype) {
        return 0;
    }
    let slot = &ledger[type_index(rtype)];
    slot.avg_bytes().unwrap_or(DEFAULT_BYTES[type_index(rtype)])
}

/// The milliseconds one blocked request of this type is
/// credited — same discipline as [`credit_bytes`].
pub fn credit_duration_ms(ledger: &[TypeLedger; TYPE_COUNT], rtype: ResourceType) -> u64 {
    if credit_excluded(rtype) {
        return 0;
    }
    let slot = &ledger[type_index(rtype)];
    slot.avg_duration_ms()
        .unwrap_or(DEFAULT_DURATION_MS[type_index(rtype)])
}

/// Parse a `Content-Length` header value defensively: ASCII
/// digits only (after trimming), no sign, no comma-joined
/// duplicates, no zero (a zero-length response teaches
/// nothing about blocked payloads), nothing above the
/// per-sample ceiling. Anything else is `None` — skip.
pub fn parse_content_length(raw: Option<&str>) -> Option<u64> {
    let raw = raw?.trim();
    if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: u64 = raw.parse().ok()?;
    if n == 0 || n > MAX_SAMPLE_BYTES {
        return None;
    }
    Some(n)
}

/// Bucket an allowed response by its Content-Type, for the
/// byte calibration of responses whose duration correlation
/// missed (and as the ledger's vocabulary check). The
/// response's own declared type is the ground truth of what
/// its bytes were. `None` when the header is missing or
/// unusable — such responses teach nothing.
pub fn classify_response_bucket(content_type: Option<&str>) -> Option<ResourceType> {
    let ct = content_type?.split(';').next()?.trim().to_ascii_lowercase();
    if ct.is_empty() || !ct.contains('/') || !ct.is_ascii() {
        return None;
    }
    let (main, sub) = ct.split_once('/')?;
    // AUDIT 0.30 (A2b): malformed-but-slashy values teach
    // nothing either — an empty side, embedded whitespace, a
    // comma-joined duplicate, or a second slash means the
    // header was not a usable media type.
    if main.is_empty()
        || sub.is_empty()
        || main.chars().any(|c| c.is_whitespace() || c == ',')
        || sub
            .chars()
            .any(|c| c.is_whitespace() || c == ',' || c == '/')
    {
        return None;
    }
    let bucket = match (main, sub) {
        ("image", _) => ResourceType::Image,
        ("font", _) => ResourceType::Font,
        ("video", _) | ("audio", _) => ResourceType::Media,
        ("text", "css") => ResourceType::Stylesheet,
        ("text", "html") => ResourceType::Document,
        ("text", s) if s.contains("javascript") || s.contains("ecmascript") => ResourceType::Script,
        ("application", "vnd.ms-fontobject")
        | ("application", "font-woff")
        | ("application", "font-woff2")
        | ("application", "x-font-woff")
        | ("application", "x-font-woff2")
        | ("application", "x-font-ttf") => ResourceType::Font,
        ("application", s) if s.contains("javascript") || s.contains("ecmascript") => {
            ResourceType::Script
        }
        ("application", s) if s == "json" || s.ends_with("+json") => ResourceType::Xhr,
        ("application", "octet-stream") => ResourceType::Other,
        ("text", _) => ResourceType::Xhr,
        _ => ResourceType::Other,
    };
    Some(bucket)
}

// ---------------------------------------------------------------------------
// Humanizers (ONE pure implementation; the start page's JS
// mirrors these exactly and shared fixtures pin both)
// ---------------------------------------------------------------------------

/// Format a byte count with 2–3 significant figures over
/// decimal SI units (the bandwidth convention): `0 B`,
/// `999 B`, `1 KB`, `1.5 KB`, `86.4 MB`, `1.25 GB`.
pub fn humanize_bytes(n: u64) -> String {
    if n < 1000 {
        return format!("{n} B");
    }
    // AUDIT 0.31 (A5-L13): EB added — through 0.30.1 the unit
    // table stopped at PB, so a figure at the 2^62 sanitize
    // cap printed as "4611 PB" instead of ~4.61 EB.
    const UNITS: [&str; 6] = ["KB", "MB", "GB", "TB", "PB", "EB"];
    let mut value = n as f64;
    let mut unit = 0usize;
    while value >= 1000.0 && unit < UNITS.len() {
        value /= 1000.0;
        unit += 1;
    }
    // Rounding at 3 sig figs can push 999.95 over the unit
    // boundary ("1000 MB"); promote instead ("1 GB").
    if value >= 999.95 && unit < UNITS.len() {
        value /= 1000.0;
        unit += 1;
    }
    // `unit` counts the divisions performed; the value is
    // already scaled, its unit is UNITS[unit - 1].
    format!("{} {}", sig_figs(value), UNITS[unit - 1])
}

/// Format a duration with 2–3 significant figures, promoting
/// seconds → minutes → hours → days: `0 s`, `45 s`, `1.5 min`,
/// `1 h`, `2.5 days`.
pub fn humanize_duration(ms: u64) -> String {
    let seconds = ms as f64 / 1000.0;
    if seconds < 60.0 {
        return format!("{} s", sig_figs(seconds));
    }
    let minutes = seconds / 60.0;
    if minutes < 60.0 {
        return format!("{} min", sig_figs(minutes));
    }
    let hours = minutes / 60.0;
    if hours < 24.0 {
        return format!("{} h", sig_figs(hours));
    }
    let days = sig_figs(hours / 24.0);
    if days == "1" {
        "1 day".to_string()
    } else {
        format!("{days} days")
    }
}

/// 2–3 significant figures, trailing zeros trimmed:
/// 1 → "1", 1.5 → "1.5", 1.25 → "1.25", 86.4 → "86.4",
/// 850 → "850", 999.9 → "1,000"-class values round UP to the
/// next figure rather than printing four digits — callers
/// promote units before this matters.
fn sig_figs(v: f64) -> String {
    // Rounding is HALF-UP at the kept digit in every branch
    // (AUDIT 0.30, A2b): the JS mirror formats with
    // `toFixed`, whose ties go up, while Rust's `{:.1}` /
    // `{:.2}` formatting rounds exact ties half-to-EVEN —
    // the two diverged at values like 12.25 ("12.2 KB" here
    // vs "12.3 KB" on the page). The decimal branches
    // therefore quantize explicitly (floor(x·10ᵏ + 0.5) /
    // 10ᵏ, exact at these magnitudes) and only then format;
    // the integer branch's `round()` was already half-up.
    // Shared fixtures, ties included, pin both sides.
    if v >= 99.95 {
        format!("{}", v.round() as u64)
    } else if v >= 9.995 {
        trim_num(format!("{:.1}", (v * 10.0 + 0.5).floor() / 10.0))
    } else {
        trim_num(format!("{:.2}", (v * 100.0 + 0.5).floor() / 100.0))
    }
}

fn trim_num(s: String) -> String {
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    }
}

// ---------------------------------------------------------------------------
// Pending-request timing correlation (capped, expiring)
// ---------------------------------------------------------------------------

/// Correlation entries held at most.
pub const PENDING_CAP: usize = 2048;
/// A request with no response after this long is forgotten
/// (3 minutes): its duration would be an outlier anyway, and
/// the slot is worth more than the sample.
pub const PENDING_TTL_MS: u64 = 180_000;

/// In-flight allowed requests awaiting their response event,
/// keyed by the app layer's request-identity token. Pure and
/// bounded: insertion sweeps expired entries and evicts the
/// oldest beyond [`PENDING_CAP`], so a page that opens
/// requests and never reads responses cannot grow this map.
#[derive(Debug, Default)]
pub struct PendingTimings {
    // (noted-at ms, request type, is_head) — AUDIT 0.31
    // (A2-F9): a HEAD entry's response must not teach the
    // ledger (its Content-Length describes the body a GET
    // would have returned, not bytes actually transferred),
    // so the flag rides the entry to `note_response`.
    map: HashMap<u64, (u64, ResourceType, bool)>,
}

impl PendingTimings {
    pub fn new() -> Self {
        PendingTimings {
            map: HashMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Note a dispatched allowed request. `now_ms` is the
    /// app's monotonic clock (one epoch for the process).
    pub fn note(&mut self, token: u64, rtype: ResourceType, now_ms: u64) {
        self.note_head(token, rtype, now_ms, false);
    }

    /// [`note`](Self::note) with the request's HEAD flag
    /// (AUDIT 0.31, A2-F9).
    pub fn note_head(&mut self, token: u64, rtype: ResourceType, now_ms: u64, is_head: bool) {
        // AUDIT 0.30 (A2b): re-noting a token already present
        // only overwrites it — running the cap eviction for an
        // insert that cannot grow the map threw away an
        // unrelated live entry for nothing.
        if let Some(entry) = self.map.get_mut(&token) {
            *entry = (now_ms, rtype, is_head);
            return;
        }
        if self.map.len() >= PENDING_CAP {
            self.map
                .retain(|_, (t0, _, _)| now_ms.saturating_sub(*t0) <= PENDING_TTL_MS);
            while self.map.len() >= PENDING_CAP {
                // Evict the single oldest entry per pass; the
                // map is capped, so this scan is bounded too.
                let oldest = self
                    .map
                    .iter()
                    .min_by_key(|(_, (t0, _, _))| *t0)
                    .map(|(k, _)| *k);
                match oldest {
                    Some(k) => {
                        self.map.remove(&k);
                    }
                    None => break,
                }
            }
        }
        self.map.insert(token, (now_ms, rtype, is_head));
    }

    /// Take a completed request's entry: its type, its
    /// duration when the correlation is still fresh, and its
    /// HEAD flag (AUDIT 0.31, A2-F9). The
    /// type is returned even for a STALE entry (the response
    /// is known to belong to that class, so its bytes still
    /// calibrate the right slot) — but a duration older than
    /// [`PENDING_TTL_MS`] is forgotten, per the header law:
    /// slow outliers are exactly what the TTL exists to keep
    /// out of the averages (AUDIT 0.30, A2b — the first cut
    /// enforced only the 600 s sample ceiling here, so a
    /// 181–600 s response silently counted). `None` only for
    /// a true miss (never noted, or evicted).
    pub fn take(&mut self, token: u64, now_ms: u64) -> Option<(ResourceType, Option<u64>, bool)> {
        let (t0, rtype, is_head) = self.map.remove(&token)?;
        let duration = now_ms.saturating_sub(t0);
        if duration > PENDING_TTL_MS {
            return Some((rtype, None, is_head));
        }
        Some((rtype, Some(duration), is_head))
    }

    pub fn clear(&mut self) {
        self.map.clear();
    }
}

// ---------------------------------------------------------------------------
// The book (thread-safe runtime state)
// ---------------------------------------------------------------------------

struct BookInner {
    lifetime: Savings,
    session_shield: EngineStats,
    session_lite: EngineStats,
    pending: PendingTimings,
    /// Per-tab prune-report gate: last accepted report time.
    prune_last: HashMap<u64, u64>,
    prune_session_bytes: u64,
    prune_session_events: u64,
    prune_log_left: usize,
}

impl BookInner {
    fn engine_mut(&mut self, engine: Engine, session: bool) -> &mut EngineStats {
        match (engine, session) {
            (Engine::Shield, false) => &mut self.lifetime.shield,
            (Engine::Lite, false) => &mut self.lifetime.lite,
            (Engine::Shield, true) => &mut self.session_shield,
            (Engine::Lite, true) => &mut self.session_lite,
        }
    }
}

/// The savings book shared between the request handlers, the
/// response observer and the app. In-memory authoritative;
/// the app mirrors [`StatsBook::lifetime`] into persisted
/// state at its normal save points.
pub struct StatsBook {
    inner: Mutex<BookInner>,
}

impl Default for StatsBook {
    fn default() -> Self {
        Self::new()
    }
}

impl StatsBook {
    pub fn new() -> Self {
        StatsBook {
            inner: Mutex::new(BookInner {
                lifetime: Savings::default(),
                session_shield: EngineStats::default(),
                session_lite: EngineStats::default(),
                pending: PendingTimings::new(),
                prune_last: HashMap::new(),
                prune_session_bytes: 0,
                prune_session_events: 0,
                prune_log_left: PRUNE_LOG_SAMPLE,
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BookInner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Load the persisted lifetime at startup (already
    /// sanitized by the state layer). Session starts at zero
    /// by definition.
    pub fn load(&self, lifetime: Savings) {
        self.lock().lifetime = lifetime;
    }

    /// One-time migration seed (0.30): a pre-0.30 state file
    /// has a legacy `blocked_total` but no savings book; the
    /// Shield's lifetime count carries over so the tile does
    /// not pretend the user's history never happened. Bytes
    /// and time honestly start at zero — they were never
    /// measured.
    /// AUDIT 0.31 (A5-L4): returns true iff the seed was
    /// actually applied — through 0.30.1 the caller could not
    /// tell a seeded migration from a no-op (non-empty book /
    /// zero legacy total), so its log line claimed a carry-over
    /// that may never have happened.
    pub fn seed_shield_blocked(&self, legacy_total: u64) -> bool {
        let mut inner = self.lock();
        // AUDIT 0.30 (A2b): the seed fires only into a
        // COMPLETELY empty book (the genuine pre-0.30 shape —
        // any 0.30-written book is authoritative as it stands)
        // and takes the engine figures' sanitize cap itself.
        if inner.lifetime.is_zero() && legacy_total > 0 {
            inner.lifetime.shield.blocked = legacy_total.min(1 << 62);
            return true;
        }
        false
    }

    /// Record ONE delivered block (called by the request
    /// handler only after the neutered response was set — the
    /// v22-9 law). The credit is computed from the ledger as
    /// it stands at this instant.
    ///
    /// AUDIT 0.30.1 (A2-C10): the request's method matters
    /// for exactly one case — a blocked HEAD request. It
    /// counts (the block is exact, like every block), but it
    /// earns ZERO byte and time credit: a HEAD response
    /// carries no body by definition, so everything the
    /// block prevented was a headers-only exchange, and the
    /// per-type averages describe body transfers — crediting
    /// one would fabricate savings. (Crediting a measured
    /// headers figure instead was considered; Pico does not
    /// observe blocked responses' headers, so any figure
    /// would be invented. Zero is the honest number.) An
    /// unknown method (`None` — the handler could not read
    /// it) credits normally, as before: methods fail open.
    pub fn record_block(&self, engine: Engine, rtype: ResourceType, method: Option<&str>) {
        let mut inner = self.lock();
        let (bytes, duration) = if method.is_some_and(|m| m.eq_ignore_ascii_case("HEAD")) {
            (0, 0)
        } else {
            (
                credit_bytes(&inner.lifetime.ledger, rtype),
                credit_duration_ms(&inner.lifetime.ledger, rtype),
            )
        };
        for session in [false, true] {
            let e = inner.engine_mut(engine, session);
            e.blocked = e.blocked.saturating_add(1);
            e.est_bytes = e.est_bytes.saturating_add(bytes);
            e.est_duration_ms = e.est_duration_ms.saturating_add(duration);
        }
    }

    /// Note a dispatched allowed request for duration
    /// correlation (app layer, request handler).
    pub fn note_request(&self, token: u64, rtype: ResourceType, now_ms: u64) {
        self.lock().pending.note(token, rtype, now_ms);
    }

    /// [`note_request`](Self::note_request) with the
    /// request's method (AUDIT 0.31, A2-F9): a HEAD request's
    /// response carries a Content-Length for a body that was
    /// never transferred, so its entry must not teach the
    /// ledger bytes or duration — `note_response` consumes
    /// the entry and records nothing for it. NEW SIGNATURE
    /// for the app wiring: `note_request_with_method(token,
    /// rtype, now_ms, method)`; the old 3-arg `note_request`
    /// stays as the non-HEAD wrapper.
    pub fn note_request_with_method(
        &self,
        token: u64,
        rtype: ResourceType,
        now_ms: u64,
        method: Option<&str>,
    ) {
        let is_head = method.is_some_and(|m| m.eq_ignore_ascii_case("HEAD"));
        self.lock().pending.note_head(token, rtype, now_ms, is_head);
    }

    /// Record one allowed response. With a correlation hit,
    /// the sample lands in the REQUEST's type slot (its true
    /// class) with duration, plus bytes when the response
    /// carried a valid Content-Length. A HEAD entry (AUDIT
    /// 0.31, A2-F9) is consumed and teaches nothing. A stale hit (past the
    /// correlation TTL) still lands its bytes in the request's
    /// slot but contributes no duration. On a true miss, bytes
    /// alone calibrate — bucketed by the response's
    /// Content-Type — and no duration is recorded.
    pub fn note_response(
        &self,
        token: u64,
        content_length: Option<u64>,
        content_type: Option<&str>,
        now_ms: u64,
    ) {
        // AUDIT 0.31 (A2-F9): validate the byte sample HERE,
        // like record_sample and parse_content_length already
        // did at their own seams — through 0.30.1 a caller
        // handing note_response an unvalidated Content-Length
        // (0, or above MAX_SAMPLE_BYTES) taught the ledger
        // from it verbatim on both the correlated and the
        // bucket fallback paths. Out-of-range is None.
        let content_length = match content_length {
            Some(b) if b > 0 && b <= MAX_SAMPLE_BYTES => Some(b),
            _ => None,
        };
        let mut inner = self.lock();
        if let Some((rtype, duration, is_head)) = inner.pending.take(token, now_ms) {
            // AUDIT 0.31 (A2-F9): a HEAD response teaches
            // NOTHING — its Content-Length names the body a
            // GET would have returned and its duration is a
            // headers-only exchange. The correlation entry
            // above is still consumed (take removed it); the
            // ledger is simply not fed.
            if is_head {
                return;
            }
            let slot = &mut inner.lifetime.ledger[type_index(rtype)];
            // A zero-millisecond "duration" teaches nothing
            // (a same-tick cache hit is not evidence a blocked
            // copy would have been free) — the same doctrine
            // as zero bytes (AUDIT 0.30, A2b).
            if let Some(d) = duration {
                if d > 0 && d <= MAX_SAMPLE_DURATION_MS {
                    slot.total_duration_ms = slot.total_duration_ms.saturating_add(d);
                    slot.duration_samples = slot.duration_samples.saturating_add(1);
                }
            }
            if let Some(bytes) = content_length {
                slot.total_bytes = slot.total_bytes.saturating_add(bytes);
                slot.byte_samples = slot.byte_samples.saturating_add(1);
            }
            return;
        }
        if let (Some(bytes), Some(bucket)) =
            (content_length, classify_response_bucket(content_type))
        {
            let slot = &mut inner.lifetime.ledger[type_index(bucket)];
            slot.total_bytes = slot.total_bytes.saturating_add(bytes);
            slot.byte_samples = slot.byte_samples.saturating_add(1);
        }
    }

    /// Record a response sample directly (the seam the tests
    /// drive; the observer path above is this plus
    /// correlation).
    pub fn record_sample(&self, rtype: ResourceType, bytes: Option<u64>, duration_ms: Option<u64>) {
        let mut inner = self.lock();
        let slot = &mut inner.lifetime.ledger[type_index(rtype)];
        if let Some(b) = bytes {
            if b > 0 && b <= MAX_SAMPLE_BYTES {
                slot.total_bytes = slot.total_bytes.saturating_add(b);
                slot.byte_samples = slot.byte_samples.saturating_add(1);
            }
        }
        if let Some(d) = duration_ms {
            if d <= MAX_SAMPLE_DURATION_MS {
                slot.total_duration_ms = slot.total_duration_ms.saturating_add(d);
                slot.duration_samples = slot.duration_samples.saturating_add(1);
            }
        }
    }

    /// Accept (or refuse) one `shield_pruned` report. The
    /// gates, in order: payload bounds (re-checked here,
    /// defense in depth behind the IPC parser), the 0/0
    /// noise refusal, the per-tab rate floor, and the
    /// session ceilings (bytes and events). Returns true
    /// when the report was recorded.
    pub fn record_prune(&self, tab: u64, bytes: u64, events: u64, now_ms: u64) -> bool {
        if bytes > MAX_PRUNE_REPORT_BYTES || events > MAX_PRUNE_REPORT_EVENTS {
            return false;
        }
        // AUDIT 0.30 (A1-3): a 0/0 report is never the honest
        // runtime's (it posts only when something accumulated)
        // — refuse it BEFORE it can consume the tab's rate
        // slot or a log sample. (Bytes 0 with events > 0 is
        // legitimate: zero-size subtrees exist.)
        if bytes == 0 && events == 0 {
            return false;
        }
        let mut inner = self.lock();
        if let Some(last) = inner.prune_last.get(&tab) {
            if now_ms.saturating_sub(*last) < PRUNE_MIN_INTERVAL_MS {
                return false;
            }
        }
        if inner.prune_session_bytes.saturating_add(bytes) > PRUNE_SESSION_BYTE_CAP {
            return false;
        }
        if inner.prune_session_events.saturating_add(events) > PRUNE_SESSION_EVENT_CAP {
            return false;
        }
        inner.prune_last.insert(tab, now_ms);
        inner.prune_session_bytes = inner.prune_session_bytes.saturating_add(bytes);
        inner.prune_session_events = inner.prune_session_events.saturating_add(events);
        for session in [false, true] {
            let e = inner.engine_mut(Engine::Shield, session);
            e.pruned_events = e.pruned_events.saturating_add(events);
            e.pruned_bytes = e.pruned_bytes.saturating_add(bytes);
        }
        true
    }

    /// Drop a closed tab's prune gate (AUDIT 0.30, A2b: the
    /// map otherwise keeps one entry per tab for the whole
    /// process). Tab ids are never recycled, so this is pure
    /// hygiene — a forgotten tab can never report again.
    pub fn forget_tab(&self, tab: u64) {
        self.lock().prune_last.remove(&tab);
    }

    /// Consume one prune log-sample slot (session-capped;
    /// a stats reset replenishes it — see `reset`).
    pub fn claim_prune_log(&self) -> bool {
        let mut inner = self.lock();
        if inner.prune_log_left > 0 {
            inner.prune_log_left -= 1;
            true
        } else {
            false
        }
    }

    /// The persisted lifetime, for the app's save points.
    pub fn lifetime(&self) -> Savings {
        self.lock().lifetime
    }

    /// The render view for the start page.
    pub fn view(&self) -> StatsView {
        let inner = self.lock();
        StatsView {
            shield: EngineView::of(&inner.lifetime.shield, &inner.session_shield),
            lite: EngineView::of(&inner.lifetime.lite, &inner.session_lite),
        }
    }

    /// A cheap change signature for the view (the app's
    /// push gate): every figure the band renders, hashed.
    pub fn view_signature(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let v = self.view();
        let mut h = std::collections::hash_map::DefaultHasher::new();
        v.shield.blocked.hash(&mut h);
        v.shield.est_bytes.hash(&mut h);
        v.shield.est_duration_ms.hash(&mut h);
        v.shield.pruned_events.hash(&mut h);
        v.shield.pruned_bytes.hash(&mut h);
        v.shield.session_blocked.hash(&mut h);
        v.shield.session_est_bytes.hash(&mut h);
        v.shield.session_est_duration_ms.hash(&mut h);
        v.lite.blocked.hash(&mut h);
        v.lite.est_bytes.hash(&mut h);
        v.lite.est_duration_ms.hash(&mut h);
        v.lite.pruned_events.hash(&mut h);
        v.lite.pruned_bytes.hash(&mut h);
        v.lite.session_blocked.hash(&mut h);
        v.lite.session_est_bytes.hash(&mut h);
        v.lite.session_est_duration_ms.hash(&mut h);
        h.finish()
    }

    /// Zero everything (the start page's Reset stats control,
    /// via the gated dispatch): lifetime, session, the
    /// calibration ledger, pending correlation and the prune
    /// gates. The next save point persists the zero book.
    pub fn reset(&self) {
        let mut inner = self.lock();
        inner.lifetime = Savings::default();
        inner.session_shield = EngineStats::default();
        inner.session_lite = EngineStats::default();
        inner.pending.clear();
        inner.prune_last.clear();
        inner.prune_session_bytes = 0;
        inner.prune_session_events = 0;
        inner.prune_log_left = PRUNE_LOG_SAMPLE;
    }
}

/// One engine's figures as the start page renders them:
/// lifetime (the big numbers) plus the session's blocked
/// count and estimated bytes (the small line).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineView {
    pub blocked: u64,
    pub est_bytes: u64,
    pub est_duration_ms: u64,
    pub pruned_events: u64,
    pub pruned_bytes: u64,
    pub session_blocked: u64,
    pub session_est_bytes: u64,
    pub session_est_duration_ms: u64,
}

impl EngineView {
    fn of(lifetime: &EngineStats, session: &EngineStats) -> Self {
        EngineView {
            blocked: lifetime.blocked,
            est_bytes: lifetime.est_bytes,
            est_duration_ms: lifetime.est_duration_ms,
            pruned_events: lifetime.pruned_events,
            pruned_bytes: lifetime.pruned_bytes,
            session_blocked: session.blocked,
            session_est_bytes: session.est_bytes,
            session_est_duration_ms: session.est_duration_ms,
        }
    }
}

/// The whole band's render data. Time saved is presented as
/// one combined line by the page (Shield + Lite credited
/// durations), so the view carries both engines separately
/// and the page sums — keeping the per-engine truth in the
/// data rather than baking a merged figure into it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsView {
    pub shield: EngineView,
    pub lite: EngineView,
}

impl StatsView {
    pub fn time_saved_ms(&self) -> u64 {
        self.shield
            .est_duration_ms
            .saturating_add(self.lite.est_duration_ms)
    }

    pub fn session_time_saved_ms(&self) -> u64 {
        self.shield
            .session_est_duration_ms
            .saturating_add(self.lite.session_est_duration_ms)
    }

    pub fn any_activity(&self) -> bool {
        self.shield.blocked > 0 || self.lite.blocked > 0 || self.shield.pruned_events > 0
    }
}

/// Is this origin one the ledger may learn from / credit
/// against? Only the public web: loopback traffic is the
/// user's own dev servers (unminified bundles would poison
/// every average) and internal traffic is Pico itself. The
/// pipeline already guarantees blocks only happen on public
/// pages; this is the sampling side of the same law.
pub fn is_samplable(class: OriginClass) -> bool {
    class == OriginClass::Public
}

#[cfg(test)]
mod tests {
    use super::*;
    use ResourceType::*;

    #[test]
    fn type_index_covers_every_variant_once() {
        let all = [
            Document,
            Subdocument,
            Script,
            Image,
            Stylesheet,
            Xhr,
            Media,
            Font,
            WebSocket,
            Ping,
            Other,
        ];
        let mut seen = [false; TYPE_COUNT];
        for rt in all {
            let i = type_index(rt);
            assert!(i < TYPE_COUNT);
            assert!(!seen[i], "index collision at {i}");
            seen[i] = true;
        }
        assert!(seen.iter().all(|s| *s), "every slot claimed");
    }

    #[test]
    fn defaults_are_conservative_and_exclusions_are_zero() {
        let empty = [TypeLedger::default(); TYPE_COUNT];
        // Excluded types never earn bytes or time.
        for rt in [Document, Subdocument, WebSocket, Ping] {
            assert_eq!(credit_bytes(&empty, rt), 0);
            assert_eq!(credit_duration_ms(&empty, rt), 0);
        }
        // Uncalibrated credits are exactly the documented
        // defaults.
        assert_eq!(credit_bytes(&empty, Image), 50_000);
        assert_eq!(credit_bytes(&empty, Media), 400_000);
        assert_eq!(credit_duration_ms(&empty, Image), 800);
        assert_eq!(credit_duration_ms(&empty, Media), 2500);
    }

    #[test]
    fn calibration_flips_at_min_samples() {
        let book = StatsBook::new();
        // 19 samples: still the default.
        for _ in 0..(MIN_SAMPLES - 1) {
            book.record_sample(Image, Some(100_000), Some(2_000));
        }
        let lt = book.lifetime();
        assert_eq!(credit_bytes(&lt.ledger, Image), 50_000);
        assert_eq!(credit_duration_ms(&lt.ledger, Image), 800);
        // The 20th sample flips both averages to the measured
        // ones (uniform samples → the average is the sample).
        book.record_sample(Image, Some(100_000), Some(2_000));
        let lt = book.lifetime();
        assert_eq!(credit_bytes(&lt.ledger, Image), 100_000);
        assert_eq!(credit_duration_ms(&lt.ledger, Image), 2_000);
    }

    #[test]
    fn blocks_credit_the_ledger_at_the_moment_of_the_block() {
        let book = StatsBook::new();
        book.record_block(Engine::Shield, Image, None);
        let v = book.view();
        assert_eq!(v.shield.blocked, 1);
        assert_eq!(v.shield.est_bytes, 50_000, "default credit pre-calibration");
        // Calibrate, then block again: only the second block
        // earns the measured average (no retroactive rewrite).
        for _ in 0..MIN_SAMPLES {
            book.record_sample(Image, Some(200_000), Some(4_000));
        }
        book.record_block(Engine::Shield, Image, None);
        let v = book.view();
        assert_eq!(v.shield.blocked, 2);
        assert_eq!(v.shield.est_bytes, 50_000 + 200_000);
        assert_eq!(v.shield.est_duration_ms, 800 + 4_000);
        // Session mirrors lifetime within the first session.
        assert_eq!(v.shield.session_blocked, 2);
        // Lite is a separate set of books.
        assert_eq!(v.lite.blocked, 0);
    }

    #[test]
    fn head_blocks_count_but_earn_no_credit() {
        // AUDIT 0.30.1 (A2-C10): a HEAD response has no body
        // by definition; through 0.30 a blocked HEAD request
        // was credited the full per-type body average.
        let book = StatsBook::new();
        book.record_block(Engine::Shield, Image, Some("HEAD"));
        let v = book.view();
        assert_eq!(v.shield.blocked, 1, "the block itself is exact");
        assert_eq!(v.shield.est_bytes, 0, "no body, no byte credit");
        assert_eq!(v.shield.est_duration_ms, 0, "no body, no time credit");
        assert_eq!(v.shield.session_blocked, 1);
        // The method match is case-insensitive; other methods
        // and an unknown method credit normally.
        book.record_block(Engine::Shield, Image, Some("head"));
        book.record_block(Engine::Shield, Image, Some("GET"));
        book.record_block(Engine::Shield, Image, None);
        let v = book.view();
        assert_eq!(v.shield.blocked, 4);
        assert_eq!(v.shield.est_bytes, 2 * 50_000);
        assert_eq!(v.shield.est_duration_ms, 2 * 800);
    }

    #[test]
    fn content_length_parsing_is_defensive() {
        assert_eq!(parse_content_length(Some("12345")), Some(12345));
        assert_eq!(parse_content_length(Some("  42 ")), Some(42));
        assert_eq!(
            parse_content_length(Some("0")),
            None,
            "zero teaches nothing"
        );
        assert_eq!(parse_content_length(Some("")), None);
        assert_eq!(
            parse_content_length(Some("12, 12")),
            None,
            "duplicates refused"
        );
        assert_eq!(parse_content_length(Some("-5")), None);
        assert_eq!(parse_content_length(Some("+5")), None);
        assert_eq!(parse_content_length(Some("5.5")), None);
        assert_eq!(
            parse_content_length(Some("99999999999999999999")),
            None,
            "overflow"
        );
        assert_eq!(
            parse_content_length(Some(&(MAX_SAMPLE_BYTES + 1).to_string())),
            None,
            "above the sample ceiling"
        );
        assert_eq!(parse_content_length(None), None);
    }

    #[test]
    fn response_bucket_classification() {
        assert_eq!(classify_response_bucket(Some("image/png")), Some(Image));
        assert_eq!(
            classify_response_bucket(Some("image/jpeg; charset=binary")),
            Some(Image)
        );
        assert_eq!(classify_response_bucket(Some("font/woff2")), Some(Font));
        assert_eq!(
            classify_response_bucket(Some("application/vnd.ms-fontobject")),
            Some(Font)
        );
        assert_eq!(classify_response_bucket(Some("video/mp4")), Some(Media));
        assert_eq!(classify_response_bucket(Some("audio/mpeg")), Some(Media));
        assert_eq!(
            classify_response_bucket(Some("text/css; charset=utf-8")),
            Some(Stylesheet)
        );
        assert_eq!(
            classify_response_bucket(Some("application/javascript")),
            Some(Script)
        );
        assert_eq!(
            classify_response_bucket(Some("text/html; charset=utf-8")),
            Some(Document)
        );
        assert_eq!(
            classify_response_bucket(Some("application/json; charset=utf-8")),
            Some(Xhr)
        );
        assert_eq!(
            classify_response_bucket(Some("application/vnd.api+json")),
            Some(Xhr)
        );
        assert_eq!(classify_response_bucket(Some("text/plain")), Some(Xhr));
        assert_eq!(
            classify_response_bucket(Some("application/octet-stream")),
            Some(Other)
        );
        assert_eq!(classify_response_bucket(Some("")), None);
        assert_eq!(classify_response_bucket(Some("garbage")), None);
        assert_eq!(classify_response_bucket(None), None);
        assert_eq!(
            classify_response_bucket(Some("IMAGE/PNG")),
            Some(Image),
            "case-insensitive"
        );
    }

    #[test]
    fn pending_timings_correlate_cap_and_expire() {
        let mut p = PendingTimings::new();
        p.note(7, Image, 1_000);
        assert_eq!(p.take(7, 1_600), Some((Image, Some(600), false)));
        assert_eq!(p.take(7, 1_700), None, "a token is consumed once");
        assert_eq!(p.take(99, 1_700), None, "unknown token");
        // AUDIT 0.31 (A2-F9): a HEAD-noted entry carries its
        // flag back out of take.
        p.note_head(11, Image, 2_000, true);
        assert_eq!(p.take(11, 2_300), Some((Image, Some(300), true)));
        // AUDIT 0.30 (A2b): past the TTL the duration is
        // forgotten (not clamped, not counted) — but the type
        // still returns, so the response's bytes calibrate
        // the right slot.
        p.note(8, Script, 0);
        assert_eq!(
            p.take(8, PENDING_TTL_MS + 1),
            Some((Script, None, false)),
            "stale: type known, duration forgotten"
        );
        p.note(9, Font, 0);
        assert_eq!(
            p.take(9, PENDING_TTL_MS),
            Some((Font, Some(PENDING_TTL_MS), false)),
            "at the TTL the sample still counts"
        );
        // The cap holds: flooding distinct tokens keeps the
        // map at PENDING_CAP and evicts the oldest first.
        let mut p = PendingTimings::new();
        for i in 0..(PENDING_CAP as u64 + 50) {
            p.note(i, Other, i);
        }
        assert_eq!(p.len(), PENDING_CAP);
        assert_eq!(p.take(0, 60_000), None, "oldest evicted");
        assert!(
            p.take(PENDING_CAP as u64 + 49, 60_000).is_some(),
            "newest kept"
        );
    }

    #[test]
    fn note_response_uses_correlation_then_falls_back_to_bucket() {
        let book = StatsBook::new();
        // Correlated: lands in the request's slot, both
        // measures.
        book.note_request(42, Script, 10_000);
        book.note_response(42, Some(30_000), Some("text/html"), 10_300);
        let lt = book.lifetime();
        let script = lt.ledger[type_index(Script)];
        assert_eq!(script.byte_samples, 1);
        assert_eq!(script.total_bytes, 30_000);
        assert_eq!(script.duration_samples, 1);
        assert_eq!(script.total_duration_ms, 300);
        // The Content-Type said html, but the correlated slot
        // (Script) won — correlation beats classification.
        assert_eq!(lt.ledger[type_index(Document)].byte_samples, 0);
        // Uncorrelated: bytes only, bucketed by Content-Type,
        // no duration invented.
        book.note_response(43, Some(80_000), Some("image/png"), 11_000);
        let lt = book.lifetime();
        let image = lt.ledger[type_index(Image)];
        assert_eq!(image.byte_samples, 1);
        assert_eq!(image.total_bytes, 80_000);
        assert_eq!(image.duration_samples, 0);
        // Uncorrelated AND unbucketable: nothing recorded.
        book.note_response(44, Some(80_000), None, 11_000);
        let lt2 = book.lifetime();
        assert_eq!(lt2.ledger, lt.ledger);
        // Stale correlation (AUDIT 0.30, A2b): the bytes land
        // in the REQUEST's slot, but no duration is recorded.
        book.note_request(45, Font, 0);
        book.note_response(45, Some(40_000), Some("text/html"), PENDING_TTL_MS + 5_000);
        let font = book.lifetime().ledger[type_index(Font)];
        assert_eq!(font.byte_samples, 1);
        assert_eq!(font.total_bytes, 40_000);
        assert_eq!(font.duration_samples, 0, "stale durations are forgotten");
        // A zero-millisecond duration teaches nothing either.
        book.note_request(46, Font, 5_000);
        book.note_response(46, Some(41_000), None, 5_000);
        let font = book.lifetime().ledger[type_index(Font)];
        assert_eq!(font.byte_samples, 2);
        assert_eq!(font.duration_samples, 0, "zero durations are not samples");
    }

    #[test]
    fn prune_reports_are_bounded_rated_and_counted() {
        let book = StatsBook::new();
        assert!(book.record_prune(1, 1_000, 3, 10_000));
        // Too soon for the same tab.
        assert!(!book.record_prune(1, 1_000, 3, 10_500));
        // Another tab is its own gate.
        assert!(book.record_prune(2, 2_000, 4, 10_500));
        // After the interval, the first tab may report again.
        assert!(book.record_prune(1, 500, 1, 11_500));
        // Payload bounds are re-checked in the book.
        assert!(!book.record_prune(3, MAX_PRUNE_REPORT_BYTES + 1, 1, 20_000));
        assert!(!book.record_prune(3, 1, MAX_PRUNE_REPORT_EVENTS + 1, 20_000));
        let v = book.view();
        assert_eq!(v.shield.pruned_bytes, 3_500);
        assert_eq!(v.shield.pruned_events, 8);
        assert_eq!(v.lite.pruned_bytes, 0, "prune is the Shield's alone");
        // AUDIT 0.30 (A1-3): a 0/0 report is refused AND does
        // not consume the tab's rate slot — a real report
        // 100 ms later is still accepted.
        assert!(!book.record_prune(9, 0, 0, 30_000));
        assert!(
            book.record_prune(9, 10, 1, 30_100),
            "0/0 left the slot free"
        );
        // The session event ceiling binds too: a thousand
        // maximum reports fill it exactly; one more is refused.
        let flood = StatsBook::new();
        for i in 0..1_000u64 {
            assert!(flood.record_prune(1, 1, MAX_PRUNE_REPORT_EVENTS, i * 1_000));
        }
        assert!(!flood.record_prune(1, 1, 1, 1_000_000));
    }

    #[test]
    fn sanitize_caps_totals_at_what_samples_support() {
        let mut s = Savings::default();
        s.ledger[type_index(Image)] = TypeLedger {
            total_bytes: u64::MAX,
            byte_samples: 3,
            total_duration_ms: u64::MAX,
            duration_samples: 2,
        };
        s.shield.blocked = u64::MAX;
        let s = s.sanitized();
        assert_eq!(
            s.ledger[type_index(Image)].total_bytes,
            3 * MAX_SAMPLE_BYTES
        );
        assert_eq!(
            s.ledger[type_index(Image)].total_duration_ms,
            2 * MAX_SAMPLE_DURATION_MS
        );
        assert_eq!(s.shield.blocked, 1 << 62);
        // A zero-sample ledger claims zero totals.
        let mut s2 = Savings::default();
        s2.ledger[type_index(Font)].total_bytes = 5_000;
        assert_eq!(s2.sanitized().ledger[type_index(Font)].total_bytes, 0);
        // AUDIT 0.30 (A5): absurd sample counts scale down
        // proportionally — the measured average survives, the
        // figures stay bounded.
        let mut s3 = Savings::default();
        s3.ledger[type_index(Script)] = TypeLedger {
            total_bytes: 1_000_000_000_000,
            byte_samples: 2_000_000_000,
            total_duration_ms: 600_000_000_000,
            duration_samples: 2_000_000_000,
        };
        let l = s3.sanitized().ledger[type_index(Script)];
        assert_eq!(l.byte_samples, 1 << 30);
        assert_eq!(l.duration_samples, 1 << 30);
        assert_eq!(
            l.avg_bytes(),
            Some(500),
            "average preserved by the scale-down"
        );
        assert_eq!(l.avg_duration_ms(), Some(300));
        assert!(l.total_bytes <= (1 << 62));
    }

    #[test]
    fn reset_zeroes_everything() {
        let book = StatsBook::new();
        book.record_block(Engine::Lite, Font, None);
        book.record_sample(Font, Some(10_000), Some(100));
        assert!(book.record_prune(1, 100, 1, 0));
        book.note_request(5, Image, 0);
        book.reset();
        assert!(book.lifetime().is_zero());
        let v = book.view();
        assert_eq!(v.lite.blocked, 0);
        assert_eq!(v.lite.session_blocked, 0);
        // Pending correlation is gone too: the response falls
        // back to bucket-only (or nothing without a type).
        book.note_response(5, None, None, 100);
        assert!(book.lifetime().is_zero());
    }

    #[test]
    fn humanizers_match_the_shared_fixtures() {
        // These cases are ALSO pinned verbatim in the start
        // page's DOM suite (scripts/dom_test.mjs) against the
        // JS mirrors — one fixture set, two implementations.
        let bytes: [(u64, &str); 14] = [
            (0, "0 B"),
            (999, "999 B"),
            (1_000, "1 KB"),
            (1_500, "1.5 KB"),
            (12_300, "12.3 KB"),
            (999_499, "999 KB"),
            (1_000_000, "1 MB"),
            (86_400_000, "86.4 MB"),
            (1_250_000_000, "1.25 GB"),
            (999_950_000, "1 GB"),
            (5_500_000_000_000, "5.5 TB"),
            (42, "42 B"),
            // Exact rounding ties (AUDIT 0.30, A2b): half-up
            // in BOTH implementations.
            (12_250, "12.3 KB"),
            (10_250, "10.3 KB"),
        ];
        for (n, want) in bytes {
            assert_eq!(humanize_bytes(n), want, "bytes {n}");
        }
        let durations: [(u64, &str); 12] = [
            (0, "0 s"),
            (400, "0.4 s"),
            (45_000, "45 s"),
            (59_000, "59 s"),
            (90_000, "1.5 min"),
            (3_600_000, "1 h"),
            (5_400_000, "1.5 h"),
            (86_400_000, "1 day"),
            (216_000_000, "2.5 days"),
            (1_000, "1 s"),
            (1_125, "1.13 s"),
            (12_250, "12.3 s"),
        ];
        for (n, want) in durations {
            assert_eq!(humanize_duration(n), want, "duration {n}");
        }
    }
}
