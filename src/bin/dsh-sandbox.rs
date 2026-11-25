//! A best-effort Windows write sandbox for tool commands.
//!
//! The engine starts this helper instead of the command itself. The helper:
//! 1. lowers the child token to Low integrity, so writes outside folders marked
//!    Low are denied by the OS (reads and network still work);
//! 2. puts the child in a Job Object, so stopping the helper kills the whole
//!    process tree;
//! 3. hands the helper's own stdio handles to the child, so live output works
//!    exactly as before.
//!
//! Workspace mode marks the conversation folder Low (once, with a marker file);
//! read-only mode marks only a private temp folder Low, leaving the workspace at
//! its normal integrity so the sandboxed command cannot write it.
//!
//! This is not an AppContainer: it does not hide the rest of the disk or block
//! network access. It is the Windows equivalent of "workspace-write" for the
//! process, enforced by the OS instead of by scanning command text.

#[cfg(not(windows))]
fn main() {
    eprintln!("SANDBOX ERROR: 仅支持 Windows");
    std::process::exit(125);
}

#[cfg(windows)]
fn main() {
    use std::ffi::c_void;
    use std::path::PathBuf;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0};
    use windows_sys::Win32::Security::Authorization::ConvertStringSidToSidW;
    use windows_sys::Win32::Security::{
        CreateRestrictedToken, SetTokenInformation, TokenIntegrityLevel,
        DISABLE_MAX_PRIVILEGE, PSID, SID_AND_ATTRIBUTES, TOKEN_ADJUST_DEFAULT,
        TOKEN_ADJUST_SESSIONID, TOKEN_ASSIGN_PRIMARY, TOKEN_DUPLICATE, TOKEN_MANDATORY_LABEL,
        TOKEN_QUERY,
    };
    use windows_sys::Win32::System::Console::{
        GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
    };
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows_sys::Win32::System::SystemServices::SE_GROUP_INTEGRITY;
    use windows_sys::Win32::System::Threading::{
        CreateProcessAsUserW, GetCurrentProcess, GetExitCodeProcess, OpenProcessToken, ResumeThread,
        TerminateProcess, WaitForSingleObject, CREATE_NO_WINDOW, CREATE_SUSPENDED,
        CREATE_UNICODE_ENVIRONMENT, INFINITE, PROCESS_INFORMATION, STARTF_USESTDHANDLES,
        STARTUPINFOW,
    };

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Quote one argument the way CommandLineToArgvW / CreateProcess expect.
    fn quote_arg(arg: &str) -> String {
        if !arg.is_empty() && !arg.chars().any(|ch| matches!(ch, ' ' | '\t' | '"')) {
            return arg.to_string();
        }
        let mut out = String::from("\"");
        let mut slashes = 0usize;
        for ch in arg.chars() {
            match ch {
                '\\' => slashes += 1,
                '"' => {
                    out.push_str(&"\\".repeat(slashes * 2 + 1));
                    out.push('"');
                    slashes = 0;
                }
                _ => {
                    out.push_str(&"\\".repeat(slashes));
                    out.push(ch);
                    slashes = 0;
                }
            }
        }
        out.push_str(&"\\".repeat(slashes * 2));
        out.push('"');
        out
    }

    fn fail(message: &str) -> ! {
        eprintln!("SANDBOX ERROR: {message}");
        std::process::exit(125);
    }

    /// Set a folder's integrity level, once for the requested level.
    fn mark_integrity(folder: &PathBuf, level: &str, recursive: bool) {
        let marker = folder.join(".dsh-flow").join("integrity");
        if std::fs::read_to_string(&marker)
            .map(|current| current.trim() == level)
            .unwrap_or(false)
        {
            return;
        }
        let mut command = std::process::Command::new("icacls");
        command
            .arg(folder)
            .args(["/setintegritylevel", &format!("(OI)(CI){level}"), "/Q", "/C"]);
        if recursive {
            command.arg("/T");
        }
        // This helper runs without a console of its own, so icacls must not
        // open one.
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        let status = command.status();
        if !status.map(|status| status.success()).unwrap_or(false) {
            fail("无法设置工作区的完整性级别（icacls 失败）");
        }
        let _ = std::fs::create_dir_all(folder.join(".dsh-flow"));
        let _ = std::fs::write(marker, level.as_bytes());
    }

    let mut args = std::env::args().skip(1);
    let first = args.next();
    if first.as_deref() == Some("--check") {
        // Probe the APIs the sandbox depends on; PAgent uses the exit code to
        // decide whether to hand commands to the helper at all.
        unsafe {
            let mut token: HANDLE = std::ptr::null_mut();
            let desired = TOKEN_QUERY
                | TOKEN_DUPLICATE
                | TOKEN_ASSIGN_PRIMARY
                | TOKEN_ADJUST_DEFAULT
                | TOKEN_ADJUST_SESSIONID;
            if OpenProcessToken(GetCurrentProcess(), desired, &mut token) == 0 {
                fail("无法打开当前进程令牌");
            }
            let mut restricted: HANDLE = std::ptr::null_mut();
            if CreateRestrictedToken(
                token,
                DISABLE_MAX_PRIVILEGE,
                0,
                std::ptr::null(),
                0,
                std::ptr::null(),
                0,
                std::ptr::null(),
                &mut restricted,
            ) == 0
            {
                close(token);
                fail("无法创建受限令牌");
            }
            close(token);
            let mut low_sid: PSID = std::ptr::null_mut();
            let low_text = wide("S-1-16-4096");
            if ConvertStringSidToSidW(low_text.as_ptr(), &mut low_sid) == 0 {
                close(restricted);
                fail("无法解析低完整性 SID");
            }
            let label = TOKEN_MANDATORY_LABEL {
                Label: SID_AND_ATTRIBUTES {
                    Sid: low_sid,
                    Attributes: SE_GROUP_INTEGRITY as u32,
                },
            };
            if SetTokenInformation(
                restricted,
                TokenIntegrityLevel,
                &label as *const _ as *const c_void,
                std::mem::size_of::<TOKEN_MANDATORY_LABEL>() as u32,
            ) == 0
            {
                close(restricted);
                fail("无法把令牌降到低完整性");
            }
            close(restricted);
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() || job == INVALID_HANDLE_VALUE {
                fail("无法创建作业对象");
            }
            close(job);
        }
        return;
    }

    fn close(handle: HANDLE) {
        if !handle.is_null() && handle != INVALID_HANDLE_VALUE {
            unsafe {
                let _ = CloseHandle(handle);
            }
        }
    }

    let spec_path = match first.as_deref() {
        Some("--spec") => match args.next() {
            Some(path) => PathBuf::from(path),
            None => fail("--spec 缺少路径"),
        },
        _ => fail("用法：dsh-sandbox --spec <file> | --check"),
    };
    let text = std::fs::read_to_string(&spec_path).unwrap_or_else(|error| {
        fail(&format!("无法读取 spec：{error}"));
    });
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let _ = std::fs::remove_file(&spec_path);
    let value: serde_json::Value = serde_json::from_str(text).unwrap_or_else(|error| {
        fail(&format!("spec 不是合法 JSON：{error}"));
    });
    let cwd = value
        .get("cwd")
        .and_then(|item| item.as_str())
        .unwrap_or_default()
        .to_string();
    let workspace = value
        .get("workspace")
        .and_then(|item| item.as_str())
        .unwrap_or_default()
        .to_string();
    let readonly = value
        .get("readonly")
        .and_then(|item| item.as_bool())
        .unwrap_or(false);
    let file = value
        .get("file")
        .and_then(|item| item.as_str())
        .unwrap_or_default()
        .to_string();
    if cwd.is_empty() || file.is_empty() {
        fail("spec 缺少 cwd 或 file");
    }
    let file_args: Vec<String> = value
        .get("args")
        .and_then(|item| item.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    // A private temp folder the low-integrity child can write. The engine points
    // TEMP/TMP here for the helper.
    if !workspace.is_empty() {
        let temp = PathBuf::from(&workspace).join(".dsh-flow").join("sandbox-tmp");
        let _ = std::fs::create_dir_all(&temp);
        mark_integrity(&temp, "L", false);
        // Read-only restores a Medium workspace that an earlier workspace-mode
        // run lowered; a workspace never lowered needs no label change.
        let marker = PathBuf::from(&workspace).join(".dsh-flow").join("integrity");
        let current = std::fs::read_to_string(&marker).unwrap_or_default();
        if !readonly || current.trim() == "L" {
            mark_integrity(&PathBuf::from(&workspace), if readonly { "M" } else { "L" }, true);
        }
    }

    let command_line: String = std::iter::once(quote_arg(&file))
        .chain(file_args.iter().map(|arg| quote_arg(arg)))
        .collect::<Vec<_>>()
        .join(" ");
    let mut command_line = wide(&command_line);
    let cwd_wide = wide(&cwd);
    let file_wide = wide(&file);

    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        let desired = TOKEN_QUERY
            | TOKEN_DUPLICATE
            | TOKEN_ASSIGN_PRIMARY
            | TOKEN_ADJUST_DEFAULT
            | TOKEN_ADJUST_SESSIONID;
        if OpenProcessToken(GetCurrentProcess(), desired, &mut token) == 0 {
            fail("无法打开当前进程令牌");
        }
        let mut restricted: HANDLE = std::ptr::null_mut();
        if CreateRestrictedToken(
            token,
            DISABLE_MAX_PRIVILEGE,
            0,
            std::ptr::null(),
            0,
            std::ptr::null(),
            0,
            std::ptr::null(),
            &mut restricted,
        ) == 0
        {
            close(token);
            fail("无法创建受限令牌");
        }
        close(token);

        // Low integrity: writes to Medium/High objects are denied; reads and
        // network remain available.
        let mut low_sid: PSID = std::ptr::null_mut();
        let low_text = wide("S-1-16-4096");
        if ConvertStringSidToSidW(low_text.as_ptr(), &mut low_sid) == 0 {
            close(restricted);
            fail("无法解析低完整性 SID");
        }
        let label = TOKEN_MANDATORY_LABEL {
            Label: SID_AND_ATTRIBUTES {
                Sid: low_sid,
                Attributes: SE_GROUP_INTEGRITY as u32,
            },
        };
        if SetTokenInformation(
            restricted,
            TokenIntegrityLevel,
            &label as *const _ as *const c_void,
            std::mem::size_of::<TOKEN_MANDATORY_LABEL>() as u32,
        ) == 0
        {
            close(restricted);
            fail("无法把令牌降到低完整性");
        }

        let stdin = GetStdHandle(STD_INPUT_HANDLE);
        let stdout = GetStdHandle(STD_OUTPUT_HANDLE);
        let stderr = GetStdHandle(STD_ERROR_HANDLE);
        let mut startup: STARTUPINFOW = std::mem::zeroed();
        startup.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        startup.dwFlags = STARTF_USESTDHANDLES;
        startup.hStdInput = stdin;
        startup.hStdOutput = stdout;
        startup.hStdError = stderr;
        let mut process: PROCESS_INFORMATION = std::mem::zeroed();
        let created = CreateProcessAsUserW(
            restricted,
            file_wide.as_ptr(),
            command_line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
            CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT | CREATE_SUSPENDED,
            std::ptr::null(),
            cwd_wide.as_ptr(),
            &startup,
            &mut process,
        );
        if created == 0 {
            close(restricted);
            fail("CreateProcessAsUser 失败（Windows 拒绝受限令牌启动）");
        }

        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() || job == INVALID_HANDLE_VALUE {
            TerminateProcess(process.hProcess, 125);
            close(restricted);
            fail("无法创建作业对象");
        }
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &limits as *const _ as *const c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        ) == 0
            || AssignProcessToJobObject(job, process.hProcess) == 0
        {
            TerminateProcess(process.hProcess, 125);
            close(job);
            close(restricted);
            fail("无法把进程放进作业对象");
        }
        let _ = ResumeThread(process.hThread);
        let wait = WaitForSingleObject(process.hProcess, INFINITE);
        let mut code = 1u32;
        let _ = GetExitCodeProcess(process.hProcess, &mut code);
        close(process.hThread);
        close(process.hProcess);
        // Closing the job kills anything the command left behind.
        close(job);
        close(restricted);
        if wait != WAIT_OBJECT_0 {
            std::process::exit(125);
        }
        std::process::exit(code as i32);
    }
}