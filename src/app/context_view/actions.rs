//! What each tool call did, and how long the rounds took.

use super::*;

/// Longest target line one action shows before it is clipped.
pub(crate) const ACTION_TEXT: usize = 120;

/// Most rows one action list shows before it says how many are left.
pub(crate) const ACTION_ROWS: usize = 8;

/// What one tool call did, grouped the way the actions card lists it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ActionKind {
    /// A file was written, edited, moved or deleted.
    File,
    /// A command, a snippet or a background task ran.
    Command,
    /// An HTTP request went out.
    Network,
    /// An MCP tool was called.
    Mcp,
    /// A plugin tool was called.
    Plugin,
    /// A tool that only read (read / list / glob / tree / grep / symbols / git).
    Other,
}

impl ActionKind {
    /// The name the chip shows.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::File => "文件",
            Self::Command => "命令",
            Self::Network => "网络",
            Self::Mcp => "MCP",
            Self::Plugin => "插件",
            Self::Other => "读取",
        }
    }
}

/// One tool call of a request, as the actions card lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RequestAction {
    /// Tool the model called.
    pub(crate) tool: String,
    /// What it acted on: a path, a command line, a URL, an MCP tool name.
    pub(crate) detail: String,
    /// False when the call failed.
    pub(crate) ok: bool,
    /// Which group it belongs to.
    pub(crate) kind: ActionKind,
}

/// Every tool call that answered the `turn`-th request, oldest first.
///
/// A request's calls are the tool rows between the answer before it and its own
/// answer, which is the same span `request_rows` counts for its `tools` figure.
pub(crate) fn request_actions(messages: &[ChatMessage], turn: u64) -> Vec<RequestAction> {
    let mut seen = 0u64;
    let mut current: Vec<&ChatMessage> = Vec::new();
    for message in messages {
        match message.role {
            Role::Tool => current.push(message),
            Role::Assistant => {
                seen += 1;
                if seen == turn {
                    return current
                        .iter()
                        .map(|message| request_action(message))
                        .collect();
                }
                current.clear();
            }
            _ => {}
        }
    }
    Vec::new()
}

/// Read one tool row into an action.
pub(crate) fn request_action(message: &ChatMessage) -> RequestAction {
    let tool = tool_of(message);
    RequestAction {
        kind: action_kind(&tool),
        detail: action_detail(&tool, &message.input, &message.text),
        ok: !message.failed,
        tool,
    }
}

/// The tool name of a row: older rows only wrote it inside the title.
pub(crate) fn tool_of(message: &ChatMessage) -> String {
    let tool = message.tool.trim();
    if !tool.is_empty() {
        return tool.to_string();
    }
    message
        .text
        .split('·')
        .next()
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// Which group a tool belongs to; a name the engine never uses is a plugin tool.
pub(crate) fn action_kind(tool: &str) -> ActionKind {
    match tool {
        "write" | "edit" | "replace" | "move" | "delete" => ActionKind::File,
        "shell" | "python" | "node" | "task" => ActionKind::Command,
        "http" | "web_search" | "read_url" => ActionKind::Network,
        "mcp" => ActionKind::Mcp,
        name if name.starts_with("mcp_") => ActionKind::Mcp,
        "read" | "list" | "glob" | "tree" | "grep" | "symbols" | "git" => ActionKind::Other,
        _ => ActionKind::Plugin,
    }
}

/// What the call acted on, preferring the arguments over the row's title.
pub(crate) fn action_detail(tool: &str, input: &str, text: &str) -> String {
    let args = serde_json::from_str::<serde_json::Value>(input).ok();
    let field = |key: &str| {
        args.as_ref()
            .and_then(|value| value.get(key))
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    let detail = match tool {
        "shell" => first_line(&field("command")),
        "python" | "node" => first_line(&field("code")),
        "http" => field("url"),
        "write" | "edit" | "delete" => field("path"),
        "replace" => match field("pattern") {
            pattern if pattern.is_empty() => field("path"),
            pattern => pattern,
        },
        "move" => match (field("from"), field("to")) {
            (from, to) if from.is_empty() && to.is_empty() => String::new(),
            (from, to) if from.is_empty() => to,
            (from, to) if to.is_empty() => from,
            (from, to) => format!("{from} → {to}"),
        },
        "mcp" => match (field("server"), field("tool")) {
            (server, tool) if server.is_empty() => tool,
            (server, tool) if tool.is_empty() => server,
            (server, tool) => format!("{server}:{tool}"),
        },
        _ => {
            // A tool row's title is `tool · target`, so the target is what the
            // card wants: a grep pattern, a plugin payload, an MCP tool name.
            let after = text
                .split_once('·')
                .map(|(_, after)| after)
                .unwrap_or_default();
            let target = after.split('·').next().unwrap_or_default().trim();
            if target.is_empty() {
                first_line(&field("tool"))
            } else {
                target.to_string()
            }
        }
    };
    clip_action(&detail, ACTION_TEXT)
}

/// The first non-empty line of a snippet.
pub(crate) fn first_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
        .to_string()
}

/// Keep one action line readable.
pub(crate) fn clip_action(text: &str, limit: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let mut clipped: String = text.chars().take(limit).collect();
    clipped.push('…');
    clipped
}

/// How one request's wall clock split, as the timing card shows it.
///
/// The engine measures the two phases it drives — the model round trips and the
/// tool runs — and reports them with the answer. The card subtracts them from
/// the request's total, which is how the rest (compaction, approvals the person
/// answered, prompt assembly, process start) gets a name instead of vanishing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RequestTiming {
    /// Wall clock spent waiting for model round trips.
    pub(crate) model_ms: u64,
    /// Wall clock spent running tools.
    pub(crate) tool_ms: u64,
    /// Time neither bucket claimed.
    pub(crate) other_ms: u64,
    /// The whole request, never smaller than what was measured.
    pub(crate) total_ms: u64,
}

/// Split one request's time; `None` when the engine measured nothing, so an
/// older save shows no card rather than a made-up `0% / 0% / 100%` bar.
pub(crate) fn request_timing(
    model_ms: u64,
    tool_ms: u64,
    elapsed_ms: u64,
) -> Option<RequestTiming> {
    if model_ms == 0 && tool_ms == 0 {
        return None;
    }
    let measured = model_ms.saturating_add(tool_ms);
    // A measured phase can outlast the answer's own stopwatch when the two
    // clocks start at different moments; the bar follows the sum it can prove.
    let total = elapsed_ms.max(measured).max(1);
    Some(RequestTiming {
        model_ms,
        tool_ms,
        other_ms: total.saturating_sub(measured),
        total_ms: total,
    })
}

/// One row of the timing card: a bucket, its colour, and its share of the total.
pub(crate) fn timing_rows(
    timing: RequestTiming,
    palette: Palette,
) -> [(&'static str, u64, u32); 3] {
    [
        ("模型", timing.model_ms, palette.accent),
        ("工具", timing.tool_ms, palette.ok),
        ("其他", timing.other_ms, palette.skipped),
    ]
}
