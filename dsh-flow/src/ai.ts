/**
 * The `ai` bridge: one question to a chat-completions provider.
 *
 * The window sends `{ prompt, system?, model?, apiKey?, baseUrl? }` on stdin and
 * reads one JSON line back (`{ text, model, usage }` or `{ error }`). Credentials
 * arrive per request so the UI can own them; `DEEPSEEK_API_KEY`,
 * `DEEPSEEK_BASE_URL` and `DEEPSEEK_MODEL` act as fallbacks for shell use.
 */

import { streamCompletion } from './stream.ts'
import { contextSize } from './stream.ts'
import { estimateTokens } from './compact.ts'
import { serverToolDeclarations } from './web.ts'

/** Default provider endpoint; DeepSeek is OpenAI-compatible here. */
export const DEFAULT_BASE_URL = 'https://api.deepseek.com'

/** Model used when the request and the environment do not name one. */
export const DEFAULT_MODEL = 'deepseek-flash'

/** One request as it arrives on stdin. */
/** One earlier turn as the window sends it. */
export interface WireTurn {
  /** `user` or `assistant`; anything else is dropped. */
  role?: string
  /** What was said. */
  content?: string
  /**
   * Server-tool calls of an assistant turn, raw as the provider sent them.
   * Passing them back is what lets the provider restore the pages it read
   * earlier; a framework that drops them still gets an answer, just without
   * that context.
   */
  serverToolCalls?: unknown[]
}

/** One attached file as the window sends it: base64 bytes, no `data:` prefix. */
export interface WireAttachment {
  /** `image` or `pdf`; anything else is sent as an image when it has a MIME type. */
  kind?: 'image' | 'pdf'
  /** File name, used in error messages and the PDF part. */
  name?: string
  /** MIME type, for example `image/png` or `application/pdf`. */
  mimeType?: string
  /** The file itself, base64 encoded. */
  data?: string
}

/** One user message: text, or text plus attached files. */
export type UserContent = string | Array<Record<string, unknown>>

/** One MCP server the window linked and switched on. */
export interface McpServerConfig {
  /** Stable server id; also the key in the window's settings. */
  id: string
  /** Name shown in the model's tool descriptions. */
  name?: string
  /** One-line description from a manifest. */
  description?: string
  /** Why a catalog entry cannot be used; absent when it looks usable. */
  problem?: string
  /** `stdio` (a child process) or `http` (streamable HTTP). */
  transport?: string
  /** Executable for a stdio server. */
  command?: string
  /** Arguments passed to `command`. */
  args?: string[]
  /** Directory a stdio server starts in; absent means the conversation's. */
  cwd?: string
  /** Extra environment variables for a stdio server. */
  env?: Record<string, string>
  /** Endpoint for an http server. */
  url?: string
  /** Extra HTTP headers for an http server. */
  headers?: Record<string, string>
}

export interface AiRequest {
  /** The question. */
  prompt: string
  /**
   * Files attached to this question.
   *
   * Images travel as `image_url` parts and PDFs as `file` parts of the same
   * user message, which is how OpenAI-compatible providers accept them. A
   * provider without vision still answers the text, or reports why it refused.
   */
  attachments?: WireAttachment[]
  /** Optional instruction prepended as a system message. */
  system?: string
  /** Earlier turns of this conversation, oldest first. */
  history?: WireTurn[]
  /** Model id, for example `deepseek-reasoner`. */
  model?: string
  /** Key sent as a bearer token; falls back to `DEEPSEEK_API_KEY`. */
  apiKey?: string
  /** Endpoint base; falls back to `DEEPSEEK_BASE_URL` then DeepSeek's host. */
  baseUrl?: string
  /** Sampling temperature, when the caller needs it pinned. */
  temperature?: number
  /** Upper bound on generated tokens. */
  maxTokens?: number
  /**
   * Thinking choice: `off` answers without reasoning, `low` / `high` / `max`
   * set how hard the model thinks, anything else leaves it to the endpoint.
   * Unknown values send nothing, which keeps older windows working.
   */
  reasoning?: string
  /**
   * True to declare DeepSeek's server tools (`deepseek_web_search`,
   * `deepseek_read_url`). The model decides whether to search, and the search
   * runs on the provider's side, so nothing in this engine executes it. Only
   * `deepseek-flash` offers them; an endpoint that refuses the entries is
   * asked again without them rather than failing the question.
   */
  webSearch?: boolean
  /** Model context window in tokens, as the window reports it; 0 when unknown. */
  contextWindow?: number
  /** Abort the request after this many milliseconds. */
  timeoutMs?: number
  /** True when the loop may use tools at all; absent means the answer is prose. */
  tools?: boolean
  /** Model round-trips one question may take; the engine clamps it. */
  maxRounds?: number
  /** Tool calls one question may make; the engine clamps it. */
  maxCalls?: number
  /** Folders outside the working directory the tools may read, never write. */
  readOnlyRoots?: string[]
  /** Tool names this request may use; absent means every tool. */
  allowedTools?: string[]
  /** Folder holding the installed plugins, one per subfolder. */
  pluginDir?: string
  /** Plugin folder names that are switched on; absent or empty loads none. */
  plugins?: string[]
  /** MCP servers to open for this question; absent or empty starts none. */
  mcpServers?: McpServerConfig[]
  /** Folder holding `mcpserver` manifests, for the `mcp` tool's catalog. */
  mcpDir?: string
  /** True when the caller keeps stdin open for approval replies. */
  interactive?: boolean
  /** True when this question runs in plan mode: investigate, then submit a plan. */
  plan?: boolean
  /**
   * True when the host keeps a long-term memory file, which is what makes the
   * `remember` tool available. The file itself belongs to the window: the
   * engine only reports the entry and the host writes it.
   */
  memory?: boolean
  /** True when `write` / `edit` wait for a decision before running. */
  confirmWrites?: boolean
  /** `workspace` keeps writes under `scopeRoot`; `full` allows any path. */
  fileScope?: 'workspace' | 'full'
  /** Writable root for the `workspace` scope. */
  scopeRoot?: string
  /**
   * Top-level folder names under the workspace that searches must not walk.
   *
   * Every conversation is a folder directly under the workspace, so without this
   * `glob` / `grep` / `tree` from one conversation would list and search the files
   * another conversation's assistant wrote.
   */
  skipDirs?: string[]
  /**
   * Folders outside the workspace that `shell`, `python` and `node` may run in
   * and refer to. Only consulted while `fileScope` is `workspace`; the
   * full-access modes run without a path guard.
   */
  allowPaths?: string[]
  /**
   * Context window of the model, in tokens. Compaction thresholds scale from
   * it, so the engine compacts at the same point the window shows as full.
   */
  windowTokens?: number
}

/** What the provider answered. */
export interface AiResult {
  /** Assistant message content. */
  text: string
  /** Model that produced it, as reported by the request. */
  model: string
  /** Token accounting, when the provider returns it. */
  usage?: unknown
  /** Prompt + answer tokens of the request: how much it put in the window. */
  context: number
  /**
   * Thinking text the provider streamed before the answer, when it has one.
   * The window estimates tokens from it, so none of that output is invisible.
   */
  reasoning: string
  /** Server tools the provider ran on its own side, raw as they came. */
  serverCalls: unknown[]
  /** Citation entries of the answer (`url_citation`), raw as they came. */
  annotations: unknown[]
  /** Why the round ended: `stop`, `length`, `content_filter`, or `''`. */
  finishReason: string
}

/** What a model listing answered. */
export interface ModelListResult {
  /** Model ids the key is allowed to call, in the provider's order. */
  models: string[]
  /** True when `GET /models` failed and the single configured model was probed. */
  degraded?: boolean
  /** Why the plain listing failed, when `degraded` is true. */
  reason?: string
}

/** Injectable side effects, so tests need no network. */
export interface AskOptions {
  /** Swap in a stub instead of the global fetch. */
  fetchImpl?: typeof fetch
  /** Environment to read fallbacks from. */
  env?: Record<string, string | undefined>
  /** Called with each fragment of the answer, as the model writes it. */
  onDelta?: (text: string) => void
  /** Called with each fragment of the model's thinking, when it streams one. */
  onReasoning?: (text: string) => void
  /** Called with each server-tool event, so a search can be shown as it runs. */
  onServerEvent?: (event: unknown) => void
}

/** One chat message in the provider's shape. */
export interface ChatMessage {
  role: 'system' | 'user' | 'assistant'
  content: UserContent
  /**
   * Server tools the provider ran in this assistant turn, passed back verbatim
   * so it can restore the pages it read. Absent on user turns.
   */
  server_tool_calls?: unknown[]
}

/**
 * Build the user message, attaching images and PDFs when there are any.
 *
 * Plain text stays a string so endpoints that predate content parts keep
 * working. The order is text first, then the files in the order they were
 * attached.
 * @param prompt - the rendered question.
 * @param attachments - files from the window, base64 encoded.
 * @returns the content the provider expects.
 */
export function userContent(prompt: string, attachments: WireAttachment[] | undefined): UserContent {
  const files = (attachments ?? []).filter(
    (file) => typeof file.data === 'string' && file.data.trim() !== ''
  )
  if (files.length === 0) return prompt
  const parts: Array<Record<string, unknown>> = [{ type: 'text', text: prompt }]
  for (const file of files) {
    const mime = (file.mimeType ?? '').trim()
    const data = file.data!.trim()
    if (file.kind === 'pdf' || mime === 'application/pdf') {
      parts.push({
        type: 'file',
        file: {
          filename: (file.name ?? 'attachment.pdf').trim() || 'attachment.pdf',
          file_data: `data:${mime || 'application/pdf'};base64,${data}`
        }
      })
    } else {
      parts.push({
        type: 'image_url',
        image_url: { url: `data:${mime || 'image/png'};base64,${data}` }
      })
    }
  }
  return parts
}

/** How many earlier turns are replayed at most. */
export const MAX_HISTORY = 40

/**
 * Rough token budget for those turns.
 *
 * A message count alone is not enough: a few long answers can fill the window
 * even when the conversation is short. Measured in tokens, not characters,
 * because the two units disagree by a factor of four: 120k characters is about
 * 30k tokens of English but about 120k tokens of Chinese, which alone fills a
 * small window. The window keeps its own history to 30k tokens, so a shell
 * caller gets the same cut-down history the UI would send.
 */
export const MAX_HISTORY_TOKENS = 30_000

/**
 * Turn the window's history into wire messages.
 *
 * The window sends the conversation so a follow-up question ("那它呢？") still has
 * a subject. Unknown roles and blank turns are dropped rather than failing the
 * request; the last {@link MAX_HISTORY} turns are kept, and whole oldest turns
 * are dropped while the text exceeds {@link MAX_HISTORY_TOKENS} so a long
 * conversation cannot blow up the prompt.
 * @param history - earlier turns, oldest first.
 * @returns messages the provider accepts, oldest first.
 */
export function historyMessages(history: WireTurn[] | undefined): ChatMessage[] {
  const messages: ChatMessage[] = []
  // Measured as each turn is accepted: the content of a user turn may become
  // content parts later, and the estimate is what the window itself uses.
  const sizes: number[] = []
  for (const turn of history ?? []) {
    const role = turn?.role === 'assistant' ? 'assistant' : turn?.role === 'user' ? 'user' : null
    const content = (turn?.content ?? '').trim()
    if (!role || !content) continue
    // The provider reads its own server calls back from the assistant turn; the
    // shape is opaque to this engine, so it is passed through untouched.
    const calls = role === 'assistant' ? turn.serverToolCalls : undefined
    messages.push(
      calls && calls.length > 0 ? { role, content, server_tool_calls: calls } : { role, content }
    )
    sizes.push(estimateTokens(content))
  }
  const first = Math.max(0, messages.length - MAX_HISTORY)
  const kept = messages.slice(first)
  const keptSizes = sizes.slice(first)
  let total = keptSizes.reduce((sum, size) => sum + size, 0)
  let drop = 0
  // Keep at least one turn: dropping every message would erase the subject.
  while (total > MAX_HISTORY_TOKENS && drop + 1 < kept.length) {
    total -= keptSizes[drop]!
    drop += 1
  }
  return kept.slice(drop)
}

/**
 * Apply the window's thinking choice to a request body.
 *
 * DeepSeek's `deepseek-flash` and `deepseek-v4-pro` think by default. The API
 * switches that off with `{"thinking":{"type":"disabled"}}` and trades depth for
 * latency with `reasoning_effort` (`low` / `high` / `max`); the effort is only
 * sent for a choice that names one, so the endpoint keeps its own default
 * otherwise. Endpoints that know neither key ignore what they do not recognise,
 * and `default` sends nothing at all.
 * @param body - the request body being built.
 * @param reasoning - the choice from the window, when there is one.
 */
export function applyThinking(body: Record<string, unknown>, reasoning?: string): void {
  if (reasoning === 'off') {
    body.thinking = { type: 'disabled' }
    return
  }
  if (reasoning === 'low' || reasoning === 'high' || reasoning === 'max') {
    body.reasoning_effort = reasoning
  }
}

/**
 * Ask the provider one question.
 * @param request - the parsed stdin request.
 * @param options - fetch and environment overrides.
 * @returns the assistant text plus the model that answered.
 */
export async function askModel(request: AiRequest, options: AskOptions = {}): Promise<AiResult> {
  const env = options.env ?? process.env
  const prompt = (request.prompt ?? '').trim()
  if (!prompt) throw new Error('prompt 为空')

  const apiKey = firstNonEmpty(request.apiKey, env.DEEPSEEK_API_KEY)
  if (!apiKey) {
    throw new Error('缺少 DeepSeek API Key：在界面「设置 → AI」中填写，或设置 DEEPSEEK_API_KEY')
  }
  const baseUrl = firstNonEmpty(request.baseUrl, env.DEEPSEEK_BASE_URL, DEFAULT_BASE_URL).replace(/\/+$/, '')
  const model = firstNonEmpty(request.model, env.DEEPSEEK_MODEL, DEFAULT_MODEL)

  const messages: ChatMessage[] = []
  const system = (request.system ?? '').trim()
  if (system) messages.push({ role: 'system', content: system })
  messages.push(...historyMessages(request.history))
  messages.push({ role: 'user', content: userContent(prompt, request.attachments) })

  const body: Record<string, unknown> = { model, messages, stream: true, stream_options: { include_usage: true } }
  if (typeof request.temperature === 'number') body.temperature = request.temperature
  if (typeof request.maxTokens === 'number') body.max_tokens = request.maxTokens
  applyThinking(body, request.reasoning)
  // The server tools are the whole `tools` array here: a one-shot question has
  // no function tools to mix them with.
  if (request.webSearch === true) body.tools = serverToolDeclarations()

  const answer = await streamCompletion({
    url: `${baseUrl}/chat/completions`,
    apiKey,
    label: model,
    // Token counts only arrive on the stream when they are asked for.
    body,
    fetchImpl: options.fetchImpl,
    timeoutMs: request.timeoutMs,
    onDelta: options.onDelta,
    onReasoning: options.onReasoning,
    onServerEvent: options.onServerEvent
  })
  const text = answer.content.trim()
  if (!text) throw new Error(emptyAnswerReason(model, answer.finishReason))
  return {
    text,
    model,
    usage: answer.usage,
    context: contextSize(answer.usage),
    reasoning: answer.reasoning,
    serverCalls: answer.serverCalls,
    annotations: answer.annotations,
    finishReason: answer.finishReason
  }
}

/**
 * Why a round came back empty, in the words the window shows.
 *
 * `content_filter` is the provider's review refusing the answer, and `length`
 * is the output cap cutting it off before any text survived; both used to read
 * as the vague "没有返回内容".
 * @param model - model id, for the message.
 * @param finishReason - what the provider reported, when it reported anything.
 * @returns a sentence naming the cause.
 */
export function emptyAnswerReason(model: string, finishReason: string): string {
  if (finishReason === 'content_filter') return `${model} 的回答被内容审核拒绝（content_filter）`
  if (finishReason === 'length') return `${model} 的输出达到长度上限（length），没有可用文本`
  return `${model} 没有返回内容`
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

/** Shape of the bits of the provider's model listing this bridge reads. */
interface ModelListResponse {
  data?: Array<{ id?: string }>
  models?: Array<string | { id?: string; name?: string }>
  error?: { message?: string }
  message?: string
}

/** The part of a request `listModels` needs: where to ask, and with which key. */
export interface EndpointRequest {
  /** Key sent as a bearer token; falls back to `DEEPSEEK_API_KEY`. */
  apiKey?: string
  /** Endpoint base; falls back to `DEEPSEEK_BASE_URL` then DeepSeek's host. */
  baseUrl?: string
}

/**
 * Ask the provider which models the key may call.
 *
 * Version names change often (for example DeepSeek's `deepseek-chat` pointing at
 * a newer generation), so the window offers whatever the endpoint reports rather
 * than a hard-coded list. OpenAI-compatible providers answer `GET {base}/models`.
 * @param request - the parsed stdin request; `apiKey` and `baseUrl` are used.
 * @param options - fetch and environment overrides.
 * @returns the model ids, de-duplicated and sorted.
 */
export async function listModels(
  request: EndpointRequest,
  options: AskOptions = {}
): Promise<ModelListResult> {
  const env = options.env ?? process.env
  const doFetch = options.fetchImpl ?? fetch
  const apiKey = firstNonEmpty(request.apiKey, env.DEEPSEEK_API_KEY)
  if (!apiKey) {
    throw new Error('缺少 API Key：先在接入点里填好 Key')
  }
  const baseUrl = firstNonEmpty(request.baseUrl, env.DEEPSEEK_BASE_URL, DEFAULT_BASE_URL).replace(/\/+$/, '')

  const response = await doFetch(`${baseUrl}/models`, {
    method: 'GET',
    headers: { authorization: `Bearer ${apiKey}` }
  })
  const payload = (await response.json().catch(() => null)) as ModelListResponse | null
  if (!response.ok) {
    const detail = payload?.error?.message ?? payload?.message ?? `HTTP ${response.status}`
    throw new Error(`读取模型列表失败：${detail}`)
  }

  const ids = new Set<string>()
  for (const entry of payload?.data ?? []) {
    const id = (entry?.id ?? '').trim()
    if (id) ids.add(id)
  }
  for (const entry of payload?.models ?? []) {
    const id = (typeof entry === 'string' ? entry : entry?.id ?? entry?.name ?? '').trim()
    if (id) ids.add(id)
  }
  if (ids.size === 0) throw new Error('该接入点没有返回任何模型')
  return { models: [...ids].sort() }
}
