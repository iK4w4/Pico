//! Manifest-free DPI awareness.
//!
//! Pico ships with NO embedded manifest (see BUILD_NOTES.md for
//! the forensic rationale: a hand-built `.rsrc` is what made v11
//! fail with "side-by-side configuration is incorrect" on the
//! user's Windows, while the manifest-less v1/v2 binaries
//! launched fine). The one job the manifest used to do —
//! declaring Per-Monitor V2 DPI awareness — is done here at
//! runtime instead, before the tao `EventLoop` exists:
//!
//! 1. `SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)`
//! 2. `SetProcessDpiAwareness(PROCESS_PER_MONITOR_DPI_AWARE)`
//! 3. `SetProcessDPIAware()`
//!
//! First success wins. tao 0.34's Windows backend runs the same
//! ladder in `become_dpi_aware()` when the event loop is created;
//! if our earlier call already set the awareness, tao's later
//! calls fail harmlessly (ERROR_ACCESS_DENIED) and are ignored
//! there. The outcome is recorded in a static so `app::run` can
//! write it to pico.log once logging is up.

use std::sync::OnceLock;

static OUTCOME: OnceLock<String> = OnceLock::new();

/// Which rung of the ladder succeeded (or why none did).
pub fn outcome() -> &'static str {
    OUTCOME.get().map(String::as_str).unwrap_or("not attempted")
}

/// Attempt the DPI ladder. Call once, before creating the
/// EventLoop. Never panics; failures are recorded, not
/// propagated.
#[cfg(windows)]
pub fn enable() {
    use windows_sys::Win32::Foundation::GetLastError;
    use windows_sys::Win32::UI::HiDpi::{
        SetProcessDpiAwareness, SetProcessDpiAwarenessContext,
        DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, PROCESS_PER_MONITOR_DPI_AWARE,
    };
    // In windows-sys 0.59 the legacy rung lives in
    // WindowsAndMessaging.
    use windows_sys::Win32::UI::WindowsAndMessaging::SetProcessDPIAware;

    let result = unsafe {
        // SAFETY: the three ladder calls are process-global DPI
        // setters with no pointer arguments; they are called once,
        // before any window or event loop exists, and their BOOL /
        // HRESULT results are checked rung by rung. GetLastError is
        // read immediately after each failure, on the same thread.
        if SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) != 0 {
            "SetProcessDpiAwarenessContext(PER_MONITOR_AWARE_V2) succeeded"
        } else {
            let e1 = GetLastError();
            // Rung 2: Per-Monitor via shcore (Win8.1+). S_OK == 0.
            if SetProcessDpiAwareness(PROCESS_PER_MONITOR_DPI_AWARE) == 0 {
                "SetProcessDpiAwareness(PROCESS_PER_MONITOR_DPI_AWARE) succeeded"
            } else {
                let e2 = GetLastError();
                // Rung 3: legacy system-DPI awareness (Vista+).
                if SetProcessDPIAware() != 0 {
                    "SetProcessDPIAware() succeeded"
                } else {
                    let e3 = GetLastError();
                    // Note: ERROR_ACCESS_DENIED (5) on rung 1
                    // usually means awareness was already set — a
                    // success in effect, just not by us.
                    return store(format!(
                        "no DPI call succeeded (errors: context={e1}, awareness={e2}, legacy={e3}); \
                         tao's become_dpi_aware() will retry at EventLoop creation"
                    ));
                }
            }
        }
    };
    store(result.to_string());
}

#[cfg(not(windows))]
pub fn enable() {
    store("not attempted (non-Windows build)".to_string());
}

fn store(s: String) {
    let _ = OUTCOME.set(s);
}
