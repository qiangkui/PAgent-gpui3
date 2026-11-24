//! Letting a helper process' dialog come to the front.
//!
//! Both pickers run their dialog in a child process (gpui has none of its own).
//! Windows only lets the *foreground* process raise a window, so a dialog shown
//! by a freshly spawned helper normally lands behind this window — or only
//! flashes its taskbar button, which reads as "the click did nothing". Asking the
//! system for the permission first is the documented way around that.

/// Allow the next window any process shows to take the foreground.
#[cfg(windows)]
pub fn allow_child_window_foreground() {
    /// `ASFW_ANY`.
    const ASFW_ANY: u32 = u32::MAX;

    #[link(name = "user32")]
    unsafe extern "system" {
        fn AllowSetForegroundWindow(process_id: u32) -> i32;
    }

    // SAFETY: the call takes no pointers; it only relaxes the foreground lock so
    // the helper's dialog can come up in front of this window.
    unsafe {
        AllowSetForegroundWindow(ASFW_ANY);
    }
}

/// Nothing to relax on a platform without the Windows foreground rules.
#[cfg(not(windows))]
pub fn allow_child_window_foreground() {}
