//! Small pure helpers: time, sizes, titles, and text cleaning.

use super::*;

/// Milliseconds since the Unix epoch for a monotonic instant.
///
/// Saved runs need wall-clock times, while the running code measures with
/// `Instant`; both are kept because only the latter survives a clock change.
pub(crate) fn epoch_ms(instant: Instant) -> u64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64);
    now.saturating_sub(instant.elapsed().as_millis() as u64)
}

/// Milliseconds since the Unix epoch, for something happening now.
pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
}

/// "刚刚" / "3 分钟前" for a timestamp in the past.
pub(crate) fn ago(finished_ms: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(finished_ms, |since| since.as_millis() as u64);
    elapsed_label(now.saturating_sub(finished_ms) / 1000)
}

/// The words for an age in seconds.
///
/// Split from [`ago`] so the wording can be checked without waiting for a clock.
pub(crate) fn elapsed_label(seconds: u64) -> String {
    match seconds {
        0..=4 => "刚刚".to_string(),
        5..=59 => format!("{seconds} 秒前"),
        60..=3599 => format!("{} 分钟前", seconds / 60),
        3600..=86_399 => format!("{} 小时前", seconds / 3600),
        _ => format!("{} 天前", seconds / 86_400),
    }
}

/// How a file's size reads in a chip: B / KB / MB, one decimal past a KB.
pub(crate) fn short_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.;
    let value = bytes as f64;
    if value < KB {
        format!("{bytes} B")
    } else if value < KB * KB {
        format!("{:.1} KB", value / KB)
    } else {
        format!("{:.1} MB", value / (KB * KB))
    }
}

/// Read one dropped file as an attachment.
///
/// Images and PDFs are recognised by their magic bytes and travel to the model
/// as bytes; their `text` is a one-line summary so the transcript still says
/// what was handed over. Everything else must be UTF-8 text: binary files are
/// refused by their NUL bytes and invalid encoding, so the model gets text or
/// nothing, not mojibake. Long text keeps its head and says it was cut, so the
/// person can tell why the answer stops.
pub(crate) fn read_attachment(path: &Path) -> Result<Attachment, String> {
    let meta =
        std::fs::metadata(path).map_err(|error| format!("读不到 {}：{error}", path.display()))?;
    if !meta.is_file() {
        return Err(format!("{} 不是文件", path.display()));
    }
    if meta.len() > MAX_ATTACHMENT_BYTES {
        return Err(format!(
            "{} 太大（{}），单个附件上限 {}",
            path.display(),
            short_bytes(meta.len()),
            short_bytes(MAX_ATTACHMENT_BYTES)
        ));
    }
    let raw = std::fs::read(path).map_err(|error| format!("读不到 {}：{error}", path.display()))?;
    let name = path.display().to_string();
    if let Some((kind, mime)) = binary_kind(&raw) {
        let text = format!(
            "{} {}（{}）。发送时以原始文件交给模型；后续轮次只保留这条说明。",
            kind.label(),
            name,
            short_bytes(meta.len())
        );
        return Ok(Attachment {
            name,
            kind,
            text,
            bytes: raw,
            mime: mime.to_string(),
            truncated: false,
        });
    }
    if raw.iter().take(8_192).any(|byte| *byte == 0) {
        return Err(format!(
            "{} 看起来不是文本文件（图片 / PDF 之外的类型暂不支持）",
            path.display()
        ));
    }
    let text = String::from_utf8(raw)
        .map_err(|_| format!("{} 不是 UTF-8 文本，暂不支持作为附件", path.display()))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    if text.trim().is_empty() {
        return Err(format!("{} 是空文件", path.display()));
    }
    let truncated = text.chars().count() > MAX_ATTACHMENT_CHARS;
    let kept: String = text.chars().take(MAX_ATTACHMENT_CHARS).collect();
    Ok(Attachment {
        name,
        kind: AttachmentKind::Text,
        text: kept,
        bytes: Vec::new(),
        mime: String::new(),
        truncated,
    })
}

/// An attachment made from an image on the clipboard.
pub(crate) fn clipboard_attachment(bytes: Vec<u8>) -> Attachment {
    Attachment {
        name: format!("剪贴板图片-{}.png", now_ms()),
        kind: AttachmentKind::Image,
        text: format!(
            "剪贴板图片（{}）。发送时以原始图片交给模型；后续轮次只保留这条说明。",
            short_bytes(bytes.len() as u64)
        ),
        bytes,
        mime: "image/png".to_string(),
        truncated: false,
    }
}

/// What a set of attachments would put in one request, in bytes.
///
/// Images and PDFs carry their bytes; text files carry their text. Only the
/// larger of the two counts, because a binary's `text` is just a note.
pub(crate) fn attached_bytes(attachments: &[Attachment]) -> usize {
    attachments
        .iter()
        .map(|item| item.bytes.len().max(item.text.len()))
        .sum()
}

/// Recognise the image and PDF formats providers accept, by magic bytes.
fn binary_kind(bytes: &[u8]) -> Option<(AttachmentKind, &'static str)> {
    let image = |mime| Some((AttachmentKind::Image, mime));
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        return image("image/png");
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return image("image/jpeg");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return image("image/gif");
    }
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return image("image/webp");
    }
    if bytes.starts_with(b"BM") {
        return image("image/bmp");
    }
    if bytes.starts_with(b"%PDF-") {
        return Some((AttachmentKind::Pdf, "application/pdf"));
    }
    None
}

/// How an attached file reaches the model: its name, then its text.
pub(crate) fn attachment_turn(name: &str, text: &str) -> String {
    format!("附件 {name} 的内容：\n{text}")
}

/// One conversation as Markdown, for an export.
///
/// Tool rows and notes are kept as short as the transcript shows them; approval
/// cards keep their diff, because that is what a reader would want later.
pub(crate) fn transcript_markdown(chat: &Chat) -> String {
    let mut out = format!("# {}\n\n", chat.title.trim());
    for message in &chat.messages {
        match message.role {
            Role::User => {
                out.push_str("## 你\n\n");
                out.push_str(message.text.trim());
                out.push_str("\n\n");
            }
            Role::Assistant => {
                out.push_str("## 助手");
                if !message.model.trim().is_empty() {
                    out.push_str(&format!("（{}）", message.model.trim()));
                }
                out.push_str("\n\n");
                out.push_str(message.text.trim());
                out.push_str("\n\n");
            }
            Role::Tool => {
                out.push_str(&format!("- 工具：{}\n\n", one_line(&message.text)));
            }
            Role::Attachment => {
                out.push_str(&format!("> 附件：{}\n\n", message.path.trim()));
            }
            Role::Approval => {
                out.push_str(&format!("> 确认：{}\n\n", one_line(&message.text)));
                if !message.input.trim().is_empty() {
                    out.push_str("```diff\n");
                    out.push_str(message.input.trim());
                    out.push_str("\n```\n\n");
                }
            }
            Role::Note => {
                out.push_str(&format!("> {}\n\n", message.text.trim()));
            }
            Role::Question => {
                let state = if message.output.trim().is_empty() {
                    "未决"
                } else {
                    message.output.trim()
                };
                out.push_str(&format!(
                    "> 提问 · {}（{}）\n\n",
                    one_line(&message.text),
                    state
                ));
            }
            Role::Plan => {
                let state = if message.output.trim().is_empty() {
                    "未决"
                } else {
                    message.output.trim()
                };
                out.push_str(&format!(
                    "# 计划（{}）\n\n{}\n\n",
                    state,
                    message.text.trim()
                ));
            }
            // Thinking is not part of the conversation the model reads back.
            Role::Thinking => {}
        }
    }
    out
}

/// One line of text, for a list row.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// What the next question would carry: turns, characters, estimated tokens and
/// how many turns are already past the limits.
pub(crate) fn history_stats(messages: &[ChatMessage]) -> (usize, usize, usize, usize) {
    let (turns, dropped) = history_with_dropped(messages);
    let chars: usize = turns.iter().map(|turn| turn.text.chars().count()).sum();
    let tokens: usize = turns.iter().map(|turn| estimate_tokens(&turn.text)).sum();
    (turns.len(), chars, tokens, dropped)
}

/// Rough token count for text, used for the history budget and the readout.
///
/// Providers do not expose their tokenizer, and bundling one would be a large
/// dependency for a number that only chooses a budget. CJK and other wide
/// characters count as one token each; ASCII counts as one per four characters,
/// which tracks common BPE vocabularies closely enough for trimming.
pub(crate) fn estimate_tokens(text: &str) -> usize {
    let mut tokens = 0usize;
    let mut ascii = 0usize;
    for ch in text.chars() {
        if ch.is_ascii() {
            ascii += 1;
        } else {
            tokens += ascii.div_ceil(4);
            ascii = 0;
            tokens += 1;
        }
    }
    tokens + ascii.div_ceil(4)
}

/// Byte ranges of `needle` in `text`, case-insensitive and non-overlapping.
///
/// The needle is expected lower-cased already. Ranges always sit on character
/// boundaries, so the shaping the highlight is painted against can map them.
pub(crate) fn find_ranges(text: &str, needle: &str, limit: usize) -> Vec<Range<usize>> {
    let wanted: Vec<char> = needle.chars().flat_map(char::to_lowercase).collect();
    let mut ranges = Vec::new();
    if wanted.is_empty() || limit == 0 {
        return ranges;
    }
    let mut start = 0;
    while start < text.len() && ranges.len() < limit {
        if let Some(len) = match_ci(&text[start..], &wanted) {
            ranges.push(start..start + len);
            start += len;
        } else {
            start += text[start..].chars().next().map_or(1, char::len_utf8);
        }
    }
    ranges
}

/// How many bytes the lower-cased needle covers at the start of `hay`, if it
/// matches there.
fn match_ci(hay: &str, wanted: &[char]) -> Option<usize> {
    let mut len = 0;
    let mut chars = hay.chars();
    for want in wanted {
        let found = chars.next()?;
        let mut lower = found.to_lowercase();
        if lower.next()? != *want || lower.next().is_some() {
            return None;
        }
        len += found.len_utf8();
    }
    Some(len)
}

/// The first transcript message drawn for a conversation.
///
/// Long conversations only paint the newest [`CHAT_WINDOW`] entries; an explicit
/// start (the reader asked for more) wins and is clamped to the list.
pub(crate) fn window_start(explicit: Option<usize>, total: usize) -> usize {
    explicit
        .unwrap_or_else(|| total.saturating_sub(CHAT_WINDOW))
        .min(total)
}

/// The same window, narrowed until the newest messages hold at most `budget`
/// markdown blocks.
///
/// `block_counts` are the blocks of the last [`CHAT_WINDOW`] messages, oldest
/// first. The newest `min_messages` are always kept; an explicit start is left
/// alone, because hiding a message the reader just asked to see would be worse
/// than the layout it costs.
pub(crate) fn window_start_with_budget(
    block_counts: &[usize],
    explicit: Option<usize>,
    budget: usize,
    min_messages: usize,
) -> usize {
    let total = block_counts.len();
    if let Some(start) = explicit {
        return start.min(total);
    }
    let mut start = 0;
    let mut blocks = 0;
    for index in (0..total).rev() {
        blocks += block_counts[index];
        if blocks > budget && total - index > min_messages {
            start = index + 1;
            break;
        }
        start = index;
    }
    start
}

/// A file name for an exported conversation.
///
/// Windows refuses `<>:"/\|?*` and control characters in a name; those become
/// `_`, dots at the edges go away, and a blank title falls back to a plain name
/// so the export always has somewhere to go.
pub(crate) fn export_name(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|character| {
            if character.is_control() || "<>:\"/\\|?*".contains(character) {
                '_'
            } else {
                character
            }
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.').trim();
    let name: String = trimmed.chars().take(40).collect();
    if name.is_empty() {
        "对话记录".to_string()
    } else {
        name
    }
}

/// Clipboard text as a field can hold it.
///
/// Line breaks are normalised to `\n`; they are kept only for the composer, where
/// a pasted snippet may span lines, and turned into spaces everywhere else.
pub(crate) fn clean_paste(raw: &str, keep_lines: bool) -> String {
    let normalised = raw.replace("\r\n", "\n").replace('\r', "\n");
    normalised
        .chars()
        .filter_map(|character| match character {
            '\n' if keep_lines => Some('\n'),
            '\n' => Some(' '),
            control if control.is_control() && control != '\t' => None,
            other => Some(other),
        })
        .collect()
}

/// Whether a conversation matches a sidebar search.
///
/// The needle is already lower-cased; the title and every message body are
/// searched, so a chat can be found by something the assistant said.
pub(crate) fn chat_matches(chat: &Chat, needle: &str) -> bool {
    chat.title.to_lowercase().contains(needle)
        || chat
            .messages
            .iter()
            .any(|message| message.text.to_lowercase().contains(needle))
}

/// The first message of `chat` that matches `needle`, with a one-line snippet.
///
/// `needle` is already lower-cased. The snippet is the text around the hit on
/// one line, so a card says *what* matched instead of only that something did;
/// the sidebar opens the conversation at this message.
pub(crate) fn chat_match(chat: &Chat, needle: &str) -> Option<(usize, String)> {
    if needle.is_empty() {
        return None;
    }
    for (index, message) in chat.messages.iter().enumerate() {
        let lowered = message.text.to_lowercase();
        let Some(at) = lowered.find(needle) else {
            continue;
        };
        // Lower-casing can change the byte layout; when it does, the snippet is
        // taken from the lowered text rather than slicing the original wrongly.
        let source = if lowered.len() == message.text.len() {
            message.text.as_str()
        } else {
            lowered.as_str()
        };
        let mut start = at.saturating_sub(36);
        while start > 0 && !source.is_char_boundary(start) {
            start -= 1;
        }
        let mut end = (at + needle.len() + 52).min(source.len());
        while end < source.len() && !source.is_char_boundary(end) {
            end += 1;
        }
        let flat = source[start..end]
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let snippet = format!(
            "{}{flat}{}",
            if start > 0 { "…" } else { "" },
            if end < source.len() { "…" } else { "" }
        );
        return Some((index, snippet));
    }
    None
}

/// The turns of a conversation the model needs, oldest first.///
/// Tool rows and notes are the window's own bookkeeping, so they are left out;
/// only questions, answers and attached files go back. Blank text is skipped and
/// the oldest turns are dropped past [`MAX_TURNS`].
///
/// The window itself needs the dropped count too, so this stays a test helper.
#[cfg(test)]
pub(crate) fn history_turns(messages: &[ChatMessage]) -> Vec<Turn> {
    history_with_dropped(messages).0
}

/// The same history, plus how many turns were left out of it.
///
/// The count lets the window say when an answer came from a cut-down context
/// instead of quietly pretending the whole conversation was sent.
pub(crate) fn history_with_dropped(messages: &[ChatMessage]) -> (Vec<Turn>, usize) {
    let (dropped, kept) = split_history(messages);
    (kept, dropped.len())
}

/// Where a compaction checkpoint is kept inside a conversation folder.
///
/// The checkpoint is the last compaction layer's output: the oldest turns the
/// window stopped replaying, condensed by the model into a fixed-structure
/// summary. It is a file beside `chat.json` rather than a message, so the
/// transcript still reads as the person wrote it while the model still gets the
/// background.
pub(crate) const COMPACT_FILE: &str = "compact.json";

/// Oldest turns the LLM fallback condenses in one checkpoint.
pub(crate) const COMPACT_SPAN_TURNS: usize = 30;

/// Rough token budget for the span handed to the model.
pub(crate) const COMPACT_INPUT_TOKENS: usize = 24_000;

/// Longest checklist handed back to the model with every later question.
pub(crate) const COMPACT_NOTE_TOKENS: usize = 2_500;

/// A condensed earlier span, as `compact.json` stores it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Checkpoint {
    /// The condensed text itself.
    pub(crate) summary: String,
    /// How many replayed turns it replaced.
    pub(crate) turns: usize,
    /// Model that wrote it; empty when it came from the deterministic layer.
    pub(crate) model: String,
    /// When it was made, in epoch milliseconds.
    pub(crate) at_ms: u64,
    /// Fingerprint of the span it condensed, so a re-run is not attempted on
    /// the same content twice.
    pub(crate) span: u64,
}

impl Checkpoint {
    /// True when there is something worth injecting into the prompt.
    pub(crate) fn usable(&self) -> bool {
        !self.summary.trim().is_empty()
    }
}

/// Read a conversation's checkpoint, if it has one.
///
/// A missing or broken file simply means "no checkpoint yet": the window then
/// falls back to the deterministic recap, so a damaged file never blocks a
/// question.
pub(crate) fn read_checkpoint(dir: &std::path::Path) -> Option<Checkpoint> {
    let text = std::fs::read_to_string(dir.join(COMPACT_FILE)).ok()?;
    let value = serde_json::from_str::<serde_json::Value>(&text).ok()?;
    let summary = value.get("summary").and_then(|item| item.as_str())?;
    let checkpoint = Checkpoint {
        summary: summary.trim().to_string(),
        turns: value
            .get("turns")
            .and_then(|item| item.as_u64())
            .unwrap_or(0) as usize,
        model: value
            .get("model")
            .and_then(|item| item.as_str())
            .unwrap_or_default()
            .to_string(),
        at_ms: value
            .get("atMs")
            .and_then(|item| item.as_u64())
            .unwrap_or(0),
        span: value
            .get("span")
            .and_then(|item| item.as_u64())
            .unwrap_or(0),
    };
    checkpoint.usable().then_some(checkpoint)
}

/// Write a conversation's checkpoint, replacing any earlier one.
pub(crate) fn write_checkpoint(
    dir: &std::path::Path,
    checkpoint: &Checkpoint,
) -> Result<(), String> {
    let body = serde_json::json!({
        "summary": checkpoint.summary,
        "turns": checkpoint.turns,
        "model": checkpoint.model,
        "atMs": checkpoint.at_ms,
        "span": checkpoint.span,
    })
    .to_string();
    std::fs::create_dir_all(dir).map_err(|error| format!("无法创建 {}：{error}", dir.display()))?;
    crate::store::write_atomic(&dir.join(COMPACT_FILE), &body)
        .map_err(|error| format!("无法保存压缩摘要：{error}"))
}

/// The checkpoint as the prompt sees it.
///
/// The blocks are labelled so the model can tell a condensed span from a
/// message the person actually wrote, and the tail is cut at the note budget so
/// a runaway summary cannot itself fill the window.
pub(crate) fn checkpoint_prompt(checkpoint: &Checkpoint) -> String {
    let mut body = checkpoint.summary.trim().to_string();
    // Cut by estimated tokens, not characters: a CJK summary costs one token a
    // character, so a character cap would let ten times the budget through.
    if estimate_tokens(&body) > COMPACT_NOTE_TOKENS {
        let room = COMPACT_NOTE_TOKENS.saturating_sub(40);
        body = clip_turn(&body, room);
    }
    let source = if checkpoint.model.is_empty() {
        String::new()
    } else {
        format!("（由 {} 压缩）", checkpoint.model)
    };
    format!(
        "早前对话已压缩成下面这份检查点{source}；把它当作已经确认的背景，不要复述，直接从后面的消息继续：\n{body}"
    )
}

/// The instruction handed to the model when the window condenses a span.
///
/// Mirrors the engine's own compaction directive, so a checkpoint made here and
/// one made inside a round read the same way.
pub(crate) const COMPACT_INSTRUCTION: &str = "你现在是这个助理的压缩引擎。把上面的对话压成一份结构化检查点，\
让另一个模型在不丢失关键上下文的情况下接着做事。\n\
严格按下面的小节输出，一个都不能少，顺序不能变；用简短条目，空的小节写「(无)」。\n\
## 主要请求与意图\n## 关键技术概念\n## 文件与代码\n## 错误与修复\n\
## 待办事项\n## 当前进展\n## 下一步\n## 关键上下文\n\
规则：路径、命令、报错原文、标识符、数值、函数签名原样保留；忠实记录用户的纠正；\
不要提到这次压缩；只输出检查点正文、不要调用工具；用对话本身的语言回答。";

/// Which turns the last compaction layer should condense.
///
/// The span is the turns the history window already stopped replaying: they are
/// the ones whose detail is otherwise lost. A checkpoint that already covers
/// them is reused rather than rebuilt, so a repeated question costs no extra
/// call.
/// @param messages - the transcript.
/// @param checkpoint - the checkpoint already on disk, if any.
/// @returns the span to condense, or `None` when there is nothing new.
pub(crate) fn compaction_span(
    messages: &[ChatMessage],
    checkpoint: Option<&Checkpoint>,
) -> Option<(Vec<Turn>, u64)> {
    let (dropped, _) = split_history(messages);
    if dropped.is_empty() {
        return None;
    }
    let span = dropped.len().min(COMPACT_SPAN_TURNS);
    let picked: Vec<Turn> = dropped[dropped.len() - span..].to_vec();
    let mut tokens = 0usize;
    let mut keep = 0usize;
    // Take the newest turns of the span first: they are the ones the next
    // question is most likely to depend on, and the budget has to stop somewhere.
    for turn in picked.iter().rev() {
        tokens += estimate_tokens(&turn.text);
        if tokens > COMPACT_INPUT_TOKENS && keep > 0 {
            break;
        }
        keep += 1;
    }
    let picked = picked[picked.len() - keep..].to_vec();
    let fingerprint = span_fingerprint(&picked);
    if checkpoint.is_some_and(|entry| entry.span == fingerprint && entry.usable()) {
        return None;
    }
    Some((picked, fingerprint))
}

/// A stable number for a span, so the same content is not summarized twice.
fn span_fingerprint(turns: &[Turn]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for turn in turns {
        for byte in turn
            .speaker
            .role()
            .bytes()
            .chain(turn.text.as_bytes().iter().copied())
        {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x100_0000_01b3);
        }
    }
    hash
}

/// The replayable turns of a conversation, split into (dropped, kept).
fn split_history(messages: &[ChatMessage]) -> (Vec<Turn>, Vec<Turn>) {
    let mut turns: Vec<Turn> = messages
        .iter()
        .filter_map(|message| {
            let speaker = match message.role {
                Role::User => Speaker::User,
                Role::Assistant => Speaker::Assistant,
                Role::Attachment => Speaker::User,
                Role::Tool | Role::Note | Role::Approval | Role::Thinking => return None,
                Role::Question | Role::Plan => return None,
            };
            let text = match message.role {
                Role::Attachment => attachment_turn(&message.path, &message.text),
                _ => message.text.clone(),
            };
            let server_tool_calls = if message.role == Role::Assistant {
                message.server_tool_calls.clone()
            } else {
                Vec::new()
            };
            (!text.trim().is_empty()).then_some(Turn {
                speaker,
                text,
                server_tool_calls,
            })
        })
        .collect();
    let mut start = turns.len().saturating_sub(MAX_TURNS);
    // The count alone can still be enormous: a few long answers are enough to
    // fill the window, so drop whole oldest turns until the estimate fits.
    let mut tokens: usize = turns[start..]
        .iter()
        .map(|turn| estimate_tokens(&turn.text))
        .sum();
    while tokens > MAX_HISTORY_TOKENS && start + 1 < turns.len() {
        tokens -= estimate_tokens(&turns[start].text);
        start += 1;
    }
    let kept: Vec<Turn> = turns
        .split_off(start)
        .into_iter()
        .map(|turn| Turn {
            speaker: turn.speaker,
            text: clip_turn(&turn.text, MAX_HISTORY_TURN_TOKENS),
            server_tool_calls: turn.server_tool_calls,
        })
        .collect();
    (turns, kept)
}

/// Longest one turn may contribute after compression.
pub(crate) const MAX_HISTORY_TURN_TOKENS: usize = 12_000;

/// Keep both ends of a turn that alone would exceed the token budget.
///
/// Dropping such a turn would take the question (or the conclusion) with it, so
/// the middle is cut instead and the model is told why.
fn clip_turn(text: &str, limit: usize) -> String {
    if estimate_tokens(text) <= limit {
        return text.to_string();
    }
    // Leave room for the marker itself.
    let budget = limit.saturating_sub(48);
    let head = take_tokens(text, budget / 2, false);
    let tail = take_tokens(text, budget - budget / 2, true);
    let omitted = text
        .chars()
        .count()
        .saturating_sub(head.chars().count() + tail.chars().count());
    format!("{head}\n…（中间省略 {omitted} 字符，避免单条历史占满上下文）…\n{tail}")
}

/// Take roughly `limit` tokens from one end of `text`.
///
/// The same wide-character rule as [`estimate_tokens`] is used, one character
/// at a time: an exact head/tail split does not need the ASCII run grouping.
fn take_tokens(text: &str, limit: usize, from_end: bool) -> String {
    let mut used = 0.0f64;
    let mut taken: Vec<char> = Vec::new();
    let chars: Box<dyn Iterator<Item = char>> = if from_end {
        Box::new(text.chars().rev())
    } else {
        Box::new(text.chars())
    };
    for ch in chars {
        used += if ch.is_ascii() { 0.25 } else { 1.0 };
        if used > limit as f64 {
            break;
        }
        taken.push(ch);
    }
    if from_end {
        taken.reverse();
    }
    taken.into_iter().collect()
}

/// How many omitted turns the recap names.
const HISTORY_RECAP_TURNS: usize = 8;

/// Longest recap handed to the model when the oldest turns were dropped.
pub(crate) const HISTORY_RECAP_TOKENS: usize = 800;

/// Longest first line kept from one omitted turn.
const HISTORY_RECAP_LINE: usize = 120;

/// A compact memory of the turns that no longer fit in the history window.
///
/// Without it a long conversation silently forgets why it started; the recap
/// keeps the opening request (and the first few follow-ups) in the system
/// prompt, so the model can still connect a later question to the task.
pub(crate) fn history_recap(messages: &[ChatMessage]) -> String {
    let (dropped, _) = split_history(messages);
    if dropped.is_empty() {
        return String::new();
    }
    let mut entries: Vec<String> = Vec::new();
    let mut used = 0usize;
    for turn in dropped.iter().take(HISTORY_RECAP_TURNS) {
        let line = turn
            .text
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("")
            .trim();
        if line.is_empty() {
            continue;
        }
        let clipped: String = line.chars().take(HISTORY_RECAP_LINE).collect();
        let suffix = if line.chars().count() > HISTORY_RECAP_LINE {
            "…"
        } else {
            ""
        };
        let who = if turn.speaker == Speaker::User {
            "你"
        } else {
            "助手"
        };
        let entry = format!("- {who}：{clipped}{suffix}\n");
        let cost = estimate_tokens(&entry);
        if used + cost > HISTORY_RECAP_TOKENS {
            break;
        }
        used += cost;
        entries.push(entry);
    }
    if entries.is_empty() {
        return String::new();
    }
    format!(
        "早前对话摘要（最早的 {} 条已从上下文省略；需要细节时请用户重述或重新读取文件）：\n{}",
        dropped.len(),
        entries.concat()
    )
}

/// The platform name the model should assume when it writes commands.
pub(crate) fn platform_label() -> &'static str {
    match std::env::consts::OS {
        "windows" => "Windows",
        "macos" => "macOS",
        "linux" => "Linux",
        other => other,
    }
}

/// Today's date in UTC, as `YYYY-MM-DD`.
///
/// Rust's standard library has no local time zone, so the date is stated as UTC
/// rather than silently pretending to be local.
pub(crate) fn today_iso() -> String {
    let days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| (since.as_secs() / 86_400) as i64);
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}")
}

/// The Gregorian date for days since 1970-01-01.
///
/// Howard Hinnant's `civil_from_days`, which needs no table and no crate.
pub(crate) fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    (year + i64::from(month <= 2), month as u32, day as u32)
}

/// Folders a workspace map never descends into: generated or dependency trees.
const MAP_SKIPPED: [&str; 12] = [
    ".git",
    "node_modules",
    "target",
    "dist",
    ".dsh-flow",
    "vendor",
    ".venv",
    "venv",
    "__pycache__",
    ".idea",
    ".next",
    "out",
];

/// A compact map of the workspace, handed to the model with every question.
///
/// The assistant would otherwise start every conversation by calling `tree`,
/// which costs a round-trip and often lands on its own chat folder. This is a
/// deliberately shallow, bounded listing: enough to know where to look, never
/// a substitute for reading the files.
/// @param root - the configured workspace.
/// @param exclude - the conversation's own folder, which is noise in a map.
/// @param depth - levels to list; 2 means root entries and their children.
/// @param max_entries - hard cap on listed entries.
/// @param max_chars - hard cap on the whole text, header included.
/// @returns the map, or an empty string when there is nothing to show.
pub(crate) fn workspace_map(
    root: &Path,
    exclude: Option<&Path>,
    depth: usize,
    max_entries: usize,
    max_chars: usize,
) -> String {
    // The caller passes absolute paths; comparing relative ones avoids a
    // canonicalize call for every entry in the walk.
    let excluded = exclude
        .and_then(|path| path.strip_prefix(root).ok())
        .map(Path::to_path_buf);
    let mut lines: Vec<String> = Vec::new();
    let mut used = 0usize;
    let mut truncated = false;
    walk_map(
        root,
        Path::new(""),
        0,
        depth.max(1),
        max_entries.max(1),
        max_chars,
        excluded.as_deref(),
        &mut lines,
        &mut used,
        &mut truncated,
    );
    if lines.is_empty() {
        return String::new();
    }
    let mut text = format!(
        "工作区结构（自动生成，仅用于定位；完整内容用 tree / glob / grep 查看）：\n{}",
        root.display()
    );
    for line in lines {
        text.push('\n');
        text.push_str(&line);
    }
    if truncated {
        text.push_str("\n…（已省略部分条目，用 tree 查看完整结构）");
    }
    text
}

/// Whether a top-level folder is one of this app's `chat-N` transcript folders.
fn is_chat_folder(name: &str) -> bool {
    name.strip_prefix("chat-").is_some_and(|number| {
        !number.is_empty() && number.chars().all(|char| char.is_ascii_digit())
    })
}

/// One depth-first step of [`workspace_map`].
#[allow(clippy::too_many_arguments)]
fn walk_map(
    root: &Path,
    relative: &Path,
    level: usize,
    depth: usize,
    max_entries: usize,
    max_chars: usize,
    excluded: Option<&Path>,
    lines: &mut Vec<String>,
    used: &mut usize,
    truncated: &mut bool,
) {
    let dir = if relative.as_os_str().is_empty() {
        root.to_path_buf()
    } else {
        root.join(relative)
    };
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    // One directory can hold hundreds of thousands of entries; collect a
    // bounded sample instead of the whole listing just to sort it.
    let cap = max_entries.saturating_sub(*used).saturating_mul(8).max(64);
    let mut entries: Vec<std::fs::DirEntry> = read.flatten().take(cap + 1).collect();
    if entries.len() > cap {
        entries.truncate(cap);
        *truncated = true;
    }
    entries.sort_by(|a, b| {
        let kind = b
            .file_type()
            .map(|kind| kind.is_dir())
            .unwrap_or(false)
            .cmp(&a.file_type().map(|kind| kind.is_dir()).unwrap_or(false));
        kind.then_with(|| a.file_name().cmp(&b.file_name()))
    });
    for entry in entries {
        if *used >= max_entries {
            *truncated = true;
            return;
        }
        let file_name = entry.file_name();
        let shown = file_name.to_string_lossy();
        let is_dir = entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false);
        // `chat-N` folders are this app's own transcripts. When the workspace
        // is a real project they would otherwise be the first thing the model
        // sees, and the project would look like a list of chats.
        let own_chat = level == 0 && is_chat_folder(&shown);
        if is_dir && (MAP_SKIPPED.contains(&shown.as_ref()) || own_chat) {
            continue;
        }
        let child = if relative.as_os_str().is_empty() {
            PathBuf::from(&file_name)
        } else {
            relative.join(&file_name)
        };
        if excluded.is_some_and(|skip| child == skip) {
            continue;
        }
        let line = format!(
            "{}{}{}",
            "  ".repeat(level),
            shown,
            if is_dir { "/" } else { "" }
        );
        if *used + line.chars().count() + 1 > max_chars {
            *truncated = true;
            return;
        }
        *used += line.chars().count() + 1;
        lines.push(line);
        if is_dir && level + 1 < depth {
            walk_map(
                root,
                &child,
                level + 1,
                depth,
                max_entries,
                max_chars,
                excluded,
                lines,
                used,
                truncated,
            );
        }
    }
}

/// What the model should know about where it is running.
pub(crate) fn environment_prompt(workspace: &Path, model: &str) -> String {
    let model = model.trim();
    let model = if model.is_empty() {
        "未知".to_string()
    } else {
        model.to_string()
    };
    format!(
        "当前环境：{}；UTC 日期：{}；模型：{}；对话目录：{}。",
        platform_label(),
        today_iso(),
        model,
        workspace.display()
    )
}

/// The command line inside a shell approval's diff.
///
/// The diff is `$ <command>`, optionally after a `# 工作目录：…` line. A command
/// may itself span lines, so everything from the `$ ` line onward belongs to it.
pub(crate) fn command_from_approval(diff: &str) -> String {
    let mut lines = diff.lines();
    while let Some(line) = lines.next() {
        let Some(first) = line.strip_prefix("$ ") else {
            continue;
        };
        let mut command = first.to_string();
        for rest in lines {
            command.push('\n');
            command.push_str(rest);
        }
        return command.trim().to_string();
    }
    String::new()
}

/// Files read as project instructions, in preference order.
const PROJECT_RULE_FILES: [&str; 2] = ["AGENTS.md", ".github/copilot-instructions.md"];

/// Text the engine adds when a card shows only the head of a long snippet.
const CLIPPED_CARD_MARK: &str = "确认卡只显示前";

/// Whether a card may offer "本次会话都允许".
///
/// Only the exact command or snippet is remembered, so a card whose text was
/// clipped cannot name what it would be remembering.
pub(crate) fn allow_session_button(tool: &str, input: &str) -> bool {
    matches!(tool, "shell" | "python" | "node") && !input.contains(CLIPPED_CARD_MARK)
}

/// The key remembered by "本次会话都允许": the tool plus its exact command or snippet.
///
/// The tool is part of the key so a code snippet can never approve a command
/// that happens to start with the same text.
pub(crate) fn session_key(tool: &str, command: &str) -> String {
    format!("{tool}\u{1f}{command}")
}

/// Longest project instruction text handed to the model.
pub(crate) const PROJECT_RULES_MAX: usize = 20_000;

/// Project instructions from the workspace or this conversation's folder.
///
/// The first file that exists wins, so a per-conversation `AGENTS.md` can
/// override the workspace one. Empty files are skipped.
/// @param workspace - the configured workspace root.
/// @param chat - the conversation's own folder (inside the workspace).
/// @returns the formatted instructions, or an empty string.
pub(crate) fn project_rules(workspace: &Path, chat: &Path) -> String {
    // The conversation's own file wins, so one chat can carry extra rules.
    for root in [chat, workspace] {
        for name in PROJECT_RULE_FILES {
            let file = root.join(name);
            let Ok(text) = std::fs::read_to_string(&file) else {
                continue;
            };
            let text = text.strip_prefix('\u{feff}').unwrap_or(&text).trim();
            if text.is_empty() {
                continue;
            }
            let mut kept: String = text.chars().take(PROJECT_RULES_MAX).collect();
            if text.chars().count() > PROJECT_RULES_MAX {
                kept.push_str("\n…（已截断）");
            }
            return format!(
                "项目指令（来自 {name}；只作约定参考，安全边界仍以工具权限为准）：\n{kept}"
            );
        }
    }
    String::new()
}

/// How many tool rows one recap keeps.
const RECAP_STEPS: usize = 16;

/// Longest recap handed to the model with the next question.
pub(crate) const RECAP_CHARS: usize = 20_000;

/// Longest output kept from one tool row in the recap.
const RECAP_STEP_CHARS: usize = 2_000;

/// What the tools found in the turn before this question.
///
/// History only carries questions and answers, so a follow-up would otherwise
/// forget every file the last turn read. The recap is appended to the system
/// prompt, never stored in the transcript, and says the files may have changed
/// so the model re-reads rather than trusting stale text.
pub(crate) fn tool_recap(messages: &[ChatMessage]) -> String {
    let mut picks: Vec<&ChatMessage> = Vec::new();
    for message in messages.iter().rev() {
        match message.role {
            Role::Tool => {
                if picks.len() < RECAP_STEPS {
                    picks.push(message);
                }
            }
            // The question that started the turn is where the recap stops.
            Role::User | Role::Attachment => break,
            Role::Assistant | Role::Note | Role::Approval | Role::Thinking => {}
            Role::Question | Role::Plan => {}
        }
    }
    // Newest first while filling the budget, then back to reading order: the
    // most recent evidence matters more than the start of the turn.
    let mut entries: Vec<String> = Vec::new();
    let mut used = 0;
    for message in picks {
        let output = message.output.trim();
        if output.is_empty() {
            continue;
        }
        let entry = format!(
            "- {}：{}\n",
            message.text.trim(),
            clip_chars(output, RECAP_STEP_CHARS)
        );
        let size = entry.chars().count();
        if used + size > RECAP_CHARS {
            break;
        }
        used += size;
        entries.push(entry);
    }
    entries.reverse();
    let body: String = entries.concat();
    if body.is_empty() {
        return String::new();
    }
    format!("上一轮用工具取得的内容（文件可能已经改动；需要准确内容时重新读取）：\n{body}")
}

/// Keep at most `limit` characters, with a visible marker when cut.
fn clip_chars(text: &str, limit: usize) -> String {
    let mut kept: String = text.chars().take(limit).collect();
    if text.chars().count() > limit {
        kept.push_str("\n…（已截断）");
    }
    kept
}

/// Messages whose text contains `needle`, oldest first; empty query finds none.
pub(crate) fn find_messages(messages: &[ChatMessage], needle: &str) -> Vec<usize> {
    let needle = needle.trim().to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    messages
        .iter()
        .enumerate()
        .filter(|(_, message)| message.text.to_lowercase().contains(&needle))
        .map(|(index, _)| index)
        .collect()
}

/// Resolve a model-written path for the undo write.
///
/// `work_dir` is where relative paths start. `scope_root` is the writable
/// boundary; `None` means full access. Symlinks may not point outside a scope.
pub(crate) fn resolve_scoped(
    work_dir: &Path,
    scope_root: Option<&Path>,
    target: &str,
) -> Option<PathBuf> {
    if target.trim().is_empty() {
        return None;
    }
    let relative = Path::new(target);
    let joined = if relative.is_absolute() {
        relative.to_path_buf()
    } else {
        work_dir.join(relative)
    };
    let parent = joined.parent()?.canonicalize().ok()?;
    let file_name = joined.file_name()?;
    let real = match joined.canonicalize() {
        Ok(real) => real,
        Err(_) => parent.join(file_name),
    };
    if let Some(root) = scope_root {
        let real_root = root.canonicalize().ok()?;
        if !real.starts_with(&real_root) {
            return None;
        }
    }
    Some(real)
}

/// Put a file back after an approved write, refusing to overwrite later edits.
///
/// `after` is what that write produced; the file must still hold exactly that.
pub(crate) fn restore_file(
    target: &Path,
    after: &str,
    before: &str,
    existed: bool,
) -> Result<(), String> {
    let current = std::fs::read_to_string(target).ok();
    if current.as_deref() != Some(after) {
        return Err("无法撤销：文件已被修改".to_string());
    }
    if existed {
        std::fs::write(target, before).map_err(|error| format!("撤销失败：{error}"))
    } else {
        match std::fs::remove_file(target) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("撤销失败：{error}")),
        }
    }
}

/// Width of one caption button, matching Windows' own 46px caption cells.
pub(crate) const WINDOW_BUTTON_WIDTH: f32 = 46.;

/// Which caption button sits under a pointer position, if any.
///
/// The three buttons are flush with the window's right edge and the title bar
/// is the window's first row, so the pointer alone names the button. The
/// window paints their hover state from this: the platform treats those cells
/// as non-client, so `WM_NCMOUSEMOVE` is what arrives there and gpui's own
/// hover hit-test does not always follow it.
pub(crate) fn window_button_at(
    x: f32,
    y: f32,
    window_width: f32,
    bar_height: f32,
) -> Option<WindowControlArea> {
    if !(0.0..bar_height).contains(&y) {
        return None;
    }
    let from_right = window_width - x;
    if !(0.0..=3.0 * WINDOW_BUTTON_WIDTH).contains(&from_right) {
        return None;
    }
    if from_right <= WINDOW_BUTTON_WIDTH {
        Some(WindowControlArea::Close)
    } else if from_right <= 2.0 * WINDOW_BUTTON_WIDTH {
        Some(WindowControlArea::Max)
    } else {
        Some(WindowControlArea::Min)
    }
}

/// The badge on an approval card.
///
/// Besides the engine's own tools there are two families the window never sees
/// by name: MCP tools (`mcp` and the `mcp_<server>_<tool>` names it assigns)
/// and whatever an external plugin registered. Anything that is neither a known
/// built-in nor an MCP tool is a plugin tool, because those are the only two
/// sources the engine lets add tools.
pub(crate) fn approval_label(tool: &str) -> &'static str {
    match tool {
        "shell" => "命令确认",
        "http" => "请求确认",
        "python" | "node" => "代码确认",
        "task" => "任务确认",
        "move" => "移动确认",
        "delete" => "删除确认",
        "replace" => "批量替换确认",
        "write" | "edit" => "写入确认",
        "mcp" => "MCP 确认",
        name if name.starts_with("mcp_") => "MCP 确认",
        _ => "插件确认",
    }
}

/// The hint under an approval card's buttons.
pub(crate) fn approval_hint(tool: &str) -> &'static str {
    match tool {
        "shell" => "允许后才会执行；「本次会话都允许」只记住同一条命令",
        "http" => "允许后才会发出请求；请求不会自动跟随重定向",
        "python" | "node" => "允许后才会运行这段代码；「本次会话都允许」只记住同一段代码",
        "task" => "允许后才会结束后台任务；拒绝则它继续运行",
        "move" => "允许后才会移动；拒绝则文件保持原样",
        "delete" => "允许后才会删除；删除后不能撤销",
        "replace" => "允许后才会改写列出的文件；拒绝则文件保持原样",
        "write" | "edit" => "允许后才会写入；拒绝则文件保持原样",
        "mcp" => "允许后才会调用 MCP 工具；每次调用都会再问一次",
        name if name.starts_with("mcp_") => "允许后才会调用 MCP 工具；每次调用都会再问一次",
        // Plugin code runs inside the engine, so it asks every single time and
        // there is nothing the window could remember for the session.
        _ => "允许后才会运行插件代码；每次调用都会再问一次，拒绝则什么也不会执行",
    }
}

/// What a card says after the call behind it finished.
pub(crate) fn approval_state(tool: &str, ok: bool) -> &'static str {
    match (tool, ok) {
        ("shell" | "python" | "node", true) => "已执行",
        ("shell" | "python" | "node", false) => "执行失败",
        ("http", true) => "已完成",
        ("http", false) => "请求失败",
        ("task", true) => "已结束",
        ("task", false) => "结束失败",
        ("move", true) => "已移动",
        ("move", false) => "移动失败",
        ("delete", true) => "已删除",
        ("delete", false) => "删除失败",
        ("replace", true) => "已替换",
        ("replace", false) => "替换失败",
        ("write" | "edit", true) => "已写入",
        ("write" | "edit", false) => "写入失败",
        ("mcp", true) => "已执行",
        ("mcp", false) => "执行失败",
        // MCP tools are prefixed by the engine; they run as an RPC call rather
        // than a file write.
        (name, true) if name.starts_with("mcp_") => "已执行",
        (name, false) if name.starts_with("mcp_") => "执行失败",
        // Whatever is left is a plugin tool: it ran code, it did not write a
        // file, so it must not borrow the write wording (or a write's undo).
        (_, true) => "已执行",
        (_, false) => "执行失败",
    }
}

/// What one plugin added to the last request, as a card's suffix.
///
/// Empty when the plugin contributed nothing (or is switched off), so a plain
/// card keeps its short line.
pub(crate) fn plugin_contribution(
    id: &str,
    tools: &[(String, String)],
    prompts: &[(String, String)],
) -> String {
    let tools = tools.iter().filter(|(plugin, _)| plugin == id).count();
    let prompts = prompts.iter().filter(|(plugin, _)| plugin == id).count();
    let mut text = String::new();
    if tools > 0 {
        text.push_str(&format!(" · {tools} 个工具"));
    }
    if prompts > 0 {
        text.push_str(&format!(" · {prompts} 段提示"));
    }
    text
}

/// Mark every undecided approval, question or plan in a conversation.
///
/// A stopped, failed or finished request can no longer take a decision, so the
/// card must not keep offering buttons that would go nowhere.
pub(crate) fn settle_interactions(chat: &mut Chat, state: &str) {
    for message in &mut chat.messages {
        if matches!(message.role, Role::Approval | Role::Question | Role::Plan)
            && message.output.trim().is_empty()
        {
            message.output = state.to_string();
            // Settled cards fold; their diff stays one click away. Questions and
            // plans have no folded body, so this only affects approvals.
            message.open = false;
        }
    }
}

/// Whether a low-risk approval can be granted by rule instead of a card.
///
/// Deliberately conservative: only writes inside the conversation's workspace
/// with an ordinary path, and a short list of read-only commands. Anything a
/// shell can chain (`&&`, `|`, `>`, `$(…)`), anything that deletes or publishes,
/// and every plugin/MCP call still asks. Note that `cargo` commands on the list
/// do run the workspace's own build scripts and tests, so auto review is only
/// meant for a workspace the person trusts.
pub(crate) fn auto_review_allows(tool: &str, command: &str, path: &str, diff: &str) -> bool {
    if matches!(tool, "write" | "edit") {
        let path = path.trim().to_ascii_lowercase();
        if path.is_empty() {
            return false;
        }
        // A relative path only: an absolute path can leave the workspace, and
        // `..` can climb out of it.
        if path.starts_with('/')
            || path.starts_with('\\')
            || path.contains("..")
            || path.contains(':')
        {
            return false;
        }
        if path.contains(".git/") || path.contains(".git\\") {
            return false;
        }
        if path.contains(".env")
            || path.ends_with(".pem")
            || path.ends_with(".key")
            || path.contains("credential")
            || path.contains("secret")
            || path.contains(".ssh/")
        {
            return false;
        }
        // A huge rewrite is worth a card even inside the workspace.
        return diff.len() <= 200_000;
    }
    if tool != "shell" {
        return false;
    }
    let command = command.trim();
    if command.is_empty() || command.len() > 2_000 {
        return false;
    }
    // Shell metacharacters can chain a safe-looking command with an unsafe one.
    // `$` and the parentheses cover command substitution — `cat $(rm -rf .)`
    // stays inside the allow-list otherwise — and are refused rather than
    // guessed at, so a command using them asks for a card instead.
    if command.chars().any(|ch| {
        matches!(
            ch,
            '&' | '|' | ';' | '>' | '<' | '`' | '\n' | '\r' | '$' | '(' | ')'
        )
    }) {
        return false;
    }
    // Options that turn a "read" into a write or an exec.
    if command.contains("--fix")
        || command.contains("--output")
        || command.contains("--exec")
        || command.contains("--ext-diff")
    {
        return false;
    }
    let words: Vec<&str> = command.split_whitespace().collect();
    matches!(
        words.as_slice(),
        ["git", "status", ..]
            | ["git", "diff", ..]
            | ["git", "log", ..]
            | ["git", "show", ..]
            | ["git", "rev-parse", ..]
            | ["git", "ls-files", ..]
            | ["git", "branch"]
            | ["git", "branch", "--show-current"]
            | ["cargo", "check", ..]
            | ["cargo", "test", ..]
            | ["cargo", "clippy", ..]
            | ["cargo", "build", ..]
            | ["dir", ..]
            | ["ls", ..]
            | ["type", ..]
            | ["cat", ..]
            | ["node", "--version"]
            | ["node", "-v"]
            | ["python", "--version"]
            | ["python", "-V"]
    )
}

/// Short conversation title from the first question.
pub(crate) fn title_from(text: &str) -> String {
    let single_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let limit = 18;
    if single_line.chars().count() <= limit {
        return single_line;
    }
    let mut title: String = single_line.chars().take(limit).collect();
    title.push('…');
    title
}

/// Short, readable duration.
/// The faint line that closes an answer: who wrote it and what it cost.
pub(crate) fn answer_label(model: &str, elapsed_ms: u64, tokens: u64) -> String {
    let mut parts = vec!["助手".to_string()];
    parts.extend(answer_details(model, elapsed_ms, tokens));
    parts.join(" · ")
}

/// Model, time and cost, in that order, without the "助手" prefix.
///
/// Empty when nothing is known, which is what old saves and stopped answers look
/// like; the label then drops the whole line instead of saying "助手" alone.
pub(crate) fn answer_details(model: &str, elapsed_ms: u64, tokens: u64) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    if !model.is_empty() {
        parts.push(model.to_string());
    }
    if elapsed_ms > 0 {
        parts.push(format_duration(elapsed_ms));
    }
    if tokens > 0 {
        parts.push(format!("{} tokens", short_count(tokens)));
    }
    parts
}

/// The whole conversation as Markdown, for copying it somewhere else.
pub(crate) fn transcript_text(title: &str, messages: &[ChatMessage]) -> String {
    let mut out = String::new();
    let title = title.trim();
    if !title.is_empty() {
        out.push_str(&format!("# {title}\n\n"));
    }
    for (position, message) in messages.iter().enumerate() {
        match message.role {
            Role::User => out.push_str(&format!("## 你\n\n{}\n\n", message.text.trim())),
            Role::Assistant => {
                let details = answer_details(&message.model, message.elapsed_ms, message.tokens);
                let heading = if details.is_empty() {
                    "## 助手".to_string()
                } else {
                    format!("## 助手（{}）", details.join(" · "))
                };
                out.push_str(&format!("{heading}\n\n{}\n\n", message.text.trim()));
            }
            Role::Tool => {
                out.push_str(&format!("- 工具 · {}\n", message.text.trim()));
                // One blank line ends a run of steps, so the next heading is a
                // heading and consecutive steps stay a list.
                let more = messages
                    .get(position + 1)
                    .is_some_and(|next| matches!(next.role, Role::Tool));
                if !more {
                    out.push('\n');
                }
            }
            Role::Approval => {
                let state = if message.output.trim().is_empty() {
                    "未决"
                } else {
                    message.output.trim()
                };
                out.push_str(&format!(
                    "- 确认 · {}（{}）\n\n",
                    message.text.trim(),
                    state
                ));
            }
            Role::Attachment => {
                out.push_str(&format!(
                    "## 附件 · {}\n\n```text\n{}\n```\n\n",
                    message.path.trim(),
                    message.text.trim()
                ));
            }
            Role::Note => out.push_str(&format!("> {}\n\n", message.text.trim())),
            Role::Question => {
                let state = if message.output.trim().is_empty() {
                    "未决"
                } else {
                    message.output.trim()
                };
                out.push_str(&format!(
                    "## 提问（{}）\n\n{}\n\n",
                    state,
                    message.text.trim()
                ));
            }
            Role::Plan => {
                let state = if message.output.trim().is_empty() {
                    "未决"
                } else {
                    message.output.trim()
                };
                out.push_str(&format!(
                    "## 计划（{}）\n\n{}\n\n",
                    state,
                    message.text.trim()
                ));
            }
            // Thinking is the model's, not the conversation's: the plain-text
            // export keeps what a reader could have said or been told.
            Role::Thinking => {}
        }
    }
    out.trim_end().to_string() + "\n"
}

/// Element id for one code block's copy button.
///
/// The streaming draft uses `DRAFT_MESSAGE` (`usize::MAX`) as its slot, so the
/// multiplication has to wrap: overflowing here panicked inside the render pass
/// and took the window down with it.
pub(crate) fn code_copy_id(message: usize, block: usize) -> usize {
    message.wrapping_mul(1000).wrapping_add(block)
}

/// The small line under a conversation's title.
pub(crate) fn chat_meta(count: usize, busy: bool, updated_ms: u64, draft: bool) -> String {
    let mut text = if busy {
        format!("正在回答 · {count} 条消息")
    } else if draft {
        format!("草稿 · {count} 条消息")
    } else {
        format!("对话 · {count} 条消息")
    };
    if updated_ms > 0 {
        text.push_str(" · ");
        text.push_str(&ago(updated_ms));
    }
    text
}

/// How full the model window is, 0–100; zero when nothing is known yet.
///
/// `context` is what the provider said the last request carried, so it measures
/// the window itself. The billed `tokens` of a turn sums every tool round and
/// therefore runs past the window on its own — it is the spend figure, not this
/// one.
pub(crate) fn chat_context_percent(chat: &Chat, model: &str) -> f64 {
    chat_context_percent_in(chat, crate::ai::context_window(model))
}

/// The same gauge against a window the caller resolved.
///
/// A connection may name its own window (a local model, or one newer than the
/// name table); the gauge has to use that figure or it would disagree with
/// where the engine compacts.
pub(crate) fn chat_context_percent_in(chat: &Chat, window: u64) -> f64 {
    let in_context = chat
        .messages
        .iter()
        .rev()
        .find(|message| message.context > 0)
        .map_or(0, |message| message.context);
    if in_context == 0 || window == 0 {
        0.0
    } else {
        in_context as f64 * 100.0 / window as f64
    }
}

/// The title as the header row can show it: long renamed titles are clipped so
/// the figures beside them stay on the same line.
pub(crate) fn header_title(title: &str) -> String {
    let limit = 18;
    let mut chars = title.chars();
    let short: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() {
        format!("{short}…")
    } else {
        short
    }
}

/// Whole counts below a thousand, one decimal above it: `845`, `1.2k`, `23k`.
pub(crate) fn short_count(count: u64) -> String {
    if count < 1000 {
        return count.to_string();
    }
    let thousands = count as f64 / 1000.;
    if thousands < 10. {
        format!("{thousands:.1}k")
    } else if thousands < 100. {
        format!("{thousands:.0}k")
    } else {
        format!("{}k", count / 1000)
    }
}

pub(crate) fn format_duration(ms: u64) -> String {
    if ms < 1000 {
        return format!("{ms} ms");
    }
    if ms < 60_000 {
        return format!("{:.1} s", ms as f64 / 1000.0);
    }
    format!("{}m {:02}s", ms / 60_000, (ms % 60_000) / 1000)
}

/// How long ago a run settled.
/// Open a path with the platform's handler, never through a shell.
pub(crate) fn open_path(path: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = std::process::Command::new("explorer");
        command.arg(path);
        command
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = std::process::Command::new("open");
        command.arg(path);
        command
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut command = std::process::Command::new("xdg-open");
        command.arg(path);
        command
    };
    command.spawn().map(|_| ())
}

/// Open an `http(s)` link in the system browser, never through a shell.
///
/// Answers are untrusted text, so the address is checked first (see
/// [`crate::markdown::is_openable_link`]) and passed as a single argument: a
/// `file:` or `javascript:` "link" must not be able to run anything.
pub(crate) fn open_url(url: &str) -> std::io::Result<()> {
    let url = url.trim();
    if !crate::markdown::is_openable_link(url) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "只能打开 http / https / mailto 链接",
        ));
    }
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = std::process::Command::new("rundll32");
        command.args(["url.dll,FileProtocolHandler", url]);
        command
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = std::process::Command::new("open");
        command.arg(url);
        command
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut command = std::process::Command::new("xdg-open");
        command.arg(url);
        command
    };
    command.spawn().map(|_| ())
}

/// Whether the transcript ends with an answer that can be picked up again.
///
/// The interrupted answer is the last entry, or sits behind the failure note that
/// cancelled it; a complete answer, an empty transcript and anything else are
/// not continuable.
pub(crate) fn continuable(messages: &[ChatMessage]) -> bool {
    match messages.last() {
        Some(message) if message.role == Role::Assistant => message.interrupted,
        Some(message) if message.role == Role::Note && message.failed => messages
            .get(messages.len().saturating_sub(2))
            .is_some_and(|prev| prev.role == Role::Assistant && prev.interrupted),
        _ => false,
    }
}

/// What a stopped answer leaves in the transcript.
///
/// Only the text that was already written, kept as a normal answer: the stream was
/// cut short, the words were shown, and stopping needs no note of its own. With no
/// text there is nothing to keep.
pub(crate) fn stopped_messages(draft: &str) -> Vec<ChatMessage> {
    let text = draft.trim();
    if text.is_empty() {
        Vec::new()
    } else {
        let mut message = ChatMessage::assistant(text.to_string());
        // The answer stopped mid-stream: the reader can pick it up again.
        message.interrupted = true;
        vec![message]
    }
}

/// Writes that belong to the turn ending at `answer` and can still be undone.
///
/// The turn starts after the previous question: tool rows and approval cards
/// are stored before the answer they produced, so rolling back an answer means
/// walking that stretch backwards.
pub(crate) fn turn_writes(messages: &[ChatMessage], answer: usize) -> Vec<usize> {
    let end = answer.min(messages.len());
    let start = messages[..end]
        .iter()
        .rposition(|message| matches!(message.role, Role::User))
        .map_or(0, |index| index + 1);
    messages[start..end]
        .iter()
        .enumerate()
        .filter(|(_, message)| {
            // A write is undoable whether it was approved on a card or run
            // straight through in 完全权限, where the tool row is its record.
            message.undoable
                && !message.path.is_empty()
                && match message.role {
                    Role::Approval => message.output == "已写入",
                    Role::Tool => true,
                    _ => false,
                }
        })
        .map(|(offset, _)| start + offset)
        .collect()
}

/// What one answer changed: files touched and diff lines, as a compact summary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TurnChange {
    /// Distinct files written or edited, even if one was changed twice.
    pub files: usize,
    /// Added lines counted from the approval diffs.
    pub added: usize,
    /// Removed lines counted the same way.
    pub removed: usize,
}

/// Summarise the writes still visible under `answer` (the ones not undone).
///
/// The counts come from the line diff the confirmation card already holds, so
/// no file is read again; a diff clipped at 400 lines makes the numbers a
/// lower bound, which is why the label says 约.
pub(crate) fn turn_change(messages: &[ChatMessage], answer: usize) -> Option<TurnChange> {
    let end = answer.min(messages.len());
    let start = messages[..end]
        .iter()
        .rposition(|message| matches!(message.role, Role::User))
        .map_or(0, |index| index + 1);
    let mut paths: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut added = 0usize;
    let mut removed = 0usize;
    for message in &messages[start..end] {
        if !matches!(message.role, Role::Approval)
            || message.output != "已写入"
            || !matches!(message.tool.as_str(), "write" | "edit")
        {
            continue;
        }
        if !message.path.is_empty() {
            paths.insert(message.path.as_str());
        }
        for line in message.input.lines() {
            if line.starts_with("+++") || line.starts_with("---") {
                continue;
            }
            if line.starts_with("+ ") {
                added += 1;
            } else if line.starts_with("- ") {
                removed += 1;
            }
        }
    }
    (added > 0 || removed > 0).then_some(TurnChange {
        files: paths.len(),
        added,
        removed,
    })
}

/// Longest live command output kept on the tool row.
pub(crate) const LIVE_OUTPUT_MAX: usize = 4_000;

/// The last non-empty line of a live output chunk, trimmed for the row.
///
/// A command may print a progress bar or repeat a line; only the newest line
/// belongs in the collapsed row. `None` when the chunk is blank.
pub(crate) fn last_output_line(text: &str) -> Option<String> {
    text.split(['\n', '\r'])
        .rev()
        .find(|line| !line.trim().is_empty())
        .map(|line| line.trim().chars().take(120).collect())
}

/// The collapsed row text for a running command.
pub(crate) fn live_row_text(tool: &str, target: &str, line: Option<&str>) -> String {
    match (line, target.is_empty()) {
        (Some(line), false) => format!("{tool} · {target} — {line}"),
        (Some(line), true) => format!("{tool} — {line}"),
        (None, false) => format!("{tool} · {target}"),
        (None, true) => tool.to_string(),
    }
}

/// Append live output while keeping the newest [`LIVE_OUTPUT_MAX`] characters.
pub(crate) fn append_live_output(current: &mut String, text: &str) {
    current.push_str(text);
    let total = current.chars().count();
    if total > LIVE_OUTPUT_MAX
        && let Some((index, _)) = current.char_indices().nth(total - LIVE_OUTPUT_MAX)
    {
        current.drain(..index);
    }
}

/// Where the last question starts and what it said.
///
/// Everything after it is the answer (and tool rows) that a retry throws away.
pub(crate) fn last_question(messages: &[ChatMessage]) -> Option<(usize, String)> {
    messages
        .iter()
        .rposition(|message| matches!(message.role, Role::User))
        .map(|index| (index, messages[index].text.clone()))
}

/// Take the newest question out of a transcript, returning its text.
///
/// Everything after it — the answer, tool rows, notes — goes with it: asking the
/// question again or editing it means replacing what it produced.
pub(crate) fn take_last_question(messages: &mut Vec<ChatMessage>) -> Option<String> {
    let (position, question) = last_question(messages)?;
    messages.truncate(position);
    Some(question)
}

/// Which message carries the "retry" control, if any.
///
/// Retrying asks the newest question again, so the control goes where the answer
/// should have been: on a failure note when the transcript ends with one (there is
/// no answer to put it on), otherwise on the last message that is not a note —
/// and only when that is an answer, since a question still waiting for one has
/// nothing to replace.
pub(crate) fn retry_message(messages: &[ChatMessage]) -> Option<usize> {
    let last = messages.len().checked_sub(1)?;
    // A failure ends the transcript: the control goes on the note itself. A stop
    // note is not a failure, so it falls through to the half-written answer.
    if matches!(messages[last].role, Role::Note) && messages[last].failed {
        return last_question(messages).map(|_| last);
    }
    let answer = messages.iter().rposition(|message| {
        !matches!(message.role, Role::Note | Role::Approval | Role::Attachment)
    })?;
    (matches!(messages[answer].role, Role::Assistant) && last_question(messages).is_some())
        .then_some(answer)
}
