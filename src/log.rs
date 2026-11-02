//! Tiny structured log at `<data dir>\pico.log` — Pico's data
//! directory: `data\` beside the exe since v27 (appdata only
//! as the not-writable fallback; AUDIT V27-C).
//!
//! Diagnostics only — nothing leaves the machine. One line per
//! event: `[<unix-ms>] <message>`. The file is rotated at
//! startup when it exceeds 1 MiB **and** mid-session when the
//! running total crosses the same cap (AUDIT L12 — checking only
//! at startup let one long session grow it without bound).
//! Rotation drops our own handle BEFORE removing the file:
//! Windows cannot delete an open file, so the order is the
//! feature. All writes are best-effort: logging must never
//! crash or block the app.
//!
//! Anything URL-shaped must pass through
//! `pico_core::core::redact::redact_url` before it is formatted
//! into a message (AUDIT M5): query strings and fragments never
//! reach this file. Control characters are stripped here as a
//! second wall.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_BYTES: u64 = 1024 * 1024;

struct LogState {
    file: Option<std::fs::File>,
    written: u64,
}

static LOG: Mutex<Option<LogState>> = Mutex::new(None);

pub fn log_path() -> PathBuf {
    pico_core::core::state::pico_dir().join("pico.log")
}

fn open_log_file() -> Option<std::fs::File> {
    let path = log_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok()
}

/// Open (creating if needed) the log file. Rotates by deletion
/// when oversized. Call once, early.
pub fn init() {
    let path = log_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut existing_len = 0u64;
    if let Ok(meta) = std::fs::metadata(&path) {
        existing_len = meta.len();
        if existing_len > MAX_BYTES {
            // AUDIT 0.31 (A5-L8): the byte counter may only be
            // zeroed when the file is actually GONE (the remove
            // succeeded, or a re-stat finds nothing). Through
            // 0.30 a failed remove still zeroed it, leaving the
            // 1 MiB cap unenforced exactly when deletes kept
            // failing.
            let removed = std::fs::remove_file(&path).is_ok() || std::fs::metadata(&path).is_err();
            if removed {
                existing_len = 0;
            }
        }
    }
    if let Ok(mut guard) = LOG.lock() {
        *guard = Some(LogState {
            file: open_log_file(),
            written: existing_len,
        });
    }
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// Make a message safe to store in a plain-text log a human may
/// later open in a terminal: strip every control character
/// (ANSI escapes, CR/LF smuggling, NULs — log lines carry
/// attacker-shaped strings like page titles) and cap the line
/// length so a hostile value cannot bloat the file one line at
/// a time.
fn sanitize(msg: &str) -> String {
    const MAX_CHARS: usize = 4096;
    let mut out = String::with_capacity(msg.len().min(MAX_CHARS));
    for c in msg.chars().take(MAX_CHARS) {
        // AUDIT V28 (A5-L6): Cc controls are not the whole
        // story — U+2028/U+2029 and the bidi/zero-width format
        // characters pass is_control() and forge log lines
        // visually; the shared helper in core::redact names
        // them and both cleaners strip them.
        if c.is_control() || pico_core::core::redact::is_invisible_format(c) {
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    if msg.chars().count() > MAX_CHARS {
        out.push_str("\u{2026}[truncated]");
    }
    out
}

pub fn write_line(msg: &str) {
    if let Ok(mut guard) = LOG.lock() {
        if let Some(state) = guard.as_mut() {
            // Mid-session rotation: if this session has pushed
            // the file past the cap, start over with a fresh
            // file (handle dropped first — see module doc).
            if state.written > MAX_BYTES {
                state.file = None;
                // AUDIT 0.31 (A5-L8): only a confirmed removal
                // resets the counter; otherwise it stays over
                // the cap and the next line retries the
                // rotation instead of appending to an already
                // oversized file under a zeroed counter.
                let path = log_path();
                let removed =
                    std::fs::remove_file(&path).is_ok() || std::fs::metadata(&path).is_err();
                state.file = open_log_file();
                if removed {
                    state.written = 0;
                }
            }
            // AUDIT V28 (A5-L5): a failed open (at init or in
            // rotation) used to leave file=None for the whole
            // session — every later line silently dropped,
            // exactly when diagnostics mattered. Retry the open
            // on each write while there is no handle.
            if state.file.is_none() {
                state.file = open_log_file();
            }
            if let Some(file) = state.file.as_mut() {
                let line = format!("[{}] {}\n", now_ms(), sanitize(msg));
                state.written += line.len() as u64;
                let _ = file.write_all(line.as_bytes());
                let _ = file.flush();
            }
        }
    }
}

pub fn info(msg: &str) {
    write_line(msg);
}

pub fn error(msg: &str) {
    write_line(&format!("ERROR {msg}"));
}

/// Install a panic hook that records panics in the log before
/// the default hook prints them (release builds use
/// `panic = "abort"`, but the hook still runs first).
pub fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let loc = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "?".to_string());
        let payload = if let Some(s) = info.payload().downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "unknown payload".to_string()
        };
        write_line(&format!("PANIC at {loc}: {payload}"));
        default_hook(info);
    }));
}
