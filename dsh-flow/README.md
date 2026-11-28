# dsh-flow

**确定性优先的工作流运行器。** 清单只描述"顺序、数据流、条件"；真正干活的是真实程序文件（任意语言）；需要模型判断的地方才显式写成 AI 步骤。

零依赖、无构建：`node >= 23.6` 直接跑 `.ts`。

项目整体（界面、协作规则、阶段计划）见上层目录的 [README.md](../README.md)、[RULES.md](../RULES.md) 与 [PLAN.md](../PLAN.md)；引擎侧只认本文件。

---

## 为什么不是又一个 workflow DSL

因为你要的是"AI 写一次代码，以后就只跑代码"。所以：

| 传统 workflow 引擎 | dsh-flow |
|---|---|
| 步骤是 YAML 里的表达式和小脚本 | 步骤是**真实的程序文件**，可以 `git diff`、可以单测、可以随便换语言 |
| 引擎提供一套内置动作 | 引擎只提供**契约**：stdin 一份 JSON、stdout 一份 JSON |
| 逻辑写在配置里 | 逻辑写在你熟悉的语言里，配置只声明连接关系 |

代价是多语言之间只能靠 JSON 通信。这是刻意的：这样每一步都能独立运行、独立测试、独立替换。

---

## 快速开始

```sh
node src/cli.ts init my-flow --lang python
node src/cli.ts run my-flow/flow.yml --input who=you
```

跑一下仓库自带的示例（全确定性，**0 token**）：

```sh
node src/cli.ts explain examples/repo-health/flow.yml
node src/cli.ts run     examples/repo-health/flow.yml --input repo=../../.. --report out/report.html
```

---

## 清单

```yaml
name: repo-health
description: 仓库健康巡检——全确定性，不调用模型
version: 1

inputs:
  repo:
    description: 要巡检的目录
    default: .

vars:
  threshold: 0

defaults:
  run:
    timeoutMs: 120000
    env:
      TZ: UTC

steps:
  - id: version
    desc: 记录 git 版本
    run: [git, --version]
    save: gitVersion          # vars.gitVersion = 该步的主输出

  - id: counts
    desc: 统计文件数与体积
    run: [python, steps/count_files.py]
    save: counts

  - id: verdict
    when: steps.counts.json.files > 0
    set:
      summary: "{{ vars.counts.files }} files · {{ vars.gitVersion | trim }}"
    save: report
```

### 步骤字段

| 字段 | 说明 |
|---|---|
| `id` | 必填，同层唯一，被引用为 `steps.<id>` |
| `desc` | 进度输出里的说明 |
| `when` | 表达式，假则该步 `skipped`（不执行、不报错） |
| `skip` | 恒跳过，用来留占位 |
| `allowFailure` | 失败不中断整个流程 |
| `retries` / `retryDelayMs` | 失败重试，默认 0 次 / 250ms |
| `save` | 把主输出写进 `vars.<save>` |
| `why` | 为什么这一步必须问模型（`ai` 步骤在严格模式下必填） |
| `run` | 外部程序：`[cmd, ...args]`（不经 shell）、字符串（经 shell）、或对象 |
| `set` | 纯赋值，值支持模板 |
| `map` | 对数组扇出，内部再跑一组步骤，默认并发 4 |
| `ai` | 显式模型调用（见下一节），与 `run`/`set`/`map` 互斥 |
| `use` | 调另一个清单做子流程（见下一节），与其它执行器互斥 |

`run` 对象形式：`{ command, cwd?, env?, timeoutMs?, shell?, textOnly? }`。

---

### 子流程（`use`）

把 login / fetch / parse 这样的片段拆成独立清单，父流程只声明调用和输入：

```yaml
steps:
  - id: login
    use:
      manifest: ../shared/login/flow.yml
      inputs:
        user: "{{ inputs.user }}"      # 模板值保留类型（数字仍是数字）
    save: session                       # 子流程最终的 vars 落到 vars.session

  - id: greet
    set:
      line: "{{ vars.session.token }}"
```

也可以只写路径：`use: ../shared/login/flow.yml`。

- 子流程有自己的目录、自己的 `.dsh-flow/answers` 与 `ledger`，所以它可以单独 `run` / `test` / 固化。
- 父流程拿到的是子流程**最终的 `vars`**，作为 `steps.<id>.json`：能读 `steps.<id>.json.<名字>`，`save` 取的就是它。
- `inputs` 只接受子清单声明过的名字；必填缺失会在子流程开始前报错。
- 子流程失败 → 这一步失败（`error` 里写明失败在哪个子步骤）；`allowFailure: true` 时父流程继续。
- 嵌套最多 5 层；清单调用自己（或互相调用）会被拒绝，报 `sub-flow cycle`。
- 子流程不写父流程的 trace（避免步骤 id 撞车），它的每一行输出带 `[<父步骤 id>]` 前缀出现在父流程日志里。

---

### AI 步骤（`ai`）

模型调用必须**显式写出来**，并且默认要交代两件事：`why`（为什么这里必须做判断）和 `schema`
（输出长什么样）。两者缺一个，`validate` 就报错；`--no-strict` 只用于还在起草清单的时候。

```yaml
steps:
  - id: classify
    desc: 归类并给出理由
    why: 自然语言反馈的归类无法用规则穷举
    save: verdict
    ai:
      prompt: |
        把下面的反馈归类，并给出不超过 20 字的理由。
        反馈：{{ steps.normalize.json.note }}
      system: 你是一个严谨的分类器
      model: deepseek-chat          # 可选，缺省走接入点默认模型
      temperature: 0                # 可选
      maxTokens: 512                # 可选
      schema:                       # 严格模式下必填
        type: object
        required: [category, reason]
        properties:
          category:
            type: string
            enum: [bug, question, other]
          reason:
            type: string
            maxLength: 40
```

- **schema 子集**：`type`（object / array / string / number / integer / boolean / null）、
  `properties`、`required`、`items`、`enum`、`minimum`、`maximum`、`minLength`、`maxLength`、
  `minItems`、`maxItems`、`additionalProperties`。别的关键字会在 `validate` 阶段报"不支持"。
- **回答不合规会自动修一次**：把上一轮的回答和校验错误一起发回去重问；第二次仍不合规就失败，
  错误里带具体路径（例如 `$.category: 期望取值 "bug" / "question"`）。
- **契约与代码步骤相同**：结果写进 `steps.<id>.text`；能解析成 JSON（且有 schema 时通过校验）
  的还写进 `steps.<id>.json`，`save` 取 `json ?? text`。

#### 看图，以及不能回放的判断

- `ai.images`：一组路径（可写模板，相对清单目录解析）。`http` 后端把它们作为 `image_url` 内容块发出
  ——一次最多 3 张、单张 5 MB、总量 12 MB base64，超了在花 token 之前就报错；`cli` 后端只把
  `[{path, mime}]` 放进 stdin，脚本自己打开。图片的 sha256 会进判断记录的 key，所以**换了图就是新判断**。
- `ai.cache: false`：每次都问模型，既不读也不写记录。验证码这类「输入每次都不一样」的判断点必须这么写；
  `--level manual` 遇到它会直接说明「没有答案可回放」，而不是回放昨天的验证码。
- `ai.model` 也支持模板，一条流程可以只让某一步走视觉模型：

```yaml
  - id: read
    why: 验证码是图灵测试，规则与普通 OCR 都无法稳定识别
    ai:
      model: "{{ inputs.vision_model }}"   # 空字符串 → 用接入点默认模型
      cache: false
      images: ["{{ steps.shot.json.image }}"]
      prompt: 逐字读出图里的 4 个字符；看不清就说看不清，不要猜
      schema:
        type: object
        required: [code, readable, reason]
        properties:
          code: { type: string, maxLength: 8 }
          readable: { type: boolean }
          reason: { type: string, maxLength: 40 }
```

#### 三档介入

| `--level` | 行为 | 模型调用 |
|---|---|---|
| `manual` | 只回放已记录的决定；没有记录就失败并提示怎么记录 | 0 |
| `assist`（默认） | 有记录就回放，没有才问模型，问完记录 | 只补缺的 |
| `draft` | 每次重新问模型并覆盖记录 | 每次都问 |

记录放在清单旁的 `.dsh-flow/answers/<step-id>/<key>.json`，`key` 由**后端、渲染后的 prompt、system、
schema、model、temperature、maxTokens** 一起算出。改了其中任何一个，旧记录就失效——所以
"日常 `run` ≡ `manual`"：流程固化后每天跑，0 token；改了判断点才需要再问一次。

#### 两种后端

| `ai.backend` | 说明 |
|---|---|
| `http`（默认） | OpenAI 兼容的 `/chat/completions`；Key 从 `DEEPSEEK_API_KEY` 取（或由界面按请求传入），失败只对看起来是暂时性的错误重试 2 次 |
| `cli` | 自己提供命令：stdin 收 `{prompt, system, model, temperature, maxTokens, schema}`，stdout 回 `{"text":"…"}`、`{"json":…}` 或纯文本 |

`examples/triage` 用 `cli` 后端接一个本地 Python 替身，所以不需要 Key 也能把示例跑通；
删掉 `backend` / `command` 两行就换回真实模型。这条路径也是把已有脚本、本地模型接进来的方式。

### 运行时的名字

清单里写逻辑名，引擎解析成这台机器上真实存在的可执行文件：

| 逻辑名 | 接受的实现 |
|---|---|
| `python` | `python` / `python3` / `py -3` |
| `node` `npm` `git` `rg` | 同名 |
| `bash` `sh` `pwsh` `powershell` `cmd` | 同名 |

解析不到会在 `validate` 阶段报错，而不是等到运行时才炸。

---

## 步骤协议

程序被调用时：

**stdin**（关闭前写入一份 JSON）

```json
{
  "inputs": { "repo": "." },
  "vars": { "counts": { "files": 3 } },
  "item": null,
  "index": null,
  "steps": {
    "counts": { "id": "counts", "kind": "run", "status": "ok", "exitCode": 0, "json": { "files": 3 }, "durationMs": 12 }
  }
}
```

**stdout** —— 只放结果。能解析成 JSON 就进 `steps.<id>.json`，否则当文本放在 `steps.<id>.stdout`。

**stderr** —— 日志，原样转发，不参与结果。

**exit code** —— `0` 成功；非零失败（除 `allowFailure`）。

**环境变量** —— 引擎强制注入 UTF-8（`PYTHONUTF8=1`、`PYTHONIOENCODING=utf-8`）。Windows 上 Python 默认按 ANSI 代码页输出，不设这个，中文 JSON 会直接解析失败。

**秘密输入** —— `inputs` 里的名字可以标 `secret: true`：这样的输入**不能写 `default`**（加载时就报错，
因为清单会被提交）、**不能走 `--input`**（会进命令行、进程列表与日志），只能来自
`--secrets-stdin`（界面的输入卡就是这么送进来的）或环境变量 `DSH_SECRET_<名字>`。
引擎把它的值在日志行、trace、步骤的 `stdout` / `stderr` / `text` / `json`、`vars`、汇总与 HTML 报告里
统一替换成 `***`；步骤程序自己仍然通过 `{{ inputs.名字 }}` 拿到明文。

**其他** —— 单流输出上限 4 MB（超出截断并标记）；超时先 SIGTERM，再强杀；命令默认不经 shell（`[cmd, ...args]`），用字符串写命令才走 shell。每个步骤还会收到 `DSH_ENGINE_DIR`（引擎自己的目录），需要调 `dsh-flow browser` 的步骤用它，不必在清单里写死路径。

**步骤可以描述自己的结果**（三个保留键，都是可选的；一次只能输出**一个** JSON 文档）：

```json
{
  "summary": "共 51 条（第 1–10 条）",
  "artifacts": [{"kind": "file", "name": "使用单位导出.xls", "path": "成品的绝对或相对路径"}],
  "error": {"kind": "regex-miss", "message": "分页条里没有「共 N 条记录」",
            "hint": "站点改版时正则要跟着改", "where": "maers_site.py:query",
            "expected": "共 51 条", "gotten": "每页：10…"}
}
```

- `summary` 进卡片的那一行；`ai` 步骤没写时用回答的首行；
- `artifacts` 最多 12 个，路径相对清单目录、**必须存在**，密钥类文件被拒；大小由引擎补算；
- `error.kind` 限 `credential` / `config` / `selector-miss` / `regex-miss` / `image-unreadable` /
  `assert` / `timeout` / `model` / `engine`，其它归为 `engine`。
  它不改变退出码的含义：**失败仍然是非零退出**，`error` 只是把"为什么、怎么修、期望什么"说清楚。

---

## 模板与表达式

两种语法，边界是固定的：

- **模板** `{{ path | filter }}` —— 只做插值。过滤器：`trim` `upper` `lower` `json` `len` `first` `last` `lines`。
- **表达式**（仅 `when` 与 `map.over`）—— 字面量、点路径、`== != > >= < <= && || !`、括号，以及 `contains` `startswith` `endswith` `matches` `len` `empty` `num` `str` `lower` `upper`。

路径来自：`inputs.*`、`vars.*`、`steps.<id>.*`、`env.*`、`item`、`index`，以及 `map` 绑定的名字。

如果整个字段就是一个不带过滤器的模板，**保留原类型**：`"{{ steps.x.json.total }}"` 得到数字 7，不是字符串 `"7"`。

表达式**不会执行 JavaScript**。清单可能由模型生成，所以只认上面这一小套语法。

---

## 命令

```sh
node src/cli.ts run     <manifest> [--input k=v]... [--level manual|assist|draft] [--no-strict] [--credentials-stdin] [--secrets-stdin] [--events] [--dry] [--report file.html] [--json] [--quiet]
node src/cli.ts validate <manifest> [--no-strict]
node src/cli.ts explain  <manifest> [--input k=v]...
node src/cli.ts test     <manifest> [--update] [--filter <step-id>]
node src/cli.ts init     <dir> [--lang python|node]
node src/cli.ts derive   <session.jsonl> [--out <dir>] [--name <name>] [--description <text>]
node src/cli.ts distill  <chat.json> [--out <dir>] [--turn <n>] [--name <name>] [--description <text>]
node src/cli.ts ai                       # 一次模型问答：stdin 一个 JSON，stdout 一个 JSON
node src/cli.ts models                   # 该接入点可用的模型：stdin {baseUrl,apiKey}，stdout {models:[…]}
node src/cli.ts browser '{"action":"goto","url":"…"}' [--workdir <dir>] [--json]
                                         # 一个浏览器动作：请求是浏览器工具的同一套参数，
                                         # 默认输出文本，--json 输出 {ok,text,files,image}
```

- `run` 会在清单旁写 `.dsh-flow/runs/<runId>.jsonl` 轨迹；失败时自动在同目录生成 HTML 报告，也可以用 `--report` 指定路径。
- `run` 默认 `--level assist`：先回放 `.dsh-flow/answers/` 里的判断记录，只有缺的才问模型；`--level manual` 一次模型都不调，`--level draft` 每次都问。
- `browser` 是给**步骤**用的：工作流里的程序（Python / Node 都行）调它打开页面、填表、截图、等下载，
  与问答里的 `browser` 工具共用同一个浏览器、同一个页面和同一份登录态（指针在
  `<workdir>/.dsh-flow/browser/session.json`，浏览器活过单个进程）。请求体就是工具参数：
  `{"action":"eval","expression":"…"}`。退出码 0/1 表示动作是否成功，`--json` 时失败也照样打印
  `{"ok":false,"text":"…"}`，脚本可以直接分支。`wait_download`（等文件写完）与
  `downloads`（列目录）不需要浏览器在跑；`screenshot` 支持用 `selector` 只截一个元素。
- `explain` 打印步骤表和"多少步骤不需要模型"的占比（`ai N` + `ai 判断点` 会列出每一步的 `why`）——这是判断一条流程值不值得固化的指标。
- `--dry` 只渲染不执行；`--no-strict` 允许 `ai` 步骤暂时不写 `why` / `schema`。
- `--credentials-stdin` 让 `run` 从 stdin 读一份 `{"apiKey","baseUrl","model","secrets"}` 交给 `http` 后端
  （`cli` 后端不读它）。界面就是这样在不暴露 Key 的前提下运行带 `ai` 步骤的工作流；
  不给这个参数时回退到 `DEEPSEEK_API_KEY` / `DEEPSEEK_BASE_URL` / `DEEPSEEK_MODEL`。
- `--secrets-stdin` 是同一份载荷的窄写：只读 `{"secrets": {名字: 值}}`，交给 `secret: true` 的输入。
- `--events` 把运行边界做成 NDJSON 打在 stdout 上，**最后一行仍是汇总对象**：
  `run/start`、`step/start`、`step/end`（带 `status` / `summary` / `artifacts` / `error`）、`run/end`（带 verdict）。
  人读的进度同时照旧走 stderr。`--events` 与 `--json` 互斥。
- `ai` 是给界面和脚本用的桥：`{"prompt":"…","system":"…","history":[…],"model":"deepseek-chat","apiKey":"…","baseUrl":"…"}`
  进，`{"text":"…","model":"…","usage":…}` 或 `{"error":"…"}` 出；缺省从 `DEEPSEEK_API_KEY` /
  `DEEPSEEK_BASE_URL` / `DEEPSEEK_MODEL` 取，接口按 OpenAI 兼容的 `/chat/completions` 调用。
- `history` 是这轮之前的对话，按时间顺序排列成 `[{role:"user"|"assistant", content:"…"}]`
  （未知角色与空内容会被丢掉，只保留最近 40 条，见 `ai.ts` 的 `historyMessages`）；
  有了它，"那它呢？"这类追问才有主语。
- `ai` **边写边出**：请求带 `stream: true`，模型每吐出一段文字就输出一行
  `{"type":"delta","text":"…"}`（`src/stream.ts` 负责 SSE 与 `tool_calls` 分片拼装），
  最后再输出完整的 `{"text":…}`；对面不支持流式（回 JSON）时自动退回整段解析。
- `models` 走 OpenAI 兼容的 `GET {baseUrl}/models`，返回该 Key 能用的模型 id（排序去重，也兼容
  `{models:[…]}` 这种简写），供界面列出可选版本；失败时返回 `{"error":"…"}`。
- `ai` 可带 `"reasoning": "off" | "low" | "high" | "max"` 选择思考方式（省略或 `default` = 端点默认）：
  `off` 写成 `{"thinking":{"type":"disabled"}}`，其余写成 `reasoning_effort`；多轮循环、轮次用尽的总结轮
  与单次问答用的是同一个选择。开思考时端点把思考作为 `reasoning_content` 流回来，引擎逐行输出
  `{"type":"reasoning","text":"…"}`（不混进 `delta`），并回报逐轮估算 `roundReasoning` 与整问 `reasoning`；
  思考不计入回答，也不进提示词。
- 压缩用的摘要调用（`src/compact.ts`）对 DeepSeek 端点固定关闭思考：摘要必须落在自己的
  `max_tokens` 里，思考会把它吃掉。
- `ai` 可带 `"webSearch": true`：在 `tools` 最前面加上服务端工具 `deepseek_web_search` /
  `deepseek_read_url`。搜什么、打开哪一页由模型决定，搜索在 DeepSeek 服务器上执行，**客户端不执行工具**；
  回包的 `server_tool_calls` / `annotations` 会被翻译成与本地工具同形的 `step`（`web_search` / `read_url`，
  含「标题 — URL」来源），最终答案另带 `webSearch: { searches, fetches, sources }` 与原始 `serverToolCalls`。
  搜索开始时先按 `server_tool_event` 的 `call_started` 发一条 `tool_start`（进行中行），完成时用同一 `call` id
  的 `step` 替换；网关不认识这两个工具类型时自动去掉它们重发一次。
- `history` 里的助手段落可带 `"serverToolCalls"`（就是上一次回答的 `serverToolCalls`）：引擎原样放进该条
  assistant 消息的 `server_tool_calls`，提供方据此恢复之前读过的页面；同一问内的工具轮也会把本轮的服务端
  调用带上。丢了它请求仍会成功，只是早先读的页面不在上下文里。
- `ai` 带上 `"tools": true` 时，引擎会先跑工具循环（`src/agent.ts` + `src/tools.ts`）：
  每个工具调用**一执行完就**输出一行 `{"type":"step","tool":"read","target":"a.txt","ok":true,"detail":"已读取 12 行"}`
  （`detail` 是摘要：`read` / `glob` / `grep` 折成行数、文件数、匹配数），最后再输出 `{"text":…}`。
  工具被限制在当前工作目录内。注意工具轮里的 `delta` 属于那一轮的过程文本，界面收到 `step` 会清掉它。
- 工具轮的上下文有上限：单个结果按 `MAX_TOOL_MESSAGE` 截取，整问的结果总量按 `MAX_TOOL_CONTEXT` 逐步省略；
  一次调用的参数超过 `MAX_WIRE_ARGUMENTS` 时，执行完就换成一句说明（id 与工具名保留，`tool_call_id` 配对不变），
  免得一次大文件写入在后续每一轮里重发。模型因输出上限被截断时，回答末尾会带一句「输出达到长度上限」。

---

## 契约测试

步骤可以被单测，因为它只依赖 stdin 的 JSON。

```
fixtures/
  counts.in.json     # { "inputs": {...}, "vars": {...}, "steps": {...} }
  counts.out.json    # 该步期望的主输出
  classify.ai.json   # ai 步骤：录制下来的模型回复 { "text": … } 或 { "json": … }
```

```sh
node src/cli.ts test examples/repo-health/flow.yml           # 比对
node src/cli.ts test examples/repo-health/flow.yml --update  # 首次录制
```

没有 fixture 的步骤会报 `uncovered`——**覆盖率本身就是个信号**：能钉住输出的步骤，才是能被代码替换的步骤。

`ai` 步骤**只用 `.ai.json` 里的回复**跑契约测试，绝不联网、也不会写判断记录（`cache: false`，
否则录制的回复会被下一次 `run` 当成"已决定"的答案）；缺这个文件就报 `uncovered`，
`--update` 也会拒绝（模型回复不能由引擎自己编）。示例 `examples/triage` 三步都有 fixture，
`test` 全绿且 0 token。

示例里 `counts` 有 fixture（扫固定样本目录），`verdict` 用输入 fixture 钉住，`tooling`（`git --version`）故意没有——它跨机器不稳定，不该被钉。

---

## 从会话派生骨架（derive）

```sh
node src/cli.ts derive session.jsonl                 # 清单打印到 stdout
node src/cli.ts derive session.jsonl --out out/derived --name triage-session
```

它读 dsh 的会话 JSONL（第一行 `{"type":"session",…}`，之后每行 `{"type":"session_event","event":{…}}`），
把"当时真的跑了什么"变成一份可以 review 的清单：

| 会话里的东西 | 派生成什么 |
|---|---|
| `tool/call` 的参数里有 `command` / `cmd` | 一个 `run` 步骤，命令原样保留（数组形式写列表，多行字符串写 `run: |`），前一段助手文字变成 `desc` |
| 其它工具调用（`read` / `write` / `glob` / 浏览器…） | `# TODO` 注释，带工具名、参数摘要和当时的意图 |
| 最后一段没有后续动作的助手文字 | `# TODO` 注释（判断/结论：…） |
| 第 2 轮及以后的提问 | **不派生**，只在文件头注明有几轮被跳过 |
| 解析不了的行 | 跳过并计数，不报错 |

三条刻意的边界：

- **只生成 `run` 步骤，不生成 `ai` 步骤**。哪里该问模型是人的判断；把 TODO 提升成 `ai:` 时再补
  `why` 和 `schema`（严格模式会提醒你）。
- **一条流程只跑一轮**。会话里后面几轮是另一条流程，除非人工合并。
- **不推断**：没见过的工具不会被猜成命令，参数不会被改写。派生出来的是草稿，`validate` / `test` /
  人工 review 之后才算数。

窗口里的聊天记录不是会话 JSONL，用下一节的 `distill`。

## 从聊天记录固化（distill）

`derive` 读的是 dsh 的会话 JSONL；窗口保存的聊天是另一份形状：`data/chats/chat-N/chat.json`
是一个 JSON 文档，`messages` 里混着助手文字、思考与工具行（工具行的 `text` 以 `工具名 · …` 开头，
`input` 是参数 JSON，`output` 是结果）。把 `chat.json` 交给 `derive` 只会得到 `0 run · 1 skipped`。

```sh
node src/cli.ts distill data/chats/chat-1/chat.json --out out/derived
node src/cli.ts distill data/chats/chat-1/chat.json --out out/derived --turn 1 --name maers-units-export
```

它只做确定性的部分：

| 聊天记录里的东西 | 派生成什么 |
|---|---|
| `shell` 且 `args.command` 是字符串 | `run` 步骤，命令原样保留（多行写 `run: |`）；同参相邻重复合并为 1 步 + `retries` |
| `browser` 的 open/goto/click/type/press/select、`http`、文件改动、`show`、`run_tests`… | `skip` 占位步骤：`desc` 写清要实现的动作，参数留在 `actions.json` |
| `browser` 的 eval/snapshot/text 与整页截图；`list`/`glob`/`read`/`grep`… | 丢弃（探索与探测），只在报告里计数，提示写进 SOP |
| `browser screenshot` 带 `selector` / `ref` | `skip` 占位（产物）：元素截图可以成为 workflow 的 artifact |
| 同名同参但中间隔着别的步骤 | 不合并，在报告「合并与重试」里点名，由人判断是重试还是两次真实运行 |
| 收尾的助手结论、看图的调用 | 「判断点候选」：要提升成 `ai` 时再补 `why` / `schema` / `cache: false` |

产物三份：`flow.draft.yml`（能 `validate` 的草稿）、`actions.json`（每个调用的参数、结果、处置与原因）、
`distill.md`（动作表、被合并/丢弃的调用、判断点、产物与下一步）。默认只固化**最后一个提问**所在的轮次，
`--turn` 可以指定；第 2 轮及以后不会自动混进来。`--name` 与 `--description` 原样写进清单（不做 slug），
中文标题可以直接用；只有缺省时才从标题生成 ASCII 名。

三条边界与 `derive` 相同：**不生成 ai 步骤**、**不改写参数**、**不判断成功**。草稿里的 `skip` 占位是
「这里还需要一个步骤程序」的占位符；哪些步骤留下、合并或删掉，由 review 决定。

review 的口径写在应用侧的 Skill `skills/workflow-distill/SKILL.md`（Skill 页导入后启用）。

## 设计约束

1. **准确优先，少用 AI 是目标函数**：`min(AI 步骤数) s.t. 步骤契约测试全绿`。
2. **通过与否由校验器/契约测试判断**，不由模型自评。
3. **AI 步骤与代码步骤的对外契约相同**（`steps.x.json` / `steps.x.text`），所以可以把某一步在"模型实现"和"程序实现"之间来回替换，清单其余部分不动。
4. **派生出来的是代码，不是产物**：需要 review、进 git、可 diff。

---

## 当前状态

**P0（已完成）**

- YAML/JSON 清单解析与静态校验
- `run` / `set` / `map` / `when` / `skip` / `allowFailure` / `retries` / `save`
- 运行时解析、UTF-8 强制、超时与输出上限、并发扇出
- `validate` / `explain` / `run` / `test` / `init`
- 运行轨迹 `.dsh-flow/runs/*.jsonl`、自包含 HTML 报告
- `dsh-flow ai` 桥：DeepSeek（OpenAI 兼容）单轮问答，凭据按请求传入或走环境变量
- 119 个测试（`node --test tests/`）

**P1（已完成）**

- `ai:` 步骤：`prompt` / `system` / `schema` / `model` / `temperature` / `maxTokens`，回答不合规自动修一次
- schema 子集校验器（`src/schema.ts`），清单加载时先查 schema 本身
- 后端：`http`（重试 + 超时）、`cli`（外部命令）、`mock`（测试）
- 强制 `why` + `schema`（`--no-strict` 可关），`validate` 报出缺项
- 三档介入 `manual` / `assist` / `draft`，判断记录可回放，日常 `run` 0 次模型调用
- `explain` 输出 AI 依赖度（`ai` 步骤数 / 总步骤数）与每个判断点的 `why`
- `derive`：从 dsh 会话 JSONL 生成骨架（命令 → `run` 步骤；判断点与其它调用 → `# TODO`）
- `distill`：从窗口的 `chat.json` 生成草稿（命令 → `run`；其余动作 → `skip` 占位；探索/探测丢弃并报告，同参重复合并）
- `run` 的两处通道：`--credentials-stdin`（界面把接入点交给 `ai` 步骤）与字符串命令走 shell
- 思考开关与力度：`ai` 桥的 `reasoning`（`off` / `low` / `high` / `max`），逐轮思考随 `reasoning` 事件与 `roundReasoning` 回报

**未实现（按计划推进）**

| 能力 | 说明 |
|---|---|
| `approve` / `pause` | 提交前人工确认；2FA/验证码人工接管 |
| `fetch` 录制回放 | 把响应体存成 fixture，`--offline` 回放 |

（`services`、`once` + ledger、`use:` 已实现，见上文。）

---

## 注意

- 它直接执行 shell 命令，**绕过任何沙箱与审批**。派生或生成的清单必须人工 review 后再跑。
- `.dsh-flow/`、`out/` 已在 `.gitignore` 中。
