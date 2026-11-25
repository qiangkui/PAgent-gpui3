/**
 * Read a chat-completions answer as it arrives.
 *
 * Providers stream server-sent events: `data: {...}` chunks and then
 * `data: [DONE]`. A provider that ignores `stream: true` answers with one JSON
 * body instead, so both shapes land here and the caller sees one result type.
 */

/** One tool call as the provider sends it. */
export interface WireToolCall {
  /** Provider-assigned id; the tool result refers back to it. */
  id: string
  /** Always `function` for the tools this app defines. */
  type: 'function'
  function: {
    /** Tool name. */
    name: string
    /** Arguments as a JSON string, as the provider wrote them. */
    arguments: string
  }
}

/** What one round with the model produced. */
export interface StreamResult {
  /** Assistant text of this round, already concatenated. */
  content: string
  /**
   * Thinking text the provider streamed before the answer, when it has one.
   *
   * DeepSeek's reasoner sends it as `reasoning_content`, some gateways as
   * `reasoning`. It is output the bill includes but never a prompt, which is
   * why it is measured and kept apart from {@link content}.
   */
  reasoning: string
  /** Tool calls the model asked for; empty when it answered in prose. */
  toolCalls: WireToolCall[]
  /** Token accounting, when the provider reports it. */
  usage?: unknown
  /**
   * Server tools the provider ran on its own side this round (web search, page
   * reads), raw as they came. Nothing in this client executes them; they are
   * kept so the transcript can show what was searched and so a caller can list
   * the sources.
   */
  serverCalls: unknown[]
  /** Citation entries (`url_citation`) of the answer, raw as they came. */
  annotations: unknown[]
  /** Why the round ended: `stop`, `length`, `content_filter`, or `''`. */
  finishReason: string
}

/**
 * How much one round put in front of the model.
 *
 * A single request's prompt plus its answer is what the next request carries,
 * so this grows with the conversation rather than with the number of tool
 * rounds. Summing {@link StreamResult.usage} over rounds instead measures what
 * the question cost, which says nothing about how full the window is.
 * @param usage - what the provider reported for one round, if anything.
 * @returns prompt + completion, or 0 when the round reported nothing.
 */
export function contextSize(usage: unknown): number {
  if (!usage || typeof usage !== 'object') return 0
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
  return prompt + completion > 0 ? prompt + completion : sum
}

/**
 * How many prompt tokens of one round the provider served from its cache.
 *
 * Providers name this differently — DeepSeek sends `prompt_cache_hit_tokens`,
 * Anthropic `cache_read_input_tokens`, OpenAI nests it under
 * `prompt_tokens_details.cached_tokens` — so all three are read here.
 * @param usage - what the provider reported for one round, if anything.
 * @returns the cached count, or 0 when the round reported none.
 */
export function cacheHitSize(usage: unknown): number {
  if (!usage || typeof usage !== 'object') return 0
  const counts = usage as Record<string, unknown>
  const number = (...keys: string[]): number => {
    for (const key of keys) {
      const value = counts[key]
      if (typeof value === 'number' && Number.isFinite(value)) return value
    }
    return 0
  }
  const direct = number('prompt_cache_hit_tokens', 'cache_read_input_tokens', 'cached_tokens')
  if (direct > 0) return direct
  const details = counts.prompt_tokens_details
  if (details && typeof details === 'object') {
    const nested = (details as Record<string, unknown>).cached_tokens
    if (typeof nested === 'number' && Number.isFinite(nested)) return nested
  }
  return 0
}

/** One streaming request. */
export interface StreamRequest {
  /** Full endpoint, for example `https://api.deepseek.com/chat/completions`. */
  url: string
  /** Key sent as the bearer token. */
  apiKey: string
  /** Prefix for error messages, usually the model id. */
  label: string
  /** Request body, already shaped for the provider. */
  body: unknown
  /** Stub for tests. */
  fetchImpl?: typeof fetch
  /** Abort the request after this many milliseconds. */
  timeoutMs?: number
  /** Called for every text fragment, as it arrives. */
  onDelta?: (text: string) => void
  /** Called for every fragment of the model's thinking, when it streams one. */
  onReasoning?: (text: string) => void
  /**
   * Called with each `server_tool_event` the provider sends (`call_started`,
   * `call_completed`, `step_reasoning_done`): the search or page read is running
   * on its side, so this is the only progress there is.
   */
  onServerEvent?: (event: unknown) => void
}

/** Shape of a streaming chunk this module reads. */
interface Chunk {
  choices?: Array<{
    delta?: {
      content?: string | null
      /** DeepSeek's spelling of the thinking stream. */
      reasoning_content?: string | null
      /** The spelling other gateways use. */
      reasoning?: string | null
      tool_calls?: unknown
      /** Server tools the provider ran (web search, page reads). */
      server_tool_calls?: unknown
      /** Citations of the answer, once per search. */
      annotations?: unknown
    }
    /** `stop`, `length` or `content_filter`; absent while streaming. */
    finish_reason?: string | null
    /** Progress of a server tool, next to `delta` rather than inside it. */
    server_tool_event?: unknown
    /** Server tools the provider ran, when a gateway puts them here. */
    server_tool_calls?: unknown
    /** Citations of the answer, when a gateway puts them here. */
    annotations?: unknown
  }>
  usage?: unknown
  error?: { message?: string }
  message?: string
}

/** Shape of a whole JSON answer, when the provider does not stream. */
interface CompletionBody {
  choices?: Array<{
    message?: {
      content?: string | null
      reasoning_content?: string | null
      reasoning?: string | null
      tool_calls?: unknown
      /** Server tools the provider ran (web search, page reads). */
      server_tool_calls?: unknown
      /** Citations of the answer, once per search. */
      annotations?: unknown
    }
    finish_reason?: string | null
  }>
  usage?: unknown
  error?: { message?: string }
  message?: string
}

/** Tool calls being assembled, keyed by the provider's index. */
interface PartialCall {
  id: string
  name: string
  arguments: string
}

/** Read one value as a list; empty when it is not an array. */
function listOf(value: unknown): unknown[] {
  return Array.isArray(value) ? value : []
}

/** Read one value as a non-empty string, or `''`. */
function finishOf(value: unknown): string {
  return typeof value === 'string' ? value.trim() : ''
}

/**
 * Ask the model and report the answer as it is written.
 * @param request - endpoint, credentials and body.
 * @returns the round's text and any tool calls.
 */
export async function streamCompletion(request: StreamRequest): Promise<StreamResult> {
  const doFetch = request.fetchImpl ?? fetch
  const send = (body: unknown) =>
    doFetch(request.url, {
    method: 'POST',
    headers: {
      'content-type': 'application/json',
      authorization: `Bearer ${request.apiKey}`
    },
      body: JSON.stringify(body),
      signal: request.timeoutMs === undefined ? undefined : AbortSignal.timeout(request.timeoutMs)
    })

  let response = await sendWithRetry(request, send)

  if (!response.ok) {
    let detail = await failureDetail(response)
    // An endpoint that refuses the token-accounting field still has an answer
    // worth showing; ask again without it instead of failing the question. The
    // same goes for the server tools: a gateway that does not know them still
    // answers, just without search.
    const retry = withoutUsageRequest(request.body, detail) ?? withoutServerTools(request.body, detail)
    if (retry) {
      response = await send(retry)
      if (!response.ok) detail = await failureDetail(response)
    }
    if (!response.ok) throw new Error(`${request.label} 调用失败：${detail}`)
  }

  const type = response.headers?.get?.('content-type') ?? ''
  if (!type.includes('event-stream') || !response.body) {
    const raw = await response.text().catch(() => '')
    // A proxy can strip the stream's content type; the body still says `data:`.
    if (/^\s*data:/m.test(raw)) {
      const replay = new Response(raw, {
        headers: { 'content-type': 'text/event-stream' }
      })
      if (replay.body) return await readStream(replay.body, request)
    }
    const payload = parse<CompletionBody>(raw)
    const choice = payload?.choices?.[0]
    const message = choice?.message ?? {}
    return {
      content: typeof message.content === 'string' ? message.content : '',
      reasoning: thinkingOf(message),
      toolCalls: readToolCalls(message.tool_calls),
      usage: payload?.usage,
      serverCalls: listOf(message.server_tool_calls),
      annotations: listOf(message.annotations),
      finishReason: finishOf(choice?.finish_reason)
    }
  }

  return readStream(response.body, request)
}

/** Statuses worth another attempt: the request never produced an answer. */
const RETRY_STATUSES = new Set([408, 409, 425, 429, 500, 502, 503, 504])

/** How many times one round may be sent. */
const RETRY_ATTEMPTS = 3

/** The first backoff, in milliseconds; each later attempt waits once more. */
const RETRY_BASE_MS = 400

/**
 * The thinking text a message carries, in whichever spelling the provider used.
 * @param message - a whole message or one stream delta.
 * @returns the thinking text, or an empty string when there is none.
 */
function thinkingOf(message: {
  reasoning_content?: string | null
  reasoning?: string | null
}): string {
  if (typeof message.reasoning_content === 'string') return message.reasoning_content
  if (typeof message.reasoning === 'string') return message.reasoning
  return ''
}

/**
 * Send one round, retrying transient rejections.
 *
 * Only failures before the stream starts are retried: once fragments have been
 * handed to `onDelta`, sending the body again would repeat them.
 * @param request - the round being sent.
 * @param send - one attempt.
 * @returns the response that got through.
 */
async function sendWithRetry(
  request: StreamRequest,
  send: (body: unknown) => Promise<Response>
): Promise<Response> {
  for (let attempt = 1; ; attempt += 1) {
    try {
      const response = await send(request.body)
      if (!RETRY_STATUSES.has(response.status) || attempt >= RETRY_ATTEMPTS) return response
      // The body belongs to a failed attempt; release it before asking again.
      await response.body?.cancel().catch(() => undefined)
    } catch (error) {
      if (attempt >= RETRY_ATTEMPTS || !isTransient(error)) throw error
    }
    await pause(RETRY_BASE_MS * attempt)
  }
}

/** Wait between two attempts. */
function pause(milliseconds: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, milliseconds))
}

/**
 * Whether a fetch error looks like a network hiccup rather than a refusal.
 * @param error - what `fetch` threw.
 * @returns true for connection-level failures; false for aborts and timeouts.
 */
function isTransient(error: unknown): boolean {
  if (!(error instanceof Error)) return false
  // A timeout already used the whole window, and an abort was the user's.
  if (error.name === 'TimeoutError' || error.name === 'AbortError') return false
  if (error.name === 'TypeError') return true
  return /fetch failed|network|ECONN|ETIMEDOUT|EAI_AGAIN/i.test(error.message)
}

/** What a failed response says about itself. */
async function failureDetail(response: Response): Promise<string> {
  const payload = (await response.json().catch(() => null)) as CompletionBody | null
  return payload?.error?.message ?? payload?.message ?? `HTTP ${response.status}`
}

/**
 * The same body without the token-accounting request, for endpoints that
 * refuse fields they do not know. `undefined` when dropping it cannot help.
 * @param body - the body that was refused.
 * @param detail - what the endpoint said about it.
 * @returns a body to send again, or `undefined`.
 */
function withoutUsageRequest(body: unknown, detail: string): unknown {
  if (!body || typeof body !== 'object') return undefined
  const fields = body as Record<string, unknown>
  if (!('stream_options' in fields)) return undefined
  if (!/stream_options|include_usage/i.test(detail)) return undefined
  const retry = { ...fields }
  delete retry.stream_options
  return retry
}

/**
 * The same body without the server tools, for an endpoint that refuses them.
 *
 * Some gateways reject a `tools` entry whose `type` they do not know, and the
 * question is still answerable without search. The round is sent once more
 * without those entries instead of failing; no search row appears, which is the
 * truth of what happened.
 * @param body - the body that was refused.
 * @param detail - what the endpoint said about it.
 * @returns a body to send again, or `undefined`.
 */
function withoutServerTools(body: unknown, detail: string): unknown {
  if (!body || typeof body !== 'object') return undefined
  const fields = body as Record<string, unknown>
  const tools = fields.tools
  if (!Array.isArray(tools)) return undefined
  if (!/tool|type|unknown|unsupported|invalid|web_search|read_url/i.test(detail)) return undefined
  const kept = tools.filter((tool) => {
    const type = typeof (tool as { type?: unknown } | null)?.type === 'string'
      ? String((tool as { type?: unknown }).type)
      : ''
    return type !== 'deepseek_web_search' && type !== 'deepseek_read_url'
  })
  if (kept.length === tools.length) return undefined
  const retry = { ...fields }
  if (kept.length === 0) delete retry.tools
  else retry.tools = kept
  return retry
}

/**
 * Walk the event stream line by line.
 * @param body - the response body.
 * @param request - the original request, for the error prefix and `onDelta`.
 * @returns the round's text and tool calls.
 */
async function readStream(
  body: ReadableStream<Uint8Array>,
  request: StreamRequest
): Promise<StreamResult> {
  const reader = body.getReader()
  const decoder = new TextDecoder()
  const calls = new Map<number, PartialCall>()
  let buffer = ''
  let rawText = ''
  let content = ''
  let reasoning = ''
  let usage: unknown
  const serverCalls: unknown[] = []
  const annotations: unknown[] = []
  let finishReason = ''

  const result = (): StreamResult => ({
    content,
    reasoning,
    toolCalls: toWireCalls(calls),
    usage,
    serverCalls,
    annotations,
    finishReason
  })

  /** Fold one SSE line in; true when it was the `[DONE]` marker. */
  const consume = (line: string): boolean => {
    if (!line.startsWith('data:')) return false
    const data = line.slice(5).trim()
    if (data === '[DONE]') return true
    const chunk = parse<Chunk>(data)
    if (!chunk) return false
    if (chunk.error) {
      throw new Error(`${request.label} 调用失败：${chunk.error.message ?? '未知错误'}`)
    }
    if (chunk.usage) usage = chunk.usage
    const delta = chunk.choices?.[0]?.delta ?? {}
    if (typeof delta.content === 'string' && delta.content) {
      content += delta.content
      request.onDelta?.(delta.content)
    }
    // A reasoner streams its thinking before the answer; it is billed output,
    // so it is kept apart instead of being folded into the answer text.
    const thinking = thinkingOf(delta)
    if (thinking) {
      reasoning += thinking
      request.onReasoning?.(thinking)
    }
    collect(calls, delta.tool_calls)
    // Server tools run on the provider's side; the finished calls and the
    // citations arrive in the last chunk, before `[DONE]`. Some gateways put
    // them inside the delta, others beside it on the choice, so both are read.
    for (const call of listOf(delta.server_tool_calls ?? chunk.choices?.[0]?.server_tool_calls)) {
      serverCalls.push(call)
    }
    for (const note of listOf(delta.annotations ?? chunk.choices?.[0]?.annotations)) {
      annotations.push(note)
    }
    const finished = finishOf(chunk.choices?.[0]?.finish_reason)
    if (finished !== '') finishReason = finished
    const serverEvent = chunk.choices?.[0]?.server_tool_event
    if (serverEvent !== undefined && serverEvent !== null) request.onServerEvent?.(serverEvent)
    return false
  }

  for (;;) {
    const { value, done } = await reader.read()
    if (done) break
    const text = decoder.decode(value, { stream: true })
    buffer += text
    rawText += text

    let newline = buffer.indexOf('\n')
    while (newline >= 0) {
      const line = buffer.slice(0, newline).replace(/\r$/, '')
      buffer = buffer.slice(newline + 1)
      newline = buffer.indexOf('\n')

      if (consume(line)) {
        await reader.cancel().catch(() => undefined)
        return result()
      }
    }
  }
  // A proxy may end the stream without a final newline; that last event is
  // still the answer, so dropping it would lose the whole reply.
  if (buffer.trim() !== '') consume(buffer.replace(/\r$/, ''))
  rawText += decoder.decode()
  // The reverse mix-up: a body labelled as a stream that is one whole JSON
  // answer. Without this the round would look empty and the answer would throw.
  if (content === '' && calls.size === 0) {
    const whole = parse<CompletionBody>(rawText.trim())
    const choice = whole?.choices?.[0]
    const message = choice?.message
    if (message) {
      if (typeof message.content === 'string') content = message.content
      reasoning = thinkingOf(message)
      collect(calls, message.tool_calls)
      for (const call of listOf(message.server_tool_calls)) serverCalls.push(call)
      for (const note of listOf(message.annotations)) annotations.push(note)
    }
    finishReason = finishOf(choice?.finish_reason) || finishReason
    if (whole?.usage) usage = whole.usage
  }
  return result()
}

/**
 * Fold one chunk's tool-call fragments into the calls being assembled.
 *
 * Providers send a call in pieces: the id and the name once, then the argument
 * string character by character, so every field has to be appended.
 * @param calls - partial calls, keyed by index.
 * @param input - the chunk's `tool_calls` field.
 */
function collect(calls: Map<number, PartialCall>, input: unknown): void {
  if (!Array.isArray(input)) return
  input.forEach((raw, position) => {
    const entry = raw as {
      index?: number
      id?: string
      function?: { name?: string; arguments?: string }
    }
    const index = typeof entry?.index === 'number' ? entry.index : position
    const call = calls.get(index) ?? { id: '', name: '', arguments: '' }
    if (typeof entry?.id === 'string' && entry.id && call.id === '') call.id = entry.id
    // A name normally arrives once; a few gateways repeat the whole name in
    // every fragment, and appending it blindly would turn `read` into `readread`
    // (an unknown tool the model could never call again).
    const name = entry?.function?.name
    if (typeof name === 'string' && name) {
      if (call.name === '') call.name = name
      else if (call.name !== name) call.name += name
    }
    const args = entry?.function?.arguments
    if (typeof args === 'string') call.arguments += args
    // Some gateways send the arguments already parsed; keep the wire shape a
    // string so history and the tool loop stay uniform.
    else if (args !== null && typeof args === 'object') call.arguments = JSON.stringify(args)
    calls.set(index, call)
  })
}

/**
 * Read a complete `tool_calls` array, as a non-streaming provider sends it.
 * @param input - the message's `tool_calls` field.
 * @returns the calls, ready for the wire history.
 */
function readToolCalls(input: unknown): WireToolCall[] {
  const calls = new Map<number, PartialCall>()
  collect(calls, input)
  return toWireCalls(calls)
}

/**
 * Turn the assembled calls into the provider's own shape.
 * @param calls - partial calls, keyed by index.
 * @returns the calls in index order, unnamed entries dropped.
 */
function toWireCalls(calls: Map<number, PartialCall>): WireToolCall[] {
  return [...calls.entries()]
    .sort((left, right) => left[0] - right[0])
    .filter(([, call]) => call.name.trim().length > 0)
    .map(([index, call]) => ({
      id: call.id.trim() || `call_${index}`,
      type: 'function' as const,
      function: { name: call.name.trim(), arguments: call.arguments.trim() || '{}' }
    }))
}

/**
 * Parse one JSON line, `null` when it is not JSON.
 * @param data - the text after `data:`.
 * @returns the parsed value, or `null`.
 */
function parse<T>(data: string): T | null {
  try {
    return JSON.parse(data) as T
  } catch {
    return null
  }
}
