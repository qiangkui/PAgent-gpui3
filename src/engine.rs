//! Bridge to the `dsh-flow` engine.
//!
//! The UI owns no workflow logic: it discovers manifests on disk and runs them
//! through the real `dsh-flow` CLI, reading the JSON summary it prints. That
//! keeps one implementation of the engine and lets the UI stay a viewer.
//!
//! A run is streamed: stderr reaches the log pane while the engine works, and
//! the child process stays reachable so the window can cancel it.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use crate::manifest::{self, ManifestSpec};

/// One workflow manifest found on disk.
#[derive(Clone)]
pub struct WorkflowEntry {
    /// `name:` from the manifest, or the folder name.
    pub name: String,
    /// `description:` from the manifest.
    pub description: String,
    /// Absolute path to the manifest file.
    pub path: PathBuf,
    /// Fields the plan pane shows before a run.
    pub spec: ManifestSpec,
}

impl WorkflowEntry {
    /// Path relative to the scan root, for compact display.
    pub fn label(&self, root: &Path) -> String {
        self.path
            .strip_prefix(root)
            .unwrap_or(&self.path)
            .display()
            .to_string()
    }
}

/// One step inside a finished run.
#[derive(Clone, Debug)]
pub struct StepView {
    /// Step id.
    pub id: String,
    /// Executor kind: run / set / map / ai.
    pub kind: String,
    /// ok / failed / skipped.
    pub status: String,
    /// Wall-clock duration in milliseconds.
    pub duration_ms: u64,
    /// Failure or skip reason.
    pub detail: String,
    /// Rendered command line, when the step ran a program.
    pub command: Option<String>,
    /// Captured stderr, when the engine reports it.
    pub stderr: Option<String>,
    /// Captured stdout, when the engine reports it.
    pub stdout: Option<String>,
    /// Answer text, when an `ai` step answered.
    pub text: Option<String>,
    /// Model that answered, for `ai` steps.
    pub model: Option<String>,
    /// True when the answer came from the recorded decision cache.
    pub cached: bool,
    /// The step's own one-line result, which the card shows first.
    pub summary: Option<String>,
    /// Files and pictures the step produced.
    pub artifacts: Vec<ArtifactView>,
    /// Structured failure, when the step reported one.
    pub error: Option<ErrorView>,
}

/// The settled state of one run.
#[derive(Clone, Debug)]
pub struct RunOutcome {
    /// True when every step that mattered succeeded.
    pub ok: bool,
    /// Engine-assigned run id.
    pub run_id: String,
    /// Total duration in milliseconds.
    pub duration_ms: u64,
    /// Steps in the order the engine reported them.
    pub steps: Vec<StepView>,
    /// Failure message for the run as a whole.
    pub error: Option<String>,
    /// Path of the HTML report, when the engine wrote one.
    pub report: Option<String>,
    /// Process exit code.
    pub exit_code: Option<i32>,
}

impl RunOutcome {
    /// Count of steps in the given status.
    pub fn count(&self, status: &str) -> usize {
        self.steps
            .iter()
            .filter(|step| step.status == status)
            .count()
    }
}

/// Credentials one run may use for its `ai` steps.
///
/// They travel on the child's stdin, never on the command line and never in the
/// log pane, so a workflow run can call the same verified endpoint the chat uses
/// without writing the key anywhere.
pub struct RunCredentials {
    /// Secret sent as the bearer token.
    pub api_key: String,
    /// Endpoint base, without `/chat/completions`.
    pub base_url: String,
    /// Model id used when a step does not name one.
    pub model: String,
}

/// One file or picture a step handed over.
#[derive(Clone, Debug, Default)]
pub struct ArtifactView {
    /// `file` or `image`.
    pub kind: String,
    /// Name the card shows.
    pub name: String,
    /// Absolute path.
    pub path: String,
    /// Size in bytes.
    pub bytes: u64,
}

/// Why a step failed, as the step itself described it.
#[derive(Clone, Debug, Default)]
pub struct ErrorView {
    /// `credential`, `selector-miss`, `assert`… (see the engine's vocabulary).
    pub kind: String,
    /// One sentence: what happened.
    pub message: String,
    /// One sentence: what to do about it.
    pub hint: Option<String>,
    /// Where it happened, for example `maers-login:read`.
    pub location: Option<String>,
    /// What was expected, when the failure is a comparison.
    pub expected: Option<String>,
    /// What was actually found.
    pub gotten: Option<String>,
}

/// One line of the engine's live event stream (`--events`).
#[derive(Clone, Debug, Default)]
pub struct StepEvent {
    /// `run/start`, `step/start`, `step/end`, `run/end`.
    pub kind: String,
    /// Step id, empty for run-level events.
    pub id: String,
    /// Executor of the step: `run`, `ai`, `set`, `map`, `use`.
    pub step_kind: String,
    /// `running` for a start, otherwise the settled status.
    pub status: String,
    /// How long it took, for a finished step.
    pub duration_ms: u64,
    /// The step's own one-line result.
    pub summary: Option<String>,
    /// Failure text or skip reason.
    pub detail: Option<String>,
    /// Rendered command line.
    pub command: Option<String>,
    /// Files and pictures the step produced.
    pub artifacts: Vec<ArtifactView>,
    /// Structured failure, when the step reported one.
    pub error: Option<ErrorView>,
}

/// One message from a running engine.
pub enum RunMessage {
    /// A line written to stderr by the engine.
    Line(String),
    /// One record from the `--events` stream.
    Event(StepEvent),
    /// The engine finished and produced a summary.
    Done(Box<RunOutcome>),
    /// The engine could not be read.
    Failed(String),
}

/// A live engine process: progress lines in, summary out.
pub struct RunSession {
    queue: Arc<Mutex<Vec<RunMessage>>>,
    child: Arc<Mutex<Option<Child>>>,
    cancelled: Arc<AtomicBool>,
    started: Instant,
}

/// Lock a shared slot, keeping it usable after a thread panicked.
///
/// A poisoned lock is not a reason to lose the window: the data inside is still
/// a valid list or slot, and panicking here would take the whole app with it.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Kill one process and, on Windows, everything it started.
///
/// The engine may have a `shell` step running under it; killing only `node`
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
            .creation_flags(0x0800_0000)
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

impl RunSession {
    /// Spawn `node src/cli.ts run <manifest> --json`.
    ///
    /// `inputs` become repeated `--input name=value` flags, so the engine stays
    /// the only place that decides what an input means. `credentials`, when
    /// present, are written to the child's stdin so `ai` steps can call the
    /// verified endpoint without the key appearing in argv or the log.
    pub fn start(
        engine_dir: &Path,
        manifest: &Path,
        inputs: &[(String, String)],
        credentials: Option<&RunCredentials>,
        secrets: &[(String, String)],
    ) -> Result<Self, String> {
        let cli = engine_dir.join("src").join("cli.ts");
        if !cli.exists() {
            return Err(format!(
                "engine not found: {}\nset DSH_FLOW_HOME to the dsh-flow directory",
                cli.display()
            ));
        }

        let mut command = Command::new("node");
        // `--events` is the live channel: one NDJSON record per step boundary,
        // plus the summary as the last line. The log pane still gets stderr.
        command.arg(&cli).arg("run").arg(manifest).arg("--events");
        for (name, value) in inputs {
            command.arg("--input").arg(format!("{name}={value}"));
        }
        let handover = credentials.is_some() || !secrets.is_empty();
        if handover {
            command.arg("--credentials-stdin");
        }
        command
            .current_dir(engine_dir)
            .stdin(if handover {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // The UI is a window app: a console window for the child would flash.
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // A group of its own, so a stop can take the steps it started too.
            command.process_group(0);
        }
        apply_proxy_env(&mut command);

        let mut child = command
            .spawn()
            .map_err(|error| format!("cannot start node: {error}"))?;

        if handover {
            let Some(mut stdin) = child.stdin.take() else {
                let _ = child.kill();
                let _ = child.wait();
                return Err("引擎在凭据交接前关闭了 stdin".to_string());
            };
            // One payload carries both: the endpoint the `ai` steps use, and the
            // secret inputs. Neither ever appears on the command line.
            let mut payload = serde_json::Map::new();
            if let Some(credentials) = credentials {
                payload.insert("apiKey".to_string(), serde_json::json!(credentials.api_key));
                payload.insert(
                    "baseUrl".to_string(),
                    serde_json::json!(credentials.base_url),
                );
                payload.insert("model".to_string(), serde_json::json!(credentials.model));
            }
            if !secrets.is_empty() {
                let map: serde_json::Map<String, serde_json::Value> = secrets
                    .iter()
                    .map(|(name, value)| (name.clone(), serde_json::json!(value)))
                    .collect();
                payload.insert("secrets".to_string(), serde_json::Value::Object(map));
            }
            // Dropping the handle after the write closes stdin, which is
            // what tells the CLI the payload is complete. A failed write means
            // the run would start without the key/secrets it was given, so fail
            // here instead of letting a later `ai` step report "no api key".
            if let Err(error) =
                stdin.write_all(serde_json::Value::Object(payload).to_string().as_bytes())
            {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("无法把凭据交给引擎：{error}"));
            }
        }

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        let queue = Arc::new(Mutex::new(Vec::new()));
        let slot = Arc::new(Mutex::new(Some(child)));
        let cancelled = Arc::new(AtomicBool::new(false));

        if let Some(stderr) = stderr {
            let queue = queue.clone();
            thread::spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    lock(&queue).push(RunMessage::Line(line));
                }
            });
        }

        {
            let queue = queue.clone();
            let slot = slot.clone();
            thread::spawn(move || {
                // Events arrive line by line; anything that is not an event is
                // the summary, which may be pretty-printed across lines.
                let mut summary_text = String::new();
                if let Some(stdout) = stdout {
                    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                        match step_event(&line) {
                            Some(event) => lock(&queue).push(RunMessage::Event(event)),
                            None => {
                                summary_text.push_str(&line);
                                summary_text.push('\n');
                            }
                        }
                    }
                }
                let exit_code = lock(&slot)
                    .take()
                    .and_then(|mut child| child.wait().ok())
                    .and_then(|status| status.code());
                let message = match parse_summary(&summary_text, exit_code) {
                    Ok(outcome) => RunMessage::Done(Box::new(outcome)),
                    Err(error) => RunMessage::Failed(error),
                };
                lock(&queue).push(message);
            });
        }

        Ok(Self {
            queue,
            child: slot,
            cancelled,
            started: Instant::now(),
        })
    }

    /// Take every message produced since the last call.
    pub fn poll(&self) -> Vec<RunMessage> {
        std::mem::take(&mut *lock(&self.queue))
    }

    /// Kill the engine process, and everything it started, once. Safe to call
    /// more than once.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        // Only while the handle is still ours: once the reader thread has taken
        // it out, the process has exited and its id could belong to something
        // else by now.
        if let Some(child) = lock(&self.child).as_mut() {
            kill_tree(child.id());
            let _ = child.kill();
        }
    }

    /// True after [`Self::cancel`] was called.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// Milliseconds since the process started.
    pub fn elapsed_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }
}

impl Drop for RunSession {
    /// A run nobody polls any more must not keep the engine alive.
    ///
    /// Closing the window drops the session, and the child it started may still
    /// be working: killing it here is what stops a step from writing files after
    /// the window is gone.
    fn drop(&mut self) {
        self.cancel();
    }
}

/// The proxy every child process should use, when one is configured.
static PROXY: Mutex<Option<String>> = Mutex::new(None);

/// Remember the proxy the settings ask for.
///
/// `Some` makes every engine process start with `HTTP(S)_PROXY` in its
/// environment; `None` (an empty setting) leaves them direct.
pub fn set_proxy(proxy: &str) {
    let proxy = proxy.trim();
    let value = (!proxy.is_empty()).then(|| proxy.to_string());
    *lock(&PROXY) = value;
}

/// Give a child process the configured proxy, if there is one.
///
/// Node reads `HTTP(S)_PROXY` from its environment when `NODE_USE_ENV_PROXY` is
/// set (Node 24 and later); older runtimes ignore it and connect directly.
/// Local addresses are always excluded, so a local model server keeps working.
pub fn apply_proxy_env(command: &mut Command) {
    let Some(proxy) = lock(&PROXY).clone() else {
        return;
    };
    command
        .env("HTTP_PROXY", &proxy)
        .env("HTTPS_PROXY", &proxy)
        .env("ALL_PROXY", &proxy)
        .env("NO_PROXY", "127.0.0.1,localhost,::1")
        .env("NODE_USE_ENV_PROXY", "1");
}

/// True when the engine's CLI is present.
pub fn engine_ready(engine_dir: &Path) -> bool {
    engine_dir.join("src").join("cli.ts").exists()
}

/// The `node` on PATH, as it reports itself; `None` when it cannot be started.
///
/// Shown on the diagnostics page: every run and every model call starts this
/// binary, so "is it there, and which version" is the first question a broken
/// install raises.
pub fn node_version() -> Option<String> {
    let mut command = Command::new("node");
    command
        .arg("--version")
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    // The UI is a window app: node is a console program, so without this the
    // check would open a console window of its own.
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then_some(text)
}

/// Why starting `node` failed, with the fix that usually applies.
pub fn node_error(error: &std::io::Error) -> String {
    format!(
        "无法启动 node：{error}（请确认已安装 Node.js {}.{} 或更新，并且 node 在 PATH 中）",
        MIN_NODE.0, MIN_NODE.1
    )
}

/// Lowest Node.js the engine runs on.
///
/// The engine executes its TypeScript directly, and type stripping is on by
/// default from Node 23.6; an older runtime fails while importing the first file.
pub const MIN_NODE: (u64, u64) = (23, 6);

/// What is wrong with the Node.js that was found, if anything.
///
/// The engine executes its TypeScript directly, so a runtime older than
/// [`MIN_NODE`] fails while importing the first file. Saying so on the banner is
/// the difference between "the assistant never answers" and one fixable sentence.
/// @param version - the line [`node_version`] returned, or an empty string when
/// node could not be started at all.
/// @returns the problem in one sentence, or `None` when the runtime is usable.
pub fn node_version_problem(version: &str) -> Option<String> {
    let text = version.trim();
    if text.is_empty() {
        return Some(format!(
            "没有找到 Node.js：引擎需要 Node.js {}.{} 或更新。安装后确认 `node --version` 能在命令行运行，再重启界面。",
            MIN_NODE.0, MIN_NODE.1
        ));
    }
    let cleaned = text.trim_start_matches('v');
    let mut parts = cleaned.split('.');
    let major = parts.next().and_then(|part| part.parse::<u64>().ok());
    let minor = parts.next().and_then(|part| part.parse::<u64>().ok());
    let (Some(major), Some(minor)) = (major, minor) else {
        return Some(format!(
            "无法识别 Node.js 的版本（{text}）：引擎需要 {}.{} 或更新。",
            MIN_NODE.0, MIN_NODE.1
        ));
    };
    if (major, minor) >= MIN_NODE {
        return None;
    }
    Some(format!(
        "当前 Node.js {text} 太旧：引擎直接运行 TypeScript，需要 {}.{} 或更新。升级 Node.js 后重启界面。",
        MIN_NODE.0, MIN_NODE.1
    ))
}

/// Locate the `dsh-flow` engine directory next to this crate.
///
/// Resolution order: `DSH_FLOW_HOME`, then every path from
/// [`engine_candidates`], then `<cwd>/dsh-flow`, then the current directory.
/// The working directory comes last because the window may be started from
/// anywhere (Explorer, a shortcut, another shell).
pub fn locate_engine_dir() -> PathBuf {
    if let Some(from_env) = std::env::var_os("DSH_FLOW_HOME") {
        return resolve_links(&PathBuf::from(from_env));
    }
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let candidates = engine_candidates(&crate_dir);
    if let Some(found) = candidates.iter().find(|dir| engine_ready(dir)) {
        return resolve_links(found);
    }
    if let Ok(cwd) = std::env::current_dir() {
        let nested = cwd.join("dsh-flow");
        if engine_ready(&nested) {
            return resolve_links(&nested);
        }
        return normalize(&cwd);
    }
    resolve_links(&crate_dir.join("dsh-flow"))
}

/// Follow a linked directory to the real one.
///
/// A junction is how the engine is made visible under the crate when it lives
/// in a sibling checkout. The workflow scan walks the folder *holding* the
/// engine, and the examples sit beside the real engine rather than beside the
/// link, so the link must be followed before that parent is taken. `normalize`
/// alone cannot do it: it never touches the filesystem.
///
/// `canonicalize` is avoided on purpose: on Windows it returns a `\\?\` path,
/// which then shows up in the window and is passed to `node`.
fn resolve_links(path: &Path) -> PathBuf {
    let mut current = normalize(path);
    // A link chain is short; the bound only stops a cycle from spinning.
    for _ in 0..8 {
        let Ok(metadata) = fs::symlink_metadata(&current) else {
            break;
        };
        if !metadata.file_type().is_symlink() {
            break;
        }
        let Ok(target) = fs::read_link(&current) else {
            break;
        };
        let next = if target.is_absolute() {
            normalize(&target)
        } else {
            match current.parent() {
                Some(parent) => normalize(&parent.join(target)),
                None => break,
            }
        };
        current = next;
    }
    current
}

/// Every directory the engine may sit in, nearest first.
///
/// The bundled engine beside the executable comes first: after copying the
/// program to another drive, the compile-time crate path must not win over the
/// `dsh-flow` sitting next to the exe. A development checkout still works
/// because its own `target\debug` / `target\release` has no nested engine, so
/// the crate-relative candidates are the next fallback.
fn engine_candidates(crate_dir: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        let mut here = exe.parent().map(Path::to_path_buf);
        for _ in 0..4 {
            let Some(directory) = here else { break };
            found.push(directory.join("dsh-flow"));
            here = directory.parent().map(Path::to_path_buf);
        }
    }
    found.push(crate_dir.join("dsh-flow"));
    found.push(crate_dir.join("..").join("dsh-flow"));
    found
}

/// Find workflow manifests under a root directory.
///
/// Walks at most four levels and skips build and VCS directories, so pointing
/// the app at a repository stays fast.
pub fn discover(root: &Path) -> Vec<WorkflowEntry> {
    let mut found = Vec::new();
    collect(root, 0, &mut found);
    found.sort_by_key(|left| left.name.to_lowercase());
    found
}

/// Depth-first search for `flow.yml` / `flow.yaml` / `flow.json`.
fn collect(directory: &Path, depth: usize, found: &mut Vec<WorkflowEntry>) {
    if depth > 4 {
        return;
    }
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_file() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name == "flow.yml" || name == "flow.yaml" || name == "flow.json" {
                found.push(read_entry(&path));
            }
            continue;
        }
        if !file_type.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if matches!(
            name.as_str(),
            "node_modules" | ".git" | "target" | "lib" | "dist" | ".dsh-flow" | "out" | "backups" | "flows"
        ) {
            continue;
        }
        collect(&path, depth + 1, found);
    }
}

/// Read the identity fields of one manifest.
///
/// The engine validates the real structure; this only fills the plan pane.
fn read_entry(path: &Path) -> WorkflowEntry {
    let fallback = path
        .parent()
        .and_then(|parent| parent.file_name())
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "workflow".to_string());
    let text = fs::read_to_string(path).unwrap_or_default();
    let spec = manifest::parse(&text);
    WorkflowEntry {
        name: spec.name.clone().unwrap_or(fallback),
        description: spec.description.clone().unwrap_or_default(),
        path: path.to_path_buf(),
        spec,
    }
}

/// Turn the engine's stdout into a [`RunOutcome`].
///
/// The summary is the JSON document on stdout; anything printed before it is
/// ignored so a chatty engine still works. The *last* document wins, because a
/// log line printed first must not be mistaken for the result.
fn parse_summary(stdout: &str, exit_code: Option<i32>) -> Result<RunOutcome, String> {
    let summary = crate::json::object(stdout).ok_or_else(|| {
        format!(
            "engine produced no JSON summary (exit code {})\nstdout: {}",
            exit_code.map_or_else(|| "?".to_string(), |code| code.to_string()),
            stdout.trim()
        )
    })?;

    let steps = match summary.get("steps") {
        Some(serde_json::Value::Object(object)) => object.values().map(step_from_json).collect(),
        Some(serde_json::Value::Array(items)) => items.iter().map(step_from_json).collect(),
        _ => Vec::new(),
    };
    let status = summary.get("status").and_then(|value| value.as_str());
    let error = summary
        .get("error")
        .and_then(|value| value.as_str())
        .map(|text| text.to_string());

    Ok(RunOutcome {
        // A non-zero exit code is not a successful run, whatever `status` says.
        ok: status == Some("ok") && error.is_none() && exit_code.is_none_or(|code| code == 0),
        run_id: summary
            .get("runId")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown")
            .to_string(),
        duration_ms: number_at(&summary, "durationMs"),
        steps,
        error,
        report: summary
            .get("report")
            .or_else(|| summary.get("reportPath"))
            .and_then(|value| value.as_str())
            .map(|text| text.to_string()),
        exit_code,
    })
}

/// Convert one step object from the engine summary.
fn step_from_json(value: &serde_json::Value) -> StepView {
    let get_str = |key: &str| {
        value
            .get(key)
            .and_then(|entry| entry.as_str())
            .map(|text| text.to_string())
    };
    StepView {
        id: get_str("id").unwrap_or_else(|| "?".to_string()),
        kind: get_str("kind").unwrap_or_else(|| "run".to_string()),
        status: get_str("status").unwrap_or_else(|| "unknown".to_string()),
        duration_ms: number_at(value, "durationMs"),
        detail: get_str("error").unwrap_or_default(),
        command: get_str("command"),
        stderr: get_str("stderr").filter(|text| !text.trim().is_empty()),
        stdout: get_str("stdout").filter(|text| !text.trim().is_empty()),
        text: get_str("text").filter(|text| !text.trim().is_empty()),
        model: get_str("model"),
        cached: value
            .get("cached")
            .and_then(|entry| entry.as_bool())
            .unwrap_or(false),
        summary: get_str("summary"),
        artifacts: artifact_views(value),
        error: error_view(value),
    }
}

/// Read one `--events` record, or `None` when the line is not one.
fn step_event(line: &str) -> Option<StepEvent> {
    let trimmed = line.trim();
    if !trimmed.starts_with('{') {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(trimmed).ok()?;
    let kind = value.get("type").and_then(|entry| entry.as_str())?;
    if !(kind.starts_with("step/") || kind.starts_with("run/")) {
        return None;
    }
    let get_str = |key: &str| {
        value
            .get(key)
            .and_then(|entry| entry.as_str())
            .map(str::to_string)
    };
    // A `step/start` has no verdict yet; the row shows it as running meanwhile.
    let status = get_str("status").unwrap_or_else(|| {
        if kind == "step/start" {
            "running".to_string()
        } else {
            String::new()
        }
    });
    Some(StepEvent {
        kind: kind.to_string(),
        id: get_str("id").unwrap_or_default(),
        step_kind: get_str("kind").unwrap_or_default(),
        status,
        duration_ms: number_at(&value, "durationMs"),
        summary: get_str("summary"),
        detail: get_str("detail"),
        command: get_str("command"),
        artifacts: artifact_views(&value),
        error: error_view(&value),
    })
}

/// Read the `artifacts` array an event or a step result carries.
fn artifact_views(value: &serde_json::Value) -> Vec<ArtifactView> {
    value
        .get("artifacts")
        .and_then(|entry| entry.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let path = item
                        .get("path")
                        .and_then(|entry| entry.as_str())?
                        .to_string();
                    Some(ArtifactView {
                        kind: item
                            .get("kind")
                            .and_then(|entry| entry.as_str())
                            .unwrap_or("file")
                            .to_string(),
                        name: item
                            .get("name")
                            .and_then(|entry| entry.as_str())
                            .unwrap_or_default()
                            .to_string(),
                        path,
                        bytes: item
                            .get("bytes")
                            .and_then(|entry| entry.as_u64())
                            .unwrap_or(0),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Read a structured failure from an event (`error`) or a step result (`errorInfo`).
fn error_view(value: &serde_json::Value) -> Option<ErrorView> {
    let raw = value
        .get("errorInfo")
        .or_else(|| value.get("error"))
        .and_then(|entry| entry.as_object())?;
    let message = raw
        .get("message")
        .and_then(|entry| entry.as_str())
        .unwrap_or_default()
        .to_string();
    if message.is_empty() {
        return None;
    }
    let text = |key: &str| {
        raw.get(key)
            .and_then(|entry| entry.as_str())
            .map(str::to_string)
    };
    Some(ErrorView {
        kind: text("kind").unwrap_or_else(|| "engine".to_string()),
        message,
        hint: text("hint"),
        location: text("where"),
        expected: text("expected"),
        gotten: text("gotten"),
    })
}

/// Read a millisecond count, accepting numbers written as strings.
fn number_at(value: &serde_json::Value, key: &str) -> u64 {
    value
        .get(key)
        .and_then(|entry| {
            entry
                .as_u64()
                .or_else(|| entry.as_str().and_then(|text| text.parse().ok()))
        })
        .unwrap_or(0)
}

/// Collapse `.` and `..` segments without touching the filesystem.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_node_version_is_read_or_refused() {
        // The engine runs TypeScript directly: 23.6 is the first runtime that can.
        assert!(node_version_problem("v24.11.1").is_none());
        assert!(node_version_problem("v23.6.0").is_none());
        assert!(node_version_problem("23.6.0").is_none());
        assert!(node_version_problem("v23.5.9").is_some());
        assert!(node_version_problem("v20.11.1").is_some());
        let old = node_version_problem("v20.11.1").expect("too old");
        assert!(old.contains("23.6"), "{old}");
        // Nothing installed, and output that is not a version at all.
        let missing = node_version_problem("").expect("missing");
        assert!(missing.contains("没有找到 Node.js"), "{missing}");
        assert!(node_version_problem("garbage").is_some());
    }

    #[test]
    fn prefers_the_engine_beside_the_executable() {
        let crate_dir = std::env::temp_dir().join("dsh-flow-ui-candidates");
        let found = engine_candidates(&crate_dir);
        let exe_dir = std::env::current_exe()
            .expect("test exe")
            .parent()
            .expect("test exe directory")
            .to_path_buf();
        assert_eq!(found[0], exe_dir.join("dsh-flow"));
        assert!(found.contains(&crate_dir.join("dsh-flow")));
        assert!(found.contains(&crate_dir.join("..").join("dsh-flow")));
    }

    /// A temp directory that removes itself when the test ends.
    struct Fixture(PathBuf);

    impl Fixture {
        /// Create `<temp>/<tag>` with a fake engine that runs `script`.
        fn engine(tag: &str, script: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("dsh-flow-ui-{}-{tag}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(dir.join("src")).unwrap();
            fs::write(dir.join("src").join("cli.ts"), script).unwrap();
            Self(dir)
        }

        /// Create an empty `<temp>/<tag>`.
        fn empty(tag: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("dsh-flow-ui-{}-{tag}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn node_available() -> bool {
        Command::new("node")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    /// Drive a session until it settles, then hand back everything it produced.
    fn drain(session: &RunSession) -> (Vec<String>, Option<RunOutcome>, Option<String>) {
        let deadline = Instant::now() + std::time::Duration::from_secs(30);
        let mut lines = Vec::new();
        loop {
            for message in session.poll() {
                match message {
                    RunMessage::Line(line) => lines.push(line),
                    RunMessage::Event(_) => {}
                    RunMessage::Done(outcome) => return (lines, Some(*outcome), None),
                    RunMessage::Failed(error) => return (lines, None, Some(error)),
                }
            }
            if Instant::now() > deadline {
                panic!("engine did not finish; lines so far: {lines:?}");
            }
            thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    #[test]
    fn normalize_collapses_parent_segments() {
        let collapsed = normalize(Path::new("a/b/../c/./d"));
        assert!(collapsed.ends_with(Path::new("a").join("c").join("d")));
    }

    /// A junction looks like a link, not a directory, so the workflow scan would
    /// stop at it. The engine must be resolved to its real folder first.
    #[cfg(windows)]
    #[test]
    fn follows_a_junction_to_the_folder_holding_the_engine() {
        let root =
            std::env::temp_dir().join(format!("dsh-flow-ui-junction-{}", std::process::id()));
        let real = root.join("engine");
        let link = root.join("dsh-flow");
        let _ = fs::remove_dir(&link);
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&real).expect("target folder");
        let made = Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&real)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        // Developer mode is not needed for a junction, but an unusual policy
        // could still refuse; the test then has nothing to check.
        if made {
            assert_eq!(resolve_links(&link), normalize(&real));
            // Remove the link itself, never the folder it points at.
            let _ = fs::remove_dir(&link);
        }
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn reads_identity_fields_from_yaml() {
        let dir = Fixture::empty("entry");
        let manifest = dir.path().join("flow.yml");
        fs::write(
            &manifest,
            "name: demo-flow\ndescription: does a thing\nversion: 1\n",
        )
        .unwrap();
        let entry = read_entry(&manifest);
        assert_eq!(entry.name, "demo-flow");
        assert_eq!(entry.description, "does a thing");
    }

    #[test]
    fn discovers_the_shipped_example() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("dsh-flow")
            .join("examples");
        if !root.exists() {
            return;
        }
        let found = discover(&root);
        assert!(
            found.iter().any(|entry| entry.name == "repo-health"),
            "expected repo-health under {}",
            root.display()
        );
    }

    #[test]
    fn discovery_ignores_archived_workflows_under_backups() {
        let root = std::env::temp_dir().join(format!(
            "dsh-flow-ui-discover-backups-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("backups").join("old")).unwrap();
        std::fs::write(root.join("backups").join("old").join("flow.yml"), "name: archived\n").unwrap();
        std::fs::create_dir_all(root.join("workflows").join("live")).unwrap();
        std::fs::write(root.join("workflows").join("live").join("flow.yml"), "name: live\n").unwrap();
        let found = discover(&root);
        assert!(found.iter().any(|entry| entry.name == "live"));
        assert!(!found.iter().any(|entry| entry.name == "archived"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn discovery_skips_subflows_under_flows() {
        let root = std::env::temp_dir().join(format!(
            "dsh-flow-ui-discover-flows-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let workflow = root.join("workflows").join("w");
        std::fs::create_dir_all(workflow.join("flows").join("login")).unwrap();
        std::fs::write(workflow.join("flow.yml"), "name: w\n").unwrap();
        std::fs::write(
            workflow.join("flows").join("login").join("flow.yml"),
            "name: login\n",
        )
        .unwrap();
        let found = discover(&root);
        assert!(found.iter().any(|entry| entry.name == "w"));
        assert!(!found.iter().any(|entry| entry.name == "login"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn reports_a_missing_engine_clearly() {
        let engine = std::env::temp_dir().join("dsh-flow-ui-no-engine");
        let error = RunSession::start(&engine, Path::new("flow.yml"), &[], None, &[])
            .err()
            .unwrap();
        assert!(error.contains("engine not found"), "{error}");
    }

    #[test]
    fn refuses_a_summary_that_is_not_json() {
        let error = parse_summary("boom", Some(3)).unwrap_err();
        assert!(error.contains("no JSON summary"), "{error}");
        assert!(error.contains("exit code 3"), "{error}");
    }

    #[test]
    fn streams_progress_and_reads_the_summary() {
        if !node_available() {
            return;
        }
        let engine = Fixture::engine(
            "ok",
            r#"console.error("progress: counting");
const steps = { counts: { id: "counts", kind: "run", status: "ok", durationMs: 40, command: "python steps/count.py", stderr: "1 line" } };
console.log(JSON.stringify({ status: "ok", runId: "run-1", durationMs: 42, report: "out/report.html", steps }));"#,
        );
        let session =
            RunSession::start(engine.path(), Path::new("flow.yml"), &[], None, &[]).unwrap();
        let (lines, outcome, failure) = drain(&session);
        assert!(failure.is_none(), "{failure:?}");
        let outcome = outcome.expect("summary");
        assert!(outcome.ok);
        assert_eq!(outcome.run_id, "run-1");
        assert_eq!(outcome.duration_ms, 42);
        assert_eq!(outcome.report.as_deref(), Some("out/report.html"));
        assert_eq!(outcome.steps.len(), 1);
        assert_eq!(
            outcome.steps[0].command.as_deref(),
            Some("python steps/count.py")
        );
        assert!(
            lines.iter().any(|line| line.contains("counting")),
            "{lines:?}"
        );
    }

    #[test]
    fn reads_the_event_stream_and_the_summary_after_it() {
        if !node_available() {
            return;
        }
        let engine = Fixture::engine(
            "events",
            r#"console.log(JSON.stringify({ts:"t",type:"run/start",detail:"demo"}));
console.log(JSON.stringify({ts:"t",type:"step/start",id:"export",kind:"run"}));
console.log(JSON.stringify({ts:"t",type:"step/end",id:"export",kind:"run",status:"ok",durationMs:8,summary:"已导出 x.xls（3 字节）",artifacts:[{kind:"file",name:"x.xls",path:"C:/tmp/x.xls",bytes:3}]}));
console.log(JSON.stringify({ts:"t",type:"step/end",id:"verify",kind:"run",status:"failed",durationMs:2,error:{kind:"assert",message:"条数不对",hint:"先查询",where:"units.py:verify",expected:"51 条",gotten:"0 条"}}));
console.log(JSON.stringify({ts:"t",type:"run/end",status:"failed",durationMs:10,detail:"条数不对"}));
console.log(JSON.stringify({status:"failed",runId:"run-4",durationMs:10,steps:{export:{id:"export",kind:"run",status:"ok",durationMs:8,summary:"已导出 x.xls（3 字节）",artifacts:[{kind:"file",name:"x.xls",path:"C:/tmp/x.xls",bytes:3}]}}}));"#,
        );
        let session =
            RunSession::start(engine.path(), Path::new("flow.yml"), &[], None, &[]).unwrap();
        let deadline = Instant::now() + std::time::Duration::from_secs(30);
        let mut events = Vec::new();
        let mut outcome = None;
        while outcome.is_none() {
            assert!(Instant::now() <= deadline, "engine did not finish");
            for message in session.poll() {
                match message {
                    RunMessage::Event(event) => events.push(event),
                    RunMessage::Done(summary) => outcome = Some(*summary),
                    RunMessage::Failed(error) => panic!("{error}"),
                    RunMessage::Line(_) => {}
                }
            }
            thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(events.len(), 5);
        let ended = events
            .iter()
            .find(|event| event.id == "export" && event.status == "ok")
            .expect("step end");
        assert_eq!(ended.summary.as_deref(), Some("已导出 x.xls（3 字节）"));
        assert_eq!(ended.artifacts.len(), 1);
        assert_eq!(ended.artifacts[0].bytes, 3);
        let failed = events
            .iter()
            .find(|event| event.id == "verify")
            .expect("failed step");
        let error = failed.error.as_ref().expect("error view");
        assert_eq!(error.kind, "assert");
        assert_eq!(error.location.as_deref(), Some("units.py:verify"));
        assert_eq!(error.expected.as_deref(), Some("51 条"));
        // The summary still comes from the last plain JSON line.
        let outcome = outcome.expect("summary");
        assert!(!outcome.ok);
        assert_eq!(
            outcome.steps[0].summary.as_deref(),
            Some("已导出 x.xls（3 字节）")
        );
    }

    #[test]
    fn reads_an_ai_step_answer_and_model() {
        let summary = r#"{
  "status": "ok",
  "runId": "run-3",
  "durationMs": 5,
  "steps": {
    "classify": {
      "id": "classify",
      "kind": "ai",
      "status": "ok",
      "durationMs": 12,
      "text": "{\"category\":\"bug\"}",
      "json": { "category": "bug" },
      "model": "deepseek-chat",
      "cached": true
    }
  }
}"#;
        let outcome = parse_summary(summary, Some(0)).unwrap();
        let step = &outcome.steps[0];
        assert_eq!(step.kind, "ai");
        assert_eq!(step.model.as_deref(), Some("deepseek-chat"));
        assert_eq!(step.text.as_deref(), Some("{\"category\":\"bug\"}"));
        assert!(step.cached);
    }

    #[test]
    fn passes_credentials_on_stdin() {
        if !node_available() {
            return;
        }
        let engine = Fixture::engine(
            "credentials",
            r#"let raw = "";
process.stdin.setEncoding("utf8");
process.stdin.on("data", chunk => { raw += chunk });
process.stdin.on("end", () => {
  console.log(JSON.stringify({
    status: "ok",
    runId: "run-c",
    durationMs: 1,
    steps: { probe: { id: "probe", kind: "run", status: "ok", durationMs: 1, stdout: process.argv.slice(2).join(" ") + " || " + raw } }
  }));
});"#,
        );
        let credentials = RunCredentials {
            api_key: "sk-test-unit".to_string(),
            base_url: "http://127.0.0.1:9".to_string(),
            model: "test-model".to_string(),
        };
        let session = RunSession::start(
            engine.path(),
            Path::new("flow.yml"),
            &[],
            Some(&credentials),
            &[],
        )
        .unwrap();
        let (_lines, outcome, failure) = drain(&session);
        assert!(failure.is_none(), "{failure:?}");
        let outcome = outcome.expect("summary");
        assert!(outcome.ok);
        let seen = outcome.steps[0].stdout.clone().unwrap_or_default();
        assert!(seen.contains("--credentials-stdin"), "{seen}");
        assert!(seen.contains("\"apiKey\":\"sk-test-unit\""), "{seen}");
        assert!(seen.contains("\"model\":\"test-model\""), "{seen}");
    }

    #[test]
    fn passes_inputs_as_flags() {
        if !node_available() {
            return;
        }
        let engine = Fixture::engine(
            "inputs",
            r#"console.error("argv " + process.argv.slice(2).join(" "));
console.log(JSON.stringify({ status: "ok", runId: "run-2", durationMs: 1, steps: {} }));"#,
        );
        let inputs = vec![("repo".to_string(), "E:/work".to_string())];
        let session =
            RunSession::start(engine.path(), Path::new("flow.yml"), &inputs, None, &[]).unwrap();
        let (lines, outcome, failure) = drain(&session);
        assert!(failure.is_none(), "{failure:?}");
        assert!(outcome.expect("summary").ok);
        let argv = lines
            .iter()
            .find(|line| line.starts_with("argv "))
            .expect("argv echoed");
        assert!(
            argv.contains("run flow.yml --events --input repo=E:/work"),
            "{argv}"
        );
    }

    #[test]
    fn reports_a_failure_summary() {
        let outcome = parse_summary(
            r#"{"status":"failed","runId":"r","durationMs":5,"error":"step counts failed","steps":[{"id":"counts","status":"failed","durationMs":4,"error":"exit 1"}]}"#,
            Some(1),
        )
        .unwrap();
        assert!(!outcome.ok);
        assert_eq!(outcome.error.as_deref(), Some("step counts failed"));
        assert_eq!(outcome.steps[0].detail, "exit 1");
        assert_eq!(outcome.exit_code, Some(1));
    }

    #[test]
    fn ignores_a_json_log_line_before_the_summary() {
        // A chatty engine prints a log object first; taking the first `{` would
        // read that line as the result and report a good run as a failure.
        let stdout = "{\"level\":\"info\",\"msg\":\"starting\"}\n{\"status\":\"ok\",\"runId\":\"run-9\",\"steps\":[]}";
        let outcome = parse_summary(stdout, Some(0)).unwrap();
        assert!(outcome.ok);
        assert_eq!(outcome.run_id, "run-9");
        assert!(outcome.steps.is_empty());
    }

    #[test]
    fn a_non_zero_exit_code_is_not_success() {
        let outcome = parse_summary(r#"{"status":"ok","runId":"r","steps":[]}"#, Some(1)).unwrap();
        assert!(
            !outcome.ok,
            "the process failed even though the status said ok"
        );
    }
}
