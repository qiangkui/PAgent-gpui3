//! Saved workflow cards.
//!
//! A workflow is discovered from the engine folder, so the window only keeps
//! what it decided about the card — the category it was filed under, whether it
//! was taken out of the 工作区 list — together with the short summary of each
//! finished run, which is what restores the card's status dot and its
//! "3 分钟前".
//!
//! Like conversations, every workflow gets its own folder:
//! `<data dir>\workflows\<name>\workflow.json`, so one unreadable card cannot
//! take the others with it. The single `runs.json` and the workflow entries of
//! `sidebar.json` written by older versions are folded into folders the first
//! time they are read.

use crate::store;
use std::path::Path;

/// Folder under the data directory holding one folder per workflow.
pub const FOLDER: &str = "workflows";

/// The record inside a workflow folder.
const ENTRY: &str = "workflow.json";

/// Every run in one file, as written by older versions.
pub const LEGACY_RUNS: &str = "runs.json";

/// The sidebar file that used to carry the filing and the hidden flags.
const LEGACY_SIDEBAR: &str = "sidebar.json";

/// How many runs one workflow keeps; the newest ones win.
pub const MAX_RUNS: usize = 50;

/// One finished run, as it survives a restart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunSummary {
    /// Workflow name.
    pub workflow: String,
    /// Manifest path relative to the scan root.
    pub manifest: String,
    /// True when the run finished without error.
    pub ok: bool,
    /// True when the user stopped it.
    pub cancelled: bool,
    /// How long it ran.
    pub duration_ms: u64,
    /// Steps in the plan.
    pub steps: usize,
    /// Steps that failed.
    pub failed_steps: usize,
    /// When it finished, in milliseconds since the Unix epoch.
    pub finished_ms: u64,
}

/// One workflow as it is stored: its filing, its visibility, and its runs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StoredWorkflow {
    /// Name of the workflow, as the manifest declares it.
    pub name: String,
    /// Id of the category the card sits in; empty for the default one.
    pub category: String,
    /// True when the card was taken out of the 工作区 list.
    pub hidden: bool,
    /// Finished runs, newest first.
    pub runs: Vec<RunSummary>,
}

impl StoredWorkflow {
    /// True when there is nothing worth a file.
    fn is_empty(&self) -> bool {
        self.category.is_empty() && !self.hidden && self.runs.is_empty()
    }
}

/// Read the saved workflows from a data directory.
///
/// A missing or broken folder is skipped; a single `runs.json` (and the
/// workflow entries of `sidebar.json`) left by an older version is folded into
/// folders once and then removed. Folders win over the old file, so a split
/// that was cut short only adds what is still missing.
pub fn load(dir: &Path) -> Vec<StoredWorkflow> {
    let stored = load_folders(&dir.join(FOLDER));
    let legacy = legacy_entries(dir);
    if legacy.is_empty() {
        return stored;
    }
    let mut merged = stored;
    for item in legacy {
        let workflow = entry(&mut merged, &item.name);
        if workflow.category.is_empty() {
            workflow.category = item.category;
        }
        workflow.hidden |= item.hidden;
        for run in item.runs {
            if workflow.runs.len() >= MAX_RUNS {
                break;
            }
            let duplicate = workflow.runs.iter().any(|saved| {
                saved.finished_ms == run.finished_ms && saved.manifest == run.manifest
            });
            if !duplicate {
                workflow.runs.push(run);
            }
        }
    }
    // Keep the old file until every folder is written: a failed split must not
    // cost the runs it could not store.
    match save(dir, &merged) {
        Ok(()) => {
            let runs = dir.join(LEGACY_RUNS);
            if runs.is_file()
                && let Err(error) = std::fs::remove_file(&runs)
            {
                eprintln!("无法删除旧的 {}：{error}", runs.display());
            }
        }
        Err(error) => eprintln!("无法把旧的工作流记录拆成文件夹：{error}"),
    }
    merged
}

/// Write the workflows into a data directory, one folder each.
///
/// A workflow that carries nothing keeps no file, and the folder of a card that
/// this run read but no longer exists is removed.
pub fn save(dir: &Path, workflows: &[StoredWorkflow]) -> Result<(), String> {
    let root = dir.join(FOLDER);
    let mut names: Vec<String> = Vec::new();
    for workflow in workflows {
        if workflow.is_empty() {
            continue;
        }
        let name = store::folder_name(&workflow.name, &names);
        let folder = root.join(&name);
        std::fs::create_dir_all(&folder)
            .map_err(|error| format!("无法创建 {}：{error}", folder.display()))?;
        store::write_if_changed(&folder.join(ENTRY), &entry_value(workflow).to_string())?;
        names.push(name);
    }
    remove_orphans(&root, &names);
    Ok(())
}

/// Build the stored workflows out of what the window holds in memory.
///
/// The three lists overlap — a card can be filed away, hidden and have runs at
/// the same time — so they are merged by workflow name. Runs arrive newest
/// first and each workflow keeps its newest [`MAX_RUNS`].
pub fn collect(
    filings: &[(String, String)],
    hidden: &[String],
    runs: &[RunSummary],
) -> Vec<StoredWorkflow> {
    let mut workflows: Vec<StoredWorkflow> = Vec::new();
    for (name, category) in filings {
        entry(&mut workflows, name).category = category.clone();
    }
    for name in hidden {
        entry(&mut workflows, name).hidden = true;
    }
    for run in runs {
        let workflow = entry(&mut workflows, &run.workflow);
        if workflow.runs.len() < MAX_RUNS {
            workflow.runs.push(run.clone());
        }
    }
    workflows
}

/// The entry for `name`, added when it is not there yet.
fn entry<'a>(workflows: &'a mut Vec<StoredWorkflow>, name: &str) -> &'a mut StoredWorkflow {
    if let Some(index) = workflows.iter().position(|workflow| workflow.name == name) {
        return &mut workflows[index];
    }
    workflows.push(StoredWorkflow {
        name: name.to_string(),
        ..StoredWorkflow::default()
    });
    workflows.last_mut().expect("an entry was just pushed")
}

/// Read every workflow folder, sorted by name.
fn load_folders(root: &Path) -> Vec<StoredWorkflow> {
    store::folder_names(root)
        .iter()
        .filter_map(|name| read_entry(&root.join(name)))
        .collect()
}

/// Rebuild one workflow from its folder, `None` when the file is broken.
///
/// A record that survives being read is remembered, so a later save knows the
/// folder belongs to a live card and may remove it when that card is gone.
fn read_entry(folder: &Path) -> Option<StoredWorkflow> {
    let file = folder.join(ENTRY);
    let body = std::fs::read_to_string(&file).ok()?;
    let value: serde_json::Value = serde_json::from_str(&body).ok()?;
    let name = text(value.get("name"));
    if name.is_empty() {
        return None;
    }
    let runs = value
        .get("runs")
        .and_then(|item| item.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|run| read_run(run, Some(&name)))
                .take(MAX_RUNS)
                .collect()
        })
        .unwrap_or_default();
    let workflow = StoredWorkflow {
        name,
        category: text(value.get("category")),
        hidden: value
            .get("hidden")
            .and_then(|flag| flag.as_bool())
            .unwrap_or(false),
        runs,
    };
    store::remember_file(&file, &body);
    Some(workflow)
}

/// Delete folders whose workflow is no longer in the list.
///
/// Only folders this run has read or written are removed: an unreadable record
/// is not proof that nobody needs it.
fn remove_orphans(root: &Path, names: &[String]) {
    for name in store::folder_names(root) {
        if names.iter().any(|saved| saved == &name) {
            continue;
        }
        let folder = root.join(&name);
        let file = folder.join(ENTRY);
        if !store::known(&file) {
            continue;
        }
        match std::fs::remove_dir_all(&folder) {
            Ok(()) => store::forget(&file),
            Err(error) => eprintln!("无法删除 {}：{error}", folder.display()),
        }
    }
}

/// What an older version kept in `sidebar.json` and `runs.json`.
fn legacy_entries(dir: &Path) -> Vec<StoredWorkflow> {
    let mut workflows: Vec<StoredWorkflow> = Vec::new();
    if let Some(value) = read_json(&dir.join(LEGACY_SIDEBAR)) {
        if let Some(list) = value.get("workflows").and_then(|item| item.as_array()) {
            for item in list {
                let name = text(item.get("name"));
                if !name.is_empty() {
                    entry(&mut workflows, &name).category = text(item.get("category"));
                }
            }
        }
        if let Some(list) = value.get("hidden").and_then(|item| item.as_array()) {
            for name in list.iter().filter_map(|item| item.as_str()) {
                let name = name.trim();
                if !name.is_empty() {
                    entry(&mut workflows, name).hidden = true;
                }
            }
        }
    }
    if let Some(value) = read_json(&dir.join(LEGACY_RUNS))
        && let Some(list) = value.get("runs").and_then(|item| item.as_array())
    {
        for run in list
            .iter()
            .filter_map(|run| read_run(run, None))
            .take(MAX_RUNS)
        {
            entry(&mut workflows, &run.workflow).runs.push(run);
        }
    }
    workflows
}

/// A JSON file, `None` when it is missing or unreadable.
fn read_json(file: &Path) -> Option<serde_json::Value> {
    let text = std::fs::read_to_string(file).ok()?;
    serde_json::from_str(&text).ok()
}

/// One workflow as it is written to disk.
fn entry_value(workflow: &StoredWorkflow) -> serde_json::Value {
    let runs: Vec<serde_json::Value> = workflow
        .runs
        .iter()
        .take(MAX_RUNS)
        .map(|run| {
            serde_json::json!({
                "manifest": run.manifest,
                "ok": run.ok,
                "cancelled": run.cancelled,
                "durationMs": run.duration_ms,
                "steps": run.steps,
                "failedSteps": run.failed_steps,
                "finishedMs": run.finished_ms,
            })
        })
        .collect();
    serde_json::json!({
        "name": workflow.name,
        "category": workflow.category,
        "hidden": workflow.hidden,
        "runs": runs,
    })
}

/// Rebuild one run; without a `workflow` argument the name comes from the file.
fn read_run(value: &serde_json::Value, workflow: Option<&str>) -> Option<RunSummary> {
    let workflow = workflow.map_or_else(|| text(value.get("workflow")), str::to_string);
    if workflow.is_empty() {
        return None;
    }
    Some(RunSummary {
        workflow,
        manifest: text(value.get("manifest")),
        ok: flag(value, "ok"),
        cancelled: flag(value, "cancelled"),
        duration_ms: number(value, "durationMs"),
        steps: number(value, "steps") as usize,
        failed_steps: number(value, "failedSteps") as usize,
        finished_ms: number(value, "finishedMs"),
    })
}

/// A trimmed string field, empty when it is absent.
fn text(value: Option<&serde_json::Value>) -> String {
    text_of(value).trim().to_string()
}

/// Turn a JSON value into a string.
fn text_of(value: Option<&serde_json::Value>) -> String {
    value
        .and_then(|item| item.as_str())
        .unwrap_or_default()
        .to_string()
}

/// A boolean field, false when it is absent.
fn flag(value: &serde_json::Value, key: &str) -> bool {
    value
        .get(key)
        .and_then(|item| item.as_bool())
        .unwrap_or(false)
}

/// A number field, zero when it is absent.
fn number(value: &serde_json::Value, key: &str) -> u64 {
    value.get(key).and_then(|item| item.as_u64()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A throw-away data directory.
    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dsh-flow-ui-workflows-{}-{tag}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// One run of `workflow`, finished at `finished_ms`.
    fn run(workflow: &str, finished_ms: u64) -> RunSummary {
        RunSummary {
            workflow: workflow.to_string(),
            manifest: format!("{workflow}/flow.yml"),
            ok: true,
            cancelled: false,
            duration_ms: 1234,
            steps: 3,
            failed_steps: 1,
            finished_ms,
        }
    }

    #[test]
    fn writes_one_folder_per_workflow() {
        let dir = temp_dir("folders");
        let workflows = vec![
            StoredWorkflow {
                name: "repo-health".to_string(),
                category: "c2".to_string(),
                hidden: false,
                runs: vec![run("repo-health", 100)],
            },
            StoredWorkflow {
                name: "build".to_string(),
                hidden: true,
                ..StoredWorkflow::default()
            },
        ];
        save(&dir, &workflows).unwrap();

        assert!(!dir.join(LEGACY_RUNS).exists(), "runs have no shared file");
        let first = dir.join(FOLDER).join("repo-health").join(ENTRY);
        let second = dir.join(FOLDER).join("build").join(ENTRY);
        assert!(first.is_file());
        assert!(second.is_file());

        let loaded = load(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(loaded.len(), 2);
        let health = loaded
            .iter()
            .find(|item| item.name == "repo-health")
            .expect("repo-health");
        assert_eq!(health.category, "c2");
        assert_eq!(health.runs, vec![run("repo-health", 100)]);
        // The run remembers which workflow it belongs to.
        assert_eq!(health.runs[0].workflow, "repo-health");
        let build = loaded
            .iter()
            .find(|item| item.name == "build")
            .expect("build");
        assert!(build.hidden);
    }

    #[test]
    fn a_workflow_with_nothing_to_say_keeps_no_file() {
        let dir = temp_dir("empty");
        save(
            &dir,
            &[StoredWorkflow {
                name: "blank".to_string(),
                ..StoredWorkflow::default()
            }],
        )
        .unwrap();
        let folder = dir.join(FOLDER).join("blank").exists();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(!folder);
    }

    #[test]
    fn folds_the_single_runs_file_and_the_sidebar_entries_into_folders() {
        let dir = temp_dir("legacy");
        std::fs::write(
            dir.join(LEGACY_SIDEBAR),
            r#"{"categories":[{"id":"c1","name":"对话"}],"workflows":[
                {"name":"repo-health","category":"c2"},
                {"name":"triage","category":"c2"}],"hidden":["triage"]}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join(LEGACY_RUNS),
            r#"{"runs":[{"workflow":"repo-health","ok":true,"finishedMs":900},
                        {"workflow":"build","ok":false,"finishedMs":800}]}"#,
        )
        .unwrap();

        let loaded = load(&dir);
        assert_eq!(loaded.len(), 3);
        assert!(!dir.join(LEGACY_RUNS).exists(), "the old file went away");
        assert!(dir.join(FOLDER).join("build").join(ENTRY).is_file());

        let health = loaded
            .iter()
            .find(|item| item.name == "repo-health")
            .expect("repo-health");
        assert_eq!(health.category, "c2");
        assert_eq!(health.runs.len(), 1);
        let triage = loaded
            .iter()
            .find(|item| item.name == "triage")
            .expect("triage");
        assert!(triage.hidden);
        assert_eq!(triage.category, "c2");

        let again = load(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(again.len(), 3, "the folders are the source of truth now");
    }

    #[test]
    fn a_cut_short_split_only_adds_what_is_missing() {
        let dir = temp_dir("partial");
        save(
            &dir,
            &[StoredWorkflow {
                name: "build".to_string(),
                category: "c2".to_string(),
                hidden: false,
                runs: vec![run("build", 900)],
            }],
        )
        .unwrap();
        // A split that stopped before deleting the old file leaves it behind.
        std::fs::write(
            dir.join(LEGACY_RUNS),
            r#"{"runs":[{"workflow":"build","manifest":"build/flow.yml","finishedMs":900},
                        {"workflow":"triage","ok":true,"finishedMs":800}]}"#,
        )
        .unwrap();

        let loaded = load(&dir);
        assert!(!dir.join(LEGACY_RUNS).exists());
        let build = loaded
            .iter()
            .find(|item| item.name == "build")
            .expect("build");
        // The folder already said "c2"; the run it already has is not doubled.
        assert_eq!(build.category, "c2");
        assert_eq!(build.runs.len(), 1);
        let triage = loaded
            .iter()
            .find(|item| item.name == "triage")
            .expect("triage");
        assert_eq!(triage.runs.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn merges_the_three_lists_by_workflow_name() {
        let filings = vec![("repo-health".to_string(), "c2".to_string())];
        let hidden = vec!["triage".to_string()];
        let runs = vec![run("repo-health", 200), run("triage", 100)];
        let workflows = collect(&filings, &hidden, &runs);

        assert_eq!(workflows.len(), 2);
        let health = workflows
            .iter()
            .find(|item| item.name == "repo-health")
            .expect("repo-health");
        assert_eq!(health.category, "c2");
        assert!(!health.hidden);
        assert_eq!(health.runs, vec![run("repo-health", 200)]);
        let triage = workflows
            .iter()
            .find(|item| item.name == "triage")
            .expect("triage");
        assert!(triage.hidden);
        assert_eq!(triage.runs, vec![run("triage", 100)]);
    }

    #[test]
    fn keeps_the_newest_runs_only() {
        let dir = temp_dir("cap");
        let runs: Vec<RunSummary> = (0..MAX_RUNS + 10)
            .map(|index| run("build", index as u64))
            .collect();
        let workflows = collect(&[], &[], &runs);
        assert_eq!(workflows[0].runs.len(), MAX_RUNS);
        assert_eq!(workflows[0].runs[0].finished_ms, 0, "newest first");

        save(&dir, &workflows).unwrap();
        let loaded = load(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(loaded[0].runs.len(), MAX_RUNS);
    }

    #[test]
    fn tolerates_a_missing_or_broken_record() {
        let dir = temp_dir("broken");
        assert!(load(&dir).is_empty());
        std::fs::create_dir_all(dir.join(FOLDER).join("bad")).unwrap();
        std::fs::write(dir.join(FOLDER).join("bad").join(ENTRY), "{not json").unwrap();
        let loaded = load(&dir);
        let kept = dir.join(FOLDER).join("bad").join(ENTRY).is_file();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(loaded.is_empty());
        assert!(kept, "an unreadable record is not deleted");
    }

    #[test]
    fn drops_entries_without_a_name() {
        let dir = temp_dir("sparse");
        std::fs::create_dir_all(dir.join(FOLDER).join("blank")).unwrap();
        std::fs::write(
            dir.join(FOLDER).join("blank").join(ENTRY),
            r#"{"name":"  ","runs":[]}"#,
        )
        .unwrap();
        let loaded = load(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(loaded.is_empty());
    }
}
