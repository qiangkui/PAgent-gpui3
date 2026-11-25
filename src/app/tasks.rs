//! Background tasks: the records the engine's `shell` tool leaves behind, the
//! panel that lists them, and ending one by its pid.
//!
//! The engine writes one `<id>.json` and one `<id>.log` per task under the
//! conversation's `.dsh-flow/tasks` folder. The panel reads those files, so it
//! shows exactly what the `task` tool would see, even for a task started before
//! this window opened.

use super::*;
use std::io::{Read, Seek, SeekFrom};

/// Lines of a task's log the panel keeps.
const TASK_LOG_LINES: usize = 400;
/// Bytes read from the end of a log; enough for the lines above in any script.
const TASK_LOG_BYTES: u64 = 256 * 1024;
/// Tasks listed at once; older ones stay on disk.
const TASK_ROWS: usize = 30;

/// Folder under a conversation that holds one record and one log per task.
pub(crate) fn tasks_dir(chat_dir: &Path) -> PathBuf {
    chat_dir.join(".dsh-flow").join("tasks")
}

/// Whether a task is still running, already exited, or was ended by hand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TaskState {
    Running,
    Exited,
    Stopped,
}

impl TaskState {
    /// What the row says.
    fn label(self) -> &'static str {
        match self {
            TaskState::Running => "运行中",
            TaskState::Exited => "已退出",
            TaskState::Stopped => "已结束",
        }
    }

    /// Dot colour in the task row.
    fn color(self, palette: Palette) -> u32 {
        match self {
            TaskState::Running => palette.running,
            TaskState::Exited => palette.faint,
            TaskState::Stopped => palette.warn,
        }
    }
}

/// One background task as recorded by the engine.
#[derive(Clone)]
pub(crate) struct BackgroundTask {
    /// `task-1`, unique inside the conversation.
    pub(crate) id: String,
    /// The command line that was approved.
    pub(crate) command: String,
    /// Working directory, absolute.
    pub(crate) cwd: String,
    /// Process id, or zero when it never started.
    pub(crate) pid: u32,
    /// ISO timestamp written by the engine.
    pub(crate) started_at: String,
    /// Where its stdout and stderr go.
    pub(crate) log: PathBuf,
    /// True once the record was marked as ended.
    pub(crate) stopped: bool,
    /// What the row shows.
    pub(crate) state: TaskState,
}

/// Read every task record in one conversation, oldest id first.
pub(crate) fn load_tasks(chat_dir: &Path) -> Vec<BackgroundTask> {
    let dir = tasks_dir(chat_dir);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut tasks: Vec<BackgroundTask> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
        .filter_map(|text| parse_task(&text, &dir))
        .collect();
    for task in &mut tasks {
        task.state = if task.stopped {
            TaskState::Stopped
        } else if is_running(task.pid) {
            TaskState::Running
        } else {
            TaskState::Exited
        };
    }
    tasks.sort_by_key(|task| (task_number(&task.id), task.id.clone()));
    tasks
}

/// One record, or `None` when the file is not a task at all.
fn parse_task(text: &str, dir: &Path) -> Option<BackgroundTask> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let id = value.get("id")?.as_str()?.trim().to_string();
    let command = value.get("command")?.as_str()?.to_string();
    if id.is_empty() {
        return None;
    }
    let text_of = |key: &str| {
        value
            .get(key)
            .and_then(|field| field.as_str())
            .unwrap_or_default()
            .to_string()
    };
    let log = PathBuf::from(text_of("log"));
    Some(BackgroundTask {
        id: id.clone(),
        command,
        cwd: text_of("cwd"),
        pid: value
            .get("pid")
            .and_then(|field| field.as_u64())
            .unwrap_or(0)
            .min(u64::from(u32::MAX)) as u32,
        started_at: text_of("startedAt"),
        log: if log.as_os_str().is_empty() {
            dir.join(format!("{id}.log"))
        } else {
            log
        },
        stopped: value
            .get("stopped")
            .and_then(|field| field.as_bool())
            .unwrap_or(false),
        state: TaskState::Exited,
    })
}

/// The number in `task-12`; unknown ids sort before the numbered ones.
fn task_number(id: &str) -> u64 {
    id.strip_prefix("task-")
        .and_then(|rest| rest.parse().ok())
        .unwrap_or(0)
}

/// The last lines of one log, read from the end so a long log stays cheap.
pub(crate) fn task_log_tail(log: &Path, lines: usize) -> Vec<String> {
    let mut text = String::new();
    let Ok(mut file) = std::fs::File::open(log) else {
        return Vec::new();
    };
    let length = file.metadata().map_or(0, |meta| meta.len());
    let partial = length > TASK_LOG_BYTES;
    if partial {
        let _ = file.seek(SeekFrom::Start(length - TASK_LOG_BYTES));
    }
    let mut bytes = Vec::new();
    let _ = file.read_to_end(&mut bytes);
    text.push_str(&String::from_utf8_lossy(&bytes));
    let all: Vec<&str> = text.lines().collect();
    // The window starts mid-line when it was cut; that first line is a
    // fragment rather than the start of a line.
    let all = if partial && all.len() > 1 {
        &all[1..]
    } else {
        &all[..]
    };
    all[all.len().saturating_sub(lines)..]
        .iter()
        .map(|line| line.trim_end_matches('\r').to_string())
        .collect()
}

/// Whether a pid still points at a live process.
///
/// The panel asks once per second, so it must not spawn a process itself:
/// Windows answers through `OpenProcess` / `GetExitCodeProcess` (the same
/// question `process.kill(pid, 0)` asks on the engine side).
pub(crate) fn is_running(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(windows)]
    {
        const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
        const STILL_ACTIVE: u32 = 259;

        const ERROR_ACCESS_DENIED: u32 = 5;

        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn OpenProcess(access: u32, inherit: i32, pid: u32) -> isize;
            fn GetExitCodeProcess(process: isize, code: *mut u32) -> i32;
            fn CloseHandle(handle: isize) -> i32;
            fn GetLastError() -> u32;
        }

        // SAFETY: the handle is only passed back to the same library and is
        // closed before returning; the code pointer is a live local.
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if handle == 0 {
            // A process owned by another user cannot be opened, but its pid is
            // still alive; anything else (invalid parameter) means it is gone.
            return unsafe { GetLastError() } == ERROR_ACCESS_DENIED;
        }
        let mut code = 0_u32;
        let ok = unsafe { GetExitCodeProcess(handle, &mut code) };
        unsafe { CloseHandle(handle) };
        ok != 0 && code == STILL_ACTIVE
    }
    #[cfg(not(windows))]
    {
        let mut command = std::process::Command::new("ps");
        command.args(["-p", &pid.to_string(), "-o", "pid="]);
        command.output().is_ok_and(|output| {
            output.status.success() && !String::from_utf8_lossy(&output.stdout).trim().is_empty()
        })
    }
}

/// `2026-11-22T02:05:48.879Z` as Unix milliseconds.
///
/// The engine writes an ISO timestamp just after spawning the wrapper, so the
/// recorded time is within a second of the process's real creation time.
fn started_at_ms(text: &str) -> Option<u64> {
    let bytes = text.as_bytes();
    if bytes.len() < 19 || bytes[4] != b'-' || bytes[7] != b'-' || bytes[10] != b'T' {
        return None;
    }
    if bytes[13] != b':' || bytes[16] != b':' {
        return None;
    }
    let number = |start: usize, end: usize| text.get(start..end)?.parse::<u32>().ok();
    let year = number(0, 4)? as i64;
    let month = number(5, 7)?;
    let day = number(8, 10)?;
    let hour = number(11, 13)?;
    let minute = number(14, 16)?;
    let second = number(17, 19)?;
    if month == 0 || month > 12 || day == 0 || day > 31 || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let millis = text
        .get(19..)
        .filter(|rest| rest.starts_with('.'))
        .and_then(|rest| rest.get(1..4))
        .and_then(|digits| digits.parse::<u32>().ok())
        .unwrap_or(0);
    let days = days_from_civil(year, month, day);
    let seconds = days
        .saturating_mul(86_400)
        .saturating_add(hour as i64 * 3_600)
        .saturating_add(minute as i64 * 60)
        .saturating_add(second as i64);
    u64::try_from(seconds.saturating_mul(1000).saturating_add(millis as i64)).ok()
}

/// Days since 1970-01-01 for a civil date, as the engine's ISO stamps use.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let shifted_month = i64::from(month) + 9;
    let day_of_year = (153 * (shifted_month % 12) + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// When Windows created a pid currently points at, in Unix milliseconds.
#[cfg(windows)]
fn process_started_ms(pid: u32) -> Option<u64> {
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;

    #[repr(C)]
    #[derive(Default)]
    struct FileTime {
        low: u32,
        high: u32,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> isize;
        fn GetProcessTimes(
            process: isize,
            creation: *mut FileTime,
            exit: *mut FileTime,
            kernel: *mut FileTime,
            user: *mut FileTime,
        ) -> i32;
        fn CloseHandle(handle: isize) -> i32;
    }

    // SAFETY: one live handle is opened for the query, four live `FILETIME`
    // outputs are passed, and the handle is closed before returning.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle == 0 {
        return None;
    }
    let mut creation = FileTime::default();
    let mut exit = FileTime::default();
    let mut kernel = FileTime::default();
    let mut user = FileTime::default();
    let ok = unsafe { GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) };
    unsafe { CloseHandle(handle) };
    if ok == 0 {
        return None;
    }
    // A FILETIME counts 100 ns intervals from 1601-01-01; Unix starts 11644473600
    // seconds later. Convert before dividing so the epoch offset is not rounded.
    let ticks = (u64::from(creation.high) << 32) | u64::from(creation.low);
    let unix_100ns = ticks.checked_sub(116_444_736_000_000_000)?;
    Some(unix_100ns / 10_000)
}

#[cfg(not(windows))]
fn process_started_ms(_pid: u32) -> Option<u64> {
    None
}

/// Refuse to end a pid that no longer belongs to the task that recorded it.
///
/// A background task outlives the engine, so its numeric pid is all that is
/// left; Windows can reuse that number for an unrelated process. Comparing the
/// process creation time with the record's start time closes that gap. A small
/// tolerance covers the moment between spawning the wrapper and writing the
/// record.
fn verify_task_process(task: &BackgroundTask) -> Result<(), String> {
    if task.pid == 0 {
        return Err(format!("任务 {} 没有记录 pid", task.id));
    }
    #[cfg(windows)]
    {
        let expected = started_at_ms(&task.started_at)
            .ok_or_else(|| format!("任务 {} 的启动时间无法解析", task.id))?;
        let actual = process_started_ms(task.pid).ok_or_else(|| {
            format!(
                "无法确认 pid {} 的启动时间，为安全起见没有结束{}",
                task.pid, task.id
            )
        })?;
        const TOLERANCE_MS: u64 = 120_000;
        if expected.abs_diff(actual) > TOLERANCE_MS {
            return Err(format!(
                "pid {} 已被其他进程占用（启动时间不匹配），没有结束{}",
                task.pid, task.id
            ));
        }
    }
    Ok(())
}

/// End a task's whole process tree.
pub(crate) fn kill_task_tree(pid: u32) -> Result<(), String> {
    if pid == 0 {
        return Ok(());
    }
    #[cfg(windows)]
    let mut command = {
        let mut command = std::process::Command::new("taskkill");
        command.args(["/PID", &pid.to_string(), "/T", "/F"]);
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
        command
    };
    #[cfg(windows)]
    {
        let output = command
            .output()
            .map_err(|error| format!("无法结束进程 {pid}：{error}"))?;
        if output.status.success() {
            Ok(())
        } else if !is_running(pid) {
            // The helper can exit on its own between the spawn and the kill, and
            // taskkill then answers 128 for a pid it cannot find. The caller only
            // wants the tree gone, so a process that already ended is fine.
            Ok(())
        } else {
            let message = String::from_utf8_lossy(&output.stderr).trim().to_string();
            Err(if message.is_empty() {
                format!("结束进程 {pid} 失败（退出码 {:?}）", output.status.code())
            } else {
                message
            })
        }
    }
    #[cfg(not(windows))]
    {
        // A background task leads its own group; a plain child does not, so try
        // the group first and fall back to the pid alone.
        let mut group_command = std::process::Command::new("kill");
        group_command.args(["-9", &format!("-{pid}")]);
        let group = group_command.output();
        if group.is_ok_and(|output| output.status.success()) {
            return Ok(());
        }
        let single = std::process::Command::new("kill")
            .args(["-9", &pid.to_string()])
            .output()
            .map_err(|error| format!("无法结束进程 {pid}：{error}"))?;
        if single.status.success() {
            return Ok(());
        }
        let message = String::from_utf8_lossy(&single.stderr).trim().to_string();
        Err(if message.is_empty() {
            format!("结束进程 {pid} 失败（退出码 {:?}）", single.status.code())
        } else {
            message
        })
    }
}

/// Mark one record as ended, so the `task` tool and the panel agree.
pub(crate) fn mark_task_stopped(chat_dir: &Path, id: &str) -> Result<(), String> {
    let file = task_file(chat_dir, id)?;
    let text = std::fs::read_to_string(&file)
        .map_err(|error| format!("无法读取 {}：{error}", file.display()))?;
    let mut value: serde_json::Value = serde_json::from_str(&text)
        .map_err(|error| format!("{} 不是有效的任务记录：{error}", file.display()))?;
    let Some(object) = value.as_object_mut() else {
        return Err(format!("{} 不是有效的任务记录", file.display()));
    };
    if object.get("stopped").and_then(|field| field.as_bool()) == Some(true) {
        return Ok(());
    }
    object.insert("stopped".to_string(), serde_json::Value::Bool(true));
    let body = serde_json::to_string_pretty(&value)
        .map_err(|error| format!("无法写出任务记录：{error}"))?;
    crate::store::write_atomic(&file, &body)
        .map_err(|error| format!("无法保存 {}：{error}", file.display()))
}

/// A record's path, refusing ids that could leave the tasks folder.
fn task_file(chat_dir: &Path, id: &str) -> Result<PathBuf, String> {
    if id.is_empty() || id.contains(['/', '\\']) || id.contains("..") {
        return Err(format!("任务 id 不合法：{id}"));
    }
    Ok(tasks_dir(chat_dir).join(format!("{id}.json")))
}

/// `2026-11-22T02:05:48.879Z` → `2026-11-22 02:05:48`, as the engine shows it.
fn task_when(started_at: &str) -> String {
    let text = started_at.replace('T', " ");
    text.chars().take(19).collect()
}

impl DshFlowApp {
    /// Keep the panel's list fresh while the window is alive.
    pub(crate) fn spawn_tasks_pump(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let keep_going = this
                    .update(cx, |view, cx| {
                        // Also read while the task tab is closed, so its
                        // running dot can mark a live task without a click.
                        let before: Vec<(String, TaskState)> = view
                            .tasks
                            .iter()
                            .map(|task| (task.id.clone(), task.state))
                            .collect();
                        view.refresh_tasks();
                        let after: Vec<(String, TaskState)> = view
                            .tasks
                            .iter()
                            .map(|task| (task.id.clone(), task.state))
                            .collect();
                        if view.shown_chat_tab() == ChatTab::Tasks || before != after {
                            cx.notify();
                        }
                        true
                    })
                    .unwrap_or(false);
                if !keep_going {
                    break;
                }
            }
        })
        .detach();
    }

    /// Re-read the records and the selected log.
    pub(crate) fn refresh_tasks(&mut self) {
        let dir = self.chat_dir(self.active_chat);
        let switched = self.tasks_dir != dir;
        if switched {
            self.tasks_dir = dir.clone();
            self.task_selected = None;
            self.task_log.clear();
            self.task_stop_confirm = None;
        }
        let follow = self.task_log_at_bottom();
        self.tasks = load_tasks(&dir);
        if let Some(id) = self.task_selected.clone() {
            match self.tasks.iter().find(|task| task.id == id) {
                Some(task) => {
                    self.task_log = task_log_tail(&task.log, TASK_LOG_LINES);
                    if follow || switched {
                        self.task_log_scroll.scroll_to_bottom();
                    }
                }
                None => {
                    self.task_selected = None;
                    self.task_log.clear();
                }
            }
        }
        if let Some(id) = &self.task_stop_confirm {
            let still_running = self
                .tasks
                .iter()
                .any(|task| task.id == *id && task.state == TaskState::Running);
            if !still_running {
                self.task_stop_confirm = None;
            }
        }
    }

    /// Whether the log box is scrolled to its end.
    fn task_log_at_bottom(&self) -> bool {
        let offset = self.task_log_scroll.offset();
        let max = self.task_log_scroll.max_offset();
        max.y + offset.y <= px(24.)
    }

    /// Show one task's log, or fold the open one away.
    pub(crate) fn select_task(&mut self, id: String, cx: &mut Context<Self>) {
        if self.task_selected.as_deref() == Some(id.as_str()) {
            self.task_selected = None;
            self.task_log.clear();
        } else {
            self.task_stop_confirm = None;
            self.task_log = self
                .tasks
                .iter()
                .find(|task| task.id == id)
                .map_or_else(Vec::new, |task| task_log_tail(&task.log, TASK_LOG_LINES));
            self.task_selected = Some(id);
            self.task_log_scroll.scroll_to_bottom();
        }
        cx.notify();
    }

    /// End a task: the first press asks, the second kills the tree and marks the
    /// record so the `task` tool reports the same state.
    pub(crate) fn stop_task(&mut self, id: String, cx: &mut Context<Self>) {
        if self.task_stop_confirm.as_deref() != Some(id.as_str()) {
            self.task_stop_confirm = Some(id);
            cx.notify();
            return;
        }
        self.task_stop_confirm = None;
        let dir = self.chat_dir(self.active_chat);
        let mut failure = None;
        let mut settled = true;
        if let Some(task) = self.tasks.iter().find(|task| task.id == id).cloned() {
            if task.state == TaskState::Running && is_running(task.pid) {
                // The pid is only meaningful while it still names the process
                // this record started; Windows reuses numbers.
                if let Err(error) = verify_task_process(&task) {
                    failure = Some(format!("结束 {} 失败：{error}", task.id));
                    settled = false;
                } else if let Err(error) = kill_task_tree(task.pid) {
                    failure = Some(format!("结束 {} 失败：{error}", task.id));
                    settled = false;
                }
            }
            if settled && let Err(error) = mark_task_stopped(&dir, &task.id) {
                failure = failure.or(Some(error));
            }
        } else if let Err(error) = mark_task_stopped(&dir, &id) {
            failure = Some(error);
        }
        if let Some(text) = failure {
            self.banner = Some(Banner::error(text));
        }
        self.refresh_tasks();
        cx.notify();
    }

    /// The task tab: the conversation's background tasks, filling the pane that
    /// the transcript and the context breakdown share.
    ///
    /// It used to be a small floating window opened from a header icon; the tab
    /// is now the only entrance, so the list gets the full width and the log can
    /// be read without a cramped popup.
    pub(crate) fn render_tasks_view(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let running = self
            .tasks
            .iter()
            .filter(|task| task.state == TaskState::Running)
            .count();
        let count = match (self.tasks.len(), running) {
            (0, _) => "没有任务".to_string(),
            (total, 0) => format!("{total} 个"),
            (total, running) => format!("{total} 个 · {running} 个运行中"),
        };

        let mut rows: Vec<AnyElement> = Vec::new();
        if self.tasks.is_empty() {
            rows.push(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .w_full()
                    .min_h(px(0.))
                    .py(px(48.))
                    .px_6()
                    .rounded_lg()
                    .bg(rgb(palette.canvas))
                    .border_1()
                    .border_color(rgb(palette.border))
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(palette.muted))
                            .child("这个对话还没有后台任务"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.faint))
                            .text_center()
                            .whitespace_normal()
                            .child("让助手用 shell 的 background=true 启动一个，它会在回答结束后继续运行。"),
                    )
                    .into_any_element(),
            );
        }
        for task in self.tasks.iter().rev().take(TASK_ROWS) {
            let selected = self.task_selected.as_deref() == Some(task.id.as_str());
            let id = task.id.clone();
            let mut detail: Vec<AnyElement> = Vec::new();
            if selected {
                let stopping = self.task_stop_confirm.as_deref() == Some(task.id.as_str());
                let mut meta = vec![format!("pid {}", task.pid)];
                if !task.cwd.is_empty() {
                    meta.push(task.cwd.clone());
                }
                let when = task_when(&task.started_at);
                if !when.is_empty() {
                    meta.push(when);
                }
                detail.push(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .mt_1()
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette.faint))
                                .child(meta.join(" · ")),
                        )
                        .child(
                            div()
                                .id(SharedString::from(format!("task-log-{}", task.id)))
                                .max_h(px(190.))
                                .overflow_y_scroll()
                                .contained_wheel()
                                .track_scroll(&self.task_log_scroll)
                                .rounded_md()
                                .bg(rgb(palette.background))
                                .p_2()
                                .font_family(MONO_FONT)
                                .text_size(px(11.))
                                .text_color(rgb(palette.muted))
                                .whitespace_normal()
                                .children(if self.task_log.is_empty() {
                                    vec![
                                        div()
                                            .text_color(rgb(palette.faint))
                                            .child("（还没有输出）")
                                            .into_any_element(),
                                    ]
                                } else {
                                    self.task_log
                                        .iter()
                                        .map(|line| {
                                            div()
                                                .whitespace_normal()
                                                .child(line.clone())
                                                .into_any_element()
                                        })
                                        .collect()
                                }),
                        )
                        .when(task.state == TaskState::Running, |this| {
                            let stop_id = task.id.clone();
                            this.child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .mt_1()
                                    .child(
                                        div()
                                            .id(SharedString::from(format!(
                                                "task-stop-{}",
                                                task.id
                                            )))
                                            .px_2()
                                            .py_1()
                                            .rounded_md()
                                            .border_1()
                                            .border_color(rgb(if stopping {
                                                palette.failed
                                            } else {
                                                palette.border
                                            }))
                                            .bg(rgb(if stopping {
                                                palette.panel_soft
                                            } else {
                                                palette.canvas
                                            }))
                                            .text_xs()
                                            .text_color(rgb(if stopping {
                                                palette.failed
                                            } else {
                                                palette.muted
                                            }))
                                            .cursor_pointer()
                                            .hover(move |style| {
                                                style
                                                    .border_color(rgb(palette.failed))
                                                    .text_color(rgb(palette.failed))
                                            })
                                            .on_click(cx.listener(move |view, _, _, cx| {
                                                view.stop_task(stop_id.clone(), cx);
                                            }))
                                            .child(if stopping {
                                                "确认结束？"
                                            } else {
                                                "结束"
                                            }),
                                    )
                                    .when(stopping, |this| {
                                        this.child(
                                            div()
                                                .text_xs()
                                                .text_color(rgb(palette.faint))
                                                .child("会按 pid 结束整棵进程树"),
                                        )
                                    }),
                            )
                        })
                        .into_any_element(),
                );
            }
            rows.push(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .w_full()
                    .min_w(px(0.))
                    .p_3()
                    .rounded_lg()
                    .bg(rgb(if selected {
                        palette.panel_soft
                    } else {
                        palette.canvas
                    }))
                    .border_1()
                    .border_color(rgb(if selected {
                        palette.accent
                    } else {
                        palette.border
                    }))
                    .hover(move |style| style.bg(rgb(palette.panel_soft)))
                    .child(
                        // Only the summary folds the log open; pressing inside
                        // the detail (the log or its buttons) must not.
                        div()
                            .id(SharedString::from(format!("task-row-{}", task.id)))
                            .flex()
                            .flex_col()
                            .gap_1()
                            .cursor_pointer()
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.select_task(id.clone(), cx);
                            }))
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .child(status_dot(task.state.color(palette)))
                                    .child(
                                        div()
                                            .flex_shrink_0()
                                            .text_xs()
                                            .text_color(rgb(palette.text))
                                            .child(task.id.clone()),
                                    )
                                    .child(
                                        div()
                                            .flex_shrink_0()
                                            .text_xs()
                                            .text_color(rgb(task.state.color(palette)))
                                            .child(task.state.label()),
                                    )
                                    .child(div().flex_1().min_w(px(0.)))
                                    .child(
                                        div()
                                            .flex_shrink_0()
                                            .text_xs()
                                            .text_color(rgb(palette.faint))
                                            .child(format!("pid {}", task.pid)),
                                    ),
                            )
                            .child(
                                div()
                                    .min_w(px(0.))
                                    .truncate()
                                    .font_family(MONO_FONT)
                                    .text_size(px(11.))
                                    .text_color(rgb(palette.muted))
                                    .child(task.command.clone()),
                            ),
                    )
                    .children(detail)
                    .into_any_element(),
            );
        }

        div()
            .id("tasks-view")
            .flex()
            .flex_col()
            .flex_1()
            .w_full()
            .min_h(px(0.))
            .min_w(px(0.))
            .px(px(self.chat_gutter))
            .pt_3()
            .pb_3()
            .gap_3()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .flex_shrink_0()
                    .w_full()
                    .min_w(px(0.))
                    .px_1()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(rgb(palette.text))
                            .child("后台任务"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(if running > 0 {
                                palette.running
                            } else {
                                palette.faint
                            }))
                            .child(count),
                    )
                    .child(div().flex_1().min_w(px(0.)))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.faint))
                            .child("每秒刷新"),
                    ),
            )
            .child(
                div()
                    .id("task-list")
                    .flex()
                    .flex_col()
                    .gap_2()
                    .flex_1()
                    .min_h(px(0.))
                    .min_w(px(0.))
                    .w_full()
                    .overflow_y_scroll()
                    .children(rows),
            )
            .when(!self.tasks.is_empty(), |this| {
                this.child(
                    div()
                        .flex_shrink_0()
                        .px_1()
                        .text_xs()
                        .text_color(rgb(palette.faint))
                        .whitespace_normal()
                        .child("点一个任务看日志尾部；「结束」会结束整棵进程树。"),
                )
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh folder for one test, removed by the caller.
    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dsh-tasks-{}-{name}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |since| since.as_nanos())
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(tasks_dir(&dir)).unwrap();
        dir
    }

    #[test]
    fn records_are_read_with_their_state_and_order() {
        let dir = temp_dir("records");
        let folder = tasks_dir(&dir);
        std::fs::write(
            folder.join("task-10.json"),
            r#"{"id":"task-10","command":"npm run dev","cwd":"C:\\w","pid":9,"startedAt":"2026-11-22T02:05:48.879Z","log":"C:\\w\\task-10.log"}"#,
        )
        .unwrap();
        std::fs::write(
            folder.join("task-2.json"),
            r#"{"id":"task-2","command":"node serve.js","cwd":"C:\\w","pid":0,"startedAt":"2026-11-22T01:00:00.000Z","log":"","stopped":true}"#,
        )
        .unwrap();
        std::fs::write(folder.join("notes.txt"), "not a task").unwrap();

        let tasks = load_tasks(&dir);
        assert_eq!(tasks.len(), 2, "only *.json records count");
        assert_eq!(tasks[0].id, "task-2", "task-10 sorts after task-2");
        assert_eq!(tasks[0].state, TaskState::Stopped);
        assert_eq!(tasks[1].id, "task-10");
        assert_eq!(tasks[1].state, TaskState::Exited, "pid 9 is not running");
        assert_eq!(tasks[1].cwd, "C:\\w");
        assert_eq!(task_when(&tasks[1].started_at), "2026-11-22 02:05:48");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_log_path_falls_back_to_the_task_folder() {
        let dir = temp_dir("fallback");
        std::fs::write(
            tasks_dir(&dir).join("task-1.json"),
            r#"{"id":"task-1","command":"x","pid":0}"#,
        )
        .unwrap();
        let tasks = load_tasks(&dir);
        assert_eq!(tasks[0].log, tasks_dir(&dir).join("task-1.log"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_log_tail_is_capped_and_reads_utf8_lossily() {
        let dir = temp_dir("tail");
        let log = dir.join("task-1.log");
        let mut body = String::new();
        for line in 0..500 {
            body.push_str(&format!("line {line}\r\n"));
        }
        std::fs::write(&log, body).unwrap();
        let tail = task_log_tail(&log, 3);
        assert_eq!(tail, vec!["line 497", "line 498", "line 499"]);
        assert!(task_log_tail(&dir.join("missing.log"), 10).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn marking_stopped_is_idempotent_and_keeps_the_other_fields() {
        let dir = temp_dir("stopped");
        let file = tasks_dir(&dir).join("task-3.json");
        std::fs::write(
            &file,
            r#"{"id":"task-3","command":"ping","cwd":"C:\\w","pid":7,"startedAt":"t"}"#,
        )
        .unwrap();
        mark_task_stopped(&dir, "task-3").unwrap();
        mark_task_stopped(&dir, "task-3").unwrap();
        let value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(value["stopped"], serde_json::Value::Bool(true));
        assert_eq!(
            value["command"],
            serde_json::Value::String("ping".to_string())
        );
        assert_eq!(value["pid"], serde_json::Value::from(7));
        assert!(mark_task_stopped(&dir, "../escape").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn liveness_answers_for_this_process_and_for_zero() {
        assert!(!is_running(0), "a task that never started is not running");
        assert!(is_running(std::process::id()), "the test process is alive");
        // A pid above the platform's range is never alive; 0 is the documented
        // "never started" value, so a far-away pid only checks the failure path.
        assert!(!is_running(u32::MAX - 1));
    }

    #[test]
    fn a_live_child_reads_as_running_and_can_be_ended() {
        let dir = temp_dir("live");
        #[cfg(windows)]
        let child = std::process::Command::new("cmd")
            .args(["/C", "ping", "-n", "30", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        #[cfg(not(windows))]
        let child = std::process::Command::new("sleep")
            .arg("30")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        let mut child = child.expect("start a helper process");
        let pid = child.id();
        std::fs::write(
            tasks_dir(&dir).join("task-1.json"),
            format!(r#"{{"id":"task-1","command":"helper","pid":{pid},"startedAt":"t"}}"#),
        )
        .unwrap();

        let tasks = load_tasks(&dir);
        assert_eq!(tasks[0].state, TaskState::Running, "pid {pid} is alive");

        if let Err(error) = kill_task_tree(pid) {
            panic!("the tree ends: {error}");
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while is_running(pid) && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(!is_running(pid), "pid {pid} is gone");
        let _ = child.wait();

        // The record still says nothing about stopping, so the row reports the
        // process, not the wish: it is now merely exited.
        assert_eq!(load_tasks(&dir)[0].state, TaskState::Exited);
        mark_task_stopped(&dir, "task-1").unwrap();
        assert_eq!(load_tasks(&dir)[0].state, TaskState::Stopped);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn engine_start_times_parse_to_unix_milliseconds() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 1, 1), 10_957);
        assert_eq!(started_at_ms("1970-01-01T00:00:00.000Z"), Some(0));
        assert_eq!(
            started_at_ms("2000-01-01T00:00:00.000Z"),
            Some(946_684_800_000)
        );
        assert_eq!(
            started_at_ms("2026-11-22T02:05:48.879Z"),
            Some(1_795_313_148_879)
        );
        // Half-written or non-ISO stamps are refused rather than guessed at.
        assert_eq!(started_at_ms("2026-11-22"), None);
        assert_eq!(started_at_ms("not a date"), None);
    }

    #[cfg(windows)]
    #[test]
    fn reads_this_process_creation_time() {
        // The FFI path used before ending a task must answer for a live pid.
        assert!(process_started_ms(std::process::id()).is_some());
    }
}
