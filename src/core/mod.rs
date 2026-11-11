//! The testable core of Pico. `app.rs` is wiring only: every
//! decision that can be made without a window lives here and is
//! covered by the integration suites in `tests/`.

/// The version the USER names this release by: **0.34.1**.
/// Every user-visible string — the chrome badge, the start
/// page, the log banner — shows exactly this. The display
/// form is the name the user gives the release: a feature
/// release is named by major.minor ("0.32" for Cargo
/// 0.32.0, as "0.31" was for 0.31.0); a repair release is
/// named by its full version ("0.30.1" for Cargo 0.30.1).
/// 0.32 adds the download manager (`core::download`);
/// 0.32.1 is its audit-repair release (no new features);
/// 0.32.2 is the first on-device fix round (interception
/// logging, instant pause, the downloads popover);
/// 0.32.3 is the second (the captured-navigation unwind,
/// the download toast, the themed address bar);
/// 0.32.4 is the third (panels close on a click into the
/// page via content focus; a site-opened tab that only
/// carried a download closes when its task lands).
/// 0.33 is the downloads feature release (the dedicated
/// downloads page, honest recovery with refresh-link,
/// reason-aware auto-retry, limits, and the bulletproof
/// captured-tab close).
/// 0.34 is the downloads rebuild (one command table,
/// one coordinator, one shared row renderer — the
/// coordination layer rebuilt from zero; the engine
/// underneath is 0.33's, kept).
/// The pin test below ties the two together — the display
/// form must be the Cargo version or a dotted prefix of
/// it — so a Cargo bump without a display bump (or the
/// reverse) fails the suite.
pub const DISPLAY_VERSION: &str = "0.40.2";

#[cfg(test)]
mod tests {
    #[test]
    fn display_version_matches_cargo_version() {
        let cargo = env!("CARGO_PKG_VERSION");
        let display = super::DISPLAY_VERSION;
        assert!(
            cargo == display || cargo.starts_with(&format!("{display}.")),
            "DISPLAY_VERSION ({display}) must be the Cargo version ({cargo}) \
             or its dotted prefix (the name the user gives the release)"
        );
    }
}

pub mod action;
pub mod addr;
pub mod assets;
pub mod ctxmenu;
pub mod detect;
pub mod dlcmd;
pub mod dlcoord;
pub mod download;
pub mod fileopen;
pub mod filterpipe;
pub mod history;
pub mod ipc;
pub mod layout;
pub mod lite;
// Public edition only: filter-list acquisition (canonical
// sources, cache, validation). The dev tree embeds its
// lists instead and has no such module.
pub mod lists;
pub mod media;
pub mod qr;
pub mod redact;
pub mod server;
pub mod sha256;
pub mod shield;
pub mod state;
pub mod stats;
pub mod teleport;
pub mod themes;
pub mod urlstate;
