//! The built-in tool catalogue: one row per tool the assistant can call.
//!
//! The engine owns what a tool *does* (`dsh-flow/src/tools/specs.ts`, plus the
//! interaction tools in `tools/interaction.ts`). This table is the window's side
//! of the same list — the card text the plugins page draws, and the two facts the
//! permission sentence depends on: whether a tool writes a file, and whether it
//! asks in every level except 完全权限.
//!
//! Keeping it in one place is what makes a consistency test possible: the lists
//! used to live in four files, and every new tool had to be added to all of them
//! by hand (`wait_for exit` and `task close/wait` are what that cost).
//!
//! The `description` here is only the fallback: once the engine has reported its
//! schemas, the page draws the engine's own description ([`shown_description`]),
//! because that text is the one the model is actually told.

use std::collections::HashMap;

/// One card on the plugins page, and what the permission sentence needs to know.
pub(crate) struct ToolCard {
    /// Name the model calls.
    pub name: &'static str,
    /// Two lines of grey text under the name.
    pub description: &'static str,
    /// Writes a file inside the workspace (asks only in 仅可查看).
    pub writes: bool,
    /// A command, a request or a destructive file action: asks in every level
    /// except 完全权限.
    pub always: bool,
    /// Section the page files this card under.
    pub group: ToolGroup,
}

/// The sections the tools page draws, in order.
///
/// The engine's catalogue is one flat list; a person reading the page thinks in
/// families ("file things", "running things"), so the page groups them without
/// changing what the model is offered. A group is a *display* fact: it must not
/// turn into a second permission or concurrency rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ToolGroup {
    /// Reading, writing and moving files.
    Files,
    /// Finding files and symbols.
    Search,
    /// Looking at and changing the repository.
    VersionControl,
    /// Commands, background tasks and the checks that run them.
    Run,
    /// Requests and pictures.
    Network,
    /// Talking to the person, and the MCP gateway.
    Interactive,
}

/// What the engine says a tool does, read out of the schema it reported.
///
/// The engine sends the very function object it gives the model, so the text
/// lives under `function.description`. A schema that does not parse, or carries
/// no text, yields `None` and the catalogue line stays.
/// @param schema - one `items[].schema` string from the `tools` event.
/// @returns the description on one line, when there is one.
pub(crate) fn engine_description(schema: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(schema).ok()?;
    let text = value
        .get("function")
        .and_then(|function| function.get("description"))
        .or_else(|| value.get("description"))
        .and_then(serde_json::Value::as_str)?;
    let text = one_line(text);
    (!text.is_empty()).then_some(text)
}

/// The text a card shows: the engine's own description when it has been reported,
/// and the catalogue's shorter line until then.
///
/// The engine owns what a tool does — its description is what the model is told —
/// so the page reads that instead of keeping a second, independently edited copy.
/// @param card - the catalogue row.
/// @param engine - tool name → description, as the last `tools` event described.
/// @returns the description the page draws.
pub(crate) fn shown_description(card: &ToolCard, engine: &HashMap<String, String>) -> String {
    match engine.get(card.name) {
        Some(text) if !text.trim().is_empty() => one_line(text),
        _ => card.description.to_string(),
    }
}

/// Collapse the line breaks and runs of spaces a description may carry.
/// @param text - the raw description.
/// @returns one line, trimmed.
pub(crate) fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl ToolGroup {
    /// Every section, in the order the page draws them.
    pub(crate) const ALL: [ToolGroup; 6] = [
        ToolGroup::Files,
        ToolGroup::Search,
        ToolGroup::VersionControl,
        ToolGroup::Run,
        ToolGroup::Network,
        ToolGroup::Interactive,
    ];

    /// Heading the section shows.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Files => "文件与目录",
            Self::Search => "检索与索引",
            Self::VersionControl => "版本控制",
            Self::Run => "执行与任务",
            Self::Network => "网络与图片",
            Self::Interactive => "交互与外部",
        }
    }
}

/// Every built-in tool, in the order the catalogue was built.
pub(crate) const BUILTIN_TOOLS: [ToolCard; 33] = [
    ToolCard {
        name: "read",
        description: "读取对话目录里的文本文件，带行号，最多 400 行 / 20 万字符；PDF 也能读：按页抽取文字（需要 pdftotext 或 pypdf），pages 可指定范围",
        writes: false,
        always: false,
        group: ToolGroup::Files,
    },
    ToolCard {
        name: "write",
        description: "新建或整体覆盖文件，自动建目录，单次 ≤ 40 万字符；写回一律 UTF-8，覆盖非 UTF-8 文件会提示",
        writes: true,
        always: false,
        group: ToolGroup::Files,
    },
    ToolCard {
        name: "edit",
        description: "改文件：old / new 唯一替换、edits 批量替换，或按行号 startLine / endLine 替换与 insertAfter 插入；非 UTF-8（GBK）文件只改 ASCII 原文",
        writes: true,
        always: false,
        group: ToolGroup::Files,
    },
    ToolCard {
        name: "replace",
        description: "按 glob 选多个文件（可用 path 限定目录），批量替换同一段文本；默认每文件唯一",
        writes: true,
        always: false,
        group: ToolGroup::Files,
    },
    ToolCard {
        name: "move",
        description: "移动或重命名文件；目标已存在时拒绝，默认执行前确认",
        writes: false,
        always: true,
        group: ToolGroup::Files,
    },
    ToolCard {
        name: "copy",
        description: "复制文件（二进制资源也可以）；目标已存在时默认拒绝，确认后执行",
        writes: true,
        always: true,
        group: ToolGroup::Files,
    },
    ToolCard {
        name: "mkdir",
        description: "新建文件夹，父目录一并创建；确认后执行",
        writes: true,
        always: true,
        group: ToolGroup::Files,
    },
    ToolCard {
        name: "delete",
        description: "删除文件；recursive 为 true 时连文件夹内容一起删（不可撤销），默认需确认",
        writes: false,
        always: true,
        group: ToolGroup::Files,
    },
    ToolCard {
        name: "list",
        description: "列出文件夹的直接子项，目录以 / 结尾，不递归",
        writes: false,
        always: false,
        group: ToolGroup::Files,
    },
    ToolCard {
        name: "glob",
        description: "按通配符列文件，例如 src/**/*.ts；可用 path 限定目录，跳过 .git / node_modules",
        writes: false,
        always: false,
        group: ToolGroup::Search,
    },
    ToolCard {
        name: "tree",
        description: "按树形看结构（可设 depth），跳过 .git / node_modules / target，遵守 .gitignore",
        writes: false,
        always: false,
        group: ToolGroup::Files,
    },
    ToolCard {
        name: "grep",
        description: "按正则搜内容，可忽略大小写、带上下文、用 path 限定目录、perFile 控制每文件行数；返回「路径:行号: 内容」，最多 200 行",
        writes: false,
        always: false,
        group: ToolGroup::Search,
    },
    ToolCard {
        name: "symbols",
        description: "按名字找声明（Rust / TS / JS / Python / Go），首次建索引并缓存，10 分钟内复用；find: references 查整词引用",
        writes: false,
        always: false,
        group: ToolGroup::Search,
    },
    ToolCard {
        name: "git",
        description: "只读查看仓库：status / diff / log / show / ls-files 等，不会改动仓库",
        writes: false,
        always: false,
        group: ToolGroup::VersionControl,
    },
    ToolCard {
        name: "git_write",
        description: "改动仓库的三种安全操作：commit（按需 add 后提交）、branch（新建 / 切换）、stash（push / pop / list）；默认执行前确认",
        writes: false,
        always: true,
        group: ToolGroup::VersionControl,
    },
    ToolCard {
        name: "shell",
        description: "在当前目录运行命令（测试 / 构建 / 格式化）；路径受沙箱约束，默认执行前确认，可给 stdin 或用 background 转为后台任务",
        writes: false,
        always: true,
        group: ToolGroup::Run,
    },
    ToolCard {
        name: "task",
        description: "管理后台任务：list 列出、output 看输出、input 写 stdin、close 关 stdin、wait 等结束、stop 结束一个、stopAll 结束全部",
        writes: false,
        always: true,
        group: ToolGroup::Run,
    },
    ToolCard {
        name: "wait_for",
        description: "等端口能连上 / 页面应答 / 后台任务日志出现某段文本；默认最多 30 秒，执行前确认（等任务结束用 task 的 wait）",
        writes: false,
        always: true,
        group: ToolGroup::Run,
    },
    ToolCard {
        name: "run_tests",
        description: "跑项目的测试（自动识别 cargo test / go test / dotnet test / mvn -B test / gradle test / npm test / pytest），把失败用例解析成清单；执行前确认",
        writes: false,
        always: true,
        group: ToolGroup::Run,
    },
    ToolCard {
        name: "diagnostics",
        description: "跑编译器 / 类型检查（自动识别 cargo check --message-format=json / go vet / tsc --noEmit / dotnet build / mvn compile / gradle classes / eslint），把错误与警告解析成「文件:行:列 代码」清单；执行前确认",
        writes: false,
        always: true,
        group: ToolGroup::Run,
    },
    ToolCard {
        name: "python",
        description: "用 python -c 运行一段 Python 代码；支持多行，默认执行前确认",
        writes: false,
        always: true,
        group: ToolGroup::Run,
    },
    ToolCard {
        name: "node",
        description: "用 node -e 运行一段 Node.js 代码；支持多行，默认执行前确认",
        writes: false,
        always: true,
        group: ToolGroup::Run,
    },
    ToolCard {
        name: "http",
        description: "发 HTTP 请求（GET / POST / PUT / PATCH / DELETE / HEAD；只允许 http / https；跟随最多 5 次重定向，不发 authorization / cookie，HTML 转成文本；saveTo 可把正文原样存到文件）；认证用 auth 引用环境变量（值不外露），这类请求在完全权限下也会确认",
        writes: false,
        always: true,
        group: ToolGroup::Network,
    },
    ToolCard {
        name: "env",
        description: "一次性探出平台、node / python / git / cargo 等运行时版本与项目文件（package.json、Cargo.toml、.venv 等），只读不确认",
        writes: false,
        always: false,
        group: ToolGroup::Run,
    },
    ToolCard {
        name: "read_image",
        description: "读一张图片（PNG / JPEG / GIF / WebP / BMP）并把图片附给模型，用来检查截图与界面稿；只读不确认（上限 5 MB / 20 MP）",
        writes: false,
        always: false,
        group: ToolGroup::Network,
    },
    ToolCard {
        name: "screenshot",
        description: "截取屏幕（默认前台窗口，region=all 才整屏；x/y/width/height 四个都给时只截那块区域；长边缩到 1600px、存成 JPEG）并把图片附给模型，默认存到对话的 .dsh-flow/shots/；只读不确认",
        writes: false,
        always: false,
        group: ToolGroup::Network,
    },
    ToolCard {
        name: "browser",
        description: "无头浏览器（本机 Edge / Chrome，不下载浏览器）：open / goto 打开页面，snapshot 列出可点元素（[编号]），click / type / press / select 用真实输入事件操作，text / eval 读取，console / network 看日志与请求，screenshot 把画面附给模型，downloads 查看下载的文件；导航与输入类动作默认确认，纯读取不确认",
        writes: false,
        always: true,
        group: ToolGroup::Network,
    },
    ToolCard {
        name: "show",
        description: "把结果文件展示在对话里：这一步下面列出文件卡片，点一下就用系统程序打开，也能定位到所在文件夹；只列出已存在的文件，不读内容、不写入，一次最多 12 个",
        writes: false,
        always: false,
        group: ToolGroup::Files,
    },
    ToolCard {
        name: "mcp",
        description: "连接并调用 MCP 服务器：list 看 mcpserver 目录，link 按 id 或 command/url 连接，tools 看本次已连接，call 调用工具，disconnect 断开；连接在本次提问内保持",
        writes: false,
        always: true,
        group: ToolGroup::Interactive,
    },
    ToolCard {
        name: "ask_user_question",
        description: "向用户提一个带选项的问题并等待回答；关键方案拿不准时使用",
        writes: false,
        always: false,
        group: ToolGroup::Interactive,
    },
    ToolCard {
        name: "todo_write",
        description: "写入或替换本次对话的待办列表；开始多步任务时列计划，之后更新状态",
        writes: false,
        always: false,
        group: ToolGroup::Interactive,
    },
    ToolCard {
        name: "remember",
        description: "写入一条长期记忆（跨对话）；用户的偏好、项目固定约定、反复踩到的坑",
        writes: false,
        always: false,
        group: ToolGroup::Interactive,
    },
    ToolCard {
        name: "exit_plan_mode",
        description: "计划模式下提交完整计划，等待用户批准后开始执行",
        writes: false,
        always: false,
        group: ToolGroup::Interactive,
    },
];

#[cfg(test)]
mod tests {
    use super::{BUILTIN_TOOLS, ToolGroup, engine_description, shown_description};
    use crate::app::ALL_TOOLS;
    use std::collections::HashMap;

    /// The names in the catalogue, in order.
    fn names() -> Vec<&'static str> {
        BUILTIN_TOOLS.iter().map(|tool| tool.name).collect()
    }

    #[test]
    fn every_engine_tool_has_a_card() {
        let listed = names();
        for name in ALL_TOOLS {
            assert!(
                listed.contains(&name),
                "工具 {name} 没有出现在 src/app/tool_catalog.rs 的卡片里"
            );
        }
    }

    #[test]
    fn interaction_tools_and_the_gateway_have_cards_too() {
        let listed = names();
        for name in [
            "ask_user_question",
            "todo_write",
            "exit_plan_mode",
            "remember",
            "mcp",
        ] {
            assert!(listed.contains(&name), "缺少 {name} 的卡片");
        }
    }

    #[test]
    fn no_card_is_listed_twice_and_every_card_is_known() {
        let listed = names();
        for (index, name) in listed.iter().enumerate() {
            assert!(
                !listed[index + 1..].contains(name),
                "{name} 在工具目录里出现了两次"
            );
            assert!(
                ALL_TOOLS.contains(name) || is_extra(name),
                "{name} 既不在 ALL_TOOLS 里，也不是交互工具 / mcp 网关"
            );
        }
    }

    /// The four tools the engine adds on its own, plus the MCP gateway.
    fn is_extra(name: &str) -> bool {
        matches!(
            name,
            "ask_user_question" | "todo_write" | "exit_plan_mode" | "remember" | "mcp"
        )
    }

    #[test]
    fn a_card_shows_the_engines_description_once_it_is_known() {
        let card = BUILTIN_TOOLS
            .iter()
            .find(|tool| tool.name == "read")
            .expect("read card");
        let mut engine = HashMap::new();
        assert_eq!(shown_description(card, &engine), card.description);
        // The engine's text wins, on one line: the card clamps it to two lines and
        // offers the arrow when it does not fit.
        let schema = r#"{"type":"function","function":{"name":"read","description":"读文件：\n  带行号\n返回内容。"}}"#;
        let text = engine_description(schema).expect("description");
        assert_eq!(text, "读文件： 带行号 返回内容。");
        engine.insert("read".to_string(), text);
        assert_eq!(
            shown_description(card, &engine),
            "读文件： 带行号 返回内容。"
        );
        // A blank report is not a description: the fallback stays.
        engine.insert("read".to_string(), "   ".to_string());
        assert_eq!(shown_description(card, &engine), card.description);
        // A schema without text, or one that does not parse, yields nothing.
        assert_eq!(engine_description(r#"{"function":{"name":"read"}}"#), None);
        assert_eq!(engine_description("not json"), None);
    }

    /// Every card is drawn once, under exactly one section, with its own heading.
    #[test]
    fn every_section_has_cards_and_every_card_has_a_section() {
        let mut counted = 0;
        for group in ToolGroup::ALL {
            let cards = BUILTIN_TOOLS
                .iter()
                .filter(|tool| tool.group == group)
                .count();
            assert!(
                cards > 0,
                "分组「{}」没有卡片，页面上只会剩一个空标题",
                group.label()
            );
            counted += cards;
        }
        assert_eq!(counted, BUILTIN_TOOLS.len(), "有卡片不属于任何分组");
    }

    /// The headings are what a person reads, so two sections must not share one.
    #[test]
    fn section_headings_are_distinct() {
        let mut labels: Vec<&str> = ToolGroup::ALL.iter().map(|group| group.label()).collect();
        labels.sort_unstable();
        let shown = labels.len();
        labels.dedup();
        assert_eq!(labels.len(), shown);
    }

    /// Cards stay in the family a reader would look for them in.
    #[test]
    fn families_keep_the_cards_a_reader_expects() {
        let group_of = |name: &str| {
            BUILTIN_TOOLS
                .iter()
                .find(|tool| tool.name == name)
                .map(|tool| tool.group)
        };
        assert_eq!(group_of("write"), Some(ToolGroup::Files));
        assert_eq!(group_of("grep"), Some(ToolGroup::Search));
        assert_eq!(group_of("git_write"), Some(ToolGroup::VersionControl));
        assert_eq!(group_of("task"), Some(ToolGroup::Run));
        assert_eq!(group_of("screenshot"), Some(ToolGroup::Network));
        assert_eq!(group_of("browser"), Some(ToolGroup::Network));
        assert_eq!(group_of("ask_user_question"), Some(ToolGroup::Interactive));
    }
}
