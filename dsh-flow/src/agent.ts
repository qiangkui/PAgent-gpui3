/**
 * The assistant loop: ask the model, run the tools it asks for, ask again.
 *
 * A single request/answer pair is not enough once the model can touch files, so
 * this module keeps feeding tool results back until the model answers in prose.
 * Every tool runs inside the conversation directory (see `tools.ts`); the model
 * never sees or sets a path outside it.
 */

import { DEFAULT_BASE_URL, DEFAULT_MODEL, historyMessages, userContent } from './ai.ts'
import type { AiRequest } from './ai.ts'
import {
  MAX_CODE_CHARS,
  MAX_SHELL_COMMAND,
  TOOLS,
  previewToolScoped,
  runToolScoped,
  safeEnd
} from './tools.ts'
import type { Sandbox } from './tools.ts'
import { outsidePaths, resolveCommandDir, sandboxRefusal } from './tools/sandbox.ts'
import { httpSummary } from './tools/http.ts'
import { streamCompletion } from './stream.ts'
import { cacheHitSize, contextSize } from './stream.ts'
import { collectPanels, runPluginTool } from './plugins.ts'
import type { PanelReport, PluginError, PluginPanel, PluginTool } from './plugins.ts'
import type { McpTool } from './mcp.ts'
import type { WireToolCall } from './stream.ts'
import {
  compactLimits,
  compactTranscript,
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
const READ_ONLY_TOOLS = new Set(['read', 'list', 'glob', 'grep', 'tree', 'symbols', 'git'])

/** Longest tool argument text kept for the transcript's expanded row. */
export const MAX_STEP_INPUT = 2_000

/** Longest tool result text kept for the transcript's expanded row. */
export const MAX_STEP_OUTPUT = 4_000

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
   * Prompt tokens the provider served from its cache, summed over rounds. The
   * bill is cheaper by this much, so the window shows it beside the spend.
   */
  cacheHits: number
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
  /**
   * Folders `shell` / `python` / `node` may reach; null (or absent) skips the
   * path check. The window passes the workspace plus its allowlist, and leaves
   * it off in the full-access modes.
   */
  sandbox?: Sandbox | null
  /** Called before a `write` / `edit`; resolves false to leave the file alone. */
  approve?: (request: ApprovalRequest) => Promise<boolean>
  /** Reports a write that proceeds without waiting for a decision. */
  onApproval?: (request: ApprovalRequest) => void
  /**
   * Called before every `shell` command and every destructive file action
   * (`move`, `delete`); resolves false to skip it. Without it those calls are
   * refused, so a caller that cannot ask never runs one.
   */
  confirmActions?: (request: ApprovalRequest) => Promise<boolean>
  /** Called once per tool call, before its result goes back to the model. */
  onStep?: (step: AgentStep) => void
  /**
   * Called before every round trip, with the round about to start and how many
   * tool calls have finished, so a status line can follow the loop live.
   */
  onProgress?: (progress: AgentProgress) => void
  /** Called with each fragment of the answer, as the model writes it. */
  onDelta?: (text: string) => void
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
}

/** One entry of the wire-level `messages` array. */
interface WireMessage {
  role: 'system' | 'user' | 'assistant' | 'tool'
  /** A string for every role except the user message that carries files. */
  content: string | Array<Record<string, unknown>>
  tool_calls?: WireToolCall[]
  tool_call_id?: string
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
  // Plugin and MCP tools are not in the window's allow-list (that list names
  // the engine's own tools), so they are added on top and callable by name.
  const pluginNames = new Set(pluginTools.map((tool) => tool.name))
  const mcpNames = new Set(mcpTools.map((tool) => tool.name))
  const tools = [
    ...builtins,
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
  /** Whether this request may run the named tool. */
  const permitted = (name: string): boolean =>
    allowed === null || allowed.has(name) || pluginNames.has(name) || mcpNames.has(name)
  const scope = options.scopeRoot === undefined ? options.root : options.scopeRoot

  const messages: WireMessage[] = []
  const system = (request.system ?? '').trim()
  // Prompt fragments from plugins ride along with the system prompt, after the
  // window's own instructions so they can only add to them.
  const injected = (options.promptInjections ?? [])
    .map((prompt) => prompt.text.trim())
    .filter((text) => text !== '')
    .join('\n\n')
  const full = [system, injected].filter((part) => part !== '').join('\n\n')
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
    toolChars: tools.reduce(
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
    items: tools.map((tool) => ({ name: tool.function.name, schema: JSON.stringify(tool) })),
    prompts: options.promptInjections ?? [],
    panels: collected.panels,
    panelErrors: collected.errors
  })

  let calls = 0
  let reason = ''
  // Indices of tool messages, oldest first, plus their text, so the transcript
  // can shrink without breaking the `tool_call_id` pairing.
  const toolResults: number[] = []
  let toolChars = 0
  /** Replace the oldest results until the transcript fits its budget. */
  const trimToolContext = (): void => {
    while (toolChars > MAX_TOOL_CONTEXT && toolResults.length > 1) {
      const oldest = toolResults[0]!
      const content = messages[oldest]!.content
      // Dropping a result that is already shorter than the note would only
      // make the transcript longer, so a run of tiny results stops here.
      if (typeof content !== 'string' || content.length <= DROPPED_TOOL_RESULT.length) break
      toolResults.shift()
      toolChars -= content.length
      messages[oldest]!.content = DROPPED_TOOL_RESULT
      toolChars += DROPPED_TOOL_RESULT.length
    }
  }
  /** Record one tool result and keep the whole set inside the budget. */
  const pushToolResult = (id: string, content: string): void => {
    messages.push({ role: 'tool', tool_call_id: id, content })
    toolResults.push(messages.length - 1)
    toolChars += content.length
    trimToolContext()
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
  // Prompt tokens the provider served from its cache, summed over rounds.
  let cacheHits = 0
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
    // rebuilt from the transcript instead of trusted.
    toolResults.length = 0
    toolChars = 0
    for (const [index, message] of messages.entries()) {
      if (message.role !== 'tool' || typeof message.content !== 'string') continue
      toolResults.push(index)
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
    build: () => Record<string, unknown>
  ): Promise<{ answer: Awaited<ReturnType<typeof streamCompletion>>; retried: boolean }> => {
    try {
      return {
        answer: await streamCompletion({
          url: `${baseUrl}/chat/completions`,
          apiKey,
          label: model,
          body: build(),
          fetchImpl: options.fetchImpl,
          timeoutMs: request.timeoutMs,
          onDelta: options.onDelta
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
          onDelta: options.onDelta
        }),
        retried: true
      }
    }
  }
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
        tools,
        stream_options: { include_usage: true }
      }
      if (typeof request.temperature === 'number') body.temperature = request.temperature
      if (typeof request.maxTokens === 'number') body.max_tokens = request.maxTokens
      return body
    }
    const { answer, retried } = await askOnce(build)

    spent = addUsage(usage, answer.usage) || spent
    context = contextSize(answer.usage) || context
    cacheHits += cacheHitSize(answer.usage)
    const counted = roundCount(answer.usage)
    if (counted !== null) roundTokens.push([counted[0], counted[1], answer.toolCalls.length])

    if (answer.toolCalls.length === 0) {
      const text = answer.content.trim()
      if (!text) throw new Error(`${model} 没有返回内容`)
      return {
        text,
        model,
        usage: spent ? usage : undefined,
        context,
        rounds: retried ? round + 1 : round,
        roundTokens,
        cacheHits,
        compaction
      }
    }

    messages.push({ role: 'assistant', content: answer.content, tool_calls: answer.toolCalls })
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
    const prepared = onlyReads
      ? await Promise.all(
          planned.map((entry, index) => {
            if (index >= remaining || entry.parsed.problem !== '') return null
            if (!permitted(entry.name)) return null
            return runToolWithTimeout(options, scope, entry.call.id, entry.name, entry.args)
          })
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
        pushToolResult(call.id, skipped)
        reason = `工具调用达到 ${maxCalls} 次上限`
        continue
      }
      // The model only sees the allowed tools, but a hallucinated name still
      // must not reach a tool that this request turned off. Broken arguments
      // are reported as-is instead of running the tool with an empty object,
      // which would only produce a misleading "缺少参数" error.
      const ready = prepared[index]
      let result: { ok: boolean; text: string }
      if (ready) {
        result = ready
      } else if (parsed.problem !== '') {
        result = { ok: false, text: parsed.problem }
      } else if (!permitted(name)) {
        result = { ok: false, text: `工具未启用：${name}` }
      } else {
        // A broken approval bridge must fail this call, not the answer: the
        // write stays undone and the model still gets a usable turn.
        result = await runToolWithTimeout(options, scope, call.id, name, args)
      }
      options.onStep?.({
        tool: name,
        target: describe(name, args),
        ok: result.ok,
        detail: summarise(name, result.ok, result.text),
        input: clip(argsText, MAX_STEP_INPUT),
        output: clip(result.text, MAX_STEP_OUTPUT),
        call: call.id
      })
      pushToolResult(call.id, fitToolResult(result.text))
    }
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
  try {
    const answer = await streamCompletion({
      url: `${baseUrl}/chat/completions`,
      apiKey,
      label: model,
      body,
      fetchImpl: options.fetchImpl,
      timeoutMs: request.timeoutMs,
      onDelta: options.onDelta
    })
    spent = addUsage(usage, answer.usage) || spent
    context = contextSize(answer.usage) || context
    cacheHits += cacheHitSize(answer.usage)
    const counted = roundCount(answer.usage)
    if (counted !== null) roundTokens.push([counted[0], counted[1], 0])
    const text = answer.content.trim()
    const note = `（${reason}，先给出基于已有信息的结果；需要继续时再发一条消息。）`
    return {
      text: text ? `${text}\n\n${note}` : note,
      model,
      usage: spent ? usage : undefined,
      context,
      rounds: taken + 1,
      roundTokens,
      cacheHits,
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
      cacheHits,
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
): Promise<{ ok: boolean; text: string }> {
  if (name === 'mcp') {
    const gateway = options.mcpTools?.find((tool) => tool.name === 'mcp')
    if (!gateway) return { ok: false, text: 'MCP 工具没有加载。' }
    const action = typeof args.action === 'string' ? args.action.trim().toLowerCase() : ''
    // Listing and disconnecting change nothing outside this question; linking
    // starts a process or opens a connection, and calling runs its tool.
    if (action === 'link' || action === 'call') {
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
      cwd: ''
    }
    if (!(await options.confirmActions(question))) {
      return { ok: false, text: '用户拒绝了这次请求，没有发出。' }
    }
  }
  if (name === 'task') {
    const action = typeof args.action === 'string' ? args.action.trim().toLowerCase() : ''
    if (action === 'stop' || action === 'stopall') {
      const taskId = typeof args.id === 'string' ? args.id.trim() : ''
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
  }
  if (name === 'move' || name === 'delete') {
    const from = typeof args.from === 'string' ? args.from : ''
    const to = typeof args.to === 'string' ? args.to : ''
    const target = typeof args.path === 'string' ? args.path : ''
    if (name === 'move' && (!from || !to)) {
      return { ok: false, text: '缺少参数 from / to' }
    }
    if (name === 'delete' && !target) return { ok: false, text: '缺少参数 path' }
    if (!options.confirmActions) {
      return { ok: false, text: '没有确认界面，这次改动没有执行。' }
    }
    const question: ApprovalRequest =
      name === 'move'
        ? {
            call: id,
            tool: name,
            target: `${from} → ${to}`,
            path: from,
            diff: `移动：${from} → ${to}`,
            undoable: false
          }
        : {
            call: id,
            tool: name,
            target,
            path: target,
            diff: `删除文件：${target}`,
            undoable: false
          }
    if (!(await options.confirmActions(question))) {
      return { ok: false, text: '用户拒绝了这次改动，文件没有改动。' }
    }
  }
  if (name === 'write' || name === 'edit' || name === 'replace') {
    // Even without a decision the window gets the diff and the undo pair, so
    // direct writes stay visible and reversible. A batch replace has no undo
    // pair: one card cannot cover several files, so it reports `undoable: false`.
    const preview = await previewToolScoped(options.root, scope, { id, name, args })
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
      } else {
        options.onApproval?.(question)
      }
    }
  }
  if (name === 'shell' || name === 'python' || name === 'node' || name === 'http') {
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
    options.sandbox ?? null
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
): Promise<{ ok: boolean; text: string }> {
  const work = runAllowedTool(options, scope, id, name, args).catch((error: unknown) => {
    const message = error instanceof Error ? error.message : String(error)
    return { ok: false, text: `工具没有执行：${message}` }
  })
  if (name === 'shell' || name === 'python' || name === 'node') return await work
  let timer: ReturnType<typeof setTimeout> | undefined
  const limit = new Promise<{ ok: boolean; text: string }>((resolve) => {
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
  // `task` is named by what it does, not by a path it does not have.
  if (name === 'task') {
    const action = typeof args.action === 'string' ? args.action.trim() : ''
    const id = typeof args.id === 'string' ? args.id.trim() : ''
    return id === '' ? action : `${action} ${id}`
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
 */
function fitToolResult(text: string): string {
  return fitForModel(text, MAX_TOOL_MESSAGE)
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
