/**
 * The assistant loop: ask the model, run the tools it asks for, ask again.
 *
 * A single request/answer pair is not enough once the model can touch files, so
 * this module keeps feeding tool results back until the model answers in prose.
 * Every tool runs inside the conversation directory (see `tools.ts`); the model
 * never sees or sets a path outside it.
 */

import {
  DEFAULT_BASE_URL,
  DEFAULT_MODEL,
  applyThinking,
  emptyAnswerReason,
  historyMessages,
  userContent
} from './ai.ts'
import type { AiRequest } from './ai.ts'
import { serverToolDeclarations, webEventRow, webReport, webSteps } from './web.ts'
import type { WebSearchReport } from './web.ts'
import {
  MAX_CODE_CHARS,
  MAX_IMAGE_BASE64,
  MAX_IMAGES_PER_QUESTION,
  MAX_MEMORY_ENTRY,
  MAX_SHELL_COMMAND,
  TOOLS,
  previewToolScoped,
  runToolScoped,
  safeEnd
} from './tools.ts'
import type { Sandbox, ToolDefinition, ToolFile, ToolImage, ToolResult } from './tools.ts'
import { outsidePaths, resolveCommandDir, sandboxRefusal } from './tools/sandbox.ts'
import { httpSummary } from './tools/http.ts'
import { browserNeedsCard, browserSummary } from './tools/browser.ts'
import { INTERACTION_NAMES, INTERACTION_TOOLS } from './tools/interaction.ts'
import { streamCompletion } from './stream.ts'
import { cacheHitSize, contextSize } from './stream.ts'
import { collectPanels, runPluginTool } from './plugins.ts'
import type { PanelReport, PluginError, PluginPanel, PluginTool } from './plugins.ts'
import type { McpTool } from './mcp.ts'
import type { WireToolCall } from './stream.ts'
import {
  compactLimits,
  compactTranscript,
  estimateTokens,
  fitForModel,
  transcriptTokens
} from './compact.ts'
import type { CompactReport } from './compact.ts'

/** How many model round-trips one question may take. */
export const MAX_ROUNDS = 6

/** How many tool calls one question may make in total. */
export const MAX_CALLS = 20

/** Highest round-trip budget a request may ask for. */
export const MAX_ROUNDS_LIMIT = 50

/** Highest tool-call budget a request may ask for. */
export const MAX_CALLS_LIMIT = 200

/** How long one tool may run before the loop moves on without its result. */
export const TOOL_TIMEOUT_MS = 60_000

/**
 * Tools that only read. Several of these in one round can run at once; a write
 * or a command stays serial so its approval cannot race another call.
 */
const READ_ONLY_TOOLS = new Set([
  'read',
  'list',
  'glob',
  'grep',
  'tree',
  'symbols',
  'git',
  'env',
  'read_image',
  'screenshot',
  // `show` only lists files that already exist: it writes nothing, so several
  // shows in one round can run together like the other readers.
  'show'
])

/**
 * Tools whose side effect has to be observed through.
 *
 * They are local file operations that finish in milliseconds; racing one
 * against the tool timer would report a timeout while the change still lands,
 * leaving the model wrong and the window's undo record behind the disk.
 */
const MUTATING_TOOLS = new Set(['write', 'edit', 'replace', 'move', 'delete', 'copy', 'mkdir'])

/** Guidance added while the conversation is in plan mode. */
const PLAN_GUIDE = [
  '当前处于计划模式：先只读调查，不要修改文件，也不要运行会改变状态的命令。',
  '把要做的事整理成一份完整计划（以 # 标题开头的 markdown），然后调用 exit_plan_mode 提交。',
  '用户批准后才能开始执行；用户要求修改时，按反馈修订计划再提交。'
].join('\n')

/**
 * Sent once when an answer described an action but called no tool.
 *
 * Small models do this: they write「我先打开这个页面看看」and end the turn, and the
 * person sees a conversation that looks like it died. The reminder names the
 * mistake and says where the tool list really lives, because the same replies
 * often claim a tool is missing while it is right there in the request's `tools`.
 */
const PROMISE_NUDGE =
  '（系统提醒）你上一条只说了要做什么，却没有调用任何工具，所以这一轮什么也没有发生。' +
  '如果还需要操作，请现在直接调用对应的工具——工具清单以随请求附带的 tools 为准，不要凭记忆判断某个工具不存在；' +
  '如果不需要工具或已经完成，请直接给出最终回答。'

/**
 * Whether an answer only *promises* to act.
 *
 * Only short replies that carry no code are considered: a real explanation that
 * happens to contain「我先…」should not be interrupted by a reminder.
 * @param text - the answer text.
 * @returns true when one reminder round is worth it.
 */
export function unfulfilledPromise(text: string): boolean {
  const trimmed = text.trim()
  if (trimmed === '' || trimmed.length > 240 || trimmed.includes('```')) return false
  const chinese =
    /(我先|让我|我这就|现在我|接下来我|我来)[^。！？\n]{0,12}(打开|看看|看一下|看下|读取|读一下|运行|执行|检查|搜索|搜一下|查一下|点开|点击|输入|登录|下载|抓取|分析)/
  const english =
    /^(let me|i'?ll|i will|first,?\s+i)\b[^\n]{0,60}\b(open|check|look at|read|run|search|fetch|navigate|click)\b/i
  return chinese.test(trimmed) || english.test(trimmed)
}

/** One todo item as the model writes it. */
export interface TodoUpdate {
  /** What has to be done. */
  text: string
  /** `pending`, `in_progress` or `completed`. */
  status: 'pending' | 'in_progress' | 'completed'
}

/** A structured question the model asks the person. */
export interface QuestionRequest {
  /** Provider id of the call, echoed back with the answer. */
  call: string
  /** What the model asks. */
  question: string
  /** The choices, in order. */
  options: string[]
  /** True when a free-form answer is allowed. */
  allowCustom: boolean
}

/** A plan submitted for approval. */
export interface PlanReviewRequest {
  /** Provider id of the call, echoed back with the decision. */
  call: string
  /** The plan, markdown. */
  plan: string
}

// `ask_user_question`, `todo_write`, `exit_plan_mode` and `remember` live in
// `tools/interaction.ts`, so the plugin registry can reserve the same names.

/** Longest tool argument text kept for the transcript's expanded row. */
export const MAX_STEP_INPUT = 2_000

/** Longest tool result text kept for the transcript's expanded row. */
export const MAX_STEP_OUTPUT = 4_000

/**
 * Longest tool arguments replayed to the model after their call has run.
 *
 * A `write` may carry up to `MAX_WRITE_CHARS` characters, and the assistant turn
 * that asked for it is resent on every later round of the same question:
 * compaction cannot reach it, because it belongs to the newest question. Once
 * the result is in, the body has served its purpose; a note keeps the pairing
 * (same id and name) without carrying the file through the rest of the loop.
 */
export const MAX_WIRE_ARGUMENTS = 8_000

/**
 * Longest tool result handed back to the model. `read` alone may return 20 万
 * characters, and every round resends the whole transcript, so one huge result
 * would crowd out the rest of the conversation.
 */
export const MAX_TOOL_MESSAGE = 20_000

/**
 * Total tool-result text kept across one question.
 *
 * Every round resends the whole transcript, so the per-result cap alone is not
 * enough: twenty results at the cap would cost 40 万 characters. Older results
 * are replaced by a note as soon as the total passes this budget; the message
 * itself stays, so each `tool_call_id` still has its answer.
 */
export const MAX_TOOL_CONTEXT = 60_000

/** What replaces a tool result that was dropped to stay inside the budget. */
export const DROPPED_TOOL_RESULT =
  '（较早的工具结果已省略，以免占满上下文；需要时请重新调用工具。）'

/**
 * What replaces a dropped result when its call is known.
 *
 * The model can read `read big.txt` and decide whether that result is still
 * worth re-running; the old generic note left it guessing which one went away.
 * @param label - tool name and target, as the step row shows them.
 * @returns the note that stands in for the dropped text.
 */
export function droppedToolResult(label: string): string {
  const shown = label.trim()
  return shown === ''
    ? DROPPED_TOOL_RESULT
    : `（较早的 ${shown} 结果已省略，以免占满上下文；需要时请重新调用。）`
}

/** How much tool text one result may keep, and how much the whole question may. */
export interface ToolBudgets {
  /** Characters kept from one result, head and tail. */
  message: number
  /** Characters kept across every result of the question. */
  context: number
}

/**
 * Tool-text budgets for one window.
 *
 * The old figures were written for a 128k window and applied to every model, so a
 * 1M model dropped the same results a 128k one did. The fractions below are
 * character counts: roughly two characters per token for code and Chinese, so
 * `message` is about 2% of the window in tokens and `context` about 15%. They
 * never go below the old constants, which keeps a small window at exactly the
 * behaviour it had.
 * @param windowTokens - the model's context window; 0 or absent means the default.
 * @returns the per-result and per-question character budgets.
 */
export function toolBudgets(windowTokens: number): ToolBudgets {
  const window =
    Number.isFinite(windowTokens) && windowTokens > 0 ? windowTokens : DEFAULT_WINDOW_TOKENS
  const message = Math.min(Math.max(Math.round(window * 0.04), MAX_TOOL_MESSAGE), 100_000)
  const context = Math.min(Math.max(Math.round(window * 0.3), MAX_TOOL_CONTEXT), 400_000)
  // Two results at the per-result cap have to fit, or one big read would be
  // dropped the moment it arrived.
  return { message, context: Math.max(context, message * 2) }
}

/**
 * Context window assumed when the caller names none.
 *
 * The window's own table answers the same figure for an unknown model, so the
 * engine compacts at the same point the gauge turns red.
 */
export const DEFAULT_WINDOW_TOKENS = 128_000

/** One tool call, as reported to the caller for display. */
export interface AgentStep {
  /** Tool name, for example `read`. */
  tool: string
  /** The path or pattern the call was about. */
  target: string
  /** False when the tool refused or failed. */
  ok: boolean
  /** First line of what the tool returned, trimmed for display. */
  detail: string
  /** Arguments as the model wrote them, capped for the expanded row. */
  input: string
  /** What the tool returned, capped for the expanded row. */
  output: string
  /** Provider id of the call; links a step to its approval card. */
  call: string
  /**
   * The picture the tool showed the model, when it showed one.
   *
   * Only the file is reported: the window keeps it so the next question can put
   * the same picture in front of the model again. Empty when the capture could
   * not be saved anywhere.
   */
  image?: { mime: string; path: string }
  /**
   * Finished files the tool put in front of the person, for `show`.
   *
   * The window draws one card per file under the row. Only files the model chose
   * to hand over are here: a browser download stays the model's business until it
   * calls `show`.
   */
  files?: ToolFile[]
}

/**
 * How far a question has got, reported before every round trip.
 *
 * The strip shows these while the answer is still arriving, so they have to come
 * from the loop that knows the budget, not from what the transcript happens to
 * hold.
 */
export interface AgentProgress {
  /** Round trip about to start, 1-based. */
  round: number
  /** Tool calls finished so far for this question. */
  calls: number
}

/** One write, command or request waiting for a person to allow or refuse it. */
export interface ApprovalRequest {
  /** Provider id of the call, echoed back with the decision. */
  call: string
  /** `write`, `edit` or a batch `replace`; also commands, requests and file actions. */
  tool: string
  /** The path the call would change, as the model wrote it. */
  target: string
  /** The same path, kept for the undo write. */
  path: string
  /** What the file would look like afterwards. */
  diff: string
  /** The file before the change, when small enough to undo. */
  before?: string
  /** The file after the change, for the same reason. */
  after?: string
  /** False when the call creates the file. */
  existed?: boolean
  /** True when `before` / `after` came along and undo is possible. */
  undoable: boolean
  /** For `shell`: the command line about to run. Empty for a write. */
  command?: string
  /** For `shell`: the directory as the model wrote it. Empty for a write. */
  cwd?: string
  /**
   * True when the call would send a credential (`http` with `auth`).
   *
   * The window must show the card even in 完全权限: a key leaving the machine is
   * the one action a blanket "run everything" setting should not cover.
   */
  secret?: boolean
}

/**
 * Where a question's wall clock went, in milliseconds.
 *
 * Only the two phases the loop itself drives are measured: the model round
 * trips (streaming included, so time-to-first-token is part of it) and the tool
 * runs. Everything else — compaction, approvals the person answered, prompt
 * assembly, process start — stays unattributed, which is what lets the window
 * show an honest "其他" bucket instead of guessing.
 */
export interface AgentTiming {
  /** Wall clock spent waiting for model round trips. */
  modelMs: number
  /** Wall clock spent running tools. */
  toolMs: number
}

/** What the loop produced. */
export interface AgentResult {
  /** The assistant's final text. */
  text: string
  /** Model that produced it. */
  model: string
  /** Token accounting for everything the question cost, summed over rounds. */
  usage?: unknown
  /**
   * Prompt + answer of the last round: what the next question carries into the
   * window. Zero when the provider reported no counts.
   */
  context: number
  /**
   * Model round trips this question took, the final answer included. Every
   * round sends the whole prompt again, so this is why the bill is a multiple
   * of the window figure.
   */
  rounds: number
  /**
   * What each round cost, oldest first, as `[prompt, completion, toolCalls]`.
   * Summed, this is the bill; the prompts are the repeated part, and `toolCalls`
   * says how many results that round added for the next one.
   */
  roundTokens: Array<[number, number, number]>
  /**
   * Estimated thinking tokens of each round, oldest first: output the bill
   * includes that the prompt never holds. Empty for providers that do not
   * stream reasoning, which reads as zero everywhere downstream.
   */
  roundReasoning: number[]
  /**
   * The thinking text itself, every round joined with a blank line: what the
   * window shows in a collapsible block above the answer. Empty when the
   * provider streams none.
   */
  reasoning: string
  /**
   * Prompt tokens the provider served from its cache, summed over rounds. The
   * bill is cheaper by this much, so the window shows it beside the spend.
   */
  cacheHits: number
  /**
   * Where the question's wall clock went: the model round trips and the tool
   * runs, both summed. The window subtracts them from the answer's elapsed time
   * to show what it could not attribute, and hides the card when both are zero
   * (an older engine reported neither).
   */
  timing: AgentTiming
  /**
   * What the provider's own server tools did for this question: searches and
   * page reads run on its side, so this is the only trace of them the window
   * gets besides the rows handed to `onStep`.
   */
  webSearch: WebSearchReport
  /**
   * The raw `server_tool_calls` and citation entries of every round, oldest
   * first. Kept so a caller can store them and pass them back on the next
   * question: the provider restores pages read earlier from them, and drops
   * that context silently when they are missing.
   */
  serverCalls: unknown[]
  /** Raw `url_citation` entries of every round, in arrival order. */
  annotations: unknown[]
  /**
   * What the context compaction did to this question, when it did anything.
   * Absent for a question that fit, so a short one is never annotated.
   */
  compaction?: CompactReport
}

/** Injectable side effects, so tests need no network. */
export interface AgentOptions {
  /** Fetch implementation to use; defaults to the global one. */
  fetchImpl?: typeof fetch
  /** Environment for the key and base URL fallbacks. */
  env?: Record<string, string | undefined>
  /** Absolute directory the tools may touch. */
  root: string
  /** Round-trip limit; defaults to {@link MAX_ROUNDS}. */
  maxRounds?: number
  /** Tool-call limit for the whole question; defaults to {@link MAX_CALLS}. */
  maxCalls?: number
  /** Folders outside `root` the tools may read, never write. */
  readOnlyRoots?: string[]
  /** Writable root; `null` means the whole computer, `undefined` means `root`. */
  scopeRoot?: string | null
  /** Top-level workspace folders the search tools leave out (sibling chats). */
  skipDirs?: readonly string[]
  /**
   * Folders `shell` / `python` / `node` may reach; null (or absent) skips the
   * path check. The window passes the workspace plus its allowlist, and leaves
   * it off in the full-access modes.
   */
  sandbox?: Sandbox | null
  /** Called before a `write` / `edit`; resolves false to leave the file alone. */
  approve?: (request: ApprovalRequest) => Promise<boolean>
  /**
   * True when the caller asked for writes to be confirmed but has no
   * `approve` bridge to ask through (a one-shot run). A write is then refused
   * instead of running silently, the same way commands are refused without a
   * confirmation bridge.
   */
  requireWriteApproval?: boolean
  /** Reports a write that proceeds without waiting for a decision. */
  onApproval?: (request: ApprovalRequest) => void
  /**
   * Called before every `shell` command and every destructive file action
   * (`move`, `delete`); resolves false to skip it. Without it those calls are
   * refused, so a caller that cannot ask never runs one.
   */
  confirmActions?: (request: ApprovalRequest) => Promise<boolean>
  /** True when the question starts in plan mode. */
  plan?: boolean
  /** Ask the person a structured question and wait for the chosen option. */
  askUser?: (request: QuestionRequest) => Promise<string>
  /** Submit a plan for approval; resolves false when the person wants changes. */
  reviewPlan?: (request: PlanReviewRequest) => Promise<boolean>
  /** The model replaced the conversation's todo list. */
  onTodo?: (items: TodoUpdate[]) => void
  /**
   * Keep one long-term memory line. The host owns the file, so it decides where
   * it lives and how large it may grow; returning false means the line was not
   * kept, while a void return counts as done (the host reports its own errors).
   */
  remember?: (entry: { call: string; text: string }) => Promise<boolean | void> | boolean | void
  /** Called once per tool call, before its result goes back to the model. */
  onStep?: (step: AgentStep) => void
  /**
   * Called before every round trip, with the round about to start and how many
   * tool calls have finished, so a status line can follow the loop live.
   */
  onProgress?: (progress: AgentProgress) => void
  /** Called with each fragment of the answer, as the model writes it. */
  onDelta?: (text: string) => void
  /** Called with each fragment of the model's thinking, when it streams one. */
  onReasoning?: (text: string) => void
  /** Called when a `shell` command is about to run, before its first output. */
  onToolStart?: (call: string, tool: string, target: string) => void
  /** Called with throttled `shell` output while the command is still running. */
  onToolOutput?: (call: string, tool: string, target: string, text: string) => void
  /**
   * Called once per question with the tool schemas the request carries, so a
   * caller can show how much of the prompt the definitions take. The schema is
   * the exact JSON sent to the provider.
   */
  onTools?: (report: ToolReport) => void
  /** Tools the enabled plugins registered; every call asks for a decision. */
  plugins?: PluginTool[]
  /** Panels the enabled plugins registered, built before the first round. */
  panels?: PluginPanel[]
  /** Tools the enabled MCP servers offer; every call asks for a decision. */
  mcpTools?: McpTool[]
  /** Prompt fragments the enabled plugins inject, added to the system prompt. */
  promptInjections?: Array<{ id: string; text: string }>
  /**
   * Context window of the model, in tokens. Compaction thresholds scale from
   * it; absent means {@link DEFAULT_WINDOW_TOKENS}.
   */
  windowTokens?: number
  /** False turns compaction off, leaving only the per-result cap. */
  compaction?: boolean
}

/** What one request carries besides the conversation. */
export interface ToolReport {
  /** One entry per tool offered to the model, built-ins included. */
  items: ToolSchema[]
  /** Plugin prompt fragments, for the context panel's breakdown. */
  prompts: Array<{ id: string; text: string }>
  /** Panels the plugins built for this request. */
  panels: PanelReport[]
  /** Panels that failed, reported rather than fatal. */
  panelErrors: PluginError[]
}

/** One tool definition as it goes over the wire. */
export interface ToolSchema {
  /** Tool name the model calls. */
  name: string
  /** The whole definition, serialised, for a readable breakdown. */
  schema: string
  /** Plugin folder that registered the tool, when a plugin did. */
  plugin?: string
  /** MCP server that offers the tool, when one does. */
  server?: string
}

/** One entry of the wire-level `messages` array. */
interface WireMessage {
  role: 'system' | 'user' | 'assistant' | 'tool'
  /** A string for every role except the user message that carries files. */
  content: string | Array<Record<string, unknown>>
  tool_calls?: WireToolCall[]
  tool_call_id?: string
  /**
   * Server tools this assistant turn ran, passed back so the provider keeps the
   * pages it read in context for the rounds that follow.
   */
  server_tool_calls?: unknown[]
}

/**
 * Answer one question, running whatever tools the model asks for.
 * @param request - the parsed stdin request.
 * @param options - fetch, environment, working directory and callbacks.
 * @returns the final answer plus the model that produced it.
 */
export async function runAgent(request: AiRequest, options: AgentOptions): Promise<AgentResult> {
  const env = options.env ?? process.env
  const prompt = (request.prompt ?? '').trim()
  if (!prompt) throw new Error('prompt 为空')

  const apiKey = firstNonEmpty(request.apiKey, env.DEEPSEEK_API_KEY)
  if (!apiKey) {
    throw new Error('缺少 API Key：在界面「设置 → AI」中填写，或设置 DEEPSEEK_API_KEY')
  }
  const baseUrl = firstNonEmpty(request.baseUrl, env.DEEPSEEK_BASE_URL, DEFAULT_BASE_URL).replace(/\/+$/, '')
  const model = firstNonEmpty(request.model, env.DEEPSEEK_MODEL, DEFAULT_MODEL)
  const maxRounds = bounded(options.maxRounds ?? request.maxRounds, MAX_ROUNDS, 1, MAX_ROUNDS_LIMIT)
  const maxCalls = bounded(options.maxCalls ?? request.maxCalls, MAX_CALLS, 1, MAX_CALLS_LIMIT)
  const allowed = request.allowedTools ? new Set(request.allowedTools) : null
  const pluginTools = options.plugins ?? []
  const mcpTools = options.mcpTools ?? []
  const builtins = allowed ? TOOLS.filter((tool) => allowed.has(tool.function.name)) : TOOLS
  // Plan guidance and the exit tool only exist while the conversation is in
  // plan mode; asking and todo-writing are always available with tools on.
  const planMode = options.plan === true || request.plan === true
  // The loop's tool executor reads the same flag; an approved plan clears it.
  options.plan = planMode
  // Remembering is offered only when the host has a file to write it into.
  const memory = request.memory === true
  const interactionTools = INTERACTION_TOOLS.filter(
    (tool) =>
      (planMode || tool.function.name !== 'exit_plan_mode') &&
      (memory || tool.function.name !== 'remember')
  )
  // Plugin and MCP tools are not in the window's allow-list (that list names
  // the engine's own tools), so they are added on top and callable by name.
  const pluginNames = new Set(pluginTools.map((tool) => tool.name))
  const mcpNames = new Set(mcpTools.map((tool) => tool.name))
  // The tools this engine runs itself. They stay a separate list from the wire
  // array because the prompt accounting and the window's panel count function
  // schemas only, and the server tools have none.
  const functionTools = [
    ...builtins,
    ...interactionTools,
    ...pluginTools.map((tool) => ({
      type: 'function' as const,
      function: {
        name: tool.name,
        description: tool.description,
        parameters: tool.parameters
      }
    })),
    ...mcpTools.map((tool) => ({
      type: 'function' as const,
      function: {
        name: tool.name,
        description: tool.description,
        parameters: tool.parameters
      }
    }))
  ]
  // `deepseek_web_search` / `deepseek_read_url`: the model may go online and the
  // provider runs the search itself, so these are declared to the wire and never
  // added to the local loop.
  const webSearch = request.webSearch === true
  const wireTools = webSearch
    ? [...serverToolDeclarations(), ...functionTools]
    : functionTools
  /** Whether this request may run the named tool. */
  const permitted = (name: string): boolean =>
    INTERACTION_NAMES.has(name) ||
    allowed === null ||
    allowed.has(name) ||
    pluginNames.has(name) ||
    mcpNames.has(name)
  // Which tool came from where. The window counts a plugin's contribution on its
  // card; the origin never reaches the model, so it rides beside the schema.
  const origins = new Map<string, { plugin?: string; server?: string }>()
  for (const tool of pluginTools) origins.set(tool.name, { plugin: tool.plugin })
  for (const tool of mcpTools) {
    // The `mcp` gateway belongs to no single server, so it keeps no origin.
    if (tool.name === 'mcp') continue
    origins.set(tool.name, { server: tool.server })
  }
  const scope = options.scopeRoot === undefined ? options.root : options.scopeRoot
  // Sibling conversation folders are skipped by the search tools; the request
  // carries the list because only the window knows which folders are chats.
  options.skipDirs = request.skipDirs ?? []

  const messages: WireMessage[] = []
  const system = (request.system ?? '').trim()
  // Prompt fragments from plugins ride along with the system prompt, after the
  // window's own instructions so they can only add to them.
  const injected = (options.promptInjections ?? [])
    .map((prompt) => prompt.text.trim())
    .filter((text) => text !== '')
    .join('\n\n')
  const full = [system, injected, planMode ? PLAN_GUIDE : '']
    .filter((part) => part !== '')
    .join('\n\n')
  if (full) messages.push({ role: 'system', content: full })
  messages.push(...historyMessages(request.history))
  messages.push({ role: 'user', content: userContent(prompt, request.attachments) })

  // Panels are built from what this request is made of, so a plugin can count,
  // chart or summarise it. They are asked before the first round, and a panel
  // that fails is reported rather than fatal.
  const collected = await collectPanels(options.panels ?? [], {
    model,
    cwd: options.root,
    contextWindow: typeof request.contextWindow === 'number' ? request.contextWindow : 0,
    systemChars: full.length,
    injectedChars: injected.length,
    toolChars: functionTools.reduce(
      (total, tool) =>
        total +
        tool.function.name.length +
        tool.function.description.length +
        JSON.stringify(tool.function.parameters).length,
      0
    ),
    turns: messages.map((message) => ({
      role: message.role,
      chars:
        typeof message.content === 'string'
          ? message.content.length
          : JSON.stringify(message.content).length
    }))
  })
  options.onTools?.({
    items: functionTools.map((tool) => {
      const origin = origins.get(tool.function.name)
      return origin === undefined
        ? { name: tool.function.name, schema: JSON.stringify(tool) }
        : { name: tool.function.name, schema: JSON.stringify(tool), ...origin }
    }),
    prompts: options.promptInjections ?? [],
    panels: collected.panels,
    panelErrors: collected.errors
  })

  let calls = 0
  let reason = ''
  // Ids the transcript already used: a gateway that omits them gets the same
  // `call_0` on every round, and a repeated id makes the tool pairing ambiguous
  // for stricter endpoints.
  const seenCalls = new Set<string>()
  // How much tool text one question may carry, scaled from the window: a 1M
  // model can keep far more of what it read than the 60k written for 128k.
  const budgets = toolBudgets(options.windowTokens ?? DEFAULT_WINDOW_TOKENS)
  // Indices of tool messages, oldest first, plus what each one was, so the
  // transcript can shrink without breaking the `tool_call_id` pairing and the
  // model can still tell which result went away.
  const toolResults: Array<{ index: number; label: string }> = []
  let toolChars = 0
  /** Replace the oldest results until the transcript fits its budget. */
  const trimToolContext = (): void => {
    while (toolChars > budgets.context && toolResults.length > 1) {
      const oldest = toolResults[0]
      if (oldest === undefined) break
      const content = messages[oldest.index]?.content
      const note = droppedToolResult(oldest.label)
      // Dropping a result that is already shorter than the note would only
      // make the transcript longer, so a run of tiny results stops here.
      if (typeof content !== 'string' || content.length <= note.length) break
      toolResults.shift()
      toolChars -= content.length
      messages[oldest.index]!.content = note
      toolChars += note.length
    }
  }
  /** Record one tool result and keep the whole set inside the budget. */
  const pushToolResult = (id: string, content: string, label: string): void => {
    messages.push({ role: 'tool', tool_call_id: id, content })
    toolResults.push({ index: messages.length - 1, label })
    toolChars += content.length
    trimToolContext()
  }
  // Images one question may show the model. Every round resends the transcript,
  // so a screenshot loop has to be bounded or it would crowd out the answer.
  let imageCount = 0
  let imageChars = 0
  /** Whether one more image still fits this question. */
  const imageRoom = (image: ToolImage): boolean =>
    imageCount < MAX_IMAGES_PER_QUESTION && imageChars + image.data.length <= MAX_IMAGE_BASE64
  /**
   * Attach an image the model should look at.
   *
   * A tool message only carries text, so the picture rides as a user message
   * right after it: that is the one shape every OpenAI-compatible endpoint
   * accepts for `image_url`.
   */
  const attachImage = (tool: string, image: ToolImage): void => {
    imageCount += 1
    imageChars += image.data.length
    messages.push({
      role: 'user',
      content: [
        { type: 'text', text: `（这是工具 ${tool} 返回的图片。）` },
        { type: 'image_url', image_url: { url: `data:${image.mime};base64,${image.data}` } }
      ]
    })
  }
  // Every round is billed separately, so the answer reports the whole question.
  const usage = { prompt_tokens: 0, completion_tokens: 0, total_tokens: 0 }
  // ...while "how full the window is" comes from the newest round alone.
  let context = 0
  let spent = false
  // How many round trips the loop actually used, for the bill's explanation.
  let taken = 0
  // What each round cost, so the window can show where the bill comes from.
  const roundTokens: Array<[number, number, number]> = []
  // Thinking tokens per round, measured from the text the provider streamed.
  const roundReasoning: number[] = []
  // The thinking itself, so the window can show it under the answer instead of
  // only counting what it cost.
  const reasoningParts: string[] = []
  // Prompt tokens the provider served from its cache, summed over rounds.
  let cacheHits = 0
  // Server tools the provider ran for this question and the citations of the
  // answer, both raw so the report can list the sources.
  const webCalls: unknown[] = []
  const webCitations: unknown[] = []
  /**
   * Show what the provider's server tools did this round.
   *
   * They run on its side, so there is no local tool loop to report them; the
   * rows go out as ordinary steps, which is what the transcript, the actions
   * card and the saved conversation already know how to draw.
   * @param answer - the round that just finished.
   * @param prefix - id prefix, unique per round.
   */
  const reportWeb = (
    answer: Awaited<ReturnType<typeof streamCompletion>>,
    prefix: string
  ): void => {
    for (const step of webSteps(answer.serverCalls, answer.annotations, prefix)) {
      options.onStep?.(step)
    }
    webCalls.push(...answer.serverCalls)
    webCitations.push(...answer.annotations)
  }
  // Where the question's wall clock went. A compaction pass or a person's slow
  // approval is deliberately left out of both, so it reads as 其他 downstream
  // instead of being blamed on the model or the tools.
  let modelMs = 0
  let toolMs = 0
  /**
   * Run one phase and add its wall clock to a timing bucket.
   * @param bucket - which of the two measured phases this is.
   * @param run - the work to time; its result and errors pass through.
   * @returns whatever `run` resolved to.
   */
  const timed = async <T>(bucket: 'model' | 'tool', run: () => Promise<T>): Promise<T> => {
    const startedAt = Date.now()
    try {
      return await run()
    } finally {
      // `finally`, so a failed request still counts as time the person waited.
      const spent = Date.now() - startedAt
      if (bucket === 'model') modelMs += spent
      else toolMs += spent
    }
  }
  // The compaction layers: prune under pressure, summarize the oldest whole
  // turns with the model, and drop that range only if the model call fails.
  const limits = compactLimits(options.windowTokens ?? DEFAULT_WINDOW_TOKENS)
  const compactionTarget = {
    baseUrl,
    apiKey,
    model,
    timeoutMs: request.timeoutMs,
    maxTokens: limits.maxSummaryTokens,
    fetchImpl: options.fetchImpl
  }
  /**
   * Run the layers before one request goes out.
   *
   * The dropped tool results already keep their own budget, so this is what
   * catches a transcript made of long *answers*; it rewrites the local array,
   * which is the same one the next round sends.
   * @param force - compact even below the pressure line, after an overflow.
   * @returns the report when something changed, otherwise undefined.
   */
  const compactNow = async (force: boolean): Promise<CompactReport | undefined> => {
    if (options.compaction === false) return undefined
    const report = await compactTranscript(messages, {
      limits,
      target: compactionTarget,
      force
    })
    if (!report.changed) return undefined
    // Indices moved when the range was replaced, so the pairing bookkeeping is
    // rebuilt from the transcript instead of trusted. The labels come from the
    // assistant turns that requested the calls: after compaction the id is the
    // only link left, and it is enough.
    toolResults.length = 0
    toolChars = 0
    const labels = new Map<string, string>()
    for (const message of messages) {
      for (const call of message.tool_calls ?? []) {
        if (call.id !== '') labels.set(call.id, call.function?.name ?? '')
      }
    }
    for (const [index, message] of messages.entries()) {
      if (message.role !== 'tool' || typeof message.content !== 'string') continue
      const id = typeof message.tool_call_id === 'string' ? message.tool_call_id : ''
      toolResults.push({ index, label: labels.get(id) ?? '' })
      toolChars += message.content.length
    }
    return report
  }
  let compaction: CompactReport | undefined
  /**
   * Ask once, compacting and retrying when the endpoint says the prompt is too
   * long.
   *
   * The token estimate is a heuristic, so an endpoint can still refuse a prompt
   * it counts as too big; compacting more aggressively and sending again is
   * what keeps a long conversation answerable instead of failing outright.
   */
  const askOnce = async (
    build: () => Record<string, unknown>,
    round: number
  ): Promise<{ answer: Awaited<ReturnType<typeof streamCompletion>>; retried: boolean }> => {
    // Server tools run on the provider's side, so each `call_started` is shown
    // as a running row with the id the finished step will use; the counter
    // counts calls only, matching the order the finished calls arrive in.
    let liveCalls = 0
    const onServerEvent = (event: unknown): void => {
      const row = webEventRow(event, `round-${round}`, liveCalls + 1)
      if (row === null) return
      liveCalls += 1
      options.onToolStart?.(row.call, row.tool, row.target)
    }
    try {
      return {
        answer: await streamCompletion({
          url: `${baseUrl}/chat/completions`,
          apiKey,
          label: model,
          body: build(),
          fetchImpl: options.fetchImpl,
          timeoutMs: request.timeoutMs,
          onDelta: options.onDelta,
          onReasoning: options.onReasoning,
          onServerEvent
        }),
        retried: false
      }
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error)
      if (!isContextOverflow(message)) throw error
      const report = await compactNow(true)
      if (report === undefined) throw error
      compaction = report
      return {
        answer: await streamCompletion({
          url: `${baseUrl}/chat/completions`,
          apiKey,
          label: model,
          body: build(),
          fetchImpl: options.fetchImpl,
          timeoutMs: request.timeoutMs,
          onDelta: options.onDelta,
          onReasoning: options.onReasoning,
          // The retried attempt can run server tools too; without this its
          // searches would only appear once they finish, never as running rows.
          onServerEvent
        }),
        retried: true
      }
    }
  }
  /** Whether this question already sent the "you promised but acted on nothing" reminder. */
  let nudged = false
  /** The round that reminder went out in. */
  let nudgedRound = 0
  for (let round = 1; round <= maxRounds; round += 1) {
    taken = round
    // The status strip follows the loop, so it is told before the request goes
    // out rather than after the answer lands.
    options.onProgress?.({ round, calls })
    const underPressure = await compactNow(false)
    if (underPressure !== undefined) compaction = underPressure
    const build = (): Record<string, unknown> => {
      const body: Record<string, unknown> = {
        model,
        messages,
        stream: true,
        tools: wireTools,
        stream_options: { include_usage: true }
      }
      if (typeof request.temperature === 'number') body.temperature = request.temperature
      if (typeof request.maxTokens === 'number') body.max_tokens = request.maxTokens
      applyThinking(body, request.reasoning)
      return body
    }
    const { answer, retried } = await timed('model', () => askOnce(build, round))

    spent = addUsage(usage, answer.usage) || spent
    context = contextSize(answer.usage) || context
    cacheHits += cacheHitSize(answer.usage)
    const counted = roundCount(answer.usage)
    if (counted !== null) roundTokens.push([counted[0], counted[1], answer.toolCalls.length])
    // Thinking is output, so it rides with the round that produced it instead
    // of being folded into the answer or the prompt.
    roundReasoning.push(estimateTokens(answer.reasoning))
    if (answer.reasoning.trim() !== '') reasoningParts.push(answer.reasoning)
    // A search the model ran on the provider's side is shown before the answer
    // it produced, the way a local tool's row is.
    reportWeb(answer, `round-${round}`)

    let renamed = 0
    for (const call of answer.toolCalls) {
      if (call.id === '' || seenCalls.has(call.id)) {
        call.id = `call_${round}_${renamed}`
        renamed += 1
      }
      seenCalls.add(call.id)
    }

    if (answer.toolCalls.length === 0) {
      const text = fullAnswer(answer.content, answer.finishReason)
      if (!text.trim()) throw new Error(emptyAnswerReason(model, answer.finishReason))
      // A small model sometimes narrates the next step and stops without calling
      // anything ("我先打开这个页面看看…"). Ending the answer there looks like the
      // conversation died, and it happened in the field. One reminder costs a
      // round and usually produces the call it described.
      const promised = unfulfilledPromise(text)
      if (!nudged && round < maxRounds && promised) {
        nudged = true
        nudgedRound = round
        messages.push({ role: 'user', content: PROMISE_NUDGE })
        continue
      }
      // Still nothing after the reminder: say so in the answer rather than leave
      // the person wondering why it stopped.
      const stopped = nudged && promised && round === nudgedRound + 1
      return {
        text: stopped ? `${text}\n\n（这一轮没有调用任何工具；如果需要我继续，回复「继续」。）` : text,
        model,
        usage: spent ? usage : undefined,
        context,
        rounds: retried ? round + 1 : round,
        roundTokens,
        roundReasoning,
        reasoning: reasoningParts.join('\n\n'),
        cacheHits,
        webSearch: webReport(webCalls, webCitations),
        serverCalls: webCalls,
        annotations: webCitations,
        timing: { modelMs, toolMs },
        compaction
      }
    }

    messages.push({
      role: 'assistant',
      content: answer.content,
      tool_calls: answer.toolCalls,
      // The search this round ran is part of the turn too; the next round's
      // request keeps those pages in context only when they ride along.
      ...(answer.serverCalls.length > 0 ? { server_tool_calls: answer.serverCalls } : {})
    })
    const planned = answer.toolCalls.map((call) => {
      const name = call.function?.name ?? ''
      const rawArgs: unknown = call.function?.arguments
      const argsText =
        typeof rawArgs === 'string' ? rawArgs : rawArgs === undefined ? '' : JSON.stringify(rawArgs)
      const parsed = parseArguments(rawArgs)
      return { call, name, argsText, parsed, args: parsed.args }
    })
    // A round made only of reads can start every call together; one write or
    // command anywhere in the round keeps the whole batch serial, because the
    // model may have written a file that a later call reads back.
    const remaining = Math.max(0, maxCalls - calls)
    const onlyReads = planned.every((entry) => READ_ONLY_TOOLS.has(entry.name))
    // A model that asks the same read twice in one round should not pay for the
    // same text twice: the second call gets the first one's promise.
    const roundReads = new Map<string, Promise<ToolResult>>()
    const prepared = onlyReads
      ? await timed('tool', () =>
          Promise.all(
            planned.map((entry, index) => {
              if (index >= remaining || entry.parsed.problem !== '') return null
              if (!permitted(entry.name)) return null
              const key = `${entry.name}\u001f${entry.argsText}`
              const seen = roundReads.get(key)
              if (seen !== undefined) return seen
              const work = runToolWithTimeout(options, scope, entry.call.id, entry.name, entry.args)
              roundReads.set(key, work)
              return work
            })
          )
        )
      : planned.map(() => null)
    for (const [index, entry] of planned.entries()) {
      const { call, name, argsText, parsed, args } = entry
      calls += 1
      if (calls > maxCalls) {
        // Every tool call still needs an answer, or the next round is malformed.
        const skipped = `已达到一次提问的工具调用上限（${maxCalls} 次），这一步没有执行。`
        options.onStep?.({
          tool: name,
          target: describe(name, args),
          ok: false,
          detail: '已达到调用上限',
          input: clip(argsText, MAX_STEP_INPUT),
          output: skipped,
          call: call.id
        })
        pushToolResult(call.id, skipped, `${name} ${describe(name, args)}`.trim())
        reason = `工具调用达到 ${maxCalls} 次上限`
        continue
      }
      // The model only sees the allowed tools, but a hallucinated name still
      // must not reach a tool that this request turned off. Broken arguments
      // are reported as-is instead of running the tool with an empty object,
      // which would only produce a misleading "缺少参数" error.
      const ready = prepared[index]
      let result: ToolResult
      if (ready) {
        result = ready
      } else if (parsed.problem !== '') {
        result = { ok: false, text: parsed.problem }
      } else if (!permitted(name)) {
        result = { ok: false, text: `工具未启用：${name}` }
      } else {
        // A broken approval bridge must fail this call, not the answer: the
        // write stays undone and the model still gets a usable turn.
        result = await timed('tool', () => runToolWithTimeout(options, scope, call.id, name, args))
      }
      // A tool that saw something shows it; anything else answers in text only.
      const shows = result.ok && result.image !== undefined && imageRoom(result.image)
      const dropped =
        result.ok && result.image !== undefined && !shows
          ? `\n（这张图没有附上：一次提问最多 ${MAX_IMAGES_PER_QUESTION} 张、合计 ${Math.round(MAX_IMAGE_BASE64 / (1024 * 1024))} MB。）`
          : ''
      const label = `${name} ${describe(name, args)}`.trim()
      pushToolResult(call.id, fitToolResult(result.text + dropped, budgets.message), label)
      if (shows && result.image !== undefined) attachImage(name, result.image)
      // The window keeps the file so the next question can show it again; the
      // bytes stay in this request, where they were paid for.
      const kept =
        result.image?.path !== undefined && result.image.path !== ''
          ? { image: { mime: result.image.mime, path: result.image.path } }
          : {}
      // Files are what the reader gets to open, so they ride on the step even when
      // the tool also returned text the model has already read.
      const shown =
        result.ok && result.files !== undefined && result.files.length > 0
          ? { files: result.files }
          : {}
      options.onStep?.({
        tool: name,
        target: describe(name, args),
        ok: result.ok,
        detail: summarise(name, result.ok, result.text),
        input: clip(argsText, MAX_STEP_INPUT),
        output: clip(result.text, MAX_STEP_OUTPUT),
        call: call.id,
        ...kept,
        ...shown
      })
    }
    // The results are in, so the arguments no longer have to ride along in
    // full: the assistant turn is replayed on every later round of this
    // question, and compaction cannot drop a turn of the newest question.
    for (const entry of planned) shrinkArguments(entry.call, entry.args)
    if (reason !== '') break
  }
  if (reason === '') reason = `模型往返达到 ${maxRounds} 轮上限`

  // Spend the last request on a summary instead of throwing the work away.
  // No tools are declared, so this round cannot loop again. The transcript gets
  // one compaction pass first, so the forced summary fits the window too.
  const lastPass = await compactNow(false)
  if (lastPass !== undefined) compaction = lastPass
  const body: Record<string, unknown> = {
    model,
    messages,
    stream: true,
    stream_options: { include_usage: true }
  }
  if (typeof request.temperature === 'number') body.temperature = request.temperature
  if (typeof request.maxTokens === 'number') body.max_tokens = request.maxTokens
  applyThinking(body, request.reasoning)
  try {
    const answer = await timed('model', () =>
      streamCompletion({
        url: `${baseUrl}/chat/completions`,
        apiKey,
        label: model,
        body,
        fetchImpl: options.fetchImpl,
        timeoutMs: request.timeoutMs,
        onDelta: options.onDelta,
        onReasoning: options.onReasoning
      })
    )
    spent = addUsage(usage, answer.usage) || spent
    context = contextSize(answer.usage) || context
    cacheHits += cacheHitSize(answer.usage)
    const counted = roundCount(answer.usage)
    if (counted !== null) roundTokens.push([counted[0], counted[1], 0])
    roundReasoning.push(estimateTokens(answer.reasoning))
    if (answer.reasoning.trim() !== '') reasoningParts.push(answer.reasoning)
    const text = fullAnswer(answer.content, answer.finishReason)
    const note = `（${reason}，先给出基于已有信息的结果；需要继续时再发一条消息。）`
    return {
      text: text ? `${text}\n\n${note}` : note,
      model,
      usage: spent ? usage : undefined,
      context,
      rounds: taken + 1,
      roundTokens,
      roundReasoning,
      reasoning: reasoningParts.join('\n\n'),
      cacheHits,
      webSearch: webReport(webCalls, webCitations),
      serverCalls: webCalls,
      annotations: webCitations,
      timing: { modelMs, toolMs },
      compaction
    }
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    return {
      text: `（${reason}，生成总结时出错：${message}）`,
      model,
      usage: spent ? usage : undefined,
      context,
      rounds: taken + 1,
      roundTokens,
      roundReasoning,
      reasoning: reasoningParts.join('\n\n'),
      cacheHits,
      webSearch: webReport(webCalls, webCitations),
      serverCalls: webCalls,
      annotations: webCitations,
      timing: { modelMs, toolMs },
      compaction
    }
  }
}

/**
 * Check a command or snippet against the sandbox before anything is asked.
 * @param options - root, scope and the sandbox to enforce.
 * @param scope - writable root, or null for full access.
 * @param source - the command line or the code about to run.
 * @param cwd - the directory as the model wrote it.
 * @returns why it may not run, or an empty string when it may.
 */
async function sandboxProblem(
  options: AgentOptions,
  scope: string | null,
  source: string,
  cwd: string
): Promise<string> {
  const sandbox = options.sandbox ?? null
  if (sandbox === null) return ''
  let resolved: string
  try {
    resolved = await resolveCommandDir(options.root, scope, sandbox, cwd)
  } catch (error) {
    return error instanceof Error ? error.message : String(error)
  }
  const outside = await outsidePaths(source, resolved, sandbox)
  return outside.length === 0 ? '' : sandboxRefusal(outside, sandbox)
}

/**
 * Run one allowed tool, asking first when it would write a file or run a command.
 * @param options - root, read-only roots and the approval callback.
 * @param id - provider id of the call.
 * @param name - tool name.
 * @param args - parsed arguments.
 * @returns whether it worked, and the text handed back to the model.
 */
async function runAllowedTool(
  options: AgentOptions,
  scope: string | null,
  id: string,
  name: string,
  args: Record<string, unknown>
): Promise<ToolResult> {
  if (name === 'ask_user_question') {
    const question = typeof args.question === 'string' ? args.question.trim() : ''
    if (question === '') return { ok: false, text: '缺少参数 question' }
    const offered = Array.isArray(args.options) ? args.options : []
    const choices = offered
      .map((option) => (typeof option === 'string' ? option.trim() : ''))
      .filter((option) => option !== '')
    if (choices.length < 2 || choices.length > 6) {
      return { ok: false, text: 'options 需要 2–6 个非空选项' }
    }
    if (!options.askUser) return { ok: false, text: '没有提问界面，这次提问没有发出。' }
    const answer = await options.askUser({
      call: id,
      question,
      options: choices,
      allowCustom: args.allowCustom === true
    })
    if (answer.trim() === '') return { ok: false, text: '用户没有给出回答。' }
    return { ok: true, text: `用户选择：${answer.trim()}` }
  }
  if (name === 'todo_write') {
    const listed = Array.isArray(args.items) ? args.items : []
    if (listed.length > 30) return { ok: false, text: '待办最多 30 项' }
    const items: TodoUpdate[] = []
    for (const item of listed) {
      if (!item || typeof item !== 'object') continue
      const entry = item as Record<string, unknown>
      const text = typeof entry.text === 'string' ? entry.text.trim() : ''
      if (text === '') continue
      const status =
        entry.status === 'in_progress' || entry.status === 'completed'
          ? entry.status
          : 'pending'
      items.push({ text, status })
    }
    const done = items.filter((item) => item.status === 'completed').length
    options.onTodo?.(items)
    return {
      ok: true,
      text: items.length === 0 ? '待办列表已清空。' : `已更新待办：${done}/${items.length} 完成。`
    }
  }
  if (name === 'remember') {
    const text = typeof args.text === 'string' ? args.text.trim() : ''
    if (text === '') return { ok: false, text: '缺少参数 text' }
    if (text.length > MAX_MEMORY_ENTRY) {
      return {
        ok: false,
        text: `要记的内容太长（${text.length} 字符，上限 ${MAX_MEMORY_ENTRY}）；请压缩成一句可复用的结论。`
      }
    }
    if (!options.remember) {
      return { ok: false, text: '没有长期记忆的写入通道，这次没有记下。' }
    }
    const kept = await options.remember({ call: id, text })
    return kept === false
      ? { ok: false, text: '长期记忆没有写入（文件不可写或已到上限），这条没有记下。' }
      : { ok: true, text: `已记入长期记忆：${text}` }
  }
  if (name === 'exit_plan_mode') {
    if (options.plan !== true) return { ok: false, text: '当前不在计划模式。' }
    const plan = typeof args.plan === 'string' ? args.plan.trim() : ''
    if (!plan.startsWith('#')) {
      return { ok: false, text: '计划需要是一份以 # 标题开头的完整 markdown。' }
    }
    if (!options.reviewPlan) return { ok: false, text: '没有计划审批界面，计划没有提交。' }
    const approved = await options.reviewPlan({ call: id, plan })
    if (!approved) {
      // Keep plan mode on: the model revises and submits again.
      return { ok: false, text: '用户没有批准计划，请继续完善后再次提交。' }
    }
    // The current run may now execute; the window also clears the conversation's
    // plan flag, so the next question starts in normal mode.
    options.plan = false
    return { ok: true, text: '用户已批准计划，可以开始执行。' }
  }
  if (name === 'mcp') {
    const gateway = options.mcpTools?.find((tool) => tool.name === 'mcp')
    if (!gateway) return { ok: false, text: 'MCP 工具没有加载。' }
    const action = typeof args.action === 'string' ? args.action.trim().toLowerCase() : ''
    // Listing and describing change nothing outside this question; linking
    // starts a process or opens a connection, calling runs its tool, and
    // disconnecting stops a server that is already running.
    if (action === 'link' || action === 'call' || action === 'disconnect') {
      if (!options.confirmActions) {
        return { ok: false, text: '没有确认界面，MCP 操作没有执行。' }
      }
      const target =
        action === 'call'
          ? `${textArg(args.server)}:${textArg(args.tool)}`
          : textArg(args.server) ||
            textArg(args.id) ||
            textArg(args.name) ||
            textArg(args.command) ||
            textArg(args.url) ||
            '新服务器'
      const question: ApprovalRequest = {
        call: id,
        tool: 'mcp',
        target: `${action} ${target}`,
        path: '',
        diff: `MCP ${action}：\n${clipArgs(args)}`,
        undoable: false,
        command: '',
        cwd: ''
      }
      if (!(await options.confirmActions(question))) {
        return { ok: false, text: '用户拒绝了这次 MCP 操作，没有执行。' }
      }
    }
    return await gateway.run(args)
  }
  const mcp = options.mcpTools?.find((tool) => tool.name === name)
  if (mcp) {
    // An MCP server is a separate process with its own reach, so its tools ask
    // every time, like plugin tools do.
    if (!options.confirmActions) {
      return { ok: false, text: '没有确认界面，MCP 工具没有执行。' }
    }
    const question: ApprovalRequest = {
      call: id,
      tool: name,
      target: `${mcp.server}:${mcp.original}`,
      path: '',
      diff: `MCP 服务器「${mcp.server}」的工具：${mcp.original}\n${clipArgs(args)}`,
      undoable: false,
      command: '',
      cwd: ''
    }
    if (!(await options.confirmActions(question))) {
      return { ok: false, text: '用户拒绝了这次 MCP 调用，没有执行。' }
    }
    return await mcp.run(args)
  }
  const plugin = options.plugins?.find((tool) => tool.name === name)
  if (plugin) {
    // Plugin code runs in this process, so its tools ask every single time:
    // nothing tells us what it will do with the arguments it was handed.
    if (!options.confirmActions) {
      return { ok: false, text: '没有确认界面，插件工具没有执行。' }
    }
    const question: ApprovalRequest = {
      call: id,
      tool: name,
      target: describe(name, args),
      // A plugin tool touches no known file, so there is nothing to undo; the
      // empty path is what tells the window this card is not a write.
      path: '',
      diff: `插件 ${plugin.plugin} 的工具：${name}\n${clipArgs(args)}`,
      undoable: false,
      command: '',
      cwd: ''
    }
    if (!(await options.confirmActions(question))) {
      return { ok: false, text: '用户拒绝了这次插件调用，没有执行。' }
    }
    return await runPluginTool(plugin, args, options.root)
  }
  if (name === 'shell') {
    const command = typeof args.command === 'string' ? args.command.trim() : ''
    if (!command) return { ok: false, text: '缺少参数 command' }
    if (command.length > MAX_SHELL_COMMAND) {
      return {
        ok: false,
        text: `命令太长（${command.length} 字符，上限 ${MAX_SHELL_COMMAND}）`
      }
    }
    const cwd = typeof args.cwd === 'string' && args.cwd.trim() ? args.cwd : '.'
    // Run exactly what the card showed.
    args.command = command
    args.cwd = cwd
    // The sandbox is checked before the card: asking someone to approve a
    // command that will then be refused only wastes their attention.
    const problem = await sandboxProblem(options, scope, command, cwd)
    if (problem !== '') return { ok: false, text: problem }
    if (!options.confirmActions) {
      return { ok: false, text: '没有确认界面，命令没有执行。' }
    }
    const shown = cwd === '.' ? `$ ${command}` : `# 工作目录：${cwd}\n$ ${command}`
    const question: ApprovalRequest = {
      call: id,
      tool: name,
      target: describe(name, args),
      // A command has no file to undo; the empty path also tells the window
      // that this card is a command rather than a write.
      path: '',
      diff: args.background === true ? `# 后台运行（问答结束后继续）\n${shown}` : shown,
      undoable: false,
      command,
      cwd
    }
    if (!(await options.confirmActions(question))) {
      return { ok: false, text: '用户拒绝了这条命令，没有执行。' }
    }
  }
  if (name === 'python' || name === 'node') {
    const code = typeof args.code === 'string' ? args.code.trim() : ''
    if (!code) return { ok: false, text: '缺少参数 code' }
    if (code.length > MAX_CODE_CHARS) {
      return {
        ok: false,
        text: `代码太长（${code.length} 字符，上限 ${MAX_CODE_CHARS}）`
      }
    }
    const cwd = typeof args.cwd === 'string' && args.cwd.trim() ? args.cwd : '.'
    // Run exactly what the card showed.
    args.code = code
    args.cwd = cwd
    const problem = await sandboxProblem(options, scope, code, cwd)
    if (problem !== '') return { ok: false, text: problem }
    if (!options.confirmActions) {
      return { ok: false, text: '没有确认界面，代码没有执行。' }
    }
    const flag = name === 'python' ? '-c' : '-e'
    const question: ApprovalRequest = {
      call: id,
      tool: name,
      target: describe(name, args),
      // A snippet has no file to undo; the empty path also tells the window
      // that this card is code rather than a write.
      path: '',
      diff: `${cwd === '.' ? '' : `# 工作目录：${cwd}\n`}$ ${name} ${flag}\n${clipCode(code)}`,
      undoable: false,
      command: '',
      cwd
    }
    if (!(await options.confirmActions(question))) {
      return { ok: false, text: '用户拒绝了这段代码，没有执行。' }
    }
  }
  if (name === 'http') {
    if (!options.confirmActions) {
      return { ok: false, text: '没有确认界面，请求没有发出。' }
    }
    const question: ApprovalRequest = {
      call: id,
      tool: name,
      target: describe(name, args),
      // A request touches no file and has nothing to undo; the empty path is
      // what tells the window this card is not a write.
      path: '',
      diff: httpSummary(args),
      undoable: false,
      command: '',
      cwd: '',
      // A credential has to be confirmed even where commands run unattended.
      secret: hasCredential(args)
    }
    if (!(await options.confirmActions(question))) {
      return { ok: false, text: '用户拒绝了这次请求，没有发出。' }
    }
  }
  if (name === 'browser') {
    const action = typeof args.action === 'string' ? args.action.trim().toLowerCase() : ''
    // Reading the page changes nothing outside this question, so snapshot / text /
    // console / network / screenshot / wait_for / close run without a card, like
    // `read_image`. Navigation and anything that touches the page gets one.
    if (browserNeedsCard(action)) {
      if (!options.confirmActions) {
        return { ok: false, text: '没有确认界面，浏览器操作没有执行。' }
      }
      const summary = browserSummary(args)
      const question: ApprovalRequest = {
        call: id,
        tool: name,
        target: summary,
        // A page action has no file to undo; the empty path is what tells the
        // window this card is not a write.
        path: '',
        diff: summary,
        undoable: false,
        command: '',
        cwd: ''
      }
      if (!(await options.confirmActions(question))) {
        return { ok: false, text: '用户拒绝了这次浏览器操作，没有执行。' }
      }
    }
  }
  if (name === 'task') {
    const action = typeof args.action === 'string' ? args.action.trim().toLowerCase() : ''
    const taskId = typeof args.id === 'string' ? args.id.trim() : ''
    if (action === 'stop' || action === 'stopall') {
      if (!options.confirmActions) {
        return { ok: false, text: '没有确认界面，后台任务没有结束。' }
      }
      const question: ApprovalRequest = {
        call: id,
        tool: name,
        target: action === 'stopall' ? '全部后台任务' : taskId,
        path: '',
        diff:
          action === 'stopall'
            ? '结束后台任务：全部'
            : `结束后台任务：${taskId === '' ? '（缺少 id）' : taskId}`,
        undoable: false,
        command: '',
        cwd: ''
      }
      if (!(await options.confirmActions(question))) {
        return { ok: false, text: '用户拒绝了这次操作，后台任务没有结束。' }
      }
    }
    if (action === 'input') {
      const text = typeof args.text === 'string' ? args.text : ''
      if (taskId === '') return { ok: false, text: '缺少参数 id（例如 task-1）' }
      if (text.trim() === '') return { ok: false, text: '缺少参数 text（要写进 stdin 的内容）' }
      if (!options.confirmActions) {
        return { ok: false, text: '没有确认界面，输入没有写入。' }
      }
      const question: ApprovalRequest = {
        call: id,
        tool: name,
        target: `${taskId} ← 输入`,
        path: '',
        diff: `向 ${taskId} 的 stdin 写入：\n${clipCode(text)}`,
        undoable: false,
        command: '',
        cwd: ''
      }
      if (!(await options.confirmActions(question))) {
        return { ok: false, text: '用户拒绝了这次输入，没有写入。' }
      }
    }
    if (action === 'close') {
      if (taskId === '') return { ok: false, text: '缺少参数 id（例如 task-1）' }
      if (!options.confirmActions) {
        return { ok: false, text: '没有确认界面，stdin 没有关闭。' }
      }
      const question: ApprovalRequest = {
        call: id,
        tool: name,
        target: `${taskId} ← 关闭 stdin`,
        path: '',
        diff: `通知 ${taskId} 关闭 stdin：它读到文件末尾后会继续（很多 CLI 等在这一点）。`,
        undoable: false,
        command: '',
        cwd: ''
      }
      if (!(await options.confirmActions(question))) {
        return { ok: false, text: '用户拒绝了这次操作，stdin 没有关闭。' }
      }
    }
    if (action === 'wait') {
      if (taskId === '') return { ok: false, text: '缺少参数 id（例如 task-1）' }
      // Waiting only polls the record; it changes nothing, so no card.
    }
    if (
      action !== 'list' &&
      action !== 'output' &&
      action !== 'input' &&
      action !== 'close' &&
      action !== 'wait' &&
      action !== 'stop' &&
      action !== 'stopall'
    ) {
      return {
        ok: false,
        text: 'action 必须是 list、output、input、close、wait、stop 或 stopAll'
      }
    }
  }
  if (name === 'move' || name === 'delete' || name === 'copy' || name === 'mkdir') {
    const from = typeof args.from === 'string' ? args.from : ''
    const to = typeof args.to === 'string' ? args.to : ''
    const target = typeof args.path === 'string' ? args.path : ''
    if ((name === 'move' || name === 'copy') && (!from || !to)) {
      return { ok: false, text: '缺少参数 from / to' }
    }
    if ((name === 'delete' || name === 'mkdir') && !target) {
      return { ok: false, text: '缺少参数 path' }
    }
    if (!options.confirmActions) {
      return { ok: false, text: '没有确认界面，这次改动没有执行。' }
    }
    const overwrite = name === 'copy' && args.overwrite === true
    const common = { call: id, tool: name, undoable: false }
    const question: ApprovalRequest =
      name === 'move'
        ? { ...common, target: `${from} → ${to}`, path: from, diff: `移动：${from} → ${to}` }
        : name === 'copy'
          ? {
              ...common,
              target: `${from} → ${to}`,
              // The written side is the target, which is also what the window's
              // automatic review has to judge; the source is only read.
              path: to,
              diff: `复制：${from} → ${to}${overwrite ? '（覆盖已存在的目标）' : ''}`
            }
          : name === 'mkdir'
            ? { ...common, target, path: target, diff: `新建目录：${target}` }
            : {
                ...common,
                target,
                path: target,
                diff:
                  args.recursive === true
                    ? `删除整个文件夹及其内容：${target}`
                    : `删除文件：${target}`
              }
    if (!(await options.confirmActions(question))) {
      return { ok: false, text: '用户拒绝了这次改动，文件没有改动。' }
    }
  }
  if (name === 'git_write') {
    const action = typeof args.action === 'string' ? args.action.trim().toLowerCase() : ''
    if (action !== 'commit' && action !== 'branch' && action !== 'stash') {
      return { ok: false, text: 'action 只能是 commit、branch 或 stash' }
    }
    const message = typeof args.message === 'string' ? args.message.trim() : ''
    const branch = typeof args.name === 'string' ? args.name.trim() : ''
    const stashAction =
      typeof args.stashAction === 'string' && args.stashAction.trim() !== ''
        ? args.stashAction.trim().toLowerCase()
        : 'push'
    if (action === 'commit' && message === '') {
      return { ok: false, text: '缺少参数 message（提交说明）' }
    }
    if (action === 'branch' && branch === '') {
      return { ok: false, text: '缺少参数 name（分支名）' }
    }
    if (!options.confirmActions) {
      return { ok: false, text: '没有确认界面，仓库没有改动。' }
    }
    const paths = Array.isArray(args.paths)
      ? args.paths.filter((entry): entry is string => typeof entry === 'string')
      : []
    const lines = ['$ git_write']
    if (action === 'commit') {
      lines.push(`提交说明：${message}`)
      lines.push(
        args.all === true
          ? '先 git add -A'
          : paths.length > 0
            ? `先 git add -- ${paths.join(' ')}`
            : '只提交已暂存的改动'
      )
    } else if (action === 'branch') {
      lines.push(`新建分支：${branch}${args.checkout === true ? '（并切过去）' : ''}`)
    } else {
      lines.push(
        `stash ${stashAction}${args.includeUntracked === true ? '（含未跟踪文件）' : ''}`
      )
    }
    const question: ApprovalRequest = {
      call: id,
      tool: name,
      target: describe(name, args),
      path: '',
      diff: lines.join('\n'),
      undoable: false,
      command: '',
      cwd: ''
    }
    if (!(await options.confirmActions(question))) {
      return { ok: false, text: '用户拒绝了这次仓库操作，仓库没有改动。' }
    }
  }
  if (name === 'wait_for') {
    const target = typeof args.target === 'string' ? args.target.trim().toLowerCase() : ''
    if (target !== 'port' && target !== 'url' && target !== 'output') {
      // Waiting for a task to end belongs to `task action=wait`; say so instead
      // of letting the model retry the same question under another name.
      return {
        ok: false,
        text: 'target 只能是 port、url 或 output；等后台任务结束请用 task 的 action=wait'
      }
    }
    if (target === 'output' && typeof args.id !== 'string') {
      return { ok: false, text: 'target=output 需要 id（例如 task-1）' }
    }
    if (target === 'output' && typeof args.pattern !== 'string') {
      return { ok: false, text: 'target=output 需要 pattern（要等的文本）' }
    }
    const asked = typeof args.timeoutMs === 'number' ? args.timeoutMs : 0
    const timeout = asked > 0 ? asked : 30_000
    if (!options.confirmActions) {
      return { ok: false, text: '没有确认界面，这次等待没有开始。' }
    }
    const question: ApprovalRequest = {
      call: id,
      tool: name,
      target: describe(name, args),
      path: '',
      diff: `等待：${describe(name, args)}\n最多 ${Math.round(timeout / 1000)} 秒，只轮询不改动任何东西。`,
      undoable: false,
      command: '',
      cwd: ''
    }
    if (!(await options.confirmActions(question))) {
      return { ok: false, text: '用户拒绝了这次等待，没有开始。' }
    }
  }
  if (name === 'run_tests' || name === 'diagnostics') {
    // Both write build output (cargo's target, npm's cache). 「仅可查看」 promises
    // the workspace stays untouched, so the call is refused before the card
    // rather than approved and then failing on a permission error.
    if (process.env.DSH_SANDBOX_READONLY === '1') {
      return {
        ok: false,
        text: '仅可查看模式不跑测试 / 编译检查：它们会写构建产物。切到「工作区内修改」再试，或改用只读命令。'
      }
    }
  }
  if (name === 'run_tests') {
    const command = typeof args.command === 'string' ? args.command.trim() : ''
    if (!options.confirmActions) {
      return { ok: false, text: '没有确认界面，测试没有运行。' }
    }
    const question: ApprovalRequest = {
      call: id,
      tool: name,
      target: describe(name, args),
      path: '',
      diff: `运行测试：\n${command === '' ? '（自动识别：cargo test / npm test / pytest）' : `$ ${command}`}\n失败用例会被解析成清单，完整输出附在后面。`,
      undoable: false,
      command,
      cwd: typeof args.cwd === 'string' ? args.cwd : ''
    }
    if (!(await options.confirmActions(question))) {
      return { ok: false, text: '用户拒绝了这次测试，没有运行。' }
    }
  }
  if (name === 'diagnostics') {
    const command = typeof args.command === 'string' ? args.command.trim() : ''
    if (!options.confirmActions) {
      return { ok: false, text: '没有确认界面，编译检查没有运行。' }
    }
    const question: ApprovalRequest = {
      call: id,
      tool: name,
      target: describe(name, args),
      path: '',
      diff: `运行编译检查：\n${command === '' ? '（自动识别：cargo check --message-format=json / tsc --noEmit）' : `$ ${command}`}\n问题会按「文件:行:列 代码」解析成清单，完整输出附在后面。`,
      undoable: false,
      command,
      cwd: typeof args.cwd === 'string' ? args.cwd : ''
    }
    if (!(await options.confirmActions(question))) {
      return { ok: false, text: '用户拒绝了这次编译检查，没有运行。' }
    }
  }
  // A `replace` that only previews writes nothing, so it needs no card. The
  // preview it returns is the same plan the card would have shown.
  if (
    (name === 'write' || name === 'edit' || name === 'replace') &&
    !(name === 'replace' && args.dryRun === true)
  ) {
    // Even without a decision the window gets the diff and the undo pair, so
    // direct writes stay visible and reversible. A batch replace has no undo
    // pair: one card cannot cover several files, so it reports `undoable: false`.
    const preview = await previewToolScoped(options.root, scope, { id, name, args }, options.skipDirs ?? [])
    if (preview) {
      const question: ApprovalRequest = {
        call: id,
        tool: name,
        target: describe(name, args),
        path: typeof args.path === 'string' ? args.path : '',
        diff: preview.diff,
        before: preview.before,
        after: preview.after,
        existed: preview.existed,
        undoable: preview.undoable
      }
      if (options.approve) {
        const allowed = await options.approve(question)
        if (!allowed) {
          return { ok: false, text: '用户拒绝了这次写入，文件没有改动。' }
        }
      } else if (options.requireWriteApproval) {
        // The caller wanted a decision but gave no way to ask for one; running
        // the write anyway would be a confirmation bypass.
        return { ok: false, text: '没有确认界面，这次写入没有执行。' }
      } else {
        options.onApproval?.(question)
      }
    }
  }
  if (
    name === 'shell' ||
    name === 'python' ||
    name === 'node' ||
    name === 'http' ||
    name === 'run_tests' ||
    name === 'diagnostics'
  ) {
    options.onToolStart?.(id, name, describe(name, args))
  }
  return await runToolScoped(
    options.root,
    scope,
    { id, name, args },
    options.readOnlyRoots,
    options.onToolOutput
      ? (text) => options.onToolOutput?.(id, name, describe(name, args), text)
      : undefined,
    options.sandbox ?? null,
    options.skipDirs ?? []
  )
}

/**
 * Run one tool with a hard ceiling, reporting a timeout as a failed call.
 *
 * `shell`, `python` and `node` are exempt: they manage their own timer and
 * have to kill the process tree before the loop can move on.
 * @param options - root, read-only roots and the approval callback.
 * @param scope - writable root, or null for full access.
 * @param id - provider id of the call.
 * @param name - tool name.
 * @param args - parsed arguments.
 * @returns whether it worked, and the text handed back to the model.
 */
async function runToolWithTimeout(
  options: AgentOptions,
  scope: string | null,
  id: string,
  name: string,
  args: Record<string, unknown>
): Promise<ToolResult> {
  const work = runAllowedTool(options, scope, id, name, args).catch((error: unknown) => {
    const message = error instanceof Error ? error.message : String(error)
    return { ok: false, text: `工具没有执行：${message}` }
  })
  // A tool whose effect has to be observed is never raced: giving up while a
  // write or a delete is still in flight would report a failure that did not
  // happen, and the window's undo record would disagree with the disk.
  if (
    name === 'shell' ||
    name === 'python' ||
    name === 'node' ||
    name === 'git_write' ||
    name === 'wait_for' ||
    name === 'run_tests' ||
    name === 'diagnostics' ||
    // A browser call starts a process, waits for a page, or clicks something the
    // model will ask about next: abandoning it mid-flight would leave the page in
    // a state the transcript does not describe.
    name === 'browser' ||
    MUTATING_TOOLS.has(name) ||
    INTERACTION_NAMES.has(name)
  ) {
    return await work
  }
  let timer: ReturnType<typeof setTimeout> | undefined
  const limit = new Promise<ToolResult>((resolve) => {
    timer = setTimeout(() => {
      resolve({
        ok: false,
        text: `${name} 超时（${TOOL_TIMEOUT_MS} ms），这一步没有等到结果。`
      })
    }, TOOL_TIMEOUT_MS)
  })
  try {
    return await Promise.race([work, limit])
  } finally {
    if (timer !== undefined) clearTimeout(timer)
  }
}

/**
 * One round's counts, or `null` when the provider sent none.
 * @param usage - what the provider reported for one round.
 * @returns `[prompt, completion]`.
 */
export function roundCount(usage: unknown): [number, number] | null {  if (!usage || typeof usage !== 'object') return null
  const counts = usage as Record<string, unknown>
  const number = (...keys: string[]): number => {
    for (const key of keys) {
      const value = counts[key]
      if (typeof value === 'number' && Number.isFinite(value)) return value
    }
    return 0
  }
  const prompt = number('prompt_tokens', 'input_tokens')
  const reported = number('completion_tokens', 'output_tokens')
  const sum = number('total_tokens')
  const completion = reported > 0 ? reported : Math.max(0, sum - prompt)
  if (prompt === 0 && completion === 0) return null
  return [prompt, completion]
}

/**
 * Add one round's token counts into a running total.
 * @param total - the running total, updated in place.
 * @param usage - what the provider reported for one round, if anything.
 * @returns true when the round reported a count.
 */
function addUsage(
  total: { prompt_tokens: number; completion_tokens: number; total_tokens: number },
  usage: unknown
): boolean {
  if (!usage || typeof usage !== 'object') return false
  const counts = usage as Record<string, unknown>
  const number = (...keys: string[]): number => {
    for (const key of keys) {
      const value = counts[key]
      if (typeof value === 'number' && Number.isFinite(value)) return value
    }
    return 0
  }
  const prompt = number('prompt_tokens', 'input_tokens')
  const completion = number('completion_tokens', 'output_tokens')
  const sum = number('total_tokens')
  if (prompt === 0 && completion === 0 && sum === 0) return false
  total.prompt_tokens += prompt
  total.completion_tokens += completion
  total.total_tokens += sum > 0 ? sum : prompt + completion
  return true
}

/**
 * Read the arguments of a tool call.
 * Providers send them as a JSON string; a broken string is reported back to the
 * model so it can retry instead of running the tool with no arguments.
 * @param raw - the argument text.
 * @returns the parsed object, or a problem to hand back to the model.
 */
function parseArguments(raw: unknown): {
  args: Record<string, unknown>
  problem: string
} {
  if (raw === undefined || raw === null) return { args: {}, problem: '' }
  // A gateway may hand back the arguments already parsed; use them as they are.
  if (typeof raw === 'object') {
    if (Array.isArray(raw)) {
      return { args: {}, problem: '工具参数必须是 JSON 对象，请重新调用。' }
    }
    return { args: raw as Record<string, unknown>, problem: '' }
  }
  if (typeof raw !== 'string') {
    return { args: {}, problem: '工具参数不是合法的 JSON，请检查后重新调用。' }
  }
  if (raw.trim() === '') return { args: {}, problem: '' }
  let parsed: unknown
  try {
    parsed = JSON.parse(raw)
  } catch {
    return { args: {}, problem: '工具参数不是合法的 JSON，请检查后重新调用。' }
  }
  if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) {
    return { args: {}, problem: '工具参数必须是 JSON 对象，请重新调用。' }
  }
  return { args: parsed as Record<string, unknown>, problem: '' }
}

/** A trimmed string argument, or an empty string. */
function textArg(value: unknown): string {
  return typeof value === 'string' ? value.trim() : ''
}

/**
 * Whether an `http` call would send a credential.
 *
 * Only the shape is checked here: whether the variable exists is the tool's
 * answer, because the card has to be shown before the request is allowed.
 * @param args - the call's arguments.
 * @returns true when the call named an `auth` object.
 */
function hasCredential(args: Record<string, unknown>): boolean {
  const auth = args.auth
  return auth !== null && auth !== undefined && typeof auth === 'object' && !Array.isArray(auth)
}

/**
 * A short, human-readable description of what a call touches.
 * @param name - the tool name.
 * @param args - the parsed arguments.
 * @returns a path, a pattern, or the tool name.
 */
function describe(name: string, args: Record<string, unknown>): string {
  const path = typeof args.path === 'string' ? args.path : ''
  const pattern = typeof args.pattern === 'string' ? args.pattern : ''
  const command = typeof args.command === 'string' ? args.command.trim() : ''
  const code = typeof args.code === 'string' ? args.code.trim() : ''
  // The MCP gateway is named by its action and target.
  if (name === 'mcp') {
    const action = textArg(args.action)
    const target =
      textArg(args.server) ||
      textArg(args.id) ||
      textArg(args.name) ||
      textArg(args.command) ||
      textArg(args.url)
    return target === '' ? action : `${action} ${target}`
  }
  // A snippet is more useful as a size than as its first line.
  if ((name === 'python' || name === 'node') && code !== '') {
    const lines = code.split('\n').filter((line) => line.trim() !== '').length
    return `${lines} 行代码`
  }
  // `http` is named by its method and URL; it has no path or command.
  if (name === 'http') {
    const url = typeof args.url === 'string' ? args.url.trim() : ''
    const method =
      typeof args.method === 'string' && args.method.trim() !== ''
        ? args.method.trim().toUpperCase()
        : 'GET'
    return url === '' ? `${method}（缺少 url）` : `${method} ${url}`
  }
  // `browser` is named by what it does to the page, not by a path it has none of.
  if (name === 'browser') {
    const action = typeof args.action === 'string' ? args.action.trim().toLowerCase() : 'snapshot'
    const url = typeof args.url === 'string' ? args.url.trim() : ''
    if (url !== '') return `${action} ${url}`
    return action
  }
  // `task` is named by what it does, not by a path it does not have.
  if (name === 'task') {
    const action = typeof args.action === 'string' ? args.action.trim() : ''
    const id = typeof args.id === 'string' ? args.id.trim() : ''
    return id === '' ? action : `${action} ${id}`
  }
  // The interaction tools are named by what they ask for.
  if (name === 'ask_user_question') {
    const question = typeof args.question === 'string' ? args.question.trim() : ''
    return question.length > 200 ? `${question.slice(0, safeEnd(question, 200))}…` : question
  }
  if (name === 'todo_write') {
    const items = Array.isArray(args.items) ? args.items : []
    return `${items.length} 项待办`
  }
  if (name === 'exit_plan_mode') return '提交计划'
  if (name === 'remember') {
    const text = typeof args.text === 'string' ? args.text.trim() : ''
    return text.length > 60 ? `${text.slice(0, safeEnd(text, 60))}…` : text
  }
  // `env` has no target of its own unless the model named a folder.
  if (name === 'env') return path.trim() === '' ? '当前环境' : path
  // `show` names the files the reader will see: the first one, then how many.
  if (name === 'show') {
    const list = Array.isArray(args.paths)
      ? args.paths.filter((item): item is string => typeof item === 'string')
      : []
    const files = list.length > 0 ? list : path.trim() === '' ? [] : [path]
    if (files.length === 0) return '（缺少 paths）'
    const first = files[0].trim().split(/[\\/]/).pop() ?? ''
    return files.length === 1 ? first : `${first} 等 ${files.length} 个文件`
  }
  // A capture names the area it took: the foreground window by default, the whole
  // screen on request, and the crop when the call asked for one. Saying「全屏」for
  // the default was wrong and made the row read like a privacy surprise.
  if (name === 'screenshot') {
    const region =
      typeof args.region === 'string' && args.region.trim().toLowerCase() === 'all'
        ? '整屏'
        : '前台窗口'
    const x = Number(args.x)
    const y = Number(args.y)
    const width = Number(args.width)
    const height = Number(args.height)
    const crop =
      Number.isFinite(x) && Number.isFinite(y) && width > 0 && height > 0
        ? `裁切 ${width}×${height} @ (${x},${y})`
        : ''
    if (crop !== '') return `${region} · ${crop}`
    return path.trim() === '' ? region : path
  }
  if (name === 'run_tests') {
    const command = typeof args.command === 'string' ? args.command.trim() : ''
    const filter = typeof args.filter === 'string' ? args.filter.trim() : ''
    if (command !== '') return firstLine(command)
    return filter === '' ? '自动识别测试' : `测试过滤：${filter}`
  }
  if (name === 'diagnostics') {
    const command = typeof args.command === 'string' ? args.command.trim() : ''
    if (command !== '') return firstLine(command)
    const severity = typeof args.severity === 'string' ? args.severity.trim() : ''
    return severity === '' || severity === 'all' ? '自动识别编译检查' : `编译检查（${severity}）`
  }
  if (name === 'git_write') {
    const action = typeof args.action === 'string' ? args.action.trim().toLowerCase() : ''
    const message = typeof args.message === 'string' ? args.message.trim() : ''
    const branch = typeof args.name === 'string' ? args.name.trim() : ''
    const stash =
      typeof args.stashAction === 'string' && args.stashAction.trim() !== ''
        ? args.stashAction.trim()
        : 'push'
    if (action === 'commit') return `commit ${firstLine(message)}`
    if (action === 'branch') return `branch ${branch}`
    return `stash ${stash}`
  }
  if (name === 'wait_for') {
    const target = typeof args.target === 'string' ? args.target.trim().toLowerCase() : ''
    if (target === 'port') {
      const host = typeof args.host === 'string' && args.host.trim() ? args.host.trim() : '127.0.0.1'
      return `port ${host}:${String(args.port ?? '')}`
    }
    if (target === 'url') {
      return typeof args.url === 'string' ? args.url.trim() : 'url'
    }
    const id = typeof args.id === 'string' ? args.id.trim() : ''
    const pattern = typeof args.pattern === 'string' ? args.pattern.trim() : ''
    return `${id}「${pattern}」`
  }
  // `copy` is named by both ends; `from` alone would hide the destination.
  if (name === 'copy') {
    const from = typeof args.from === 'string' ? args.from : ''
    const to = typeof args.to === 'string' ? args.to : ''
    return `${from} → ${to}`
  }
  // `move` has no `path`: without this its step row would only say "move".
  const from = typeof args.from === 'string' ? args.from : ''
  const target = path || pattern || command || from || name
  return target.length > 200 ? `${target.slice(0, safeEnd(target, 200))}…` : target
}

/** Longest snippet an approval card shows; the full code still runs. */
const APPROVAL_CODE_CHARS = 4_000

/** Keep an approval card readable without changing what is executed. */
function clipCode(code: string): string {
  if (code.length <= APPROVAL_CODE_CHARS) return code
  return `${code.slice(0, safeEnd(code, APPROVAL_CODE_CHARS))}\n…（共 ${code.length} 字符，确认卡只显示前 ${APPROVAL_CODE_CHARS} 字符；执行的是完整代码）`
}

/** Keep the head of a long argument or result, with a visible marker. */
function clip(text: string, limit: number): string {
  return text.length <= limit ? text : `${text.slice(0, safeEnd(text, limit))}\n…（已截断）`
}

/** The arguments of a plugin tool, as the approval card shows them. */
function clipArgs(args: Record<string, unknown>): string {  let text: string
  try {
    text = JSON.stringify(args, null, 2)
  } catch {
    text = String(args)
  }
  return clip(text === undefined ? '{}' : text, APPROVAL_CODE_CHARS)
}

/**
 * Keep one tool result inside the model's share of the context.
 *
 * Both ends survive: the head is what the model asked for, and the tail is
 * where `read` puts its "共 N 行" note and where commands put their errors.
 * The shared implementation lives in `compact.ts`, the first compaction layer.
 * @param text - what the tool produced.
 * @param limit - characters to keep, from {@link toolBudgets}.
 */
function fitToolResult(text: string, limit: number): string {
  return fitForModel(text, limit)
}

/** What an answer cut short by the output cap says about itself. */
const LENGTH_NOTE = '（输出达到长度上限，回答可能未写完。）'

/**
 * The text of one answer, flagged when the provider stopped at the output cap.
 *
 * A partial answer is still worth showing, but returning it as if it were
 * complete hides that it was cut; the person would only notice by re-reading it.
 * @param content - what the provider wrote.
 * @param finishReason - why the round ended.
 * @returns the answer text, with a note when it ended on `length`.
 */
function fullAnswer(content: string, finishReason: string): string {
  const text = content.trim()
  if (text === '' || finishReason !== 'length') return text
  return `${text}\n\n${LENGTH_NOTE}`
}

/**
 * Replace oversized tool arguments with a note once the call has run.
 *
 * The id, type and name stay untouched, so the assistant turn still pairs with
 * its tool result; only the body the model already used is shortened. Without
 * this, one `write` of a large file rides in every later round of the same
 * question, and compaction cannot drop a turn of the newest question.
 * @param call - the wire call, updated in place.
 * @param args - the arguments the tool actually ran with.
 */
function shrinkArguments(call: WireToolCall, args: Record<string, unknown>): void {
  const text = call.function.arguments
  if (text.length <= MAX_WIRE_ARGUMENTS) return
  const where = textArg(args.path) || textArg(args.from) || textArg(args.url)
  call.function.arguments = JSON.stringify({
    ...(where === '' ? {} : { path: where }),
    omitted: `参数过长（${text.length} 字符），已省略；执行结果见对应的工具消息。`
  })
}

/**
 * Whether a provider error means "this prompt is longer than my window".
 *
 * Endpoints word it differently, and the status is not always visible in the
 * message, so the patterns cover the common phrasings in both languages. A miss
 * only costs the retry, never a wrong answer.
 * @param message - what the endpoint said.
 * @returns true when compacting and sending again could help.
 */
function isContextOverflow(message: string): boolean {
  return /context length|context_length|maximum context|too many tokens|prompt is too long|reduce the length|input is too long|上下文长度|超出.{0,6}(长度|上限)|token.{0,10}(超|上限)|请求过长/i.test(
    message
  )
}

/** The first non-empty line of a tool result. */
function firstLine(text: string): string {
  const line = text.split('\n').find((candidate) => candidate.trim().length > 0) ?? ''
  return line.length > 120 ? `${line.slice(0, safeEnd(line, 120))}…` : line
}

/**
 * A one-line account of what a call did, for the transcript.
 *
 * Lists (`list`, `glob`, `grep`, `read`) would otherwise show their first line —
 * a file name or a line of code — so they are counted instead.
 * @param name - the tool that ran.
 * @param ok - whether it succeeded.
 * @param text - everything the tool returned.
 * @returns a label such as `已读取 12 行`, or the error's first line.
 */
export function summarise(name: string, ok: boolean, text: string): string {
  if (!ok || name === 'write' || name === 'edit') return firstLine(text)
  if (text.startsWith('没有匹配')) return firstLine(text)
  if (name === 'read') {
    const shown = count(text, /^\d+\t/)
    return shown === 0 ? '文件是空的' : `已读取 ${shown} 行`
  }
  if (name === 'list') {
    if (text.startsWith('文件夹是空的')) return text
    const items = text
      .split('\n')
      .filter((line) => line.length > 0 && !line.startsWith('…')).length
    return `${items} 项`
  }
  if (name === 'glob') return `${count(text, /\S/)} 个文件`
  if (name === 'tree') {
    // The first line is the header ("…（深度 2，12 项）"), the rest are entries.
    const entries = count(text, /\S/) - 1
    return entries <= 0 ? firstLine(text) : `${entries} 项`
  }
  if (name === 'grep') {
    const hits = count(text, /^.*:\d+: /)
    return hits === 0 ? firstLine(text) : `${hits} 处匹配`
  }
  return firstLine(text)
}

/** How many lines of a tool result match a pattern. */
function count(text: string, pattern: RegExp): number {
  return text.split('\n').filter((line) => pattern.test(line)).length
}

/**
 * Clamp a requested budget into the range the loop can honour.
 * @param value - what the caller asked for, if anything.
 * @param fallback - the default when the value is absent or not a number.
 * @param min - smallest accepted value.
 * @param max - largest accepted value.
 * @returns an integer inside the range.
 */
function bounded(value: number | undefined, fallback: number, min: number, max: number): number {
  const chosen = typeof value === 'number' && Number.isFinite(value) ? Math.trunc(value) : fallback
  return Math.min(Math.max(chosen, min), max)
}

/**
 * The first value that is present and not blank.
 * @param values - candidates, highest priority first.
 * @returns the trimmed value, or an empty string.
 */
function firstNonEmpty(...values: Array<string | undefined>): string {
  for (const value of values) {
    const trimmed = (value ?? '').trim()
    if (trimmed) return trimmed
  }
  return ''
}
