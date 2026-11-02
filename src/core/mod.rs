//! The testable core of Pico. `app.rs` is wiring only: every
//! decision that can be made without a window lives here and is
//! covered by the integration suites in `tests/`.

/// The version the USER names this release by: **0.31**.
/// Every user-visible string — the chrome badge, the start
/// page, the log banner — shows exactly this. The display
/// form is the name the user gives the release: a feature
/// release is named by major.minor ("0.31" for Cargo
/// 0.31.0, as "0.30" was for 0.30.0); a repair release is
/// named by its full version ("0.30.1" for Cargo 0.30.1).
/// The pin test below ties the two together — the display
/// form must be the Cargo version or a dotted prefix of
/// it — so a Cargo bump without a display bump (or the
/// reverse) fails the suite.
pub const DISPLAY_VERSION: &str = "0.31";

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
pub mod detect;
pub mod fileopen;
pub mod filterpipe;
pub mod history;
pub mod ipc;
pub mod layout;
pub mod lists;
pub mod lite;
pub mod media;
pub mod qr;
pub mod redact;
pub mod server;
pub mod shield;
pub mod state;
pub mod stats;
pub mod teleport;
pub mod themes;
pub mod urlstate;
