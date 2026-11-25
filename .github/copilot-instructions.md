# 开发协作约定

## 发布：每次开发对话结束时生成 release 版

> 本节面向**在本仓库工作的开发助理**。应用内的对话智能体不适用本节，不要据此执行发布流程。

在本仓库完成一次对话/任务时，收尾运行仓库自带的发布脚本：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\release.ps1
```

脚本会依次执行：

1. `cargo test --release`（Rust 测试）；
2. 引擎 `npm ci` / `tsc --noEmit` / `node --test`；
3. `cargo build --release`（旧的 `dsh-flow-ui.exe` 若被占用，会先改名再构建）；
4. 打包 `dist\PAgent-<version>-<stamp>.zip`，并打印 SHA-256 与体积。

完成后向用户汇报：zip 路径、SHA-256、大小，以及测试是否全部通过。

例外：本轮只是纯文档改动、或用户明确表示不需要、或 exe 被占用且无法改名时，说明原因并跳过。

## 不留备份

- 改代码直接改，不要为每次修改留副本：不产生 `*.bak`、`*_old`、`copy of …`、临时目录快照，也不要在 `dist/`、`backups/` 之外另存一份“以防万一”。
- `dist/` 只放当前这一版：发布脚本打包后自己删 `PAgent-<version>-<stamp>/` 暂存目录，并删掉此前的 `PAgent-*.zip`，只留刚生成的包；`dist\data`（对话与统计）、`dist\logs`、`dist\config.json` 等运行时目录不是发布产物，永远不要顺手删。
- 需要回退时用 git（`git diff` / `git status` / 暂存区），不要用备份文件。
