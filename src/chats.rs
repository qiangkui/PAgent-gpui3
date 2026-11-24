//! Saved conversations.
//!
//! Closing the window used to lose every chat, which made the sidebar list of
//! conversations pointless across restarts. Each transcript gets its own folder
//! under `<data directory>\chats\chat-N\chat.json` — never inside the repository
//! — so one damaged conversation cannot take the others with it. The data
//! directory defaults to `<program folder>\data` and can be changed in the
//! settings window; a `chats.json` left by an older version is split into
//! folders the first time it is read.
//!
//! `index.json` only records the order the cards are drawn in, which the
//! transcript itself cannot carry.
//!
//! This module only knows the stored shape; the window maps its own message
//! types onto it, which keeps the storage free of UI types.

use crate::store;
use std::collections::HashMap;
use std::path::Path;

/// Folder under the data directory holding one folder per conversation.
pub const FOLDER: &str = "chats";

/// The transcript inside a conversation folder.
const TRANSCRIPT: &str = "chat.json";

/// The card order, as a JSON array of folder names.
const INDEX: &str = "index.json";

/// One line per answered request, inside a conversation folder.
///
/// `chat.json` keeps only the totals: a request's full text (the system prompt,
/// the tool schemas, the replayed history) is far too big to rewrite on every
/// save, so it is appended here once, when the answer arrives, and read back
/// only when that request is opened in the context panel.
const REQUESTS: &str = "requests.jsonl";

/// Most turns one conversation keeps request details for.
///
/// The file is the only place a request's pieces survive, and a conversation can
/// run for thousands of answers; keeping every one of them would mean re-reading
/// megabytes whenever the context panel opens.
pub const MAX_REQUEST_TURNS: usize = 300;

/// Turns whose pieces are kept whole: these are the answers a reader opens again.
const KEEP_FULL_TURNS: usize = 10;

/// Pieces kept from an older turn. The category and its token estimate survive,
/// which is what the panel sums; only the text gets shorter.
const OLD_PIECE_COUNT: usize = 12;

/// Characters kept from each piece of an older turn.
const OLD_PIECE_CHARS: usize = 160;

/// The file is rewritten once it grows past this, so compaction runs a few times
/// over a very long conversation rather than on every answer.
const COMPACT_AFTER_BYTES: u64 = 4 * 1024 * 1024;

/// Every conversation in one file, as written by older versions.
pub const LEGACY_FILE: &str = "chats.json";

/// One conversation as it is stored.
pub struct StoredChat {
    /// Title shown in the sidebar.
    pub title: String,
    /// Folder under the workspace, for example `chat-3`.
    pub dir: String,
    /// Id of the sidebar category it sits in; empty for older files, which the
    /// window puts in the default category.
    pub category: String,
    /// Messages in order.
    pub messages: Vec<StoredMessage>,
    /// `<details>` sections the reader opened, as (message, block) pairs.
    pub details: Vec<(usize, usize)>,
    /// Task boxes the reader clicked, as (message, block) pairs.
    pub checks: Vec<(usize, usize)>,
    /// When the conversation last changed, in epoch milliseconds; zero for
    /// files written before this was recorded.
    pub updated_ms: u64,
    /// True when the card is held at the top of its category.
    pub pinned: bool,
    /// Text typed into the composer but never sent; empty when there is none.
    pub draft: String,
    /// Files attached but never sent, by path. Their text is read again when
    /// the window opens, so an old draft never carries a stale file body.
    pub draft_files: Vec<String>,
    /// Whether this conversation may use tools.
    pub tools: bool,
    /// Whether writes wait for approval.
    pub read_only: bool,
    /// Whether tools may touch any path.
    pub full_access: bool,
    /// Whether commands and destructive actions run without asking.
    pub auto_actions: bool,
    /// Model round-trips one question may take.
    pub max_rounds: u32,
    /// Tool calls one question may make.
    pub max_calls: u32,
    /// Mode id selecting external plugins, skills and MCP servers.
    pub mode: String,
}

/// One request's detail, as one line of `requests.jsonl`.
pub struct StoredRequest {
    /// Which question it was in the conversation, counting from one.
    pub turn: u64,
    /// When the answer arrived, in epoch milliseconds.
    pub at_ms: u64,
    /// Model that answered.
    pub model: String,
    /// Tokens the endpoint billed for the whole question.
    pub tokens: u64,
    /// Prompt + answer of the last round, which is what filled the window.
    pub context: u64,
    /// Tool calls the question made; zero for lines written before it was kept.
    pub calls: u64,
    /// Every counted piece of the request, in the order it was sent.
    pub items: Vec<StoredRequestItem>,
}

/// One counted piece of a request: a block, a tool schema, a turn.
pub struct StoredRequestItem {
    /// Category key, one of `system` / `tools` / `user` / `injected` /
    /// `skills` / `assistant` / `toolResults`.
    pub kind: String,
    /// What it is: a block name, a tool name, the first line of a turn.
    pub label: String,
    /// The whole text that went to the model.
    pub text: String,
    /// Estimated tokens for this piece.
    pub tokens: u64,
}

/// Whether a conversation already has a request file.
///
/// The statistics scan only needs a transcript when this is false: an answer's
/// figures live in that file once it exists, and copying a whole history across
/// threads just to read it back would be wasted work.
pub fn has_requests(dir: &Path, chat: &str) -> bool {
    dir.join(FOLDER).join(chat).join(REQUESTS).is_file()
}

/// Append one request's detail to its conversation folder.
///
/// One line per request instead of rewriting `chat.json`: the transcript is
/// written on every change, and carrying the whole prompt in it would make each
/// save megabytes.
pub fn append_request(dir: &Path, chat: &str, request: &StoredRequest) -> Result<(), String> {
    let folder = dir.join(FOLDER).join(chat);
    std::fs::create_dir_all(&folder)
        .map_err(|error| format!("无法创建 {}：{error}", folder.display()))?;
    let path = folder.join(REQUESTS);
    let line = request_value(request).to_string();
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|error| format!("无法写入请求明细：{error}"))?;
    writeln!(file, "{line}").map_err(|error| format!("无法写入请求明细：{error}"))?;
    // Once the file is large it is rewritten in the same call. A failure here
    // only costs the panel's detail, never the answer that was just stored.
    let too_big = file.metadata().map_or(0, |meta| meta.len()) > COMPACT_AFTER_BYTES;
    drop(file);
    if too_big {
        let _ = compact_requests(&path);
    }
    Ok(())
}

/// Keep one request file small enough to be read on demand.
///
/// The newest line of each turn wins, exactly as [`load_requests`] reads them.
/// Turns past [`MAX_REQUEST_TURNS`] are dropped, and the text of everything but
/// the newest [`KEEP_FULL_TURNS`] is shortened: the categories and their token
/// estimates stay, so the panel's sums do not change.
fn compact_requests(file: &Path) -> Result<(), String> {
    let Ok(body) = std::fs::read_to_string(file) else {
        return Ok(());
    };
    let mut by_turn: std::collections::BTreeMap<u64, serde_json::Value> =
        std::collections::BTreeMap::new();
    for line in body.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            // A line that cannot be read back is not worth keeping.
            continue;
        };
        let turn = value.get("turn").and_then(|turn| turn.as_u64()).unwrap_or(0);
        if turn > 0 {
            by_turn.insert(turn, value);
        }
    }
    let turns: Vec<u64> = by_turn.keys().copied().collect();
    let keep_from = turns.len().saturating_sub(MAX_REQUEST_TURNS);
    let full_from = turns.len().saturating_sub(KEEP_FULL_TURNS);
    let mut kept = String::new();
    for (index, turn) in turns.iter().enumerate() {
        if index < keep_from {
            continue;
        }
        let Some(mut value) = by_turn.remove(turn) else {
            continue;
        };
        if index < full_from {
            clip_items(&mut value);
        }
        kept.push_str(&value.to_string());
        kept.push('\n');
    }
    crate::store::write_atomic(file, &kept)
        .map_err(|error| format!("无法压缩请求明细：{error}"))
}

/// Shorten the pieces of one older request, keeping what the panel sums.
fn clip_items(value: &mut serde_json::Value) {
    let Some(items) = value
        .get_mut("items")
        .and_then(|items| items.as_array_mut())
    else {
        return;
    };
    items.truncate(OLD_PIECE_COUNT);
    for item in items.iter_mut() {
        let Some(text) = item.get("text").and_then(|text| text.as_str()) else {
            continue;
        };
        if text.chars().count() <= OLD_PIECE_CHARS {
            continue;
        }
        let kept: String = text.chars().take(OLD_PIECE_CHARS).collect();
        item["text"] = serde_json::Value::String(format!("{kept}…（已精简）"));
    }
}

/// Read one request's detail back, or `None` when it was never stored.
///
/// The last matching line wins, so a repeated turn (a rewritten conversation)
/// still opens the newest record.
pub fn load_request(dir: &Path, chat: &str, turn: u64) -> Option<StoredRequest> {
    load_requests(dir, chat).remove(&turn)
}

/// Read every request detail of a conversation, keyed by turn.
///
/// The context panel needs the pieces of every listed request to add each
/// round's figures up, not only the one that is open. The file is read once;
/// a repeated turn (a rewritten conversation) keeps its newest line.
pub fn load_requests(dir: &Path, chat: &str) -> HashMap<u64, StoredRequest> {
    let Ok(body) = std::fs::read_to_string(dir.join(FOLDER).join(chat).join(REQUESTS)) else {
        return HashMap::new();
    };
    let mut requests = HashMap::new();
    for line in body.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let request = stored_request(&value);
        if request.turn > 0 {
            requests.insert(request.turn, request);
        }
    }
    requests
}

/// One request as it is written to its line.
fn request_value(request: &StoredRequest) -> serde_json::Value {
    serde_json::json!({
        "turn": request.turn,
        "atMs": request.at_ms,
        "model": request.model,
        "tokens": request.tokens,
        "context": request.context,
        "calls": request.calls,
        "items": request
            .items
            .iter()
            .map(|item| {
                serde_json::json!({
                    "kind": item.kind,
                    "label": item.label,
                    "text": item.text,
                    "tokens": item.tokens,
                })
            })
            .collect::<Vec<_>>(),
    })
}

/// Read a request back from its stored line.
fn stored_request(value: &serde_json::Value) -> StoredRequest {
    let items = value
        .get("items")
        .and_then(|items| items.as_array())
        .map(|items| {
            items
                .iter()
                .map(|item| StoredRequestItem {
                    kind: text_of(item.get("kind")),
                    label: text_of(item.get("label")),
                    text: text_of(item.get("text")),
                    tokens: item
                        .get("tokens")
                        .and_then(|tokens| tokens.as_u64())
                        .unwrap_or(0),
                })
                .collect()
        })
        .unwrap_or_default();
    StoredRequest {
        turn: value
            .get("turn")
            .and_then(|turn| turn.as_u64())
            .unwrap_or(0),
        at_ms: value.get("atMs").and_then(|at| at.as_u64()).unwrap_or(0),
        model: text_of(value.get("model")),
        tokens: value
            .get("tokens")
            .and_then(|tokens| tokens.as_u64())
            .unwrap_or(0),
        context: value
            .get("context")
            .and_then(|context| context.as_u64())
            .unwrap_or(0),
        calls: value
            .get("calls")
            .and_then(|calls| calls.as_u64())
            .unwrap_or(0),
        items,
    }
}

/// One message as it is stored.
pub struct StoredMessage {
    /// `user`, `assistant`, `tool` or `note`.
    pub role: String,
    /// The text as it was shown.
    pub text: String,
    /// True when it reported a problem.
    pub failed: bool,
    /// Model that wrote an answer; empty for other messages and older files.
    pub model: String,
    /// How long the answer took, in milliseconds; zero when unknown.
    pub elapsed_ms: u64,
    /// Tokens the answer cost; zero when the endpoint did not say.
    pub tokens: u64,
    /// Tokens the last request carried; zero for older files.
    pub context: u64,
    /// Prompt tokens the provider served from its cache; zero for older files
    /// and for endpoints that do not report them.
    pub cache_hits: u64,
    /// Round trips the question took, the answer included; zero for older files.
    pub rounds: u32,
    /// What each round cost, oldest first, as `[prompt, completion, calls]`.
    pub round_tokens: Vec<(u64, u64, u32)>,
    /// What that request was made of, as the category estimates in
    /// `ContextKind` order; empty for older files and for non-answers.
    pub composition: Vec<u32>,
    /// Estimated tokens its tools cost (arguments + what came back); zero for
    /// older files and for answers that used none.
    pub tools_tokens: u32,
    /// True when the answer was cut short and can be continued.
    pub interrupted: bool,
    /// What kind of attachment a file message carries: `text`, `image` or `pdf`.
    pub kind: String,
    /// Arguments of a tool call, for the expanded row.
    pub input: String,
    /// What a tool returned, for the expanded row.
    pub output: String,
    /// True when the reader left this tool row expanded.
    pub open: bool,
    /// Provider id of an approval's tool call; empty otherwise.
    pub call: String,
    /// Path of a write approval.
    pub path: String,
    /// File text before an approved write.
    pub before: String,
    /// File text after an approved write.
    pub after: String,
    /// False when the approval created the file.
    pub existed: bool,
    /// True when the saved pair can be restored.
    pub undoable: bool,
}

/// Treat the conversation folders already in `dir` as known.
///
/// A data directory that was just moved into place holds transcripts this run
/// never read or wrote; without this, deleting a card right after the move
/// would leave its folder behind and the card would come back on restart.
///
/// Only a transcript that reads back as a conversation counts. A file that does
/// not parse is *not* proof that nobody needs it: remembering it here would let
/// the next save delete its folder as an orphan.
pub fn adopt(dir: &Path) {
    let root = dir.join(FOLDER);
    for name in store::folder_names(&root) {
        let file = root.join(&name).join(TRANSCRIPT);
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        if transcript(&text).is_some() {
            store::remember_file(&file, &text);
        }
    }
}

/// Read the saved conversations from a data directory.
///
/// A missing or broken folder is skipped, so a first run or a freshly chosen
/// folder opens with one blank conversation. A single `chats.json` from an
/// older version is split into folders once and then removed.
pub fn load(dir: &Path) -> Vec<StoredChat> {
    let root = dir.join(FOLDER);
    let chats = load_folders(&root);
    if !chats.is_empty() {
        return chats;
    }
    let legacy = dir.join(LEGACY_FILE);
    if !legacy.is_file() {
        return Vec::new();
    }
    let saved = load_from(&legacy);
    if saved.is_empty() {
        return saved;
    }
    // Keep the old file until every folder is written: a failed split must not
    // cost the transcripts it could not store.
    match save(dir, &saved) {
        Ok(()) => {
            if let Err(error) = std::fs::remove_file(&legacy) {
                eprintln!("无法删除旧的 {}：{error}", legacy.display());
            }
        }
        Err(error) => eprintln!("无法把旧对话拆成文件夹：{error}"),
    }
    saved
}

/// Write the conversations into a data directory, one folder each.
///
/// Transcripts that did not change since the last write are left alone, and a
/// folder whose conversation is gone is removed, so deleting a card also
/// deletes its record.
pub fn save(dir: &Path, chats: &[StoredChat]) -> Result<(), String> {
    let root = dir.join(FOLDER);
    std::fs::create_dir_all(&root)
        .map_err(|error| format!("无法创建 {}：{error}", root.display()))?;

    let mut names: Vec<String> = Vec::with_capacity(chats.len());
    for chat in chats {
        let name = store::folder_name(&chat.dir, &names);
        let folder = root.join(&name);
        std::fs::create_dir_all(&folder)
            .map_err(|error| format!("无法创建 {}：{error}", folder.display()))?;
        let body = chat_value(chat).to_string();
        store::write_if_changed(&folder.join(TRANSCRIPT), &body)?;
        names.push(name);
    }
    let index = serde_json::to_string(&names).unwrap_or_else(|_| "[]".to_string());
    store::write_if_changed(&root.join(INDEX), &index)?;
    remove_orphans(&root, &names);
    Ok(())
}

/// Read every conversation folder, in the order the index asks for.
///
/// Folders missing from the index (an interrupted save, or one dropped in by
/// hand) come last rather than being ignored.
fn load_folders(root: &Path) -> Vec<StoredChat> {
    let mut names = read_index(root);
    for name in store::folder_names(root) {
        if !names.iter().any(|saved| saved == &name) {
            names.push(name);
        }
    }
    names
        .iter()
        .filter_map(|name| read_folder(&root.join(name)))
        .collect()
}

/// The index array, empty when it is missing or unreadable.
fn read_index(root: &Path) -> Vec<String> {
    std::fs::read_to_string(root.join(INDEX))
        .ok()
        .and_then(|text| serde_json::from_str::<Vec<String>>(&text).ok())
        .unwrap_or_default()
}

/// Rebuild one conversation from its folder, `None` when the file is broken.
///
/// A transcript that survives being read is remembered, so a later save knows
/// the folder belongs to a conversation and may remove it if that card is
/// deleted. A folder that cannot be read is left alone instead of being wiped.
fn read_folder(folder: &Path) -> Option<StoredChat> {
    let file = folder.join(TRANSCRIPT);
    let text = std::fs::read_to_string(&file).ok()?;
    let chat = transcript(&text)?;
    store::remember_file(&file, &text);
    Some(chat)
}

/// The conversation a transcript text holds, `None` when it cannot be read.
fn transcript(text: &str) -> Option<StoredChat> {
    let value = serde_json::from_str::<serde_json::Value>(text).ok()?;
    read_chat(&value)
}

/// Delete folders whose conversation is no longer in the list.
///
/// Only folders this run has read or written are removed: an unreadable
/// transcript is not proof that nobody needs it.
fn remove_orphans(root: &Path, names: &[String]) {
    for name in store::folder_names(root) {
        if names.iter().any(|saved| saved == &name) {
            continue;
        }
        let folder = root.join(&name);
        let file = folder.join(TRANSCRIPT);
        if !store::known(&file) {
            continue;
        }
        match std::fs::remove_dir_all(&folder) {
            Ok(()) => store::forget(&file),
            Err(error) => eprintln!("无法删除 {}：{error}", folder.display()),
        }
    }
}

/// Read conversations from an arbitrary file.
fn load_from(file: &Path) -> Vec<StoredChat> {
    let Ok(text) = std::fs::read_to_string(file) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    let Some(list) = value.get("chats").and_then(|item| item.as_array()) else {
        return Vec::new();
    };
    list.iter().filter_map(read_chat).collect()
}

/// Rebuild one conversation, or `None` when it carries no folder name.
fn read_chat(value: &serde_json::Value) -> Option<StoredChat> {
    let dir = text(value.get("dir"));
    if dir.is_empty() {
        return None;
    }
    let title = text(value.get("title"));
    // Old files had these settings in `config.json`; every chat now starts at
    // the maximum when the fields are missing.
    let tools = value
        .get("tools")
        .and_then(|flag| flag.as_bool())
        .unwrap_or(true);
    let read_only = value
        .get("toolsReadOnly")
        .and_then(|flag| flag.as_bool())
        .unwrap_or(false);
    let full_access = value
        .get("fullAccess")
        .and_then(|flag| flag.as_bool())
        .unwrap_or(false);
    let auto_actions = value
        .get("autoActions")
        .and_then(|flag| flag.as_bool())
        .unwrap_or(false);
    let max_rounds = value
        .get("maxRounds")
        .and_then(|item| item.as_u64())
        .map_or(crate::config::DEFAULT_MAX_ROUNDS, |item| {
            item.clamp(1, u64::from(crate::config::MAX_ROUNDS_CHOICE)) as u32
        });
    let max_calls = value
        .get("maxCalls")
        .and_then(|item| item.as_u64())
        .map_or(crate::config::DEFAULT_MAX_CALLS, |item| {
            item.clamp(1, u64::from(crate::config::MAX_CALLS_CHOICE)) as u32
        });
    let messages: Vec<StoredMessage> = value
        .get("messages")
        .and_then(|item| item.as_array())
        .map(|list| list.iter().filter_map(read_message).collect())
        .unwrap_or_default();
    Some(StoredChat {
        title: if title.is_empty() {
            "新对话".to_string()
        } else {
            title
        },
        dir,
        category: text(value.get("category")),
        messages,
        details: read_pairs(value.get("details")),
        checks: read_pairs(value.get("checks")),
        updated_ms: value
            .get("updatedMs")
            .and_then(|item| item.as_u64())
            .unwrap_or(0),
        draft: text_of(value.get("draft")),
        draft_files: read_names(value.get("draftFiles")),
        pinned: value
            .get("pinned")
            .and_then(|flag| flag.as_bool())
            .unwrap_or(false),
        tools,
        read_only,
        full_access,
        auto_actions,
        max_rounds,
        max_calls,
        mode: {
            let mode = text(value.get("mode"));
            if mode.is_empty() {
                crate::config::MODE_FULL.to_string()
            } else {
                mode
            }
        },
    })
}

/// Rebuild a saved list of paths, dropping the blanks.
fn read_names(value: Option<&serde_json::Value>) -> Vec<String> {
    value
        .and_then(|item| item.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|item| item.as_str())
                .filter(|item| !item.trim().is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Rebuild a saved list of (message, block) pairs.
fn read_pairs(value: Option<&serde_json::Value>) -> Vec<(usize, usize)> {
    value
        .and_then(|item| item.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|pair| {
                    let pair = pair.as_array()?;
                    let message = pair.first()?.as_u64()? as usize;
                    let block = pair.get(1)?.as_u64()? as usize;
                    Some((message, block))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Rebuild one message; unknown roles are kept as `note` so text is not lost.
fn read_message(value: &serde_json::Value) -> Option<StoredMessage> {
    let text = text(value.get("text"));
    if text.is_empty() {
        return None;
    }
    let role = match text_of(value.get("role")).as_str() {
        "user" => "user",
        "assistant" => "assistant",
        "tool" => "tool",
        "attachment" => "attachment",
        "approval" => "approval",
        _ => "note",
    };
    Some(StoredMessage {
        role: role.to_string(),
        text,
        failed: value
            .get("failed")
            .and_then(|flag| flag.as_bool())
            .unwrap_or(false),
        model: text_of(value.get("model")),
        elapsed_ms: value
            .get("elapsedMs")
            .and_then(|item| item.as_u64())
            .unwrap_or(0),
        tokens: value
            .get("tokens")
            .and_then(|item| item.as_u64())
            .unwrap_or(0),
        context: value
            .get("context")
            .and_then(|item| item.as_u64())
            .unwrap_or(0),
        cache_hits: value
            .get("cacheHits")
            .and_then(|item| item.as_u64())
            .unwrap_or(0),
        rounds: value
            .get("rounds")
            .and_then(|item| item.as_u64())
            .unwrap_or(0)
            .min(u64::from(u32::MAX)) as u32,
        round_tokens: value
            .get("roundTokens")
            .and_then(|items| items.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        let item = item.as_array()?;
                        let prompt = item.first()?.as_u64()?;
                        let completion = item.get(1).and_then(|count| count.as_u64()).unwrap_or(0);
                        let calls = item
                            .get(2)
                            .and_then(|count| count.as_u64())
                            .unwrap_or(0)
                            .min(u64::from(u32::MAX)) as u32;
                        Some((prompt, completion, calls))
                    })
                    .collect()
            })
            .unwrap_or_default(),
        composition: value
            .get("composition")
            .and_then(|item| item.as_array())
            .map(|list| {
                list.iter()
                    .map(|entry| entry.as_u64().unwrap_or(0).min(u64::from(u32::MAX)) as u32)
                    .collect()
            })
            .unwrap_or_default(),
        tools_tokens: value
            .get("toolsTokens")
            .and_then(|item| item.as_u64())
            .unwrap_or(0)
            .min(u64::from(u32::MAX)) as u32,
        interrupted: value
            .get("interrupted")
            .and_then(|flag| flag.as_bool())
            .unwrap_or(false),
        kind: text_of(value.get("kind")),
        input: text_of(value.get("input")),
        output: text_of(value.get("output")),
        open: value
            .get("open")
            .and_then(|flag| flag.as_bool())
            .unwrap_or(false),
        call: text_of(value.get("call")),
        path: text_of(value.get("path")),
        before: text_of(value.get("before")),
        after: text_of(value.get("after")),
        existed: value
            .get("existed")
            .and_then(|flag| flag.as_bool())
            .unwrap_or(false),
        undoable: value
            .get("undoable")
            .and_then(|flag| flag.as_bool())
            .unwrap_or(false),
    })
}

/// Write conversations to an arbitrary file, replacing what is there.
///
/// Only the tests need this: it produces the single `chats.json` an older
/// version wrote, which `load` splits into folders.
#[cfg(test)]
fn save_to(file: &Path, chats: &[StoredChat]) -> Result<(), String> {
    let items: Vec<serde_json::Value> = chats.iter().map(chat_value).collect();
    let body = serde_json::json!({ "chats": items }).to_string();

    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("无法创建 {}：{error}", parent.display()))?;
    }
    store::write_atomic(file, &body).map_err(|error| format!("无法保存对话：{error}"))
}

/// One conversation as it is written to disk.
fn chat_value(chat: &StoredChat) -> serde_json::Value {
    let messages: Vec<serde_json::Value> = chat
        .messages
        .iter()
        .map(|message| {
            serde_json::json!({
                "role": message.role,
                "text": message.text,
                "failed": message.failed,
                "model": message.model,
                "elapsedMs": message.elapsed_ms,
                "tokens": message.tokens,
                "context": message.context,
                "cacheHits": message.cache_hits,
                "rounds": message.rounds,
                "roundTokens": message
                    .round_tokens
                    .iter()
                    .map(|(prompt, completion, calls)| serde_json::json!([prompt, completion, calls]))
                    .collect::<Vec<_>>(),
                "composition": message.composition,
                "toolsTokens": message.tools_tokens,
                "interrupted": message.interrupted,
                "kind": message.kind,
                "input": message.input,
                "output": message.output,
                "open": message.open,
                "call": message.call,
                "path": message.path,
                "before": message.before,
                "after": message.after,
                "existed": message.existed,
                "undoable": message.undoable,
            })
        })
        .collect();
    serde_json::json!({
        "title": chat.title,
        "dir": chat.dir,
        "category": chat.category,
        "updatedMs": chat.updated_ms,
        "details": chat
            .details
            .iter()
            .map(|(message, block)| serde_json::json!([message, block]))
            .collect::<Vec<_>>(),
        "checks": chat
            .checks
            .iter()
            .map(|(message, block)| serde_json::json!([message, block]))
            .collect::<Vec<_>>(),
        "draft": chat.draft,
        "draftFiles": chat.draft_files,
        "pinned": chat.pinned,
        "tools": chat.tools,
        "toolsReadOnly": chat.read_only,
        "fullAccess": chat.full_access,
        "autoActions": chat.auto_actions,
        "maxRounds": chat.max_rounds,
        "maxCalls": chat.max_calls,
        "mode": chat.mode,
        "messages": messages,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A throw-away file path.
    fn temp_file(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "dsh-flow-ui-chats-{}-{tag}.json",
            std::process::id()
        ))
    }

    /// A throw-away data directory.
    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dsh-flow-ui-chats-dir-{}-{tag}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A conversation filed under `dir`.
    fn named(dir: &str, title: &str) -> StoredChat {
        StoredChat {
            dir: dir.to_string(),
            title: title.to_string(),
            ..titled(title)
        }
    }

    /// One stored message.
    fn message(role: &str, text: &str, failed: bool) -> StoredMessage {
        StoredMessage {
            role: role.to_string(),
            text: text.to_string(),
            failed,
            model: String::new(),
            elapsed_ms: 0,
            tokens: 0,
            context: 0,
            cache_hits: 0,
            rounds: 0,
            round_tokens: Vec::new(),
            composition: Vec::new(),
            tools_tokens: 0,
            interrupted: false,
            kind: "text".to_string(),
            input: String::new(),
            output: String::new(),
            open: false,
            call: String::new(),
            path: String::new(),
            before: String::new(),
            after: String::new(),
            existed: false,
            undoable: false,
        }
    }

    /// A conversation with a title and nothing else.
    fn titled(title: &str) -> StoredChat {
        StoredChat {
            title: title.to_string(),
            dir: "chat-1".to_string(),
            category: String::new(),
            messages: Vec::new(),
            details: Vec::new(),
            checks: Vec::new(),
            updated_ms: 0,
            draft: String::new(),
            draft_files: Vec::new(),
            pinned: false,
            tools: true,
            read_only: false,
            full_access: true,
            auto_actions: false,
            max_rounds: crate::config::MAX_ROUNDS_CHOICE,
            max_calls: crate::config::MAX_CALLS_CHOICE,
mode: crate::config::MODE_FULL.to_string(),
        }
    }

    #[test]
    fn a_request_detail_survives_a_round_trip_and_the_newest_line_wins() {
        use super::{
            StoredRequest, StoredRequestItem, append_request, load_request, load_requests,
        };
        let dir = temp_dir("requests");
        let first = StoredRequest {
            turn: 1,
            at_ms: 1_760_000_000_000,
            model: "deepseek-chat".to_string(),
            tokens: 1840,
            context: 1750,
            calls: 2,
            items: vec![
                StoredRequestItem {
                    kind: "system".to_string(),
                    label: "基础指令".to_string(),
                    text: "你是助手".to_string(),
                    tokens: 4,
                },
                StoredRequestItem {
                    kind: "user".to_string(),
                    label: "你好".to_string(),
                    text: "你好，介绍一下自己".to_string(),
                    tokens: 9,
                },
            ],
        };
        append_request(&dir, "chat-1", &first).expect("append");
        let loaded = load_request(&dir, "chat-1", 1).expect("line one");
        assert_eq!(loaded.model, "deepseek-chat");
        assert_eq!(loaded.tokens, 1840);
        assert_eq!(loaded.context, 1750);
        assert_eq!(loaded.calls, 2);
        assert_eq!(loaded.items.len(), 2);
        // The text is kept whole: the panel shows it, so cutting it here would
        // silently hide the end of a prompt.
        assert_eq!(loaded.items[1].text, "你好，介绍一下自己");
        assert_eq!(loaded.items[1].kind, "user");
        assert_eq!(loaded.items[1].tokens, 9);

        // A second request appends a line instead of rewriting the first.
        let mut second = StoredRequest {
            turn: 2,
            at_ms: 1_760_000_100_000,
            model: "deepseek-chat".to_string(),
            tokens: 2000,
            context: 1900,
            calls: 0,
            items: Vec::new(),
        };
        append_request(&dir, "chat-1", &second).expect("second append");
        second.items = vec![StoredRequestItem {
            kind: "tools".to_string(),
            label: "read".to_string(),
            text: "{}".to_string(),
            tokens: 1,
        }];
        // Rewriting a turn (a repeated answer) must not lose the newest line.
        append_request(&dir, "chat-1", &second).expect("rewrite");
        assert_eq!(load_request(&dir, "chat-1", 1).unwrap().tokens, 1840);
        let newest = load_request(&dir, "chat-1", 2).expect("line two");
        assert_eq!(newest.items.len(), 1);
        assert_eq!(newest.items[0].kind, "tools");
        // Reading the whole file once gives the same newest line per turn.
        let all = load_requests(&dir, "chat-1");
        assert_eq!(all.len(), 2);
        assert_eq!(all.get(&1).map(|request| request.tokens), Some(1840));
        assert_eq!(all.get(&2).map(|request| request.items.len()), Some(1));
        // A turn that was never stored, and a folder with no file, are `None`
        // rather than an error or a panic.
        assert!(load_request(&dir, "chat-1", 9).is_none());
        assert!(load_request(&dir, "chat-missing", 1).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_large_request_file_is_compacted_to_its_newest_turns() {
        use super::{
            MAX_REQUEST_TURNS, REQUESTS, StoredRequest, StoredRequestItem, compact_requests,
            load_requests,
        };
        let dir = temp_dir("compact");
        let folder = dir.join(FOLDER).join("chat-1");
        std::fs::create_dir_all(&folder).unwrap();
        let file = folder.join(REQUESTS);
        let long = "x".repeat(400);
        let mut lines = String::new();
        for turn in 1..=(MAX_REQUEST_TURNS as u64 + 20) {
            let request = StoredRequest {
                turn,
                at_ms: 1_760_000_000_000 + turn,
                model: "m".to_string(),
                tokens: 100 + turn,
                context: 90 + turn,
                calls: 0,
                items: vec![
                    StoredRequestItem {
                        kind: "user".to_string(),
                        label: format!("问 {turn}"),
                        text: long.clone(),
                        tokens: 7,
                    },
                    StoredRequestItem {
                        kind: "tools".to_string(),
                        label: "read".to_string(),
                        text: "{}".to_string(),
                        tokens: 1,
                    },
                ],
            };
            lines.push_str(&super::request_value(&request).to_string());
            lines.push('\n');
        }
        std::fs::write(&file, &lines).unwrap();

        compact_requests(&file).unwrap();
        let kept = load_requests(&dir, "chat-1");

        // The oldest turns are dropped; the newest are all there.
        assert_eq!(kept.len(), MAX_REQUEST_TURNS);
        assert!(!kept.contains_key(&1));
        assert!(kept.contains_key(&(MAX_REQUEST_TURNS as u64 + 20)));
        // A newest turn keeps its text whole; an older one is shortened but keeps
        // its category and its token estimate, which is what the panel sums.
        let newest = kept.get(&(MAX_REQUEST_TURNS as u64 + 20)).expect("newest");
        assert_eq!(newest.items[0].text.chars().count(), 400);
        let oldest = kept.get(&21).expect("oldest kept");
        assert!(oldest.items[0].text.contains("已精简"));
        assert_eq!(oldest.items[0].kind, "user");
        assert_eq!(oldest.items[0].tokens, 7);
        assert_eq!(oldest.tokens, 121);
        // Reads stay cheap: one line per turn.
        let lines = std::fs::read_to_string(&file).unwrap().lines().count();
        assert_eq!(lines, MAX_REQUEST_TURNS);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn replaces_the_file_without_leaving_a_temporary_one() {
        let file = temp_file("atomic");
        save_to(&file, &[titled("一")]).expect("first save");
        save_to(&file, &[titled("二")]).expect("second save");
        let loaded = load_from(&file);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].title, "二");
        assert!(!file.with_extension("json.tmp").exists());
        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn round_trips_a_conversation() {
        let file = temp_file("round");
        let mut answer = message("assistant", "## 标题\n正文", false);
        answer.model = "deepseek-chat".to_string();
        answer.elapsed_ms = 2400;
        answer.tokens = 1840;
        answer.context = 1750;
        answer.composition = vec![210, 780, 90, 320, 0, 260, 90];
        answer.tools_tokens = 412;
        answer.rounds = 3;
        answer.round_tokens = vec![(1200, 100, 2), (1500, 120, 1), (1750, 140, 0)];
        let chat = StoredChat {
            title: "统计文件".to_string(),
            dir: "chat-3".to_string(),
            category: "c1".to_string(),
            updated_ms: 1_760_000_000_000,
            details: vec![(1, 2)],
            checks: vec![(0, 3)],
            draft: "还没发的草稿".to_string(),
            draft_files: vec!["C:\\tmp\\a.txt".to_string()],
            pinned: true,
            tools: false,
            read_only: true,
            full_access: false,
            auto_actions: false,
            max_rounds: 12,
            max_calls: 40,
mode: crate::config::MODE_FULL.to_string(),
            messages: vec![
                message("user", "你好", false),
                {
                    let mut tool = message("tool", "read · a.txt — 已读取 2 行", false);
                    tool.input = "{\"path\":\"a.txt\"}".to_string();
                    tool.output = "1\talpha\n".to_string();
                    tool.open = true;
                    tool.call = "call-1".to_string();
                    tool.path = "a.txt".to_string();
                    tool.before = "old".to_string();
                    tool.after = "new".to_string();
                    tool.existed = true;
                    tool.undoable = true;
                    tool
                },
                answer,
                message("note", "出错了", true),
            ],
        };

        save_to(&file, std::slice::from_ref(&chat)).unwrap();
        let loaded = load_from(&file);
        let _ = std::fs::remove_file(&file);

        assert_eq!(loaded.len(), 1);
        let restored = &loaded[0];
        assert_eq!(restored.title, "统计文件");
        assert_eq!(restored.dir, "chat-3");
        assert_eq!(restored.category, "c1");
        assert_eq!(restored.messages.len(), 4);
        // The model that wrote an answer is worth keeping with it.
        assert_eq!(restored.messages[2].model, "deepseek-chat");
        assert_eq!(restored.messages[2].elapsed_ms, 2400);
        assert_eq!(restored.messages[2].tokens, 1840);
        // The request size travels with it, so the header's gauge survives a
        // restart instead of reading "上下文 —".
        assert_eq!(restored.messages[2].context, 1750);
        // Each answer remembers how its request was split, so the context panel
        // can show a past request instead of only the next one.
        assert_eq!(
            restored.messages[2].composition,
            vec![210, 780, 90, 320, 0, 260, 90]
        );
        // What its tools cost travels with it, so the request list can show it.
        assert_eq!(restored.messages[2].tools_tokens, 412);
        assert_eq!(restored.messages[0].tools_tokens, 0);
        // So does the round-trip count behind the bill.
        assert_eq!(restored.messages[2].rounds, 3);
        assert_eq!(restored.messages[0].rounds, 0);
        // And each round's own counts, so the repeated prompts survive a restart.
        assert_eq!(
            restored.messages[2].round_tokens,
            vec![(1200, 100, 2), (1500, 120, 1), (1750, 140, 0)]
        );
        assert!(restored.messages[0].round_tokens.is_empty());
        assert!(restored.messages[0].composition.is_empty());
        // Older files have no token count and must not fail to load.
        assert_eq!(restored.messages[0].tokens, 0);
        assert_eq!(restored.messages[0].context, 0);
        // The card's "3 分钟前" needs the time the conversation last changed.
        assert_eq!(restored.updated_ms, 1_760_000_000_000);
        // Open `<details>` sections come back with the conversation.
        assert_eq!(restored.details, vec![(1, 2)]);
        // So do the task boxes someone clicked.
        assert_eq!(restored.checks, vec![(0, 3)]);
        // Unsent text and attached files survive a restart too.
        assert_eq!(restored.draft, "还没发的草稿");
        assert_eq!(restored.draft_files, vec!["C:\\tmp\\a.txt".to_string()]);
        // A pinned card stays pinned across restarts.
        assert!(restored.pinned);
        // So do the conversation's own tool settings.
        assert!(!restored.tools);
        assert!(restored.read_only);
        assert!(!restored.full_access);
        assert!(!restored.auto_actions);
        assert_eq!(restored.max_rounds, 12);
        assert_eq!(restored.max_calls, 40);
        assert!(restored.messages[0].model.is_empty());
        assert_eq!(restored.messages[0].role, "user");
        assert_eq!(restored.messages[1].text, "read · a.txt — 已读取 2 行");
        assert_eq!(restored.messages[1].input, "{\"path\":\"a.txt\"}");
        assert_eq!(restored.messages[1].output, "1\talpha\n");
        assert!(restored.messages[1].open);
        assert_eq!(restored.messages[1].call, "call-1");
        assert_eq!(restored.messages[1].path, "a.txt");
        assert_eq!(restored.messages[1].before, "old");
        assert_eq!(restored.messages[1].after, "new");
        assert!(restored.messages[1].existed);
        assert!(restored.messages[1].undoable);
        assert_eq!(restored.messages[2].text, "## 标题\n正文");
        assert!(restored.messages[3].failed);
    }

    #[test]
    fn an_approval_keeps_its_code_and_folded_state() {
        let file = temp_file("approval");
        let mut approval = message("approval", "node · 3 行代码", false);
        approval.input = "$ node -e\nconsole.log(1)".to_string();
        approval.output = "已执行".to_string();
        approval.open = false;
        approval.call = "call-1".to_string();
        let mut chat = titled("代码确认");
        chat.messages.push(approval);

        save_to(&file, &[chat]).unwrap();
        let loaded = load_from(&file);
        let _ = std::fs::remove_file(&file);

        let restored = &loaded[0].messages[0];
        assert_eq!(restored.role, "approval");
        // The code has to survive the restart, or the folded card has nothing
        // to show when it is expanded again.
        assert_eq!(restored.input, "$ node -e\nconsole.log(1)");
        assert_eq!(restored.output, "已执行");
        assert!(!restored.open, "a decided card comes back folded");
    }

    #[test]
    fn tolerates_a_missing_or_broken_file() {
        assert!(load_from(&temp_file("missing")).is_empty());
        let file = temp_file("broken");
        std::fs::write(&file, "{not json").unwrap();
        assert!(load_from(&file).is_empty());
        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn drops_entries_without_a_folder_or_text() {
        let file = temp_file("sparse");
        std::fs::write(
            &file,
            concat!(
                r#"{"chats":[{"title":"no dir","messages":[]},"#,
                r#"{"dir":"chat-2","messages":[{"role":"user","text":"  "},"#,
                r#"{"role":"weird","text":"保留"}]}]}"#
            ),
        )
        .unwrap();
        let loaded = load_from(&file);
        let _ = std::fs::remove_file(&file);

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].title, "新对话");
        assert_eq!(loaded[0].messages.len(), 1);
        assert_eq!(loaded[0].messages[0].role, "note");
        assert_eq!(loaded[0].messages[0].text, "保留");
    }

    #[test]
    fn old_chats_fall_back_to_the_default_preset() {
        let file = temp_file("old-settings");
        std::fs::write(&file, r#"{"chats":[{"dir":"chat-1","messages":[]}]}"#).unwrap();
        let loaded = load_from(&file);
        let _ = std::fs::remove_file(&file);
        let chat = &loaded[0];
        assert!(chat.tools);
        assert!(!chat.read_only);
        assert!(!chat.full_access);
        assert_eq!(chat.max_rounds, crate::config::DEFAULT_MAX_ROUNDS);
        assert_eq!(chat.max_calls, crate::config::DEFAULT_MAX_CALLS);
    }

    #[test]
    fn writes_the_documented_shape() {
        let file = temp_file("shape");
        let chat = StoredChat {
            title: "t".to_string(),
            dir: "chat-1".to_string(),
            category: String::new(),
            updated_ms: 0,
            details: Vec::new(),
            checks: Vec::new(),
            messages: Vec::new(),
            draft: String::new(),
            draft_files: Vec::new(),
            pinned: false,
            tools: true,
            read_only: false,
            full_access: true,
            auto_actions: false,
            max_rounds: crate::config::MAX_ROUNDS_CHOICE,
            max_calls: crate::config::MAX_CALLS_CHOICE,
mode: crate::config::MODE_FULL.to_string(),
        };
        save_to(&file, std::slice::from_ref(&chat)).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        let _ = std::fs::remove_file(&file);
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["chats"][0]["dir"], "chat-1");
        assert_eq!(value["chats"][0]["messages"], serde_json::json!([]));
        assert_eq!(value["chats"][0]["fullAccess"], true);
        assert_eq!(value["chats"][0]["autoActions"], false);
        assert_eq!(
            value["chats"][0]["maxRounds"],
            crate::config::MAX_ROUNDS_CHOICE
        );
    }

    #[test]
    fn writes_one_folder_per_conversation() {
        let dir = temp_dir("folders");
        let chats = vec![named("chat-1", "一"), named("chat-2", "二")];
        save(&dir, &chats).unwrap();

        // Nothing is left in one big file: each conversation owns a folder.
        assert!(!dir.join(LEGACY_FILE).exists());
        let first = dir.join(FOLDER).join("chat-1").join(TRANSCRIPT);
        let second = dir.join(FOLDER).join("chat-2").join(TRANSCRIPT);
        assert!(first.is_file());
        assert!(second.is_file());
        let index = std::fs::read_to_string(dir.join(FOLDER).join(INDEX)).unwrap();
        assert_eq!(index, r#"["chat-1","chat-2"]"#);

        let loaded = load(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        // The card order travels in the index, not in the folder names.
        assert_eq!(
            loaded
                .iter()
                .map(|chat| chat.dir.as_str())
                .collect::<Vec<_>>(),
            vec!["chat-1", "chat-2"]
        );
        assert_eq!(loaded[1].title, "二");
    }

    #[test]
    fn splits_the_single_file_an_older_version_wrote() {
        let dir = temp_dir("legacy");
        let chats = vec![named("chat-1", "旧一"), named("chat-3", "旧三")];
        save_to(&dir.join(LEGACY_FILE), &chats).unwrap();

        let loaded = load(&dir);
        assert_eq!(loaded.len(), 2);
        // The old file went away only after every folder was written.
        assert!(!dir.join(LEGACY_FILE).exists());
        assert!(dir.join(FOLDER).join("chat-1").join(TRANSCRIPT).is_file());
        assert!(dir.join(FOLDER).join("chat-3").join(TRANSCRIPT).is_file());

        let again = load(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(again.len(), 2);
        assert_eq!(again[1].title, "旧三");
    }

    #[test]
    fn deleting_a_card_removes_its_folder() {
        let dir = temp_dir("delete");
        save(&dir, &[named("chat-1", "一"), named("chat-2", "二")]).unwrap();
        // Read them back, the way a restart would.
        assert_eq!(load(&dir).len(), 2);

        save(&dir, &[named("chat-1", "一")]).unwrap();
        let gone = dir.join(FOLDER).join("chat-2").exists();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(!gone, "the deleted conversation took its folder with it");
    }

    #[test]
    fn keeps_every_transcript_inside_the_chats_folder() {
        let dir = temp_dir("escape");
        // A `dir` read from a hand-edited file must not be able to walk out.
        save(&dir, &[named("..\\..\\outside", "逃逸")]).unwrap();

        let root = dir.join(FOLDER);
        let folders = store::folder_names(&root);
        let written = folders
            .first()
            .is_some_and(|name| root.join(name).join(TRANSCRIPT).is_file());
        let escaped = dir.join("outside").exists();
        let loaded = load(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(folders.len(), 1, "{folders:?}");
        assert!(written);
        assert!(!escaped, "nothing lands outside the chats folder");
        // The transcript still remembers the folder it stands for.
        assert_eq!(loaded[0].dir, "..\\..\\outside");
    }

    #[test]
    fn adopting_a_moved_folder_lets_it_be_removed() {
        let from = temp_dir("adopt-from");
        let to = temp_dir("adopt-to");
        save(&from, &[named("chat-1", "一"), named("chat-2", "二")]).unwrap();
        // Moving the data directory carries the whole folder across.
        std::fs::rename(from.join(FOLDER), to.join(FOLDER)).unwrap();
        adopt(&to);

        save(&to, &[named("chat-1", "一")]).unwrap();
        let gone = to.join(FOLDER).join("chat-2").exists();
        let _ = std::fs::remove_dir_all(&from);
        let _ = std::fs::remove_dir_all(&to);
        assert!(
            !gone,
            "a conversation deleted after the move is gone for good"
        );
    }

    #[test]
    fn keeps_a_folder_it_could_not_read() {
        let dir = temp_dir("broken");
        save(&dir, &[named("chat-1", "一")]).unwrap();
        // A transcript that cannot be parsed is not proof that nobody wants it.
        let broken = dir.join(FOLDER).join("chat-9");
        std::fs::create_dir_all(&broken).unwrap();
        std::fs::write(broken.join(TRANSCRIPT), "{not json").unwrap();

        save(&dir, &[named("chat-1", "一")]).unwrap();
        let kept = broken.join(TRANSCRIPT).is_file();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(kept, "an unreadable transcript is left alone");
    }

    #[test]
    fn adopting_does_not_mark_a_broken_transcript_as_known() {
        let dir = temp_dir("adopt-broken");
        let root = dir.join(FOLDER);
        let good = root.join("chat-1");
        std::fs::create_dir_all(&good).unwrap();
        std::fs::write(
            good.join(TRANSCRIPT),
            chat_value(&named("chat-1", "一")).to_string(),
        )
        .unwrap();
        let broken = root.join("chat-2");
        std::fs::create_dir_all(&broken).unwrap();
        // Readable, but not a transcript. Remembering it would let the next
        // save delete the folder as an orphan, losing what it still holds.
        std::fs::write(broken.join(TRANSCRIPT), "{\"title\":\"缺目录\"}").unwrap();

        adopt(&dir);
        let good_known = store::known(&good.join(TRANSCRIPT));
        let broken_known = store::known(&broken.join(TRANSCRIPT));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(good_known, "a readable transcript counts as known");
        assert!(!broken_known, "a broken one must not be treated as an orphan");
    }
}
