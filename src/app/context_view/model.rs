//! The shapes the page reads: one row per request, one report per conversation.

use super::*;

/// How many answered questions the detail table lists, newest first.
pub(crate) const REQUEST_ROWS: usize = 30;

/// One answered question, as the panel's detail table lists it.
#[derive(Clone)]
pub(crate) struct RequestRow {
    /// Which question it was in this conversation, counting from one.
    pub(crate) turn: u64,
    /// Model that answered.
    pub(crate) model: String,
    /// How long the whole question took, in milliseconds.
    pub(crate) elapsed_ms: u64,
    /// Wall clock the engine spent waiting for model round trips, in
    /// milliseconds; zero when it did not report timing (older engines).
    pub(crate) model_ms: u64,
    /// Wall clock the engine spent running tools, in milliseconds; zero when
    /// it did not report timing.
    pub(crate) tool_ms: u64,
    /// Tokens the endpoint billed: every tool round of that question, added up.
    pub(crate) tokens: u64,
    /// Prompt tokens the endpoint reported for its rounds, added up; zero when
    /// it reported no per-round counts.
    pub(crate) prompt_tokens: u64,
    /// Answer tokens the endpoint reported for its rounds, added up; zero when
    /// it reported no per-round counts.
    pub(crate) answer_tokens: u64,
    /// Prompt tokens the endpoint served from its cache for those rounds, added
    /// up; zero when it did not report them.
    pub(crate) cache_hits: u64,
    /// Prompt + answer of its last round, which is what filled the window.
    pub(crate) context: u64,
    /// Model round trips it took, the answer included; zero when unknown, and
    /// capped at [`ROUND_LIMIT`] so a damaged save cannot size the panel's round
    /// list.
    pub(crate) rounds: u32,
    /// What each round cost, oldest first, as `(prompt, completion, tool calls)`;
    /// empty for answers stored before the engine reported it.
    pub(crate) round_tokens: Vec<(u64, u64, u32)>,
    /// Server-side tool calls of each round (`web_search` / `read_url`).
    ///
    /// The engine's per-round figure counts only local tools, and a server call
    /// runs inside the round trip that asked for it, so these are read from the
    /// transcript's call ids (`round-1-2` names round 1, second call).
    pub(crate) round_server_calls: Vec<u32>,
    /// Estimated thinking tokens of each round, oldest first: output the bill
    /// includes that no prompt holds. Empty for providers that send none.
    pub(crate) round_reasoning: Vec<u32>,
    /// The thinking text of each round, oldest first, built from the transcript's
    /// thinking rows so a picked round opens its own words.
    pub(crate) round_reasoning_text: Vec<String>,
    /// Tool calls made while answering it.
    pub(crate) tools: u64,
    /// Which compaction layer ran just before it (`prune`, `summary`, `drop`);
    /// empty when nothing was compressed or the answer predates the field.
    pub(crate) compaction_layer: String,
    /// What that request was made of, in [`ContextKind::ALL`] order, with every
    /// round's estimate added up; empty for answers saved before it was
    /// recorded.
    pub(crate) composition: Vec<u32>,
    /// The same make-up closed on the endpoint's own figures: the prompt side
    /// is measured, the answer side is measured, and what the estimate could
    /// not see joins the category it came from (a server search's pages to
    /// 工具结果, the engine's replay to 注入).
    ///
    /// The request card, the whole-conversation table, the ring and the
    /// timeline all read this, so a server search cannot be 80% of one view
    /// and 10% of another.
    pub(crate) closed_composition: Vec<usize>,
    /// What each round of the request was made of, oldest first, as raw
    /// estimates in [`ContextKind::ALL`] order. One entry when the answer has
    /// no per-round figures; empty when nothing was recorded.
    pub(crate) round_composition: Vec<Vec<usize>>,
}

/// Which tab the conversation pane shows.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChatTab {
    /// The transcript and its composer.
    Chat,
    /// The breakdown of the next request.
    Context,
    /// The conversation's background tasks.
    Tasks,
}

/// Where one block of the system prompt came from.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum PromptKind {
    /// Rules the window always sends.
    System,
    /// Everything added on top: project instructions, maps, summaries, the
    /// previous turn's tool recap.
    Injected,
    /// Instructions from the skill folders switched on in the Skill page.
    Skills,
}

/// One labelled, separately counted piece of the system prompt.
pub(crate) struct PromptBlock {
    /// Which panel category it belongs to.
    pub(crate) kind: PromptKind,
    /// Short name shown above the block's text.
    pub(crate) label: &'static str,
    /// Separator that goes before it, so joining matches what was sent before
    /// this breakdown existed.
    pub(crate) sep: &'static str,
    /// The text itself.
    pub(crate) text: String,
}

/// One category of the next request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ContextKind {
    /// The rules and the environment line.
    System,
    /// The tool schemas the engine declares.
    Tools,
    /// What the person asked.
    User,
    /// Project instructions, the workspace map, the early-turns summary.
    Injected,
    /// Prompt text from switched-on skill folders.
    Skills,
    /// What the assistant said.
    Assistant,
    /// What the previous turn's tools returned.
    ToolResults,
    /// What the model wrote back as that round's output.
    Output,
    /// Thinking the model streamed before its answer; only reasoners send it.
    ///
    /// Appended last on purpose: compositions are stored by position, so a new
    /// category at the end leaves every older save readable.
    Reasoning,
}

impl ContextKind {
    /// Every category, in the order the panel lists them.
    pub(crate) const ALL: [Self; 9] = [
        Self::System,
        Self::Tools,
        Self::User,
        Self::Injected,
        Self::Skills,
        Self::Assistant,
        Self::ToolResults,
        Self::Output,
        Self::Reasoning,
    ];

    /// The name the panel shows.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::System => "系统提示词",
            Self::Tools => "工具定义",
            Self::User => "用户消息",
            Self::Injected => "注入内容",
            Self::Skills => "注入技能",
            Self::Assistant => "助手消息",
            Self::ToolResults => "工具结果",
            Self::Output => "输出文字",
            Self::Reasoning => "推理过程",
        }
    }

    /// The chart colour: the same in both themes, because it names a category
    /// rather than a surface.
    pub(crate) fn color(self) -> u32 {
        match self {
            Self::System => 0x006b_7fd7,
            Self::Tools => 0x00e0_8a2c,
            Self::User => 0x002f_9e5f,
            Self::Injected => 0x00a8_55f7,
            Self::Skills => 0x00ef_6c2f,
            Self::Assistant => 0x003b_82f6,
            Self::ToolResults => 0x0014_a3a3,
            Self::Output => 0x00d9_4a8b,
            Self::Reasoning => 0x004f_46e5,
        }
    }

    /// Anything to say about a category this build never fills.
    pub(crate) fn empty_note(self) -> Option<&'static str> {
        match self {
            Self::Skills => Some("没有启用的技能（左侧 Skill 页开启）"),
            Self::Reasoning => Some("未回报思考过程；仅推理模型会记录"),
            _ => None,
        }
    }

    /// The name this category is stored under in `requests.jsonl`.
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Tools => "tools",
            Self::User => "user",
            Self::Injected => "injected",
            Self::Skills => "skills",
            Self::Assistant => "assistant",
            Self::ToolResults => "toolResults",
            Self::Output => "output",
            Self::Reasoning => "reasoning",
        }
    }

    /// Read a stored category back; an unknown key is skipped rather than
    /// guessed at.
    pub(crate) fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.key() == key)
    }
}

/// One countable piece of a category, with the text that can be read.
#[derive(Clone)]
pub(crate) struct ContextItem {
    /// What it is: a block name, a tool name, a turn's first line.
    pub(crate) label: String,
    /// The whole text that goes to the model.
    pub(crate) text: String,
    /// Estimated tokens for this piece.
    pub(crate) tokens: usize,
}

/// One category of one request.
#[derive(Clone)]
pub(crate) struct ContextPart {
    /// Which category this is.
    pub(crate) kind: ContextKind,
    /// The pieces it is made of.
    pub(crate) items: Vec<ContextItem>,
    /// Estimated tokens for the whole category.
    pub(crate) tokens: usize,
}

/// One counted piece of a request, kept so a past request can be opened again.
#[derive(Clone)]
pub(crate) struct RequestItem {
    /// Which category it belongs to.
    pub(crate) kind: ContextKind,
    /// What it is: a block name, a tool name, a turn's first line.
    pub(crate) label: String,
    /// The whole text that went to the model.
    pub(crate) text: String,
    /// Estimated tokens for this piece.
    pub(crate) tokens: usize,
}

/// What a conversation holds, for the 上下文tab.
#[derive(Default)]
pub(crate) struct ContextReport {
    /// The model's window, for the percentage.
    pub(crate) window: u64,
    /// Questions asked in this conversation.
    pub(crate) asked: u64,
    /// Estimated tokens of the replayed history alone.
    pub(crate) history_tokens: usize,
    /// Tool calls recorded in this conversation.
    pub(crate) steps: u64,
    /// Answers produced in this conversation.
    pub(crate) replies: u64,
    /// Room kept for the answer, from the connection's token cap.
    pub(crate) output: u64,
    /// Estimated tokens of history the engine replays before it starts
    /// dropping the oldest turns.
    pub(crate) history_budget: usize,
    /// Tokens the endpoint billed over the whole conversation.
    pub(crate) spend: u64,
    /// Every request's closed make-up added up, in [`ContextKind::ALL`] order;
    /// drives the whole-conversation ring and table.
    pub(crate) total_composition: Vec<usize>,
    /// Every request's raw local estimate added up: the figure the panel would
    /// count without the endpoint's closing, shown as 「分类估算」 beside the bill.
    pub(crate) estimate_total: u64,
    /// Every round's prompt the endpoint reported, over the whole conversation.
    pub(crate) prompt_total: u64,
    /// Every round's answer the endpoint reported, over the whole conversation.
    pub(crate) answer_total: u64,
    /// Billed tokens no round accounted for, over the whole conversation: the
    /// answers whose endpoint reported a total but no per-round counts (an
    /// older record, or a question sent with the tools switched off).
    pub(crate) unreported: u64,
    /// The answered questions, newest first, for the request list.
    pub(crate) requests: Vec<RequestRow>,
    /// Turns whose answer followed a compaction note in a save that predates
    /// the structured field. Read once with the report instead of on every
    /// paint of the timeline.
    pub(crate) noted_compactions: std::collections::HashSet<u64>,
}

impl ContextReport {
    /// True when the replayed history has reached the trimming budget.
    pub(crate) fn history_full(&self) -> bool {
        self.history_budget > 0 && self.history_tokens >= self.history_budget
    }
}

/// The pieces one round carried: every category as stored, but only the first
/// `upto` tool results —the ones the rounds before it had already produced.
///
/// The request's stored pieces are shared by every view, so a round narrows
/// them instead of copying every label and body: with large tool results, the
/// old copy-per-paint made scrolling the panel needlessly expensive.
pub(crate) enum PartItems<'a> {
    /// Every stored piece belongs to the view.
    All(&'a [ContextItem]),
    /// Only the tool results returned before the round started; a server
    /// result is already visible in the round that ran it.
    ToolResults {
        items: &'a [ContextItem],
        upto: usize,
    },
    /// One piece of the round's own (its thinking).
    One(&'a ContextItem),
}

impl<'a> PartItems<'a> {
    /// Walk the pieces the view actually carries, in stored order.
    pub(crate) fn iter(&self) -> PartItemsIter<'a> {
        match self {
            Self::All(items) => PartItemsIter::All(items.iter()),
            Self::ToolResults { items, upto } => PartItemsIter::Tools {
                items: items.iter(),
                upto: *upto,
                local: 0,
            },
            Self::One(item) => PartItemsIter::One(Some(item)),
        }
    }

    /// How many pieces the view carries and what they estimate, in one pass.
    pub(crate) fn count_and_estimate(&self) -> (usize, usize) {
        self.iter().fold((0, 0), |(count, tokens), item| {
            (count + 1, tokens + item.tokens)
        })
    }
}

/// One step of [`PartItems::iter`].
pub(crate) enum PartItemsIter<'a> {
    All(std::slice::Iter<'a, ContextItem>),
    Tools {
        items: std::slice::Iter<'a, ContextItem>,
        upto: usize,
        local: usize,
    },
    One(Option<&'a ContextItem>),
}

impl<'a> Iterator for PartItemsIter<'a> {
    type Item = &'a ContextItem;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::All(items) => items.next(),
            Self::One(item) => item.take(),
            Self::Tools { items, upto, local } => loop {
                let item = items.next()?;
                if is_server_result(&item.label) {
                    return Some(item);
                }
                let index = *local;
                *local += 1;
                if index < *upto {
                    return Some(item);
                }
            },
        }
    }
}

/// One category row in the request card: the pieces on show, the closed token
/// figure for the row, and any measured remainder the pieces do not explain.
pub(crate) struct PartView<'a> {
    pub(crate) kind: ContextKind,
    pub(crate) items: PartItems<'a>,
    pub(crate) tokens: usize,
    pub(crate) gap: Option<usize>,
    /// How many entries the expansion lists: the pieces plus the measured gap.
    pub(crate) entries: usize,
}

/// The two thin scrollbars the context panel draws by hand.
///
/// Both sit beside a fixed-height box and behave the same way — track press,
/// thumb drag, wheel — so one implementation serves them; the kind picks the
/// handle and the drag slot.
#[derive(Clone, Copy)]
pub(crate) enum BarKind {
    /// The category table inside the totals card.
    Totals,
    /// The text of the expanded category item.
    Item,
}
