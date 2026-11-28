//! Where API keys are kept.
//!
//! On Windows a key lives in the Credential Manager (generic credentials under
//! `dsh-flow-ui/<connection id>`), so `config.json` never holds one. Other
//! platforms, and a Windows run where the store refuses the write, fall back to
//! the settings file: the window has to keep working even there, and the
//! fallback is written by `config.rs`, not here.

#[cfg(test)]
use std::collections::HashMap;
#[cfg(test)]
use std::sync::Mutex;

/// Prefix every stored credential carries, so they are recognisable in the
/// Windows Credential Manager and can be removed by hand.
pub const TARGET_PREFIX: &str = "dsh-flow-ui/";

/// Somewhere a secret can be kept between runs.
pub trait SecretStore {
    /// The secret stored under `target`, if any.
    fn read(&self, target: &str) -> Option<String>;
    /// Keep `secret` under `target`.
    fn write(&self, target: &str, secret: &str) -> Result<(), String>;
    /// Forget `target`; a missing entry is not an error.
    fn forget(&self, target: &str);
}

/// Forget one connection's key, best effort.
///
/// Called when a card is removed: leaving the secret behind would leave a
/// credential the window can no longer show or delete.
pub fn forget_connection(id: &str) {
    system().forget(&format!("{TARGET_PREFIX}{id}"));
}

/// The target one workflow input is kept under.
///
/// Workflow inputs live in the same store as endpoint keys, separated by a
/// second level: `dsh-flow-ui/workflow:<name>:input:<input>`. The shape is
/// documented in the workflow card's design notes, so both sides agree on what
/// "保存到本机" means.
/// @param workflow - the manifest's `name`.
/// @param input - the declared input's name.
/// @returns the credential target name.
pub fn input_target(workflow: &str, input: &str) -> String {
    format!(
        "{TARGET_PREFIX}workflow:{}:input:{}",
        credential_label(workflow),
        credential_label(input)
    )
}

/// One part of a target, with separators that would confuse the key removed.
///
/// The Credential Manager shows target names verbatim, and the UI reads the
/// suffix back to know which input a secret belongs to, so `:` and `/` may not
/// appear inside a name.
fn credential_label(text: &str) -> String {
    let cleaned: String = text
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_alphanumeric() || matches!(ch, '_' | '-' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "_".to_string()
    } else {
        cleaned
    }
}

/// A workflow input kept on this machine, if any.
/// @param workflow - the manifest's `name`.
/// @param input - the declared input's name.
/// @returns the stored value; empty values are treated as absent.
pub fn read_input(workflow: &str, input: &str) -> Option<String> {
    system()
        .read(&input_target(workflow, input))
        .filter(|value| !value.is_empty())
}

/// Keep one workflow input on this machine.
/// @returns the store's refusal, when it has one (no store on this platform).
pub fn save_input(workflow: &str, input: &str, value: &str) -> Result<(), String> {
    system().write(&input_target(workflow, input), value)
}

/// Forget one workflow input, best effort.
pub fn forget_input(workflow: &str, input: &str) {
    system().forget(&input_target(workflow, input));
}

/// The store a run should use.
pub fn system() -> &'static dyn SecretStore {
    #[cfg(windows)]
    {
        &WindowsStore
    }
    #[cfg(not(windows))]
    {
        &NoStore
    }
}

/// Stand-in for a machine without the Windows Credential Manager.
///
/// Writes fail on purpose: `config.rs` then keeps the key in the settings file
/// instead of losing it.
#[cfg(any(test, not(windows)))]
pub struct NoStore;

#[cfg(any(test, not(windows)))]
impl SecretStore for NoStore {
    fn read(&self, _target: &str) -> Option<String> {
        None
    }

    fn write(&self, _target: &str, _secret: &str) -> Result<(), String> {
        Err("这台系统没有可用的凭据存储".to_string())
    }

    fn forget(&self, _target: &str) {}
}

/// An in-memory store, for tests and for a run whose store cannot be written.
#[cfg(test)]
#[derive(Default)]
pub struct MemoryStore {
    entries: Mutex<HashMap<String, String>>,
}

#[cfg(test)]
impl MemoryStore {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// The secret kept under `target`, for assertions.
    pub fn get(&self, target: &str) -> Option<String> {
        self.entries
            .lock()
            .ok()
            .and_then(|entries| entries.get(target).cloned())
    }
}

#[cfg(test)]
impl SecretStore for MemoryStore {
    fn read(&self, target: &str) -> Option<String> {
        self.entries
            .lock()
            .ok()
            .and_then(|entries| entries.get(target).cloned())
    }

    fn write(&self, target: &str, secret: &str) -> Result<(), String> {
        self.entries
            .lock()
            .map_err(|_| "凭据存储不可用".to_string())?
            .insert(target.to_string(), secret.to_string());
        Ok(())
    }

    fn forget(&self, target: &str) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.remove(target);
        }
    }
}

#[cfg(windows)]
mod win {
    use std::ffi::c_void;

    use super::{SecretStore, TARGET_PREFIX};

    /// `CRED_TYPE_GENERIC`.
    const CRED_TYPE_GENERIC: u32 = 1;
    /// `CRED_PERSIST_LOCAL_MACHINE`.
    const CRED_PERSIST_LOCAL_MACHINE: u32 = 2;

    /// One credential as `CredReadW` returns it.
    #[repr(C)]
    struct Credential {
        flags: u32,
        kind: u32,
        target_name: *mut u16,
        comment: *mut u16,
        last_written: [u32; 2],
        credential_blob_size: u32,
        credential_blob: *mut u8,
        persist: u32,
        attribute_count: u32,
        attributes: *mut c_void,
        target_alias: *mut u16,
        user_name: *mut u16,
    }

    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn CredWriteW(credential: *const Credential, flags: u32) -> i32;
        fn CredReadW(
            target: *const u16,
            kind: u32,
            flags: u32,
            credential: *mut *mut Credential,
        ) -> i32;
        fn CredDeleteW(target: *const u16, kind: u32, flags: u32) -> i32;
        fn CredFree(buffer: *mut c_void);
    }

    /// The Windows Credential Manager, as a store.
    pub struct WindowsStore;

    impl SecretStore for WindowsStore {
        fn read(&self, target: &str) -> Option<String> {
            let name = wide(&target_name(target));
            let mut credential: *mut Credential = std::ptr::null_mut();
            // SAFETY: `name` is a NUL-terminated UTF-16 string that outlives the
            // call, and the pointer is only used after a successful read.
            let ok = unsafe { CredReadW(name.as_ptr(), CRED_TYPE_GENERIC, 0, &mut credential) };
            if ok == 0 || credential.is_null() {
                return None;
            }
            // SAFETY: a successful `CredReadW` hands back one credential that
            // `CredFree` has to release, and the blob is `size` bytes long.
            let secret = unsafe {
                let entry = &*credential;
                let bytes = std::slice::from_raw_parts(
                    entry.credential_blob,
                    entry.credential_blob_size as usize,
                );
                let text = String::from_utf8_lossy(bytes).to_string();
                CredFree(credential.cast::<c_void>());
                text
            };
            Some(secret)
        }

        fn write(&self, target: &str, secret: &str) -> Result<(), String> {
            let mut name = wide(&target_name(target));
            let mut blob = secret.as_bytes().to_vec();
            let credential = Credential {
                flags: 0,
                kind: CRED_TYPE_GENERIC,
                target_name: name.as_mut_ptr(),
                comment: std::ptr::null_mut(),
                last_written: [0; 2],
                credential_blob_size: blob.len() as u32,
                credential_blob: blob.as_mut_ptr(),
                persist: CRED_PERSIST_LOCAL_MACHINE,
                attribute_count: 0,
                attributes: std::ptr::null_mut(),
                target_alias: std::ptr::null_mut(),
                user_name: std::ptr::null_mut(),
            };
            // SAFETY: every pointer in `credential` refers to a live local that
            // outlives the call, and the blob length matches its buffer.
            let ok = unsafe { CredWriteW(&credential, 0) };
            if ok == 0 {
                return Err(format!(
                    "写入 Windows 凭据管理器失败（错误码 {}）",
                    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
                ));
            }
            Ok(())
        }

        fn forget(&self, target: &str) {
            let name = wide(&target_name(target));
            // SAFETY: the name is a live NUL-terminated string. A missing entry
            // is reported through the return value and is not an error here.
            unsafe {
                CredDeleteW(name.as_ptr(), CRED_TYPE_GENERIC, 0);
            }
        }
    }

    /// The name the credential is stored under.
    fn target_name(id: &str) -> String {
        format!("{TARGET_PREFIX}{id}")
    }

    /// A NUL-terminated UTF-16 copy of `text`.
    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }
}

#[cfg(windows)]
pub use win::WindowsStore;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_memory_store_round_trips_and_forgets() {
        let store = MemoryStore::new();
        assert_eq!(store.read("dsh-flow-ui/a"), None);
        store.write("dsh-flow-ui/a", "sk-a").expect("write");
        assert_eq!(store.read("dsh-flow-ui/a").as_deref(), Some("sk-a"));
        assert_eq!(store.get("dsh-flow-ui/a").as_deref(), Some("sk-a"));
        store.forget("dsh-flow-ui/a");
        assert_eq!(store.read("dsh-flow-ui/a"), None);
        // Forgetting twice is fine.
        store.forget("dsh-flow-ui/a");
    }

    #[test]
    fn the_fallback_store_refuses_to_hold_secrets() {
        let store = NoStore;
        assert!(store.write("dsh-flow-ui/a", "sk-a").is_err());
        assert_eq!(store.read("dsh-flow-ui/a"), None);
    }

    #[test]
    fn workflow_inputs_get_their_own_target() {
        assert_eq!(
            input_target("maers-units-export", "password"),
            "dsh-flow-ui/workflow:maers-units-export:input:password"
        );
        // Separators that would split the key are folded, so the suffix after
        // the last colon is always the input name.
        assert_eq!(
            input_target("a/b:c", "p:w/d"),
            "dsh-flow-ui/workflow:a_b_c:input:p_w_d"
        );
        assert_eq!(input_target("  ", ""), "dsh-flow-ui/workflow:_:input:_");
        // It is not the endpoint key of a connection that happens to share the id.
        assert_ne!(input_target("chat", "key"), format!("{TARGET_PREFIX}chat"));
    }

    #[test]
    fn a_memory_store_round_trips_one_workflow_input() {
        let store = MemoryStore::new();
        let target = input_target("maers-units-export", "password");
        store.write(&target, "p@ss").expect("write");
        assert_eq!(store.read(&target).as_deref(), Some("p@ss"));
        assert_eq!(
            store.read(&input_target("maers-units-export", "account")),
            None,
            "each input has its own slot"
        );
        store.forget(&target);
        assert_eq!(store.read(&target), None);
    }

    /// Round-trips through the real store; run by hand because it writes a
    /// credential into the profile: `cargo test real_credential -- --ignored`.
    #[cfg(windows)]
    #[test]
    #[ignore = "writes a test credential into the real Windows Credential Manager"]
    fn the_windows_store_round_trips() {
        let store = WindowsStore;
        let target = "dsh-flow-ui/selftest";
        store.write(target, "sk-selftest").expect("write");
        assert_eq!(store.read(target).as_deref(), Some("sk-selftest"));
        store.forget(target);
        assert_eq!(store.read(target), None);
    }
}
