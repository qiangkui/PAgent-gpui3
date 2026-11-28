//! Detailed snapshots of finished runs.
//!
//! The window keeps a summary per run for its cards (see
//! [`crate::workflow_store`]), but a post-mortem needs more than "failed at
//! step 3": the step's own output, the engine's log lines, the artifacts, and
//! the structured error. This module writes that whole picture into
//! `<data dir>\workflows\<name>\runs\<run id>.json`.
//!
//! Everything stored here has already passed the engine's redactor: secrets are
//! `***` by the time the window sees a line, so a snapshot cannot undo that
//! promise. The files are plain JSON on purpose — during development they are
//! what a failure gets read from, and a reviewer can open one without the app.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::engine::{ArtifactView, ErrorView, StepView};

/// How many run files one workflow keeps; the newest win.
pub const MAX_RUNS: usize = 30;

/// Folder under a workflow's data entry that holds the snapshots.
const FOLDER: &str = "runs";

/// One finished run, in full.
#[derive(Clone, Debug, Default)]
pub struct StoredRun {
    /// Workflow name, as the manifest declares it.
    pub workflow: String,
    /// Manifest path relative to the scan root.
    pub manifest: String,
    /// Engine-assigned id; empty when the run never reported one.
    pub run_id: String,
    /// True when every step that mattered succeeded.
    pub ok: bool,
    /// True when the reader stopped it.
    pub cancelled: bool,
    /// Wall-clock duration.
    pub duration_ms: u64,
    /// When it finished, in milliseconds since the Unix epoch.
    pub finished_ms: u64,
    /// The values it ran with, with secrets already masked.
    pub inputs: Vec<(String, String)>,
    /// Steps in execution order, with their output and errors.
    pub steps: Vec<StepView>,
    /// The engine's own log lines.
    pub log: Vec<String>,
    /// Failure text for the run as a whole.
    pub error: Option<String>,
    /// HTML report the engine wrote, when it did.
    pub report: Option<String>,
    /// Process exit code, when the engine reported one.
    pub exit_code: Option<i32>,
}

/// Write one run snapshot, creating its folder and pruning old files.
pub fn save(data_dir: &Path, run: &StoredRun) -> Result<(), String> {
    let folder = run_folder(data_dir, &run.workflow);
    std::fs::create_dir_all(&folder)
        .map_err(|error| format!("无法创建 {}：{error}", folder.display()))?;
    let file = folder.join(format!("{}.json", run_file_stem(run)));
    let body = serde_json::to_string_pretty(&run_json(run))
        .map_err(|error| format!("无法序列化运行记录：{error}"))?;
    crate::store::write_if_changed(&file, &body)?;
    prune(&folder);
    Ok(())
}

/// The newest snapshots across every workflow, newest first.
///
/// Reads at most `limit` files: a restarted window only needs the tail of the
/// history to show something useful, and a developer reading a failure opens
/// the folder directly.
pub fn load_latest(data_dir: &Path, limit: usize) -> Vec<StoredRun> {
    let root = data_dir.join("workflows");
    let Ok(entries) = std::fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut files: Vec<(u64, PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let folder = entry.path().join(FOLDER);
        let Ok(runs) = std::fs::read_dir(&folder) else {
            continue;
        };
        for run in runs.flatten() {
            let path = run.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let modified = std::fs::metadata(&path)
                .and_then(|meta| meta.modified())
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |age| age.as_millis() as u64);
            files.push((modified, path));
        }
    }
    files.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    files
        .into_iter()
        .take(limit)
        .filter_map(|(_, path)| read_run(&path))
        .collect()
}

/// One workflow's newest snapshots, newest first.
pub fn load_workflow(data_dir: &Path, workflow: &str, limit: usize) -> Vec<StoredRun> {
    let folder = run_folder(data_dir, workflow);
    let Ok(entries) = std::fs::read_dir(&folder) else {
        return Vec::new();
    };
    let mut runs: Vec<StoredRun> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
        .filter_map(|path| read_run(&path))
        .collect();
    runs.sort_by_key(|run| std::cmp::Reverse(run.finished_ms));
    runs.truncate(limit);
    runs
}

/// The folder holding one workflow's snapshots.
fn run_folder(data_dir: &Path, workflow: &str) -> PathBuf {
    data_dir
        .join("workflows")
        .join(folder_label(workflow))
        .join(FOLDER)
}

/// A name that is safe as a folder and still recognisable.
fn folder_label(name: &str) -> String {
    let cleaned: String = name
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
        "workflow".to_string()
    } else {
        cleaned
    }
}

/// The file name one run is written to.
fn run_file_stem(run: &StoredRun) -> String {
    let id: String = run
        .run_id
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
    if id.is_empty() || id == "_" || id == "-" {
        format!("run-{}", run.finished_ms)
    } else {
        id
    }
}

/// Delete the oldest files once a workflow has more than [`MAX_RUNS`].
fn prune(folder: &Path) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
        .filter_map(|path| {
            let modified = std::fs::metadata(&path).ok()?.modified().ok()?;
            Some((modified, path))
        })
        .collect();
    if files.len() <= MAX_RUNS {
        return;
    }
    files.sort_by_key(|(modified, _)| *modified);
    let excess = files.len() - MAX_RUNS;
    for (_, path) in files.into_iter().take(excess) {
        let _ = std::fs::remove_file(path);
    }
}

/// One run as JSON.
fn run_json(run: &StoredRun) -> Value {
    json!({
        "version": 1,
        "workflow": run.workflow,
        "manifest": run.manifest,
        "runId": run.run_id,
        "ok": run.ok,
        "cancelled": run.cancelled,
        "durationMs": run.duration_ms,
        "finishedMs": run.finished_ms,
        "inputs": run
            .inputs
            .iter()
            .map(|(name, value)| json!({ "name": name, "value": value }))
            .collect::<Vec<Value>>(),
        "steps": run.steps.iter().map(step_json).collect::<Vec<Value>>(),
        "log": run.log,
        "error": run.error,
        "report": run.report,
        "exitCode": run.exit_code,
    })
}

/// One step as JSON.
fn step_json(step: &StepView) -> Value {
    json!({
        "id": step.id,
        "kind": step.kind,
        "status": step.status,
        "durationMs": step.duration_ms,
        "detail": step.detail,
        "command": step.command,
        "stdout": step.stdout,
        "stderr": step.stderr,
        "text": step.text,
        "model": step.model,
        "cached": step.cached,
        "summary": step.summary,
        "artifacts": step
            .artifacts
            .iter()
            .map(|artifact| json!({
                "kind": artifact.kind,
                "name": artifact.name,
                "path": artifact.path,
                "bytes": artifact.bytes,
            }))
            .collect::<Vec<Value>>(),
        "error": step.error.as_ref().map(error_json),
    })
}

/// One structured error as JSON.
fn error_json(error: &ErrorView) -> Value {
    json!({
        "kind": error.kind,
        "message": error.message,
        "hint": error.hint,
        "location": error.location,
        "expected": error.expected,
        "gotten": error.gotten,
    })
}

/// Read one snapshot; a broken file is skipped rather than taking the rest with it.
fn read_run(path: &Path) -> Option<StoredRun> {
    let text = std::fs::read_to_string(path).ok()?;
    let value = serde_json::from_str::<Value>(&text).ok()?;
    let workflow = text_at(&value, "workflow")?;
    let inputs = value
        .get("inputs")
        .and_then(|item| item.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|item| {
                    Some((
                        item.get("name")?.as_str()?.to_string(),
                        item.get("value")?.as_str()?.to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    let steps: Vec<StepView> = value
        .get("steps")
        .and_then(|item| item.as_array())
        .map(|list| list.iter().filter_map(read_step).collect())
        .unwrap_or_default();
    let log: Vec<String> = value
        .get("log")
        .and_then(|item| item.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|line| line.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    Some(StoredRun {
        workflow,
        manifest: text_at(&value, "manifest").unwrap_or_default(),
        run_id: text_at(&value, "runId").unwrap_or_default(),
        ok: value.get("ok").and_then(Value::as_bool).unwrap_or(false),
        cancelled: value
            .get("cancelled")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        duration_ms: number_at(&value, "durationMs"),
        finished_ms: number_at(&value, "finishedMs"),
        inputs,
        steps,
        log,
        error: text_at(&value, "error"),
        report: text_at(&value, "report"),
        exit_code: value
            .get("exitCode")
            .and_then(Value::as_i64)
            .map(|code| code as i32),
    })
}

/// One step from JSON; a step without an id is not a step.
fn read_step(value: &Value) -> Option<StepView> {
    let id = text_at(value, "id")?;
    let artifacts: Vec<ArtifactView> = value
        .get("artifacts")
        .and_then(|item| item.as_array())
        .map(|list| list.iter().filter_map(read_artifact).collect())
        .unwrap_or_default();
    Some(StepView {
        id,
        kind: text_at(value, "kind").unwrap_or_default(),
        status: text_at(value, "status").unwrap_or_default(),
        duration_ms: number_at(value, "durationMs"),
        detail: text_at(value, "detail").unwrap_or_default(),
        command: text_at(value, "command"),
        stderr: text_at(value, "stderr"),
        stdout: text_at(value, "stdout"),
        text: text_at(value, "text"),
        model: text_at(value, "model"),
        cached: value
            .get("cached")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        summary: text_at(value, "summary"),
        artifacts,
        error: value.get("error").and_then(read_error),
    })
}

/// One artifact from JSON.
fn read_artifact(value: &Value) -> Option<ArtifactView> {
    Some(ArtifactView {
        kind: text_at(value, "kind").unwrap_or_default(),
        name: text_at(value, "name").unwrap_or_default(),
        path: text_at(value, "path")?,
        bytes: number_at(value, "bytes"),
    })
}

/// One structured error from JSON.
fn read_error(value: &Value) -> Option<ErrorView> {
    Some(ErrorView {
        kind: text_at(value, "kind").unwrap_or_default(),
        message: text_at(value, "message").unwrap_or_default(),
        hint: text_at(value, "hint"),
        location: text_at(value, "location"),
        expected: text_at(value, "expected"),
        gotten: text_at(value, "gotten"),
    })
}

/// A string field, `None` when absent or empty.
fn text_at(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|text| !text.is_empty())
}

/// A number field, zero when absent.
fn number_at(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("dsh-run-store-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn sample(workflow: &str, run_id: &str, finished: u64) -> StoredRun {
        StoredRun {
            workflow: workflow.to_string(),
            manifest: "workflows/demo/flow.yml".to_string(),
            run_id: run_id.to_string(),
            ok: false,
            cancelled: false,
            duration_ms: 1200,
            finished_ms: finished,
            inputs: vec![("account".to_string(), "***".to_string())],
            steps: vec![StepView {
                id: "login".to_string(),
                kind: "use".to_string(),
                status: "failed".to_string(),
                duration_ms: 900,
                detail: "缺少账号".to_string(),
                command: None,
                stderr: Some("bad credentials".to_string()),
                stdout: None,
                text: None,
                model: None,
                cached: false,
                summary: Some("未登录".to_string()),
                artifacts: vec![ArtifactView {
                    kind: "image".to_string(),
                    name: "captcha.png".to_string(),
                    path: "C:\\work\\captcha.png".to_string(),
                    bytes: 1024,
                }],
                error: Some(ErrorView {
                    kind: "credential".to_string(),
                    message: "缺少账号".to_string(),
                    hint: Some("在卡片上填写".to_string()),
                    location: Some("login:read".to_string()),
                    expected: None,
                    gotten: None,
                }),
            }],
            log: vec!["[FAIL] login".to_string()],
            error: Some("第 1 步失败".to_string()),
            report: None,
            exit_code: Some(1),
        }
    }

    #[test]
    fn a_snapshot_round_trips_with_every_detail() {
        let root = temp_root("roundtrip");
        let run = sample("maers-units-export", "run-42", 1_700_000_000_000);
        save(&root, &run).expect("save");
        let loaded = load_workflow(&root, "maers-units-export", 10);
        assert_eq!(loaded.len(), 1);
        let back = &loaded[0];
        assert_eq!(back.workflow, "maers-units-export");
        assert_eq!(back.run_id, "run-42");
        assert!(!back.ok);
        assert_eq!(back.inputs, run.inputs);
        assert_eq!(back.log, run.log);
        assert_eq!(back.exit_code, Some(1));
        let step = &back.steps[0];
        assert_eq!(step.id, "login");
        assert_eq!(step.stderr.as_deref(), Some("bad credentials"));
        assert_eq!(step.artifacts[0].name, "captcha.png");
        assert_eq!(
            step.error.as_ref().and_then(|error| error.hint.as_deref()),
            Some("在卡片上填写")
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_newest_runs_come_first_and_old_files_are_pruned() {
        let root = temp_root("prune");
        for index in 0..(MAX_RUNS + 3) {
            let run = sample(
                "demo",
                &format!("run-{index:03}"),
                1_700_000_000_000 + index as u64,
            );
            save(&root, &run).expect("save");
        }
        let folder = run_folder(&root, "demo");
        let files = std::fs::read_dir(&folder).unwrap().count();
        assert_eq!(files, MAX_RUNS, "old snapshots are dropped");
        let loaded = load_workflow(&root, "demo", MAX_RUNS);
        assert_eq!(loaded.len(), MAX_RUNS);
        assert_eq!(loaded[0].run_id, format!("run-{:03}", MAX_RUNS + 2));
        let newest = load_latest(&root, 3);
        assert_eq!(newest.len(), 3);
        assert!(newest[0].finished_ms >= newest[1].finished_ms);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_broken_snapshot_is_skipped_and_names_are_safe() {
        let root = temp_root("broken");
        let folder = run_folder(&root, "a:b");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("broken.json"), "{ not json").unwrap();
        let run = sample("a:b", "", 1_700_000_000_000);
        save(&root, &run).expect("save");
        // A run that failed before the engine assigned an id gets a stamp, not a
        // file called `-.json` that every such run would overwrite.
        let failed = sample("a:b", "-", 1_700_000_000_001);
        save(&root, &failed).expect("save");
        assert!(folder.join("run-1700000000001.json").exists());
        let loaded = load_workflow(&root, "a:b", 10);
        assert_eq!(loaded.len(), 2, "the unreadable file did not take the rest");
        assert!(folder.join("run-1700000000000.json").exists());
        let _ = std::fs::remove_dir_all(root);
    }
}
