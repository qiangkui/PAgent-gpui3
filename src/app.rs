//! The window: a conversation with the assistant, the workflows it can run, and
//! the state of those runs.
//!
//! The view owns presentation state only. Conversations go through
//! [`crate::ai`], runs through [`crate::engine`], and the plan pane is filled by
//! [`crate::manifest`]; none of them implement workflow or model logic here.
//!
//! The views themselves live in the submodules (`sidebar`, `chat`, `flow`,
//! `settings`, …): this file keeps the state, the shared types, and the
//! prelude their `use super::*` resolves to.

use std::cell::RefCell;
use std::ops::Range;
use std::panic::Location;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{
    Animation, AnimationExt, AnyElement, App, Background, BorderStyle, Bounds, ClipboardItem,
    Context, Corners, CursorStyle, Div, Edges, Element, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, ExternalPaths, FocusHandle, GlobalElementId, Hsla, InspectorElementId,
    IntoElement, KeyDownEvent, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, PaintQuad,
    Pixels, Point, Render, ScrollEvent, ScrollHandle, SharedString, TextRun, UTF16Selection,
    Window, WindowControlArea, WindowHandle, div, point, px, relative, rgb, size, svg,
};

use crate::ai::{AiEvent, AiRequest, Speaker, Turn};
use crate::config::{AiSettings, Connection, PRESETS, Reasoning, model_versions, offered_models};
use crate::engine::{
    self, RunCredentials, RunMessage, RunOutcome, RunSession, StepView, WorkflowEntry,
};
use crate::manifest::{InputSpec, ManifestSpec};
use crate::markdown;
use crate::select_text::{
    DragEvent, OnDrag, SelectableTable, SelectableText, TableLook, TokenColors,
};
use crate::text_input::{FieldKey, TextField};
use crate::theme::{Palette, ThemeKind};

/// Size of a field's right-click menu, so the dismiss sheet can leave it alone.
const FIELD_MENU_WIDTH: f32 = 112.;
/// Four 12px rows (`text_xs` + `py(px(1.))`), three 1px gaps, the card's `p_1`
/// and its 1px border. Keep in step with `render_field_menu`.
const FIELD_MENU_HEIGHT: f32 = 86.;

/// Font size of text fields; the renderer and the click hit-test must agree.
const FIELD_FONT_SIZE: f32 = 14.;
/// Left padding inside a field row (`px_3`), where its text starts.
const FIELD_PADDING: f32 = 12.;

/// Lines kept per run before the oldest are dropped.
const MAX_LOG_LINES: usize = 4000;
/// Lines drawn per frame; older ones stay in the record.
const VISIBLE_LOG_LINES: usize = 400;
/// Runs kept in the history list.
const MAX_HISTORY: usize = 12;
/// Stderr/stdout lines shown by an expanded step.
const DETAIL_LINES: usize = 40;
/// How many earlier turns of a conversation are sent back with a question.
const MAX_TURNS: usize = 40;
/// Rough token budget for the replayed turns, so a long conversation cannot
/// fill the model window even when the character count looks small (CJK).
const MAX_HISTORY_TOKENS: usize = 30_000;
/// How many files one question may carry.
const MAX_ATTACHMENTS: usize = 5;
/// Characters kept from one attached file.
const MAX_ATTACHMENT_CHARS: usize = 120_000;
/// Bytes read from one attached file; anything bigger is not a snippet.
const MAX_ATTACHMENT_BYTES: u64 = 4 * 1024 * 1024;
/// Bytes one question's attachments may add up to. A provider rejects a request
/// long before five full-size files fit, so the window stops earlier with a
/// message instead of a failure from the endpoint.
const MAX_ATTACHMENT_TOTAL_BYTES: usize = 12 * 1024 * 1024;
/// Characters of an attached file drawn when its card is expanded.
const MAX_ATTACHMENT_PREVIEW: usize = 20_000;
/// Per-conversation instruction file, read with every question.
pub(crate) const CHAT_RULES: &str = "AGENTS.md";
/// Messages drawn at once in a long conversation; older ones wait behind a chip.
const CHAT_WINDOW: usize = 60;
/// How many older messages one click on that chip brings back.
const CHAT_EARLIER_STEP: usize = 40;
/// Blocks the transcript draws before it hides more of the oldest messages: one
/// enormous answer must not make every keystroke re-lay it out.
const CHAT_BLOCK_BUDGET: usize = 300;
/// Messages kept no matter how many blocks they hold, so a single huge answer
/// still shows the question above it.
const CHAT_WINDOW_MIN: usize = 8;
/// Search hits painted per block; past this the rest of the answer stays plain.
const MAX_FIND_MARKS: usize = 200;
/// How long a floating hint stays on screen, fade included.
const TOAST_TOTAL: Duration = Duration::from_millis(1100);
/// How long the hint stays fully opaque before it starts to fade.
const TOAST_HOLD: Duration = Duration::from_millis(300);
/// Width the window opens with, matching `main::preferred_size`; only used to
/// seed the answer width before the first frame measures the real pane.
const INITIAL_WINDOW_WIDTH: f32 = 1140.;
/// Height of the custom title bar.
const TITLEBAR_HEIGHT: f32 = 36.;
/// Narrowest the sidebar may be dragged.
const SIDEBAR_MIN: f32 = 208.;
/// Widest the sidebar may be dragged.
const SIDEBAR_MAX: f32 = 420.;
/// Sidebar width at startup.
const SIDEBAR_DEFAULT: f32 = 264.;
/// Width of the drag handle between the sidebar and the pane.
const SPLITTER_WIDTH: f32 = 6.;
/// Radius of the right-hand panel's top-left corner.
const PANEL_RADIUS: f32 = 14.;
/// Room left below the transcript for the floating composer, in its one-line
/// form. The card and the status capsule under it together take this much, so
/// the last message can scroll clear of both.
const COMPOSER_CLEARANCE: f32 = 186.;
/// Widest the composer card grows; past this it stays centred instead of
/// stretching with the window. The status capsule shares the same width.
const COMPOSER_MAX_WIDTH: f32 = 760.;
/// Height of the status capsule under the composer.
const STATUS_CAPSULE_HEIGHT: f32 = 30.;
/// Gap between the composer card and the capsule.
const STATUS_CAPSULE_GAP: f32 = 6.;
/// Height of one line of the composer.
const COMPOSER_LINE_HEIGHT: f32 = 20.;
/// Height of the attachment row above the input line, gap included.
const ATTACHMENT_ROW_HEIGHT: f32 = 28.;

/// Instruction handed to the model with every question.
const SYSTEM_PROMPT: &str = "你是 dsh-flow 桌面应用里的助手。回答简洁，优先使用中文；\
需要操作步骤时给出可直接执行的命令，不要编造仓库里没有的文件。";

/// Tools the assistant may call when tools are on.
const ALL_TOOLS: [&str; 18] = [
    "read", "write", "edit", "replace", "move", "delete", "list", "glob", "tree", "grep",
    "symbols", "git", "shell", "task", "python", "node", "http", "mcp",
];

/// The engine tools a request may use after its mode removed MCP.
///
/// The `mcp` tool is a gateway that can link a server at runtime, so it only
/// belongs in a request that actually carries at least one MCP server.
pub(crate) fn mode_tools(selection: &crate::config::ModeSelection) -> Vec<&'static str> {
    ALL_TOOLS
        .iter()
        .copied()
        .filter(|name| *name != "mcp" || !selection.mcp_servers.is_empty())
        .collect()
}

/// Added whenever the assistant answers: how to work, not only what it can use.
const WORK_PROMPT: &str = "工作方式：动手前先看清结构（list / tree / glob / grep），需要时先写一个简短计划再执行；\
改完代码要跑相关测试或构建，并把真实结果写进回答；没有运行过的验证不要声称通过。\
多文件里同一段文本的批量替换用 replace；同一文件多处不同改动用 edit 的批量 edits；需求不明确时先问一句再改。\
不要读取或输出密钥类文件（.env、*.pem、凭据文件）；确实需要时只说明用途。";

/// Added when the assistant may call tools: the rules a schema cannot carry.
///
/// What each tool does, and its parameters, are already in the `tools` payload
/// of every request; repeating them here would send the same text twice on
/// every round, so this only states permissions and runtime behaviour.
const TOOL_PROMPT: &str = "各工具的作用与参数见随请求附带的工具定义（name / description / parameters），这里只补充权限与运行时规则。\
当前目录是这个对话自己的工作目录：默认权限下相对路径从当前目录开始，可以用 .. 但必须留在工作区内，工作区外的绝对路径会被拒绝；开启完全权限后可以操作电脑上的任意路径。\
glob 与 grep 在默认权限下以整个工作区为根（命中以绝对路径给出），完全权限下只扫当前目录；不带 path 的 tree 在默认权限下也以工作区为根，不是这个对话自己的目录。\
它们还会搜索引擎和工作流参考目录（只读，命中结果以绝对路径给出），这些目录只能读、不能写入。\
write / edit / replace 在「工作区内修改」与「完全权限」下直接执行，在「仅可查看」下会先请用户确认；move / delete 与 shell / python / node 在前两档都先请用户确认，只有「完全权限」会直接执行。\
shell 命令引用的路径必须落在工作区内（沙箱检查，完全权限不检查），cwd 也受同一套校验；需要常驻的进程（dev server 等）用 background=true，不要在前台等它超时。\
python 与 node 运行时如果目录里有 .venv 会优先用它，node_modules/.bin 与 .venv 的 bin 会加到 PATH 前面，输出只保留首尾。\
后台任务虽然在这轮问答之后继续运行，但会随「停止」或关闭窗口一起结束，用完主动 task stop。\
被拒绝时不要原样重试，也不要换个写法重试，先说明原因或提出更小的改动。\
mcp 用来连接并调用 MCP 服务器：list 先看 mcpserver 目录里有什么，link 按 id 连接或直接给 command/url 现场连接并列出工具，call 调用某个服务器的工具，连接在本次提问内保持。\
写入后不必再读回同一内容；不要读取或打印环境变量里的密钥。";

/// Sent when the reader asks the assistant to pick up a stopped answer.
const CONTINUE_PROMPT: &str = "请从上一条回答停下的地方接着写，不要重复已经写过的内容。";

/// How long a native dialog may stay unanswered before a click drops the record.
///
/// A dialog can legitimately be open for a while, but one that was killed (or
/// hidden by a security tool) never reports back; without a ceiling every later
/// click would be swallowed by a stale entry.
pub(crate) const PICKER_STALE_AFTER: Duration = Duration::from_secs(180);

/// How long the settings page waits after the last keystroke before writing.
///
/// Typing an API key or an endpoint changes the text on every character; the
/// file (and the credential store with it) is written once the typing pauses,
/// not forty times.
pub(crate) const AI_SAVE_DEBOUNCE: Duration = Duration::from_millis(400);

/// Whether an open picker should be forgotten before the next one starts.
/// @param elapsed - how long ago the dialog was opened.
pub(crate) fn picker_stale(elapsed: Duration) -> bool {
    elapsed > PICKER_STALE_AFTER
}

/// What a native dialog reported back.
///
/// Kept apart from the answer itself so a dialog that failed to start is not
/// mistaken for one the reader cancelled.
pub(crate) enum PickResult {
    /// Files chosen for the composer.
    Files(Vec<std::path::PathBuf>),
    /// One folder chosen to be installed as a skill or an external plugin.
    Folder(std::path::PathBuf),
    /// The dialog closed without a choice.
    Cancelled,
}

/// What a native dialog was opened for.
///
/// The answer has to be routed when it arrives: a dialog cannot change what it
/// was asking for while it is up.
#[derive(Clone, Debug)]
pub(crate) enum PickPurpose {
    /// Attach the chosen files to the conversation in this folder.
    Attach(String),
    /// Copy the chosen folder in as a skill.
    Skill,
    /// Copy the chosen folder in as an external plugin.
    Plugin,
}

/// What the main pane shows.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum View {
    /// Conversation with the assistant; the default.
    Chat,
    /// One workflow: plan, run, log.
    Flow,
    /// Integrations that are still planned.
    Plugins,
    /// MCP servers linked to the assistant.
    Mcp,
    /// Skills: prompt packages switched on for every question.
    Skills,
    /// Usage over a chosen window: requests, tokens, models.
    Stats,
}

/// One combined tool permission preset, matching dsh's three levels.
///
/// dsh bundles a sandbox reach and an approval policy into one preset:
/// `read-only`, `workspace-write` (asks before wider steps) and
/// `danger-full-access` (no approval prompts). Keeping the same three levels —
/// and the same names, labels and glyphs — means a person moving between the
/// two apps reads the same choice the same way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ToolMode {
    /// 仅可查看：reads directly; every change and command asks.
    ReadOnly,
    /// 工作区内修改：read/write inside the workspace; commands ask.
    Workspace,
    /// 完全权限：read/write anywhere and no approval prompts.
    Full,
}

impl ToolMode {
    /// Every level, in the order dsh lists them.
    pub(crate) const ALL: [ToolMode; 3] = [ToolMode::ReadOnly, ToolMode::Workspace, ToolMode::Full];

    /// The level a conversation is currently on.
    ///
    /// A conversation from an older build may still have tools switched off;
    /// that reads as the narrowest level, which is the closest thing left.
    pub(crate) fn of(chat: &Chat) -> Self {
        if chat.read_only || !chat.tools {
            Self::ReadOnly
        } else if chat.full_access {
            Self::Full
        } else {
            Self::Workspace
        }
    }

    /// Name on the chip and in the menu, worded as in dsh.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::ReadOnly => "仅可查看",
            Self::Workspace => "工作区内修改",
            Self::Full => "完全权限",
        }
    }

    /// The glyph dsh draws for this level.
    pub(crate) fn icon(self) -> &'static str {
        match self {
            Self::ReadOnly => "icons/permission-read-only.svg",
            Self::Workspace => "icons/permission-workspace.svg",
            Self::Full => "icons/permission-full.svg",
        }
    }

    /// One line under the menu row, saying exactly what will be asked.
    pub(crate) fn hint(self) -> &'static str {
        match self {
            Self::ReadOnly => "读取直接可用；写入、移动、删除与命令都先询问",
            Self::Workspace => "工作区内直接读写；移动 / 删除 / 命令先询问",
            Self::Full => "任意路径直接读写，命令与删除不再询问，请谨慎使用",
        }
    }

    /// Write this level into a conversation.
    ///
    /// The three presets always keep tools on: dsh has no "no tools" level, and
    /// a conversation without tools can no longer be picked.
    pub(crate) fn apply(self, chat: &mut Chat) {
        chat.tools = true;
        chat.read_only = self == Self::ReadOnly;
        chat.full_access = self == Self::Full;
        chat.auto_actions = self == Self::Full;
        if self == Self::ReadOnly {
            chat.auto_review = false;
        }
    }
}

/// Tool permission and budgets a new conversation starts with.
#[derive(Clone, Copy)]
pub(crate) struct ChatDefaults {
    /// Combined permission level.
    mode: ToolMode,
    /// Model round-trips one question may take.
    max_rounds: u32,
    /// Tool calls one question may make.
    max_calls: u32,
}

impl Default for ChatDefaults {
    fn default() -> Self {
        Self {
            // Fixed: 工作区内修改 + 6 轮 / 20 次. The workspace is writable,
            // anything wider still asks.
            mode: ToolMode::Workspace,
            max_rounds: crate::config::DEFAULT_MAX_ROUNDS,
            max_calls: crate::config::DEFAULT_MAX_CALLS,
        }
    }
}

/// The blocks of the settings page, listed on its left.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SettingsTab {
    /// Colours the window uses.
    Interface,
    /// Model connections.
    Ai,
    /// Which external plugins, skills and MCP servers reach the model.
    Modes,
    /// Instructions that apply to the open conversation only.
    Instructions,
    /// Data locations, backups, restores and exports.
    Data,
    /// Paths, versions and diagnostics.
    Maintenance,
}

impl SettingsTab {
    /// Every block, in the order the index shows them.
    const ALL: [SettingsTab; 6] = [
        SettingsTab::Interface,
        SettingsTab::Ai,
        SettingsTab::Modes,
        SettingsTab::Instructions,
        SettingsTab::Data,
        SettingsTab::Maintenance,
    ];

    /// Name shown in the index and as the block's heading.
    fn label(self) -> &'static str {
        match self {
            SettingsTab::Interface => "界面",
            SettingsTab::Ai => "AI",
            SettingsTab::Modes => "模式",
            SettingsTab::Instructions => "指令",
            SettingsTab::Data => "数据",
            SettingsTab::Maintenance => "诊断",
        }
    }
}

/// Index the answer being written is drawn at.
///
/// The transcript shows it before it is saved, so it needs a slot no stored
/// message can have.
pub(crate) const DRAFT_MESSAGE: usize = usize::MAX;

/// Who said one message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Role {
    /// The person.
    User,
    /// The assistant.
    Assistant,
    /// A tool call's row.
    Tool,
    /// The model's thinking, one row per round trip: the transcript keeps it in
    /// the order it arrived, between the tool rows it came before.
    Thinking,
    /// Something the window itself has to say.
    Note,
    /// A write or command waiting for a decision.
    Approval,
    /// A structured question from the model, waiting for an option.
    Question,
    /// A plan the model submitted for approval before executing it.
    Plan,
    /// A file sent with a question.
    Attachment,
}

/// What kind of file an attachment carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AttachmentKind {
    /// Text, read into the conversation.
    Text,
    /// An image, sent as bytes.
    Image,
    /// A PDF, sent as bytes.
    Pdf,
}

impl AttachmentKind {
    /// How the kind is stored in `chat.json`.
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Image => "image",
            Self::Pdf => "pdf",
        }
    }

    /// Read a stored kind; anything unknown is text.
    pub(crate) fn from_key(key: &str) -> Self {
        match key.trim().to_ascii_lowercase().as_str() {
            "image" => Self::Image,
            "pdf" => Self::Pdf,
            _ => Self::Text,
        }
    }

    /// Name used in an attachment's one-line note.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Text => "文本",
            Self::Image => "图片",
            Self::Pdf => "PDF",
        }
    }
}

/// One item of the conversation's todo list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TodoItem {
    /// What has to be done.
    pub(crate) text: String,
    /// Where it stands.
    pub(crate) status: TodoStatus,
}

/// Where one todo item stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TodoStatus {
    /// Not started.
    Pending,
    /// Being worked on.
    Active,
    /// Finished.
    Done,
}

impl TodoStatus {
    /// How the status is stored in `chat.json` and sent by the engine.
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Active => "in_progress",
            Self::Done => "completed",
        }
    }

    /// Read a stored status; anything unknown is still pending.
    pub(crate) fn from_key(key: &str) -> Self {
        match key.trim().to_ascii_lowercase().as_str() {
            "in_progress" | "active" | "doing" => Self::Active,
            "completed" | "done" => Self::Done,
            _ => Self::Pending,
        }
    }

    /// Name shown beside the item.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Pending => "待办",
            Self::Active => "进行中",
            Self::Done => "完成",
        }
    }
}

/// A file sent with a question.
#[derive(Clone, Debug)]
pub(crate) struct Attachment {
    /// Path as it was dropped, shown on the message.
    pub(crate) name: String,
    /// Whether the text or the bytes go to the model.
    pub(crate) kind: AttachmentKind,
    /// What the model reads: the text, or a note about the binary.
    pub(crate) text: String,
    /// The file itself, for images and PDFs.
    pub(crate) bytes: Vec<u8>,
    /// MIME type of the bytes, when they are sent.
    pub(crate) mime: String,
    /// True when the text was cut at [`MAX_ATTACHMENT_CHARS`].
    pub(crate) truncated: bool,
}

/// One message of a conversation.
#[derive(Clone)]
pub(crate) struct ChatMessage {
    /// Who said it.
    pub(crate) role: Role,
    /// What it says.
    pub(crate) text: String,
    /// True when it reports a failure.
    pub(crate) failed: bool,
    /// Tool a `Tool` or `Approval` row belongs to.
    pub(crate) tool: String,
    /// Model that answered, for an assistant message.
    pub(crate) model: String,
    /// How long the answer took, in milliseconds.
    pub(crate) elapsed_ms: u64,
    /// Wall clock the engine spent waiting for model round trips during that
    /// answer, in milliseconds; zero when it did not report timing (older
    /// engines and saves).
    pub(crate) model_ms: u64,
    /// Wall clock the engine spent running tools during that answer, in
    /// milliseconds; zero when it did not report timing.
    pub(crate) tool_ms: u64,
    /// Tokens the answer cost, as the endpoint reported them.
    pub(crate) tokens: u64,
    /// Tokens the request carried, for the context gauge.
    pub(crate) context: u64,
    /// Prompt tokens the provider served from its cache for that question,
    /// summed over its rounds; zero when it did not report them.
    pub(crate) cache_hits: u64,
    /// Model round trips the question took, the answer included; each one sent
    /// the whole prompt again, so the bill is about this many times the
    /// [`Self::context`] figure. Zero for answers stored before it was kept.
    pub(crate) rounds: u32,
    /// What each round cost, oldest first, as `(prompt, completion)`; empty for
    /// answers stored before it was kept.
    pub(crate) round_tokens: Vec<(u64, u64, u32)>,
    /// Estimated thinking tokens of each round, oldest first: output the bill
    /// includes that no prompt holds. Empty for providers that do not stream
    /// reasoning.
    pub(crate) round_reasoning: Vec<u32>,
    /// What that request was made of, as the category estimates in
    /// [`ContextKind::ALL`] order, with every round's copy added up; empty when
    /// it was not recorded.
    pub(crate) composition: Vec<u32>,
    /// Estimated tokens the tools cost during that request: every round's
    /// arguments and results added up. Zero for answers stored before it was
    /// recorded and for questions that used no tools.
    pub(crate) tools_tokens: u32,
    /// Which compaction layer ran on the transcript just before that answer
    /// (`prune`, `summary`, `drop`); empty when nothing was compressed. Kept as
    /// a field so the panel reads a fact instead of matching the note's prose.
    pub(crate) compaction_layer: String,
    /// Tokens that compaction saved, when one ran; zero otherwise.
    pub(crate) compaction_saved: u64,
    /// Thinking the model streamed for this answer, in order; empty for providers
    /// that send none. The transcript shows it in a collapsible block above the
    /// answer, so it is never mistaken for what the model replied.
    pub(crate) reasoning: String,
    /// Raw server-tool calls of this answer, kept so the next question can hand
    /// them back: the provider restores the pages it read from them.
    pub(crate) server_tool_calls: Vec<serde_json::Value>,
    /// True when the answer was stopped part-way.
    pub(crate) interrupted: bool,
    /// For a thinking row: the round trip it belongs to, counting from one. Zero
    /// for every other row.
    pub(crate) round: u32,
    /// Kind of file an attachment carries.
    pub(crate) kind: AttachmentKind,
    /// Tool arguments, or the diff of an approval card.
    pub(crate) input: String,
    /// What the tool returned, or the decision on an approval card.
    pub(crate) output: String,
    /// True when an approval card shows its diff.
    pub(crate) open: bool,
    /// Provider id of the call an approval card belongs to.
    pub(crate) call: String,
    /// Path the approval card is about.
    pub(crate) path: String,
    /// File contents before the change, for undo.
    pub(crate) before: String,
    /// File contents after the change, for the same reason.
    pub(crate) after: String,
    /// False when the change creates the file.
    pub(crate) existed: bool,
    /// True when the undo pair is complete.
    pub(crate) undoable: bool,
}

impl ChatMessage {
    /// A message with nothing but its role and text.
    fn blank(role: Role, text: impl Into<String>) -> Self {
        Self {
            role,
            text: text.into(),
            failed: false,
            tool: String::new(),
            model: String::new(),
            elapsed_ms: 0,
            model_ms: 0,
            tool_ms: 0,
            tokens: 0,
            context: 0,
            cache_hits: 0,
            rounds: 0,
            round_tokens: Vec::new(),
            round_reasoning: Vec::new(),
            composition: Vec::new(),
            tools_tokens: 0,
            compaction_layer: String::new(),
            compaction_saved: 0,
            reasoning: String::new(),
            server_tool_calls: Vec::new(),
            interrupted: false,
            round: 0,
            kind: AttachmentKind::Text,
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

    /// A question.
    pub(crate) fn user(text: impl Into<String>) -> Self {
        Self::blank(Role::User, text)
    }

    /// An answer.
    pub(crate) fn assistant(text: impl Into<String>) -> Self {
        Self::blank(Role::Assistant, text)
    }

    /// A tool call's row; `failed` colours it.
    pub(crate) fn tool(text: impl Into<String>, failed: bool) -> Self {
        let mut message = Self::blank(Role::Tool, text);
        message.failed = failed;
        message
    }

    /// One round's thinking, kept as its own row so the transcript shows it in
    /// place, between the tool rows of the question it belongs to.
    pub(crate) fn thinking(text: impl Into<String>, round: u32) -> Self {
        let mut message = Self::blank(Role::Thinking, text);
        message.round = round;
        message
    }

    /// A note from the window: a summary, a warning, an error.
    pub(crate) fn note(text: impl Into<String>, failed: bool) -> Self {
        let mut message = Self::blank(Role::Note, text);
        message.failed = failed;
        message
    }

    /// A file sent with a question; the note is what the model reads later.
    pub(crate) fn attachment(name: String, text: String) -> Self {
        let mut message = Self::blank(Role::Attachment, text);
        message.path = name;
        message
    }

    /// The same, for a file whose bytes are sent rather than its text.
    pub(crate) fn attachment_of(kind: AttachmentKind, name: String, text: String) -> Self {
        let mut message = Self::attachment(name, text);
        message.kind = kind;
        message
    }

    /// A write or command waiting for a decision, with its diff.
    pub(crate) fn approval(
        call: String,
        path: String,
        text: String,
        diff: String,
        before: String,
        after: String,
        existed: bool,
        undoable: bool,
    ) -> Self {
        let mut message = Self::blank(Role::Approval, text);
        message.call = call;
        message.path = path;
        message.input = diff;
        message.before = before;
        message.after = after;
        message.existed = existed;
        message.undoable = undoable;
        // A card opens with its diff so the change can be read before deciding.
        message.open = true;
        message
    }

    /// A structured question waiting for one of its options.
    pub(crate) fn question(call: String, text: String, options: String) -> Self {
        let mut message = Self::blank(Role::Question, text);
        message.call = call;
        message.input = options;
        message.tool = "ask_user_question".to_string();
        message
    }

    /// A plan submitted for approval before the model executes it.
    pub(crate) fn plan_review(call: String, text: String) -> Self {
        let mut message = Self::blank(Role::Plan, text);
        message.call = call;
        message.tool = "exit_plan_mode".to_string();
        message
    }

    /// A message read back from `chat.json`.
    pub(crate) fn restored(role: Role, text: String, failed: bool) -> Self {
        let mut message = Self::blank(role, text);
        message.failed = failed;
        message
    }
}

/// One conversation: its title, folder and messages.
#[derive(Clone)]
pub(crate) struct Chat {
    /// Title shown on the card and above the transcript.
    pub(crate) title: String,
    /// Folder under the workspace holding its files.
    pub(crate) dir: String,
    /// The transcript, oldest first.
    pub(crate) messages: Vec<ChatMessage>,
    /// When it was last touched, in milliseconds since the epoch.
    pub(crate) updated_ms: u64,
    /// Id of the sidebar category it sits in.
    pub(crate) category: String,
    /// True when its card is kept at the top of its category.
    pub(crate) pinned: bool,
    /// True when questions may use tools at all.
    pub(crate) tools: bool,
    /// True when writes and commands always ask first.
    pub(crate) read_only: bool,
    /// True when the tools may reach outside the workspace.
    pub(crate) full_access: bool,
    /// True when commands and deletions run without asking.
    pub(crate) auto_actions: bool,
    /// True when low-risk writes and read-only commands are approved by rule
    /// instead of showing a card (workspace mode only).
    pub(crate) auto_review: bool,
    /// Model round-trips one question may take.
    pub(crate) max_rounds: u32,
    /// Tool calls one question may make.
    pub(crate) max_calls: u32,
    /// Mode selecting external plugins, skills and MCP servers for this chat.
    pub(crate) mode: String,
    /// This chat's web-search override: `None` follows the connection's own
    /// switch, `Some` forces searching on or off for this conversation alone.
    pub(crate) web_search: Option<bool>,
    /// True while the conversation runs in plan mode: the model investigates
    /// and submits a plan for approval before it executes.
    pub(crate) plan: bool,
    /// The conversation's standing todo list, newest write wins.
    pub(crate) todos: Vec<TodoItem>,
    /// Commands allowed for the rest of the session, as `tool\u{0}command`.
    pub(crate) allow_session: std::collections::HashSet<String>,
    /// Collapsible blocks the reader opened, as `(message, block)`.
    pub(crate) details: std::collections::HashSet<(usize, usize)>,
    /// Checkbox items the reader ticked, as `(message, item)`.
    pub(crate) checks: std::collections::HashSet<(usize, usize)>,
}

impl Chat {
    /// A conversation on the shipped defaults.
    pub(crate) fn empty(dir: String) -> Self {
        Self::empty_with(dir, ChatDefaults::default())
    }

    /// A blank conversation at the permission and budgets given.
    pub(crate) fn empty_with(dir: String, defaults: ChatDefaults) -> Self {
        let mut chat = Self {
            title: "新对话".to_string(),
            dir,
            messages: Vec::new(),
            updated_ms: util::now_ms(),
            category: String::new(),
            pinned: false,
            tools: true,
            read_only: false,
            full_access: true,
            auto_actions: false,
            auto_review: false,
            max_rounds: defaults.max_rounds,
            max_calls: defaults.max_calls,
            mode: crate::config::MODE_FULL.to_string(),
            web_search: None,
            plan: false,
            todos: Vec::new(),
            allow_session: std::collections::HashSet::new(),
            details: std::collections::HashSet::new(),
            checks: std::collections::HashSet::new(),
        };
        defaults.mode.apply(&mut chat);
        chat
    }

    /// A conversation read back from disk.
    pub(crate) fn restored(title: String, dir: String, messages: Vec<ChatMessage>) -> Self {
        let mut chat = Self::empty(dir);
        chat.title = title;
        chat.messages = messages;
        chat
    }

    /// Mark it as touched now, so the sidebar can sort it.
    pub(crate) fn touch(&mut self) {
        self.updated_ms = util::now_ms();
    }
}

/// One sidebar category.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Category {
    /// Stable id the conversations point at.
    pub(crate) id: String,
    /// Name shown in the sidebar.
    pub(crate) name: String,
}

impl Default for Category {
    fn default() -> Self {
        Self::default_one()
    }
}

impl Category {
    /// The category a first run starts with.
    pub(crate) fn default_one() -> Self {
        Self {
            id: "default".to_string(),
            name: "对话".to_string(),
        }
    }
}

/// A conversation that was removed and can still be put back.
pub(crate) struct DeletedChat {
    /// Where it was in the list.
    pub(crate) index: usize,
    /// The conversation itself.
    pub(crate) chat: Chat,
    /// True when a blank placeholder took its place.
    pub(crate) placeholder: bool,
}

/// Which card's "…" menu is open.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CardMenu {
    /// A conversation, by index into [`DshFlowApp::chats`].
    Chat(usize),
    /// A workflow card, by index into [`DshFlowApp::workflows`].
    Workflow(usize),
}

/// One editable input of the selected manifest.
pub(crate) struct InputField {
    /// What the manifest says about it.
    pub(crate) spec: InputSpec,
    /// The box holding its value.
    pub(crate) field: TextField,
}

/// A run in flight.
pub(crate) struct ActiveRun {
    /// The engine process, shared with the poller.
    pub(crate) session: Arc<RunSession>,
    /// Name of the workflow being run.
    pub(crate) workflow_name: String,
    /// Path of its manifest.
    pub(crate) manifest: PathBuf,
    /// Path relative to the scan root, shown in the pane.
    pub(crate) manifest_label: String,
    /// The manifest itself, for the step cards.
    pub(crate) spec: ManifestSpec,
    /// The values it was started with, as `(name, value)`.
    pub(crate) inputs: Vec<(String, String)>,
    /// Log lines so far.
    pub(crate) log: Vec<String>,
}

/// A finished run.
pub(crate) struct RunRecord {
    /// Name of the workflow that ran.
    pub(crate) workflow_name: String,
    /// Path of its manifest.
    pub(crate) manifest: PathBuf,
    /// Path relative to the scan root.
    pub(crate) manifest_label: String,
    /// The manifest itself.
    pub(crate) spec: ManifestSpec,
    /// The values it ran with, as `(name, value)`.
    pub(crate) inputs: Vec<(String, String)>,
    /// What the engine reported.
    pub(crate) outcome: RunOutcome,
    /// The whole log.
    pub(crate) log: Vec<String>,
    /// True when it was cancelled.
    pub(crate) cancelled: bool,
    /// When it finished.
    pub(crate) finished: Instant,
}

impl RunRecord {
    /// The status colour a card and a row use.
    pub(crate) fn color(&self, palette: Palette) -> u32 {
        if self.cancelled {
            palette.skipped
        } else if self.outcome.ok {
            palette.ok
        } else {
            palette.failed
        }
    }
}

/// One question being answered.
pub(crate) struct PendingAnswer {
    /// Folder of the conversation it belongs to.
    pub(crate) dir: String,
    /// The engine process, shared with the poller.
    pub(crate) request: Arc<AiRequest>,
    /// Text written so far, shown in place of the answer.
    pub(crate) draft: String,
    /// Thinking streamed for this question, in arrival order: the request line
    /// keeps the whole text, and a provider that sends no live rows still gets
    /// one.
    pub(crate) reasoning: String,
    /// The row the current thinking stretch is writing to, if one is open.
    pub(crate) thinking_row: Option<usize>,
    /// How many thinking rows this question has created, so a provider that
    /// only sends the finished text still gets one row.
    pub(crate) thinking_rows: u32,
    /// True while the transcript follows the answer as it grows.
    pub(crate) follow: bool,
    /// Scroll offset when following started.
    pub(crate) bottom: Pixels,
    /// True when the conversation is on screen.
    pub(crate) viewing: bool,
    /// Provider id of an approval card being waited on.
    pub(crate) approval: Option<String>,
    /// Provider id of a structured question being waited on.
    pub(crate) question: Option<String>,
    /// Provider id of a plan review being waited on.
    pub(crate) plan: Option<String>,
    /// Writes approved before the decision arrived, waiting for their row.
    pub(crate) writes: Vec<ApprovedWrite>,
    /// Model answering, for the label.
    pub(crate) model: String,
    /// True once a tool ran, so the next text is not the answer.
    pub(crate) acted: bool,
    /// Round trip the loop is on, as the engine reported it; zero before the
    /// first `progress` event.
    pub(crate) round: u32,
    /// Tool calls the loop has finished for this question.
    pub(crate) calls: u32,
    /// Every counted piece of the request, appended to `requests.jsonl` when the
    /// answer arrives so the request can be opened again later.
    pub(crate) items: Vec<RequestItem>,
    /// Tool schemas the engine sent for this run, as `(name, schema JSON)`.
    ///
    /// The engine only reports them once a run has started, so the first
    /// question of a conversation had none when its pieces were recorded; they
    /// are added before the answer is written.
    pub(crate) schemas: Vec<(String, String)>,
    /// What to send if the reader asks again with another model.
    pub(crate) retry: RetryAsk,
}

/// A write whose decision arrived before its tool row did.
pub(crate) struct ApprovedWrite {
    /// Provider id of the call.
    pub(crate) call: String,
    /// Path the write touched.
    pub(crate) path: String,
    /// File contents before the change.
    pub(crate) before: String,
    /// File contents after the change.
    pub(crate) after: String,
    /// False when the change creates the file.
    pub(crate) existed: bool,
    /// True when the undo pair is complete.
    pub(crate) undoable: bool,
}

/// Everything a retry with another model needs.
pub(crate) struct RetryAsk {
    /// The question as it was sent.
    pub(crate) prompt: String,
    /// Working directory the question ran in.
    pub(crate) work_dir: PathBuf,
    /// System prompt it carried.
    pub(crate) system: String,
    /// Earlier turns it replayed.
    pub(crate) history: Vec<Turn>,
    /// Whether tools were allowed.
    pub(crate) tools: bool,
    /// Whether writes and commands had to ask.
    pub(crate) read_only: bool,
    /// Writable root of the workspace scope.
    pub(crate) scope_root: Option<PathBuf>,
    /// Folders the tools could read.
    pub(crate) read_only_roots: Vec<PathBuf>,
    /// Round-trip budget.
    pub(crate) max_rounds: u32,
    /// Tool-call budget.
    pub(crate) max_calls: u32,
    /// Mode id used by the original request.
    pub(crate) mode: String,
    /// Whether that request ran in plan mode.
    pub(crate) plan: bool,
    /// Whether that request carried the provider's search tools.
    pub(crate) web_search: bool,
    /// Files that went with the question, as the request carried them.
    pub(crate) attachments: Vec<crate::ai::AskAttachment>,
    /// Connections already tried, so a retry moves on.
    pub(crate) tried: Vec<String>,
}

/// Editable boxes of one model connection.
pub(crate) struct ConnFields {
    /// Model id.
    pub(crate) model: TextField,
    /// API key.
    pub(crate) api_key: TextField,
    /// Endpoint base.
    pub(crate) base_url: TextField,
}

/// A model listing in flight.
pub(crate) struct ListState {
    /// Connection it is for.
    pub(crate) index: usize,
    /// Where the thread reports back.
    pub(crate) result: std::sync::mpsc::Receiver<Result<crate::ai::ModelList, String>>,
}

/// A connection test in flight.
pub(crate) struct TestState {
    /// Connection it is for.
    pub(crate) index: usize,
    /// The one-token request being answered.
    pub(crate) request: Arc<AiRequest>,
}

/// One end of a text selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TextPoint {
    /// Message it is in; [`DRAFT_MESSAGE`] for the answer being written.
    pub(crate) message: usize,
    /// Block of that message.
    pub(crate) block: usize,
    /// Byte offset inside the block's text.
    pub(crate) index: usize,
}

impl TextPoint {
    /// Where it falls in reading order.
    fn key(self) -> (usize, usize, usize) {
        (self.message, self.block, self.index)
    }
}

/// Text selected in the transcript.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TextSelection {
    /// Where the drag started.
    pub(crate) anchor: TextPoint,
    /// Where it is now.
    pub(crate) focus: TextPoint,
}

impl TextSelection {
    /// The two ends, in reading order.
    pub(crate) fn ordered(&self) -> (TextPoint, TextPoint) {
        if self.anchor.key() <= self.focus.key() {
            (self.anchor, self.focus)
        } else {
            (self.focus, self.anchor)
        }
    }

    /// True when the drag never left its starting point.
    pub(crate) fn is_empty(&self) -> bool {
        self.anchor.key() == self.focus.key()
    }

    /// The bytes of one block this selection covers, if any.
    pub(crate) fn part_of(&self, message: usize, block: usize, len: usize) -> Option<Range<usize>> {
        let (start, end) = self.ordered();
        let key = (message, block);
        if key < (start.message, start.block) || key > (end.message, end.block) {
            return None;
        }
        let from = if key == (start.message, start.block) {
            start.index.min(len)
        } else {
            0
        };
        let to = if key == (end.message, end.block) {
            end.index.min(len)
        } else {
            len
        };
        (from < to).then_some(from..to)
    }

    /// Everything the transcript draws as text, for a select-all.
    pub(crate) fn whole_transcript(blocks: &[(usize, Vec<usize>)]) -> Option<Self> {
        let first = blocks.iter().find(|(_, blocks)| !blocks.is_empty())?;
        let last = blocks.iter().rev().find(|(_, blocks)| !blocks.is_empty())?;
        Some(Self {
            anchor: TextPoint {
                message: first.0,
                block: 0,
                index: 0,
            },
            focus: TextPoint {
                message: last.0,
                block: last.1.len() - 1,
                index: usize::MAX,
            },
        })
    }
}

/// The bar at the top of the window.
pub(crate) struct Banner {
    /// What it says.
    pub(crate) text: String,
    /// True when it reports a failure rather than a warning.
    pub(crate) error: bool,
}

impl Banner {
    /// A failure worth interrupting for.
    pub(crate) fn error(text: impl Into<String>) -> Self {
        let text = text.into();
        crate::logging::error(&text);
        Self { text, error: true }
    }
}

/// A hint that fades out on its own.
pub(crate) struct Toast {
    /// What it says.
    pub(crate) text: String,
    /// When it appeared, so a newer hint can take over.
    pub(crate) started: Instant,
    /// True when it reports a failure.
    pub(crate) failed: bool,
}

/// A box that can hold the caret.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Field {
    /// The composer above the transcript.
    Composer,
    /// One of the selected manifest's inputs.
    Input(usize),
    /// Model id of a connection.
    ConnModel(usize),
    /// API key of a connection.
    ConnKey(usize),
    /// Endpoint base of a connection.
    ConnUrl(usize),
    /// The conversation filter.
    ChatFilter,
    /// The in-conversation find box.
    ChatFind,
    /// A conversation card's title while renaming.
    ChatTitle,
    /// The inline "new category" row.
    NewCategory,
    /// A category's name while renaming.
    CategoryName,
    /// The conversation-only instructions.
    ChatInstructions,
    /// One colour role's hex value.
    ColorHex(usize),
    /// The HTTP proxy the engine's provider calls go through.
    Proxy,
    /// Name of the custom mode selected in settings.
    ModeName,
}

impl Field {
    /// Space between the box's edge and its text.
    pub(crate) fn pad(self) -> Pixels {
        match self.chrome() {
            FieldChrome::Row => px(0.),
            _ => px(FIELD_PADDING),
        }
    }

    /// How the box is drawn.
    pub(crate) fn chrome(self) -> FieldChrome {
        match self {
            // Fields that sit inside a frame of their own draw no frame.
            Self::Composer | Self::ChatFilter | Self::ChatFind => FieldChrome::Plain,
            Self::ColorHex(_) => FieldChrome::Row,
            _ => FieldChrome::Boxed,
        }
    }
}

/// The frame a field is drawn with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FieldChrome {
    /// A framed box on the panel.
    Boxed,
    /// No frame at all: the card around it is the surface.
    Plain,
    /// A bare row, for a value that belongs to the row it sits in.
    Row,
}

/// Take the pending write of one call, if it was approved before its row came.
fn take_write(writes: &mut Vec<ApprovedWrite>, call: &str) -> Option<ApprovedWrite> {
    writes
        .iter()
        .position(|write| write.call == call)
        .map(|index| writes.remove(index))
}

/// Put an approved write onto the tool row that reported it.
fn apply_write(message: &mut ChatMessage, write: Option<ApprovedWrite>) {
    let Some(write) = write else {
        return;
    };
    message.path = write.path;
    message.before = write.before;
    message.after = write.after;
    message.existed = write.existed;
    message.undoable = write.undoable;
}

/// The workflow pane's view model.
pub(crate) struct Pane<'a> {
    /// Workflow name.
    pub(crate) title: String,
    /// Its description, empty when it has none.
    pub(crate) description: String,
    /// Manifest path relative to the scan root.
    pub(crate) manifest_label: String,
    /// The manifest itself, for the "open" button.
    pub(crate) manifest: Option<&'a Path>,
    /// The steps, with whatever the run reported.
    pub(crate) rows: Vec<StepRow>,
    /// Log lines.
    pub(crate) log: &'a [String],
    /// What the engine reported, for a finished run.
    pub(crate) outcome: Option<&'a RunOutcome>,
    /// The values it ran with, as `(name, value)`.
    pub(crate) inputs: Vec<(String, String)>,
    /// True while the engine is running.
    pub(crate) running: bool,
    /// True when the run was cancelled.
    pub(crate) cancelled: bool,
    /// True when the pane shows a finished run from the history.
    pub(crate) history: bool,
    /// How long it ran, in milliseconds.
    pub(crate) elapsed_ms: u64,
}

/// One step of the workflow pane.
#[derive(Default)]
pub(crate) struct StepRow {
    /// Step id from the manifest.
    pub(crate) id: String,
    /// Step type, as the manifest names it.
    pub(crate) kind: String,
    /// One-line summary.
    pub(crate) summary: String,
    /// Condition, when the step has one.
    pub(crate) when: Option<String>,
    /// Output key the step writes.
    pub(crate) save: Option<String>,
    /// Notes the manifest carries.
    pub(crate) notes: Vec<String>,
    /// `pending`, `ok`, `failed`…
    pub(crate) status: String,
    /// How long it took.
    pub(crate) duration_ms: u64,
    /// What the engine said about it.
    pub(crate) detail: String,
    /// The command that ran.
    pub(crate) command: Option<String>,
    /// Its stderr.
    pub(crate) stderr: Option<String>,
    /// Its stdout.
    pub(crate) stdout: Option<String>,
    /// The text it produced.
    pub(crate) text: Option<String>,
    /// The model it called, when it called one.
    pub(crate) model: Option<String>,
    /// True when the step came from a cached run.
    pub(crate) cached: bool,
}

/// The root view of the application.
pub struct DshFlowApp {
    /// Directory holding the `dsh-flow` engine.
    engine_dir: PathBuf,
    /// True when the engine's CLI exists.
    engine_ready: bool,
    /// Conversation whose delete button was pressed once and now waits for a
    /// second press; anything else that happens cancels it.
    confirm_delete: Option<usize>,
    /// The conversation removed last, kept in memory so the removal can be
    /// undone; a transcript is not worth losing to a stray double click.
    undo_delete: Option<DeletedChat>,
    /// Directory scanned for manifests.
    scan_root: PathBuf,
    /// Discovered manifests.
    workflows: Vec<WorkflowEntry>,
    /// Pane shown on the right.
    view: View,
    /// Current palette.
    theme: ThemeKind,
    /// Workflow shown by [`View::Flow`].
    selected_flow: Option<usize>,
    /// Editable values for the selected manifest's inputs.
    inputs: Vec<InputField>,
    /// The run in flight, if any.
    active: Option<ActiveRun>,
    /// Finished runs, newest first.
    history: Vec<RunRecord>,
    /// Runs from earlier sessions, newest first: only what a card needs.
    past_runs: Vec<crate::workflow_store::RunSummary>,
    /// Index into [`Self::history`] when the pane shows a past run.
    selected_run: Option<usize>,
    /// Step id whose content the workflow pane shows on the right.
    selected_step: Option<String>,
    /// Conversations, newest last.
    chats: Vec<Chat>,
    /// Index into [`Self::chats`].
    active_chat: usize,
    /// Sidebar categories, in display order; the first one is the default.
    categories: Vec<Category>,
    /// Id of the open category; empty when they are all collapsed.
    open_category: String,
    /// The inline "new category" row, while its name is being typed.
    new_category: Option<TextField>,
    /// The category whose name is being edited, and its field.
    rename_category: Option<(String, TextField)>,
    /// The open "…" menu of a category row: which category, and where it opened.
    category_menu: Option<(String, Point<Pixels>)>,
    /// The open "…" menu of a card, and where it opened.
    card_menu: Option<(CardMenu, Point<Pixels>)>,
    /// Where each workflow card was moved to, as (workflow name, category id).
    workflow_categories: Vec<(String, String)>,
    /// Workflow cards taken out of the 工作区 list.
    hidden_workflows: Vec<String>,
    /// Message being composed.
    composer: TextField,
    /// Unsent text per conversation folder: a draft follows its own conversation,
    /// not the window.
    chat_drafts: std::collections::HashMap<String, String>,
    /// Unsent attached files, by conversation folder.
    attachment_drafts: std::collections::HashMap<String, Vec<Attachment>>,
    /// Text files dropped onto the conversation, waiting to be sent.
    attachments: Vec<Attachment>,
    /// Position in the question history while the composer is recalling one.
    composer_recall: Option<usize>,
    /// A short hint floating over the transcript, fading out on its own.
    toast: Option<Toast>,
    /// First transcript message drawn, per conversation folder; missing means
    /// the newest window.
    chat_from: std::collections::HashMap<String, usize>,
    /// Where each conversation was left scrolled to, by folder.
    chat_offsets: std::collections::HashMap<String, f32>,
    /// Filters the conversation cards by title and message text.
    chat_filter: TextField,
    /// The in-conversation find field.
    chat_find: TextField,
    /// True while the find bar is open.
    chat_find_open: bool,
    /// True while the keyboard-shortcut sheet is open.
    shortcuts_open: bool,
    /// Background tasks of the conversation on screen, oldest record first.
    tasks: Vec<BackgroundTask>,
    /// Folder the list was read from, so switching conversations refreshes it.
    tasks_dir: PathBuf,
    /// Task whose log the panel shows.
    task_selected: Option<String>,
    /// Last lines of that task's log.
    task_log: Vec<String>,
    /// Task waiting for a second press on 「结束」.
    task_stop_confirm: Option<String>,
    /// Scroll position of the task log box.
    task_log_scroll: ScrollHandle,
    /// Match the next jump starts from.
    chat_find_at: usize,
    /// The query `chat_find_at` belongs to, so a new query starts at the first.
    chat_find_query: String,
    /// Which tab the conversation pane shows.
    chat_tab: ChatTab,
    /// The context category whose items are shown, if any.
    context_open: Option<ContextKind>,
    /// The one item of that category whose text is expanded, if any.
    context_item: Option<(ContextKind, usize)>,
    /// Scroll position of the context breakdown.
    context_scroll: ScrollHandle,
    /// Scroll position of the totals table inside the context panel.
    totals_scroll: ScrollHandle,
    /// Scroll position of that table's scrollbar.
    totals_drag: Option<(Pixels, Pixels)>,
    /// Scroll position of the expanded category item's text.
    context_item_scroll: ScrollHandle,
    /// Scroll position of that item's scrollbar.
    context_item_drag: Option<(Pixels, Pixels)>,
    /// State of every table drawn in the window, by its id.
    tables: RefCell<std::collections::HashMap<&'static str, TableState>>,
    /// The table whose own bar is held, with where the drag started.
    table_drag: Option<(&'static str, Pixels, Pixels)>,
    /// Tool schemas the engine last reported, keyed by conversation folder; only
    /// that conversation's panel shows them.
    tool_schemas: std::collections::HashMap<String, Vec<(String, String)>>,
    /// Plugin panels built for the last request in each conversation, keyed by
    /// the conversation folder. They describe one request and are rebuilt on the
    /// next, so only that request can show them.
    plugin_panels: std::collections::HashMap<String, Vec<crate::ai::PanelReport>>,
    /// Prompt fragments the enabled plugins inject, as `(plugin id, text)`.
    plugin_prompts: Vec<(String, String)>,
    /// Tools the enabled plugins registered for the last request, as
    /// `(plugin id, tool name)`; the plugin cards count their own.
    plugin_tools: Vec<(String, String)>,
    /// Which past request the context tab shows, by its question number; `None`
    /// follows the newest answer.
    context_request: Option<u64>,
    /// Which round of the shown request to break down; `None` is the whole
    /// request, with every round's figures added up.
    context_round: Option<u32>,
    /// True when the request list and the timeline show every request instead of
    /// the newest `REQUEST_ROWS`.
    context_all_requests: bool,
    /// True when the conversation's todo card is expanded.
    todos_open: bool,
    /// The pieces of the request on show, read once from `requests.jsonl`; the
    /// key is its folder, turn and reported size, so a continued answer reloads.
    context_detail: RefCell<Option<(String, u64, u64, u64, Vec<ContextPart>)>>,
    /// The tool calls of the request on show, read once from its tool rows and
    /// kept until the transcript grows; parsing every call's arguments on each
    /// paint made scrolling a long request re-read megabytes.
    context_actions: RefCell<Option<(String, u64, usize, Vec<RequestAction>)>>,

    /// Plugin folders found on disk, refreshed when the page is opened.
    installed_plugins: Vec<crate::config::InstalledPlugin>,
    /// Plugins the engine reported as broken, as `(id, message)`.
    plugin_errors: Vec<(String, String)>,
    /// What the last plugin action did, shown under the buttons.
    plugin_note: String,
    /// The open "…" menu of an external plugin card: which plugin, and where.
    plugin_menu: Option<(String, Point<Pixels>)>,
    /// The external plugin whose delete item is armed.
    plugin_delete_confirm: Option<String>,
    /// MCP servers found on disk, refreshed when the page is opened.
    installed_mcp: Vec<crate::mcp::McpServer>,
    /// Servers the engine reported as broken, as `(id, message)`.
    mcp_errors: Vec<(String, String)>,
    /// Tools the enabled MCP servers offered for the last request, as
    /// `(server id, tool name)`; the server cards count their own.
    mcp_tools: Vec<(String, String)>,
    /// The folder the MCP page reads.
    mcp_dir: PathBuf,
    /// What the last MCP action did, shown under the buttons.
    mcp_note: String,
    /// Cards whose detail is open on the MCP page.
    open_mcp_cards: Vec<String>,
    /// The open "…" menu of an MCP card: which server, and where it opened.
    mcp_menu: Option<(String, Point<Pixels>)>,
    /// The MCP server whose delete item is armed.
    mcp_delete_confirm: Option<String>,
    /// The open skills panel: which server, and where it opened.
    mcp_skills: Option<(String, Point<Pixels>)>,
    /// The skill whose parameters are open in the skills panel, by name.
    mcp_skills_open: Option<String>,
    /// Scroll position of the skills list.
    mcp_skills_scroll: ScrollHandle,
    /// The server whose skills are being fetched, when the fetch started, and
    /// the thread's answer. Fetching is on demand so a question never pays for
    /// it, and only one runs at a time.
    mcp_fetch: Option<(
        String,
        Instant,
        std::sync::mpsc::Receiver<Result<crate::mcp::FetchedSkills, String>>,
    )>,
    /// Skill folders found on disk, refreshed when the page is opened.
    installed_skills: Vec<crate::skill::InstalledSkill>,
    /// The folder the Skill page reads and opens.
    skills_dir: PathBuf,
    /// What the last Skill action did, shown under the heading.
    skill_note: String,
    /// The open "…" menu of a skill card: which skill, and where it opened.
    skill_menu: Option<(String, Point<Pixels>)>,
    /// The skill whose delete item is armed.
    skill_delete_confirm: Option<String>,
    /// The last breakdown built for the 上下文 tab, with the conversation state
    /// it was built from. The schemas and the model enter as fingerprints, so a
    /// different tool set or model still rebuilds it, and the answer reserve the
    /// active connection contributes is part of the key as well.
    context_cache: RefCell<
        Option<(
            (usize, u64, usize, usize, u64, usize, u64, u64),
            ContextReport,
        )>,
    >,
    /// The window the statistics page covers.
    stats_range: StatsRange,
    /// Bumped when a request reaches the permanent ledger, so the cached report
    /// is rebuilt without consulting the conversation list.
    stats_generation: u64,
    /// The statistics report on show, with the ledger state and time bucket it
    /// was built from, and the requests it read.
    stats_cache: RefCell<Option<(StatsKey, StatsReport, Rc<Vec<StatsPoint>>)>>,
    /// The statistics scan running in the background, and the state it was
    /// started for; `None` when nothing is being read.
    stats_loading: RefCell<Option<StatsKey>>,
    /// Scroll position of the statistics page.
    stats_scroll: ScrollHandle,
    /// Cards whose description is open on the plugins page.
    open_plugin_cards: Vec<String>,
    /// The conversation whose title is being edited, and the field doing it.
    renaming: Option<(usize, TextField)>,
    /// Everything `config.json` holds.
    ai: AiSettings,
    /// Editable boxes of the model connections, one set per connection.
    conn_fields: Vec<ConnFields>,
    /// The caption button the pointer is over, painted by the window itself.
    ///
    /// Those cells are non-client areas for the platform, so gpui's own hover
    /// hit-test does not always see the move that enters them.
    title_hover: Option<WindowControlArea>,
    /// The open native dialog, the thread answering it, and what the answer is
    /// for: switching chats while an attach dialog is up must not attach the
    /// chosen files to the wrong conversation.
    picker: Option<(std::sync::mpsc::Receiver<PickResult>, Instant, PickPurpose)>,
    /// True while the model menu is open.
    model_menu: bool,
    /// True while the tool menu is open.
    tools_menu: bool,
    /// True while the budget menu is open.
    /// True while the attach menu (files / commands) is open.
    attach_menu: bool,
    /// Which model tab the AI block shows.
    model_tab: usize,
    /// What the last connection action said.
    ai_status: Option<(String, bool)>,
    /// Counter that decides which deferred settings write is the newest one.
    ai_save_generation: u64,
    /// True while a typed change has not reached the disk yet.
    ai_save_pending: bool,
    /// The HTTP proxy box on the AI page.
    proxy_field: TextField,
    /// The `node` found at startup, for the diagnostics page.
    node_version: Option<String>,
    /// What the last backup or export said.
    maintenance_note: String,
    /// True when that outcome was a failure, so the page can paint it red.
    maintenance_failed: bool,
    /// True after the first press of "恢复最近备份".
    restore_armed: bool,
    /// The answer whose changed-file list is open, if any.
    changes_open: Option<usize>,
    /// Block of the settings page on show.
    settings_tab: SettingsTab,
    /// Custom mode selected in settings; built-in ids are allowed.
    mode_selected: String,
    /// Note under the mode editor after a create/delete/toggle.
    mode_note: String,
    /// Rename box for the selected custom mode.
    mode_name: TextField,
    /// True while the composer's mode menu is open.
    mode_menu: bool,
    /// The conversation-only instruction box.
    instructions: TextField,
    /// Folder the instruction box belongs to, so a switch reloads it.
    instructions_chat: Option<String>,
    /// What saving the instructions said.
    instructions_status: Option<(String, bool)>,
    /// The colour row whose swatches are open.
    color_menu: Option<usize>,
    /// One value box per colour role.
    color_fields: Vec<TextField>,
    /// What the last test of each connection said, as `(text, failed)`.
    conn_status: Vec<Option<(String, bool)>>,
    /// The model listing in flight.
    listing: Option<ListState>,
    /// Connections whose model listing was asked for once already, by id: a card
    /// keeps its place in the list when another one is added or removed.
    list_attempted: std::collections::HashSet<String>,
    /// The connection test in flight.
    testing: Option<TestState>,
    /// Questions being answered, in the order they were asked.
    pending: Vec<PendingAnswer>,
    /// Conversations whose dropped turns are being condensed right now.
    ///
    /// Keyed by conversation folder, so two questions in the same conversation
    /// cannot start two summarization calls for the same span.
    compacting: std::collections::HashSet<String>,
    /// Which text box has the caret.
    focused: Option<Field>,
    /// Focus handle of the transcript, so a click can take focus from a field.
    content_focus: FocusHandle,
    /// The bar at the top of the window.
    banner: Option<Banner>,
    /// Scroll position of the run log.
    log_scroll: ScrollHandle,
    /// Scroll position of the step list.
    steps_scroll: ScrollHandle,
    /// Scroll position of the transcript.
    chat_scroll: ScrollHandle,
    /// Text system of the frame being built, so the composer's wrapped height
    /// can be measured before the composer itself is drawn.
    window_text: Option<Rc<gpui::WindowTextSystem>>,
    /// Scroll position of each collapsible block, by `(message, block)`.
    block_scroll: RefCell<std::collections::HashMap<(usize, usize), ScrollHandle>>,
    /// Parsed markdown of each message, kept between frames.
    parsed: RefCell<std::collections::HashMap<usize, (String, Rc<Vec<crate::markdown::Rich>>)>>,
    /// Width of the sidebar in logical pixels.
    sidebar_width: f32,
    /// Width an answer is wrapped to.
    answer_width: f32,
    /// Side padding of the transcript.
    chat_gutter: f32,
    /// True while the splitter is being dragged.
    resizing_sidebar: bool,
    /// The conversation card whose delete button is under the pointer.
    hovered_delete: Option<usize>,
    /// The answer whose text was copied last.
    copied: Option<usize>,
    /// The code block whose text was copied last, as `(message, block)`.
    copied_code: Option<(usize, usize)>,
    /// True for a moment after the whole conversation was copied.
    copied_chat: bool,
    /// The title Windows shows.
    window_title: String,
    /// The settings window, once it was opened.
    settings_window: Option<WindowHandle<SettingsWindow>>,
    /// The text selected with the mouse, if any.
    selected_text: Option<TextSelection>,
    /// True while the mouse is dragging out a selection.
    text_dragging: bool,
    /// Where each field was laid out, for hit testing and the IME window.
    field_bounds: Rc<RefCell<Vec<(usize, Bounds<Pixels>)>>>,
    /// The field a press started in, while the button is still down.
    field_drag: Option<Field>,
    /// True while the caret blinks off.
    caret_blink: bool,
    /// True while the caret is shown.
    caret_on: bool,
    /// The open text box menu: which field, and where it opened.
    field_menu: Option<(Field, Point<Pixels>)>,
    /// Drag state of the transcript scrollbar.
    scroll_drag: Option<(Pixels, Pixels)>,
    /// Drag state of a tool row's own scrollbar.
    tool_drag: Option<(usize, Pixels, Pixels)>,
    /// Drag state of the MCP skills panel's scrollbar.
    mcp_skills_drag: Option<(Pixels, Pixels)>,
    /// True once the window size was fitted to the screen.
    fitted: bool,
    /// DPI scale the window was last drawn at.
    scale_factor: f32,
}

impl DshFlowApp {
    /// Build the window from what is on disk: conversations, sidebar, settings.
    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        let ai = AiSettings::load();
        let home = AiSettings::home();
        // A restore planned in the previous session runs before any record is
        // read, so the window opens on the restored files and no stale state can
        // overwrite them. Config keeps the current connection/path settings.
        let restore_notice = match crate::backup::apply_pending_restore(&home, &ai.data_dir()) {
            Ok(Some(report)) => Some((
                format!(
                    "已从 {} 恢复记录；恢复前的状态保存在 {}。",
                    report.source.display(),
                    report.safety.display()
                ),
                false,
            )),
            Ok(None) => None,
            Err(error) => Some((format!("恢复备份失败：{error}"), true)),
        };
        if let Some((text, error)) = &restore_notice {
            crate::logging::log_line(if *error { "ERROR" } else { "INFO" }, text);
        }
        // Every provider call runs in a child process; a configured proxy is
        // handed to those children instead of touching this process' own
        // environment.
        engine::set_proxy(&ai.proxy);
        // Asked once here rather than on every repaint of the diagnostics page.
        let node_version = engine::node_version();
        let proxy_text = ai.proxy.clone();
        let data = ai.data_dir();
        // The sidebar file decides the categories, which one is open, how wide
        // the panel was left, and which conversation was on screen.
        let saved = load_sidebar(&data);
        let defaults = ChatDefaults::default();
        let LoadedChats {
            chats,
            drafts,
            files,
        } = load_chats(&data, defaults);
        let chats = file_unknown_categories(chats, &saved.categories);
        let active_chat = active_chat_index(&chats, &saved.active);
        let filed = load_workflows(&data, &saved.categories);
        let chat_drafts: std::collections::HashMap<String, String> = drafts.into_iter().collect();
        // Attachments are read again rather than stored with the draft, so a file
        // edited since last time comes back as it is now.
        let attachment_drafts: std::collections::HashMap<String, Vec<Attachment>> = files
            .into_iter()
            .map(|(dir, paths)| {
                let kept = paths
                    .iter()
                    .filter_map(|path| read_attachment(Path::new(path)).ok())
                    .collect();
                (dir, kept)
            })
            .collect();
        let tasks_dir = ai.workspace_dir().join(&chats[active_chat].dir);
        let engine_dir = engine::locate_engine_dir();
        let engine_ready = engine::engine_ready(&engine_dir);
        // The MCP folder is found once; the page rescans it when opened.
        let mcp_dir = crate::mcp::locate_dir();
        // Skills live next to the program, like plugins and MCP servers.
        let skills_dir = AiSettings::skill_dir().unwrap_or_else(|| ai.data_dir().join("skills"));
        // The folder exists even before the first skill is added, so it can be
        // opened from Explorer without hunting for the right place.
        let _ = std::fs::create_dir_all(&skills_dir);
        // Manifests are scanned once, where the settings say the workspace is.
        // Manifests are scanned where the engine shipped its examples: the
        // folder holding `dsh-flow`, a few levels above `examples/<name>`.
        let scan_root = engine_dir
            .parent()
            .map_or_else(|| ai.workspace_dir(), Path::to_path_buf);
        let workflows = engine::discover(&scan_root);
        let theme = ThemeKind::from_name(&ai.theme);
        let palette = crate::theme::themed(theme, &ai.colors);
        let colors_text: Vec<String> = crate::theme::COLOR_ROLES
            .iter()
            .map(|role| crate::theme::hex((role.get)(&palette)))
            .collect();
        let conn_fields = build_conn_fields(&ai, cx);
        let conn_status = vec![None; ai.connections.len()];
        let mut app = Self {
            engine_dir,
            engine_ready,
            confirm_delete: None,
            undo_delete: None,
            scan_root,
            workflows,
            view: View::Chat,
            theme,
            selected_flow: None,
            inputs: Vec::new(),
            active: None,
            history: Vec::new(),
            past_runs: filed.runs,
            selected_run: None,
            selected_step: None,
            chats,
            active_chat,
            categories: saved.categories,
            open_category: saved.open,
            new_category: None,
            rename_category: None,
            category_menu: None,
            card_menu: None,
            workflow_categories: filed.categories,
            hidden_workflows: filed.hidden,
            composer: TextField::new(cx),
            chat_drafts,
            attachment_drafts,
            attachments: Vec::new(),
            composer_recall: None,
            toast: None,
            chat_from: std::collections::HashMap::new(),
            chat_offsets: std::collections::HashMap::new(),
            chat_filter: TextField::new(cx),
            chat_find: TextField::new(cx),
            chat_find_open: false,
            shortcuts_open: false,
            tasks: Vec::new(),
            tasks_dir,
            task_selected: None,
            task_log: Vec::new(),
            task_stop_confirm: None,
            task_log_scroll: ScrollHandle::new(),
            chat_find_at: 0,
            chat_find_query: String::new(),
            chat_tab: ChatTab::Chat,
            context_open: None,
            context_item: None,
            context_scroll: ScrollHandle::new(),
            totals_scroll: ScrollHandle::new(),
            totals_drag: None,
            context_item_scroll: ScrollHandle::new(),
            context_item_drag: None,
            tables: RefCell::new(std::collections::HashMap::new()),
            table_drag: None,
            tool_schemas: std::collections::HashMap::new(),
            plugin_panels: std::collections::HashMap::new(),
            plugin_prompts: Vec::new(),
            plugin_tools: Vec::new(),
            context_request: None,
            context_round: None,
            context_all_requests: false,
            todos_open: false,
            context_detail: RefCell::new(None),
            context_actions: RefCell::new(None),
            installed_plugins: crate::config::installed_plugins(),
            plugin_errors: Vec::new(),
            plugin_note: String::new(),
            plugin_menu: None,
            plugin_delete_confirm: None,
            installed_mcp: crate::mcp::scanned(&mcp_dir),
            mcp_errors: Vec::new(),
            mcp_tools: Vec::new(),
            mcp_dir,
            mcp_note: String::new(),
            open_mcp_cards: Vec::new(),
            mcp_menu: None,
            mcp_delete_confirm: None,
            mcp_skills: None,
            mcp_skills_open: None,
            mcp_skills_scroll: ScrollHandle::new(),
            mcp_fetch: None,
            installed_skills: crate::skill::scanned_skills(&skills_dir),
            skills_dir,
            skill_note: String::new(),
            skill_menu: None,
            skill_delete_confirm: None,
            context_cache: RefCell::new(None),
            stats_range: StatsRange::Week,
            stats_generation: 0,
            stats_cache: RefCell::new(None),
            stats_loading: RefCell::new(None),
            stats_scroll: ScrollHandle::new(),
            open_plugin_cards: Vec::new(),
            renaming: None,
            ai,
            conn_fields,
            title_hover: None,
            picker: None,
            model_menu: false,
            tools_menu: false,
            attach_menu: false,
            model_tab: 0,
            ai_status: None,
            ai_save_generation: 0,
            ai_save_pending: false,
            proxy_field: {
                let mut field = TextField::new(cx);
                field.set_text(proxy_text);
                field
            },
            node_version,
            maintenance_note: String::new(),
            maintenance_failed: false,
            restore_armed: false,
            changes_open: None,
            settings_tab: SettingsTab::Interface,
            mode_selected: crate::config::MODE_FULL.to_string(),
            mode_note: String::new(),
            mode_name: TextField::new(cx),
            mode_menu: false,
            instructions: TextField::new(cx),
            instructions_chat: None,
            instructions_status: None,
            color_menu: None,
            color_fields: colors_text
                .iter()
                .map(|text| {
                    let mut field = TextField::new(cx);
                    field.set_text(text.clone());
                    field
                })
                .collect(),
            conn_status,
            listing: None,
            list_attempted: std::collections::HashSet::new(),
            testing: None,
            pending: Vec::new(),
            compacting: std::collections::HashSet::new(),
            focused: None,
            content_focus: cx.focus_handle(),
            banner: None,
            log_scroll: ScrollHandle::new(),
            steps_scroll: ScrollHandle::new(),
            chat_scroll: ScrollHandle::new(),
        window_text: None,
            block_scroll: RefCell::new(std::collections::HashMap::new()),
            parsed: RefCell::new(std::collections::HashMap::new()),
            sidebar_width: sidebar_width(saved.width),
            answer_width: answer_text_width(px(INITIAL_WINDOW_WIDTH
                - sidebar_width(saved.width)
                - SPLITTER_WIDTH)),
            chat_gutter: transcript_pad(px(INITIAL_WINDOW_WIDTH
                - sidebar_width(saved.width)
                - SPLITTER_WIDTH)),
            resizing_sidebar: false,
            hovered_delete: None,
            copied: None,
            copied_code: None,
            copied_chat: false,
            window_title: String::new(),
            settings_window: None,
            selected_text: None,
            text_dragging: false,
            field_bounds: Rc::new(RefCell::new(Vec::new())),
            field_drag: None,
            caret_blink: false,
            caret_on: true,
            field_menu: None,
            scroll_drag: None,
            tool_drag: None,
            mcp_skills_drag: None,
            fitted: false,
            scale_factor: 1.,
        };
        if let Some((text, error)) = restore_notice {
            app.banner = Some(Banner { text, error });
        }
        if !app.engine_ready {
            app.banner = Some(Banner::error(format!(
                "找不到引擎：{}\n设置 DSH_FLOW_HOME 指向 dsh-flow 目录后重启界面。",
                app.engine_dir.join("src").join("cli.ts").display()
            )));
        }
        // Import the old per-conversation request history once. After this point
        // the statistics page reads only the permanent ledger, so deleting a
        // conversation cannot change past figures.
        match app.migrate_stats() {
            Ok(true) => app.stats_generation = app.stats_generation.saturating_add(1),
            Ok(false) => {}
            Err(error) => app.banner = Some(Banner::error(format!("统计迁移失败：{error}"))),
        }
        // What was typed but not sent comes back with the window.
        app.restore_draft(app.active_chat);
        // The task panel is fed by files on disk, so it polls instead of
        // listening to a tool result.
        app.spawn_tasks_pump(cx);
        app
    }

    /// Focus handle for the composer, so `main` can start with the caret there.
    pub(crate) fn composer_focus(&self) -> &FocusHandle {
        self.composer.focus_handle()
    }

    /// The manifest currently chosen in the sidebar.
    pub(crate) fn selected_entry(&self) -> Option<&WorkflowEntry> {
        self.selected_flow
            .and_then(|index| self.workflows.get(index))
    }

    /// The finished run currently shown, if any.
    pub(crate) fn selected_record(&self) -> Option<&RunRecord> {
        self.selected_run.and_then(|index| self.history.get(index))
    }

    /// The conversation currently shown.
    pub(crate) fn chat(&self) -> &Chat {
        &self.chats[self.active_chat.min(self.chats.len().saturating_sub(1))]
    }

    /// The request in flight for the conversation on screen, if any.
    pub(crate) fn active_pending(&self) -> Option<&PendingAnswer> {
        let dir = &self.chats.get(self.active_chat)?.dir;
        self.pending.iter().find(|pending| pending.dir == *dir)
    }

    /// Whether the conversation at `index` is waiting for an answer.
    pub(crate) fn chat_busy(&self, index: usize) -> bool {
        self.chats
            .get(index)
            .is_some_and(|chat| self.pending.iter().any(|pending| pending.dir == chat.dir))
    }

    /// Permission and budgets a newly created conversation starts with.
    pub(crate) fn chat_defaults(&self) -> ChatDefaults {
        ChatDefaults::default()
    }

    /// Folder a conversation keeps its files in: `workspace/<dir>`.
    pub(crate) fn chat_dir(&self, index: usize) -> PathBuf {
        let dir = self
            .chats
            .get(index)
            .map_or_else(|| "chat".to_string(), |chat| chat.dir.clone());
        self.ai.workspace_dir().join(dir)
    }

    /// The earlier turns of a conversation, for the model's context.
    ///
    /// Only what the person asked and what the assistant answered is replayed:
    /// tool rows and notes are the window's own bookkeeping, and resending them
    /// would confuse the model. The oldest turns are dropped past
    /// [`MAX_TURNS`] so a long conversation cannot grow without bound.
    pub(crate) fn chat_history(&self, index: usize) -> (Vec<Turn>, usize) {
        self.chats
            .get(index)
            .map(|chat| history_with_dropped(&chat.messages))
            .unwrap_or_default()
    }
}

mod ai_actions;
mod ai_settings;
mod chat;
mod chat_actions;
mod compact;
mod composer;
mod context_view;
mod conversations;
mod datatable;
mod fields;
mod flow;
mod input;
mod instructions;
mod mcp;
mod message;
mod metrics;
mod modes;
mod plugins;
mod render;
mod run_actions;
mod settings;
mod settings_window;
mod shortcuts;
mod sidebar;
mod skills;
mod stats;
mod steps;
mod tasks;
mod util;
mod widgets;

// Every view lives in its own file but shares this module's namespace: each submodule
// starts with `use super::*`, and these re-exports are what that glob resolves to.
#[allow(unused_imports)]
pub(crate) use {
    ai_actions::*, ai_settings::*, chat::*, chat_actions::*, compact::*, composer::*,
    context_view::*, conversations::*, datatable::*, fields::*, flow::*, input::*, instructions::*,
    mcp::*, message::*, metrics::*, modes::*, plugins::*, render::*, run_actions::*, settings::*,
    settings_window::*, sidebar::*, skills::*, stats::*, steps::*, tasks::*, util::*, widgets::*,
};

#[cfg(test)]
mod tests;

/// Build the application state; kept here so `main` stays a two-liner.
pub fn build(cx: &mut Context<DshFlowApp>) -> DshFlowApp {
    DshFlowApp::new(cx)
}
