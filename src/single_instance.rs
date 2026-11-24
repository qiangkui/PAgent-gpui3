//! Keeping a second window from writing the same data.
//!
//! The window keeps conversations, workflows and settings in one folder and
//! saves after every change, so two instances would overwrite each other's
//! files. The first one to start claims the folder; a later one brings the
//! existing window forward and leaves the files to it.

use std::path::Path;

/// A claim on the data folder, released when the process ends.
pub struct InstanceGuard {
    #[cfg(windows)]
    handle: isize,
    #[cfg(not(windows))]
    file: std::path::PathBuf,
}

/// Claim `home` for this process.
///
/// `None` means another instance already holds it. A system that cannot tell (a
/// mutex that cannot be created, an unreadable lock file) allows the run rather
/// than refusing to start: a window that cannot open is worse than one that
/// might conflict.
pub fn acquire(home: &Path) -> Option<InstanceGuard> {
    #[cfg(windows)]
    {
        win::acquire(&name_for(home)).map(|handle| InstanceGuard { handle })
    }
    #[cfg(not(windows))]
    {
        file::acquire(home).map(|file| InstanceGuard { file })
    }
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        #[cfg(windows)]
        win::release(self.handle);
        #[cfg(not(windows))]
        file::release(&self.file);
    }
}

/// The name the claim is filed under, derived from the data folder.
///
/// The folder decides which files are shared, so two installations next to each
/// other get different names and can still run side by side.
fn name_for(home: &Path) -> String {
    let text = home.display().to_string();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("dsh-flow-ui-{hash:016x}")
}

/// Bring an existing window to the front, best effort.
pub fn focus_existing() {
    #[cfg(windows)]
    win::focus("PAgent");
}
#[cfg(windows)]
mod win {
    use std::ffi::c_void;

    /// `ERROR_ALREADY_EXISTS`.
    const ERROR_ALREADY_EXISTS: u32 = 183;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateMutexW(attributes: *const c_void, initial_owner: i32, name: *const u16) -> isize;
        fn CloseHandle(handle: isize) -> i32;
        fn GetLastError() -> u32;
        fn FindWindowW(class: *const u16, title: *const u16) -> isize;
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn SetForegroundWindow(window: isize) -> i32;
        fn ShowWindow(window: isize, command: i32) -> i32;
    }

    /// Claim `name`; `None` when another process already did.
    pub fn acquire(name: &str) -> Option<isize> {
        let name = wide(name);
        // SAFETY: the name is a live NUL-terminated string. A zero handle means
        // the system refused; the caller then runs without a claim.
        let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
        if handle == 0 {
            return Some(0);
        }
        // SAFETY: reading the error code right after the call is the documented
        // way to tell a fresh mutex from one that already existed.
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            unsafe { CloseHandle(handle) };
            return None;
        }
        Some(handle)
    }

    /// Release a claim; a zero handle holds nothing.
    pub fn release(handle: isize) {
        if handle != 0 {
            // SAFETY: the handle came from `CreateMutexW` and is closed once.
            unsafe { CloseHandle(handle) };
        }
    }

    /// Raise the window with `title`, if it exists.
    pub fn focus(title: &str) {
        let title = wide(title);
        // SAFETY: the string is live; a missing window answers zero and nothing
        // happens.
        let window = unsafe { FindWindowW(std::ptr::null(), title.as_ptr()) };
        if window != 0 {
            const SW_RESTORE: i32 = 9;
            // SAFETY: the window handle was just looked up and is only shown and
            // raised, never destroyed.
            unsafe {
                ShowWindow(window, SW_RESTORE);
                SetForegroundWindow(window);
            }
        }
    }

    /// A NUL-terminated UTF-16 copy of `text`.
    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }
}
#[cfg(not(windows))]
mod file {
    use std::io::Write;
    use std::path::{Path, PathBuf};

    /// Claim the folder with a lock file; `None` when a live process holds it.
    pub fn acquire(home: &Path) -> Option<PathBuf> {
        let path = home.join(".dsh-flow-ui.lock");
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut handle) => {
                let _ = writeln!(handle, "{}", std::process::id());
                Some(path)
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                // A run that crashed leaves the file behind; only a live owner
                // keeps the claim.
                if owner_alive(&path) {
                    return None;
                }
                let _ = std::fs::remove_file(&path);
                acquire(home)
            }
            // Cannot tell: run without a claim rather than refuse to start.
            Err(_) => Some(PathBuf::new()),
        }
    }

    /// Drop a claim; an empty path holds nothing.
    pub fn release(path: &Path) {
        if !path.as_os_str().is_empty() {
            let _ = std::fs::remove_file(path);
        }
    }

    /// Whether the process that wrote the lock file is still running.
    fn owner_alive(path: &Path) -> bool {
        let Ok(text) = std::fs::read_to_string(path) else {
            return false;
        };
        let Some(pid) = text.trim().parse::<u32>().ok() else {
            return false;
        };
        if pid == 0 {
            return false;
        }
        #[cfg(target_os = "linux")]
        {
            Path::new(&format!("/proc/{pid}")).exists()
        }
        #[cfg(not(target_os = "linux"))]
        {
            std::process::Command::new("kill")
                .args(["-0", &pid.to_string()])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_claim_name_follows_the_data_folder() {
        let one = name_for(Path::new("C:\\apps\\one"));
        assert_eq!(one, name_for(Path::new("C:\\apps\\one")));
        assert_ne!(one, name_for(Path::new("C:\\apps\\two")));
        assert!(one.starts_with("dsh-flow-ui-"));
    }
}