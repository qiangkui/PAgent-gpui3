# 更新记录

## 2026-11-27（代码审计：筛选、状态与运行安全）

- **分类筛选会丢工作流**：侧栏按分类展开时，如果某个分类只有工作流、没有会话，搜索代码把它跳过，
  而它的工作流卡已经从 map 里取出，导致筛选结果里看不到。现在按“会话 + 工作流”判断分类是否存在，
  分类计数也改为两者相加；搜索时不再显示“扫描根下没有 flow.yml”的误导提示。
- **运行中切工作流会串台**：`pane()` 始终显示正在运行的那一个，但侧栏却可以切到另一个工作流，
  标题、输入和右栏会配错。现在运行中切到别的工作流会提示先停止。
- **凭据交接失败被吞**：把 Key/secret 写进引擎 stdin 失败时原来直接继续，后续只报 “no api key”；
  现在立即终止子进程并把真实错误返回给窗口。
- **清理过期清空标记**：`refresh_workflows` 在清单删除后同步清理 `flow_cleared`，避免
  `sidebar.json` 的 `cleared` 越积越多。
- **历史不再只显示 6 条**：内存上限从 12 提到 30，启动与打开工作流都读取该工作流磁盘上保留的全部
  30 份快照；更早的快照本就已被引擎按上限裁掉。
- **右栏源码滚动条可拖动**：复用对话里代码块的 `tool_drag` 状态，点击轨道跳转、按住滑块拖动；
  滚轮/触控板仍然可用。
- **错误信息分类**：新增 `RunErrorKind`，把运行前检查、引擎失败和步骤 `ErrorView` 统一归为
  凭据、输入、运行环境、清单、网络、模型、超时、登录、页面、步骤执行、校验、引擎、取消等类别；
  banner 显示“类别：原因”，transcript 的错误块显示类别和可执行建议，不再只看到英文 kind 或一句裸错误。
- 测试：Rust **662 通过**、`clippy --all-targets` 干净。

## 2026-11-27（空白页标志与工作流分栏圆角）

- **空白对话只留标志和一句话**：删掉「开始和助手对话」、示例问题、权限说明、目录提示和快捷键提示；
  新页面居中显示 56px 的 PAgent 标志与「请使用PAgent开始你的工作。」。
- **侧栏层次**：“工作区”下的搜索框、分类行和新分类行统一左缩进 20px；分类下没有会话时不再显示
  「这个分类还没有对话」。
- **项目/可执行程序图标**：用标志生成了 16–256 多尺寸的 `assets/app.ico`（蓝底白色 P + 节点），
  `build.rs` 通过 `embed-resource` 编译 `assets/app.rc`，把图标嵌到 exe 的资源 ID 1；GPUI 的 Windows
  窗口类会加载它作为任务栏 / Alt-Tab 图标。标题栏左侧图标也从机器人换成了同一个 `logo.svg`。
- **工作流右栏标题**：右栏顶部运行条左侧新增「步骤」面板标题，和右侧运行/停止/新建按钮同一行；
  草稿警告保留为中间可截断的一行，不额外增高。
- **修复“新建运行后重启又出现上次运行”**：「新建运行」的清空状态以前只在内存里，重启后
  `select_flow` 又把最近一次运行选回来。现在把已清空的工作流名单写进 `sidebar.json` 的 `cleared`，
  启动时载入；新记录落盘时再从名单移除。开始运行也不再提前清掉这个标记，避免运行中崩溃/关闭后
  重启又恢复旧结果。
- **插件页移除路线图卡**：删掉「工作流（未实现）」标题和 `approve / pause`、`fetch 录制回放`、
  `use 子流程`、`state 游标` 四张卡；插件页只保留内置工具和外部插件，不再把引擎路线图当成未完成插件。
- **工作流分栏**：去掉左栏与右栏之间的 1px 分割线；左栏右上、右栏右上改为直角；右栏自己铺
  `chrome` 底色并加左上圆角，圆角缺口露出下面的 `canvas`。历史标签没有右栏，仍保留页面右上圆角。
  分栏宽度计算不再为分割线预留 1px。
- **清单测试去外部依赖**：`a_manifest_names_its_secret_consumers` 改用内联清单，不再 `include_str!`
  用户工作流目录；工作流被重命名/删除时测试不会失败。
- 测试：Rust **659 项通过**、`clippy` 干净；资源测试覆盖新 `icons/logo.svg`。

## 2026-11-27（/固化为工作流：整条指令、无草稿、删除即移走）

- **`+ → 固化为工作流` 改为插入整条指令**：输入框出现 `/固化为工作流`，在指令尾部按 Backspace 会一次删掉
  整条指令；按 Enter 才发送。发送时界面把短指令显示在对话里，模型收到的是带聊天路径、扫描根与 Skill 规则的
  展开任务；**界面自己不再生成草稿卡片**。没有可用模型时只提示去接模型，不生成草稿。
- **热加载**：Agent 最终把 `flow.yml` 写进 `workflows/<name>/` 后创建空文件 `.show`；界面在回答结束（和启动时）
  重扫，发现标记就取消该名字的隐藏并删除标记，卡片立即出现。system prompt 与 Skill 都写明了这条协议。
- **删除卡片会移走文件夹**：卡片菜单的「删除工作流（含文件夹）」二次确认后，把工作流文件夹移到
  `backups/deleted-workflows/`（不在 `workflows/` 里，也不再被扫描），并清掉分类、隐藏、别名、运行记录与
  store 快照；「仅从工作区移除」保留为只隐藏的次要动作。
- **发现规则**：`backups/`、`flows/`（子流程）与 `out/` 一样不参与工作流发现；`flows/login.yml` 不会再作为
  一张可删除的卡片出现。
- **恢复**：`workflows/units-export` 的旧 9 步版本在磁盘上被删除后，已用 Agent 的 7 步版本
  （`backups/duplicate-maers-units-export-20261127/`，运行成功、含文档与 fixtures）恢复，并补 `.show`。
- 测试：Rust **659 项通过**、`clippy` 干净；release 重新构建。

## 2026-11-27（工作流卡片与固化流程收口）

- **移除「已从工作区移除 N 个工作流」提示**：它占着侧栏位置又删不掉。启动与每次重扫会把
  清单已不存在的隐藏名清掉（过期草稿/副本不再压制列表），也不再渲染这条提示。
- **工作流卡片可重命名**：卡片菜单新增「重命名」，复用对话的重命名弹窗；别名只存在
  `sidebar.json` 的 `labels` 里（manifest 不改名），运行记录、发现与同名规则都不受影响。
  侧栏卡片与工作流页头都显示别名。
- **固化为工作流改为 Agent 一次做完**：菜单里选一轮后不再直接生成草稿卡片，而是把这一轮的
  任务发给 Agent：中间 `distill` 产物只放目标工作流的 `out/draft/`（不被扫描），实现占位、
  补测试、`validate` / `test` / `run` / `explain` 全过后才把最终 `flow.yml` 写进
  `<扫描根>/workflows/<name>/`。只有没有可用模型时才回退到确定性草稿。
- **回答结束后自动重扫工作流**：Agent 在回答里新建/更新的工作流会立刻出现在左侧，不用重启；
  重扫按名字保留当前选中项。
- **units-export 运行产物按次分目录**：`paths` 步骤每次新建 `out/run-<时间戳>/`，CSV 与 xls
  副本互不覆盖；README 补了“代码与运行产物分开”。
- 测试：Rust 657 项、units-export 15 项、`validate` 通过；release 重新构建。

## 2026-11-27（长对话：预算上限与选轮次）

- **为什么 100 多轮停不下来**：单次提问的预算此前最高 24 轮 / 80 次工具调用（引擎本身允许 50 / 200），
  长任务每一轮都在半途被截断，最后一轮还可能把未解析的 `<｜｜DSML｜｜>` 调用原文当成回答。现在界面的可选值扩到
  `6/12/24/50`（轮数）与 `20/40/80/200`（工具调用），默认值随之变为 **50 / 200**；已有对话保留自己保存的
  值，在输入框 + 菜单里改。
- **固化为工作流改成选一轮**：菜单列出最近 6 个有工具调用的提问（`第 N 轮 · M 次工具调用 · 问题首行`），
  只有一轮时直接执行。`--turn` 真正传给引擎；落点 `workflows/draft-<chat>-t<N>/`，卡片名
  `标题（草稿·第N轮）`，重复点击加序号。Note/toast 都写明第几轮。
- 测试：Rust 654 项（新增 `distill_turns` 两项与轮次断言）；`cargo check` / `clippy` 通过。
- **草稿标记按占位判断**：卡片、页头与运行条提示不再看名字，改看清单里是否还有 `skip` 步骤；名字里的
  「（草稿·第N轮）」在该草稿的占位全部替换后自动隐藏（不改写清单）。新增 `StepSpec::is_skip` /
  `ManifestSpec::placeholders` 与显示名测试；Rust 656 项通过。
- **Skill 补信息边界与最小模板**：固化只读 `chat.json` / distill 产物 / 目标工作流 / SOP / 引擎 README 的清单与
  步骤协议，**禁止为固化去读** `dsh-flow/src`、`src/app` 等产品源码；一个 `name` 只允许一份工作流（先找同名再改，
  不新建副本）；给出 `flow.yml` + 步骤程序 + fixture 的最小模板与约 15 次工具调用的预算。
- **清理同名副本**：一次固化在 `dsh-flow/workflows/` 另建了同名 `maers-units-export` 并读了引擎源码；副本已移到
  `backups/duplicate-maers-units-export-20261127/`。`engine::discover` 现在跳过 `backups/`（归档的清单不再变成卡片），
  并补了对应测试；Rust 657 项通过。

## 2026-11-27（输入框「固化为工作流」）

- **输入框 + 菜单**：「指令」区在「压缩上下文」下面新增 **固化为工作流**（`icons/layers.svg`）。点击后把当前对话的
  `chat.json` 交给引擎 `distill`，在工作线程里生成草稿；回答进行中、没有工具调用、同一对话正在固化时用 toast 说明。
- **落点**：`<扫描根>/workflows/draft-<chat-N>-t<N>/`，三份产物（`flow.yml` / `actions.json` / `distill.md`）。
  成功后将 `flow.draft.yml` 改名为 `flow.yml`，工作流页因此能立刻发现；名字取对话标题 +「（草稿·第N轮）」。
  同一轮再点一次另开一份（`（草稿·第N轮 2）`、`draft-chat-1-t1-2`），**不覆盖**上一份——它可能已经被人 review 或改过。
- **反馈**：成功后对话里留一条 `Note`（不会被回放给模型）：相对路径、步数、待实现占位数，并指向 Skill「工作流固化」；
  失败也留一条带原因的红字 Note。工作流列表在成功后重新扫描，左侧工作区出现新卡片。
- **修 `distill --name`**：显式名字原样保留（此前被 slug，中文名会落空并回落 `chat-distill`）；只有缺省时才从标题生成
  ASCII 名。引擎与 CLI 各补一条测试。
- **草稿标识**：工作流卡片显示「草稿 · N 步」并把状态点改成警示色；草稿的运行条上常驻一句
  「skip 占位不会执行，先把占位实现成步骤再运行」。成功后的 Note 补上 `distill.md` 报告路径，并明确
  「草稿不能直接运行」——`validate` 通过只说明结构合法，不代表能跑。
- **文件**：`src/app/distill.rs`（计划/命令/后台/结果），`src/app/composer.rs`（菜单行），`src/app.rs`（`distilling` 状态与模块），
  `src/app/sidebar.rs`（草稿标识），`src/app/flow_board.rs`（草稿运行提示）。
- **测试**：Rust 4 项（中文标题命名、同名/同目录不覆盖、提升拒绝覆盖、`flow.yml` 能被 `engine::discover` 发现且
  `skip` 占位可数），引擎 `distill.test.ts` 8 项、`cli.test.ts` 3 项；`cargo test` / `cargo check` / `npm run check` 通过。

## 2026-11-27（聊天记录固化：distill）

- **新增 `dsh-flow distill <chat.json>`**：窗口保存的聊天记录是一份 JSON 文档（`messages` 里混着助手文字、
  思考与工具行），`derive` 读不了（实测 `0 run · 1 skipped`）。新命令只做确定性的整理：`shell` 命令 →
  `run` 步骤（同参相邻重复 → 1 步 + `retries`）；页面动作/请求/文件改动/交付 → `skip` 占位；探索
  （`list`/`glob`/`read`/`grep`）与页面探测（`browser eval`/`snapshot`/整页截图）丢弃并计数；同名同参
  但被别的步骤隔开的调用不合并，在报告里点名。收尾结论与元素截图列为判断点候选，**不自动生成 ai 步骤**。
- **产物三份**：`flow.draft.yml`（`validate` 可过）、`actions.json`（参数/结果/处置/原因）、`distill.md`
  （动作表、合并与丢弃、判断点、产物与下一步）。`--turn` 选轮次（默认最后一轮），`--name` /
  `--description` 覆盖名字。代码在 `dsh-flow/src/distill.ts`，CLI 帮助与 `dsh-flow/README.md` 同步。
- **真实 `chat-1` 实测**：21 次工具调用 → 7 步（2 命令 + 5 占位）、14 次丢弃（3 探索 + 11 探测）、
  1 组同参被 `edit` 隔开（报告点名）；草稿 `validate` 通过，`--dry` 全部按预期跳过。
- **测试**：`tests/distill.test.ts` 7 项（合并重试、轮次选择、空轮占位、多行命令、隔开的重复、
  视口/元素截图、截断参数不执行）；`cli.test.ts` 端到端（生成草稿 → `validate` → `run` 跳过占位）。`npm run check` 通过。
- **新增 Skill `skills/workflow-distill/`**：把「跑通后怎么固化」写成固定规程（distill 产物 → 保留/丢弃判据 →
  折叠成步骤脚本 → 判断点与契约 → `validate` / `test` / `run` / `explain`），只定口径、不执行；Skill 页用 +
  选中该文件夹导入。仓库副本由 `src/skill.rs` 的同名扫描器单测校验（`cargo test the_shipped_workflow_distill`）。

## 2026-11-27（验证码 AI 介入实测与修复）

- **真实站点验证码补测**：用本机界面里已存的 `deepseek-flash` 接入点，先用同站 `/captcha` 图片读出
  `5496`；随后真实 `captcha` 步骤截图交给 `units-export/flows/login` 的 `read` AI 步骤，读出 `8410`
  （`readable: true`）。
- **修元素截图坐标**：`Browser.selectorBox` / `ref` 截图把 `DOM.getBoxModel` 的视口坐标当页面坐标用，
  页面滚动后截到空白区域（验证码截图只有几百字节）。现在两处都加 `scrollX/scrollY`；真实验证码截图
  从空图恢复为可读的 PNG，浏览器 e2e 测试也加了一个「按钮在折叠下方」的回归用例。
- **登录子流程 `read` 调参**：`maxTokens` 200 → **4000**（`deepseek-flash` 的思考 token 计入上限，
  200 甚至 1500 都会在输出文本前耗尽）；prompt 从「干扰线压字就拒绝」改成「线不切断/遮住主体就照读」，
  只拒绝真正粘连或二义，否则该站每张验证码都会被判为不可读。
- 测试：`npm run check` 通过；`npm test` 461 通过、0 失败；`DSH_BROWSER_TEST=1` 的浏览器 e2e 通过。
- 未提交登录：本机没有站点账号/密码，空凭据可能触发站点锁定次数，所以停在读码结果，未做密码尝试。

## 2026-11-27（第十轮：空左栏居中提示）

- 当前标签没有运行可看时（初始、或点过「新建运行」），左栏改为上下左右居中的一句
  「请点击运行开始」；浅灰、`text_lg`、微软雅黑、加粗，替换原来的两行左对齐说明。
- 历史标签的空列表仍显示「还没有历史运行」。
- 测试：`cargo test` 646 通过、`cargo clippy --all-targets` 干净。

## 2026-11-27（第九轮：步骤单线宽行、运行按钮三态、新建运行）

- **右侧步骤改为单线宽行**：去掉填充底色和阴影，改成铺满板宽、1px 边框、圆角的宽行；点开某一步时
  边框变成天蓝色。
- **运行控制收敛为一个按钮**：初始/新建后显示「运行」；运行中只有「停止运行」；一次运行结束后只有
  「新建运行」。原来的「报告」和「回到计划」按钮删除，顶部不再有输入计数提示。
- **新建运行**：点「新建运行」把刚才那次运行归入历史并清空左栏当前标签，右侧输入恢复可编辑；
  在点击它之前，当前标签一直显示上次运行。该状态按工作流记忆，开始下一次运行后解除。
- 测试：`cargo test` 646 通过、`cargo clippy --all-targets` 干净。

## 2026-11-27（第八轮：历史阅读列限宽、历史记录卡片化）

- **历史内容限宽居中**：历史标签下左栏仍占满下方区域，但正文按聊天页的阅读列排版——最宽 900px，
  更宽的窗口把剩余宽度平均分到左右两侧留白；历史列表、打开的历史记录、长输出和图片都遵循同一列宽。
- **历史运行记录改成卡片**：每一条从下划线行改为圆角卡片（`panel` 底色、1px 边框、浅阴影、8px 间距），
  悬停变底色，整张卡片可点进入该次运行。
- 测试：`cargo test` 646 通过、`cargo clippy --all-targets` 干净。

## 2026-11-27（第七轮：历史隐藏右栏、标签/标题对齐聊天页）

- **「历史」标签下不再显示右侧源码板**：只有「当前」保留左右两栏；历史列表和打开的历史记录都只占
  左栏，左栏按整块下方区域的宽度排版（文本、输出、图片的换行宽度同步放大）。
- **「当前 / 历史」改成聊天页标签样式**：24px 高、`text_xs`、灰色选中文字与下划线、未选更淡、
  悬停变深；标签带下方是 1 个设备像素的 `chrome` 细线。
- **工作流标题改成聊天页标题样式**：`text_xl` 加粗、左对齐、状态 chip 紧随；右侧「打开文件夹」
  改成 24px 的图标按钮（16px 图标、同款悬停底色）。
- 测试：`cargo test` 646 通过、`cargo clippy --all-targets` 干净。

## 2026-11-27（第六轮：标题收紧、步骤间距、产物缩进、外壳圆角与底色）

- **标题区变矮，标签贴底**：上下内边距改为 `pt_3 / pb_0`，标题从 `text_xl` 降到 `text_lg`，描述限一行；
  「当前 / 历史」不再单独占一行，标签下沿正好落在标题区底边。
- **步骤间距收紧**：步骤容器改为 `pt_1 / pb_0`，左栏行间距从 8px 降到 4px；上一步结尾到下一步标题
  只剩十几像素，不再出现大片空白。
- **产物一行一个、二级缩进**：产物列表改为纵向；每项在步骤内容的一级缩进上再缩进 20px。文件卡片
  铺满一行（名字自动占剩余宽度），图片仍按自身宽度显示。
- **删除最底部状态条**：`render_status` 的调用与实现移除；运行摘要只留在左栏的运行结束行。
- **下方区域外壳**：标题以下包一层标题栏 `chrome` 色的圆角容器（左上/右上圆角并裁剪内部内容）；
  左栏单独铺 `canvas`（浅色主题下为白色）且右上圆角，形成灰底上的一张白栏卡片，右栏保持灰底。
- 测试：`cargo test` 646 通过、`cargo clippy --all-targets` 干净。

## 2026-11-27（第五轮：标签进标题、带标签折叠输出、一级缩进、卡片收起）

- **「当前 / 历史」标签并入标题区**：放在标题与描述下面，删掉左栏顶部原来那条带边框的独立标签行；
  标题区下边框继续做两栏的分隔。
- **左栏内容像聊天工具行**：步骤标题最靠左，该步产生的内容统一缩进一级；摘要 / stdout / stderr /
  结构化错误 / 产物各有彩色标签（结果 / 输出 / 错误输出 / 错误 / 产物），超过一行或 40 字折叠为
  一行预览，点「▸ 展开」在缩进框里看全文（最高 320px、可滚动、可选文本），「▾ 收起」收回。
- **空错误不再出现**：结构化错误只拼接非空字段，全空时整行不画；运行结束的「原因」同样只在有内容时显示。
- **产物按聊天界面展示**：文件卡 = 扩展名徽标 + 名字 + 大小 + 「所在文件夹」；图片直接显示缩略图
  （最高 480px）与文件名。
- **右栏卡片收起更干净**：只保留标题、描述与该步必要的输入框；计划说明、条件/保存/备注和源码
  都在展开后显示。
- 删除 `detail_block` / `DETAIL_LINES`（折叠代替截断）。
- 测试：`cargo test` 646 通过（新增 `error_text` 两条）、`cargo clippy --all-targets` 干净。

## 2026-11-27（第四轮：当前/历史标签、文件夹按钮、无横线、源码框滚动）

- **左栏改成两个标签「当前 / 历史」**：当前显示运行中或最新一次结束的运行；历史列出该工作流的所有
  运行（状态、相对时间、耗时、步数、run id、输入摘要），点一条进入它的输出、左上返回列表。
  原来横跨两栏的「最近运行」chips 删除；右栏在历史列表时回到计划输入，打开某条历史后按该次快照只读。
  运行在飞时打开历史记录，也以历史记录为准。
- **标题区去掉清单路径**，右侧新增「打开文件夹」图标按钮（`icons/folder-open.svg`），点击打开清单所在目录。
- **步骤/运行提示去掉左右分割线**：只剩圆点 + 彩色文字 + 细节，聊天式流不再被横线切成表格。
- **右栏源码框重做**：改用 `SelectableText` + 语法高亮 token，强制自动换行、最高 420px、纵向滚动，
  右侧有一条随滚动移动的细拇指；不再用 markdown 代码块（横向滚动 + 行号槽会把左侧遮住）。复制按钮保留。
- 删除：`render_run_history`、`RunRecord::color`、`Pane.manifest_label` 等不再使用的残留。
- 测试：`cargo test` 644 通过，`cargo clippy --all-targets` 干净；release 重编并启动。
- 备注：细滚动条目前只反映位置（滚轮/触控板滚动，拖动未接入）；历史列表只含已加载的记录
  （内存 12 条 + 打开工作流时补 6 条），更早的在 `data/workflows/<name>/runs/` 里。

## 2026-11-27（第三轮：右栏静态源码板 + 左栏聊天式运行流）

- **右栏变成静态源码板**：卡片不再显示状态、耗时、摘要、输出、错误或产物，也不再与左栏联动；
  点开一张卡看的是**这一步的源码**——清单里该步的原始行（带行号）与它运行的程序文件（或
  `use` 的子流程清单），用聊天同款 markdown 代码块高亮、可复制；超过 800 行截断并注明。
- **清单解析补源码坐标**：`StepSpec` 新增 `start_line / end_line`（YAML，1-based）、
  `program`（命令里第一个源码文件，如 `steps/units.py`）与 `child_manifest`（`use.manifest`）；
  `units-export` 的 `paths → steps/units.py`、`login → flows/login/flow.yml` 有测试断言。
- **左栏改成聊天式逐条输出**：删掉运行块卡片与“日志”折叠（stdout/stderr 本身就是日志），
  进入一步是一条带分割线的突出文字（序号 / id / kind / 状态 / 耗时），下面是摘要、markdown、
  原始输出、产物与错误；运行开始/结束各一条分割行。左栏点击不再选中右栏。
- **移除重复的日志 UI**：`render_log`、日志窗格、`Ctrl+L`、`log_scroll`、`log_color` 删除；
  引擎日志仍保留在运行快照里，供复盘与排查。
- 测试：新增清单行号/程序/子流程断言、`program_path` 取词规则、`clip_code`/`code_language`；
  `cargo test` 全绿，`cargo clippy --all-targets` 干净。
- 验证：`smoke-ui` 实跑通过（快照继续落盘）；右栏静态卡片与运行后的只读输入由截图核过。
  源码面板的“清单片段/程序文件”由单测覆盖；点击展开的自动截图在会话后段受截图读取限制，未能留存。

## 2026-11-27（输入卡）

- **`secret` 输入在屏幕上也是密码**：画成 `••••`（一个字符一个点），方向键 / 全选 / 删除 / 粘贴照常，
  光标按点在屏幕上的位置走，点哪就是点到的那一个字符；接入点的 API Key 沿用同一套掩码。
- **「按住显示」**：按住才露出明文，松开、指针移出、输入框失焦、切换工作流、开始运行都会立刻收起。
  按钮按下时拦住了窗口背景的「点一下丢焦点」，否则按下瞬间就会把刚显示的值收回去。
- **本机保存 / 清除**：复用 Windows 凭据管理器的 `SecretStore`，键空间扩到
  `dsh-flow-ui/workflow:<清单名>:input:<输入名>`（名字里的 `:` / `/` 折成 `_`）。重开窗口自动填回并亮
  「本机保存」；改了内容徽标就消失；「清除」删库并清空输入框，免得下一次运行继续用被删掉的值。
- **这个值喂给哪几步**：清单解析顺手收集 `{{ inputs.x }}`、`{{ inputs['x'] }}` 与 `use.inputs` 的转发，
  卡片第二行写「用于 N 步 · paths, login」；没人读的 secret 明说「清单里没有步骤读它」。
- **运行前校验**：必填为空时卡片实时标红；点「运行」会列出缺哪些输入并直接返回，**不启动子进程**。
  默认值、类型、路径仍由引擎判定；引擎依旧是最终裁判。
- **占位符**：secret 输入不再显示「default」（它本来就不许有默认值），改成「只在界面上填写」。
- **输入卡不再被窗口裁掉**：卡片改按窗口宽度显式分组（`input_columns`），不再依赖 `flex_wrap`——
  Taffy 在滚动面板里把那一行按 max-content 量，第三张卡会跑出窗口右边。宽窗三列、窄窗一列，
  都用截图核过。
- **文档**：`docs/工作流卡片-界面设计.md` 更新 §0 / §1 / §4 / §10 / §11，新增 §13 记录这一轮，
  §12.6 记下顺序更正（先做掩码与本机保存，再回头做共享行渲染器与运行块）。
- 测试：`secrets` 的键命名与 MemoryStore 往返、`manifest` 的 YAML/JSON 引用扫描与去重（含真实
  `workflows/units-export/flow.yml`）、掩码与光标映射（含 CJK 与 emoji），`cargo test` 全绿；
  `cargo clippy --all-targets` 干净。

## 2026-11-27（第二轮：运行对话 + 步骤板 + 运行快照）

- **工作流页换骨架**：左边是**运行对话**（复用聊天的 markdown / 图片 / 文件卡 / 错误块 / 日志折叠，
  只读、程序驱动），右边是**步骤板**（通栏步骤卡 + 运行条 + 点开详情）；底部日志窗口删除，原始引擎
  日志收进运行块折叠。`src/app/transcript.rs`、`src/app/flow_board.rs` 新增；旧的轨道/检查器/日志
  （`render_steps` / `render_step_panel` / `render_log`）删除。
- **输入长在步骤卡上**：第一个读取某个输入的步骤拥有可编辑字段（按清单 `{{ inputs.x }}` 与
  `use.inputs` 推断），其余步骤显示只读值，没人读的进顶部「运行前」卡；同一输入只有一个可编辑实例
  （两个会抢同一个槽位的 bounds）。`units-export` 的 `paths` 卡带 workspace/out_dir，`login` 卡带
  account/password/vision_model。
- **每次运行的完整快照**：新增 `src/run_store.rs`，运行结束写
  `data/workflows/<name>/runs/<runId>.json`：每步 `stdout/stderr/text/artifacts/error`、整段日志、
  输入摘要（密钥已 `***`）、耗时与退出码；每个工作流留 30 份。启动与选中工作流时读回，「最近运行」
  可以直接复盘上一次。
- **最小工作流 `workflows/smoke-ui`**（4 步、约 0.3 秒）：set → 写 md/csv/png 并报告产物 →
  `allowFailure` 的结构化失败 → 收尾；PNG 由脚本现场编码（96×48），不依赖外部图片。
- **冒烟实际跑出来的四个问题，已修**：① 产物路径必须相对清单目录，否则引擎静默丢弃（`smoke.js`
  曾写到 `steps/out`）；② 1×1 PNG 在缩略图里看不见；③ 左栏滚动停在中段（开始/结束/切换时归零）；
  ④ 字段列宽漏减了板自身的 `px_4`，两列排不出来。
- 测试：`run_store`（往返、裁剪、损坏文件、`-` 运行号落成时间戳）、`ManifestSpec::owner`、
  `input_columns` 的新宽度；`cargo test` 全绿，`cargo clippy --all-targets` 干净。

## 2026-11-26（工作流卡片：事件流、输入卡、错误卡）

- **`run --events`：界面终于能"边跑边看"**。引擎在 stdout 上打 NDJSON：`run/start`、`step/start`、
  `step/end`（带 `status` / `durationMs` / `command` / `summary` / `artifacts` / `error`）、`run/end`
  （带 verdict 与总耗时），最后一行仍是原来的汇总对象；人读的进度继续走 stderr。`--events` 与 `--json`
  互斥，旧版引擎（没有这个标志）的解析路径原样保留。
- **步骤可以描述自己的结果**：步骤 stdout 的三个保留键 `summary` / `artifacts` / `error` 会被引擎抬进
  结果、trace、事件与 HTML 报告。`artifacts` 最多 12 个、路径相对清单目录、必须存在、密钥类文件被拒、
  大小由引擎补算；`error.kind` 限九种（`credential` / `config` / `selector-miss` / `regex-miss` /
  `image-unreadable` / `assert` / `timeout` / `model` / `engine`），带 `hint` / `where` / `expected` /
  `gotten`。退出码的含义没变：失败仍然非零。
- **`secret` 输入成为一等公民**：清单里 `secret: true`，不许写 `default`（加载即报错），不许走 `--input`
  （进命令行与日志）；只认 `--secrets-stdin` / `--credentials-stdin` 的 `{"secrets": {…}}` 或
  `DSH_SECRET_<NAME>`。引擎在日志、trace、步骤输出、`vars`、汇总与 HTML 报告里统一替换成 `***`——
  一次运行里所有能落盘的地方都盖住了。界面把这类输入从 argv 里拿掉、改走 stdin，运行记录里显示 `***`。
- **`ai` 失败有了分类**：缺 Key / 被拒 → `credential`，超时 → `timeout`，其余 → `model`，每种都带一句
  该去哪儿修；这些失败不再只是一个异常字符串。子清单里的失败步骤会把它的 `errorInfo` 上浮到父步骤，
  卡片因此能直接说「`maers-login:read` · credential」。
- **界面**：`RunSession` 流式解析事件（`RunMessage::Event`），运行中的步骤轨道实时变色；
  检查器新增「错误类型 / 位置 / 期望·实际 / 建议」与可点击的「产物」行（用系统程序打开）；
  卡片第二行优先显示本次运行的 `summary`；`InputSpec` 认识 `secret`；运行记录与输入区对 secret 显示 `***`。
- **工作流**：`workflows/units-export` 的 `account` / `password` 改成 `secret` 输入（界面可填，命令行回退到
  环境变量 / 凭据文件）；每一步都写 `summary`（「共 51 条」「登录成功」「已导出 …（20.5 KB）」）、
  产物（验证码截图、xls、CSV）与结构化错误（`selector-miss` / `regex-miss` / `image-unreadable` /
  `assert` / `credential` / `timeout`）；`verify` 失败时把报告和 `error` 放进同一份 JSON（一个步骤只能输出
  一个文档）。
- **文档**：新增 `docs/工作流卡片-界面设计.md`——这一轮的落地清单、事件协议、卡片信息层级、错误 kind 表，
  以及还没做的东西（输入框掩码、本机保存、代码页签、图片缩略图、失败卡上的重试/跳过、`ai` 工具流、只读画布、
  历史逐步骤回看、工作流页的 validate/explain/test 按钮）与它们的落点、验收标准。
  §12 另记了一个决定：**聊天界面里用「一条消息 = 一个运行块」复用同一套行渲染器**（`Role::Run` 只存
  run id，`split_history` 过滤掉，不进模型上下文），输入卡排在掩码之后；并**更正**了"工作流日志会污染
  模型上下文"这一说法——`split_history` 本来就只回放 user/assistant/attachment。多 agent 只作为可选的
  "运行监督者"（P3），不用于解决这个界面问题。
- 测试：引擎新增 `tests/step-report.test.ts`（摘要/产物/结构化错误/未知 kind 兜底/秘密输入的四条规则与
  脱敏，9 项）与 `cli.test.ts` 的 `--events` / `--secrets-stdin`（3 项），共 465 通过；界面补一条事件流解析
  测试（5 条事件 + 结构化错误 + 汇总照旧），`cargo test` 全绿；工作流 15 个离线 Python 单测通过。

## 2026-11-26（工作流：浏览器成为步骤能力，判断点能看图）

- **`browser` 变成清单可调用的能力**：新增 `dsh-flow browser --workdir <dir> --json '<请求>'`，请求体就是
  问答里 `browser` 工具的同一套参数，返回 `{ok,text,files,image}` 并以退出码表达成败。它让 Python / Node
  步骤驱动同一个浏览器、同一个页面和同一份登录态（指针仍是 `<workdir>/.dsh-flow/browser/session.json`），
  于是「登录一次、后面全自动」可以写进清单而不是留在对话里。失败也照样打印 JSON，脚本可直接分支。
- **`screenshot` 支持 `selector`**：此前只认 snapshot 的 `[n]`，现在可以只截一个元素——验证码、某个按钮、
  表格的一角；找不到元素是错误而不是「截了一张视口图」。`captureBeyondViewport` 与结果文案同步。
- **新增 `wait_download`**：等某个下载出现并停止增长，参数 `name` / `pattern` / `since`（点击前的时间戳）/
  `timeoutMs`；`since` 让上一次运行的 `使用单位导出 (1).xls` 不会被当成新结果。`downloads` 同时补上
  `files: [{name,path,bytes}]`，让步骤拿到路径而不是一段说明文字。
- **`ai` 步骤能看图**：新增 `ai.images`（路径可模板，相对清单目录），`http` 后端按 `image_url` 内容块发送，
  上限 3 张 / 单张 5 MB / 总量 12 MB base64，超了在花 token 之前报错；`cli` 后端只收到 `[{path,mime}]`，
  由脚本自己打开。图片 sha256 进入判断记录的 key，换图即新判断。`ai.model` 也支持模板。
- **`ai.cache: false`**：这条判断点每次都问、不读也不写记录。验证码这类输入每次变化的判断点写它；
  `--level manual` 会明确说「没有答案可回放」，不再回放昨天的验证码。
- **步骤环境新增 `DSH_ENGINE_DIR`**：步骤要回调 `dsh-flow browser` 时不必在清单里写死引擎路径。
- **`explain` 把子流程算进来**：`use` 步骤不再显示成 `map`，并列出每个子清单的步数与判断点（含 why），
  最后给「含子流程」的总比例——否则子流程里的模型调用会让父清单读成 100% 确定性。
- **新增工作流 `workflows/units-export`**：把 `使用单位导出` 的 SOP（随流程一起放在 `docs/操作SOP.md`）
  固化为 12 步 / 1 个判断点（92% 无需模型）。
  探测登录态（`when` 门禁）→ 换码截图 → 视觉读码 → 按页面判定登录 → 导航 → 查询读总数 → 导出等文件 →
  xlrd 解析 → 表头/条数/取值校验。密码失败有 3 次预算且存在工作区，站点锁定风险不会因重试被撞穿；
  凭据只从环境变量或仓库外 JSON 读，不进 argv、不进日志。逐项设计见该目录 README。
- 测试：引擎补 `ai.images`（发送 base64、换图不复用）、`ai.cache: false`（每次问、不写记录、manual 报错原因）、
  `wait_download`（完成/旧文件不算/超时文案）、`browser` CLI（JSON 往返、缺请求与坏请求的退出码）共 10 项；
  真机浏览器测试补 `screenshot selector`（有元素时出 PNG，没有时是错误）。工作流自带 15 个离线 Python 单测
  （单元格清洗、表头契约、条数契约、取值统计、CSV 往返、密码失败预算、验证码字符集）。

## 2026-11-26（会话续命的浏览器、下载、结果文件进对话）

- `browser` 一个对话一个浏览器（上一轮）：进程活过单个请求，页面与登录态留到下一问。
- **允许下载**：`Browser.setDownloadBehavior` 从 `deny` 改成 `allow`，下载落到对话目录的 `.dsh-flow/downloads`（不碰桌面、不碰用户自己的浏览器目录）。`Browser.downloadWillBegin` / `downloadProgress` 被记录下来，触发下载的那一步结果里会写「刚下载了 report.txt（12.3 KB）→ 绝对路径」；新增 `downloads` 动作（列出下载目录，最新在前，跳过 `.crdownload` 半成品，不需要浏览器在跑），`close` 只收浏览器、不动已下载的文件。
- **新增 `show` 工具**：把做好的文件摆到对话里。引擎结果带 `files: [{name, path, bytes}]`，随 `step` 事件到达界面；转写里这一步下面画出文件卡片（扩展名徽标、名字、大小），点一下用系统程序打开，旁边「所在文件夹」在资源管理器里选中它（Windows `explorer /select,`、macOS `open -R`、其它 `xdg-open` 目录），打开失败给一条 toast。只列已存在的文件，不读内容、不写盘，一次最多 12 个，`.env` / 密钥类仍拒绝。
- **由模型决定展示什么**：网页下载的文件默认不打扰用户，只有模型调用 `show` 时才出现在对话里；这正是「LLM 报告完成了，但我在聊天界面看不到结果文件、还要去工作区翻」的解法。
- 持久化：`ChatMessage.files` 与 `chat.json` 的 `files` 一起存读，重开对话卡片还在；`ALL_TOOLS` 增到 29 项，工具页补 `show` 卡片（文件与目录组，不确认），`browser` 卡片补 `downloads`。
- 测试：引擎新增 `tests/show.test.ts`（8 项：清单、尺寸、缺文件不吞、全缺报错、越界拒绝、文件夹、上限、`step` 事件带 `files`）与浏览器两项（下载目录列表、真机下载——`DSH_BROWSER_TEST=1`）；界面补 `file_badge`、`step_file`、`chat.json` 往返（含 `files`）三处断言。

## 2026-11-26（工具审计：接线、沙箱、搜索与一致性）

- 接线补齐：`wait_for` 新增 `target: exit`（等后台任务结束，审批门禁、schema、工具页、README 一起接通）；`task` 新增 `close`（关闭 stdin 触发 EOF）与 `wait`（等结束），schema 补全 7 个动作与 `timeoutMs`；补齐两条回归测试（此前实现存在但调用不到）。
- 文案纠正：`http` 的审批提示此前写「不会自动跟随重定向」，实际会跟随 5 次；`task` 卡片把 `input` 也说成「结束」，现在统一为「已完成 / 操作失败」；工具页 `screenshot` 改为「默认前台窗口」。
- 插件保留字：除内置工具外，`ask_user_question` / `todo_write` / `exit_plan_mode` / `remember` 与 `mcp_` 前缀也不能被插件占用（此前可以注册成功，却永远收不到调用）。
- 沙箱收口：新增 `tools/spawn.ts`，`shell` / `python` / `node` / 后台任务 / `run_tests` / `diagnostics` 都走同一个 `spawnTool`；`run_tests` / `diagnostics` 此前直接 `spawn`，同样的命令换成这两个工具就绕过了低完整性 jail。`capture.ts` 改为逐行 `decodeOutput`（GBK 不乱码）并把最新一行实时送进工具行；`run_tests` 的 `filter` 对 npm 作为 `-- <filter>` 透传。
- 搜索质量：`glob` / `grep` / `replace` 新增 `path` 参数（只搜一个目录）；忽略规则改为逐目录读取 `.gitignore` / `.ignore` / `.git/info/exclude`；新增 `.venv` / `__pycache__` / `build` / `out` / `vendor` 等生成目录跳过；符号链接文件会被跟随（目录不跟随，避免环路）；`grep` 逐行解码，GBK 源码也能命中。
- 对话隔离：界面把「其它对话」的目录名随请求传给引擎（`skipDirs`），`glob` / `grep` / `tree` / `symbols` / `replace` 不再走进别的对话夹。
- 只读模式：`screenshot` 的副本在「仅可查看」下改存 `.dsh-flow/sandbox-tmp/shots`（即使调用写了 `path` 也不往工作区写，并在结果里说明），`symbols` 的索引不再落盘；`run_tests` / `diagnostics` 在「仅可查看」下直接说明「会写构建产物」而不弹卡（它们经由沙箱启动后本来就写不了 target）；`delete` 支持 `recursive: true` 删除整个文件夹（默认仍拒绝）。
- 索引新鲜度：`write` / `edit` / `replace` / `move` / `copy` / `delete` 后立即失效 `.dsh-flow/index.json`；缓存里记录跳过目录的集合，名单变了就重建；并发建索引改用唯一的临时文件名。
- 编码诚实：`write` 覆盖非 UTF-8（GBK / UTF-16）文件时在结果里说明「已按 UTF-8 写回」，不再静默改掉原文件编码。
- `http` 支持 PUT / PATCH / DELETE / HEAD；正文只允许随 POST / PUT / PATCH 发送；新增 `saveTo`：把响应正文**流式**写到工作区文件（最多 20 MB，不占上下文，下载图片后可接着 `read_image` 看）。
- 测试 / 检查补齐第二梯队：`run_tests` 自动识别 `go test ./...`（`filter` 用 `-run`），失败用例与包统计解析成清单；`diagnostics` 自动识别 `go vet ./...`，文本诊断接受 `file:line:col: message`（没有 error/warning 前缀的行按 error 计）。
- 自动审查放行 `wait_for`（只轮询、不改动任何东西）；`replace` 仍然逐次询问，工具页同步显示「按规则 / 需确认」。
- 接入点新增「图片：开 / 关」：关掉后该连接的请求不再声明 `read_image` / `screenshot`，追问也不再携带上一轮截图（纯文本模型收到 `image_url` 会拒绝整个请求）；随连接保存在 `config.json` 的 `vision`，旧配置缺字段时按开启处理。
- 界面测试补 `mode_tools_for` 与自动审查放行 `wait_for` 的断言；引擎测试补 Go 解析、`go vet` 识别、`http saveTo`（含越界拒绝且不发出请求）。
- 工具页按类分组：内置卡片分成「文件与目录 / 检索与索引 / 版本控制 / 执行与任务 / 网络与图片 / 交互与外部」六段，每段一个小标题；`ToolCard` 新增 `group` 字段，卡片渲染抽成 `builtin_tool_card`。分组只是展示，`ALL_TOOLS` / 审批 / 并发规则一概不动。
- `wait_for` 去掉 `target=exit`：等后台任务结束统一走 `task action=wait`。两处（工具接口与审批门禁）都会提示改用 `task`，schema 与工具页不再宣传第四种 target；回归测试改为验证 `task wait` 的两种情形（已结束 / 运行中结束）以及 `wait_for exit` 被拒。
- 沙箱收口（第二批）：`git` / `git_write` / `screenshot` 也改走 `spawnTool`。此前 `git commit` 会在 jail 之外执行仓库的 pre-commit hook —— 那是项目代码；截图的 PowerShell 同样不再裸跑，抓图临时文件写在沙箱可写的 `.dsh-flow/sandbox-tmp` 下。
- 边界统一：默认权限下文件工具的写入范围从「整个 workspace」收窄为**当前对话目录**，与 `dsh-sandbox` 的 workspace 完全一致；`glob` / `grep` / `tree` / `symbols` 也以它为根，命中改为相对路径。系统提示词与 README 同步改写（保留 dsh 「询问后可改」的只读语义）。
- 上下文预算随窗口缩放：`toolBudgets(windowTokens)` 取代写死的 60k / 20k（128k 窗口保持原值，1M 窗口单条 40k、整问 300k）；被省略的工具结果现在写明「较早的是 `read big.txt`」，模型能判断要不要重跑；压缩后重建索引时会从 assistant 轮次里找回工具名。
- `grep` 新增 `perFile`（默认 20，最多 200）：一个命中上百行的日志不再吃掉全部 200 行预算，结果末尾注明哪些文件被截断。
- `read_image` 新增像素上限（20 MP）：从 PNG / JPEG / GIF / BMP / WebP 头部读出尺寸，超过就拒绝并提示先缩小——只查 5 MB 字节数挡不住一张 10000×10000 的压缩图。
- 工具页顶部新增过滤框（按名字或描述，Esc 清空，复用侧栏搜索的字段机制）；过滤时隐藏「工作流（未实现）」占位，并给出「没有匹配」提示；该占位标题也从「计划中」改名，避免读成四个内置工具。
- 审批记忆扩展到文件：`write` / `edit` 卡片也提供「本次会话都允许」，按**规范化后的路径**记住（`write`/`edit` 共用，`./a.txt` = `a.txt`），同一文件连续小改只问一次；`replace` 涉及多文件，不提供。
- 小项打包：只读批处理里同一工具同一参数只执行一次（第二次复用第一次的结果）；`run_tests` / `diagnostics` 默认超时改为 300 秒；`replace` 新增 `dryRun`（只出计划、不写盘、不需要确认，agent 侧跳过审批卡）；`write` 覆盖长文件且只改少量行时提示「用 `edit` 只发差异」。
- 统计页新增「工具分布」卡片：账本按工具记调用次数（`requests.jsonl` 的 `toolNames`、日汇总的 `toolCalls`），卡片按次数排序并显示每个工具的占比；没有请求文件的旧对话从既有工具行（`read · 文件`）推出工具名，旧记录缺这项时卡片留空而总数照常。
- 工具页描述改为单一来源：卡片显示引擎上报的工具 schema 里的 `function.description`（即发给模型的那段说明），页面自己的短文案只作首次运行前的兜底；过滤框匹配当前显示的那段。新增 `engine_description` / `shown_description` 及测试。
- `run_tests` 新增两个 runner：`.sln` / `.csproj` 用 `dotnet test`（`filter` 转成 `--filter FullyQualifiedName~…`），`pom.xml` 用 `mvn -B test`（`filter` 转成 `-Dtest=…`）；两者的失败用例与统计（`Failed: 1, Passed: 2` / `Tests run: …`）都解析成清单。
- `diagnostics` 新增三个入口：`dotnet build`（Roslyn 的 `file(line,col): error CS…` 与 tsc 共用同一条解析）、`mvn -q -DskipTests compile`（`[ERROR] file:[行,列]`）、项目自带的 `eslint -f compact`（`file: line L, col C, Error - …`）；另外补了 javac 的 `file:12: error: …`（无列号）识别。
- `screenshot` 支持裁切：`x` / `y` / `width` / `height` 四个都给时只截所选区域中的那一块（相对前台窗口或整屏，自动裁到区域内），结果行会写明裁切大小与位置。
- 新增 `src/tools/project.ts`：把「这是 .NET / Maven / eslint 项目吗」的判断放在一处，`run_tests` 与 `diagnostics` 共用，避免两处检测各写一份。
- `edit` 支持非 UTF-8（GBK 等）文件：`old` / `new` / `content` 全为 ASCII 时按字节替换，中文与其余字节原样保留（字符边界扫描，GBK 尾字节不会误匹配）；带中文的替换与批量 `replace` 仍拒绝并提示先转码。
- 缺陷修复（本轮巡检）：
  - `edit` 的字节路径补齐守卫：二进制文件（含 NUL）与 UTF-16 现在明确拒绝，不再把 PNG 里的 `IDAT` 当成 GBK 文本改写；`write` 覆盖 UTF-16 文件时也会给出编码提示。
  - GBK 文件按字节改时也遵循文件自身的行尾（CRLF 文件用 `old` 里的 `\n` 仍能匹配）。
  - `run_tests` 能读 `node --test` 自己的汇总（`ℹ pass 387` / `ℹ fail 0`）、mocha 的 `passing` / `failing`、vitest 的 `N passed`；此前这类项目一律显示「未报告统计」，并且 `✖ failing tests:` 还会被当成一个失败用例。
  - maven 的测试入口改用 `mvn -B test`（原来的 `-q` 会把 Surefire 的 `Tests run:` 统计一起吞掉，只留下失败列表）。
  - `task output` 只读日志末尾，且经 `decodeOutput` 解码：中文 Windows 程序写的 GBK 日志不再显示成乱码，轮询也不再整文件读入内存。
  - `write` 的「小改动，用 edit」提示在 CRLF 文件上不再失效。
  - `screenshot` 的步骤行如实写明截的是「前台窗口 / 整屏」以及裁切范围，默认值不再误标「全屏」。
  - `env` 增加 `go` / `dotnet` / `mvn` 探针与 `pom.xml`、`build.gradle(.kts)`、`gradlew`、`eslint.config.js` 标记，模型能先知道哪个 runner 可用。
- 后台任务日志现在统一写成 **UTF-8**：包装器按行解码（`decodeOutput`，一行一个判断，混合编码也正确）再落盘，所以界面任务面板、`task output`、`wait_for output` 读到的都是正常中文；旧任务留下的 GBK 日志仍能被工具侧解码。`wait_for` 也改为只读日志末尾（8 MB），轮询不会再整份日志读入内存；无换行的进度条输出有 64 KB 缓冲上限，不会让包装器一直吃内存。
- 新增 Gradle 入口：`build.gradle(.kts)` → `gradle test --console=plain`（优先项目自带的 `gradlew` / `gradlew.bat`，`filter` 转成 `--tests`），诊断走 `gradle classes --console=plain`；新增 Kotlin 的 `e:` / `w:` 行解析（`file://` 前缀会去掉）。
- Roslyn 诊断消息末尾的 ` [xxx.csproj]` 会被剥掉，不再混进消息正文（用本机 .NET SDK 造了一个真错误验证：`CS0029` 定位到 `Program.cs:1:46`）。
- 认证 HTTP（P0 补齐）：`http` 新增 `auth`，用**环境变量名**代替密钥本身 —— `Authorization` 默认 `Bearer`，`scheme: "basic"` 把变量值 base64（`user:pass`），`header` 可指向 `X-Api-Key` 一类自定义头；变量缺失时拒绝且不发请求，值不会出现在调用、对话、卡片或结果里。跨主机重定向（或降级到 http）自动剥离凭据并在结果里写明；`headers.authorization` 仍然拒绝。带凭据的请求在「完全权限 / 自动审查」下也**永远弹卡**（引擎在审批请求上带 `secret`，窗口侧据此跳过所有自动放行路径，卡片另有醒目提示）。
- 新增 PDF 读取（P1 补齐）：`read` 认出 `.pdf`（`%PDF-` 允许出现在前 1024 字节内）后按页抽取文字，返回「第 N 页」标记，`pages: "3-7"` 指定范围（一次最多 40 页、总计 2 万字符，超出会说明「还有 N 页」给下一段范围）。抽取器按 `pdftotext -layout` → `python -c` + `pypdf` 顺序探测，都不在时报「装哪一个」，而不是「二进制文件」；扫描件（无文字层）与加密 PDF 各给可执行的下一步。Windows 上 Python 默认用 GBK 输出，脚本里显式切成 UTF-8，`®` 之类字符不再触发 `UnicodeEncodeError`（用临时 venv 装 pypdf 实测抽取 3 页成功）。
- `grep` 不再把 PDF 当文本扫（`%PDF-` 快速判定，避免在前 8 KB 无 NUL 的 PDF 里匹配乱码）；`read_image` 遇到 PDF 会指回 `read` 并提示 `pages`；聊天附件识别允许文件头前有垃圾（邮件网关常见）。
- 启动时检查 Node.js（`node --version`）：低于 23.6 或缺装直接在横幅说明需要哪个版本并停止尝试提问；`node` 启动失败的提示同样带上版本要求。此前只会等到第一次提问，拿到一段引擎堆栈。
- 长会话的存储与保存：`chat.json` 里工具行的参数/输出每种最多保留 8 000 字符（超出注明原文长度，完整内容在 `requests.jsonl`）；保存改为只序列化并写入**修订号变化过的**对话（`Chat.revision`），一项工具结果不再触发全部历史的重新序列化。
- `shell` / `python` / `node` 不再执行提到凭据文件的命令：按整词、`--flag=value` 与代码里的引号字符串识别 `.env`、密钥扩展名、`.ssh/id_rsa` 等（示例文件与散文如 `git commit -m "fix secret handling"` 不拦），拒绝时说明改用「让用户贴单个值」。
- 新增看门狗：一次回答 10 分钟没有任何事件（审批卡、提问、计划等待**不计时**）就结束请求并留下说明，不再让「正在思考」无限挂起。
- 结构拆分（行为不变，625 项界面测试全过）：统计页卡片 → `stats/cards.rs`（686 行）、数学排版 → `message/math.rs`（380 行）、`Done` 分支 → `chat_actions/answer.rs`（282 行）、审批决策与事件 → `chat_actions/approval.rs`（404 行）；`stats.rs` 2,445 → **1,767**、`message.rs` 3,366 → **2,994**、`chat_actions.rs` 1,944 → **1,370** 行。
- CI 增加打包校验：工作流会跑一次 `scripts/release.ps1 -SkipTests`，并断言 zip 里有 `dsh-flow-ui.exe`、`dsh-sandbox.exe`、`dsh-flow/src/cli.ts`，打包环节出错不再等到下一次人工发布才发现。
- 外部插件 `office` 扩到 1.2.0（原来只有 7 个工具、Word 10 个 / Excel 14 个操作，现在是 8 个工具、Word 19 / Excel 24 / PowerPoint 15 个操作）：
  - 新工具 `office_find`：在 Word 段落、Excel 单元格或 PowerPoint 幻灯片形状/备注里搜索文字（支持正则、区分大小写），返回段落号或 `表!单元格` 或幻灯片/形状与命中片段；Excel 一次取整块 `Value2` 再本地筛选，大表也不逐个读单元格。
  - **PowerPoint** 成为第三类应用：`office_open/read/edit/save/close/find/status` 全部支持 `.pptx/.ppt/.pptm`；15 个操作——`add_slide`（4 种版式）、`set_text`/`format_text`（占位符 title/body/subtitle/编号或形状编号）、`add_textbox`、`add_bullets`、`add_table`、`add_image`、`add_shape`（矩形/圆角/菱形/椭圆/箭头/V 形/直线，可设填充与文字）、`move_slide`、`duplicate_slide`、`delete_slide`（`expect` 必填）、`set_notes`、`set_background`、`set_slide_size`、`set_layout`。读取给出每页版式、形状（名称/类型/占位符/文字/位置）、表格数据与备注；导出 PDF 用 `SaveAs(…, 32)`。
  - Word 新增 `page_setup`（页边距/纸张/方向，作用于全部节）、`header_footer`（页眉/页脚文字与「页码 / 总页数」域）、`insert_toc`、`section_break`、`insert_footnote`、`insert_hyperlink`、`insert_comment`、`set_properties`、`delete_paragraph`（必须带 `expect`）、`delete_table`；`insert_paragraphs` / `set_text` / `format_text` 增加段前段后间距、行距、左右与首行缩进、与下段同页。
  - Excel 新增 `merge_cells` / `unmerge_cells`、`insert_columns` / `delete_columns`、`clear_range`、`sort`、`autofilter`、`conditional_format`（大于/小于/等于/介于/包含文字/三色色阶）、`validate_list`（下拉列表）、`add_comment`、`add_hyperlink`、`define_name`、`tab_color`、`protect_sheet`。
  - `office_read` 补齐：Word 的字数、页数、节数、标题大纲（按 `OutlineLevel`，与界面语言无关）、内嵌图片数、批注（作者/内容/所指文字）、文档属性；Excel 的 `sheet_info`（每表范围/行列数/是否隐藏）、`last_row` / `last_column`、定义名称（过滤 Excel 内建的 `_FilterDatabase` 等）、图表与 ListObject。
- 新增 `dsh-flow/tests/office-plugin.test.ts`（7 项）：`office_help` 列出的操作必须与 `Run-Word-Op` / `Run-Excel-Op` / `Run-PowerPoint-Op` 完全一致，每个 `office_*` 工具都要有桥接 action，`plugin.json` 的入口文件必须存在，桥接脚本必须保留 UTF-8 BOM（PowerShell 5.1 会把无 BOM 脚本按 ANSI 解析，中文提示被截断），以及用引擎真实的 `loadPlugins` 加载随包插件（工具名、400 字描述上限、注入的提示）。
- 修复两处 COM 细节（都经真机 Word/Excel 复现并验证）：Excel 的 `Range` 可枚举，`$x = if (...) { $range }` 会把它展开成单元格数组（`autofilter` 因此报「找不到 Address 的重载」），改用显式 `if/else` 赋值；Word 的 `BuiltInDocumentProperties` 经 PowerShell 适配层抛空引用，改写改用原始 IDispatch。
- 修复 `office_save` 不传 `path`（原地保存）时的崩溃：此前用 `$request.PSObject.Copy()` 再赋 `path = $null`，请求里没有 `path` 键时 PowerShell 无法给 PSCustomObject 加属性，直接抛「在此对象上找不到属性 path」；现在按状态写一个只带 `app` 的探测对象。PowerPoint 的「保存→关闭→重新打开」链路就是这样被发现的。
- PowerPoint 的两个实现细节：所有布尔值必须用 msoTrue/msoFalse（-1/0）；备注页上每个形状的 `Type` 都是 14，备注正文要靠 `PlaceholderFormat.Type = 2` 定位（`13` 是页码、幻灯片缩略图没有文本框）。
- Excel 日期不再是裸序列号（新增 `plugins/office/office_dates.ps1`，被读、写、搜三处共用）：
  - 读：`office_read` 默认按单元格 `NumberFormat` 把日期转成 `yyyy-MM-dd`（带时间补 `HH:mm` / `HH:mm:ss`），支持 1904 日期系统与 1900 闰年偏差；`dates: "serial"` 保留原始序列号，返回里带 `date1904`。判断只做在数值且落在序列号范围内的单元格上，整块 `Range.NumberFormat` 在 PS 5.1 上是 `DBNull`，所以逐格取格式。
  - 写：`write_range` 默认 `dates: "auto"`，把 `YYYY-MM-DD` / `YYYY/MM/DD`（可带时间）写成真日期并设数字格式；`dates: "text"` 先设文本格式再写入，避免 Excel 把日期字符串解析成序列号。含糊写法（`1/2/2026`、`2026-1-5`）仍按文本。
  - 搜：`office_find` 用同样的显示文本匹配日期格（`2026-01` 能命中），结果里显示的也是文本；扫描按 `max_rows` / `max_cols` 限制，逐格探测格式（整块格式在 PS 5.1 上不可用）。
- 新增 `dsh-flow/tests/office-dates.test.ts`（3 项，非 Windows 跳过）：日期格式识别（数字格式 vs 日期格式，含引号字面量与 `[$-804]` 区域前缀）、序列号转文本（整数/带时间/纯时间/1904/1900 闰年偏差/0 与负数）、写入识别（只接受 ISO 形状，`1/2/2026` 与公式不转换）。
- 修复「同一对话里第二个请求仍列出上一次请求的 MCP/插件工具定义」：请求记录是在引擎上报工具清单**之前**写的，此前直接沿用上一次的缓存清单；引擎报来新清单时又只补缺、不删旧，所以把 MCP 服务器在 MCP 页全部关掉后，第二个请求的「请求明细 / 请求构成」照样列着所有 `mcp_*` 定义，工具 token 也一并虚高（真正发出的 HTTP 请求本来就没有这些定义：`mcpServers` 只在有启用服务器时写入、`mcp` 网关按同一条件剔除）。现在发请求前先按本次实际携带的资源过滤缓存清单（新增 `request_schemas`：内置工具按模式清单、插件工具按启用插件、MCP 工具按启用服务器；插件注入的提示词同样按启用插件过滤，`request_prompts`），引擎上报后再用真实清单**整体替换**该请求的工具行（`set_tool_schemas`，原来只做「缺了就补」）。界面测试补两条：过滤规则（全量/关闭 MCP/基础模式/工具关闭、插件提示词五种情形）与替换语义（第二问不会留一问的工具）。
- 新增内置工具 `browser`（无头浏览器，驱动本机已安装的 Edge / Chrome，**不下载浏览器**，零 npm 依赖：CDP 客户端就是这个仓库里的 200 余行代码，用 Node 自带的 `WebSocket` / `fetch`）：
  - 动作：`open` / `goto`（http / https，拒绝 `file:` / `javascript:` / `data:`）、`snapshot`（无障碍树里可点击的元素，给稳定 `[编号]`）、`text`（正文）、`click` / `type` / `press` / `select`（真实 `Input.*` 事件：按坐标点击、`Input.insertText` 输入、下拉触发 `change`）、`eval`、`wait_for`（文字 / 选择器 / 编号）、`console` / `network`（控制台、异常、请求与失败项）、`screenshot`（视口 / `fullPage` / 单个元素，长边按 `maxEdge` 缩放，存 `.dsh-flow/shots` 并**把图直接附给模型**）、`close`；一整问共用一个浏览器与一个页面，导航后编号重新分配。
  - 安全：只用对话目录下的私有临时 profile（不读用户浏览器配置与登录态），`Browser.setDownloadBehavior=deny` 禁止下载，`close` / 进程退出时删掉 profile；`open` / `goto` / `click` / `type` / `press` / `select` / `eval` **逐次确认**，`snapshot` / `text` / `console` / `network` / `screenshot` / `wait_for` 不确认。
  - 两个实测结论写进代码注释：Chromium **无法**在低完整性令牌下启动（`CreateProcessAsUser` 直接失败），所以它是 `shell` 之外唯一不在写入沙箱内的执行类工具；Edge 更新期间 `msedge.exe` 会短暂「不存在」，启动因此带重试，并支持 `DSH_BROWSER_PATH` 覆盖。
  - 界面：工具页新增「浏览器」卡片（网络与图片分组）、`ALL_TOOLS` 由 27 增至 28、上下文页把 `browser` 归到「网络」动作类。
  - 测试：`dsh-flow/tests/browser.test.ts` 7 项（URL 规则、确认规则、卡片文案、裁剪、版本目录排序、`DSH_BROWSER_PATH`，以及 `DSH_BROWSER_TEST=1` 时用真实 Edge 跑 open / snapshot / click / type / select / wait_for / screenshot / console / network / close）；`tests/tools.test.ts` 的工具清单、`tool_catalog` 分组断言同步更新。
- 修复 `browser` 在真实对话里「打开成功、下一次调用却变成 about:blank」的缺陷（用户报告，`workspace/chat-1` 留了现场）：
  - 根因：Windows 上 `Application\msedge.exe` 是个**启动器**，交接给真正的浏览器进程后自己退出；旧实现把会话存活绑定在这个进程上（`child.on('exit')` 就把 `runtime` 置空），于是下一次调用又起了**第二个**浏览器，之前在第二个标签加载的页面就再也看不到了 —— 会话记录里留下了两个同进程 profile、18 个孤儿 headless Edge 进程和一个被锁住的 `DevToolsActivePort`（第二次启动报 `EBUSY`）。
  - 修法：① 优先直接启动**带版本号的二进制**（`Application\<version>\msedge.exe`），启动器只作为兜底；② 会话存活只看**调试 socket**，启动器退出不再算会话结束；③ 页面加载类动作（`open` / `goto` / `snapshot` / `text` / `click` …）遇到「连接已断开 / 15 秒没回复」时**自动重启会话并重试一次**；④ 会话是在同一问里被重建的，结果开头会明说「上一次的浏览器已经断开…页面回到 about:blank，需要重新 open / goto」；⑤ 页面还停在 about:blank 时任何读取动作也会提示「先 open / goto」，避免对着空白页猜。
  - 清理：`close` 与进程退出都会按 **profile 路径**杀掉这一会话的所有浏览器进程（不再依赖我们持有的那个 pid），删除 profile，并在新会话开始时清扫历史遗留的 `p-*` 目录（每个都有上千个文件）。已顺手清掉 `chat-1` 下遗留的 profile 与 18 个孤儿 Edge 进程。
  - 新增回归测试（`DSH_BROWSER_TEST=1`，真机）：`a browser killed mid-question is restarted and says the page is blank` —— 打开页面后从外部杀掉浏览器，下一次调用必须自动重启、带说明、再 `open` 能正常读到内容、且全程只留一个 profile。
- 修复「对话莫名其妙就结束了」（用户报告的那一问）：请求里**其实带了 `browser`**（`requests.jsonl` 的 tools 块第 31 项），是 `deepseek-flash` 自己记错了工具清单（thinking 里写「只给了 search/open/find/image_search」），于是只回了一句「我先打开这个登录页，看看验证码是什么形式。」**没有调用任何工具**就收尾了；循环把「没有工具调用」当作最终回答，看起来就像对话断了。现在：模型只描述动作却没调用工具时，引擎会补发一条提醒（「工具清单以随请求附带的 tools 为准，不要凭记忆判断某个工具不存在」）并再来一轮；如果提醒后仍然只说不做，就在回答末尾补一句「这一轮没有调用任何工具；如果需要我继续，回复「继续」」。每问最多提醒一次，且只在短回答、无代码块时才触发。新增 4 项引擎测试（提醒后真的调用工具、连续两次承诺收尾加说明、正常回答不被打断、承诺判定规则）。
- `browser` 改为**每对话一个常驻浏览器**，跨请求保持页面与登录状态（用户反馈「下个请求又开一个，工作没有连续性」）：
  - 引擎是「一个请求一个进程」，原来会话只活在进程里，所以下一问一定重开浏览器；现在**浏览器进程不随引擎结束**（启动时 `detached` + `unref`），并把它写进对话目录的 `.dsh-flow/browser/session.json`（endpoint、页面 target、profile、最后使用时间）。下一个请求先尝试**接上同一个浏览器和同一个页面**，接上了就在结果开头注明「已接上上一问打开的浏览器：页面与登录状态都还在」。
  - profile 从「每问一个、用完即删」改为「每对话一个」：登录态、cookie 跨请求保留；`browser close` 才是重置（关浏览器 + 删 profile + 删指针）。闲置 30 分钟（`DSH_BROWSER_IDLE_MIN` 可调）后，下一问会换一个新的；指针失效（浏览器已退出、机器休眠）时自动清掉并重开。
  - 引擎回答结束后**主动断开 CDP socket**（`detachBrowser`）：否则这个 WebSocket 会让引擎进程和窗口读取的管道一直挂着不结束（这轮用跨进程测试才暴露出来）。
  - 卫生：应用启动时后台清理上一次崩溃留下的 headless 浏览器（按 `.dsh-flow\browser` 匹配命令行，只杀自己起的），关闭窗口时一并结束。
  - 另外修了两处：复用 profile 重启前删除**过期的 `DevToolsActivePort`**（否则连到已死端口）；给 DevTools socket 常驻 `error` 监听（连接失败不再变成未捕获异常）。
  - 新增 3 项测试：跨进程连续性（两个独立 node 进程，第二个进程里 `window.__carried` 与页面文本都还在、endpoint 不变、`close` 后指针与 profile 都清掉）、session.json 读写与坏指针跳过、闲置超时可由环境变量调整；真机（`DSH_BROWSER_TEST=1`）11 项全过。
- 测试：引擎 433 项（430 通过 + 3 项需要 `DSH_BROWSER_TEST=1` 的真机浏览器测试），界面 626 项。
- 单一事实来源：新增 `src/app/tool_catalog.rs`（工具名 + 卡片描述 + 是否写入 / 是否总询问），工具页从它渲染；一致性测试保证 `ALL_TOOLS`、交互工具与 `mcp` 网关都有卡片、且没有重复。工具页的「需确认 / 直接执行」也补上自动审查的真实规则（`replace` 在自动审查下仍需确认，`write/edit/copy/mkdir/shell` 显示「按规则」）。
- 权限文案：`TOOL_PROMPT` 补上 `task` / `git_write` / `wait_for` / `run_tests` / `diagnostics` 的确认规则，以及截图 / 索引在只读模式的落盘位置。
- 仍未做（属于工作流，另行安排）：`fetch` 录制回放与 `--offline`、`approve` / `pause` 步骤、`use` 子流程、`state` 游标、工作流页的 validate / explain / test 按钮。

## 2026-11-26（构建诊断）

- 新增 `diagnostics`：自动识别 `cargo check --message-format=json`（解析 `compiler-message` 的 level / code / 主 span）与项目自带的 `tsc --noEmit --pretty false`（`file(line,col): error TSxxxx: msg`），也可以用 `command` 给别的检查命令；rustc 文本模式（`error[E0425]: …` + `--> file:line:col`）同样能解析。返回退出码、错误/警告计数、「文件:行:列 代码 — 消息」清单（默认每类 30 条，`severity` / `limit` 可调）与输出末尾；解析不出时明确说明并给末尾输出。
- 抽出 `tools/capture.ts`（头部 + 尾部各 200k 的有界采集），`run_tests` 与 `diagnostics` 共用；顺带把 `run_tests` 的内部实现收敛到同一处。
- 接线：`ALL_TOOLS` 26 → 27，工具页卡片、询问集合、审批标题/提示/状态、上下文动作卡（命令类）一并补齐；系统提示词建议「改完代码先 diagnostics，再 run_tests」。

## 2026-11-26（图片通道收尾）
## 2026-11-26（图片通道收尾）

- 截图瘦身与收窄范围：`screenshot` 默认只截**前台窗口**（`region: all` 才截整屏），长边缩到 1600px、存成 JPEG（默认质量 80），并新增 `maxEdge` / `format` / `quality` 参数；文件名精确到毫秒，同一秒内连拍两张不会互相覆盖。实测全屏 1920×1080 → 1600×900 JPEG 126 KB（base64 172 KB），原来 PNG 是 203 KB（base64 271 KB）。
- 追问保留截图：工具返回的图片会带上保存文件（`AgentStep.image = {mime, path}`），界面把 `imageMime` / `imagePath` 写进 `chat.json`；下一次提问若没有自己的附件，就把上一轮回答的截图（最多 2 张、原始文件合计 ≤4 MB、仍在对话工作区内）重新附给模型，并在对话里留一行说明。改完界面截一张图，追问「再往右挪一点」时模型仍然看得见。
- 隐私边界：新增 `tools/secrets.ts`，`read`、`read_image`、`grep`、`symbols`（索引与引用查找）都拒绝或跳过凭据/密钥类文件（`.env`、`.env.*`、`*.pem` / `.key` / `.pfx` / `.p12` / `.jks` / `.kdbx`、`id_rsa|dsa|ecdsa|ed25519`、`.ssh/`、`.npmrc` / `.pypirc` / `.netrc` / `.htpasswd` / `.git-credentials`，以及名字含 credential / secret 的文件；`.env.example` 等示例文件除外）。拒绝时给出「让用户单独提供该值」的替代做法，搜索会注明跳过了几个文件。
- 文档：README 的「工具」节补了截图默认行为、图片跟随追问的规则与凭据文件边界；系统提示词补了同样三条约束。

## 2026-11-26（P1：看图、审批规则、仓库操作、等待、子流程）
## 2026-11-26（P1：看图、审批规则、仓库操作、等待、子流程）

- 图片通道：新增 `read_image`（PNG / JPEG / GIF / WebP / BMP，字节上限 5 MB）与 `screenshot`（Windows `System.Drawing` 全屏截取，DPI 感知；默认存到对话的 `.dsh-flow/shots/`）。工具结果新增 `image` 字段，引擎在工具消息后追加一条带 `image_url` 的 user 消息，模型才真正看得到图；一次提问最多 3 张、base64 合计 12 MB，超出会在结果里注明。上下文估算里每张图按 256 token 计。
- 自动审查可配置：`config.json` 新增 `reviewAllow`（命令前缀数组），设置 → AI → 「自动审查允许的命令」一行一格（逗号 / 分号分隔），按前缀匹配整行（`npm test` 也覆盖 `npm test --watch`），原有的 `&&` / `|` / `>` / `$(…)` 与危险参数拦截仍然生效。
- 仓库操作：新增 `git_write`（commit / branch / stash，具名动作、无 shell、不跳过 hooks、不做 reset 或强推），`paths` 只接受仓库内相对路径且拒绝 `..`；执行前确认，动作为 `git add` 失败时不会继续提交。
- 等待：新增 `wait_for`（port / url / output），默认最多 30 秒、上限 120 秒；任务已退出会立刻报告并附输出末尾，不再空等到超时。
- 后台任务可继续输入：启动时的 `stdin` 直接进任务的控制文件，wrapper 每 120ms 把新增字节转给命令的 stdin；`task` 新增 `input` 动作（追加一行，需要确认），task 已结束时会立刻拒绝而不是静默排队。
- 测试结构化：新增 `run_tests`（自动识别 `cargo test` / `package.json` 的 test 脚本 / `pytest`，或直接用 `command`），按 runner 解析出通过/失败计数与失败用例（cargo 的 `---- name stdout ----`、pytest 的 `FAILED path::test - reason`、TAP / vitest / jest 的失败行），最多列 20 条 + 输出末尾；解析不出时明确说明并给末尾输出。
- 子流程：清单新增 `use:`（`manifest` + `inputs`，也可只写路径），父流程拿到子流程最终的 `vars` 作为 `steps.<id>.json`；`validate` 会检查未模板化的目标清单是否存在；嵌套上限 5 层，自调用/环调用报 `sub-flow cycle`；子流程有独立的 `answers` 与 ledger。
- 接线：新工具全部进入 `ALL_TOOLS`（现 26 个）、工具页、审批文案与上下文「动作」卡；设置 → AI 的代理下面新增「自动审查允许的命令」输入框（`Field::ReviewAllow`），失焦即保存。
- 仍未做（都属于工作流）：`fetch` 录制回放与 `--offline`、`approve` / `pause` 步骤、工作流页的 validate / explain / test 按钮。

## 2026-11-26（工具补齐）
## 2026-11-26（工具补齐）

- 工具：新增 `env`（只读环境探针：平台、node / python / git / cargo / npm 版本、项目文件、`package.json` 脚本名）、`copy`（字节原样复制，图片等二进制也可；目标已存在时需 `overwrite`）、`mkdir`（建目录）；三者都接进引擎、审批规则、上下文「动作」卡与工具页。
- `edit` 支持按行编辑：`startLine` / `endLine` 替换整段（`content` 留空即删除这几行）、`insertAfter` 在指定行后插入；仍保留 `old` / `new` 与 `edits` 批量方式，两种定位不能混用。
- `http`：跟随最多 5 次 http/https 重定向（记录每一跳；301 / 302 / 303 的 POST 转 GET 并去掉正文），HTML 自动转成正文（去 script / style / 标签、解码常用实体），正文上限从 8 000 提到 20 000 字符；仍不发 `authorization` / `cookie`。
- 长期记忆：模型可用 `remember` 写一条跨对话结论，界面追加到 `<data>/memory.md`（一行一条；超过 32 KB 丢弃最旧；注入上限 8 000 字符），以后每次提问作为「长期记忆」块注入，上下文页可见。工具只在窗口提供记忆文件时声明。
- 审批：`copy` / `mkdir` 与 `move` / `delete` 同档（默认逐次询问，自动审查按工作区路径规则放行）；卡片文案、完成状态与上下文动作分类一并补齐。

## 2026-11-26

- 聊天视图**跟着内容滚**：每次重绘时如果滚动条已经在底部（离底部 24px 以内），就把视图钉在最新一行——流式输出、工具行插入、卡片展开时不再需要手动往下拉；一旦用户往上滚（超过 24px），立刻停止跟随，右侧「↓」按钮把人带回去。判据与「↓」按钮共用 `scrolled_away_from_bottom()`，两者不会互相打架；空对话不参与。
- 输入框**软换行**：一行放不下就折到下一行，折行位置由真实字形宽度决定（优先在空格处断，避免把英文单词切成两半），不是按字符数估算。折行只影响绘制：`Enter` 仍是发送、`Shift+Enter` 仍是插入真正的换行符，光标上下移动仍按手动换行分段的逻辑行；卡片高度、光标位置、点击定位都改读同一份「视觉行」表，所以折行后点中间那行、或者把光标停在折行点时位置都是对的。同一个换行表也用于设置页里的「对话指令」编辑框。
- 可执行程序确认是 **window 子系统**（PE subsystem = 2，`main.rs` 的 `#![cfg_attr(not(test), windows_subsystem = "windows")]`），启动不分配控制台。但**它启动的子进程会闪控制台**：`engine::node_version()` 在启动时跑一次 `node --version`、`ai::sandbox_helper()` 跑一次 `dsh-sandbox.exe --check`、沙箱内的 `icacls`，这三处都没带 `CREATE_NO_WINDOW`——父进程是 GUI 程序、没有控制台可继承，Windows 就给每个控制台子进程新建一个黑框。三处都已补上该标志，启动与首次工具调用不再有控制台窗口。`dsh-sandbox.exe` 自身仍是控制台子系统（引擎要读它的 stdout），符合预期。

- 滑杆定稿：轨道上**每个档位加一个圆环**（外径 8px、2px 描边、圆心用画布色，等于在轨道上打孔），滑块左侧的环用强调色、右侧用轨道色，层级是 轨道 → 填充 → 圆环 → 滑块，所以不拖动也能数出有几档。圆环挂在填充之后绘制，白色的孔会同时穿透轨道和已填充部分（此前顺序写反，填充盖住了左侧圆环的孔）。
- 温度档位从 21 档（0.1 步进）减到 **9 档**（0.25 步进：0 / 0.25 / … / 2），读数去掉多余零（`1`、`1.25`、`1.5`）。
- **温度只在思考关闭时生效**：DeepSeek 的思考模式会忽略 `temperature`（官方文档：设置不报错但无效），而思考默认就是开的，所以思考不是「关闭」时温度整行变灰、不响应点击/拖动、「恢复默认」一并禁用，行尾标「思考开启时无效」。之前设过的值不会被清掉，关掉思考后原样回来。
- **最大输出**：档位改为 8 档 `2k / 4k / 8k / 16k / 32k / 64k / 128k / 384K`（V4 系列封顶 384K，原来最高只到 16k）。默认值不再是常数，而是端点在不发 `max_tokens` 时实际采用的数：非思考 8k、思考 64k、思考且力度最高 128k，滑块与读数都跟着思考档位移。顺带修掉帮助文案里「默认…384K」的错误（384K 是上限不是默认）。
- **上下文窗口**：档位补全为 `32k / 64k / 128k / 256k / 512k / 1M`（原缺 512k 与 64k）。默认不再固定停在 128k，而是按 `ai::context_window(model)` 找最近的一档——deepseek-flash / deepseek-v4 实际是 1M，未知模型才落在 128k。
- 四项滑杆的读数统一为「灰色＝默认、正常色＝自己设过」，`max_tokens_notch` / `window_notch` 改为接收 `reasoning` / `model`，测试同步覆盖这三条新规则。

## 2026-11-25

- 滑杆二次修正：填充与滑块改用**同一个位置函数** `active / (notches - 1)`。此前填充按「含当前档位的整段」算、滑块按「档位中心」算，两者永远差半档 —— 五档时滑块右侧漏出一截 18px 的蓝色（温度 21 档只差 4.3px，恰好被 16px 滑块盖住，所以一直没看出来），而「默认」这一档原来占第一格，填充 20%，看着就是「最小值」。现在默认不再占一格：温度未设置停在 1.0（DeepSeek 自身默认，读数显示灰色 `1.0` 而非「默认（端点值）」），思考停在「高」，最大输出停在末档 16k、上下文窗口停在 128k（这两项真正的默认值在轨道之外，行内仍写「默认」并置灰）。思路是把绘制拆成一条轨道 + 一段填充 + 一个滑块，三者共用同一个比例，档位只留作点击热区（18px 高的整条热区，仍是一档一个）。轨道两端各留半个滑块，滑块能贴到 0% / 100% 而不越出控件。「恢复默认」在默认档时灰色且不响应点击，移开后才变蓝可点。`max_tokens_notch` / `window_notch` / `reasoning_notch` 随之改为把「未设置」映射到默认档，测试同步更新。

- 滑杆修正：每一档都自己画一段轨道，滑块改成**浮在轨道上方**（绝对定位居中于当前档位）。此前当前档位那一段只画滑块、不画轨道，五档时每段 36px，轨道上会缺一大块，看起来像断开的虚线；档位多的温度滑杆因为每段只有 8.6px 才不明显。滑块也放大到 16px（其中 14px 实心点 + 1px 背景色细环），比原来的 12px 圆点少了描边发糊的观感。

- 联网搜索改为**默认开启**：删掉 `Connection.web_search` 与设置 → AI 里的「联网搜索」开关，只要端点提供 DeepSeek 的服务端工具就一律声明；「+」菜单里的本对话开关保留（默认开启，可单独关掉并「回到默认」），`chat.json` 里的 `webSearch` 语义不变。
- 设置 → AI 的四项改为**滑杆 + 恢复默认**：温度 0.0–2.0（步进 0.1，替代原「默认/精确/平衡/发散」四档，`config.json` 从 `sampling` 预设名改为 `temperature` 数值，旧文件按预设名自动换算）；思考（默认/关闭/低/高/最高）、最大输出（默认/2k/4k/8k/16k）、上下文窗口（默认/32k/128k/256k/1M）各为 5 档滑杆。滑杆按档位切成可点击区域，按下后横向拖动即可连续取值。

- 输入框「+」按钮的图标恢复（上轮换图标时它指向了已删除的 `plus-thin.svg`，所以只剩空圆）。
- 轮数 / 工具调用上限的默认值改为菜单里的上限：`DEFAULT_MAX_ROUNDS = 24`、`DEFAULT_MAX_CALLS = 80`（原来 6 / 20，只有启动时新建的第一个对话会用到它，所以看起来"默认不是最大"）。
- 快捷键表改为只从标题栏进入：最小化按钮左边新增「?」帮助按钮，点开/关闭同一张快捷键面板（`Ctrl+/`、`F1`、`Esc` 仍然有效），设置里去掉「快捷键」分页。

- 输入框瘦身：去掉「计划 / 审批 / 联网 / 预算」四个胶囊，输入框只剩「+」、权限、模型、发送（与 dsh 一致）。这些按对话生效的设置全部收进「+」菜单新增的「本次对话」区，一项一个开关：**计划模式**、**自动审查**（仅「工作区内修改」）、**联网搜索**（覆盖连接开关，被覆盖时多一行「回到跟随设置」）；**轮数上限**（6 / 12 / 24）与**工具调用上限**（20 / 40 / 80）改成一排小按钮，选中即生效并保持菜单打开。设置窗口里的温度、思考、最大输出、上下文窗口等「按连接、少改动」的项保持原位。

- 发布流程：`scripts/release.ps1` 打包后自己删掉 `PAgent-<version>-<stamp>/` 暂存目录，并删除 `dist/` 里此前的 `PAgent-*.zip`，只留刚生成的那一个包。此前每发一版就留下 20 MB 的暂存目录 + 7 MB 的 zip，`dist` 攒到 1.5 GB；现在 `dist` 只保留一个包，运行时的 `data/`（对话与统计）、`logs/`、`config.json` 等照旧不动。约定同时写进 `.github/copilot-instructions.md`：改代码不留备份副本，回退用 git。

- 重构：`src/app/context_view.rs`（6398 行）拆成模块目录 `src/app/context_view/`，根文件只剩文档与 `mod` 声明（26 行）。按职责分为 `model.rs`（数据结构，371 行）、`report.rs`（纯计算：round 估算、请求行、构成闭合，957 行）、`ring.rs`（环形路径，73 行）、`timeline.rs`（时间线取点，180 行）、`actions.rs`（工具调用与耗时，223 行）、`cards.rs`（时序 / 行动 / 面板三张卡，840 行）、`view.rs`（页面本体，2283 行）、`tests.rs`（1297 行）。拆分是纯移动：一行逻辑都没改，`cargo test` 590 项与拆分前一致。

- 图标细节：统计页的导航图标换成 `list-pen-outline`（账本 + 笔）；设置页的说明「?」换成 `info-outline` 的「i」圆环，且按钮**去掉了边框与底色**（任何状态都透明），只有图标本身在悬停时由 `faint` 变 `accent`；侧边栏工作区分类的文件夹图标从 14px 放大到 16px。

- 统计：去掉汇总区的「耗时」卡片（模型 / 工具累计时长）。计时仍照旧写进账本（`stats/requests.jsonl` 与每日汇总里的 `model_ms` / `tool_ms`），只是统计页不再汇总展示；汇总区现在是请求总数、Token 总数、工具调用量，外加有数据才出现的缓存命中 / 思考 token / 联网搜索。

- 界面图标：整套换成 dsh 的 product icon set（16 单位网格、1px 描边、`currentColor`），替换原来手绘的 24 网格字形 —— 左侧「新建对话 / 插件 / MCP / Skill / 设置 / 统计」、工作区树、对话头部、消息按钮、输入框附菜单与圆形发送按钮全部统一，深浅两套配色下同一形状。新增 `assets/icons/new-chat.svg`（圆环加号）给「新建对话」，并删掉与 `plus.svg` 重复的 `plus-thin.svg`。dsh 没有的两个字形按同一网格补画：`image.svg`（相框 + 山水）与 `help.svg`（只取问号，圆环由 13px 的 chip 自己画）。

- 设置 → AI：每个连接新增「思考」选择（默认 / 关闭 / 低 / 高 / 最高）。关闭发送 `{"thinking":{"type":"disabled"}}`，低 / 高 / 最高发送 `reasoning_effort`，默认一个字段都不发、沿用端点自己的设置；选择随连接保存进 `config.json` 的 `reasoning`，卡片上同时提示「思考开着时端点会忽略温度」。
- 引擎（dsh-flow）：请求新增 `reasoning`（`off` / `low` / `high` / `max`，省略即端点默认）；工具循环、轮次用尽的总结轮与单次问答都用同一个选择；压缩用的摘要调用对 DeepSeek 端点固定关闭思考，避免思考吃掉摘要的 `max_tokens`。
- 设置 → AI：新增「联网搜索」（**默认允许**）。开启后请求的 `tools` 最前面带上 DeepSeek 服务端工具 `deepseek_web_search` / `deepseek_read_url`：搜什么、打开哪一页由模型决定，搜索与抓取都在 DeepSeek 服务器上执行，本机不执行工具。只对 `deepseek-flash` 发送（其它模型保留开关但不发送，避免 400），卡片上有说明；选择保存在 `config.json` 的 `webSearch`。
- 输入框：新增「联网」胶囊（连接支持服务端搜索时才出现），点击在「跟随设置 → 开 → 关」之间循环，按对话保存进 `chat.json` 的 `webSearch`；发请求时用这个对话的选择覆盖连接的开关，`None` 仍跟随设置，自动切换模型重试也照用同一选择。
- 上下文：修复「构成明细」展开后滚动数据时面板跟着滚 —— 该数据区现在自己吃掉滚轮，滚到末端也不外溢。新增可复用扩展 `ContainedWheel`（`widgets.rs` 的 `contained_wheel()`），并统一用于所有**嵌套**滚动块：构成明细、分类表、表格行区、思考 / 工具 / 审批 / 附件明细、MCP / 插件 / 技能页主体、步骤面板与日志、任务日志、设置页主体。以后新增的滚动组件只要在 `overflow_y_scroll()` 后接上 `contained_wheel()`，滚轮就不会带动外层面板。
- 上下文：请求列表与时间线新增「最近 30 次 / 全部 N 次」开关（请求列表卡片右上角），一个开关同时作用于两个视图；`request_rows` 不再截断，报表保留每次请求，列表被截断时的提示也改为指向这个开关。
- 上下文：窗口占用条改回只按**提示词**构成缩放 —— 回答 token（`Output` 槽）不再挤占条宽（它是在提示词发出之后才计费的），条内各段与「已用」数字对齐；下方分类列表仍显示整单计费，口径各自注明。
- 上下文：时间线在「全部」模式下只画最近 `TIMELINE_BARS`（120）次，右上的说明追加「只画最近 120 次」，避免几百根 2px 柱排出卡片；请求列表本身仍列出全部。
- 上下文：正在看的请求若不在当前列表窗口里（从「全部」切回「最近 30」，或新回答把它挤出去），列表顶部会有一行说明它去了哪里，并指向右上角的开关，不再像选中状态凭空消失。
- 统计：请求明细表新增「思考」列（没有思考时显示 —），与账本里的 `reasoningTokens` 对齐。
- 上下文：构成明细的展开区加了右侧细滚动条 —— 滚轮、点击轨道、拖动拇指都能用（与表格条共用 `tool_bar_metrics` / `wheel_scroll`），换看另一项时回到顶部。
- 代码块：左侧新增行号（gutter）。行号在横向滚动容器之外，长行左右滚动时不会跟着跑；行高与代码一致，空块也显示 `1`；复制按钮仍只复制代码本身。（语法高亮此前已经实现，这次补的是行号。）
- 动效：新增 `appear()`（约 140ms、ease-out 淡入 + 4–8px 上移），用于输入框的模型 / 工具 / 预算 / 模式 / 附件五个弹层、上下文构成明细的展开、对话 ↔ 上下文 ↔ 后台任务的切换，以及新的快捷键面板；gpui 的「减少动态效果」开关仍然生效。
- 上下文：核实「引擎回放（未记录）」的差额来源并改正归属 —— 用当前对话核对：请求 25 483、端点提示 24 035，而分类估算只有 10 400；差额来自**服务端 `web_search` / `read_url` 拉回的完整网页正文**（本机只保存了 74 / 1 668 字符的摘要），不是历史。现在只要该请求有服务端调用，差额就归「工具结果」，并标为「服务端搜索/读页正文（本机只存摘要）」；没有服务端调用时才归「注入（引擎回放）」。同时修复：服务端工具结果此前被当作"下一轮"的结果，本轮分类里恒为 0，现在按「本轮即可见」计入（新增 `is_server_result` 与两条测试）。
- 上下文：统一「请求构成」的三处取数 —— 分类行、占比、展开条目现在都来自同一个锚定值：行 = 端点/归属后的数字，占比 = 行 ÷ 本请求 tokens，条目按文本估算比例**精确分配**（最大余数补齐，新增 `spread_tokens`），保证 条目之和 = 行、行之和 = 请求、占比合计 100%。此前行取条目估算之和、条目又各自乘系数，一旦缩放三者就会不一致。
- 上下文：统一「整个对话合计」与请求明细的口径 —— 整卡分类占比、环形图与时间线此前读本机**原始估算**再按计费缩放，明细卡读的是**关上端点差额后**的构成。服务端搜索/读页时，同一段完整网页正文在明细里是「工具结果 81.8%」，在整卡里却只剩「工具结果 9.9%」。现在 `request_rows` 为每条请求只计算一次关上差额后的构成（`RequestRow::closed_composition` / `closed_request_parts`），整卡、环、时间线与明细共用它；「分类估算」胶囊仍显示原始估算，并补两条测试覆盖「服务端差额归工具结果 / 无服务端调用归注入」。
- 性能 / 去冗余：上下文面板的热路径不再复制大段正文 —— `round_estimates` 只按分类累计 token（不再为每轮 `parts_of` / `parts_up_to` 克隆工具结果文本）；请求明细用 `PartItems` 借用已存条目（`All` / `ToolResults` / `One`），默认视图与切换轮次都不再整块复制 `detail_parts`；整条已关闭的构成直接借用 `RequestRow::closed_composition`，省掉每帧二次 `close_request_parts`；展开项计数在构建 `PartView` 时一次算好；时间线的压缩标记随报表缓存；`load_request` 先按 `"turn":N` 过滤行再解析，打开历史请求不再反序列化整个 `requests.jsonl`；`context_report` 的四次消息扫描合并为一次。
- 计划模式 / 待办 / 结构化提问：输入框新增「计划」胶囊；开启后引擎只做只读调查，`exit_plan_mode` 把完整计划交给聊天里的计划卡，批准后开始执行并自动退出计划模式。新增 `todo_write`（待办列表随 `chat.json` 保存，显示在对话标题下）与 `ask_user_question`（2–6 个选项，点选即回答）。引擎、CLI 交互桥、Rust 事件/卡片/持久化与测试一并接通。
- 审批策略：新增「审批」胶囊（工作区模式）切换「逐次询问 / 自动审查」。自动审查由规则决定：只放行工作区内普通写入与只读命令（git status/diff/log/show、cargo check/test/clippy/build、dir/ls/type），其余仍弹卡；`autoReview` 随 `chat.json` 保存，并有规则测试。
- 沙箱：新增 `dsh-sandbox.exe`（`src/bin/dsh-sandbox.rs`）——Windows 受限令牌 + 低完整性 + Job Object。工作区模式把对话目录标为低完整性可写区，仅可查看模式只给私有临时目录写权限并恢复工作区级别；`shell` / `python` / `node` / 后台任务都经由它启动，停止会杀整棵进程树。helper 缺失或受限令牌被系统拒绝时自动退回原来的路径检查 + 审批。发布脚本把 helper 一起放进便携包。
- 新增快捷键面板：`Ctrl+/` 或 `F1` 打开；分组列出现有按键（对话 / 阅读 / 应用），`Esc` 或点击遮罩关闭，卡片内点击不会误关。设置窗口新增「快捷键」页列出同一份清单（共用 `shortcuts.rs` 的分组数据），侧栏不再单列入口。
- 引擎（dsh-flow）：解析回包的 `server_tool_calls` / `annotations` / `finish_reason`，把每次搜索 / 读页翻译成与本地工具同形的 `step`（`web_search` / `read_url`，展开可见「标题 — URL」来源），最终答案另带 `webSearch` 汇总（搜索数 / 读页数 / 去重来源）；网关不认识这两个工具类型时自动去掉它们重发一次；`content_filter` / `length` 造成的空回答改为明确错误，不再只说「没有返回内容」。
- 上下文：动作卡把 `web_search` / `read_url` 归到「网络」。
- 联网搜索：搜索一开始就出现一行「web_search · 查询」（来自 `server_tool_event` 的 `call_started`），完成时用同一个 id 替换成结果行，不必等整轮结束；`content_filter` / `length` 的空回答给出明确原因。
- 修复联网搜索行只显示「参数」：进行中行与完成行的 id 现在按**轮内位置**对齐（不再依赖端点是否带 `id`），完成行能替换掉进行中行；`server_tool_calls` / `annotations` 同时接受 `delta` 内与所选 `choice` 上的写法；来源列表兼容 `sources` / `results` / `items` / `pages` 与 `url|link|href` 字段；端点确实没给来源时，展开处明确写「端点没有返回来源列表」，不再只剩参数块。
- 修复服务端调用的真实形状：`server_tool_calls` 是按思考步骤分组的 `{ index, reasoning_content, calls: [ … ] }`（有的网关是扁平数组），现在两种都拆平；分类以 `action.type` 为准（`open_page` / `find_in_page` / `read_link` 算读页，`search` / `image_search` 算搜索），深度搜索里的“翻页”不再被当成一次搜索。这也修正了统计里的搜索 / 读页次数（此前把每个步骤都算成一次搜索）。
- 统计：账本新增思考 token —— `reasoningTokens` 随请求明细与每日汇总一起保存，统计页出现「思考 token」卡片并注明占输出的比例（有思考才显示）；旧记录缺字段按 0 读，历史汇总不变。
- 上下文：轮次列表改为**直接列出**。明细卡里一次摆出「合计 N 轮往返」和「第 1…N 轮往返」，每行带提示 / 回答 / 工具数，选中行高亮；弹层、它的开关状态与根层渲染一并删除。单轮请求也因此有一致的轮次行。
- 设置：`?` 帮助气泡的出现时间从平台默认 500ms 缩到 250ms，悬停反馈更跟手。
- 联网搜索：工具行的来源现在可点——展开后每个来源一行「标题 · 打开」，用 `open_url` 打开默认浏览器（只放行 http(s) / mailto，不经 shell）。
- 联网搜索：搜索次数进账本——`WebSearches` / `WebFetches` 写进请求明细与每日汇总，统计页新增「联网搜索 N 次 · 读页 M 次」卡片（有搜索才出现）。
- 联网搜索：多轮回放——服务端调用（`server_tool_calls`）随助手段落存进 `chat.json`，下一问作为 `serverToolCalls` 回传；同一问内的工具轮也带上，追问不再丢已读页面。
- 发布脚本：目标 exe 被正在运行的程序占用时，自动改名成 `dsh-flow-ui.running-<时间>.exe` 后再构建（并在下次运行时清理遗留文件），不再需要先手动关掉应用；脚本改为带 BOM 的 UTF-8，控制台里的中文提示不再乱码。
- 设置页：说明文字不再铺在控件下面，改为标题或标签后的「?」图标，悬停时在气泡里显示（用 gpui 自带的 tooltip，气泡画在同一窗口的根层，不会被滚动区裁剪，也会自动避让窗口边缘）；覆盖面包括 AI 页（接入点、温度、思考、最大输出、用量上限、HTTP 代理）、模式页的全部卡片、对话指令、数据、备份与导出、常用片段。空状态与状态提示（「没有安装外部插件」「文件还不存在」等）仍然直接显示。
- 表格：滚轮在表格上只滚表格。gpui 的滚动容器会把同一个滚轮增量同时喂给每一层，所以表格到顶/到底后页面还会跟着跳；现在表格（上下文面板的分类表、统计页的模型表）在自己的滚动之后再拦住事件，表内滚动不会带动面板，表格外侧的滚轮照常滚面板；表格右侧滚动条上的滚轮也归表格。
- 新增可复用工具 `wheel_offset` / `wheel_scroll`（`src/app/widgets.rs`）：给「滚轮落在不会自己滚的地方（如细滚动条）」的场景按同一套边界规则移动 `ScrollHandle`，供表格条复用。
- 上下文：去掉「整个对话合计」标题右侧那串统计（请求次数、模型 / 工具耗时、会话缓存命中率）。它用 `flex_shrink_0` 排在固定宽度的卡片里，文字一长就压出卡片边界；这些数字仍能在环形中心、标题下的 chips、明细表和输入框下的胶囊里看到。
- 上下文：删除「本轮梗概」整块（用户 / 新进入 / 回答 / 压缩）与明细卡「第 N 次请求的构成」标题下方那行灰色统计（端点 · 估算 · 窗口占比 · 轮往返 · Σ 每轮 / 回答 · 缓存命中）。两者都连同数据层一起清理：`RequestBrief` / `request_brief` / `render_brief`、`RequestRow.compaction_saved`、只为那行文字服务的估算与百分比变量、`BRIEF_TEXT`，以及相关测试。明细卡现在只保留标题、窗口条、轮次选择器、轮次说明与分类列表。
- 上下文：`ContextReport` 里只为合计卡那串文字服务的 `model_ms` / `tool_ms` / `cache_hits` 汇总一并移除（每行请求自己的计时与缓存字段不受影响）。
- 设置页新增可复用组件：`help_icon`（圆角「?」+ 气泡）、`settings_card_help` / `settings_section_help`（标题带说明）、`settings_field_row_help`（标签带说明），以后新增设置项只需给一句说明。「?」为 13px 圆形，问号改为内置矢量字形（`assets/icons/help.svg`），不再受字体基线影响，位于圆圈正中。
- README：补充「模型与思考」一节（`deepseek-flash` / `deepseek-v4-pro` 默认开思考、力度的含义、温度被忽略的规则）；引擎 README 补充 `reasoning` 字段与 `reasoning` 事件说明。

## 2026-11-24

- 标题栏：最小化 / 最大化 / 关闭按钮的悬停高亮改由窗口按指针位置自己绘制，不再时而亮、时而不亮（这三个格子属于系统非客户区，gpui 的 hover 命中不总跟得上鼠标移动）。
- 上下文：新增「计时」卡片——所选请求的墙上时间拆成模型往返 / 工具执行 / 其他三段（横向三段条 + 时数与占比），「其他」注明包含上下文压缩、等待审批、提示词拼装与进程开销；没有计时数据的老记录不显示该卡片。
- 引擎（dsh-flow）：`runAgent` 与单次请求都回报 `timing: { modelMs, toolMs }`，由引擎自己计时（流式输出计入模型时间，失败也计入）；会话文件保存 `modelMs` / `toolMs`，重启后历史请求仍可查看计时。
- 上下文：「整个对话合计」标题行在有计时数据时追加累计「模型 / 工具」时长，并补充会话级「缓存命中 X%」（与输入框下的胶囊一致；端点从未回报命中时不显示）。
- 上下文：新增「窗口占用」条——所选请求的 8 类构成按模型窗口缩放显示，末端用灰块标出压缩预留（对齐引擎 80% 压缩触发线，竖线标记）与连接预留的输出窗口；填过 80% 时不画竖线、改为「已过 80% 压缩线」提示；窗口大小未知时不画。构成无法归类的差额用淡灰补齐，填充边缘始终与「已用」数字一致。
- 上下文：时间线从「最多 12 根单色增长柱」升级为每个请求一根堆叠柱（按 8 类构成上色，老记录无构成时画一块中性灰），点柱子即切换所选请求、当前请求带浅色底纹；柱顶小圆点标注压缩 / 技能 / 换模型 / 最大跳变 / 最长，下方图例只列出本窗口内实际出现的事件及数量。
- 上下文：详情卡新增「本轮梗概」——用户（打开这一轮的问题首行）、新进入（与上一次请求相比增长的分类，取前两位）、回答（首行 + 工具数），并在该请求被自动压缩时多一行「压缩」。
- 上下文：压缩事件改为结构化保存（`chat.json` 的 `compactionLayer` / `compactionSaved`），时间线标记与「本轮梗概」不再依赖匹配笔记文案；梗概写明压缩方式与省下的 token（摘要压缩 / 省略最早消息 / 精简工具结果），老对话仍以原笔记文本兜底，旧提示词措辞变化不会丢标记。
- 统计：账本（请求明细、每日汇总、模型分项）新增缓存命中、提示合计与模型 / 工具计时；统计页新增「缓存命中」与「耗时」两张汇总卡（无数据时不出现），模型分布行在端点回报命中时附缓存命中率。
- 请求明细 `requests.jsonl` 同步记录 `cacheHits` / `promptTokens` / `modelMs` / `toolMs`，重开历史请求与跨对话统计读到同一批数字。
- 引擎（dsh-flow）：解析推理模型流里的 `reasoning_content` / `reasoning`（流式与整段 JSON 都认），按轮用与窗口一致的估算规则记下思考 token，随答案回报 `roundReasoning`；单次请求也回报同样形状。思考文本不写进回答，也不进提示词。
- 上下文：新增第 9 类「推理过程」（紫色）——分类占比、环形图、窗口占用条与时间线都包含它；只在支持推理的模型（如 deepseek-reasoner）上有值，其他模型显示 0 并说明原因。
- 上下文：轮次选择选中某一轮时，标题行在「回答 / 工具」之后追加「思考 N」；`chat.json` 保存 `roundReasoning`，重开后历史请求仍能看到。类别只追加在末尾，旧的 7 / 8 槽构成读取时补零，不会错位。
- 模型：DeepSeek 预设更新为现行 `deepseek-flash` / `deepseek-v4-pro`（`deepseek-chat` / `deepseek-reasoner` 已不在官方列表，思考是这两个模型的默认模式）；模型菜单改为「预设列表 ∪ 端点回报」，拉取结果再也不会把预设里的模型挤掉；指向 `api.deepseek.com` 的自定义连接同样能拿到这两个 id。
- 上下文窗口：`deepseek-flash` / `deepseek-v4-pro` 按 1M 计算（旧的 chat / reasoner 仍按 128K）；引擎默认模型同步为 `deepseek-flash`。
- 对话：推理模型的思考过程现在单独显示——流式时回答上方出现「思考中…」（只显示最新约 1200 字），回答落地后折叠成「▸ 思考过程 · N 字」，点开可读全文，展开状态随对话保存（`chat.json` 的 `reasoning`）。思考不再混进回答，也不会出现后又被回答顶掉。
- 引擎（dsh-flow）：思考片段以 `{ type: "reasoning", text }` 实时送出，最终答案再带完整 `reasoning` 文本；单次请求与多轮（每次工具往返）都记录，且思考不计入回答或提示词。
- 上下文：`requests.jsonl` 保存思考文本与思考 token 数，「推理过程」分类现在可以展开查看具体内容；多轮请求也允许展开，因为思考是整问一段文本，而不是按轮流进提示词的东西。
- 对话：思考改为**按轮逐条记录**，和工具行一样排在时间顺序里：一轮思考一条（跨工具/新一轮时自动分段），前缀是带底色的天蓝色「思考」，点开读全文；`chat.json` 里用 `thinking` 角色 + `round` 字段保存，重开仍在原位。旧的“整段思考折叠块”仍对老存档有效，新回答不再把思考塞进回答气泡。
- 上下文：「推理过程」的 token 按引擎回报的轮次归属到**该轮**（思考发生在哪一轮就记在哪一轮），请求行仍保存整问的思考文本供展开查看。
- 修复上下文逐轮思考：端点没有回报逐轮 `roundReasoning` 时，面板曾把整问的思考合计当成某一轮、并让每轮看起来一样。现在逐轮思考直接来自对话里的「思考」记录（按 `round` 归属，缺数字时用记录文本估算）；选中某一轮时，推理数字与展开文本都只属于该轮；该轮没有回报构成时只显示这轮已知的思考，不再回退到整问合计；整问视图仍显示合计。
- 上下文：请求列表新增「缓存」列（端点回报的缓存命中占提示 token 的比例），详情头补充缓存命中率。
- 上下文：新增「动作」卡片，按文件 / 命令 / 网络 / MCP / 插件 / 读取汇总所选请求的工具调用，并列出文件活动、命令结果与失败项。
- 插件面板：`api.registerPanel` 端到端可用——引擎在 tools 事件里带上面板数据，上下文页在请求构成下方渲染（数字摘要 + 文本 / 条形 / 表格三种 section）；面板失败会记在插件卡错误里；随包附带示例插件 `plugins/panel-demo`，启用后即可看到效果。
- 插件审批：插件工具与 MCP 工具的确认卡改为「插件确认」/「MCP 确认」，提示与结束状态同步，不再显示成一次文件写入。
- 插件 / MCP 卡片：引擎的 tools 事件带上工具来源，插件卡显示上一次提问里贡献的「N 个工具 · N 段提示」，MCP 卡显示「N 个技能 · N 个工具」。
- 侧边栏：把「统计」移到「设置」之后。
- 插件 / Skill / MCP：标题栏的文件夹、刷新、添加按钮图标放大到 16px（按钮 24px），并与标题底部对齐；按钮之间收紧到 4px，和标题之间留白加大。
- Skill / 外部插件：标题旁的 + 从“生成示例”改为“添加”——打开文件夹对话框，选中后按 skill.json / plugin.json 与入口文件校验格式，通过才复制到对应目录并自动启用；MCP 页的生成示例按钮与三处示例生成代码一并移除。
- 统计：三张汇总卡片补足高度（128→144px），卡片底部的说明行（如“10 个对话有请求”“每问平均 4 次”）不再被压成一条残影。
- 外部插件：新增 `office`，通过 Windows Office COM 直接操作已打开的 Word/Excel；支持打开/状态、结构化读取、段落/表格/样式、单元格/公式/格式/工作表/图表、保存/另存为/PDF、受限制关闭；支持 `dry_run` 与审批。
- 模式：设置页新增“模式”（位于 AI 之后）：基础模式只发送内置工具，不带外部插件/Skill/MCP；全量模式发送所有手动启用的外部资源；自定义模式可按外部插件、Skill、MCP 三个列表分别勾选，并可直接编辑模式名称。插件/Skill/MCP 页面手动关闭的项在任一种模式下都不会发送。聊天输入框权限与轮次之间新增模式按钮，每个对话记住自己的模式。
- 统计：改为独立持久账本 `<exe>/data/stats`，不再随对话文件实时计算；请求明细保留 30 天，按 UTC 日汇总长期保留，删除对话不会回退历史统计；首次启动自动迁移旧请求记录。
- 设置：数据页显示统计账本路径并提供打开入口，诊断信息包含 `stats` 路径。
- 上下文面板：分类占比改为灰色图例列表，分类行只显示 `百分比 ≈ token`，占比条改为浅灰/灰；页面高度与 8 类布局重新调整。
- 聊天页：标题放大；聊天/上下文/后台任务改为小号灰色标签，选中深灰；后台任务从浮动小窗改为独立标签面板。
- 后台任务：结束进程前比较任务记录的启动时间与进程创建时间，避免 PID 复用误杀。
- 引擎：修复插件/MCP 工具在 allow-list 下被误拒绝的问题；shell 输出按行解码混合 UTF-8/GBK；补齐 6 个失败测试。
- 工程：恢复 `tsc --noEmit` 检查；新增根 README、LICENSE、Git 忽略规则、CI 与 `scripts/release.ps1`。
- 备份：按当前 `dataDir` 备份（支持外部数据目录）；新增两步确认的“恢复最近备份”，在下次启动时先自动备份当前状态再替换记录。
- 日志：新增 `logs/paagent.log`（2 MB 轮转）与 panic 崩溃报告；诊断页可打开日志目录，并附带日志尾部。
- 设置窗口：新增“数据”页，集中显示数据/工作区/备份/导出目录，并把备份、恢复、导出与日志入口移入该页；诊断页保留环境与片段管理，位置卡片已按需求删除。
- 外部插件：新增 `win32-windows`，通过 Python/Win32 只读枚举顶层窗口、按句柄展开子窗口/控件，并提供句柄、类名、标题、位置尺寸、PID、样式与进程路径。
- 外部插件：新增 `win32-input`，支持窗口聚焦、左/右/中键点击、双击、滚轮、Unicode 文字输入和按键组合；带 `dry_run`，默认不启用，每次调用需审批。
- 外部插件：`win32-windows` 增加 DPI 感知，窗口矩形/客户区坐标与 `win32-input` 的 `SendInput` 使用同一物理像素基准，修复高缩放比例下点击位置偏移。
- 外部插件：`win32-windows` 增加 `win32_observe`（UI Automation 元素树、编号、名称、控件类型、中心坐标、密码框标记和 45 秒快照）。
- 外部插件：`win32-input` 支持按快照+元素编号点击/输入/滚动，拒绝过期快照、密码框输入和未确认的危险词操作；新增拖拽与等待。