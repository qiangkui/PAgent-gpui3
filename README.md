# PAgent

一个 Windows 桌面 AI 助手：GPUI 原生界面 + 可独立运行的 `dsh-flow` TypeScript 引擎。

## 组成

- `src/` — Rust/GPUI 界面：对话、上下文、统计、后台任务、工作流、插件、MCP、Skill、设置。
- `dsh-flow/` — 工作流与 AI 请求引擎：清单执行、工具循环、MCP/插件、压缩、CLI。
- `workflows/` — 可直接运行的业务工作流（清单 + 真实步骤程序）：`units-export` 是国家医疗器械不良事件监测系统的使用单位导出，整条流程只有 1 个判断点（读验证码），说明见该目录的 README。
- `docs/` — 设计文档：`工作流卡片-界面设计.md` 记着工作流页这一轮做了什么、事件协议、以及还没做的界面部分。
- `assets/` — 图标与内嵌资源。
- `backups/` — 手动备份产物（不纳入版本控制）。
- `dist/` — 构建产物与便携发布目录（不纳入版本控制）。

## 环境

- Windows 10/11
- Rust stable（项目使用 edition 2024）
- Node.js >= 23.6（引擎要求，TypeScript 直接运行）。界面启动时会检查 `node --version`，版本过低或缺装会在横幅里直接说明，而不是等到第一次提问才失败。
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

把聊天里跑通的流程固化：引擎侧是 `node src/cli.ts distill <chat.json> --out <dir> --turn <n>`；窗口里是输入框 **+** →「固化为工作流」，它把 `/固化为工作流` 作为一条整指令插进输入框，发送后由 Agent 自动选轮次并完成：中间产物只放 `out/`，`validate` / `test` / `run` 全过后写 `flow.yml` 与 `.show`，卡片随即出现在 `workflows/<name>/`。

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
- 连接卡片上的「图片：开 / 关」：默认开启。关掉后这个接入点不再声明 `read_image` / `screenshot`，追问时也不会把上一轮的截图再带进去 —— 纯文本模型收到 `image_url` 会直接拒绝整个请求。它和温度 / 思考一样随连接保存，不影响「已接入」状态。
- 设置 → AI → 连接卡片的「上下文窗口」滑杆：默认按模型名估算（内置表，未知模型按 128k），也可以拖到 32k / 128k / 256k / 1M —— 引擎按它的 80% 触发压缩，标题下的窗口占用条也用同一个数值，两边不会各算各的。旁边的「恢复默认」清掉这个覆盖。

### 联网搜索

- **默认开启，设置里没有这个开关了**：只要端点提供这些工具（`deepseek-flash`）就一律声明，其它模型不发送，避免请求被拒。
- 想为某个对话关掉它：输入框「+」菜单 →「本次对话」→「联网搜索」开关（关闭只影响这一个对话，可选「回到默认（开启）」）。
- 开启后请求的 `tools` 里会带上 DeepSeek 的服务端工具 `deepseek_web_search` 与 `deepseek_read_url`：**搜什么、打开哪一页由模型自己决定，搜索和抓取都在 DeepSeek 服务器上执行**，本机不执行任何工具，结果直接进入模型上下文。
- 对话里每次搜索 / 读页是一行「web_search」/「read_url」：搜索开始就出现在对话里（不必等回答），展开后每个来源一行、点「打开」用默认浏览器打开；上下文页「动作」卡把它归到「网络」。
- 搜索次数会进统计账本，统计页出现「联网搜索」卡片（有搜索才显示）；已读页面随助手段落保存在对话里，追问时一并回传，不会丢。
- 思考开着时是深度搜索（多步搜索、翻页、页内查找），思考关闭时是一次搜索后带引用回答；费用按 token 计（2026-11-18 起模型调用触发的搜索不再单独收费）。

## 工具

内置工具随每次请求一起声明（定义在 `dsh-flow/src/tools/specs.ts`，确认规则见「权限与沙箱」）：

- 文件：`read` / `write` / `edit` / `replace` / `move` / `copy` / `mkdir` / `delete` / `list` / `glob` / `tree`。
  - `read` 也**能读 PDF**：按页抽取文字并给出「第 N 页」标记，`pages: "3-7"` 指定范围（一次最多 40 页、总计 2 万字符）。抽取需要本机装有 **Poppler 的 `pdftotext`**（优先，快且保留排版）或 **Python 的 `pypdf`**（`python -m pip install pypdf` 一条命令）；两者都没有时 `read` 会直接说明装哪个，而不是报「二进制文件」。扫描件（没有文字层）会提示改用 `read_image` 看渲染页或把 PDF 附到聊天里；加密 PDF 会说明需要解密后的文件。
  - `edit` 既可以用 `old` / `new` 唯一替换、用 `edits` 批量替换，也可以按行号 `startLine` / `endLine` 替换整段（`content` 留空即删除这几行）、用 `insertAfter` 在指定行后插入；两种定位方式不能混在同一次调用里。
  - `copy` 字节原样复制（图片、字体等二进制资源也可以），目标已存在时默认拒绝，`overwrite: true` 才覆盖；`mkdir` 建目录并自动带上父目录。
  - `delete` 默认只删文件；`recursive: true` 才连文件夹内容一起删（卡片会写明，不能撤销）。
  - `replace` 也可以用 `dryRun: true` 只拿「会改哪些文件、各几处」的计划，不写盘、不需要确认。
  - `write` 一律写 UTF-8；覆盖一个 GBK / UTF-16 文件时结果里会说明编码已变（`read` 两种都能读，`edit` 只改 UTF-8）。覆盖一个长文件且只改了少量行时，结果会提示「用 `edit` 只发差异可以省上下文」。
  - `edit` 也能改**非 UTF-8（如 GBK）文件**，但只限 ASCII 原文：`old` / `new` / `content` 全是 ASCII 时才按字节替换（不会碰中文，也不会改变文件编码），行尾按文件自身适配（CRLF 文件用 `\n` 写的 `old` 也能匹配）；带中文的替换会明确拒绝并提示先转码；二进制文件与 UTF-16 文件明确拒绝；`replace` 的批量替换仍然只处理 UTF-8。
- 检索：`grep`（正则，逐行解码，GBK 文件也能搜到；**每个文件默认最多列 20 行**，`perFile` 可调，避免一个文件吃掉全部结果）、`symbols`（声明索引，首个调用建索引并缓存，`find: references` 查整词引用；任何写入都会让缓存失效）。`glob` / `grep` / `replace` 都可以用 `path` 把范围限定到一个目录。
  - 忽略规则按目录逐层读取：根和每个子目录的 `.gitignore`、`.ignore`，以及 `.git/info/exclude` 都生效；`node_modules`、`target`、`dist`、`.venv`、`__pycache__`、`build`、`out` 等目录不会被走进。
  - 默认权限下搜索以**当前对话目录**为根（命中是相对路径）；其它对话的目录不可达，项目目录通过只读参考根搜索。
- 版本控制：`git`（只读子命令：status / diff / log / show / ls-files 等）；`git_write`（commit / branch / stash 三种具名操作，`add -A` 与 `paths` 二选一，不做 reset、强制推送、也不会跳过 hooks；执行前确认）。两条命令都经由沙箱启动：commit 会跑仓库的 pre-commit hook，而 hook 是项目代码，不该在 jail 外执行。
- 执行：`shell` / `python` / `node`（非交互，可给 `stdin`，`background: true` 转后台任务；后台任务的 stdin 先由启动时的 stdin 填入，之后用 `task action=input` 续写）、`task`（list / output / input / close / wait / stop / stopAll；**等任务结束用 `wait`**）、`wait_for`（等端口能连上 / 页面应答 / 后台任务日志出现某段文本，默认最多 30 秒；执行前确认）、`run_tests`（自动识别 `cargo test` / `go test` / `dotnet test`（`.sln` / `.csproj`）/ `mvn -B test`（`pom.xml`）/ `gradle test`（`build.gradle(.kts)`，优先项目自带的 `gradlew`）/ `package.json` 的 test 脚本 / `pytest`，也可以用 `command` 直接给一条命令；`filter` 对 cargo / go / pytest / npm / dotnet / maven / gradle 都生效；**默认超时 300 秒**，冷构建不用重跑；返回退出码、通过/失败统计、失败用例与原因，最多列 20 条；执行前确认）、`diagnostics`（自动识别 `cargo check --message-format=json` / `go vet` / 项目自带的 `tsc --noEmit` / `dotnet build` / `mvn compile` / `gradle classes`（含 Kotlin 的 `e:` 行）/ 项目自带的 `eslint -f compact`，也可以用 `command`；同样默认 300 秒；返回错误/警告计数与「文件:行:列 代码 — 消息」清单，`severity` / `limit` 可调；执行前确认）。
  - `run_tests` / `diagnostics` 与 `shell` 走同一个 `spawnTool`：会经由 `dsh-sandbox.exe` 启动，输出逐行解码（GBK 也不乱码），运行中把最新一行实时显示在工具行上。
- 网络：`http`（GET / POST / PUT / PATCH / DELETE / HEAD；只允许 http / https，跟随最多 5 次重定向，HTML 自动转成正文，正文上限 2 万字符；`saveTo` 可把正文**原样存到文件**（最多 20 MB，不占对话上下文，下载图片后可直接 `read_image` 看）；每次请求前确认）。需要认证时用 `auth: { env: "GITHUB_TOKEN" }` **引用环境变量**：`Authorization` 默认按 `Bearer` 发送，`scheme: "basic"` 会把变量值 base64（变量里放 `user:pass`），`header: "X-Api-Key"` 这类自定义头则原样发送；**值不会出现在调用、对话、卡片或结果里**，缺变量时直接拒绝且不发请求。重定向换主机（或降级到 http）时不再带凭据，结果会写明；直接把 `authorization` 写进 `headers` 仍被拒绝。这类请求**在「完全权限」下也照样要确认**。联网搜索是 DeepSeek 端点的服务端工具，见上一节。
- 图片：`read_image`（读一张 PNG / JPEG / GIF / WebP / BMP，把图片本身附给模型；超过 5 MB 或 **20 MP**（视觉模型按面积计费与限额）会拒绝并提示先缩小）、`screenshot`（截屏，默认**前台窗口**、长边缩到 1600px、存成 JPEG；`region: all` 才截整屏，`x` / `y` / `width` / `height` 四个都给时只截那块区域（相对所选区域，用来对准某个按钮或表格），`maxEdge` / `format` / `quality` 可调，`path` 可指定保存位置）。两者只读、不需要确认；一次提问最多附 3 张、base64 合计上限 12 MB，超出的会在结果里注明。
- 浏览器：`browser`（驱动本机已安装的 **Edge / Chrome，不下载浏览器**）：`open` / `goto` 打开 http(s) 页面并给出标题与正文，`snapshot` 列出可点元素（`[编号]`），`click` / `type` / `press` / `select` 用**真实输入事件**操作（点击按坐标、输入走 `Input.insertText`、下拉会触发 `change`），`text` 读正文（上限 2 万字符），`eval` 执行 JavaScript，`wait_for` 等文字 / 选择器 / 元素出现，`console` / `network` 看控制台与请求，`screenshot` 把画面附给模型（默认视口，`fullPage` 整页，`ref` 只截某个元素，长边按 `maxEdge` 缩小并存到 `.dsh-flow/shots`），`downloads` 列出网页下载到的文件，`close` 关闭。**一个对话共用一个浏览器**：它的进程活过单个请求，页面、滚动位置和登录态都留到下一问（指针写在 `.dsh-flow/browser/session.json`；闲置 30 分钟或 `close` 才收掉，`DSH_BROWSER_IDLE_MIN` 可调）；接上旧页面时结果里会明说。**允许下载**：网页给的文件落在这个对话的 `.dsh-flow/downloads`，结果里带上路径，`close` 之后也还在；要不要发给用户由模型用 `show` 决定。**导航与输入类动作默认确认**，纯读取（snapshot / text / console / network / screenshot / wait_for / downloads）不确认。它用对话目录下的私有 profile：读不到你自己浏览器的登录态，窗口关闭时一并结束。
- 结果文件：`show` 把做好的文件**摆到对话里**——这一步下面列出文件卡片（名字、大小、扩展名），点一下用系统程序打开，旁边的「所在文件夹」在资源管理器里定位。只列已存在的文件，不读内容、不写入盘，一次最多 12 个；`.env`、密钥这类文件仍然拒绝。模型只在需要交付时调用它（网页下载的文件也一样，默认不打扰），所以读完答案不用再去工作区翻文件。
  - 上一轮回答里看过的截图会**自动带回下一问**（最多 2 张、原始文件合计 ≤4 MB，且只在这个对话自己的工作区内读取）：改完界面截一张图，追问「再往右挪一点」时模型仍然看得见。
  - 「仅可查看」下截图不留副本到工作区（改存沙箱临时目录 `.dsh-flow/sandbox-tmp/shots`），`symbols` 的索引也只留在内存里，工作区保持只读。
  - `read` / `read_image` / `grep` 都**不碰凭据与密钥文件**（`.env`、`*.pem`、`*.key`、`.ssh/`、`id_rsa` 等；`.env.example` 这类示例文件除外）：读取会被拒绝并说明替代做法，搜索会跳过并在结果里注明跳过了几个。**`shell` / `python` / `node` 同样不执行提到这些文件的命令**（按整词、`--flag=value` 与代码里的引号字符串识别，`git commit -m "fix secret handling"` 这类散文不会被误拦），拒绝信息会说明改用「让用户贴单个值」。
- 环境：`env`（只读，不需要确认）一次给出平台、node / python / git / cargo / npm / pnpm / yarn 等版本，以及目录里的项目文件（`package.json`、`Cargo.toml`、`pyproject.toml`、`.venv`、`node_modules` 等）和 `package.json` 脚本名，省掉逐个试命令。
- 外部：插件工具；MCP 服务器工具与 `mcp` 网关（连接在本次提问内保持）。
- 交互：`ask_user_question`、`todo_write`、`exit_plan_mode`、`remember`。
- 工具页（插件 → 内置插件）按「文件与目录 / 检索与索引 / 版本控制 / 执行与任务 / 网络与图片 / 交互与外部」六组展示，顶部有一个过滤框（按工具名或描述，Esc 清空），每张卡片写明当前对话里是「直接执行 / 需确认 / 按规则 / 不再询问」。**卡片的描述取自引擎上报的工具 schema**（也就是发给模型的那段说明），页面只保留一句更短的兜底文案，供首次运行前显示；两种文案不同时以引擎为准，过滤框匹配当前显示的那段。分组与过滤只影响展示，不改变发给模型的工具集合与权限规则。

### 长期记忆

- 模型用 `remember` 写一条可复用的结论（偏好、项目约定、反复踩到的坑），界面把它追加到 `<data>/memory.md`：一行一条，多行会压成一行，超过 32 KB 丢弃最旧的条目。
- 以后每次提问都会把最新的条目作为「长期记忆」块注入上下文（注入上限 8 000 字符，超出只载入最近的部分），上下文页能看到这一块。
- 想清空就直接删这个文件；如果写入失败（例如 data 目录不可写），对话里会出现一条提示，说明这次没有记下。工具开关关掉时 `remember` 也不会出现。

## 计划、待办与提问

- 输入框「+」菜单里的「计划模式」开关：开启后本次提问在计划模式下进行 —— 模型先只读调查，把完整计划提交到聊天里的计划卡；点「批准并执行」后才开始改动，批准后计划模式自动关闭；点「继续修改」则让模型修订后再次提交。
- 模型可以调用 `todo_write` 写入本次对话的待办列表。待办卡在对话标题下方，显示完成数 / 总数，点开可看每一项的状态；列表随对话保存，重开仍在。
- 模型可以调用 `ask_user_question` 提一个带 2–6 个选项的问题。聊天里出现选项卡，点一个选项即回答，模型收到「用户选择：…」后继续。
- 「+」菜单里的「自动审查」开关（「工作区内修改」下）在「逐次询问 / 自动审查」之间切换。
- 自动审查的放行规则：内置只放行普通的工作区内写入和只读命令（`git status/diff/log/show`、`cargo check/test/clippy/build`、`dir/ls/type` 等），以及只轮询的 `wait_for`；含 `&&`、`|`、`>`、`$(…)` 的命令、删除、发布、插件 / MCP 调用仍然逐次询问。
- 审批卡的「本次会话都允许」不只用于命令：`write` / `edit` 的卡片也提供它，记住的是**这个文件的路径**（`write` 与 `edit` 共用，`./a.txt` 与 `a.txt` 视为同一个），同一文件连续小改只问一次；换一个文件仍然会问。`replace` 涉及多个文件，不提供这项。
- 想让自己常用的命令也免确认：设置 → AI → 「自动审查允许的命令」里按前缀填（逗号分隔），例如 `npm test, npm run build, pytest`；`npm test` 也覆盖 `npm test --watch`，但不会放行 `npm install`。前缀等于信任它后面的所有参数，所以别填 `npm` 这种过宽的前缀。它保存在 `config.json` 的 `reviewAllow`。
- 输入框只留「+」、权限、模型、发送四个控件；其余按对话生效的设置都在「+」菜单的「本次对话」里，一项一个开关或一排选项：计划模式、自动审查、联网搜索（默认开启，可在本对话关掉，并可「回到默认」）、轮数上限（6 / 12 / 24）、工具调用上限（20 / 40 / 80）。这些都随对话保存进 `chat.json`。自动审查只放行普通的工作区内写入和只读命令（`git status/diff/log/show`、`cargo check/test/clippy/build`、`dir/ls/type`）；含 `&&`、`|`、`>` 的命令、删除、发布、插件 / MCP 调用仍然逐次询问。轮数与工具调用默认取上限（24 轮 / 80 次工具调用）。

## 权限与沙箱

- 「仅可查看」和「工作区内修改」下，`shell` / `python` / `node`、后台任务、`run_tests` / `diagnostics`、`git` / `git_write` 与 `screenshot` 都通过程序旁边的 `dsh-sandbox.exe` 启动：Windows 低完整性令牌 + Job Object，读全盘、网络照常，写到允许范围外的位置会被系统拒绝；停止 / 关闭会结束整棵进程树。`browser` 是唯一例外：Chromium 在低完整性令牌下根本起不来（`CreateProcessAsUser` 直接失败，实测），所以它用对话目录里的私有临时 profile 在沙箱外运行，下载被禁止、不读用户浏览器的配置，页面侧操作仍逐个走确认。
- 默认权限下**文件工具与命令的边界是同一个**：相对路径从当前对话目录开始，`..` 与外面的绝对路径都被拒绝；`glob` / `grep` / `tree` / `symbols` 也以它为根。项目源码通过引擎/工作流参考目录只读可见。切换「完全权限」同时解除文件范围与沙箱。
- 工作区模式把当前对话目录设为低完整性的可写区域；仅可查看模式只给私有临时目录写权限，工作区本身保持不可写（从工作区模式切回来时会恢复）。截图与符号索引在只读模式下也按这条规则处理；`run_tests` / `diagnostics` 会写构建产物，所以在只读模式下直接说明原因、不弹确认卡。
- 「完全权限」不走沙箱。`dsh-sandbox.exe` 缺失或 `--check` 不通过时自动退回原有的路径检查 + 审批卡；若系统在真正启动命令时才拒绝受限令牌，命令会明确报 `SANDBOX ERROR`，不会静默放行。唯一例外是**带凭据的 `http` 请求**：它永远弹卡，不受「完全权限」和「自动审查」影响。
- 这是“系统级写隔离”，不是 AppContainer：它不隐藏磁盘其他位置，也不拦网络。自动审查假定工作区代码可信；不可信仓库仍建议逐次询问。

## 外部插件

插件放在 `<home>/plugins/<id>/`，入口由 `plugin.json` 的 `main` 指定，必须导出 `activate(api)`。设置页的“模式”决定每次对话向模型发送哪些外部插件、Skill 和 MCP：基础模式不带外部资源，全量模式带所有手动启用的资源，自定义模式按三个列表勾选；在各自页面关闭的资源始终不会发送。仓库里的 `plugins/win32-windows/` 是只读 Win32 窗口检查插件：

- 需要 Python 在 PATH 中；
- 可调用 `win32_list_windows` / `win32_window_info` 枚举 HWND；
- `win32_observe` 用 UI Automation 读取控件树，返回元素编号、名称、类型、中心坐标、密码框标记和 45 秒快照；
- 只读，不点击、不修改。

另有 `plugins/office/`：`office_open` / `office_status` / `office_read` / `office_find` / `office_edit` / `office_save` / `office_close` / `office_help`，通过 Windows Office COM 直接操作已打开的 Word/Excel/PowerPoint。Word 支持段落、表格、样式、页面设置、页眉页脚与页码域、目录、分节符、脚注、超链接、批注、文档属性；Excel 支持单元格、公式、格式、排序筛选、条件格式、下拉校验、合并拆分、行列增删、定义名称、工作表与图表（日期按显示文本读写，`dates` 可切换序列号）；PowerPoint 支持按版式建页、占位符文字与格式、文本框、项目符号、表格、图片、形状、背景、页面尺寸、备注、移动/复制/删除页。`office_read` 一次给出 Word 标题大纲/字数页数/批注、Excel 各表范围/末行末列/名称/图表与 PowerPoint 每页形状/表格/备注。支持搜索（`office_find`）、`dry_run`、保存/另存为/PDF 和受限制的关闭。需要本机安装 Microsoft Office 或 WPS，默认不启用。

另有 `plugins/win32-input/`：`win32_focus` / `win32_click` / `win32_scroll` / `win32_type` / `win32_keys` / `win32_drag` / `win32_wait`。优先接收 `win32_observe` 的 `snapshot` + `element`，拒绝过期快照和密码框输入；默认不启用，所有操作都需审批，并支持 `dry_run` 先验证不执行。

另有 `skills/`：可导入的 Skill 源码。`skills/workflow-distill/` 把「聊天里跑通后怎么固化」写成固定规程（先 `dsh-flow distill`，再按判据 review、折叠、用 `validate` / `test` / `run` / `explain` 验收）。在 Skill 页点 +，选中该文件夹即复制到 `<home>/skills`，开关打开后才进入每次提问。

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
| `Ctrl+R` | 运行工作流（工作流页） |
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
- 对话：`<data>/chats/<dir>/chat.json`，该对话自己的请求明细在 `requests.jsonl`。工具行的参数与输出在 `chat.json` 里**每种最多保留 8 000 字符**（超出标注「已截断，原文 N 字符；完整内容见请求明细」），否则一个工具密集的长会话会让每次读写都变慢；完整文本仍在 `requests.jsonl` 里。保存时只重写**这一轮改动过的**对话（按每条对话的修订号），其余文件不动。
- 统计账本：`<data>/stats/requests.jsonl` 保留最近 30 天请求明细；`<data>/stats/daily/*.json` 保存按 UTC 日的永久汇总，删除对话不会改变历史统计。明细与日汇总都按工具记次数（`toolCalls`），统计页的「工具分布」卡片按调用次数排序；旧记录没有工具名时该卡片留空，但总数照常统计。
- 后台任务：`<data>/chats/<dir>/.dsh-flow/tasks/`。
- 备份：`<home>/backups/`；导出：`<home>/exports/`；日志：`<home>/logs/paagent.log`。
- 备份会复制**可执行文件同目录的 data 目录**以及技能、插件、MCP、config.json，但不复制 workspace。
- “恢复最近备份”先在下次启动时执行：启动时会自动备份当前状态，再用备份替换 data 与附加组件；当前连接与路径设置（config.json）保持不变。
- API Key 不写入 `config.json`（迁移后由 Windows Credential Manager 保存）。

## 约定

- 聊天写盘使用原子写入；删除对话前会做备份/可回滚检查。
- 后台任务结束前校验进程启动时间，避免 PID 复用误杀。
- 工作流必须显式声明输入；模型步骤建议提供 `why` 与 `schema`。