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

界面查找引擎的顺序：项目内 `dsh-flow/` → 可执行文件旁的 `dsh-flow/` → `DSH_FLOW_HOME`。便携发布包会把引擎放在 exe 旁边。

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
3. 生成 `dist\PAgent-<version>-<stamp>\`，内含 exe、`dsh-flow/`、README、CHANGELOG 与 LICENSE；
4. 打成 zip 并输出 SHA-256。

## 数据位置

- 配置与凭据：`config.json` + Windows 凭据管理器；数据目录由设置中的 `dataDir` 决定。
- 对话：`<dataDir>/chats/<dir>/chat.json`，请求明细在 `requests.jsonl`。
- 后台任务：`<dataDir>/chats/<dir>/.dsh-flow/tasks/`。
- 备份：`<home>/backups/`；导出：`<home>/exports/`。
- API Key 不写入 `config.json`（迁移后由 Windows Credential Manager 保存）。

## 约定

- 聊天写盘使用原子写入；删除对话前会做备份/可回滚检查。
- 后台任务结束前校验进程启动时间，避免 PID 复用误杀。
- 工作流必须显式声明输入；模型步骤建议提供 `why` 与 `schema`。