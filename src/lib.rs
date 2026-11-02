//! Pico v26 core library: everything that does not need a GUI.
//!
//! The library has zero windowing dependencies so it can be
//! compiled and tested on any host
//! (`cargo test --no-default-features`). The Windows GUI binary
//! (`src/main.rs` + `src/app.rs`, feature `gui`) is a thin shell
//! over these modules.

pub mod core;
