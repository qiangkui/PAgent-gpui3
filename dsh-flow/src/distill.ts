/**
 * Distill a saved conversation into a workflow draft.
 *
 * `derive` starts from a dsh session log and only turns shell commands into
 * steps. A chat run saved by the window is a different shape: `chat.json` is
 * one JSON document whose `messages` mix prose, thinking and tool rows, so
 * `derive` cannot read it (every line fails to parse). This module reads that
 * transcript and does the part that is still deterministic:
 *
 * - pair each assistant sentence with the tool call that followed it;
 * - drop exploration (`list` / `glob` / `read` / `grep` …) and page probing
 *   (`browser eval` / `snapshot` / `text` …): their conclusions belong in the
 *   SOP, not in the flow;
 * - merge a call repeated with the same arguments (a retry) into one step with
 *   `retries`;
 * - keep real actions in order as `run` steps (shell) or `skip` placeholders
 *   that name what a step program still has to do.
 *
 * It will not guess: no `ai` prompt or schema, no rewritten arguments, no
 * verdict about success. Those stay as TODO comments for a human (or an agent)
 * to review; `validate` / `test` / `explain` decide whether the result is a
 * workflow.
 */

/** How to name and select the derived draft. */
export interface DistillOptions {
  /** Workflow name; used as written (no slugging). Defaults to a slug of the title. */
  name?: string
  /** One-line description; defaults to a sentence naming the source. */
  description?: string
  /** Path of the chat file, only echoed in the report header. */
  source?: string
  /** Which user turn to distill (1-based); defaults to the last one. */
  turn?: number
}

/** One tool call, normalized out of the transcript. */
export interface DistillAction {
  /** Position in the distilled turn, counting from one. */
  index: number
  /** Tool name, taken from the display line before ` · `. */
  tool: string
  /** Browser action, or the tool's own `action` argument; empty otherwise. */
  action: string
  /** What the tool row showed after ` · `. */
  detail: string
  /** Assistant sentence just before this call, if there was one. */
  intent: string
  /** Parsed `input`, or null when it was not JSON. */
  args: Record<string, unknown> | null
  /** `input` as stored, for the report when parsing failed. */
  rawInput: string
  /** First line of `output`. */
  output: string
  /** False when the tool row was marked failed. */
  ok: boolean
  /** Elapsed milliseconds the window recorded, when it did. */
  ms: number | null
  /** Pictures the call produced. */
  images: string[]
  /** Files the call produced or presented. */
  files: string[]
  /** What the draft did with it: a step, folded into the step before, or dropped. */
  verdict: 'step' | 'merged' | 'dropped'
  /** Why it was kept or dropped: `command` / `browser` / `probe` / `explore` … */
  kind: string
  /** One sentence explaining the verdict. */
  reason: string
  /** True when a model likely had to look at something (an AI-step candidate). */
  ai: boolean
  /** How many times the same call appeared in one run (1 unless merged). */
  repeat: number
  /** Step id this action fed, when it was kept. */
  stepId: string | null
  /** True when the stored arguments or output hit the transcript's size cap. */
  truncated: boolean
}

/** Counts a reader can check at a glance. */
export interface DistillSummary {
  /** Conversation title from the file. */
  title: string
  /** Conversation folder from the file, for example `chat-3`. */
  dir: string
  /** Which user turn was distilled. */
  turn: number
  /** How many user turns the file holds. */
  turns: number
  /** Messages scanned (the selected turn only). */
  messages: number
  /** Tool calls in the selected turn. */
  tools: number
  /** Steps written to the draft. */
  steps: number
  /** Steps with a real `run` command. */
  commands: number
  /** Steps that are `skip` placeholders for a script still to write. */
  placeholders: number
  /** Calls folded into the step before them. */
  merged: number
  /** Calls that repeat a step but are separated by other kept steps. */
  repeated: number
  /** Calls left out of the draft. */
  dropped: number
  /** Dropped page probing (browser eval / snapshot / text …). */
  probes: number
  /** Dropped exploration (list / glob / read / grep …). */
  explores: number
  /** Points a model likely decided (final conclusion, images to read). */
  judgments: number
  /** Stored arguments or outputs that hit the transcript's cap. */
  truncated: number
  /** Thinking rows skipped. */
  thinking: number
  /** Attachments skipped (they are inputs, not steps). */
  attachments: number
}

/** The draft, the evidence behind it, and the review report. */
export interface DistillReport {
  /** Complete `flow.draft.yml` text. */
  draft: string
  /** Every tool call, with what happened to it. */
  actions: DistillAction[]
  /** Complete `distill.md` text. */
  report: string
  /** Counts for the CLI summary. */
  summary: DistillSummary
}

/** One step as the draft writes it: the first call plus how often it repeated. */
interface Group {
  /** Tool, action and arguments; equal keys mean the same call. */
  key: string
  /** The first call that opened the step. */
  first: DistillAction
  /** Calls folded into this step. */
  repeat: number
  /** How many of them failed. */
  failures: number
  /** Step id written to the draft. */
  stepId: string
  /** True once a different kept call opened the next step. */
  closed: boolean
}

/** Steps that share one call but are not adjacent, so they were not merged. */
interface RepeatGroup {
  /** Tool, action and arguments shared by the steps. */
  key: string
  /** The steps that repeat it. */
  steps: Group[]
}

/** One message of `chat.json`, as far as this module reads it. */
interface ChatMessage {
  /** `user` / `assistant` / `thinking` / `tool` / `attachment`. */
  role?: unknown
  /** Display text: the tool line for `tool`, the prose otherwise. */
  text?: unknown
  /** Tool arguments as stored (a JSON string). */
  input?: unknown
  /** Tool result as stored. */
  output?: unknown
  /** True when the call failed. */
  failed?: unknown
  /** Wall clock the call took. */
  elapsedMs?: unknown
  /** Files a `show` presented, or the browser's downloads. */
  files?: unknown
  /** Picture path a `screenshot` / `read_image` call produced. */
  imagePath?: unknown
}

/**
 * Distill one turn of a saved conversation into a draft workflow.
 * @param source - the `chat.json` text.
 * @param options - name, description, source path and turn selection.
 * @returns the draft, the per-call evidence and the review report.
 */
export function distillChat(source: string, options: DistillOptions = {}): DistillReport {
  const doc = readChat(source)
  const title = text(doc.title) || '未命名会话'
  const dir = text(doc.dir)
  const messages = Array.isArray(doc.messages) ? (doc.messages as ChatMessage[]) : []
  const userAt: number[] = []
  messages.forEach((message, at) => {
    if (text(message.role) === 'user') userAt.push(at)
  })
  const turns = Math.max(userAt.length, 1)
  const turn = clampInteger(options.turn, 1, turns)
  const from = userAt.length === 0 ? 0 : userAt[turn - 1]!
  const to = turn < userAt.length ? userAt[turn]! : messages.length

  const actions: DistillAction[] = []
  const groups: Group[] = []
  const used = new Set<string>()
  let pending = ''
  let thinking = 0
  let attachments = 0
  let judgments = 0
  let truncated = 0

  for (let at = from; at < to; at += 1) {
    const message = messages[at]!
    const role = text(message.role)
    if (role === 'thinking') {
      thinking += 1
      continue
    }
    if (role === 'attachment') {
      attachments += 1
      continue
    }
    if (role === 'assistant') {
      const prose = text(message.text)
      if (prose !== '') pending = prose
      continue
    }
    if (role !== 'tool') continue

    const action = normalizeAction(message, actions.length + 1, pending)
    pending = ''
    actions.push(action)
    if (action.truncated) truncated += 1
    if (action.ai) judgments += 1
    if (action.verdict !== 'step') continue

    const previous = groups[groups.length - 1]
    if (previous !== undefined && !previous.closed && previous.key === actionKey(action)) {
      previous.repeat += 1
      previous.first.repeat = previous.repeat
      if (!action.ok) previous.failures += 1
      action.verdict = 'merged'
      action.stepId = previous.stepId
      continue
    }
    if (previous !== undefined) previous.closed = true
    const id = uniqueId(stepBaseId(action), used)
    groups.push({
      key: actionKey(action),
      first: action,
      repeat: 1,
      failures: action.ok ? 0 : 1,
      stepId: id,
      closed: false,
    })
    action.stepId = id
  }

  const conclusion = pending
  if (conclusion !== '') judgments += 1

  const repeated = separatedRepeats(groups).reduce((sum, entry) => sum + entry.steps.length - 1, 0)
  const commands = groups.filter(group => runnableCommand(group.first)).length
  const summary: DistillSummary = {
    title,
    dir,
    turn,
    turns,
    messages: to - from,
    tools: actions.length,
    steps: groups.length,
    commands,
    placeholders: groups.length - commands,
    merged: actions.filter(action => action.verdict === 'merged').length,
    repeated,
    dropped: actions.filter(action => action.verdict === 'dropped').length,
    probes: actions.filter(action => action.verdict === 'dropped' && action.kind === 'probe').length,
    explores: actions.filter(action => action.verdict === 'dropped' && action.kind === 'explore').length,
    judgments,
    truncated,
    thinking,
    attachments,
  }

  const explicitName = options.name === undefined ? '' : options.name.trim()
  const name = explicitName !== '' ? explicitName : slug(title) || 'chat-distill'
  const description = options.description ?? `从聊天记录「${title}」派生的草稿（待 review）`
  const origin = options.source ?? ''
  return {
    draft: buildDraft(name, description, groups, summary, origin),
    actions,
    report: buildReport(summary, actions, groups, conclusion, origin),
    summary,
  }
}

/** Tools whose only job is finding files or reading context. */
const EXPLORE_TOOLS = new Set(['list', 'glob', 'tree', 'grep', 'symbols', 'read', 'git', 'env'])

/** Tools that change files or the repository. */
const FILE_TOOLS = new Set(['write', 'edit', 'replace', 'move', 'copy', 'mkdir', 'delete', 'git_write'])

/** Tools that run a command a `run` step can replay as-is. */
const COMMAND_TOOLS = new Set(['shell'])

/** Tools that run inline code; a script file is needed before it can be a step. */
const CODE_TOOLS = new Set(['python', 'node'])

/** Tools that check or wait, which belong in the flow as gates. */
const CHECK_TOOLS = new Set(['run_tests', 'diagnostics', 'wait_for', 'task'])

/** Tools that hand files to the reader; these become the workflow's artifacts. */
const DELIVER_TOOLS = new Set(['show'])

/** Tools that make one request; a step program should own the replay. */
const HTTP_TOOLS = new Set(['http'])

/** Browser actions, split by what they mean to a workflow. */
const BROWSER_NAV = new Set(['open', 'goto'])
const BROWSER_ACT = new Set(['click', 'type', 'press', 'select'])
const BROWSER_DELIVER = new Set(['downloads', 'wait_download'])
const BROWSER_PROBE = new Set(['snapshot', 'text', 'eval', 'wait_for', 'console', 'network', 'close'])

/** A verdict plus the sentence that explains it. */
interface Verdict {
  /** Whether the call becomes a step (or may be merged into one). */
  verdict: 'step' | 'dropped'
  /** The kind written next to it in `actions.json`. */
  kind: string
  /** Why it is kept or dropped. */
  reason: string
  /** True when a model likely had to look at something. */
  ai: boolean
}

/**
 * Normalize one tool row.
 * @param message - the transcript row.
 * @param index - position in the turn, counting from one.
 * @param intent - the assistant sentence that preceded it.
 * @returns the call plus what the draft did with it.
 */
function normalizeAction(message: ChatMessage, index: number, intent: string): DistillAction {
  const display = text(message.text)
  const parts = display.split(' · ')
  const tool = (parts[0] ?? '').trim() || 'tool'
  const detail = parts.slice(1).join(' · ').trim()
  const rawInput = text(message.input)
  const args = parseArguments(rawInput)
  const action = text(args?.action).toLowerCase()
  const output = text(message.output)
  const images = imagesOf(message)
  const verdict = classify(tool, action, args, images)
  return {
    index,
    tool,
    action,
    detail,
    intent,
    args,
    rawInput,
    output: headline(output, 140),
    ok: message.failed !== true,
    ms: typeof message.elapsedMs === 'number' ? message.elapsedMs : null,
    images,
    files: filesOf(message, args, tool),
    verdict: verdict.verdict,
    kind: verdict.kind,
    reason: verdict.reason,
    ai: verdict.ai,
    repeat: 1,
    stepId: null,
    truncated: rawInput.length >= 8000 || output.length >= 8000,
  }
}

/**
 * Decide whether one call belongs in the flow.
 *
 * The line is drawn by what a replay needs: a call that changes a page, a file
 * or a download is part of the process; a call that only looks at something
 * only informed the person writing the process.
 * @param tool - tool name.
 * @param action - browser action or the tool's own `action`.
 * @param args - parsed arguments, for the details that change the verdict.
 * @param images - pictures the call produced.
 * @returns the verdict.
 */
function classify(tool: string, action: string, args: Record<string, unknown> | null, images: string[]): Verdict {
  if (COMMAND_TOOLS.has(tool)) return keep('command', '可重放的命令')
  if (CODE_TOOLS.has(tool)) return keep('code', '内联代码；先落成 steps/ 下的脚本才能重放')
  if (FILE_TOOLS.has(tool)) return keep('file', '文件改动；在流程里应由步骤脚本完成')
  if (HTTP_TOOLS.has(tool)) return keep('http', '一次请求；在流程里应由步骤脚本完成，凭据走 secret')
  if (DELIVER_TOOLS.has(tool)) return keep('deliver', '交付产物；应成为 workflow 的 artifacts')
  if (CHECK_TOOLS.has(tool)) return keep('check', '校验或等待；可成为流程的闸门步骤')
  if (EXPLORE_TOOLS.has(tool)) {
    return drop('explore', '找文件或读上下文，不进流程；结论应写进 SOP', false)
  }
  if (tool === 'read_image' || tool === 'screenshot') {
    return drop('probe', '看图或截图用于判断/探索；如果是流程判断点，提升成 ai 步骤', true)
  }
  if (tool === 'browser') {
    if (BROWSER_NAV.has(action)) return keep('browser', '页面导航；折叠进站点适配脚本')
    if (BROWSER_ACT.has(action)) return keep('browser', '页面操作；折叠进站点适配脚本')
    if (BROWSER_DELIVER.has(action)) return keep('deliver', '下载或等下载；由脚本里的动作触发')
    if (action === 'screenshot') {
      const target = text(args?.selector) || text(args?.ref)
      if (images.length === 0 || target === '') {
        return drop('probe', '视口截图用于查看页面，不是产物；用 selector/ref 截元素才是', false)
      }
      return {
        verdict: 'step',
        kind: 'artifact',
        reason: '元素截图可成为产物；若下一步要读懂它，提升成 ai',
        ai: true,
      }
    }
    if (BROWSER_PROBE.has(action)) {
      return drop('probe', '页面探测（找元素/读状态）；折叠进站点适配脚本', false)
    }
    return keep('browser', '页面动作；确认能否折叠进站点适配脚本')
  }
  return keep('unknown', '未识别的工具；确认能否脚本化后再跑')
}

/**
 * A kept call.
 * @param kind - what kind of step it suggests.
 * @param reason - why it is kept.
 * @returns the verdict.
 */
function keep(kind: string, reason: string): Verdict {
  return { verdict: 'step', kind, reason, ai: false }
}

/**
 * A dropped call.
 * @param kind - `explore` or `probe`.
 * @param reason - why it is dropped.
 * @param ai - whether it looks like a model judgement.
 * @returns the verdict.
 */
function drop(kind: string, reason: string, ai: boolean): Verdict {
  return { verdict: 'dropped', kind, reason, ai }
}

/**
 * Compare two calls: same tool, same action, same arguments.
 * @param action - the call to key.
 * @returns the key.
 */
function actionKey(action: DistillAction): string {
  const args = action.args === null ? action.rawInput.trim() : JSON.stringify(action.args)
  return `${action.tool}\u0000${action.action}\u0000${args}`
}

/**
 * Steps that share one call without being adjacent.
 *
 * A repeat separated by another kept step is not a retry: the person did
 * something in between. It is reported instead of merged, because only the
 * reviewer can say whether it was a rerun of the same work.
 * @param groups - kept steps, in order.
 * @returns one entry per repeated call, when it opened more than one step.
 */
function separatedRepeats(groups: Group[]): RepeatGroup[] {
  const byKey = new Map<string, Group[]>()
  for (const group of groups) {
    const list = byKey.get(group.key) ?? []
    list.push(group)
    byKey.set(group.key, list)
  }
  const repeats: RepeatGroup[] = []
  for (const [key, steps] of byKey) {
    if (steps.length > 1) repeats.push({ key, steps })
  }
  return repeats
}

/**
 * The step id a kept call would like to have.
 * @param action - the call.
 * @returns a manifest-safe base id.
 */
function stepBaseId(action: DistillAction): string {
  if (action.tool === 'shell') return 'shell'
  if (action.tool === 'show') return 'show'
  return action.action === '' ? action.tool : `${action.tool}-${action.action}`
}

/**
 * Whether a kept call can be written as a real `run` step.
 *
 * Only a shell command is replayable as-is. Inline code, page actions and file
 * edits need a step program, so they stay `skip` placeholders.
 * @param action - the first call of a group.
 * @returns true when the draft may emit `run`.
 */
function runnableCommand(action: DistillAction): boolean {
  if (action.tool !== 'shell') return false
  const args = action.args
  if (args === null || typeof args.command !== 'string' || args.command.trim() === '') return false
  return args.background !== true
}

/**
 * Read a `chat.json` document.
 * @param source - the file text.
 * @returns the document, with `messages` guaranteed to be a list.
 */
function readChat(source: string): Record<string, unknown> {
  let doc: unknown
  try {
    doc = JSON.parse(source)
  } catch (error) {
    throw new Error(`聊天记录不是 JSON：${messageOf(error)}`)
  }
  if (!isRecord(doc)) throw new Error('聊天记录必须是一个 JSON 对象')
  if (!Array.isArray(doc.messages)) {
    throw new Error('不是 dsh 聊天记录：缺少 messages 列表（应传 data/chats/chat-N/chat.json）')
  }
  return doc
}

/**
 * Parse stored tool arguments.
 * @param raw - the `input` text.
 * @returns the object, or null when it was empty, truncated or not an object.
 */
function parseArguments(raw: string): Record<string, unknown> | null {
  const trimmed = raw.trim()
  if (trimmed === '' || !(trimmed.startsWith('{') || trimmed.startsWith('['))) return null
  try {
    const parsed: unknown = JSON.parse(trimmed)
    return isRecord(parsed) ? parsed : null
  } catch {
    return null
  }
}

/**
 * Picture paths a tool row carries.
 * @param message - the row.
 * @returns the paths, at most one from `imagePath`.
 */
function imagesOf(message: ChatMessage): string[] {
  const one = text(message.imagePath)
  return one === '' ? [] : [one]
}

/**
 * Files a tool row carries.
 *
 * `show` puts them in its arguments; older rows may also carry a `files` list.
 * @param message - the row.
 * @param args - parsed arguments, for `show`.
 * @param tool - tool name.
 * @returns the paths, filtered to non-empty strings.
 */
function filesOf(message: ChatMessage, args: Record<string, unknown> | null, tool: string): string[] {
  const listed = Array.isArray(message.files)
    ? message.files.map(entry => text(entry)).filter(entry => entry !== '')
    : []
  if (listed.length > 0) return listed
  if (tool !== 'show' || args === null) return []
  const paths = Array.isArray(args.paths) ? args.paths : [args.path]
  return paths.map(entry => text(entry)).filter(entry => entry !== '')
}

/**
 * Build the draft manifest.
 * @param name - workflow name.
 * @param description - one-line description.
 * @param groups - kept steps, in order.
 * @param summary - counts for the header.
 * @param origin - chat file path, for the header.
 * @returns the YAML text.
 */
function buildDraft(
  name: string,
  description: string,
  groups: Group[],
  summary: DistillSummary,
  origin: string,
): string {
  const lines: string[] = []
  lines.push(`# 由聊天记录「${summary.title}」派生：只是草稿，review 之后再运行。`)
  lines.push(`# 来源：${origin === '' ? '(未记录)' : origin} · 第 ${summary.turn}/${summary.turns} 轮`)
  lines.push(
    `# 原始 ${summary.tools} 次工具调用 → ${summary.steps} 步（命令 ${summary.commands} · 占位 ${summary.placeholders}）` +
    ` · 合并 ${summary.merged} · 重复 ${summary.repeated} · 丢弃 ${summary.dropped}`,
  )
  if (summary.truncated > 0) {
    lines.push(`# 有 ${summary.truncated} 处参数或输出超过聊天记录的保存上限（约 8000 字符），需要回看原始调用。`)
  }
  lines.push('# 命令在原会话目录里跑；步骤的 cwd 是清单目录，相对路径可能要改。')
  lines.push('')
  lines.push(`name: ${quote(name)}`)
  lines.push(`description: ${quote(description)}`)
  lines.push('version: 1')
  lines.push('')
  lines.push('steps:')

  if (groups.length === 0) {
    lines.push('  # TODO: 这一轮没有可重放的动作，按下面的判断点手工补步骤。')
    lines.push('  - id: todo')
    lines.push('    set:')
    lines.push(`      todo: ${quote('TODO — 用真实步骤替换')}`)
  }

  for (const group of groups) {
    const action = group.first
    lines.push(`  # 原始调用 ${action.index}：${oneLine(`${action.tool} ${action.detail}`, 100)}`)
    lines.push(`  - id: ${group.stepId}`)
    lines.push(`    desc: ${quote(stepDesc(group))}`)
    if (runnableCommand(action)) {
      const command = String(action.args?.command ?? '')
      emitRun(lines, command)
      if (group.repeat > 1) {
        lines.push(`    retries: ${group.repeat - 1}`)
        lines.push('    retryDelayMs: 500')
      }
      continue
    }
    lines.push('    skip: true')
    lines.push('    set:')
    lines.push(`      todo: ${quote(todoText(action))}`)
  }

  lines.push('')
  lines.push('# TODO（派生时刻意没有推断的部分）：')
  const todos = reviewTodos(summary, groups)
  if (todos.length === 0) lines.push('#   - （无）')
  for (const todo of todos) lines.push(`#   - ${todo}`)
  return `${lines.join('\n')}\n`
}

/**
 * Write the `run:` line of a shell command, as a scalar or a block.
 * @param lines - the draft being built.
 * @param command - the command text from the transcript.
 */
function emitRun(lines: string[], command: string): void {
  const body = command.replace(/\r\n/g, '\n').split('\n')
  while (body.length > 1 && body[body.length - 1] === '') body.pop()
  if (body.length === 1) {
    lines.push(`    run: ${quote(body[0] ?? '')}`)
    return
  }
  lines.push('    run: |')
  for (const line of body) lines.push(line === '' ? '' : `      ${line}`)
}

/**
 * The card line for one step.
 * @param group - the step.
 * @returns a short description.
 */
function stepDesc(group: Group): string {
  const action = group.first
  const base = action.intent === '' ? argSummary(action) : headline(action.intent, 80)
  const suffix = group.repeat > 1 ? `（重复 ${group.repeat} 次）` : ''
  const text = runnableCommand(action)
    ? `${base}${suffix}`
    : `${base}（待实现：${placeholderHint(action)}）${suffix}`
  return clip(text, 140)
}

/**
 * What a `skip` placeholder still needs.
 * @param action - the kept call.
 * @returns one short phrase.
 */
function placeholderHint(action: DistillAction): string {
  if (action.kind === 'code') return '把内联代码落成 steps/ 下的脚本'
  if (action.kind === 'file') return '由步骤脚本改文件'
  if (action.kind === 'http') return '由步骤脚本发请求，凭据走 secret'
  if (action.kind === 'deliver') return '作为 workflow 的 artifacts'
  if (action.kind === 'artifact') return '截图作为产物'
  if (action.kind === 'browser') return '折叠进站点适配脚本'
  if (action.kind === 'check') return '做成闸门步骤'
  return '确认能否脚本化'
}

/**
 * The `todo` value of a placeholder step: what the original call did.
 * @param action - the kept call.
 * @returns one line, templates escaped.
 */
function todoText(action: DistillAction): string {
  return clip(oneLine(argSummary(action), 120).replace(/\{\{/g, '{ {'), 120)
}

/**
 * What a call did, from its arguments rather than its result line.
 *
 * The tool row's display text ends with what came back ("退出码 0", "已截图…"),
 * which reads like an outcome and makes a noisy `desc`. The arguments say what
 * the step is supposed to do.
 * @param action - the call.
 * @returns one short line, e.g. ``browser open https://…``.
 */
function argSummary(action: DistillAction): string {
  const verb = action.action === '' ? action.tool : `${action.tool} ${action.action}`
  const args = action.args
  if (args !== null) {
    const keys = action.tool === 'show'
      ? ['paths', 'path']
      : action.tool === 'browser'
        ? ['url', 'selector', 'ref', 'text', 'expression']
        : ['command', 'path', 'pattern', 'url', 'selector']
    for (const key of keys) {
      const value = args[key]
      if (typeof value === 'string' && value.trim() !== '') {
        return clip(`${verb} ${value.trim()}`, 100)
      }
      if (Array.isArray(value)) {
        const items = value.filter((entry): entry is string => typeof entry === 'string' && entry.trim() !== '')
        if (items.length > 0) {
          const extra = items.length > 1 ? ` 等 ${items.length} 个` : ''
          return clip(`${verb} ${items[0]!.trim()}${extra}`, 100)
        }
      }
    }
  }
  return clip(`${verb}${action.detail === '' ? '' : ` · ${action.detail}`}`, 100)
}

/**
 * The review checklist at the end of the draft.
 * @param summary - counts.
 * @param groups - kept steps.
 * @returns one line per item.
 */
function reviewTodos(summary: DistillSummary, groups: Group[]): string[] {
  const todos: string[] = []
  if (summary.judgments > 0) {
    todos.push('判断点：看图或看内容得出的结论不能进代码；确属流程判断的，写成 ai 步骤（why + schema + cache: false）')
  }
  if (summary.probes > 0) {
    todos.push(`页面探测 ${summary.probes} 次已丢弃：把定位到的选择器/等待条件写进站点适配脚本，不要一步一个 eval`)
  }
  if (summary.explores > 0) {
    todos.push(`探索 ${summary.explores} 次已丢弃：路径与结论写进 SOP，流程里用常量`)
  }
  const placeholders = groups.filter(group => !runnableCommand(group.first))
  if (placeholders.length > 0) {
    todos.push(
      `${placeholders.length} 个 skip 占位（${placeholders.map(group => group.stepId).join(' / ')}）：` +
      '实现后用真实 run/use 步骤替换',
    )
  }
  const separated = separatedRepeats(groups)
  if (separated.length > 0) {
    todos.push(
      `${separated.length} 组同名同参被其它步骤隔开（` +
      `${separated.map(entry => entry.steps.map(step => step.stepId).join('/')).join('、')}）：` +
      '是重试就合成一步 + retries，是两次真实运行就保留',
    )
  }
  if (groups.some(group => group.first.kind === 'file')) {
    todos.push('文件改动占位：如果改的是工作流自己的脚本/配置，删掉这一步；只有业务产物的改动才留在流程里')
  }
  const files = new Set<string>()
  for (const group of groups) for (const file of group.first.files) files.add(file)
  if (files.size > 0) {
    todos.push(`产物候选（${files.size} 个）：在实现步骤时通过 stdout 的 artifacts 交出，例如 ${[...files][0]}`)
  }
  if (summary.truncated > 0) {
    todos.push(`${summary.truncated} 处参数/输出被记录上限截断：实现步骤前回看原始调用`)
  }
  return todos
}

/**
 * Build the review report that sits next to the draft.
 * @param summary - counts.
 * @param actions - every call, with its verdict.
 * @param groups - kept steps.
 * @param conclusion - the assistant's last prose, when there was some.
 * @param origin - chat file path.
 * @returns the Markdown text.
 */
function buildReport(
  summary: DistillSummary,
  actions: DistillAction[],
  groups: Group[],
  conclusion: string,
  origin: string,
): string {
  const lines: string[] = []
  lines.push('# 聊天记录固化报告')
  lines.push('')
  lines.push(`- 来源：\`${origin === '' ? '(未记录)' : origin}\``)
  lines.push(`- 会话：${summary.title}${summary.dir === '' ? '' : `（\`${summary.dir}\`）`}`)
  lines.push(`- 轮次：第 ${summary.turn} / ${summary.turns} 轮 · 扫描 ${summary.messages} 条消息`)
  lines.push(
    `- 工具调用：${summary.tools} → 保留 ${summary.steps} 步（命令 ${summary.commands} · 占位 ${summary.placeholders}）` +
    ` · 合并 ${summary.merged} · 重复 ${summary.repeated} · 丢弃 ${summary.dropped}`,
  )
  lines.push(`- 产物：\`flow.draft.yml\` · \`actions.json\` · \`distill.md\``)
  lines.push('')

  lines.push('## 动作清单')
  lines.push('')
  lines.push('| # | 工具 | 调用 | 结果 | 处置 | 原因 |')
  lines.push('|---|---|---|---|---|---|')
  for (const action of actions) {
    const name = action.action === '' ? action.tool : `${action.tool}.${action.action}`
    const what = cell(argSummary(action), 50)
    const result = action.ok ? `ok${action.ms === null ? '' : ` · ${action.ms} ms`}` : '失败'
    const verdict = action.verdict === 'step'
      ? `保留 → \`${action.stepId ?? ''}\``
      : action.verdict === 'merged'
        ? `合并 → \`${action.stepId ?? ''}\``
        : '丢弃'
    lines.push(`| ${action.index} | \`${name}\` | ${what} | ${result} | ${verdict} | ${cell(action.reason, 40)} |`)
  }
  if (actions.length === 0) lines.push('| — | — | （这一轮没有工具调用） | — | — | — |')
  lines.push('')

  lines.push('## 合并与重试')
  lines.push('')
  const merged = groups.filter(group => group.repeat > 1)
  const separated = separatedRepeats(groups)
  if (merged.length === 0 && separated.length === 0) {
    lines.push('- 没有同名同参的重复调用。')
  }
  for (const group of merged) {
    const action = group.first
    const name = action.action === '' ? action.tool : `${action.tool}.${action.action}`
    const outcome = runnableCommand(action)
      ? `1 个 run 步骤 + \`retries: ${group.repeat - 1}\``
      : '1 个占位步骤（重复次数写在 desc）'
    lines.push(
      `- \`${group.stepId}\`：\`${name}\` ×${group.repeat}（失败 ${group.failures} 次）→ ${outcome}`,
    )
  }
  if (separated.length > 0) {
    if (merged.length > 0) lines.push('')
    lines.push('同名同参但不相邻，派生时刻意没有合并：')
    for (const entry of separated) {
      const first = entry.steps[0]!.first
      const name = first.action === '' ? first.tool : `${first.tool}.${first.action}`
      const ids = entry.steps.map(step => `\`${step.stepId}\``).join(' / ')
      lines.push(`- ${ids}：\`${name}\`（被其它步骤隔开）→ 是重试就合成一步 + retries，是两次真实运行就保留`)
    }
  }
  lines.push('')

  lines.push('## 被丢弃的调用')
  lines.push('')
  const dropped = actions.filter(action => action.verdict === 'dropped')
  if (dropped.length === 0) {
    lines.push('- 没有丢弃任何调用。')
  } else {
    const counts = new Map<string, number>()
    for (const action of dropped) {
      const name = action.action === '' ? `${action.tool}` : `${action.tool}.${action.action}`
      const key = `${action.kind}：\`${name}\``
      counts.set(key, (counts.get(key) ?? 0) + 1)
    }
    for (const [key, count] of counts) lines.push(`- ${key} ×${count}`)
    lines.push('')
    lines.push('探索与探测不进流程：它们的结论（路径、选择器、等待条件）应该成为脚本常量和 SOP，而不是一步一个调用。')
  }
  lines.push('')

  lines.push('## 判断点候选')
  lines.push('')
  const candidates = actions.filter(action => action.ai)
  if (conclusion === '') {
    lines.push('- 没有记录到收尾结论。')
  } else {
    lines.push(`- 收尾结论：${cell(conclusion, 160)}`)
  }
  for (const action of candidates) {
    const name = action.action === '' ? action.tool : `${action.tool}.${action.action}`
    lines.push(`- 第 ${action.index} 项 \`${name}\`：${cell(action.detail, 80)}`)
  }
  if (summary.judgments > 0) {
    lines.push('')
    lines.push('如果某个结论只能靠看图/看内容得到，把它写成 `ai` 步骤：`why` + `schema` + `cache: false`（验证码这类输入每次都变）。')
  }
  lines.push('')

  lines.push('## 产物与输入候选')
  lines.push('')
  const files = new Set<string>()
  for (const action of actions) for (const file of action.files) files.add(file)
  const images = new Set<string>()
  for (const action of actions) for (const image of action.images) images.add(image)
  if (files.size === 0 && images.size === 0) {
    lines.push('- 没有记录到文件或图片；逐步实现时用 stdout 的 `artifacts` 交出产物。')
  }
  for (const file of files) lines.push(`- 文件：\`${file}\``)
  for (const image of images) lines.push(`- 图片：\`${image}\``)
  lines.push('- URL、路径、账号、地区这类每次可以不同的值 → `inputs`；密钥 → `secret: true`。')
  if (summary.attachments > 0) {
    lines.push(`- 会话里的 ${summary.attachments} 个附件是输入候选，不是步骤。`)
  }
  if (summary.thinking > 0) {
    lines.push(`- ${summary.thinking} 段思考是过程记录，未纳入（要的是当时做了什么，不是怎么想的）。`)
  }
  if (summary.truncated > 0) {
    lines.push(`- ${summary.truncated} 处参数/输出超过聊天记录的保存上限（约 8000 字符），实现前回看原始调用。`)
  }
  lines.push('')

  lines.push('## 下一步')
  lines.push('')
  lines.push('1. 实现 `skip` 占位步骤（同类页面动作折叠成一个站点适配脚本，同类文件改动折叠成一个步骤脚本）。')
  lines.push('2. `dsh-flow validate flow.draft.yml`。')
  lines.push('3. `dsh-flow test flow.draft.yml`：给确定性步骤加 fixture。')
  lines.push('4. `dsh-flow run flow.draft.yml`。')
  lines.push('5. `dsh-flow explain flow.draft.yml`：看 AI 依赖度，目标是在测试全绿的前提下让 `ai` 尽量少。')
  lines.push('')
  lines.push('review 规程见 Skill「工作流固化」：`skills/workflow-distill/SKILL.md`。')
  return `${lines.join('\n')}\n`
}

/**
 * One Markdown table cell: single line and safe separators.
 * @param value - the text.
 * @param limit - maximum characters.
 * @returns the cell text.
 */
function cell(value: string, limit: number): string {
  return clip(oneLine(value, limit), limit).replace(/\|/g, '\\|')
}

/**
 * Accept a string, ignoring anything else.
 * @param value - the value.
 * @returns the trimmed string, or an empty string.
 */
function text(value: unknown): string {
  return typeof value === 'string' ? value.trim() : ''
}

/**
 * First non-empty line of a text.
 * @param value - the text.
 * @param limit - maximum characters.
 * @returns the line.
 */
function headline(value: string, limit = 120): string {
  const line = value.split(/\r?\n/).map(entry => entry.trim()).find(entry => entry !== '') ?? ''
  return clip(line, limit)
}

/**
 * One line of a possibly multi-line text.
 * @param value - the text.
 * @param limit - maximum characters.
 * @returns the line.
 */
function oneLine(value: string, limit = 120): string {
  return clip(value.replace(/\s*\r?\n\s*/g, ' ').trim(), limit)
}

/**
 * Shorten a text with a visible ellipsis.
 * @param value - the text.
 * @param limit - maximum characters.
 * @returns the text, or its clipped form.
 */
function clip(value: string, limit: number): string {
  if (value.length <= limit) return value
  return `${value.slice(0, Math.max(0, limit - 1))}…`
}

/**
 * Turn text into a manifest-safe identifier fragment.
 * @param value - the text.
 * @returns lowercase letters, digits, `_` and `-`.
 */
function slug(value: string): string {
  return value
    .toLowerCase()
    .replace(/[^a-z0-9_-]+/g, '-')
    .replace(/^-+|-+$/g, '')
}

/**
 * Make a step id unique inside the draft.
 * @param base - the preferred id.
 * @param used - ids already taken.
 * @returns the id, with `-2`, `-3` appended when needed.
 */
function uniqueId(base: string, used: Set<string>): string {
  const cleaned = slug(base) || 'step'
  let candidate = cleaned
  let index = 2
  while (used.has(candidate)) {
    candidate = `${cleaned}-${index}`
    index += 1
  }
  used.add(candidate)
  return candidate
}

/**
 * Clamp a turn number, defaulting to the last turn.
 * @param value - the requested turn.
 * @param min - lowest allowed.
 * @param max - highest allowed.
 * @returns the turn to distill.
 */
function clampInteger(value: number | undefined, min: number, max: number): number {
  if (typeof value !== 'number' || !Number.isFinite(value)) return max
  return Math.min(max, Math.max(min, Math.trunc(value)))
}

/**
 * Quote a string as a YAML scalar.
 * @param value - the text.
 * @returns a double-quoted scalar, which also escapes newlines correctly.
 */
function quote(value: string): string {
  return JSON.stringify(value)
}

/**
 * Read an error's message.
 * @param error - the thrown value.
 * @returns its message.
 */
function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

/**
 * Report whether a value is a plain object.
 * @param value - candidate.
 * @returns true for non-array objects.
 */
function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}