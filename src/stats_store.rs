//! Persistent statistics ledger.
//!
//! The statistics page used to read every conversation's `requests.jsonl` on
//! demand. That made deleting a conversation delete its history from the
//! figures as well. This module keeps an independent ledger under
//! `<data>/stats`:
//!
//! - `requests.jsonl` holds request details for the last 30 days;
//! - `daily/*.json` holds one permanent aggregate per UTC day;
//! - `meta.json` records the schema and whether the old conversation files were
//!   imported once.
//!
//! Appending a request updates both files. Deleting a conversation no longer
//! touches the ledger.

use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// One day in milliseconds.
pub const DAY_MS: u64 = 86_400_000;
/// Request details are kept for this long; daily aggregates are permanent.
pub const KEEP_DETAIL_MS: u64 = 30 * DAY_MS;
/// File names inside the ledger.
const REQUESTS: &str = "requests.jsonl";
const DAILY: &str = "daily";
const META: &str = "meta.json";

/// One request as the statistics page counts it.
#[derive(Clone, Debug, Default)]
pub struct StatsRecord {
    /// When the answer arrived, in epoch milliseconds.
    pub at_ms: u64,
    /// Folder of the conversation it belonged to.
    pub chat: String,
    /// Conversation title at the time of the request.
    pub title: String,
    /// Which question it was in that conversation, counting from one.
    pub turn: u64,
    /// The question label, when the save kept one.
    pub question: String,
    /// Model that answered.
    pub model: String,
    /// Tokens the endpoint billed for the whole question.
    pub tokens: u64,
    /// Prompt + answer of the last round.
    pub context: u64,
    /// Tool calls the question made.
    pub tools: u64,
    /// True when the time is the conversation's last change, not the answer's.
    pub estimated: bool,
}

impl StatsRecord {
    /// The JSON line stored in `requests.jsonl` and in a daily aggregate.
    pub fn to_value(&self) -> Value {
        json!({
            "atMs": self.at_ms,
            "chat": self.chat,
            "title": self.title,
            "turn": self.turn,
            "question": self.question,
            "model": self.model,
            "tokens": self.tokens,
            "context": self.context,
            "tools": self.tools,
            "estimated": self.estimated,
        })
    }

    /// Read one line back, `None` when it carries no usable timestamp.
    fn from_value(value: &Value) -> Option<Self> {
        let at_ms = value.get("atMs")?.as_u64()?;
        if at_ms == 0 {
            return None;
        }
        Some(Self {
            at_ms,
            chat: text(value, "chat"),
            title: text(value, "title"),
            turn: value.get("turn").and_then(Value::as_u64).unwrap_or(0),
            question: text(value, "question"),
            model: text(value, "model"),
            tokens: value.get("tokens").and_then(Value::as_u64).unwrap_or(0),
            context: value.get("context").and_then(Value::as_u64).unwrap_or(0),
            tools: value.get("tools").and_then(Value::as_u64).unwrap_or(0),
            estimated: value
                .get("estimated")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
    }
}

/// One model's contribution to a day.
#[derive(Clone, Debug, Default)]
pub struct ModelCount {
    pub requests: u64,
    pub tokens: u64,
}

/// Permanent aggregate for one UTC day.
#[derive(Clone, Debug, Default)]
pub struct DayRollup {
    /// Midnight of the day, epoch milliseconds.
    pub day_ms: u64,
    pub requests: u64,
    pub tokens: u64,
    pub prompt: u64,
    pub tools: u64,
    /// Conversations that answered at least once that day.
    pub chats: Vec<String>,
    /// Model shares for the day.
    pub models: BTreeMap<String, ModelCount>,
    /// True when any record was placed at a conversation's last change.
    pub estimated: bool,
}
impl DayRollup {
    fn to_value(&self) -> Value {
        let models: serde_json::Map<String, Value> = self
            .models
            .iter()
            .map(|(name, count)| {
                (
                    name.clone(),
                    json!({ "requests": count.requests, "tokens": count.tokens }),
                )
            })
            .collect();
        json!({
            "dayMs": self.day_ms,
            "requests": self.requests,
            "tokens": self.tokens,
            "prompt": self.prompt,
            "tools": self.tools,
            "chats": self.chats,
            "models": models,
            "estimated": self.estimated,
        })
    }

    fn from_value(value: &Value) -> Option<Self> {
        let day_ms = value.get("dayMs")?.as_u64()?;
        let mut models = BTreeMap::new();
        if let Some(map) = value.get("models").and_then(Value::as_object) {
            for (name, item) in map {
                models.insert(
                    name.clone(),
                    ModelCount {
                        requests: item.get("requests").and_then(Value::as_u64).unwrap_or(0),
                        tokens: item.get("tokens").and_then(Value::as_u64).unwrap_or(0),
                    },
                );
            }
        }
        Some(Self {
            day_ms,
            requests: value.get("requests").and_then(Value::as_u64).unwrap_or(0),
            tokens: value.get("tokens").and_then(Value::as_u64).unwrap_or(0),
            prompt: value.get("prompt").and_then(Value::as_u64).unwrap_or(0),
            tools: value.get("tools").and_then(Value::as_u64).unwrap_or(0),
            chats: value
                .get("chats")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
            models,
            estimated: value
                .get("estimated")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
    }
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// The ledger folder under the program's fixed data directory.
pub fn ledger_dir() -> PathBuf {
    crate::config::AiSettings::default_data_dir().join("stats")
}

fn requests_path(dir: &Path) -> PathBuf {
    dir.join(REQUESTS)
}

fn daily_dir(dir: &Path) -> PathBuf {
    dir.join(DAILY)
}

fn meta_path(dir: &Path) -> PathBuf {
    dir.join(META)
}

/// Whether the old conversation files were already imported.
pub fn initialized(dir: &Path) -> bool {
    let Ok(body) = std::fs::read_to_string(meta_path(dir)) else {
        return false;
    };
    serde_json::from_str::<Value>(&body)
        .ok()
        .and_then(|value| value.get("migrated").and_then(Value::as_bool))
        .unwrap_or(false)
}

fn mark_initialized(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|error| format!("无法创建 {}：{error}", dir.display()))?;
    let body = json!({
        "version": 1,
        "migrated": true,
        "migratedAtMs": now_ms(),
        "detailDays": 30,
    })
    .to_string();
    crate::store::write_atomic(&meta_path(dir), &body)
        .map_err(|error| format!("无法写入统计标记：{error}"))
}

/// Whether any ledger data exists, even without a migration marker.
fn ledger_exists(dir: &Path) -> bool {
    requests_path(dir).is_file()
        || std::fs::read_dir(daily_dir(dir))
            .map(|entries| entries.filter_map(Result::ok).next().is_some())
            .unwrap_or(false)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as u64)
}

fn day_start(at_ms: u64) -> u64 {
    at_ms / DAY_MS * DAY_MS
}

/// Days since the epoch → (year, month, day), UTC.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    if month <= 2 {
        year += 1;
    }
    (year, month, day)
}

fn day_name(day_ms: u64) -> String {
    let (year, month, day) = civil_from_days((day_ms / DAY_MS) as i64);
    format!("{year:04}-{month:02}-{day:02}.json")
}

fn day_path(dir: &Path, day_ms: u64) -> PathBuf {
    daily_dir(dir).join(day_name(day_ms))
}
fn read_records_all(dir: &Path) -> Vec<StatsRecord> {
    let Ok(body) = std::fs::read_to_string(requests_path(dir)) else {
        return Vec::new();
    };
    let mut records: Vec<StatsRecord> = body
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|value| StatsRecord::from_value(&value))
        .collect();
    records.sort_by_key(|record| record.at_ms);
    records
}

/// Read the request details that fall inside `[since_ms, until_ms)`.
pub fn load_records(dir: &Path, since_ms: u64, until_ms: u64) -> Vec<StatsRecord> {
    read_records_all(dir)
        .into_iter()
        .filter(|record| record.at_ms >= since_ms && record.at_ms < until_ms)
        .collect()
}

fn write_records(dir: &Path, records: &[StatsRecord]) -> Result<(), String> {
    let mut body = String::new();
    for record in records {
        body.push_str(&record.to_value().to_string());
        body.push('\n');
    }
    crate::store::write_atomic(&requests_path(dir), &body)
        .map_err(|error| format!("无法写入请求明细：{error}"))
}

fn read_day(dir: &Path, day_ms: u64) -> DayRollup {
    std::fs::read_to_string(day_path(dir, day_ms))
        .ok()
        .and_then(|body| serde_json::from_str::<Value>(&body).ok())
        .and_then(|value| DayRollup::from_value(&value))
        .unwrap_or_else(|| DayRollup {
            day_ms,
            ..DayRollup::default()
        })
}

fn add_to_day(rollup: &mut DayRollup, record: &StatsRecord) {
    rollup.requests = rollup.requests.saturating_add(1);
    rollup.tokens = rollup.tokens.saturating_add(record.tokens);
    rollup.prompt = rollup.prompt.saturating_add(record.context);
    rollup.tools = rollup.tools.saturating_add(record.tools);
    rollup.estimated |= record.estimated;
    let model = record.model.trim();
    let name = if model.is_empty() { "未知模型" } else { model };
    let entry = rollup.models.entry(name.to_string()).or_default();
    entry.requests = entry.requests.saturating_add(1);
    entry.tokens = entry.tokens.saturating_add(record.tokens);
}

fn write_days(dir: &Path, records: &[StatsRecord]) -> Result<(), String> {
    let mut grouped: BTreeMap<u64, Vec<&StatsRecord>> = BTreeMap::new();
    for record in records {
        grouped.entry(day_start(record.at_ms)).or_default().push(record);
    }
    std::fs::create_dir_all(daily_dir(dir))
        .map_err(|error| format!("无法创建 {}：{error}", daily_dir(dir).display()))?;
    for (day_ms, day_records) in grouped {
        let mut rollup = read_day(dir, day_ms);
        let mut chats: BTreeSet<String> = rollup.chats.iter().cloned().collect();
        for record in day_records {
            add_to_day(&mut rollup, record);
            if !record.chat.trim().is_empty() {
                chats.insert(record.chat.clone());
            }
        }
        rollup.chats = chats.into_iter().collect();
        let body = rollup.to_value().to_string();
        crate::store::write_atomic(&day_path(dir, day_ms), &body)
            .map_err(|error| format!("无法写入每日统计：{error}"))?;
    }
    Ok(())
}

/// Drop request details older than 30 days. Called at startup too, so a ledger
/// that has not received a request for a while still honors the retention limit.
pub fn prune_details(dir: &Path) -> Result<(), String> {
    let mut records = read_records_all(dir);
    let before = records.len();
    let cutoff = now_ms().saturating_sub(KEEP_DETAIL_MS);
    records.retain(|record| record.at_ms >= cutoff);
    if records.len() != before {
        write_records(dir, &records)?;
    }
    Ok(())
}

/// Add requests to the ledger: details for the last 30 days, daily aggregates
/// forever.
pub fn append_many(dir: &Path, records: &[StatsRecord]) -> Result<(), String> {
    if records.is_empty() {
        return Ok(());
    }
    std::fs::create_dir_all(dir).map_err(|error| format!("无法创建 {}：{error}", dir.display()))?;
    let cutoff = now_ms().saturating_sub(KEEP_DETAIL_MS);
    let mut details = read_records_all(dir);
    details.extend(records.iter().cloned());
    details.sort_by_key(|record| record.at_ms);
    details.retain(|record| record.at_ms >= cutoff);
    write_records(dir, &details)?;
    write_days(dir, records)
}

/// Add one completed request.
pub fn append(dir: &Path, record: &StatsRecord) -> Result<(), String> {
    append_many(dir, std::slice::from_ref(record))
}

/// Import conversations once, when upgrading from the old statistics model.
///
/// A ledger that already has data wins over the old files: if a previous run
/// wrote some records but stopped before writing `meta.json`, importing again
/// would count those requests twice.
pub fn migrate_if_needed(dir: &Path, records: &[StatsRecord]) -> Result<bool, String> {
    if initialized(dir) {
        return Ok(false);
    }
    if ledger_exists(dir) {
        mark_initialized(dir)?;
        return Ok(false);
    }
    append_many(dir, records)?;
    mark_initialized(dir)?;
    Ok(true)
}

/// Read the daily aggregates that overlap `[since_ms, until_ms)`.
pub fn load_daily(dir: &Path, since_ms: u64, until_ms: u64) -> Vec<DayRollup> {
    let Ok(entries) = std::fs::read_dir(daily_dir(dir)) else {
        return Vec::new();
    };
    let mut days: Vec<DayRollup> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".json"))
        .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
        .filter_map(|body| serde_json::from_str::<Value>(&body).ok())
        .filter_map(|value| DayRollup::from_value(&value))
        .filter(|day| day.day_ms < until_ms && day.day_ms + DAY_MS > since_ms)
        .collect();
    days.sort_by_key(|day| day.day_ms);
    days
}
#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("paagent-stats-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn record(at_ms: u64, chat: &str, tokens: u64) -> StatsRecord {
        StatsRecord {
            at_ms,
            chat: chat.to_string(),
            title: "对话".to_string(),
            turn: 1,
            question: "问题".to_string(),
            model: "deepseek-chat".to_string(),
            tokens,
            context: tokens / 2,
            tools: 1,
            estimated: false,
        }
    }

    #[test]
    fn details_are_pruned_but_days_survive() {
        let dir = temp_dir("prune");
        let now = now_ms();
        let old = now.saturating_sub(KEEP_DETAIL_MS + DAY_MS);
        append_many(&dir, &[record(old, "chat-old", 10), record(now, "chat-new", 20)])
            .unwrap();
        let details = load_records(&dir, 0, u64::MAX);
        assert_eq!(details.len(), 1, "only the recent detail remains");
        assert_eq!(details[0].chat, "chat-new");
        let days = load_daily(&dir, 0, u64::MAX);
        assert_eq!(days.len(), 2, "both days stay in the aggregate");
        assert_eq!(days.iter().map(|day| day.tokens).sum::<u64>(), 30);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn migration_runs_once_and_keeps_old_days() {
        let dir = temp_dir("migrate");
        let old = now_ms().saturating_sub(40 * DAY_MS);
        assert!(migrate_if_needed(&dir, &[record(old, "chat-1", 7)]).unwrap());
        assert!(!migrate_if_needed(&dir, &[record(old, "chat-1", 7)]).unwrap());
        let days = load_daily(&dir, 0, u64::MAX);
        assert_eq!(days.iter().map(|day| day.requests).sum::<u64>(), 1);
        assert!(load_records(&dir, 0, u64::MAX).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}