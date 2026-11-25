//! Bridge to the engine's `ai` command.
//!
//! The window never talks to a model itself: it runs `node src/cli.ts ai`, writes
//! one JSON request to stdin and reads one JSON object per line from stdout —
//! `{type:"step",…}` for each tool the assistant runs, then the answer.
//! Credentials, retries and schema checks stay inside the engine (`README.md`「工作流里的模型调用」；
//! this module only knows about one request in flight and hands the settings over.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::config::Connection;
#[cfg(test)]
use crate::config::{DEFAULT_MAX_CALLS, DEFAULT_MAX_ROUNDS};

/// Hide the console window a child process would otherwise flash.
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Engine processes still running, so closing the window kills them.
///
/// The reader thread owns the only other handle and sits in `wait()`, so the
/// registry has to keep its own reference rather than asking the request.
static CHILDREN: Mutex<Vec<(u32, Arc<Mutex<Option<Child>>>)>> = Mutex::new(Vec::new());

/// Lock a shared slot, keeping it usable after a thread panicked.
///
/// A poisoned lock is not a reason to lose the window: the data inside is still
/// a valid list or slot, and panicking here would take the whole app with it.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Remember a running child, keyed by its process id.
fn register_child(pid: u32, slot: Arc<Mutex<Option<Child>>>) {
    lock(&CHILDREN).push((pid, slot));
}

/// Drop the registry entry once the process has been reaped.
fn forget_child(pid: u32) {
    lock(&CHILDREN).retain(|(other, _)| *other != pid);
}

/// Kill one engine process and, on Windows, everything it started.
///
/// The engine may have a `shell` command running under it; killing only `node`
/// would leave that command behind. Unix relies on the process group the engine
/// was spawned into, so descendants are reached there too.
fn kill_tree(pid: u32) {
    if pid == 0 {
        return;
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .creation_flags(CREATE_NO_WINDOW)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = Command::new("kill")
            .args(["-KILL", &format!("-{pid}")])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

/// Stop every engine process still running; called as the window closes.
pub fn kill_children() {
    let children: Vec<(u32, Arc<Mutex<Option<Child>>>)> = {
        let mut children = lock(&CHILDREN);
        std::mem::take(&mut *children)
    };
    for (pid, slot) in children {
        kill_tree(pid);
        if let Ok(mut slot) = slot.lock()
            && let Some(child) = slot.as_mut()
        {
            let _ = child.kill();
        }
    }
}

/// How long a one-shot engine call (a model listing, a compaction) may take
/// before it is killed. A stalled `node` must not leave the caller's state — a
/// pending listing, a busy conversation — stuck for the rest of the session.
pub const ONE_SHOT_TIMEOUT_MS: u64 = 60_000;

/// Wait for a finished child, killing it once `timeout` has passed.
///
/// `Child::wait` cannot time out, so the wait happens on its own thread and the
/// deadline kills the process tree; the thread then ends by itself.
fn wait_with_timeout(child: Child, timeout: Duration) -> Result<std::process::Output, String> {
    let pid = child.id();
    let (sender, result) = mpsc::channel();
    thread::spawn(move || {
        let _ = sender.send(child.wait_with_output());
    });
    match result.recv_timeout(timeout) {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(error)) => Err(format!("等待引擎返回失败：{error}")),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            kill_tree(pid);
            Err(format!(
                "引擎在 {} 秒内没有返回，已结束该进程",
                timeout.as_secs()
            ))
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => Err("引擎线程意外退出".to_string()),
    }
}

/// One message from a request in flight.
pub enum AiEvent {
    /// One tool the assistant ran, before its answer.
    Step {
        /// Tool name, for example `read`.
        tool: String,
        /// The path or pattern the call was about.
        target: String,
        /// False when the tool refused or failed.
        ok: bool,
        /// First line of what the tool returned.
        detail: String,
        /// Arguments as the model wrote them, for the expanded row.
        input: String,
        /// What the tool returned, for the expanded row.
        output: String,
        /// Provider id of the call, linking it to an approval card.
        call: String,
    },
    /// A `shell` command that is about to run, before its first output.
    ToolStart {
        /// Provider id of the call, so the row can be updated in place.
        call: String,
        /// Tool name, normally `shell`.
        tool: String,
        /// The command line or path it is about.
        target: String,
    },
    /// A chunk of `shell` output while the command is still running.
    ToolOutput {
        /// Provider id of the call, so the row can be updated in place.
        call: String,
        /// Tool name, normally `shell`.
        tool: String,
        /// The command line or path it is about.
        target: String,
        /// The output this flush carried.
        text: String,
    },
    /// A fragment of the answer, as it is being written.
    Delta(String),
    /// A fragment of the model's thinking, when it streams one.
    ///
    /// Kept apart from [`Self::Delta`]: thinking is output the bill includes but
    /// never part of the answer, and the transcript shows it in its own block.
    Reasoning(String),
    /// How far the question has got: the round trip that just started, and how
    /// many tool calls have finished.
    ///
    /// Sent before each round so the status strip under the input can count
    /// along with the loop instead of waiting for the answer.
    Progress {
        /// Round trip about to start, 1-based.
        round: u32,
        /// Tool calls finished so far.
        calls: u32,
    },
    /// The tool definitions this request carries, as `(name, schema JSON)`,
    /// plus what the plugins contributed: prompt fragments and load failures.
    ///
    /// Sent once before the first round so the context panel can show how much
    /// of the prompt the definitions take, and the settings page can name a
    /// plugin that would not load.
    Tools {
        /// One entry per tool offered to the model.
        items: Vec<(String, String)>,
        /// Prompt fragments from plugins, as `(plugin id, text)`.
        prompts: Vec<(String, String)>,
        /// Plugins the engine skipped, as `(plugin id, why)`.
        errors: Vec<(String, String)>,
        /// MCP servers the engine skipped, as `(server id, why)`.
        mcp_errors: Vec<(String, String)>,
        /// Tools a plugin registered, as `(plugin id, tool name)`.
        ///
        /// Only the last request's set is kept, the same way prompts and errors
        /// are: it is what the plugin cards count.
        plugin_tools: Vec<(String, String)>,
        /// Tools an MCP server offers, as `(server id, tool name)`.
        mcp_tools: Vec<(String, String)>,
        /// Panels the enabled plugins built for this request, in plugin order.
        panels: Vec<PanelReport>,
    },
    /// The assistant answered.
    Done {
        /// The answer.
        text: String,
        /// Tokens the endpoint billed for the whole question, summed over
        /// every tool round; zero when the endpoint did not say.
        usage: Option<Usage>,
        /// Tokens the last request carried (prompt and answer), which is what
        /// the next question starts from. Zero when unknown.
        context: u64,
        /// Prompt tokens of that question served from the provider's cache,
        /// summed over its rounds; zero when unreported.
        cache_hits: u64,
        /// Model round trips the question took, the answer included. Every
        /// round sends the whole prompt again, so the bill is roughly this many
        /// times the [`Self::context`] figure.
        rounds: u32,
        /// What each round cost, oldest first, as `(prompt, completion, tool
        /// calls)`; empty when the engine did not report them.
        round_tokens: Vec<(u64, u64, u32)>,
        /// Estimated thinking tokens of each round, oldest first: output the
        /// bill includes that no prompt holds. Empty when unreported.
        round_reasoning: Vec<u32>,
        /// What the context compaction did, when it did anything.
        compaction: Option<Compaction>,
        /// The thinking text the engine streamed for the whole question, in
        /// order; empty when the provider sent none. The transcript shows it in
        /// its own block, so it is never mistaken for the answer.
        reasoning: String,
        /// Wall clock the engine spent waiting for model round trips, in
        /// milliseconds; zero when it reported no timing (older engines).
        model_ms: u64,
        /// Wall clock the engine spent running tools, in milliseconds; zero
        /// when it reported no timing.
        tool_ms: u64,
        /// Searches the provider ran on its own side for this question, when
        /// the server tools were on; zero otherwise.
        web_searches: u32,
        /// Pages the provider opened or read for this question, same condition.
        web_fetches: u32,
        /// Raw server-tool calls of every round, for the next question to hand
        /// back; empty when the server tools never ran.
        server_tool_calls: Vec<serde_json::Value>,
    },
    /// A write or command waiting for the person to allow or refuse it.
    Approval {
        /// Provider id of the call, echoed back with the decision.
        call: String,
        /// `write`, `edit` or `shell`.
        tool: String,
        /// The path or command line the call is about.
        target: String,
        /// The same path, kept for the undo write.
        path: String,
        /// The change as a line diff.
        diff: String,
        /// The file before the change, when undo is possible.
        before: String,
        /// The file after the change, for the same reason.
        after: String,
        /// False when the call creates the file.
        existed: bool,
        /// True when undo data came along.
        undoable: bool,
        /// True when the write already proceeded without waiting.
        auto: bool,
        /// Command line for `shell`; empty for a write.
        command: String,
    },
    /// A structured question waiting for one option.
    Question {
        /// Provider id of the call, echoed back with the answer.
        call: String,
        /// What the model asked.
        question: String,
        /// The choices it offered, in order.
        options: Vec<String>,
        /// True when the person may answer with their own words.
        allow_custom: bool,
    },
    /// A plan the model submitted before it executes it.
    Plan {
        /// Provider id of the call, echoed back with the decision.
        call: String,
        /// The plan itself, markdown.
        plan: String,
    },
    /// The conversation's todo list was replaced.
    Todo {
        /// The new list, as `(text, status)`; status is `pending`,
        /// `in_progress` or `completed`.
        items: Vec<(String, String)>,
    },
    /// The request could not be answered; the text is meant for the user.
    Failed(String),
}

/// One panel an enabled plugin built for a request.
///
/// The engine hands over numbers and short text; the window decides how they
/// look, so a panel never carries layout.
#[derive(Clone, Debug, PartialEq)]
pub struct PanelReport {
    /// Stable id the plugin registered.
    pub id: String,
    /// Heading the plugin declared.
    pub title: String,
    /// Plugin folder the panel came from.
    pub plugin: String,
    /// Up to four headline figures, as `(label, value)`.
    pub summary: Vec<(String, String)>,
    /// The sections, in the order the plugin returned them.
    pub sections: Vec<PanelSection>,
}

/// Which shape a panel section is drawn in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelKind {
    /// A label with its text.
    Text,
    /// A label with a bar and a number.
    Bars,
    /// Column headings with rows of cells.
    Table,
}

/// One section of a plugin panel.
#[derive(Clone, Debug, PartialEq)]
pub struct PanelSection {
    /// Heading above the section.
    pub title: String,
    /// The shape it is drawn in.
    pub kind: PanelKind,
    /// Rows for [`PanelKind::Text`] and [`PanelKind::Bars`].
    pub items: Vec<PanelItem>,
    /// Column headings for [`PanelKind::Table`].
    pub columns: Vec<String>,
    /// Cells for [`PanelKind::Table`], one row each.
    pub rows: Vec<Vec<String>>,
}

/// One entry of a panel's list.
#[derive(Clone, Debug, PartialEq)]
pub struct PanelItem {
    /// Row label.
    pub label: String,
    /// Long text shown under the label.
    pub text: String,
    /// The number a bar row is drawn from.
    pub value: Option<f64>,
    /// Short aside next to the number.
    pub note: String,
}

/// Provider token accounting for one answer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    /// Tokens the prompt cost.
    pub prompt: u64,
    /// Tokens the answer cost.
    pub completion: u64,
}

impl Usage {
    /// What the whole request cost.
    pub fn total(&self) -> u64 {
        self.prompt + self.completion
    }
}

/// What the last compaction pass did to one answer, when it did anything.
///
/// The engine compresses a long transcript in layers; this is the engine's own
/// report of which layer ran, so the window can say it above the answer instead
/// of leaving a suddenly shorter context unexplained.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Compaction {
    /// `prune`, `summary`, `drop`, or empty when nothing ran.
    pub layer: String,
    /// Estimated prompt tokens before and after the pass.
    pub before: u64,
    pub after: u64,
    /// Tool results pruned, and messages the last layer replaced.
    pub pruned: u64,
    pub turns: u64,
    /// True when the replacement came from the model.
    pub summarized: bool,
}

impl Compaction {
    /// True when a pass actually changed the transcript.
    pub fn happened(&self) -> bool {
        !self.layer.is_empty() && self.layer != "none"
    }

    /// What the reader is told, or an empty string when nothing happened.
    ///
    /// Names the layer that decided and what it saved, because "上下文被压缩"
    /// alone would read as a warning rather than as the routine trim it is.
    pub fn note(&self) -> String {
        if !self.happened() {
            return String::new();
        }
        let saved = self.before.saturating_sub(self.after);
        match self.layer.as_str() {
            "summary" => format!(
                "上下文接近窗口上限：最早的 {} 条消息已压成摘要（约省 {saved} tokens）。",
                self.turns
            ),
            "drop" => format!(
                "上下文接近窗口上限：摘要调用失败，已省略最早的 {} 条消息（约省 {saved} tokens）。",
                self.turns
            ),
            _ => format!(
                "上下文接近窗口上限：已精简 {} 段过长的工具结果（约省 {saved} tokens）。",
                self.pruned
            ),
        }
    }
}

/// How long one model round may take before the engine aborts it.
///
/// A stalled endpoint would otherwise leave "正在思考" running forever; a slow
/// answer still finishes because every tool round gets its own window.
pub const CHAT_TIMEOUT_MS: u64 = 180_000;

/// Context window used for the "context filled" gauge, when the model is unknown.
pub const DEFAULT_CONTEXT_WINDOW: u64 = 128_000;

/// Approximate context window of a model, in tokens.
///
/// Providers' model listings carry ids but not window sizes, so this is a small
/// name-based table for the conversation header's gauge. It is deliberately
/// conservative: an unknown model gets [`DEFAULT_CONTEXT_WINDOW`], and a wrong
/// guess only shifts a percentage, never a request.
pub fn context_window(model: &str) -> u64 {
    let name = model.to_ascii_lowercase();
    // The 2026 DeepSeek lineup ships a 1M window; the older ids stay at 128K so
    // a conversation already using them keeps its own gauge.
    if name.contains("deepseek-flash")
        || name.contains("deepseek-v4")
        || name.contains("deepseek-v4-pro")
    {
        return 1_000_000;
    }
    if name.contains("gpt-4.1") || name.contains("gemini") {
        return 1_000_000;
    }
    if name.contains("gpt-5") {
        return 400_000;
    }
    if name.contains("kimi") || name.contains("moonshot") {
        return 256_000;
    }
    if name.contains("claude") || name.contains("o3") || name.contains("o4") {
        return 200_000;
    }
    if name.contains("deepseek")
        || name.contains("gpt-4")
        || name.contains("qwen")
        || name.contains("glm")
        || name.contains("llama")
    {
        return 128_000;
    }
    DEFAULT_CONTEXT_WINDOW
}

/// One earlier turn of a conversation, sent back so the assistant remembers it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Turn {
    /// Who said it.
    pub speaker: Speaker,
    /// What was said.
    pub text: String,
    /// Server-tool calls of an assistant turn, raw as the provider sent them.
    ///
    /// The provider restores the pages it read earlier from these; a framework
    /// that drops them still gets an answer, just without that context. Empty
    /// for user turns and for questions that never searched.
    pub server_tool_calls: Vec<serde_json::Value>,
}

/// Who spoke one turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Speaker {
    /// The person at the window.
    User,
    /// The model's answer.
    Assistant,
}

impl Speaker {
    /// The role name the provider expects.
    pub(crate) fn role(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
        }
    }
}

/// One attached file, base64 encoded on the way into the engine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AskAttachment {
    /// Path as attached, shown in provider errors.
    name: String,
    /// MIME type, for example `image/png` or `application/pdf`.
    mime: String,
    /// `image` or `pdf`; decides which content part the engine builds.
    kind: String,
    /// Base64 bytes, without the `data:` prefix.
    data: String,
}

impl AskAttachment {
    /// Encode one image or PDF for the bridge.
    pub fn new(name: &str, mime: &str, kind: &str, bytes: &[u8]) -> Self {
        Self {
            name: name.to_string(),
            mime: mime.to_string(),
            kind: kind.to_string(),
            data: base64_encode(bytes),
        }
    }
}

/// Base64 for the wire, without pulling a dependency for one small encoder.
fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let a = chunk[0] as u32;
        let b = chunk.get(1).copied().unwrap_or(0) as u32;
        let c = chunk.get(2).copied().unwrap_or(0) as u32;
        let triple = (a << 16) | (b << 8) | c;
        out.push(ALPHABET[(triple >> 18) as usize & 0x3F] as char);
        out.push(ALPHABET[(triple >> 12) as usize & 0x3F] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(triple >> 6) as usize & 0x3F] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[triple as usize & 0x3F] as char
        } else {
            '='
        });
    }
    out
}

/// The Windows sandbox helper next to the app, once it has proven the APIs work.
///
/// A missing helper, a non-Windows build, or a machine where the restricted
/// token cannot start a process all read as `None`: the caller then keeps the
/// path guard and the approval card as the only gate.
#[cfg(windows)]
fn sandbox_helper() -> Option<std::path::PathBuf> {
    static CHECKED: std::sync::OnceLock<Option<std::path::PathBuf>> = std::sync::OnceLock::new();
    CHECKED
        .get_or_init(|| {
            let exe = std::env::current_exe().ok()?;
            let helper = exe.parent()?.join("dsh-sandbox.exe");
            if !helper.is_file() {
                return None;
            }
            let mut command = Command::new(&helper);
            command
                .arg("--check")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            // The helper is a console program; this window is not, so without
            // the flag the probe would pop a console window at startup.
            {
                use std::os::windows::process::CommandExt;
                command.creation_flags(0x0800_0000);
            }
            let ok = command.status().ok()?.success();
            ok.then_some(helper)
        })
        .clone()
}

#[cfg(not(windows))]
fn sandbox_helper() -> Option<std::path::PathBuf> {
    None
}

/// A request to the engine's `ai` command.
pub struct AiRequest {
    queue: Arc<Mutex<Vec<AiEvent>>>,
    child: Arc<Mutex<Option<Child>>>,
    /// Lines for an interactive engine: approval decisions, one JSON per line.
    responder: Mutex<mpsc::Sender<String>>,
    started: Instant,
}

impl AiRequest {
    /// Ask one question. `system` is optional extra instruction, `history` the
    /// earlier turns of this conversation, `work_dir` is where the engine runs so
    /// files it writes land with the conversation, and `tools` lets the assistant
    /// read and write inside it. `attachments` carries images and PDFs with this
    /// question only. Commands are confined to the workspace.
    #[allow(clippy::too_many_arguments)]
    pub fn ask(
        engine_dir: &Path,
        work_dir: &Path,
        prompt: &str,
        system: Option<&str>,
        history: &[Turn],
        connection: &Connection,
        tools: bool,
        allowed_tools: Option<&[&str]>,
        confirm_writes: bool,
        scope_root: Option<&Path>,
        read_only: &[PathBuf],
        max_rounds: u32,
        max_calls: u32,
        plan: bool,
        web_search: bool,
        attachments: &[AskAttachment],
        plugin_dir: Option<&Path>,
        plugins: &[String],
        mcp_servers: &[crate::mcp::McpServer],
        mcp_dir: Option<&Path>,
    ) -> Result<Self, String> {
        let cli = engine_dir.join("src").join("cli.ts");
        if !cli.exists() {
            return Err(format!(
                "找不到引擎：{}\n设置 DSH_FLOW_HOME 指向 dsh-flow 目录后重启界面。",
                cli.display()
            ));
        }

        let mut command = Command::new("node");
        command
            .arg(&cli)
            .arg("ai")
            .current_dir(work_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // A group of its own, so a stop can take the commands with it.
            command.process_group(0);
        }

        crate::engine::apply_proxy_env(&mut command);
        // Commands run through the OS sandbox when the workspace is jailed; a
        // full-access conversation keeps the old direct spawn.
        if let Some(helper) = sandbox_helper()
            && scope_root.is_some()
        {
            let temp = work_dir.join(".dsh-flow").join("sandbox-tmp");
            let _ = std::fs::create_dir_all(&temp);
            command
                .env("DSH_SANDBOX_HELPER", &helper)
                .env("DSH_SANDBOX_WORKSPACE", work_dir)
                .env(
                    "DSH_SANDBOX_READONLY",
                    if confirm_writes { "1" } else { "0" },
                )
                .env("TEMP", &temp)
                .env("TMP", &temp);
        }
        let mut child = command
            .spawn()
            .map_err(|error| crate::engine::node_error(&error))?;
        let pid = child.id();
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        // The engine itself is always readable: it is what the assistant is
        // explaining, and the caller adds the workflow directory on top.
        let mut roots = vec![engine_dir.to_path_buf()];
        roots.extend(read_only.iter().cloned());
        roots.dedup();

        let (responder, responses) = mpsc::channel::<String>();
        if let Some(mut stdin) = stdin {
            let payload = payload_with_budgets(
                prompt,
                system,
                history,
                connection,
                tools,
                allowed_tools,
                confirm_writes,
                scope_root,
                &roots,
                max_rounds,
                max_calls,
                plan,
                web_search,
                attachments,
                plugin_dir,
                plugins,
                mcp_servers,
                mcp_dir,
            );
            // With tools on, the pipe stays open after the request so the
            // engine can wait for an approval reply on the same pipe.
            let interactive = tools;
            thread::spawn(move || {
                if stdin.write_all(payload.as_bytes()).is_err()
                    || stdin.write_all(b"\n").is_err()
                    || stdin.flush().is_err()
                {
                    return;
                }
                if !interactive {
                    return;
                }
                for line in responses {
                    if stdin.write_all(line.as_bytes()).is_err() {
                        break;
                    }
                    let _ = stdin.write_all(b"\n");
                    if stdin.flush().is_err() {
                        break;
                    }
                }
            });
        }

        let queue = Arc::new(Mutex::new(Vec::new()));
        let slot = Arc::new(Mutex::new(Some(child)));
        let errors = Arc::new(Mutex::new(String::new()));
        register_child(pid, slot.clone());

        if let Some(mut stderr) = stderr {
            let errors = errors.clone();
            thread::spawn(move || {
                let mut text = String::new();
                if stderr.read_to_string(&mut text).is_ok() {
                    *lock(&errors) = text;
                }
            });
        }

        {
            let queue = queue.clone();
            let slot = slot.clone();
            let errors = errors.clone();
            thread::spawn(move || {
                // Steps are pushed as their lines arrive, so the window can show
                // each tool while the assistant is still working.
                let mut plain = String::new();
                let mut answered = false;
                if let Some(stdout) = stdout {
                    for line in BufReader::new(stdout).lines() {
                        let Ok(line) = line else { break };
                        let line = line.trim();
                        if line.is_empty() {
                            continue;
                        }
                        match line_event(line) {
                            Some(event) => {
                                if matches!(event, AiEvent::Done { .. } | AiEvent::Failed(_)) {
                                    answered = true;
                                }
                                lock(&queue).push(event);
                            }
                            None => {
                                plain.push_str(line);
                                plain.push('\n');
                            }
                        }
                    }
                }
                // Take the child out of the slot before waiting: holding that
                // lock across `wait()` would block the shutdown kill.
                let child = lock(&slot).take();
                let exit_code = child
                    .and_then(|mut child| child.wait().ok())
                    .and_then(|status| status.code());
                forget_child(pid);
                if answered {
                    return;
                }
                let stderr_text = lock(&errors).clone();
                let event = match answer_text(&plain, &stderr_text, exit_code) {
                    Ok(answer) => AiEvent::Done {
                        text: answer.text,
                        usage: answer.usage,
                        context: answer.context,
                        cache_hits: answer.cache_hits,
                        rounds: answer.rounds,
                        round_tokens: answer.round_tokens,
                        round_reasoning: answer.round_reasoning,
                        compaction: answer.compaction,
                        reasoning: answer.reasoning,
                        model_ms: answer.model_ms,
                        tool_ms: answer.tool_ms,
                        web_searches: answer.web_searches,
                        web_fetches: answer.web_fetches,
                        server_tool_calls: answer.server_tool_calls,
                    },
                    Err(error) => AiEvent::Failed(error),
                };
                lock(&queue).push(event);
            });
        }

        Ok(Self {
            queue,
            child: slot,
            responder: Mutex::new(responder),
            started: Instant::now(),
        })
    }

    /// Take every event produced since the last call.
    pub fn poll(&self) -> Vec<AiEvent> {
        std::mem::take(&mut *lock(&self.queue))
    }

    /// Allow or refuse one write the engine is waiting on.
    ///
    /// @returns false when the engine is already gone, so the caller does not
    /// claim a decision that will never be read.
    pub fn respond(&self, call: &str, allow: bool) -> bool {
        let line = serde_json::json!({
            "type": "approval",
            "call": call,
            "allow": allow,
        })
        .to_string();
        lock(&self.responder).send(line).is_ok()
    }

    /// Answer a structured question the engine is waiting on.
    pub fn respond_question(&self, call: &str, answer: &str) -> bool {
        let line = serde_json::json!({
            "type": "question",
            "call": call,
            "answer": answer,
        })
        .to_string();
        lock(&self.responder).send(line).is_ok()
    }

    /// Approve or reject a plan the engine is waiting on.
    pub fn respond_plan(&self, call: &str, approved: bool) -> bool {
        let line = serde_json::json!({
            "type": "plan",
            "call": call,
            "approved": approved,
        })
        .to_string();
        lock(&self.responder).send(line).is_ok()
    }

    /// Stop the request, including a command it started.
    pub fn cancel(&self) {
        if let Some(child) = lock(&self.child).as_mut() {
            kill_tree(child.id());
            let _ = child.kill();
        }
    }

    /// Milliseconds since the request started.
    pub fn elapsed_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }
}

impl Drop for AiRequest {
    /// A request nobody polls any more must not keep the engine alive.
    ///
    /// The conversation may be deleted, or a retry may replace this request: the
    /// child is killed here so no `node` process outlives the window. A request
    /// whose child was already reaped has an empty slot, so this is a no-op then.
    fn drop(&mut self) {
        self.cancel();
    }
}

/// The JSON the engine reads from stdin.
///
/// Kept apart from `ask` so the shape can be checked without spawning a process.
#[cfg(test)]
fn payload(
    prompt: &str,
    system: Option<&str>,
    history: &[Turn],
    connection: &Connection,
    tools: bool,
    allowed_tools: Option<&[&str]>,
    confirm_writes: bool,
    scope_root: Option<&Path>,
    read_only: &[PathBuf],
) -> String {
    payload_with_budgets(
        prompt,
        system,
        history,
        connection,
        tools,
        allowed_tools,
        confirm_writes,
        scope_root,
        read_only,
        DEFAULT_MAX_ROUNDS,
        DEFAULT_MAX_CALLS,
        false,
        true,
        &[],
        None,
        &[],
        &[],
        None,
    )
}

/// The request with the tool budgets the settings page chose.
#[allow(clippy::too_many_arguments)]
fn payload_with_budgets(
    prompt: &str,
    system: Option<&str>,
    history: &[Turn],
    connection: &Connection,
    tools: bool,
    allowed_tools: Option<&[&str]>,
    confirm_writes: bool,
    scope_root: Option<&Path>,
    read_only: &[PathBuf],
    max_rounds: u32,
    max_calls: u32,
    plan: bool,
    web_search: bool,
    attachments: &[AskAttachment],
    plugin_dir: Option<&Path>,
    plugins: &[String],
    mcp_servers: &[crate::mcp::McpServer],
    mcp_dir: Option<&Path>,
) -> String {
    let mut request = serde_json::Map::new();
    request.insert(
        "prompt".to_string(),
        serde_json::Value::String(prompt.to_string()),
    );
    if let Some(system) = system {
        request.insert(
            "system".to_string(),
            serde_json::Value::String(system.to_string()),
        );
    }
    if !history.is_empty() {
        let turns: Vec<serde_json::Value> = history
            .iter()
            .map(|turn| {
                let mut entry = serde_json::Map::new();
                entry.insert(
                    "role".to_string(),
                    serde_json::Value::String(turn.speaker.role().to_string()),
                );
                entry.insert(
                    "content".to_string(),
                    serde_json::Value::String(turn.text.clone()),
                );
                // Only an assistant turn can carry server calls, and passing
                // them back is what keeps the pages it read in the context.
                if !turn.server_tool_calls.is_empty() {
                    entry.insert(
                        "serverToolCalls".to_string(),
                        serde_json::Value::Array(turn.server_tool_calls.clone()),
                    );
                }
                serde_json::Value::Object(entry)
            })
            .collect();
        request.insert("history".to_string(), serde_json::Value::Array(turns));
    }
    request.insert(
        "model".to_string(),
        serde_json::Value::String(connection.model.clone()),
    );
    request.insert(
        "baseUrl".to_string(),
        serde_json::Value::String(connection.base_url.clone()),
    );
    request.insert(
        "apiKey".to_string(),
        serde_json::Value::String(connection.api_key.clone()),
    );
    // Left out when the connection keeps the endpoint's own value.
    if let Some(temperature) = connection.temperature {
        request.insert("temperature".to_string(), serde_json::json!(temperature));
    }
    if connection.max_tokens > 0 {
        request.insert(
            "maxTokens".to_string(),
            serde_json::json!(connection.max_tokens),
        );
    }
    // Left out for the default choice as well: the engine then sends no switch
    // and the endpoint keeps its own thinking setting.
    if connection.reasoning != crate::config::Reasoning::Default {
        request.insert(
            "reasoning".to_string(),
            serde_json::Value::String(connection.reasoning.name().to_string()),
        );
    }
    // The server tools only exist on DeepSeek's flash models; an endpoint that
    // does not offer them rejects the request outright, so the entries are left
    // out here rather than trusted to the engine's own retry. Searching is on
    // wherever it is offered; a conversation can turn it off for itself.
    if web_search && connection.supports_web_search() {
        request.insert("webSearch".to_string(), serde_json::Value::Bool(true));
    }
    request.insert(
        "timeoutMs".to_string(),
        serde_json::Value::Number(CHAT_TIMEOUT_MS.into()),
    );
    // The engine compacts at the same point the window shows the gauge filling,
    // so both sides agree on how full the window is. An explicit setting on the
    // connection wins over the name table, which cannot know a local model's
    // size or one newer than the table.
    request.insert(
        "windowTokens".to_string(),
        serde_json::json!(connection.window_tokens()),
    );
    if tools {
        request.insert("tools".to_string(), serde_json::Value::Bool(true));
        request.insert("interactive".to_string(), serde_json::Value::Bool(true));
        request.insert(
            "confirmWrites".to_string(),
            serde_json::Value::Bool(confirm_writes),
        );
        request.insert("maxRounds".to_string(), serde_json::json!(max_rounds));
        request.insert("maxCalls".to_string(), serde_json::json!(max_calls));
        if plan {
            request.insert("plan".to_string(), serde_json::Value::Bool(true));
        }
        match scope_root {
            Some(root) => {
                request.insert(
                    "fileScope".to_string(),
                    serde_json::Value::String("workspace".to_string()),
                );
                if let Some(root) = root.to_str() {
                    request.insert(
                        "scopeRoot".to_string(),
                        serde_json::Value::String(root.to_string()),
                    );
                }
            }
            None => {
                request.insert(
                    "fileScope".to_string(),
                    serde_json::Value::String("full".to_string()),
                );
            }
        }
        if let Some(allowed) = allowed_tools {
            let names: Vec<serde_json::Value> = allowed
                .iter()
                .map(|name| serde_json::Value::String((*name).to_string()))
                .collect();
            request.insert("allowedTools".to_string(), serde_json::Value::Array(names));
        }
        if !read_only.is_empty() {
            let roots: Vec<serde_json::Value> = read_only
                .iter()
                .filter_map(|dir| dir.to_str())
                .map(|dir| serde_json::Value::String(dir.to_string()))
                .collect();
            request.insert("readOnlyRoots".to_string(), serde_json::Value::Array(roots));
        }
    }
    // Plugins are named in every request: the engine reads the folder afresh,
    // so a plugin installed or switched on since the last question is live.
    if let Some(dir) = plugin_dir.and_then(|dir| dir.to_str()) {
        request.insert(
            "pluginDir".to_string(),
            serde_json::Value::String(dir.to_string()),
        );
        let names: Vec<serde_json::Value> = plugins
            .iter()
            .filter(|name| crate::config::is_plugin_id(name))
            .map(|name| serde_json::Value::String(name.clone()))
            .collect();
        request.insert("plugins".to_string(), serde_json::Value::Array(names));
    }
    // Only enabled servers that could be read travel; a broken entry is shown
    // on the MCP page instead of making every question fail.
    if tools && !mcp_servers.is_empty() {
        let servers: Vec<serde_json::Value> = mcp_servers
            .iter()
            .filter_map(|server| server.wire())
            .collect();
        if !servers.is_empty() {
            request.insert("mcpServers".to_string(), serde_json::Value::Array(servers));
        }
    }
    // The folder lets the engine's `mcp` tool read the whole catalog and save a
    // server it links on the spot.
    if tools && let Some(dir) = mcp_dir.and_then(|dir| dir.to_str()) {
        request.insert(
            "mcpDir".to_string(),
            serde_json::Value::String(dir.to_string()),
        );
    }
    if !attachments.is_empty() {
        let files: Vec<serde_json::Value> = attachments
            .iter()
            .map(|file| {
                serde_json::json!({
                    "name": file.name,
                    "kind": file.kind,
                    "mimeType": file.mime,
                    "data": file.data,
                })
            })
            .collect();
        request.insert("attachments".to_string(), serde_json::Value::Array(files));
    }
    serde_json::Value::Object(request).to_string()
}

/// Read the bridge's whole stdout the way the reader thread does, one line at a
/// time. Used by the tests and as the single definition of the wire shapes.
///
/// The documented shapes are one JSON object per line — `{type:"step",…}` for
/// each tool call and `{ text, json, usage }` for the answer — but a single
/// object or a bare text answer is also accepted so a minimal bridge still works.
#[cfg(test)]
fn events(stdout: &str, stderr: &str, exit_code: Option<i32>) -> Vec<AiEvent> {
    let mut events: Vec<AiEvent> = Vec::new();
    let mut plain = String::new();
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match line_event(line) {
            Some(event) => events.push(event),
            None => {
                plain.push_str(line);
                plain.push('\n');
            }
        }
    }

    let answered = events
        .iter()
        .any(|event| matches!(event, AiEvent::Done { .. } | AiEvent::Failed(_)));
    if !answered {
        events.push(match answer_text(&plain, stderr, exit_code) {
            Ok(answer) => AiEvent::Done {
                text: answer.text,
                usage: answer.usage,
                context: answer.context,
                cache_hits: answer.cache_hits,
                rounds: answer.rounds,
                round_tokens: answer.round_tokens,
                round_reasoning: answer.round_reasoning,
                compaction: answer.compaction,
                reasoning: answer.reasoning,
                model_ms: answer.model_ms,
                tool_ms: answer.tool_ms,
                web_searches: answer.web_searches,
                web_fetches: answer.web_fetches,
                server_tool_calls: answer.server_tool_calls,
            },
            Err(error) => AiEvent::Failed(error),
        });
    }
    events
}

/// Turn one stdout line into an event, or `None` when the line is not ours.
fn line_event(line: &str) -> Option<AiEvent> {
    if !line.starts_with('{') {
        return None;
    }
    let value = serde_json::from_str::<serde_json::Value>(line).ok()?;
    match value.get("type").and_then(|kind| kind.as_str()) {
        Some("step") => {
            return Some(AiEvent::Step {
                tool: value
                    .get("tool")
                    .and_then(|tool| tool.as_str())
                    .unwrap_or("tool")
                    .to_string(),
                target: value
                    .get("target")
                    .and_then(|target| target.as_str())
                    .unwrap_or_default()
                    .to_string(),
                ok: value.get("ok").and_then(|ok| ok.as_bool()).unwrap_or(true),
                detail: value
                    .get("detail")
                    .and_then(|detail| detail.as_str())
                    .unwrap_or_default()
                    .to_string(),
                input: value
                    .get("input")
                    .and_then(|input| input.as_str())
                    .unwrap_or_default()
                    .to_string(),
                output: value
                    .get("output")
                    .and_then(|output| output.as_str())
                    .unwrap_or_default()
                    .to_string(),
                call: value
                    .get("call")
                    .and_then(|call| call.as_str())
                    .unwrap_or_default()
                    .to_string(),
            });
        }
        Some("progress") => {
            return Some(AiEvent::Progress {
                round: value
                    .get("round")
                    .and_then(|round| round.as_u64())
                    .unwrap_or(0) as u32,
                calls: value
                    .get("calls")
                    .and_then(|calls| calls.as_u64())
                    .unwrap_or(0) as u32,
            });
        }
        Some("tool_start") => {
            return Some(AiEvent::ToolStart {
                call: value
                    .get("call")
                    .and_then(|call| call.as_str())
                    .unwrap_or_default()
                    .to_string(),
                tool: value
                    .get("tool")
                    .and_then(|tool| tool.as_str())
                    .unwrap_or_default()
                    .to_string(),
                target: value
                    .get("target")
                    .and_then(|target| target.as_str())
                    .unwrap_or_default()
                    .to_string(),
            });
        }
        Some("tool_output") => {
            let text = value
                .get("text")
                .and_then(|text| text.as_str())
                .unwrap_or_default();
            if text.is_empty() {
                return None;
            }
            return Some(AiEvent::ToolOutput {
                call: value
                    .get("call")
                    .and_then(|call| call.as_str())
                    .unwrap_or_default()
                    .to_string(),
                tool: value
                    .get("tool")
                    .and_then(|tool| tool.as_str())
                    .unwrap_or_default()
                    .to_string(),
                target: value
                    .get("target")
                    .and_then(|target| target.as_str())
                    .unwrap_or_default()
                    .to_string(),
                text: text.to_string(),
            });
        }
        Some("approval") => {
            return Some(AiEvent::Approval {
                call: value
                    .get("call")
                    .and_then(|call| call.as_str())
                    .unwrap_or_default()
                    .to_string(),
                tool: value
                    .get("tool")
                    .and_then(|tool| tool.as_str())
                    .unwrap_or_default()
                    .to_string(),
                target: value
                    .get("target")
                    .and_then(|target| target.as_str())
                    .unwrap_or_default()
                    .to_string(),
                path: value
                    .get("path")
                    .and_then(|path| path.as_str())
                    .unwrap_or_default()
                    .to_string(),
                diff: value
                    .get("diff")
                    .and_then(|diff| diff.as_str())
                    .unwrap_or_default()
                    .to_string(),
                before: value
                    .get("before")
                    .and_then(|before| before.as_str())
                    .unwrap_or_default()
                    .to_string(),
                after: value
                    .get("after")
                    .and_then(|after| after.as_str())
                    .unwrap_or_default()
                    .to_string(),
                existed: value
                    .get("existed")
                    .and_then(|existed| existed.as_bool())
                    .unwrap_or(false),
                undoable: value
                    .get("undoable")
                    .and_then(|undoable| undoable.as_bool())
                    .unwrap_or(false),
                auto: value
                    .get("auto")
                    .and_then(|auto| auto.as_bool())
                    .unwrap_or(false),
                command: value
                    .get("command")
                    .and_then(|command| command.as_str())
                    .unwrap_or_default()
                    .to_string(),
            });
        }
        Some("question") => {
            let options = value
                .get("options")
                .and_then(|options| options.as_array())
                .map(|options| {
                    options
                        .iter()
                        .filter_map(|option| option.as_str())
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            return Some(AiEvent::Question {
                call: value
                    .get("call")
                    .and_then(|call| call.as_str())
                    .unwrap_or_default()
                    .to_string(),
                question: value
                    .get("question")
                    .and_then(|question| question.as_str())
                    .unwrap_or_default()
                    .to_string(),
                options,
                allow_custom: value
                    .get("allowCustom")
                    .and_then(|allow| allow.as_bool())
                    .unwrap_or(false),
            });
        }
        Some("plan") => {
            return Some(AiEvent::Plan {
                call: value
                    .get("call")
                    .and_then(|call| call.as_str())
                    .unwrap_or_default()
                    .to_string(),
                plan: value
                    .get("plan")
                    .and_then(|plan| plan.as_str())
                    .unwrap_or_default()
                    .to_string(),
            });
        }
        Some("todo") => {
            let items = value
                .get("items")
                .and_then(|items| items.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| {
                            let text = item.get("text").and_then(|text| text.as_str())?;
                            if text.trim().is_empty() {
                                return None;
                            }
                            let status = item
                                .get("status")
                                .and_then(|status| status.as_str())
                                .unwrap_or("pending");
                            Some((text.to_string(), status.to_string()))
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            return Some(AiEvent::Todo { items });
        }
        Some("delta") => {
            let text = value
                .get("text")
                .and_then(|text| text.as_str())
                .unwrap_or_default();
            return (!text.is_empty()).then(|| AiEvent::Delta(text.to_string()));
        }
        Some("reasoning") => {
            let text = value
                .get("text")
                .and_then(|text| text.as_str())
                .unwrap_or_default();
            return (!text.is_empty()).then(|| AiEvent::Reasoning(text.to_string()));
        }
        Some("tools") => {
            // One definition per offered tool, plus — when the engine knows it —
            // which plugin or MCP server put the tool there. The origin is read
            // here so a plugin card can count its own contribution.
            let mut items: Vec<(String, String)> = Vec::new();
            let mut plugin_tools: Vec<(String, String)> = Vec::new();
            let mut mcp_tools: Vec<(String, String)> = Vec::new();
            let list = value
                .get("items")
                .and_then(|items| items.as_array())
                .map(Vec::as_slice)
                .unwrap_or_default();
            for item in list {
                let Some(name) = item.get("name").and_then(|name| name.as_str()) else {
                    continue;
                };
                let text = |key: &str| -> &str {
                    item.get(key)
                        .and_then(|entry| entry.as_str())
                        .unwrap_or_default()
                        .trim()
                };
                items.push((
                    name.to_string(),
                    item.get("schema")
                        .and_then(|schema| schema.as_str())
                        .unwrap_or_default()
                        .to_string(),
                ));
                let plugin = text("plugin");
                if !plugin.is_empty() {
                    plugin_tools.push((plugin.to_string(), name.to_string()));
                }
                let server = text("server");
                if !server.is_empty() {
                    mcp_tools.push((server.to_string(), name.to_string()));
                }
            }
            let panels: Vec<PanelReport> = value
                .get("panels")
                .and_then(|panels| panels.as_array())
                .map(|list| list.iter().filter_map(read_panel).collect())
                .unwrap_or_default();
            let pairs = |key: &str| {
                value
                    .get(key)
                    .and_then(|items| items.as_array())
                    .map(|list| {
                        list.iter()
                            .filter_map(|item| {
                                let name = item
                                    .get("name")
                                    .or_else(|| item.get("id"))
                                    .and_then(|name| name.as_str())?;
                                let text = item
                                    .get("schema")
                                    .or_else(|| item.get("text"))
                                    .or_else(|| item.get("message"))
                                    .or_else(|| item.get("error"))
                                    .and_then(|text| text.as_str())
                                    .unwrap_or_default();
                                Some((name.to_string(), text.to_string()))
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            };
            let error_pairs = |wanted: bool| {
                value
                    .get("errors")
                    .and_then(|items| items.as_array())
                    .map(|list| {
                        list.iter()
                            .filter_map(|item| {
                                let is_mcp =
                                    item.get("kind").and_then(|kind| kind.as_str()) == Some("mcp");
                                if is_mcp != wanted {
                                    return None;
                                }
                                let name = item
                                    .get("id")
                                    .or_else(|| item.get("name"))
                                    .and_then(|name| name.as_str())?;
                                let text = item
                                    .get("message")
                                    .or_else(|| item.get("error"))
                                    .and_then(|text| text.as_str())
                                    .unwrap_or_default();
                                Some((name.to_string(), text.to_string()))
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            };
            return Some(AiEvent::Tools {
                items,
                prompts: pairs("prompts"),
                errors: error_pairs(false),
                mcp_errors: error_pairs(true),
                plugin_tools,
                mcp_tools,
                panels,
            });
        }
        // A type this window does not know belongs to a newer engine; ignoring
        // it keeps the answer readable, while treating it as an answer would
        // turn an extra event into a fake failure.
        Some(_) => return None,
        None => {}
    }
    Some(match read_answer(&value) {
        Ok(answer) => AiEvent::Done {
            text: answer.text,
            usage: answer.usage,
            context: answer.context,
            cache_hits: answer.cache_hits,
            rounds: answer.rounds,
            round_tokens: answer.round_tokens,
            round_reasoning: answer.round_reasoning,
            compaction: answer.compaction,
            reasoning: answer.reasoning,
            model_ms: answer.model_ms,
            tool_ms: answer.tool_ms,
            web_searches: answer.web_searches,
            web_fetches: answer.web_fetches,
            server_tool_calls: answer.server_tool_calls,
        },
        Err(error) => AiEvent::Failed(error),
    })
}

/// Most sections one panel may add; mirrors the engine's cap so a broken engine
/// cannot grow a card without bound.
const MAX_PANEL_SECTIONS: usize = 6;
/// Most rows one panel section may list.
const MAX_PANEL_ITEMS: usize = 40;
/// Most columns a panel table may have.
const MAX_PANEL_COLUMNS: usize = 8;
/// Longest text kept from one panel entry.
const MAX_PANEL_TEXT: usize = 4_000;
/// Longest note or table cell kept.
const MAX_PANEL_SHORT: usize = 200;

/// Read one plugin panel out of a `tools` event.
fn read_panel(value: &serde_json::Value) -> Option<PanelReport> {
    let id = panel_text(value.get("id"), MAX_PANEL_SHORT);
    if id.is_empty() {
        return None;
    }
    let title = match panel_text(value.get("title"), MAX_PANEL_SHORT) {
        title if title.is_empty() => id.clone(),
        title => title,
    };
    let summary = value
        .get("summary")
        .and_then(|items| items.as_array())
        .map(|list| {
            list.iter()
                .take(4)
                .filter_map(|item| {
                    let label = panel_text(item.get("label"), MAX_PANEL_SHORT);
                    if label.is_empty() {
                        return None;
                    }
                    let printed = match item.get("value") {
                        Some(serde_json::Value::String(text)) => text.clone(),
                        Some(serde_json::Value::Number(number)) => number.to_string(),
                        _ => String::new(),
                    };
                    Some((label, printed))
                })
                .collect()
        })
        .unwrap_or_default();
    let sections = value
        .get("sections")
        .and_then(|items| items.as_array())
        .map(|list| {
            list.iter()
                .take(MAX_PANEL_SECTIONS)
                .map(read_panel_section)
                .collect()
        })
        .unwrap_or_default();
    Some(PanelReport {
        id,
        title,
        plugin: panel_text(value.get("plugin"), MAX_PANEL_SHORT),
        summary,
        sections,
    })
}

/// Read one section; a shape this window does not know is drawn as text.
fn read_panel_section(value: &serde_json::Value) -> PanelSection {
    let kind = match value.get("kind").and_then(|kind| kind.as_str()) {
        Some("bars") => PanelKind::Bars,
        Some("table") => PanelKind::Table,
        _ => PanelKind::Text,
    };
    let items = value
        .get("items")
        .and_then(|items| items.as_array())
        .map(|list| {
            list.iter()
                .take(MAX_PANEL_ITEMS)
                .filter_map(|item| {
                    let label = panel_text(item.get("label"), MAX_PANEL_SHORT);
                    let text = panel_text(item.get("text"), MAX_PANEL_TEXT);
                    let note = panel_text(item.get("note"), MAX_PANEL_SHORT);
                    let number = panel_number(item.get("value"));
                    if label.is_empty() && text.is_empty() && note.is_empty() && number.is_none() {
                        return None;
                    }
                    Some(PanelItem {
                        label,
                        text,
                        value: number,
                        note,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let columns = value
        .get("columns")
        .and_then(|columns| columns.as_array())
        .map(|list| {
            list.iter()
                .take(MAX_PANEL_COLUMNS)
                .map(|cell| panel_text(Some(cell), MAX_PANEL_SHORT))
                .collect()
        })
        .unwrap_or_default();
    let rows = value
        .get("rows")
        .and_then(|rows| rows.as_array())
        .map(|list| {
            list.iter()
                .take(MAX_PANEL_ITEMS)
                .map(|row| {
                    row.as_array()
                        .map(|cells| {
                            cells
                                .iter()
                                .take(MAX_PANEL_COLUMNS)
                                .map(|cell| panel_text(Some(cell), MAX_PANEL_SHORT))
                                .collect()
                        })
                        .unwrap_or_default()
                })
                .collect()
        })
        .unwrap_or_default();
    PanelSection {
        title: panel_text(value.get("title"), MAX_PANEL_SHORT),
        kind,
        items,
        columns,
        rows,
    }
}

/// Read a JSON string, clipped to `limit` characters with a visible marker.
fn panel_text(value: Option<&serde_json::Value>, limit: usize) -> String {
    let text = value
        .and_then(|entry| entry.as_str())
        .unwrap_or_default()
        .trim();
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let mut clipped: String = text.chars().take(limit).collect();
    clipped.push('…');
    clipped
}

/// Read a bar row's number, whether the engine sent it as a number or a string.
fn panel_number(value: Option<&serde_json::Value>) -> Option<f64> {
    let number = match value? {
        serde_json::Value::Number(number) => number.as_f64(),
        serde_json::Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }?;
    // A bar is drawn from a ratio; a non-finite value would poison the layout.
    number.is_finite().then_some(number)
}

/// One answer, with whatever the endpoint said it cost.
struct Answer {
    text: String,
    usage: Option<Usage>,
    /// Prompt + answer of the last request; zero when the engine did not say.
    context: u64,
    /// Prompt tokens the provider served from its cache, summed over rounds.
    cache_hits: u64,
    /// Round trips the question took; one when the engine does not say.
    rounds: u32,
    /// What each round cost, oldest first.
    round_tokens: Vec<(u64, u64, u32)>,
    /// Estimated thinking tokens of each round, oldest first: output the bill
    /// includes that no prompt holds. Empty for providers that do not stream
    /// reasoning.
    round_reasoning: Vec<u32>,
    /// What the context compaction did, when it did anything.
    compaction: Option<Compaction>,
    /// The thinking text the engine streamed for the whole question; empty when
    /// the provider sent none.
    reasoning: String,
    /// Wall clock the engine spent waiting for model round trips, in
    /// milliseconds; zero when it did not report timing (older engines).
    model_ms: u64,
    /// Wall clock the engine spent running tools, in milliseconds; zero when
    /// it did not report timing.
    tool_ms: u64,
    /// Searches the provider ran for this question on its own side.
    web_searches: u32,
    /// Pages the provider opened or read for this question.
    web_fetches: u32,
    /// Raw `server_tool_calls` of every round, kept so the next question can
    /// hand them back and keep the pages read earlier in context.
    server_tool_calls: Vec<serde_json::Value>,
}

/// Read the engine's compaction report out of an answer.
fn read_compaction(value: &serde_json::Value) -> Option<Compaction> {
    let layers = value.get("compaction")?;
    if !layers.is_object() {
        return None;
    }
    let number = |key: &str| layers.get(key).and_then(|item| item.as_u64()).unwrap_or(0);
    let report = Compaction {
        layer: layers
            .get("layer")
            .and_then(|item| item.as_str())
            .unwrap_or_default()
            .to_string(),
        before: number("before"),
        after: number("after"),
        pruned: number("pruned"),
        turns: number("turns"),
        summarized: layers
            .get("summarized")
            .and_then(|item| item.as_bool())
            .unwrap_or(false),
    };
    report.happened().then_some(report)
}

/// Read the engine's timing buckets; `(0, 0)` when it reported none.
///
/// The two figures are the engine's own wall clock for the phases it drives:
/// the model round trips and the tool runs. The window subtracts them from the
/// answer's total to show what it could not attribute.
fn read_timing(value: &serde_json::Value) -> (u64, u64) {
    let Some(timing) = value.get("timing") else {
        return (0, 0);
    };
    let number = |key: &str| timing.get(key).and_then(|item| item.as_u64()).unwrap_or(0);
    (number("modelMs"), number("toolMs"))
}

/// Read the provider's token counts; `None` when the endpoint sent none.
fn read_usage(value: &serde_json::Value) -> Option<Usage> {
    let usage = value.get("usage")?;
    let number = |keys: &[&str]| {
        keys.iter()
            .find_map(|key| usage.get(*key).and_then(|count| count.as_u64()))
    };
    let prompt = number(&["prompt_tokens", "input_tokens"]).unwrap_or(0);
    let reported = number(&["completion_tokens", "output_tokens"]).unwrap_or(0);
    // Some endpoints only report the sum; treat the rest as the answer.
    let completion = if reported > 0 {
        reported
    } else {
        number(&["total_tokens"]).map_or(0, |total| total.saturating_sub(prompt))
    };
    (prompt + completion > 0).then_some(Usage { prompt, completion })
}

/// Pull the answer out of one parsed object.
fn read_answer(value: &serde_json::Value) -> Result<Answer, String> {
    let usage = read_usage(value);
    let compaction = read_compaction(value);
    // What the engine's own clock measured, so the panel can split a request
    // into model, tools and the rest. Older engines send nothing, and the card
    // is hidden rather than showing a made-up zero split.
    let (model_ms, tool_ms) = read_timing(value);
    // The thinking text, kept so the transcript can show it in its own block;
    // empty when the provider streamed none.
    let reasoning = value
        .get("reasoning")
        .and_then(|text| text.as_str())
        .unwrap_or_default()
        .to_string();
    let context = value
        .get("context")
        .and_then(|context| context.as_u64())
        .unwrap_or(0);
    // Prompt tokens the provider served from its cache, which the window shows
    // beside the spend; zero for endpoints that do not report them.
    let cache_hits = value
        .get("cacheHits")
        .and_then(|hits| hits.as_u64())
        .unwrap_or(0);
    // One round trip unless the engine counted more: `ai` answers in a single
    // request, and an older engine does not report the figure at all.
    let rounds = value
        .get("rounds")
        .and_then(|rounds| rounds.as_u64())
        .map_or(1, |rounds| rounds.min(u64::from(u32::MAX)) as u32)
        .max(1);
    // Per-round counts, so the panel can show where the bill comes from.
    let round_tokens = value
        .get("roundTokens")
        .and_then(|rounds| rounds.as_array())
        .map(|rounds| {
            rounds
                .iter()
                .filter_map(|round| {
                    let round = round.as_array()?;
                    let prompt = round.first()?.as_u64()?;
                    let completion = round.get(1).and_then(|count| count.as_u64()).unwrap_or(0);
                    let calls = round
                        .get(2)
                        .and_then(|count| count.as_u64())
                        .unwrap_or(0)
                        .min(u64::from(u32::MAX)) as u32;
                    Some((prompt, completion, calls))
                })
                .collect()
        })
        .unwrap_or_default();
    // Thinking tokens per round: a reasoner bills them as output, so they are
    // neither prompt nor answer and get their own figure here.
    let round_reasoning = value
        .get("roundReasoning")
        .and_then(|rounds| rounds.as_array())
        .map(|rounds| {
            rounds
                .iter()
                .map(|count| count.as_u64().unwrap_or(0).min(u64::from(u32::MAX)) as u32)
                .collect()
        })
        .unwrap_or_default();
    // Server tools the provider ran on its own side: the counts the statistics
    // ledger keeps, so the money spent on searching stays visible.
    let web = value.get("webSearch");
    let web_count = |key: &str| {
        web.and_then(|entry| entry.get(key))
            .and_then(|count| count.as_u64())
            .unwrap_or(0)
            .min(u64::from(u32::MAX)) as u32
    };
    let web_searches = web_count("searches");
    let web_fetches = web_count("fetches");
    // The raw calls, so the next question can pass them back and keep the pages
    // this question read in the model's context.
    let server_tool_calls: Vec<serde_json::Value> = value
        .get("serverToolCalls")
        .and_then(|calls| calls.as_array())
        .cloned()
        .unwrap_or_default();
    if let Some(text) = value.get("text").and_then(|text| text.as_str())
        && !text.trim().is_empty()
    {
        return Ok(Answer {
            text: text.trim().to_string(),
            usage,
            context,
            cache_hits,
            rounds,
            round_tokens,
            round_reasoning,
            compaction,
            model_ms,
            tool_ms,
            reasoning,
            web_searches,
            web_fetches,
            server_tool_calls,
        });
    }
    if let Some(json) = value.get("json") {
        return Ok(Answer {
            text: json.to_string(),
            usage,
            context,
            cache_hits,
            rounds,
            round_tokens,
            round_reasoning,
            compaction,
            model_ms,
            tool_ms,
            reasoning,
            web_searches,
            web_fetches,
            server_tool_calls,
        });
    }
    if let Some(error) = value.get("error").and_then(|error| error.as_str()) {
        return Err(format!("AI 请求失败：{error}"));
    }
    Err("引擎没有返回内容".to_string())
}

/// The older single-answer path, kept for bridges that print plain text.
fn answer_text(stdout: &str, stderr: &str, exit_code: Option<i32>) -> Result<Answer, String> {
    let trimmed = stdout.trim();
    if let Some(value) = crate::json::object(trimmed)
        && (value.get("text").is_some()
            || value.get("json").is_some()
            || value.get("error").is_some())
    {
        return read_answer(&value);
    }
    if !trimmed.is_empty() {
        return Ok(Answer {
            text: trimmed.to_string(),
            usage: None,
            context: 0,
            cache_hits: 0,
            rounds: 1,
            round_tokens: Vec::new(),
            round_reasoning: Vec::new(),
            compaction: None,
            model_ms: 0,
            tool_ms: 0,
            web_searches: 0,
            web_fetches: 0,
            reasoning: String::new(),
            server_tool_calls: Vec::new(),
        });
    }

    let detail = stderr.trim();
    if detail.is_empty() {
        Err(format!(
            "引擎没有返回内容（退出码 {}）；见 README.md「工作流里的模型调用」。",
            exit_code.map_or_else(|| "?".to_string(), |code| code.to_string())
        ))
    } else {
        Err(format!("AI 请求失败：{detail}"))
    }
}

/// What a model listing returned.
#[derive(Debug)]
pub struct ModelList {
    /// Model ids the endpoint reports, or the one a probe proved callable.
    pub models: Vec<String>,
    /// Why a plain `/models` listing failed, when the model came from a probe.
    pub note: Option<String>,
}

/// What one compaction request answers.
#[derive(Debug, Clone)]
pub struct Compacted {
    /// The condensed checkpoint text.
    pub summary: String,
    /// Model that wrote it, as the engine reported it.
    pub model: String,
}

/// Condense one span of a conversation through the engine's `compact` command.
///
/// This is the last compaction layer: the deterministic layers (per-result cap,
/// the tool budget, the early-turns recap) run inside the window and the engine
/// without a model, and this is what keeps the *content* of the dropped turns
/// instead of just their first lines. It is a separate process like every other
/// engine call, so a failure is a readable message rather than a crash, and the
/// caller falls back to the recap. Blocking: call it from a worker thread.
pub fn compact_span(
    engine_dir: &Path,
    connection: &Connection,
    turns: &[Turn],
    system: &str,
) -> Result<Compacted, String> {
    let cli = engine_dir.join("src").join("cli.ts");
    if !cli.exists() {
        return Err(format!(
            "找不到引擎：{}\n设置 DSH_FLOW_HOME 指向 dsh-flow 目录后重启界面。",
            cli.display()
        ));
    }
    let history: Vec<serde_json::Value> = turns
        .iter()
        .map(|turn| {
            serde_json::json!({
                "role": if turn.speaker == Speaker::Assistant { "assistant" } else { "user" },
                "content": turn.text,
            })
        })
        .collect();

    let mut command = Command::new("node");
    command
        .arg(&cli)
        .arg("compact")
        .current_dir(engine_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    crate::engine::apply_proxy_env(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| crate::engine::node_error(&error))?;
    let payload = serde_json::json!({
        "history": history,
        "system": system,
        "model": connection.model,
        "baseUrl": connection.base_url,
        "apiKey": connection.api_key,
        "timeoutMs": CHAT_TIMEOUT_MS,
    })
    .to_string();
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(payload.as_bytes())
            .and_then(|()| stdin.write_all(b"\n"))
            .map_err(|error| format!("无法写入引擎：{error}"))?;
    }
    let output = wait_with_timeout(child, Duration::from_millis(CHAT_TIMEOUT_MS))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    compact_answer(&stdout, &stderr, output.status.code())
}

/// Read `{ summary, model }` (or `{ error }`) out of the `compact` command.
fn compact_answer(stdout: &str, stderr: &str, exit_code: Option<i32>) -> Result<Compacted, String> {
    let trimmed = stdout.trim();
    if let Some(value) = crate::json::object(trimmed) {
        if let Some(error) = value.get("error").and_then(|error| error.as_str()) {
            return Err(format!("压缩失败：{error}"));
        }
        if let Some(summary) = value.get("summary").and_then(|item| item.as_str()) {
            let summary = summary.trim();
            if !summary.is_empty() {
                return Ok(Compacted {
                    summary: summary.to_string(),
                    model: value
                        .get("model")
                        .and_then(|item| item.as_str())
                        .unwrap_or_default()
                        .to_string(),
                });
            }
        }
    }
    let detail = stderr.trim();
    if detail.is_empty() {
        Err(format!(
            "压缩没有返回内容（退出码 {}）",
            exit_code.map_or_else(|| "?".to_string(), |code| code.to_string())
        ))
    } else {
        Err(format!("压缩失败：{detail}"))
    }
}

/// Ask the engine which models a connection may call.
///
/// Version names move faster than any list the window could hard-code, so the
/// settings page can ask the endpoint itself (`GET {base}/models`) through the
/// engine's `models` command. Endpoints without that route are probed with the
/// configured model instead, so the connection stays usable. Blocking: call it
/// from a worker thread.
pub fn list_models(engine_dir: &Path, connection: &Connection) -> Result<ModelList, String> {
    let cli = engine_dir.join("src").join("cli.ts");
    if !cli.exists() {
        return Err(format!(
            "找不到引擎：{}\n设置 DSH_FLOW_HOME 指向 dsh-flow 目录后重启界面。",
            cli.display()
        ));
    }

    let mut command = Command::new("node");
    command
        .arg(&cli)
        .arg("models")
        .current_dir(engine_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    crate::engine::apply_proxy_env(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| crate::engine::node_error(&error))?;
    let payload = serde_json::json!({
        "prompt": "",
        "model": connection.model,
        "baseUrl": connection.base_url,
        "apiKey": connection.api_key,
    })
    .to_string();
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(payload.as_bytes())
            .and_then(|()| stdin.write_all(b"\n"))
            .map_err(|error| format!("无法写入引擎：{error}"))?;
    }

    let output = wait_with_timeout(child, Duration::from_millis(ONE_SHOT_TIMEOUT_MS))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    model_list(&stdout, &stderr, output.status.code())
}

/// Read `{ models: [...] }` (or `{ error }`) out of the bridge's stdout.
fn model_list(stdout: &str, stderr: &str, exit_code: Option<i32>) -> Result<ModelList, String> {
    let trimmed = stdout.trim();
    if let Some(value) = crate::json::object(trimmed) {
        if let Some(error) = value.get("error").and_then(|error| error.as_str()) {
            return Err(format!("AI 请求失败：{error}"));
        }
        if let Some(models) = value.get("models").and_then(|models| models.as_array()) {
            let names: Vec<String> = models
                .iter()
                .filter_map(|model| model.as_str())
                .map(str::trim)
                .filter(|model| !model.is_empty())
                .map(str::to_string)
                .collect();
            if names.is_empty() {
                return Err("该接入点没有返回任何模型".to_string());
            }
            let degraded = value
                .get("degraded")
                .and_then(|flag| flag.as_bool())
                .unwrap_or(false);
            let note = if degraded {
                let reason = value
                    .get("reason")
                    .and_then(|reason| reason.as_str())
                    .unwrap_or("")
                    .trim();
                Some(if reason.is_empty() {
                    "该接入点不提供 /models，已按当前模型探测通过".to_string()
                } else {
                    format!("该接入点不提供 /models（{reason}），已按当前模型探测通过")
                })
            } else {
                None
            };
            return Ok(ModelList {
                models: names,
                note,
            });
        }
    }

    let detail = stderr.trim();
    if detail.is_empty() {
        Err(format!(
            "引擎没有返回模型列表（退出码 {}）。",
            exit_code.map_or_else(|| "?".to_string(), |code| code.to_string())
        ))
    } else {
        Err(format!("AI 请求失败：{detail}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The compaction report of an event list, when the answer carried one.
    fn done_compaction(list: &[AiEvent]) -> Option<Compaction> {
        list.iter().find_map(|event| match event {
            AiEvent::Done { compaction, .. } => compaction.clone(),
            _ => None,
        })
    }

    #[test]
    fn reads_the_compaction_report_and_ignores_a_pass_that_did_nothing() {
        let pruned = events(
            r#"{"text":"a","compaction":{"layer":"prune","before":9000,"after":1200,"pruned":2,"turns":0,"summarized":false}}"#,
            "",
            Some(0),
        );
        let report = done_compaction(&pruned).expect("a prune pass");
        assert_eq!(report.layer, "prune");
        assert_eq!(report.before, 9000);
        assert_eq!(report.pruned, 2);
        assert!(report.happened());
        assert!(report.note().contains("工具结果"), "{}", report.note());

        let summarized = events(
            r#"{"text":"a","compaction":{"layer":"summary","before":200000,"after":20000,"pruned":0,"turns":36,"summarized":true}}"#,
            "",
            Some(0),
        );
        let report = done_compaction(&summarized).expect("a summary pass");
        assert!(report.summarized);
        assert_eq!(
            report.note(),
            "上下文接近窗口上限：最早的 36 条消息已压成摘要（约省 180000 tokens）。"
        );

        let dropped = events(
            r#"{"text":"a","compaction":{"layer":"drop","before":200000,"after":9000,"turns":4}}"#,
            "",
            Some(0),
        );
        assert!(
            done_compaction(&dropped)
                .unwrap()
                .note()
                .contains("摘要调用失败")
        );

        // `none` and a missing report both mean "nothing happened", so no note
        // is drawn and an old engine stays compatible.
        let quiet = events(
            r#"{"text":"a","compaction":{"layer":"none","before":10,"after":10}}"#,
            "",
            Some(0),
        );
        assert!(done_compaction(&quiet).is_none());
        let absent = events(r#"{"text":"a"}"#, "", Some(0));
        assert!(done_compaction(&absent).is_none());
    }
    /// The answer text of an event list, when it ended with `Done`.
    fn done_text(events: &[AiEvent]) -> Option<&str> {
        events.iter().find_map(|event| match event {
            AiEvent::Done { text, .. } => Some(text.as_str()),
            _ => None,
        })
    }

    #[test]
    fn reads_text_from_the_bridge_shape() {
        let events = events(r#"{"text":"hello","usage":{"tokens":3}}"#, "", Some(0));
        assert_eq!(done_text(&events), Some("hello"));
    }

    /// The total token count of an event list, when the answer reported one.
    fn done_tokens(list: &[AiEvent]) -> Option<u64> {
        list.iter().find_map(|event| match event {
            AiEvent::Done { usage, .. } => usage.map(|usage| usage.total()),
            _ => None,
        })
    }

    #[test]
    fn reads_the_token_count_when_the_endpoint_reports_it() {
        let split = events(
            r#"{"text":"a","usage":{"prompt_tokens":1200,"completion_tokens":64}}"#,
            "",
            Some(0),
        );
        assert_eq!(done_tokens(&split), Some(1264));

        // Some endpoints only send the sum.
        let summed = events(r#"{"text":"a","usage":{"total_tokens":300}}"#, "", Some(0));
        assert_eq!(done_tokens(&summed), Some(300));

        // No usage in the answer line: the window shows no count at all.
        let plain = events(r#"{"text":"a"}"#, "", Some(0));
        assert_eq!(done_tokens(&plain), None);
    }

    /// The context figure of an event list, when the answer reported one.
    fn done_context(list: &[AiEvent]) -> Option<u64> {
        list.iter().find_map(|event| match event {
            AiEvent::Done { context, .. } => Some(*context),
            _ => None,
        })
    }

    /// The round-trip count of an event list, when the answer reported one.
    fn done_rounds(list: &[AiEvent]) -> Option<u32> {
        list.iter().find_map(|event| match event {
            AiEvent::Done { rounds, .. } => Some(*rounds),
            _ => None,
        })
    }

    #[test]
    fn reads_the_timing_the_engine_reports() {
        // The engine's own clock, so the panel can split a request into model,
        // tools and the leftover; an older engine reports nothing at all.
        let timed = events(
            r#"{"text":"a","timing":{"modelMs":2100,"toolMs":640}}"#,
            "",
            Some(0),
        );
        let split = timed.iter().find_map(|event| match event {
            AiEvent::Done {
                model_ms, tool_ms, ..
            } => Some((*model_ms, *tool_ms)),
            _ => None,
        });
        assert_eq!(split, Some((2100, 640)));
        let older = events(r#"{"text":"a"}"#, "", Some(0));
        let missing = older.iter().find_map(|event| match event {
            AiEvent::Done {
                model_ms, tool_ms, ..
            } => Some((*model_ms, *tool_ms)),
            _ => None,
        });
        assert_eq!(missing, Some((0, 0)));
    }

    #[test]
    fn reads_the_thinking_the_engine_reports() {
        // A reasoner bills its thinking as output; the panel keeps it apart
        // from the answer, so each round's figure has to survive the wire.
        let timed = events(r#"{"text":"a","roundReasoning":[120,0,40]}"#, "", Some(0));
        let per_round = timed.iter().find_map(|event| match event {
            AiEvent::Done {
                round_reasoning, ..
            } => Some(round_reasoning.clone()),
            _ => None,
        });
        assert_eq!(per_round, Some(vec![120, 0, 40]));
        // An engine that does not measure thinking sends nothing at all.
        let older = events(r#"{"text":"a"}"#, "", Some(0));
        let missing = older.iter().find_map(|event| match event {
            AiEvent::Done {
                round_reasoning, ..
            } => Some(round_reasoning.clone()),
            _ => None,
        });
        assert_eq!(missing, Some(Vec::new()));
    }

    #[test]
    fn reads_the_thinking_text_the_engine_streamed() {
        // Live fragments arrive as their own event, and the answer repeats the
        // whole text so a dropped fragment cannot lose it.
        let live = line_event(r#"{"type":"reasoning","text":"先想"}"#);
        assert!(matches!(live, Some(AiEvent::Reasoning(text)) if text == "先想"));
        let done = events(r#"{"text":"a","reasoning":"先想一下"}"#, "", Some(0));
        let text = done.iter().find_map(|event| match event {
            AiEvent::Done { reasoning, .. } => Some(reasoning.clone()),
            _ => None,
        });
        assert_eq!(text.as_deref(), Some("先想一下"));
        // An engine that sends none reports an empty string, not a missing one.
        let older = events(r#"{"text":"a"}"#, "", Some(0));
        let empty = older.iter().find_map(|event| match event {
            AiEvent::Done { reasoning, .. } => Some(reasoning.clone()),
            _ => None,
        });
        assert_eq!(empty.as_deref(), Some(""));
    }

    #[test]
    fn reads_the_searches_the_provider_ran_on_its_own_side() {
        // The engine turns the server tools' report into counts; the ledger keeps
        // them so the statistics page can show what searching cost.
        let counted = events(
            r#"{"text":"a","webSearch":{"searches":3,"fetches":1,"sources":[]}}"#,
            "",
            Some(0),
        );
        let counts = counted.iter().find_map(|event| match event {
            AiEvent::Done {
                web_searches,
                web_fetches,
                ..
            } => Some((*web_searches, *web_fetches)),
            _ => None,
        });
        assert_eq!(counts, Some((3, 1)));
        // No server tools: both stay at zero rather than going missing.
        let plain = events(r#"{"text":"a"}"#, "", Some(0));
        let zeros = plain.iter().find_map(|event| match event {
            AiEvent::Done {
                web_searches,
                web_fetches,
                ..
            } => Some((*web_searches, *web_fetches)),
            _ => None,
        });
        assert_eq!(zeros, Some((0, 0)));
    }

    #[test]
    fn reads_the_round_trips_the_engine_reports() {
        // The bill is a multiple of the window figure because every round sends
        // the whole prompt again; the panel needs the count to say so.
        let reported = events(
            r#"{"text":"a","usage":{"total_tokens":19156},"context":5272,"rounds":5,"roundTokens":[[3400,220,2],[3700,240,1],[4200,260,0],[4600,280,0],[5000,300,0]]}"#,
            "",
            Some(0),
        );
        assert_eq!(done_rounds(&reported), Some(5));
        // Each round's own counts come along, so the panel can show the repeated
        // prompts instead of asking the reader to multiply.
        let per_round = reported.iter().find_map(|event| match event {
            AiEvent::Done { round_tokens, .. } => Some(round_tokens.clone()),
            _ => None,
        });
        assert_eq!(
            per_round,
            Some(vec![
                (3400, 220, 2),
                (3700, 240, 1),
                (4200, 260, 0),
                (4600, 280, 0),
                (5000, 300, 0)
            ])
        );

        // An engine that predates the field counted one round trip: the answer.
        let older = events(r#"{"text":"a"}"#, "", Some(0));
        assert_eq!(done_rounds(&older), Some(1));
        assert!(older.iter().all(|event| match event {
            AiEvent::Done { round_tokens, .. } => round_tokens.is_empty(),
            _ => true,
        }));

        // A nonsense zero is read as one round, never as none.
        let zero = events(r#"{"text":"a","rounds":0}"#, "", Some(0));
        assert_eq!(done_rounds(&zero), Some(1));
    }

    #[test]
    fn reads_the_context_size_the_engine_reports() {
        // The engine sends the last request's size beside the billed total, so
        // the header can fill its gauge from the request and not the spend.
        let reported = events(
            r#"{"text":"a","usage":{"prompt_tokens":2400,"completion_tokens":110,"total_tokens":2510},"context":1460}"#,
            "",
            Some(0),
        );
        assert_eq!(done_tokens(&reported), Some(2510));
        assert_eq!(done_context(&reported), Some(1460));

        // An engine that predates the field leaves the gauge empty rather than
        // borrowing the spend figure.
        let older = events(r#"{"text":"a","usage":{"total_tokens":300}}"#, "", Some(0));
        assert_eq!(done_context(&older), Some(0));
    }

    #[test]
    fn reads_the_tool_schemas_the_engine_reports() {
        // The context panel sizes the tool definitions from this event, so it
        // has to keep the schema text, not just the count.
        let event = line_event(
            r#"{"type":"tools","items":[{"name":"read","schema":"{\"type\":\"function\"}"},{"name":"list","schema":""},{"name":"plugin_echo","schema":"{}","plugin":"demo"},{"name":"mcp_files_read","schema":"{}","server":"files"}],"prompts":[{"id":"demo","text":"先看 README"}],"errors":[{"id":"old","message":"api 是 99"},{"id":"demo","message":"起不来","kind":"mcp"}]}"#,
        );
        assert!(matches!(
            event,
            Some(AiEvent::Tools {
                ref items,
                ref prompts,
                ref errors,
                ref mcp_errors,
                ref plugin_tools,
                ref mcp_tools,
                ref panels,
            }) if items.len() == 4
                && items[0].0 == "read"
                && items[0].1.contains("function")
                && items[1].0 == "list"
                && prompts == &vec![("demo".to_string(), "先看 README".to_string())]
                && errors == &vec![("old".to_string(), "api 是 99".to_string())]
                && mcp_errors == &vec![("demo".to_string(), "起不来".to_string())]
                && plugin_tools == &vec![("demo".to_string(), "plugin_echo".to_string())]
                && mcp_tools == &vec![("files".to_string(), "mcp_files_read".to_string())]
                && panels.is_empty()
        ));
        // An engine that sends only the list still reads as an event.
        assert!(matches!(
            line_event(r#"{"type":"tools","items":[]}"#),
            Some(AiEvent::Tools { ref items, ref prompts, ref errors, .. })
                if items.is_empty() && prompts.is_empty() && errors.is_empty()
        ));
    }

    #[test]
    fn reads_plugin_panels_and_keeps_only_what_it_can_draw() {
        let event = line_event(
            r#"{"type":"tools","items":[],"panels":[{"id":"budget","title":"本次构成","plugin":"demo","summary":[{"label":"窗口","value":"64000"},{"label":"轮次","value":3}],"sections":[{"title":"按类","kind":"bars","items":[{"label":"系统","value":12,"note":"chars"},{"label":"注入","value":"3.5"}]},{"title":"明细","kind":"table","columns":["项","值"],"rows":[["a","1"]]},{"title":"说明","kind":"text","items":[{"label":"提示","text":"先看 README"}]},{"title":"未知","kind":"chart","items":[]}]}]}"#,
        );
        let Some(AiEvent::Tools { ref panels, .. }) = event else {
            panic!("expected the tools event");
        };
        assert_eq!(panels.len(), 1);
        let panel = &panels[0];
        assert_eq!(panel.id, "budget");
        assert_eq!(panel.title, "本次构成");
        assert_eq!(panel.plugin, "demo");
        assert_eq!(
            panel.summary,
            vec![
                ("窗口".to_string(), "64000".to_string()),
                ("轮次".to_string(), "3".to_string())
            ]
        );
        assert_eq!(panel.sections.len(), 4);
        assert_eq!(panel.sections[0].kind, PanelKind::Bars);
        assert_eq!(panel.sections[0].items[0].value, Some(12.0));
        assert_eq!(panel.sections[0].items[0].note, "chars");
        // A number sent as a string still draws a bar.
        assert_eq!(panel.sections[0].items[1].value, Some(3.5));
        assert_eq!(panel.sections[1].kind, PanelKind::Table);
        assert_eq!(
            panel.sections[1].columns,
            vec!["项".to_string(), "值".to_string()]
        );
        assert_eq!(
            panel.sections[1].rows,
            vec![vec!["a".to_string(), "1".to_string()]]
        );
        assert_eq!(panel.sections[2].kind, PanelKind::Text);
        assert_eq!(panel.sections[2].items[0].text, "先看 README");
        // A shape this window does not know is drawn as text, not dropped.
        assert_eq!(panel.sections[3].kind, PanelKind::Text);
        // A panel with no id cannot be identified, so it is skipped.
        assert!(matches!(
            line_event(r#"{"type":"tools","items":[],"panels":[{"title":"x"}]}"#),
            Some(AiEvent::Tools { ref panels, .. }) if panels.is_empty()
        ));
        // One that lost its heading still shows under its id.
        assert!(matches!(
            line_event(r#"{"type":"tools","items":[],"panels":[{"id":"bare"}]}"#),
            Some(AiEvent::Tools { ref panels, .. })
                if panels.len() == 1 && panels[0].title == "bare"
        ));
    }

    #[test]
    fn reads_json_answers() {
        let events = events(r#"{"json":{"files":3}}"#, "", Some(0));
        assert_eq!(done_text(&events), Some("{\"files\":3}"));
    }

    #[test]
    fn context_windows_follow_the_model_name() {
        assert_eq!(context_window("deepseek-chat"), 128_000);
        assert_eq!(context_window("DeepSeek-Reasoner"), 128_000);
        // The 2026 lineup ships a 1M window.
        assert_eq!(context_window("deepseek-flash"), 1_000_000);
        assert_eq!(context_window("deepseek-v4-pro"), 1_000_000);
        assert_eq!(context_window("claude-sonnet-4"), 200_000);
        assert_eq!(context_window("kimi-k2"), 256_000);
        assert_eq!(context_window("gpt-4.1"), 1_000_000);
        assert_eq!(context_window("gpt-4o"), 128_000);
        // Unknown ids still get a sane denominator for the gauge.
        assert_eq!(context_window("some-local-model"), DEFAULT_CONTEXT_WINDOW);
        assert_eq!(context_window(""), DEFAULT_CONTEXT_WINDOW);
    }

    #[test]
    fn falls_back_to_plain_text() {
        let events = events("plain answer", "", Some(0));
        assert_eq!(done_text(&events), Some("plain answer"));
    }

    #[test]
    fn reports_engine_errors() {
        let events = events("", "unknown command: ai", Some(1));
        let error = events
            .iter()
            .find_map(|event| match event {
                AiEvent::Failed(error) => Some(error.clone()),
                _ => None,
            })
            .unwrap();
        assert!(error.contains("unknown command"), "{error}");
    }

    #[test]
    fn reads_the_loops_progress_frames() {
        let stdout = concat!(
            "{\"type\":\"progress\",\"round\":1,\"calls\":0}\n",
            "{\"type\":\"progress\",\"round\":2,\"calls\":3}\n",
            "{\"text\":\"done\",\"model\":\"deepseek-chat\",\"rounds\":2,\"roundTokens\":[[100,10,3],[120,12,0]]}\n"
        );
        let events = events(stdout, "", Some(0));
        let progress: Vec<(u32, u32)> = events
            .iter()
            .filter_map(|event| match event {
                AiEvent::Progress { round, calls } => Some((*round, *calls)),
                _ => None,
            })
            .collect();
        assert_eq!(progress, vec![(1, 0), (2, 3)]);
        // The frames are skipped by everything that only reads the answer.
        assert_eq!(done_text(&events), Some("done"));
        let done_rounds: Vec<u32> = events
            .iter()
            .filter_map(|event| match event {
                AiEvent::Done { round_tokens, .. } => {
                    Some(round_tokens.iter().map(|(_, _, calls)| *calls).sum())
                }
                _ => None,
            })
            .collect();
        assert_eq!(done_rounds, vec![3], "the calls sum to the same three");
    }

    #[test]
    fn reads_tool_steps_before_the_answer() {
        let stdout = concat!(
            "{\"type\":\"step\",\"tool\":\"read\",\"target\":\"a.txt\",\"ok\":true,\"detail\":\"1\\talpha\",\"input\":\"{\\\"path\\\":\\\"a.txt\\\"}\",\"output\":\"1\\talpha\\n\"}\n",
            "{\"type\":\"step\",\"tool\":\"write\",\"target\":\"b.txt\",\"ok\":false,\"detail\":\"路径超出工作目录\"}\n",
            "{\"text\":\"done\",\"model\":\"deepseek-chat\"}\n"
        );
        let events = events(stdout, "", Some(0));
        assert_eq!(events.len(), 3);
        match &events[0] {
            AiEvent::Step {
                tool,
                target,
                ok,
                detail,
                input,
                output,
                ..
            } => {
                assert_eq!(tool, "read");
                assert_eq!(target, "a.txt");
                assert!(*ok);
                assert_eq!(detail, "1\talpha");
                assert!(input.contains("a.txt"));
                assert!(output.contains("alpha"));
            }
            _ => panic!("first event should be a step"),
        }
        match &events[1] {
            AiEvent::Step { ok, detail, .. } => {
                assert!(!*ok);
                assert!(detail.contains("超出"));
            }
            _ => panic!("second event should be a step"),
        }
        assert_eq!(done_text(&events), Some("done"));
    }

    #[test]
    fn reads_streaming_fragments() {
        let stdout = concat!(
            "{\"type\":\"delta\",\"text\":\"你\"}\n",
            "{\"type\":\"delta\",\"text\":\"好\"}\n",
            "{\"text\":\"你好\"}\n"
        );
        let events = events(stdout, "", Some(0));
        assert_eq!(events.len(), 3);
        assert!(matches!(&events[0], AiEvent::Delta(text) if text == "你"));
        assert!(matches!(&events[1], AiEvent::Delta(text) if text == "好"));
        assert_eq!(done_text(&events), Some("你好"));
        assert!(line_event("{\"type\":\"delta\",\"text\":\"\"}").is_none());
    }

    #[test]
    fn reads_a_single_line_at_a_time() {
        assert!(line_event("not ours").is_none());
        assert!(line_event("{\"type\":\"step\",\"tool\":\"glob\",\"target\":\"*.ts\"}").is_some());
        match line_event("{\"text\":\"hi\"}") {
            Some(AiEvent::Done { text, .. }) => assert_eq!(text, "hi"),
            _ => panic!("an answer line should be done"),
        }
    }

    #[test]
    fn reads_a_live_tool_output_line() {
        match line_event(
            "{\"type\":\"tool_output\",\"call\":\"c1\",\"tool\":\"shell\",\"target\":\"npm test\",\"text\":\"ok\\n\"}",
        ) {
            Some(AiEvent::ToolOutput {
                call,
                tool,
                target,
                text,
            }) => {
                assert_eq!(call, "c1");
                assert_eq!(tool, "shell");
                assert_eq!(target, "npm test");
                assert_eq!(text, "ok\n");
            }
            _ => panic!("a tool_output line should be its own event"),
        }
        assert!(line_event("{\"type\":\"tool_output\",\"text\":\"\"}").is_none());
    }

    #[test]
    fn reads_a_tool_start_line() {
        match line_event(
            "{\"type\":\"tool_start\",\"call\":\"c1\",\"tool\":\"shell\",\"target\":\"npm test\"}",
        ) {
            Some(AiEvent::ToolStart { call, tool, target }) => {
                assert_eq!(call, "c1");
                assert_eq!(tool, "shell");
                assert_eq!(target, "npm test");
            }
            _ => panic!("a tool_start line should be its own event"),
        }
        assert!(
            line_event("{\"type\":\"future_event\",\"text\":\"x\"}").is_none(),
            "an unknown event is ignored, not mistaken for an answer"
        );
    }

    #[test]
    fn sends_the_conversation_with_the_question() {
        let connection = Connection::from_preset("deepseek").expect("preset");
        let history = vec![
            Turn {
                speaker: Speaker::User,
                text: "这里有什么文件？".to_string(),
                server_tool_calls: Vec::new(),
            },
            Turn {
                speaker: Speaker::Assistant,
                text: "看 a.txt".to_string(),
                server_tool_calls: Vec::new(),
            },
        ];
        let text = payload(
            "那它呢？",
            Some("简短回答"),
            &history,
            &connection,
            true,
            None,
            false,
            None,
            &[],
        );
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["prompt"], "那它呢？");
        assert_eq!(value["system"], "简短回答");
        assert_eq!(value["tools"], true);
        assert_eq!(value["confirmWrites"], false);
        assert_eq!(value["fileScope"], "full");
        assert_eq!(value["timeoutMs"], CHAT_TIMEOUT_MS);
        assert_eq!(
            value["windowTokens"],
            serde_json::json!(crate::ai::context_window(&connection.model)),
            "without an override the model name table answers"
        );
        assert_eq!(value["model"], connection.model.as_str());
        assert_eq!(
            value["history"],
            serde_json::json!([
                { "role": "user", "content": "这里有什么文件？" },
                { "role": "assistant", "content": "看 a.txt" },
            ])
        );
    }

    #[test]
    fn an_explicit_window_rides_with_the_request() {
        // A local model the table does not know would otherwise be treated as
        // 128k; the connection's own figure is what the engine must compact by.
        let mut connection = Connection::from_preset("deepseek").expect("preset");
        connection.model = "some-local-model".to_string();
        connection.window = 32_768;
        let value: serde_json::Value = serde_json::from_str(&payload(
            "hi",
            None,
            &[],
            &connection,
            true,
            None,
            false,
            None,
            &[],
        ))
        .unwrap();
        assert_eq!(value["windowTokens"], 32_768);
    }

    #[test]
    fn sampling_rides_along_when_it_is_not_the_endpoint_default() {
        let mut connection = Connection::from_preset("deepseek").expect("preset");
        let value: serde_json::Value = serde_json::from_str(&payload(
            "hi",
            None,
            &[],
            &connection,
            false,
            None,
            false,
            None,
            &[],
        ))
        .unwrap();
        assert!(
            value.get("temperature").is_none(),
            "the default sends nothing"
        );

        connection.temperature = Some(0.2);
        let value: serde_json::Value = serde_json::from_str(&payload(
            "hi",
            None,
            &[],
            &connection,
            false,
            None,
            false,
            None,
            &[],
        ))
        .unwrap();
        assert_eq!(value["temperature"], 0.2);
    }

    #[test]
    fn max_tokens_rides_along_only_when_it_is_set() {
        let mut connection = Connection::from_preset("deepseek").expect("preset");
        let value: serde_json::Value = serde_json::from_str(&payload(
            "hi",
            None,
            &[],
            &connection,
            false,
            None,
            false,
            None,
            &[],
        ))
        .unwrap();
        assert!(value.get("maxTokens").is_none());

        connection.max_tokens = 4096;
        let value: serde_json::Value = serde_json::from_str(&payload(
            "hi",
            None,
            &[],
            &connection,
            false,
            None,
            false,
            None,
            &[],
        ))
        .unwrap();
        assert_eq!(value["maxTokens"], 4096);
    }

    #[test]
    fn the_thinking_choice_rides_along_only_when_it_is_not_the_default() {
        let mut connection = Connection::from_preset("deepseek").expect("preset");
        let value: serde_json::Value = serde_json::from_str(&payload(
            "hi",
            None,
            &[],
            &connection,
            false,
            None,
            false,
            None,
            &[],
        ))
        .unwrap();
        assert!(
            value.get("reasoning").is_none(),
            "the default leaves thinking to the endpoint"
        );

        for (choice, name) in [
            (crate::config::Reasoning::Off, "off"),
            (crate::config::Reasoning::Low, "low"),
            (crate::config::Reasoning::High, "high"),
            (crate::config::Reasoning::Max, "max"),
        ] {
            connection.reasoning = choice;
            let value: serde_json::Value = serde_json::from_str(&payload(
                "hi",
                None,
                &[],
                &connection,
                false,
                None,
                false,
                None,
                &[],
            ))
            .unwrap();
            assert_eq!(value["reasoning"], name);
        }
    }

    #[test]
    fn web_search_is_sent_only_where_the_endpoint_offers_it() {
        // Searching is on by default: the request asks for it wherever the
        // endpoint has the tools, and leaves them out where it does not.
        let mut connection = Connection::from_preset("deepseek").expect("preset");
        let offered = payload_with_budgets(
            "hi",
            None,
            &[],
            &connection,
            false,
            None,
            false,
            None,
            &[],
            8,
            20,
            false,
            true,
            &[],
            None,
            &[],
            &[],
            None,
        );
        let value: serde_json::Value = serde_json::from_str(&offered).unwrap();
        assert_eq!(value["webSearch"], true);
        // The conversation switched it off for itself.
        let off = payload_with_budgets(
            "hi",
            None,
            &[],
            &connection,
            false,
            None,
            false,
            None,
            &[],
            8,
            20,
            false,
            false,
            &[],
            None,
            &[],
            &[],
            None,
        );
        let value: serde_json::Value = serde_json::from_str(&off).unwrap();
        assert!(value.get("webSearch").is_none(), "off sends nothing");

        connection.model = "deepseek-v4-pro".to_string();
        let unsupported = payload_with_budgets(
            "hi",
            None,
            &[],
            &connection,
            false,
            None,
            false,
            None,
            &[],
            8,
            20,
            false,
            true,
            &[],
            None,
            &[],
            &[],
            None,
        );
        let value: serde_json::Value = serde_json::from_str(&unsupported).unwrap();
        assert!(
            value.get("webSearch").is_none(),
            "only flash offers the tools"
        );
    }

    #[test]
    fn hands_the_server_calls_back_with_the_assistant_turn() {
        // The provider restores the pages it read from these; a framework that
        // drops them still gets an answer, but without that context.
        let connection = Connection::from_preset("deepseek").expect("preset");
        let call = serde_json::json!({
            "id": "search_1",
            "kind": "search",
            "action": { "type": "search", "queries": ["x"] },
            "status": "completed",
            "sources": [{ "title": "X", "url": "https://example.com/x" }]
        });
        let history = vec![
            Turn {
                speaker: Speaker::User,
                text: "问".to_string(),
                server_tool_calls: Vec::new(),
            },
            Turn {
                speaker: Speaker::Assistant,
                text: "答".to_string(),
                server_tool_calls: vec![call.clone()],
            },
        ];
        let value: serde_json::Value = serde_json::from_str(&payload(
            "下一个问题",
            None,
            &history,
            &connection,
            false,
            None,
            false,
            None,
            &[],
        ))
        .unwrap();
        assert!(
            value["history"][0].get("serverToolCalls").is_none(),
            "a user turn has none"
        );
        assert_eq!(
            value["history"][1]["serverToolCalls"],
            serde_json::json!([call])
        );
    }

    #[test]
    fn lists_reference_roots_and_allowed_tools_only_when_tools_are_on() {
        let connection = Connection::from_preset("deepseek").expect("preset");
        let roots = [PathBuf::from("D:\\engine"), PathBuf::from("D:\\flows")];
        let with_tools = payload(
            "hi",
            None,
            &[],
            &connection,
            true,
            None,
            false,
            None,
            &roots,
        );
        let value: serde_json::Value = serde_json::from_str(&with_tools).unwrap();
        assert_eq!(
            value["readOnlyRoots"],
            serde_json::json!(["D:\\engine", "D:\\flows"])
        );
        assert_eq!(value["maxRounds"], DEFAULT_MAX_ROUNDS);
        assert_eq!(value["maxCalls"], DEFAULT_MAX_CALLS);

        let without = payload(
            "hi",
            None,
            &[],
            &connection,
            false,
            None,
            false,
            None,
            &roots,
        );
        let value: serde_json::Value = serde_json::from_str(&without).unwrap();
        assert!(value.get("readOnlyRoots").is_none());

        let limited = payload(
            "hi",
            None,
            &[],
            &connection,
            true,
            Some(&["read", "glob"][..]),
            true,
            Some(Path::new("D:\\workspace")),
            &[],
        );
        let value: serde_json::Value = serde_json::from_str(&limited).unwrap();
        assert_eq!(value["allowedTools"], serde_json::json!(["read", "glob"]));
        assert_eq!(value["confirmWrites"], true);
        assert_eq!(value["fileScope"], "workspace");
        assert_eq!(value["scopeRoot"], "D:\\workspace");

        let custom = payload_with_budgets(
            "hi",
            None,
            &[],
            &connection,
            true,
            None,
            false,
            None,
            &[],
            12,
            40,
            false,
            true,
            &[],
            None,
            &[],
            &[],
            None,
        );
        let value: serde_json::Value = serde_json::from_str(&custom).unwrap();
        assert_eq!(value["maxRounds"], 12);
        assert_eq!(value["maxCalls"], 40);

        // Plugins: the folder and the names someone switched on, nothing else.
        let with_plugins = payload_with_budgets(
            "hi",
            None,
            &[],
            &connection,
            true,
            None,
            false,
            None,
            &[],
            DEFAULT_MAX_ROUNDS,
            DEFAULT_MAX_CALLS,
            false,
            true,
            &[],
            Some(Path::new(
                "C:\\Users\\me\\AppData\\Roaming\\dsh-flow-ui\\plugins",
            )),
            &[
                "demo".to_string(),
                "../evil".to_string(),
                "".to_string(),
                "with space".to_string(),
            ],
            &[],
            None,
        );
        let value: serde_json::Value = serde_json::from_str(&with_plugins).unwrap();
        assert_eq!(
            value["pluginDir"],
            "C:\\Users\\me\\AppData\\Roaming\\dsh-flow-ui\\plugins"
        );
        // Only names that are folder names reach the engine.
        assert_eq!(value["plugins"], serde_json::json!(["demo"]));
    }

    #[test]
    fn sends_enabled_mcp_servers_only_when_tools_are_on() {
        let connection = Connection::from_preset("deepseek").expect("preset");
        let server = crate::mcp::McpServer {
            id: "demo".to_string(),
            name: "演示".to_string(),
            version: "1.0.0".to_string(),
            description: String::new(),
            transport: "stdio".to_string(),
            command: "node".to_string(),
            args: vec!["server.js".to_string()],
            env: vec![("TOKEN".to_string(), "x".to_string())],
            url: String::new(),
            headers: Vec::new(),
            cwd: "E:\\mcp\\demo".to_string(),
            problem: String::new(),
            source: PathBuf::from("E:\\mcp\\demo\\mcp.json"),
            skills: Vec::new(),
        };
        let with_mcp = payload_with_budgets(
            "hi",
            None,
            &[],
            &connection,
            true,
            None,
            false,
            None,
            &[],
            DEFAULT_MAX_ROUNDS,
            DEFAULT_MAX_CALLS,
            false,
            true,
            &[],
            None,
            &[],
            std::slice::from_ref(&server),
            Some(Path::new("E:\\mcpserver")),
        );
        let value: serde_json::Value = serde_json::from_str(&with_mcp).unwrap();
        assert_eq!(value["mcpServers"][0]["id"], "demo");
        assert_eq!(value["mcpServers"][0]["command"], "node");
        assert_eq!(
            value["mcpServers"][0]["args"],
            serde_json::json!(["server.js"])
        );
        assert_eq!(value["mcpServers"][0]["cwd"], "E:\\mcp\\demo");
        assert_eq!(value["mcpDir"], "E:\\mcpserver");

        let prose = payload_with_budgets(
            "hi",
            None,
            &[],
            &connection,
            false,
            None,
            false,
            None,
            &[],
            DEFAULT_MAX_ROUNDS,
            DEFAULT_MAX_CALLS,
            false,
            true,
            &[],
            None,
            &[],
            std::slice::from_ref(&server),
            None,
        );
        let value: serde_json::Value = serde_json::from_str(&prose).unwrap();
        assert!(value.get("mcpServers").is_none());
        assert!(value.get("mcpDir").is_none());
    }

    #[test]
    fn leaves_history_out_when_there_is_none() {
        let connection = Connection::from_preset("deepseek").expect("preset");
        let value: serde_json::Value = serde_json::from_str(&payload(
            "hi",
            None,
            &[],
            &connection,
            false,
            None,
            false,
            None,
            &[],
        ))
        .unwrap();
        assert!(value.get("history").is_none());
        assert!(value.get("system").is_none());
        assert!(value.get("tools").is_none());
    }

    #[test]
    fn keeps_steps_when_the_answer_never_arrives() {
        let stdout = "{\"type\":\"step\",\"tool\":\"read\",\"target\":\"a.txt\",\"ok\":true}\n";
        let events = events(stdout, "boom", Some(1));
        assert_eq!(events.len(), 2);
        assert!(matches!(events[1], AiEvent::Failed(ref error) if error.contains("boom")));
    }

    /// Poll until an event of interest appears, or fail after a few seconds.
    fn wait_for(request: &AiRequest, mut wanted: impl FnMut(&AiEvent) -> bool) -> AiEvent {
        let deadline = Instant::now() + std::time::Duration::from_secs(8);
        let mut seen: Vec<String> = Vec::new();
        while Instant::now() < deadline {
            for event in request.poll() {
                if wanted(&event) {
                    return event;
                }
                seen.push(match &event {
                    AiEvent::Step { tool, .. } => format!("step:{tool}"),
                    AiEvent::ToolStart { tool, .. } => format!("start:{tool}"),
                    AiEvent::ToolOutput { tool, .. } => format!("output:{tool}"),
                    AiEvent::Approval { tool, .. } => format!("approval:{tool}"),
                    AiEvent::Delta(text) => format!("delta:{text}"),
                    AiEvent::Reasoning(text) => format!("reasoning:{text}"),
                    AiEvent::Tools { items, .. } => format!("tools:{}", items.len()),
                    AiEvent::Progress { round, calls } => format!("progress:{round}:{calls}"),
                    AiEvent::Done { text, .. } => format!("done:{text}"),
                    AiEvent::Failed(error) => format!("failed:{error}"),
                    AiEvent::Question { question, .. } => format!("question:{question}"),
                    AiEvent::Plan { .. } => "plan".to_string(),
                    AiEvent::Todo { items } => format!("todo:{}", items.len()),
                });
            }
            thread::sleep(std::time::Duration::from_millis(25));
        }
        panic!("timed out; saw {seen:?}");
    }

    /// The window's compaction call reaches the engine's compact command and
    /// keeps the checkpoint it writes; a refusal is a message, never a panic.
    #[test]
    fn condensation_reaches_the_engine_compact_command() {
        let engine =
            std::env::temp_dir().join(format!("dsh-flow-ui-compact-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&engine);
        std::fs::create_dir_all(engine.join("src")).unwrap();
        std::fs::write(engine.join("package.json"), "{\"type\":\"module\"}").unwrap();
        std::fs::write(
            engine.join("src").join("cli.ts"),
            concat!(
                "let raw = ''\n",
                "for await (const chunk of process.stdin) raw += chunk\n",
                "const request = JSON.parse(raw)\n",
                "const ok = process.argv[2] === 'compact' && request.history.length === 2\n",
                "console.log(JSON.stringify(ok ? { summary: '## 摘要', model: 'm' } : { error: 'boom' }))\n",
            ),
        )
        .unwrap();

        let connection = Connection::from_preset("deepseek").expect("preset");
        let turns = vec![
            Turn {
                speaker: Speaker::User,
                text: "问题".to_string(),
                server_tool_calls: Vec::new(),
            },
            Turn {
                speaker: Speaker::Assistant,
                text: "回答".to_string(),
                server_tool_calls: Vec::new(),
            },
        ];
        let compacted = compact_span(&engine, &connection, &turns, "指令").expect("a checkpoint");
        assert_eq!(compacted.summary, "## 摘要");
        assert_eq!(compacted.model, "m");

        // An engine that refuses is reported as a readable message.
        let error = compact_span(&engine, &connection, &turns[..1], "指令").unwrap_err();
        assert!(error.contains("boom"), "{error}");
        let _ = std::fs::remove_dir_all(&engine);
    }

    #[test]
    fn compact_answer_reads_a_checkpoint_and_a_failure() {
        let good = compact_answer(r#"{"summary":"  ## 摘要 ","model":"m"}"#, "", Some(0))
            .expect("a checkpoint");
        assert_eq!(good.summary, "## 摘要");
        let failed = compact_answer(r#"{"error":"超时"}"#, "", Some(1)).unwrap_err();
        assert!(failed.contains("超时"), "{failed}");
        let quiet = compact_answer("", "", Some(1)).unwrap_err();
        assert!(quiet.contains("压缩没有返回内容"), "{quiet}");
    }
    /// A step has to reach the window while the child is still running, so the
    /// transcript fills in as the assistant works instead of all at once.
    #[test]
    fn streams_a_step_before_the_process_exits() {
        let engine =
            std::env::temp_dir().join(format!("dsh-flow-ui-stream-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&engine);
        std::fs::create_dir_all(engine.join("src")).unwrap();
        std::fs::write(engine.join("package.json"), "{\"type\":\"module\"}").unwrap();
        std::fs::write(
            engine.join("src").join("cli.ts"),
            concat!(
                "import { existsSync } from 'node:fs'\n",
                "console.log(JSON.stringify({ type: 'step', tool: 'read', target: 'a.txt', ok: true, detail: '已读取 2 行' }))\n",
                "const started = Date.now()\n",
                "while (!existsSync('go.txt') && Date.now() - started < 6000) {\n",
                "  await new Promise((resolve) => setTimeout(resolve, 20))\n",
                "}\n",
                "console.log(JSON.stringify({ text: 'done' }))\n",
            ),
        )
        .unwrap();

        let connection = Connection::from_preset("deepseek").expect("preset");
        let request = AiRequest::ask(
            &engine,
            &engine,
            "hi",
            None,
            &[],
            &connection,
            true,
            None,
            false,
            None,
            &[],
            DEFAULT_MAX_ROUNDS,
            DEFAULT_MAX_CALLS,
            false,
            true,
            &[],
            None,
            &[],
            &[],
            None,
        )
        .expect("spawn");

        // The stub child blocks until `go.txt` exists, so the step can only be
        // seen early if stdout is read line by line.
        let step = wait_for(&request, |event| matches!(event, AiEvent::Step { .. }));
        match step {
            AiEvent::Step {
                tool,
                target,
                ok,
                detail,
                ..
            } => {
                assert_eq!(tool, "read");
                assert_eq!(target, "a.txt");
                assert!(ok);
                assert!(detail.contains("2 行"), "{detail}");
            }
            _ => unreachable!(),
        }

        std::fs::write(engine.join("go.txt"), "").unwrap();
        let answer = wait_for(&request, |event| {
            matches!(event, AiEvent::Done { .. } | AiEvent::Failed(_))
        });
        assert!(
            matches!(answer, AiEvent::Done { ref text, .. } if text == "done"),
            "answer"
        );
        let _ = std::fs::remove_dir_all(&engine);
    }

    /// A direct write still reports its undo pair, marked `auto`, and never
    /// waits for a reply.
    #[test]
    fn parses_a_direct_write_approval() {
        let stdout = concat!(
            "{\"type\":\"approval\",\"auto\":true,\"call\":\"call-1\",\"tool\":\"write\",",
            "\"target\":\"a.txt\",\"path\":\"a.txt\",\"diff\":\"+ hi\",\"before\":\"\",",
            "\"after\":\"hi\",\"existed\":false,\"undoable\":true}\n",
            "{\"text\":\"written\"}\n",
        );
        let parsed = events(stdout, "", Some(0));
        match &parsed[0] {
            AiEvent::Approval {
                call,
                after,
                undoable,
                auto,
                ..
            } => {
                assert_eq!(call, "call-1");
                assert_eq!(after, "hi");
                assert!(undoable);
                assert!(auto);
            }
            _ => panic!("expected an approval"),
        }
        assert!(matches!(parsed[1], AiEvent::Done { ref text, .. } if text == "written"));
    }

    /// The approval reply has to travel back over the same pipe the request
    /// arrived on: the engine is paused until the window decides.
    #[test]
    fn sends_an_approval_decision_back_to_the_engine() {
        let engine =
            std::env::temp_dir().join(format!("dsh-flow-ui-approve-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&engine);
        std::fs::create_dir_all(engine.join("src")).unwrap();
        std::fs::write(engine.join("package.json"), "{\"type\":\"module\"}").unwrap();
        std::fs::write(
            engine.join("src").join("cli.ts"),
            concat!(
                "import { createInterface } from 'node:readline'\n",
                "const lines = createInterface({ input: process.stdin })[Symbol.asyncIterator]()\n",
                "await lines.next()\n",
                "console.log(JSON.stringify({ type: 'approval', call: 'call-1', tool: 'write', target: 'a.txt', path: 'a.txt', diff: '+ hi', before: '', after: 'hi', existed: false, undoable: true }))\n",
                "const decision = JSON.parse((await lines.next()).value)\n",
                "console.log(JSON.stringify({ text: decision.allow ? 'allowed' : 'denied' }))\n",
            ),
        )
        .unwrap();

        let connection = Connection::from_preset("deepseek").expect("preset");
        let request = AiRequest::ask(
            &engine,
            &engine,
            "hi",
            None,
            &[],
            &connection,
            true,
            None,
            true,
            None,
            &[],
            DEFAULT_MAX_ROUNDS,
            DEFAULT_MAX_CALLS,
            false,
            true,
            &[],
            None,
            &[],
            &[],
            None,
        )
        .expect("spawn");

        let approval = wait_for(&request, |event| matches!(event, AiEvent::Approval { .. }));
        match approval {
            AiEvent::Approval {
                call,
                tool,
                target,
                path,
                diff,
                before,
                after,
                existed,
                undoable,
                auto,
                command,
            } => {
                assert_eq!(call, "call-1");
                assert_eq!(command, "");
                assert_eq!(tool, "write");
                assert_eq!(target, "a.txt");
                assert_eq!(path, "a.txt");
                assert_eq!(diff, "+ hi");
                assert_eq!(before, "");
                assert_eq!(after, "hi");
                assert!(!existed);
                assert!(undoable);
                assert!(!auto);
            }
            _ => unreachable!(),
        }
        request.respond("call-1", true);
        let answer = wait_for(&request, |event| {
            matches!(event, AiEvent::Done { .. } | AiEvent::Failed(_))
        });
        assert!(
            matches!(answer, AiEvent::Done { ref text, .. } if text == "allowed"),
            "answer"
        );
        let _ = std::fs::remove_dir_all(&engine);
    }

    #[test]
    fn reports_a_missing_engine() {
        let engine = std::env::temp_dir().join("dsh-flow-ui-ai-missing");
        let connection = Connection::from_preset("deepseek").expect("preset");
        let error = AiRequest::ask(
            &engine,
            &engine,
            "hi",
            None,
            &[],
            &connection,
            false,
            None,
            false,
            None,
            &[],
            DEFAULT_MAX_ROUNDS,
            DEFAULT_MAX_CALLS,
            false,
            true,
            &[],
            None,
            &[],
            &[],
            None,
        )
        .err()
        .unwrap();
        assert!(error.contains("找不到引擎"), "{error}");
    }

    #[test]
    fn base64_uses_the_standard_alphabet_and_padding() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64_encode(&[0, 255, 128]), "AP+A");
    }

    #[test]
    fn sends_binary_attachments_with_the_question() {
        let connection = Connection::from_preset("deepseek").expect("preset");
        let files = [AskAttachment::new("a.png", "image/png", "image", b"hi")];
        let body = payload_with_budgets(
            "hi",
            None,
            &[],
            &connection,
            false,
            None,
            false,
            None,
            &[],
            DEFAULT_MAX_ROUNDS,
            DEFAULT_MAX_CALLS,
            false,
            true,
            &files,
            None,
            &[],
            &[],
            None,
        );
        let value: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(
            value["attachments"],
            serde_json::json!([{
                "name": "a.png",
                "kind": "image",
                "mimeType": "image/png",
                "data": "aGk="
            }])
        );
        let plain: serde_json::Value = serde_json::from_str(&payload(
            "hi",
            None,
            &[],
            &connection,
            false,
            None,
            false,
            None,
            &[],
        ))
        .unwrap();
        assert!(plain.get("attachments").is_none(), "no files, no field");
    }

    #[test]
    fn reads_the_model_list_shape() {
        let list =
            model_list(r#"{"models":["deepseek-chat","deepseek-v4"]}"#, "", Some(0)).unwrap();
        assert_eq!(list.models, vec!["deepseek-chat", "deepseek-v4"]);
        assert!(list.note.is_none());
    }

    #[test]
    fn a_degraded_listing_says_why_it_holds_one_name() {
        let list = model_list(
            r#"{"models":["deepseek-chat"],"degraded":true,"reason":"HTTP 404"}"#,
            "",
            Some(0),
        )
        .unwrap();
        assert_eq!(list.models, vec!["deepseek-chat"]);
        let note = list.note.expect("note");
        assert!(note.contains("HTTP 404"), "{note}");
        assert!(note.contains("探测通过"), "{note}");
    }

    #[test]
    fn reports_model_list_errors() {
        let error = model_list(r#"{"error":"invalid api key"}"#, "", Some(1)).unwrap_err();
        assert!(error.contains("invalid api key"), "{error}");
        assert!(model_list(r#"{"models":[]}"#, "", Some(0)).is_err());
        assert!(
            model_list("", "boom", Some(2))
                .unwrap_err()
                .contains("boom")
        );
    }

    #[test]
    fn reports_a_missing_engine_for_model_lists() {
        let engine = std::env::temp_dir().join("dsh-flow-ui-ai-missing");
        let connection = Connection::from_preset("deepseek").expect("preset");
        let error = list_models(&engine, &connection).err().unwrap();
        assert!(error.contains("找不到引擎"), "{error}");
    }
}
