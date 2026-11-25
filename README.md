# PAgent

一个 Windows 桌面 AI 助手：GPUI 原生界面 + 可独立运行的 `dsh-flow` TypeScript 引擎。

## 组成

- `src/` — Rust/GPUI 界面：对话、上下文、统计、后台任务、工作流、插件、MCP、Skill、设置。
- `dsh-flow/` — 工作流与 AI 请求引擎：清单执行、工具循环、MCP/插件、压缩、CLI。
- `assets/` — 图标与内嵌资源。
- `backups/` — 手动备份产物（不纳入版本控制）。
- `dist/` — 构建产物与便携发布目录（不纳入版本控制）。

## 环境

- Windows 10/11
- Rust stable（项目使用 edition 2024）
- Node.js >= 23.6（引擎要求，TypeScript 直接运行）
- 可选：Python / VB 等被工作流步骤调用的运行时

## 构建与运行

```powershell
cargo build --release
.\target\release\dsh-flow-ui.exe
```

开发构建（依赖已按 release 优化）：

```powershell
cargo build
.\target\debug\dsh-flow-ui.exe
```

## 引擎

```powershell
cd dsh-flow
npm install          # 仅开发依赖，运行引擎本身没有依赖
npm run check        # tsc --noEmit
npm test             # node --test tests/
node src/cli.ts validate examples/repo-health/flow.yml
node src/cli.ts run examples/repo-health/flow.yml --input repo=.
```

界面查找引擎的顺序：`DSH_FLOW_HOME` → 项目/可执行文件附近的 `dsh-flow/` → 当前目录下的 `dsh-flow/`。便携发布包会把引擎放在 exe 旁边。

## 模型与思考

设置 → AI 里按连接保存的四项（温度、思考、最大输出、上下文窗口）都是**滑杆**，右侧显示当前值，旁边的「恢复默认」把它交回端点自己的设置。

DeepSeek 预设提供 `deepseek-flash` / `deepseek-v4-pro`：两者都支持思考模式，默认开启，力度默认 `high`。

- 设置 → AI → 连接卡片的「思考」：
  - **默认**：不发送任何思考字段，沿用端点自己的设置（DeepSeek 即“开、力度高”）；
  - **关闭**：请求体带 `{"thinking": {"type": "disabled"}}`，更快更省，也不会出现「思考」行；
  - **低 / 高 / 最高**：请求体带 `reasoning_effort`（`low` / `high` / `max`）。
- 思考开着时端点会忽略「温度」（DeepSeek 明确如此）；想用温度调风格，先在设置里把思考滑到「关闭」。
- 对话里的思考按轮显示成可折叠的「思考」行；上下文页的「推理过程」也按轮归属，可在轮次下拉里逐轮查看。
- 选择随连接保存在 `config.json` 的 `reasoning`；其它 OpenAI 兼容端点不认识的字段会被忽略。
- 设置 → AI → 连接卡片的「上下文窗口」滑杆：默认按模型名估算（内置表，未知模型按 128k），也可以拖到 32k / 128k / 256k / 1M —— 引擎按它的 80% 触发压缩，标题下的窗口占用条也用同一个数值，两边不会各算各的。旁边的「恢复默认」清掉这个覆盖。

### 联网搜索

- **默认开启，设置里没有这个开关了**：只要端点提供这些工具（`deepseek-flash`）就一律声明，其它模型不发送，避免请求被拒。
- 想为某个对话关掉它：输入框「+」菜单 →「本次对话」→「联网搜索」开关（关闭只影响这一个对话，可选「回到默认（开启）」）。
- 开启后请求的 `tools` 里会带上 DeepSeek 的服务端工具 `deepseek_web_search` 与 `deepseek_read_url`：**搜什么、打开哪一页由模型自己决定，搜索和抓取都在 DeepSeek 服务器上执行**，本机不执行任何工具，结果直接进入模型上下文。
- 对话里每次搜索 / 读页是一行「web_search」/「read_url」：搜索开始就出现在对话里（不必等回答），展开后每个来源一行、点「打开」用默认浏览器打开；上下文页「动作」卡把它归到「网络」。
- 搜索次数会进统计账本，统计页出现「联网搜索」卡片（有搜索才显示）；已读页面随助手段落保存在对话里，追问时一并回传，不会丢。
- 思考开着时是深度搜索（多步搜索、翻页、页内查找），思考关闭时是一次搜索后带引用回答；费用按 token 计（2026-11-18 起模型调用触发的搜索不再单独收费）。

## 计划、待办与提问

- 输入框「+」菜单里的「计划模式」开关：开启后本次提问在计划模式下进行 —— 模型先只读调查，把完整计划提交到聊天里的计划卡；点「批准并执行」后才开始改动，批准后计划模式自动关闭；点「继续修改」则让模型修订后再次提交。
- 模型可以调用 `todo_write` 写入本次对话的待办列表。待办卡在对话标题下方，显示完成数 / 总数，点开可看每一项的状态；列表随对话保存，重开仍在。
- 模型可以调用 `ask_user_question` 提一个带 2–6 个选项的问题。聊天里出现选项卡，点一个选项即回答，模型收到「用户选择：…」后继续。
- 「+」菜单里的「自动审查」开关（「工作区内修改」下）在「逐次询问 / 自动审查」之间切换。
- 输入框只留「+」、权限、模型、发送四个控件；其余按对话生效的设置都在「+」菜单的「本次对话」里，一项一个开关或一排选项：计划模式、自动审查、联网搜索（默认开启，可在本对话关掉，并可「回到默认」）、轮数上限（6 / 12 / 24）、工具调用上限（20 / 40 / 80）。这些都随对话保存进 `chat.json`。自动审查只放行普通的工作区内写入和只读命令（`git status/diff/log/show`、`cargo check/test/clippy/build`、`dir/ls/type`）；含 `&&`、`|`、`>` 的命令、删除、发布、插件 / MCP 调用仍然逐次询问。轮数与工具调用默认取上限（24 轮 / 80 次工具调用）。

## 权限与沙箱

- 「仅可查看」和「工作区内修改」下，`shell` / `python` / `node` 以及后台任务通过程序旁边的 `dsh-sandbox.exe` 启动：Windows 低完整性令牌 + Job Object，读全盘、网络照常，写到工作区外的位置会被系统拒绝；停止 / 关闭会结束整棵进程树。
- 工作区模式把当前对话目录设为低完整性的可写区域；仅可查看模式只给私有临时目录写权限，工作区本身保持不可写（从工作区模式切回来时会恢复）。
- 「完全权限」不走沙箱。`dsh-sandbox.exe` 缺失或 `--check` 不通过时自动退回原有的路径检查 + 审批卡；若系统在真正启动命令时才拒绝受限令牌，命令会明确报 `SANDBOX ERROR`，不会静默放行。
- 这是“系统级写隔离”，不是 AppContainer：它不隐藏磁盘其他位置，也不拦网络。自动审查假定工作区代码可信；不可信仓库仍建议逐次询问。

## 外部插件

插件放在 `<home>/plugins/<id>/`，入口由 `plugin.json` 的 `main` 指定，必须导出 `activate(api)`。设置页的“模式”决定每次对话向模型发送哪些外部插件、Skill 和 MCP：基础模式不带外部资源，全量模式带所有手动启用的资源，自定义模式按三个列表勾选；在各自页面关闭的资源始终不会发送。仓库里的 `plugins/win32-windows/` 是只读 Win32 窗口检查插件：

- 需要 Python 在 PATH 中；
- 可调用 `win32_list_windows` / `win32_window_info` 枚举 HWND；
- `win32_observe` 用 UI Automation 读取控件树，返回元素编号、名称、类型、中心坐标、密码框标记和 45 秒快照；
- 只读，不点击、不修改。

另有 `plugins/office/`：`office_open` / `office_status` / `office_read` / `office_edit` / `office_save` / `office_close` / `office_help`，通过 Windows Office COM 直接操作已打开的 Word/Excel（Word 段落、表格、样式；Excel 单元格、公式、格式、工作表与图表），支持 `dry_run`、保存/另存为/PDF 和受限制的关闭。需要本机安装 Microsoft Office 或 WPS，默认不启用。

另有 `plugins/win32-input/`：`win32_focus` / `win32_click` / `win32_scroll` / `win32_type` / `win32_keys` / `win32_drag` / `win32_wait`。优先接收 `win32_observe` 的 `snapshot` + `element`，拒绝过期快照和密码框输入；默认不启用，所有操作都需审批，并支持 `dry_run` 先验证不执行。

## 测试

```powershell
cargo test --release        # UI/核心逻辑
cargo clippy --release --all-targets --message-format=short
cd dsh-flow; npm test       # 引擎
```

## 快捷键

输入框以下按窗口处理（`Esc` 关闭弹层、查找、选择，再按一次回到对话）：

| 快捷键 | 作用 |
| --- | --- |
| `Ctrl+N` | 新建对话 |
| `Ctrl+F` | 在当前对话里查找 |
| `PageUp` / `PageDown` | 向上 / 向下翻一屏 |
| `Ctrl+Home` / `Ctrl+End` | 跳到最早 / 最新一条 |
| `Ctrl+A` / `Ctrl+C` | 全选对话 / 复制选中内容（输入框里则是全选文字 / 复制文字） |
| `Ctrl+T` | 切换深色 / 浅色主题 |
| `Ctrl+R` / `Ctrl+L` | 运行工作流 / 清空运行日志（工作流页） |
| `Ctrl+/` 或 `F1` | 打开 / 关闭快捷键表（标题栏最小化按钮旁的「?」也能打开同一张表） |

## 发布

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\release.ps1
```

脚本会：

1. 跑 Rust 测试和引擎检查/测试（可用 `-SkipTests` 跳过）；
2. 构建 release 可执行文件；旧 exe 若仍在运行，先改名成 `dsh-flow-ui.running-<时间>.exe` 再构建（遗留文件在下次运行时清理）；
3. 生成 `dist\PAgent-<version>-<stamp>\`，内含 exe、`dsh-flow/`、`plugins/`、README、CHANGELOG 与 LICENSE；
4. 打成 zip 并输出 SHA-256。

## 数据位置

- 配置与凭据：`config.json` + Windows 凭据管理器；数据固定写在可执行文件同目录的 `data/`（不可写时按现有规则回退）。
- 对话：`<data>/chats/<dir>/chat.json`，该对话自己的请求明细在 `requests.jsonl`。
- 统计账本：`<data>/stats/requests.jsonl` 保留最近 30 天请求明细；`<data>/stats/daily/*.json` 保存按 UTC 日的永久汇总，删除对话不会改变历史统计。
- 后台任务：`<data>/chats/<dir>/.dsh-flow/tasks/`。
- 备份：`<home>/backups/`；导出：`<home>/exports/`；日志：`<home>/logs/paagent.log`。
- 备份会复制**可执行文件同目录的 data 目录**以及技能、插件、MCP、config.json，但不复制 workspace。
- “恢复最近备份”先在下次启动时执行：启动时会自动备份当前状态，再用备份替换 data 与附加组件；当前连接与路径设置（config.json）保持不变。
- API Key 不写入 `config.json`（迁移后由 Windows Credential Manager 保存）。

## 约定

- 聊天写盘使用原子写入；删除对话前会做备份/可回滚检查。
- 后台任务结束前校验进程启动时间，避免 PID 复用误杀。
- 工作流必须显式声明输入；模型步骤建议提供 `why` 与 `schema`。