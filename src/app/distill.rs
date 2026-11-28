//! 固化为工作流：输入框 + 菜单插入 `/固化为工作流`，发送后由 Agent 按 Skill 完成。
//!
//! 窗口自己不再生成草稿卡片：中间 `distill` 产物落在目标工作流的 `out/draft/`，
//! 只有 Agent 验证通过并把最终 `flow.yml` 写进 `<扫描根>/workflows/<name>/`，
//! 才会在左侧出现卡片；`.show` 标记让界面在回答结束时热加载并取消隐藏。
//!
//! 这个模块还保留确定性的换算/显示辅助（轮次列表、选择保持、热加载标记），
//! 旧的文件草稿实现留作 CLI/测试参考。
#![allow(dead_code)] // deterministic draft helpers are kept for the CLI, not the UI

use super::*;
use std::process::{Command, Stdio};

/// The composer command that starts a consolidation.
///
/// The `+` menu inserts this as one token; sending it sends the prepared
/// instruction to the assistant, so the transcript keeps the short command
/// rather than a wall of paths.
pub(crate) const WORKFLOW_COMMAND: &str = "/固化为工作流";

/// 一次固化的落点：读哪份聊天记录、写到哪、叫什么名字。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DistillPlan {
    /// 对话目录名（`chat-1`），同时是「这个对话正在固化」的键。
    pub chat_dir: String,
    /// 要读的 `chat.json`。
    pub chat_file: PathBuf,
    /// 取第几个提问（1 起）。
    pub turn: u32,
    /// `workflows/` 下的文件夹名。
    pub folder: String,
    /// 三份产物的目录。
    pub out_dir: PathBuf,
    /// 草稿清单里的 `name:`，也是工作流卡片上的标题。
    pub name: String,
}

/// 把对话标题或目录名变成可做文件夹名的 ASCII 片段。
///
/// 中文标题全部落成 `-` 时退回 `chat`，真正的可读名字留在清单的 `name:` 里。
/// @param value - 标题或目录名。
/// @returns 只含小写字母、数字、`-`、`_` 的片段。
fn safe_folder(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('-');
    if trimmed.is_empty() {
        "chat".to_string()
    } else {
        trimmed.to_string()
    }
}

/// 决定这次固化写到哪里，并避开已有的名字与文件夹。
///
/// 同一份对话再点一次不会覆盖上一份草稿：名字带轮次与序号（`（草稿·第1轮 2）`），
/// 文件夹带后缀（`draft-chat-1-t1-2`）。`taken` 是已发现的工作流名字，目录存在性再兜一层。
/// @param data_dir - `<home>/data`，聊天记录所在。
/// @param scan_root - 工作流扫描根（引擎的上一级）。
/// @param chat_dir - 对话目录名，例如 `chat-1`。
/// @param title - 对话标题，用作卡片名。
/// @param turn - 取第几个提问（1 起）。
/// @param taken - 已存在的工作流名字。
/// @returns 本次的落点。
pub(crate) fn distill_plan(
    data_dir: &Path,
    scan_root: &Path,
    chat_dir: &str,
    title: &str,
    turn: u32,
    taken: &[String],
) -> DistillPlan {
    let chat_file = data_dir
        .join(crate::chats::FOLDER)
        .join(chat_dir)
        .join("chat.json");
    let root = scan_root.join("workflows");
    let stem = format!("draft-{}-t{turn}", safe_folder(chat_dir));
    let readable = title.trim();
    let base_name = if readable.is_empty() {
        chat_dir.to_string()
    } else {
        readable.to_string()
    };

    for index in 1..=9_999usize {
        let folder = if index == 1 {
            stem.clone()
        } else {
            format!("{stem}-{index}")
        };
        let name = if index == 1 {
            format!("{base_name}（草稿·第{turn}轮）")
        } else {
            format!("{base_name}（草稿·第{turn}轮 {index}）")
        };
        let out_dir = root.join(&folder);
        if taken.iter().any(|existing| existing == &name) || out_dir.exists() {
            continue;
        }
        return DistillPlan {
            chat_dir: chat_dir.to_string(),
            chat_file,
            turn,
            folder,
            out_dir,
            name,
        };
    }
    // Practically unreachable; the epoch suffix still keeps the two keys unique.
    let folder = format!("{stem}-{}", util::now_ms());
    DistillPlan {
        chat_dir: chat_dir.to_string(),
        chat_file,
        turn,
        folder: folder.clone(),
        out_dir: root.join(folder),
        name: format!("{base_name}（草稿·第{turn}轮 {}）", util::now_ms()),
    }
}

/// 把 `distill` 写出的 `flow.draft.yml` 提升为可被发现的工作流清单。
///
/// 不覆盖已有 `flow.yml`：那说明这个文件夹不是一次性的草稿，宁可报错也不动它。
/// @param out_dir - 一次固化的目录。
/// @returns 成功，或一句能读的原因。
pub(crate) fn promote_draft(out_dir: &Path) -> Result<(), String> {
    let draft = out_dir.join("flow.draft.yml");
    if !draft.is_file() {
        return Err(format!(
            "distill 没有写出 {}（引擎版本可能还没有 distill 命令）",
            draft.display()
        ));
    }
    let flow = out_dir.join("flow.yml");
    if flow.exists() {
        return Err(format!("{} 已存在，拒绝覆盖", flow.display()));
    }
    std::fs::rename(&draft, &flow)
        .map_err(|error| format!("无法把草稿改名为 flow.yml：{error}"))
}

/// 在 worker 线程里跑一次 `node src/cli.ts distill …`。
///
/// 不做网络调用，也不读密钥；只是读 `chat.json`、写三份文件。失败时把引擎自己
/// 的错误行带回来，而不是让界面猜。
/// @param engine_dir - `dsh-flow` 目录。
/// @param plan - 本次落点。
/// @returns 成功，或引擎的失败原因。
fn run_distill(engine_dir: &Path, plan: &DistillPlan) -> Result<(), String> {
    let cli = engine_dir.join("src").join("cli.ts");
    if !cli.is_file() {
        return Err(format!(
            "找不到引擎：{}\n设置 DSH_FLOW_HOME 指向 dsh-flow 目录后重启界面。",
            cli.display()
        ));
    }
    if !plan.chat_file.is_file() {
        return Err(format!(
            "聊天记录还没有落盘：{}",
            plan.chat_file.display()
        ));
    }

    let mut command = Command::new("node");
    command
        .arg(&cli)
        .arg("distill")
        .arg(&plan.chat_file)
        .arg("--out")
        .arg(&plan.out_dir)
        .arg("--name")
        .arg(&plan.name)
        .arg("--turn")
        .arg(plan.turn.to_string())
        .current_dir(engine_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }

    let output = command
        .output()
        .map_err(|error| format!("无法启动 node：{error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = first_line(&stderr).or_else(|| first_line(&stdout)).unwrap_or("没有说明");
        return Err(format!(
            "distill 失败（退出码 {}）：{detail}",
            output.status.code().unwrap_or(-1)
        ));
    }
    promote_draft(&plan.out_dir)
}

/// 第一行非空文本，用来把引擎的堆栈收成一句话。
/// @param text - 原始输出。
/// @returns 第一行，或 `None`。
fn first_line(text: &str) -> Option<&str> {
    text.lines().map(str::trim).find(|line| !line.is_empty())
}

/// 一轮可以固化的提问：序号、这一轮的工具调用数、问题的第一行。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DistillTurn {
    /// 第几个提问，1 起，和 `distill --turn` 一致。
    pub turn: u32,
    /// 这一轮里工具行的数量；0 的轮次不列出来。
    pub tools: usize,
    /// 问题第一行，用来让人认出是哪一轮。
    pub question: String,
}

/// 列出有工具调用的提问轮次，供「固化为工作流」选择。
///
/// 长对话往往有几十轮，只保留最近的 `limit` 轮：再早的实现多半已经被后面的
/// 修改覆盖，要取更早的轮次可以用命令行 `--turn`。
/// @param messages - 对话的消息，按时间顺序。
/// @param limit - 最多返回多少轮（取最近的）。
/// @returns 每轮一行，按时间顺序。
pub(crate) fn distill_turns(messages: &[ChatMessage], limit: usize) -> Vec<DistillTurn> {
    let mut turns: Vec<DistillTurn> = Vec::new();
    let mut turn = 0u32;
    let mut tools = 0usize;
    let mut question = String::new();
    for message in messages {
        if message.role == Role::User {
            if turn > 0 && tools > 0 {
                turns.push(DistillTurn {
                    turn,
                    tools,
                    question,
                });
            }
            turn += 1;
            tools = 0;
            question = short_line(&message.text, 48);
        } else if message.role == Role::Tool {
            tools += 1;
        }
    }
    if turn > 0 && tools > 0 {
        turns.push(DistillTurn {
            turn,
            tools,
            question,
        });
    }
    if turns.len() > limit {
        turns.split_off(turns.len() - limit)
    } else {
        turns
    }
}

/// 取文本第一行并截断到 `limit` 个字符（按字符，不按字节）。
/// @param text - 原始文本。
/// @param limit - 最多字符数。
/// @returns 一行说明。
fn short_line(text: &str, limit: usize) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    let count = line.chars().count();
    if count <= limit {
        line.to_string()
    } else {
        format!(
            "{}…",
            line.chars().take(limit.saturating_sub(1)).collect::<String>()
        )
    }
}

/// 重新扫描后保持阅读者的选中项：列表按名字排序，新增一张卡片会让下标整体移动，
/// 名字不会。原来选中的工作流若已不在列表里，选中清空。
/// @param selected - 扫描前选中的名字。
/// @param after - 扫描后的列表。
/// @returns 扫描后的下标。
pub(crate) fn keep_selection(selected: Option<&str>, after: &[WorkflowEntry]) -> Option<usize> {
    let name = selected?;
    after.iter().position(|entry| entry.name == name)
}

/// Show the workflows that asked to be shown.
///
/// A workflow written by the assistant is not necessarily new: it may carry a
/// name that was hidden earlier ("从工作区移除"), and a hidden name keeps the
/// fresh card off the list. The assistant therefore drops an empty `.show` file
/// in the folder when it finishes; this clears the hidden flag and removes the
/// marker. Running it at startup as well means a card created in the previous
/// session is not lost.
/// @param workflows - manifests found on disk.
/// @param hidden - hidden names, updated in place.
/// @returns true when something changed and the store should be saved.
pub(crate) fn apply_show_markers(workflows: &[WorkflowEntry], hidden: &mut Vec<String>) -> bool {
    let mut changed = false;
    for entry in workflows {
        let Some(folder) = entry.path.parent() else {
            continue;
        };
        let marker = folder.join(".show");
        if !marker.is_file() {
            continue;
        }
        let before = hidden.len();
        hidden.retain(|name| name != &entry.name);
        if hidden.len() != before {
            changed = true;
        }
        let _ = std::fs::remove_file(&marker);
    }
    changed
}

/// A suggested canonical name for a workflow built from one turn.
/// @param title - conversation title.
/// @param chat_dir - conversation folder name.
/// @param turn - which question.
/// @returns an ASCII name the agent may use or replace.
fn suggested_workflow_name(title: &str, chat_dir: &str, turn: u32) -> String {
    let slug: String = title
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = slug.trim_matches('-');
    if trimmed.is_empty() {
        format!("{chat_dir}-t{turn}")
    } else {
        format!("{trimmed}-t{turn}")
    }
}

/// A suggested canonical name when the assistant picks the turn itself.
/// @param title - conversation title.
/// @param chat_dir - conversation folder name.
/// @returns an ASCII name, e.g. `units-export-workflow`.
fn suggested_workflow_name_auto(title: &str, chat_dir: &str) -> String {
    let slug: String = title
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = slug.trim_matches('-');
    if trimmed.is_empty() {
        format!("{chat_dir}-workflow")
    } else {
        format!("{trimmed}-workflow")
    }
}

impl DshFlowApp {
    /// Put `/固化为工作流` into an empty composer as one command token.
    ///
    /// The command is not sent until the reader presses Enter; a Backspace at
    /// its end clears the whole token (see `handle_field_key`). The menu closes
    /// and the caret goes to the composer.
    pub(crate) fn insert_workflow_command(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.attach_menu = false;
        self.distill_menu = false;
        if !self.composer.text.trim().is_empty() {
            self.show_toast("输入框里已有内容：先发送或清空", true, cx);
            return;
        }
        self.composer.set_text(WORKFLOW_COMMAND);
        self.focused = Some(Field::Composer);
        window.focus(&self.composer.focus_handle().clone(), cx);
        self.start_caret_blink(cx);
        cx.notify();
    }

    /// The instruction the command expands to before it reaches the model.
    ///
    /// The turn is chosen by the assistant (the business run, not a later
    /// "继续"); the transcript shows only `/固化为工作流`.
    pub(crate) fn workflow_command_prompt(&self) -> String {
        let chat = self.chat();
        let chat_dir = chat.dir.clone();
        let title = chat.title.clone();
        let chat_file = self
            .ai
            .data_dir()
            .join(crate::chats::FOLDER)
            .join(&chat_dir)
            .join("chat.json");
        let scan_root = self.scan_root.display().to_string();
        let chat_workspace = self.chat_dir(self.active_chat).display().to_string();
        let engine_cli = self
            .engine_dir
            .join("src")
            .join("cli.ts")
            .display()
            .to_string();
        let suggested = suggested_workflow_name_auto(&title, &chat_dir);
        format!(
            "请按 Skill「工作流固化」固化当前对话，直接产出可运行的工作流；不要生成草稿卡片。\n\
             先看聊天记录，自动选择真正执行过业务流程的一轮（不是「继续」/「再改一下」）；分不清就先问我。\n\n\
             聊天记录：{chat_file}\n\
             扫描根：{scan_root}\n\
             当前对话工作区：{chat_workspace}\n\
             建议 name：{suggested}（不合适可换更贴切的英文名；同一 name 只允许一份）\n\n\
             流程要求：\n\
             1. 运行 `node \"{engine_cli}\" distill \"{chat_file}\" --out \"{scan_root}/workflows/{suggested}/out/draft\" --name \"{suggested}\" --turn <选中轮次>`；中间产物只放在 `out/draft/` 下（out 不会被工作区扫描）。\n\
             2. 按 Skill 把 `skip` 占位实现成 `steps/` 脚本，补 `tests/` 或 `fixtures/`；不要一步一个 eval。\n\
             3. 先在 `{scan_root}/workflows` 下找同名清单，有就改它，没有才新建；禁止在 `dsh-flow/workflows` 或其它位置另建副本。\n\
             4. `validate` / `test` / `explain` 全过；如果清单有验证码/登录，**live run 留给窗口「运行」**（shell 没有接入点 Key），不要以它作为固化是否完成的条件。\n\
             5. 写好后在同一文件夹创建空文件 `.show`（PowerShell: `New-Item -ItemType File -Force .show`），界面会在回答结束重扫并显示卡片。\n\
             6. 完成后汇报：路径、验证结果、仍需人工确认的部分。\n\n\
             验证与登录：\n\
             - 清单若有 `workspace` / `browserDir` 输入，验证时指向当前对话工作区 `{chat_workspace}`；probe 已登录会跳过 captcha / login / read，不需要 key。\n\
             - 如果 probe 说未登录、而 shell 没有接入点 Key（no api key）：不要在 shell 里反复跑 login / captcha；停下列出两条路：去「设置 → AI」填 Key 并测试，或用窗口工作流页的「运行」（它会把凭据送给引擎），或先在对话里把浏览器登录再重跑。",
            chat_file = chat_file.display(),
            scan_root = scan_root,
            chat_workspace = chat_workspace,
            engine_cli = engine_cli,
            suggested = suggested,
        )
    }
    ///
    /// Called after an answer: the assistant may have written or updated a
    /// workflow while it worked, and a card that only appears after a restart
    /// reads as "the workflow was never made".
    pub(crate) fn refresh_workflows(&mut self) {
        let selected = self
            .selected_flow
            .and_then(|index| self.workflows.get(index))
            .map(|entry| entry.name.clone());
        self.workflows = engine::discover(&self.scan_root);
        self.selected_flow = keep_selection(selected.as_deref(), &self.workflows);
        let hidden_before = self.hidden_workflows.len();
        self.hidden_workflows
            .retain(|name| self.workflows.iter().any(|entry| &entry.name == name));
        let labels_before = self.workflow_labels.len();
        self.workflow_labels
            .retain(|name, _| self.workflows.iter().any(|entry| &entry.name == name));
        let cleared_before = self.flow_cleared.len();
        if !self.workflows.is_empty() {
            self.flow_cleared
                .retain(|name| self.workflows.iter().any(|entry| &entry.name == name));
        }
        let shown = apply_show_markers(&self.workflows, &mut self.hidden_workflows);
        if shown
            || self.hidden_workflows.len() != hidden_before
            || self.workflow_labels.len() != labels_before
            || self.flow_cleared.len() != cleared_before
        {
            self.save_categories();
        }
    }

    /// 「固化为工作流」：把整件事交给助手，按 Skill 一次做完。
    ///
    /// 菜单只负责选一轮；中间 `distill` 产物落在 `out/draft/`（不被扫描），
    /// **不会**出现草稿卡片。助手实现步骤并验证通过后，才把最终清单写进
    /// `workflows/<name>/`；回答结束时 `refresh_workflows` 让卡片出现。
    /// 没有可验证的模型时退回确定性草稿（`distill_workflow`），至少给人一份依据。
    pub(crate) fn solidify_with_agent(&mut self, turn: u32, cx: &mut Context<Self>) {
        self.distill_menu = false;
        self.attach_menu = false;
        if self.chat_busy(self.active_chat) {
            self.show_toast("回答还在进行，完成后再固化", true, cx);
            return;
        }
        let Some(chat) = self.chats.get(self.active_chat) else {
            return;
        };
        let choice = distill_turns(&chat.messages, usize::MAX)
            .into_iter()
            .find(|candidate| candidate.turn == turn);
        let Some(choice) = choice else {
            self.show_toast(format!("第 {turn} 轮不存在"), false, cx);
            return;
        };
        if choice.tools == 0 {
            self.show_toast(
                format!("第 {turn} 轮没有工具调用，换一轮再固化"),
                false,
                cx,
            );
            return;
        }
        if !self.composer.text.trim().is_empty() {
            self.show_toast("输入框里有内容：先发送或清空，再让 Agent 固化", true, cx);
            return;
        }
        if !self.ai.active().is_some_and(|entry| entry.verified) {
            self.show_toast(
                "没有可用模型：先在「设置 → AI」接好一个可用模型，再用 + 菜单固化",
                true,
                cx,
            );
            return;
        }

        let chat_dir = chat.dir.clone();
        let title = chat.title.clone();
        let chat_file = self
            .ai
            .data_dir()
            .join(crate::chats::FOLDER)
            .join(&chat_dir)
            .join("chat.json");
        let scan_root = self.scan_root.display().to_string();
        let engine_cli = self
            .engine_dir
            .join("src")
            .join("cli.ts")
            .display()
            .to_string();
        let suggested = suggested_workflow_name(&title, &chat_dir, turn);
        let prompt = format!(
            "请按 Skill「工作流固化」把当前对话的第 {turn} 轮固化成一个可运行的工作流；不要生成草稿卡片。\n\n\
             聊天记录：{chat_file}\n\
             扫描根：{scan_root}\n\
             建议 name：{suggested}（不合适可换更贴切的英文名；同一 name 只允许一份）\n\n\
             流程要求：\n\
             1. 先运行 `node \"{engine_cli}\" distill \"{chat_file}\" --out \"{scan_root}/workflows/{suggested}/out/draft\" --name \"{suggested}\" --turn {turn}`；中间产物只放在该 `out/draft/` 下（out 不会被工作区扫描），不要动工作区卡片。\n\
             2. 按 Skill 把 `skip` 占位实现成 `steps/` 脚本，补 `tests/` 或 `fixtures/`；不要一步一个 eval。\n\
             3. 先在 `{scan_root}/workflows` 下找同名清单，有就改它，没有才新建；禁止在 `dsh-flow/workflows` 或其它位置另建副本。\n\
             4. `validate` / `test` / `run` / `explain` 全过之后，才把最终的 `flow.yml` 写到 `{scan_root}/workflows/{suggested}/`。\n\
             5. 写好后在同一文件夹创建一个空文件 `.show`（PowerShell: `New-Item -ItemType File -Force .show`）；界面会在回答结束重扫，把它从隐藏中恢复并显示卡片。\n\
             6. 完成后用一句话汇报：路径、验证结果、仍需人工确认的部分。",
            chat_file = chat_file.display(),
            scan_root = scan_root,
            engine_cli = engine_cli,
            suggested = suggested,
            turn = turn,
        );
        self.composer.set_text(prompt);
        self.send_message(cx);
    }

    /// 「固化为工作流」（确定性回退）：整理当前对话，生成一份草稿清单。
    ///
    /// 与「压缩上下文」同一套交互：菜单立刻关闭，能说明的原因用 toast 说，
    /// 真正的整理在工作线程里跑。回答进行中不做，因为要读的是刚落盘的记录。
    pub(crate) fn distill_workflow(&mut self, turn: u32, cx: &mut Context<Self>) {
        self.distill_menu = false;
        self.attach_menu = false;
        let index = self.active_chat;
        if self.chat_busy(index) {
            self.show_toast("回答还在进行，完成后再固化为工作流", true, cx);
            return;
        }
        let Some(chat) = self.chats.get(index) else {
            return;
        };
        let tools = distill_turns(&chat.messages, usize::MAX)
            .iter()
            .find(|candidate| candidate.turn == turn)
            .map_or(0, |candidate| candidate.tools);
        let chat_dir = chat.dir.clone();
        let title = chat.title.clone();
        if tools == 0 {
            self.show_toast(
                format!("第 {turn} 轮没有工具调用，换一轮再固化"),
                false,
                cx,
            );
            return;
        }
        if self.distilling.contains(&chat_dir) {
            self.show_toast("这个对话正在固化中", false, cx);
            return;
        }
        if !self.engine_ready {
            self.show_toast("找不到引擎，先修好 dsh-flow 的路径", true, cx);
            return;
        }
        // The worker reads `chat.json`, so what is on screen has to be on disk
        // first; the save is synchronous and keeps the same transcript.
        self.save_chats();
        let taken: Vec<String> = self
            .workflows
            .iter()
            .map(|entry| entry.name.clone())
            .collect();
        let plan = distill_plan(
            &self.ai.data_dir(),
            &self.scan_root,
            &chat_dir,
            &title,
            turn,
            &taken,
        );
        self.distilling.insert(chat_dir.clone());

        let engine_dir = self.engine_dir.clone();
        let worker_plan = plan.clone();
        let (sender, result) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(run_distill(&engine_dir, &worker_plan));
        });

        let watched = chat_dir.clone();
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(120))
                    .await;
                match result.try_recv() {
                    Ok(answer) => {
                        this.update(cx, |view, cx| {
                            view.finish_distill(&plan, answer, cx);
                            cx.notify();
                        })
                        .ok();
                        break;
                    }
                    // The worker died without answering; clearing the flag is what
                    // lets this conversation be distilled again.
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        this.update(cx, |view, cx| {
                            view.distilling.remove(&watched);
                            view.show_toast("固化进程意外结束，没有生成草稿", true, cx);
                            cx.notify();
                        })
                        .ok();
                        break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {}
                }
                let alive = this
                    .update(cx, |view, _| view.distilling.contains(&watched))
                    .unwrap_or(false);
                if !alive {
                    break;
                }
            }
        })
        .detach();
        self.show_toast(format!("正在整理第 {turn} 轮，生成工作流草稿…"), false, cx);
    }

    /// 记录一次固化的结果：刷新工作流列表，并在对话里留下可读的说明。
    ///
    /// 成功与失败都写 `Note`（不会被回放给模型）：toast 会消失，草稿路径和失败原因
    /// 不应该跟着消失。
    /// @param plan - 本次落点。
    /// @param answer - worker 的结果。
    /// @param cx - window context.
    fn finish_distill(
        &mut self,
        plan: &DistillPlan,
        answer: Result<(), String>,
        cx: &mut Context<Self>,
    ) {
        self.distilling.remove(&plan.chat_dir);
        let turn = plan.turn;
        match answer {
            Err(error) => {
                if let Some(chat) = self
                    .chats
                    .iter_mut()
                    .find(|chat| chat.dir == plan.chat_dir)
                {
                    chat.messages.push(ChatMessage::note(
                        format!("固化为工作流失败：{error}"),
                        true,
                    ));
                    chat.touch();
                }
                self.save_chats();
                self.show_toast(format!("固化失败：{error}"), true, cx);
            }
            Ok(()) => {
                let manifest = plan.out_dir.join("flow.yml");
                let selected_name = self
                    .selected_flow
                    .and_then(|index| self.workflows.get(index))
                    .map(|entry| entry.name.clone());
                self.workflows = engine::discover(&self.scan_root);
                self.selected_flow = keep_selection(selected_name.as_deref(), &self.workflows);
                let entry = self
                    .workflows
                    .iter()
                    .find(|entry| entry.path == manifest);
                let steps = entry.map_or(0, |entry| entry.spec.steps.len());
                let placeholders = entry.map_or(0, |entry| {
                    entry
                        .spec
                        .steps
                        .iter()
                        .filter(|step| {
                            step.notes
                                .iter()
                                .any(|note| note == "skip" || note.starts_with("skip "))
                        })
                        .count()
                });
                let label = manifest
                    .strip_prefix(&self.scan_root)
                    .unwrap_or(&manifest)
                    .display()
                    .to_string();
                let report = plan
                    .out_dir
                    .join("distill.md")
                    .strip_prefix(&self.scan_root)
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|_| plan.out_dir.join("distill.md").display().to_string());
                let note = if steps == 0 {
                    format!(
                        "已生成工作流草稿（第 {turn} 轮）：{label}。草稿不能直接运行；先按 Skill「工作流固化」实现占位步骤，再 validate/test/run。左侧工作区会出现「{}」卡片。报告：{report}",
                        plan.name
                    )
                } else {
                    format!(
                        "已生成工作流草稿（第 {turn} 轮）：{label}（{steps} 步，其中 {placeholders} 个待实现占位）。草稿不能直接运行；先按 Skill「工作流固化」替换占位步骤，再 validate/test/run。左侧工作区会出现「{}」卡片。报告：{report}",
                        plan.name
                    )
                };
                if let Some(chat) = self
                    .chats
                    .iter_mut()
                    .find(|chat| chat.dir == plan.chat_dir)
                {
                    chat.messages.push(ChatMessage::note(note, false));
                    chat.touch();
                }
                self.save_chats();
                let toast = if steps == 0 {
                    format!("已生成工作流草稿（第 {turn} 轮）：{label}")
                } else {
                    format!("已生成工作流草稿（第 {turn} 轮）：{label}（{steps} 步）")
                };
                self.show_toast(toast, false, cx);
            }
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A clean temporary root for one test.
    /// @param label - test-specific suffix.
    /// @returns the directory, which the test removes at the end.
    fn temp_root(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dsh-distill-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_chinese_title_keeps_a_readable_name_and_an_ascii_folder() {
        let root = temp_root("name");
        let plan = distill_plan(&root.join("data"), &root, "chat-1", "使用单位导出", 1, &[]);
        assert_eq!(plan.name, "使用单位导出（草稿·第1轮）");
        assert_eq!(plan.folder, "draft-chat-1-t1");
        assert_eq!(plan.turn, 1);
        assert_eq!(
            plan.chat_file,
            root.join("data")
                .join(crate::chats::FOLDER)
                .join("chat-1")
                .join("chat.json")
        );
        assert_eq!(plan.out_dir, root.join("workflows").join("draft-chat-1-t1"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_second_draft_never_overwrites_the_first() {
        let root = temp_root("again");
        let taken = vec!["新对话（草稿·第1轮）".to_string()];
        let plan = distill_plan(&root.join("data"), &root, "chat-1", "新对话", 1, &taken);
        assert_eq!(plan.folder, "draft-chat-1-t1-2");
        assert_eq!(plan.name, "新对话（草稿·第1轮 2）");

        // A folder left on disk is enough to make the next one step aside, even
        // when no workflow card carries its name (say, it could not be parsed).
        std::fs::create_dir_all(root.join("workflows").join("draft-chat-1-t1")).unwrap();
        let from_disk = distill_plan(&root.join("data"), &root, "chat-1", "新对话", 1, &[]);
        assert_eq!(from_disk.folder, "draft-chat-1-t1-2");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn lists_only_turns_that_ran_tools() {
        let messages = vec![
            ChatMessage::user("第一轮"),
            ChatMessage::tool("read · a", false),
            ChatMessage::user("第二轮"),
            ChatMessage::assistant("没有工具"),
            ChatMessage::user("第三轮"),
            ChatMessage::tool("shell · a", false),
            ChatMessage::tool("shell · b", false),
        ];
        let turns = distill_turns(&messages, 10);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].turn, 1);
        assert_eq!(turns[0].tools, 1);
        assert_eq!(turns[0].question, "第一轮");
        assert_eq!(turns[1].turn, 3);
        assert_eq!(turns[1].tools, 2);
        assert_eq!(turns[1].question, "第三轮");
    }

    #[test]
    fn keeps_the_newest_turns_when_the_chat_is_long() {
        let mut messages = Vec::new();
        for turn in 1..=12 {
            messages.push(ChatMessage::user(format!("第 {turn} 轮")));
            messages.push(ChatMessage::tool("read · a", false));
        }
        let turns = distill_turns(&messages, 8);
        assert_eq!(turns.len(), 8);
        assert_eq!(turns[0].turn, 5);
        assert_eq!(turns[7].turn, 12);
    }

    #[test]
    fn a_show_marker_unhides_a_workflow_and_is_removed() {
        let root = temp_root("show");
        let dir = root.join("workflows").join("units-export");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("flow.yml"), "name: maers-units-export\n").unwrap();
        std::fs::write(dir.join(".show"), "").unwrap();
        let entry = WorkflowEntry {
            name: "maers-units-export".to_string(),
            description: String::new(),
            path: dir.join("flow.yml"),
            spec: ManifestSpec::default(),
        };
        let mut hidden = vec!["maers-units-export".to_string(), "other".to_string()];
        assert!(apply_show_markers(&[entry], &mut hidden));
        assert_eq!(hidden, vec!["other".to_string()]);
        assert!(!dir.join(".show").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_rescan_keeps_the_selection_by_name() {
        let workflow = |name: &str| WorkflowEntry {
            name: name.to_string(),
            description: String::new(),
            path: PathBuf::from(format!("/w/{name}/flow.yml")),
            spec: ManifestSpec::default(),
        };
        // A new draft sorts before `units-export`, so index 0 is now the draft.
        let after = vec![workflow("draft-chat-1"), workflow("units-export")];
        assert_eq!(keep_selection(Some("units-export"), &after), Some(1));
        assert_eq!(keep_selection(Some("gone"), &after), None);
        assert_eq!(keep_selection(None, &after), None);
    }

    #[test]
    fn a_promoted_draft_is_discovered_as_a_workflow() {
        let root = temp_root("discover");
        let dir = root.join("workflows").join("draft-chat-1");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("flow.yml"),
            "name: \"新对话（草稿）\"\nversion: 1\nsteps:\n  - id: todo\n    skip: true\n    set:\n      todo: x\n",
        )
        .unwrap();
        let found = crate::engine::discover(&root);
        let entry = found
            .iter()
            .find(|entry| entry.name == "新对话（草稿）")
            .expect("the promoted draft is discovered");
        assert_eq!(entry.spec.steps.len(), 1);
        assert!(entry.spec.steps[0]
            .notes
            .iter()
            .any(|note| note == "skip" || note.starts_with("skip ")));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn promoting_a_draft_renames_it_and_refuses_to_clobber() {
        let root = temp_root("promote");
        std::fs::write(root.join("flow.draft.yml"), "name: x\n").unwrap();
        promote_draft(&root).unwrap();
        assert!(root.join("flow.yml").is_file());
        assert!(!root.join("flow.draft.yml").exists());

        std::fs::write(root.join("flow.draft.yml"), "name: y\n").unwrap();
        assert!(promote_draft(&root).unwrap_err().contains("已存在"));

        let empty = root.join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        assert!(promote_draft(&empty).unwrap_err().contains("flow.draft.yml"));
        let _ = std::fs::remove_dir_all(&root);
    }
}