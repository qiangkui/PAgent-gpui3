# 工作流固化：从聊天记录到 dsh-flow 清单

**什么时候用**：用户在聊天里把一件事跑通了（工具调用、页面操作、导出、校验都成功），说「固化成工作流」「整理成能重跑的流程」「去掉冗余」。
**不用于**：一次性探索；需要人工审批/接管的步骤（`approve` / `pause` 还没实现）；把多轮不同目标硬并成一条流程。

目标函数：`min(AI 步骤数) s.t. 确定性步骤的测试全绿`。判断由代码与测试做，模型只在真正需要看图/看语义的地方出现。

## 信息边界（先读这一节）

固化只读这些：

- `data/chats/chat-N/chat.json`（要固化的那一轮）；
- `distill` 的三份产物（`flow.draft.yml` / `actions.json` / `distill.md`）；
- 目标工作流目录（`flow.yml`、`steps/`、`docs/`、`tests/`）；
- `dsh-flow/README.md` 的「清单」「步骤协议」两节与 `examples/`；
- 站点自己的 SOP。

**不要读** `dsh-flow/src/`、`src/app/`、`plugins/`、`Cargo.toml` 等产品源码，除非用户明确要求改引擎/界面。
判断步骤契约不需要读实现：§3.5 的模板就是全部契约。

**一个名字只有一份工作流**：

- 先在 `<扫描根>/workflows/` 找同 `name:` 的清单；有就**改它**，没有才新建；
- 不要在 `dsh-flow/workflows/`、`out/`、`backups/` 等位置另建副本；
- 同名清单会让卡片共用记录、互相覆盖运行历史，必须避免。

**预算与停止**：一次固化控制在约 15 个工具调用：distill(1) → 读三份产物(3) → 改 2–3 个文件 → `validate` / `test` / `run`(3) → 汇报(1)。
超出预算、或连续两次校验失败，就停下来把现状与卡点告诉用户，不要用「继续」堆轮次。

**入口唯一 + 热加载**：

- 固化只从窗口的 `+ → 固化为工作流` 进入：它会把 `/固化为工作流` 插进输入框，发送后 Agent 才收到任务
  （自动选真正执行过业务流程的一轮）。用户直接打字要求固化时，提示他用这个入口，不要自己开始。
- 最终 `flow.yml` 写好后，**在同一工作流文件夹创建一个空文件 `.show`**（PowerShell：
  `New-Item -ItemType File -Force .show`）。界面在回答结束后重扫，把它从隐藏中恢复并删除标记，卡片立即出现；
  不要在回答里只说“完成”而没有写 `.show`。

## 0. 一条流程 = 一次运行

- 一条流程只有一个交付物和一条成功判据（一个文件、一个页面的条数、一条记录）。
- 聊天记录在 `data/chats/chat-N/chat.json`；**先选对一轮**：选真正执行过业务流程的那次提问，而不是
  后面的「继续」/「再改一下」。窗口的「固化为工作流」会列出最近有工具调用的轮次与次数；命令行用 `--turn`。
- 多轮里的另一件事另开一条流程；不要静默合并，先把「哪一轮、交付什么」和用户确认清楚。

## 1. 先跑 distill，不要从零手写

```sh
cd dsh-flow
node src/cli.ts distill <data>/chats/chat-N/chat.json --out out/<name> --name <name> --turn <n>
```

> 从窗口的「固化为工作流」进入时，这条命令由 Agent 执行，输出放在目标工作流的
> `out/draft/`；只有 `validate` / `test` / `run` 全过，才把最终 `flow.yml` 写进
> `<扫描根>/workflows/<name>/`，中间不生成草稿卡片。

产物三份，先全部读完再动手：

| 文件 | 看什么 |
|---|---|
| `flow.draft.yml` | 顺序、`run` 步骤、`skip` 占位、`retries` |
| `actions.json` | 每个调用的参数、结果、处置（保留/合并/丢弃）与原因 |
| `distill.md` | 被丢弃/合并了什么、判断点候选、产物与下一步 |

`distill` 只做确定性整理：**不生成 ai 步骤、不改写参数、不判断成功**。它把 `shell` 命令原样变成 `run`，其余真实动作变成 `skip` 占位，探索（`list`/`glob`/`read`/`grep`）与页面探测（`browser eval`/`snapshot`/整页截图）直接丢弃。

## 2. 审查每个动作：重放需要它吗？

**保留**（会改变页面、文件或交付）：

- `shell` 命令 → `run` 步骤；同参相邻重复合并为 1 步 + `retries`。
- 页面 `open`/`goto`/`click`/`type`/`press`/`select` → 折叠成一个站点适配脚本的子命令。
- `http`、文件改动、下载 → 步骤脚本；不要留在清单里当一次性调用。
- `show` 的文件、`downloads` 的结果 → 最终 `result` 步骤的 `artifacts`。

**丢弃**（只影响当时的判断）：

- `list`/`glob`/`tree`/`read`/`grep`/`git`/`env`；
- `browser eval`/`snapshot`/`text`/`wait_for`/`console`/`network`，以及**整页截图**；
- 它们得到的结论写进 `docs/操作SOP.md` 与脚本常量：选择器、接口、等待条件、正则。

例外：`browser screenshot` 带 `selector`/`ref` 的是元素截图，可以作为产物；`read_image`/看图得到的结论可能是判断点（见 §4）。

**判重**：

- 同参相邻 → 合并 + `retries`（distill 已做）。
- 同参但被别的步骤隔开 → 报告会点名；先判断「是重试还是两次真实运行」，不要自行合并。
- 同名不同参 → 通常是参数化点（输入），不是重复。

**容易误留的开发动作**：

- 为写工作流而 `edit` 自己的 `steps/`、`scripts/`、`docs/` → 删掉，它不属于业务流程。
- 调试用的临时打印、一次性读文件、为找元素做的多次 eval → 删掉。

## 3. 把占位变成真正的步骤

- 所有页面/HTTP/文件动作先折叠进 `steps/<site>.py` 或 `steps/<task>.py`；清单里只留 `run: [python, steps/..., 子命令]`。
- **一个子命令 = 一个业务动作**（打开、取码、登录、导航、查询、导出），不要一个 eval 一步。
- 选择器、等待时间、页面判定只放在适配脚本里；站点改版只改一处。
- `show` 改为步骤 stdout 里的 `artifacts`（最多 12 个、必须存在、密钥类被拒）；再给一行 `summary`。
- 失败写结构化 `error`：`kind`（`config` / `selector-miss` / `regex-miss` / `image-unreadable` / `assert` / `timeout` / `credential` / `model` / `engine`）+ `hint` + `where` + `expected` / `gotten`；失败仍然是非零退出。
- 需要浏览器时用 `dsh-flow browser`（与对话共用同一个浏览器与登录态），不要在步骤里自己起一个。

## 3.5 最小模板：不用读源码也能写对

清单骨架（一个 `name` 只出现一次）：

```yaml
name: <name>
description: <one line>
version: 1
inputs:
  out_dir:
    description: 产物目录
    default: out
steps:
  - id: fetch
    desc: 取一张验证码并落盘
    run: [python, steps/site.py, fetch, --out-dir, "{{ inputs.out_dir }}"]
    save: shot

  - id: read
    why: 验证码只能看图判断
    save: code
    ai:
      cache: false
      images: ["{{ steps.fetch.json.image }}"]
      prompt: 逐字读出图里的 4 个字符；看不清就说看不清，不要猜
      schema:
        type: object
        required: [code, readable]
        properties:
          code: { type: string, maxLength: 8 }
          readable: { type: boolean }

  - id: result
    desc: 写出图片与字符
    run: [python, steps/site.py, result, --out-dir, "{{ inputs.out_dir }}"]
```

步骤程序（Python 最小骨架）：

```python
import json, sys

def context():
    raw = sys.stdin.read()
    return json.loads(raw) if raw.strip() else {}

def emit(payload):
    json.dump(payload, sys.stdout, ensure_ascii=False)
    sys.stdout.write("\n")

# 成功：summary 一行结论，artifacts 交给卡片（最多 12 个、必须存在）
emit({"summary": "...", "artifacts": [{"kind": "file", "name": "x.xls", "path": "..."}]})

# 失败：结构化错误 + 非零退出；stdout 只能有一份 JSON
# emit({"error": {"kind": "assert", "message": "...", "hint": "...",
#                 "where": "site.py:fetch", "expected": "...", "gotten": "..."}})
# raise SystemExit(1)
```

fixture（`dsh-flow test` 用）：在清单旁放 `<step-id>.in.json` 与 `<step-id>.out.json`；
输入打桩、输出钉住；纯函数步骤最适合。浏览器/下载类步骤用真机跑，不拿模型回复当 fixture。

清单、步骤与测试的契约都在这一页里；不需要读 `dsh-flow/src/`。只有要改引擎/界面时才读产品源码。

## 3.6 验证与登录：别在验证码上烧 key

- 验证/运行时，若清单有 `workspace` / `browserDir` 输入，指向**当前对话工作区**（窗口输入卡已默认填）。
  `probe` 已登录时，`login` / `captcha` / `read` 整段跳过，**0 次模型调用、不需要 key**。
- 如果 `probe` 说未登录，而 shell 没有接入点 Key（`no api key`）：**不要**在 shell 里反复跑 login / captcha。
  停下来给用户两条路：①「设置 → AI」填 Key 并测试；②用窗口工作流页的「运行」（界面会把凭据送给引擎），
  或先在对话里把浏览器登录，再带 `workspace` 重跑。
- 固化验收不要求 shell 里的 live login：`validate` / `test` / `explain` 过即可；需要验证码的整段 live run
  交给窗口「运行」，或先由用户在对话里登录。不要把验证码识别当成固化是否完成的条件。
- 不要猜验证码；读不清就停。

## 4. 判断点只保留真正需要「看」的那一个

- 只有规则穷举不了的判断才写 `ai`：验证码/图像字符、图像分类、自然语言归类。
- 必须写 `why`（为什么必须问模型）与 `schema`（输出形状）；输入每次都变就写 `cache: false`，否则会回放上一次的答案。
- 成功/失败由页面事实、文件、条数、正则判定，**不让模型自评**。模型回答「读到了什么」，代码回答「这一步过没过」。
- 看不清就 `readable: false` 并失败换一张，不要把猜测写进 `code`。

```yaml
  - id: read
    why: 验证码是图灵测试，规则与普通 OCR 都无法稳定识别
    save: code
    ai:
      cache: false
      temperature: 0
      maxTokens: 4000
      images: ["{{ steps.captcha.json.image }}"]
      prompt: 逐字读出图里的 4 个字符；看不清就说看不清，不要猜
      schema:
        type: object
        required: [code, readable, reason]
        properties:
          code: { type: string, maxLength: 8 }
          readable: { type: boolean }
          reason: { type: string, maxLength: 40 }
```

## 5. 输入、密钥与失败预算

- 账号、密码、路径、地区、导出名、视觉模型 → `inputs`；稳定的给 `default`。
- 密钥/密码 → `secret: true`：不写 `default`、不走 `--input`、不进日志/trace，只从界面、环境变量或 stdin 进入。
- 账号密码被拒要计预算（站点会锁定），验证码错误不计；预算存工作区，重启不清零。
- `retries` 只给瞬时失败；不要把「反复换码」写成 5 个步骤，交给步骤/子流程的 `retries`。
- 会话超时用 `when` 探测并用子流程重新登录；不要把状态不明当成失败或成功。

## 6. 目录与文档

```
workflows/<name>/
  flow.yml              # 主清单：顺序、数据流、判断点
  flows/...             # 需要重试/复用的子流程（如 login）
  steps/<site>.py       # 站点适配：选择器、页面脚本、等待与判定
  steps/<task>.py       # 步骤子命令与契约
  docs/操作SOP.md       # 当时跑通的事实（选择器总表、失败分支、改版改哪里）
  tests/                # 确定性步骤的离线 fixture / 单测
  out/ .dsh-flow/       # 运行产物，忽略
```

- `docs/操作SOP.md` 必须能回答「站点改版时改哪里」；代码与 SOP 一起改。
- 把 `distill.md` 一并保存到 `docs/`：它是「为什么丢弃/合并」的证据。
- `.gitignore` 至少忽略 `out/`、`.dsh-flow/`、`__pycache__/`。

## 7. 验收（顺序固定，缺一不算完成）

1. `node src/cli.ts validate <flow.yml>` —— 严格模式；`ai` 步骤缺 `why`/`schema` 会报。
2. `node src/cli.ts test <flow.yml>` —— 确定性步骤必须有 fixture；**不许**拿模型回复当 fixture。跨机器不稳定的命令（`git --version` 之类）不要钉。
3. 真实 `run` 一次，确认产物真的落地。
4. `node src/cli.ts explain <flow.yml>` —— 报告 AI 占比与每个判断点的 `why`。
5. 把结果、产物路径、失败分支和「还剩哪些人工确认」写回给用户。

## 8. 不要做

- 一步一个 `browser eval`；整页截图当产物；把为找选择器做的探测留在流程里。
- 把账号/密码写进清单、命令、日志，或用 `--input` 传密钥。
- 在清单里重写站点的加密/提交逻辑（照点页面按钮即可，站点自己会算）。
- 用模型判断「成功了没有」；用 `retries` 掩盖状态不明。
- 把多轮聊天硬并成一条流程；把为写工作流而改脚本的 `edit` 当成业务步骤。
- 把 `skip` 占位当成已完成：它只是「这里还需要一个步骤程序」的占位符。

## 9. 一页速查

```text
distill → 读三份产物 → 保留业务动作 / 丢弃探索 → 折叠成 steps 脚本
        → 判断点才写 ai（why + schema + cache:false）
        → 参数化 inputs，密钥 secret
        → validate → test → run → explain → 交付说明
```

判定口径：**能重放的才进清单；只影响当时判断的进 SOP；模型只回答「看到了什么」，代码回答「过没过」。**