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

## 发布

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\release.ps1
```

脚本会：

1. 跑 Rust 测试和引擎检查/测试（可用 `-SkipTests` 跳过）；
2. 构建 release 可执行文件；
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