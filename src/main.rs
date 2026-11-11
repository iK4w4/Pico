#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Pico 0.40.2 — a tiny neumorphic Windows browser for local development.
//!
//! All logic lives in `app` (GUI shell) and the `pico_core` library
//! (static file server, localhost detector, history machine, typed
//! IPC, address parsing, state, log redaction, the Shield).
//!
//! Pico ships with NO embedded manifest and NO `.rsrc` section: the
//! hand-built resource container is what made v11 fail on the user's
//! Windows with a side-by-side (SxS) activation error, while the
//! manifest-less v1/v2 binaries launched fine. DPI awareness — the
//! one thing the manifest was for — is set at runtime in `dpi`
//! instead.

mod app;
mod dlbackend;
mod downloads;
mod dpi;
mod log;
mod winhttp;

fn main() {
    // DPI awareness must be set before the EventLoop (and therefore
    // any window) exists. The outcome is logged from app::run() once
    // the log file is open.
    dpi::enable();
    app::run();
}
