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
  /** Tool calls the model asked for; empty when it answered in prose. */
  toolCalls: WireToolCall[]
  /** Token accounting, when the provider reports it. */
  usage?: unknown
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
}

/** Shape of a streaming chunk this module reads. */
interface Chunk {
  choices?: Array<{
    delta?: {
      content?: string | null
      tool_calls?: unknown
    }
  }>
  usage?: unknown
  error?: { message?: string }
  message?: string
}

/** Shape of a whole JSON answer, when the provider does not stream. */
interface CompletionBody {
  choices?: Array<{
    message?: { content?: string | null; tool_calls?: unknown }
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
    // worth showing; ask again without it instead of failing the question.
    const retry = withoutUsageRequest(request.body, detail)
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
    const message = payload?.choices?.[0]?.message ?? {}
    return {
      content: typeof message.content === 'string' ? message.content : '',
      toolCalls: readToolCalls(message.tool_calls),
      usage: payload?.usage
    }
  }

  return readStream(response.body, request)
}

/** Statuses worth another attempt: the request never produced an answer. */
const RETRY_STATUSES = new Set([408, 409, 425, 429, 500, 502, 503, 504])

/** How many times one round may be sent. */
const RETRY_ATTEMPTS = 3

/** First backoff, in milliseconds; each later attempt waits once more. */
const RETRY_BASE_MS = 400

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
  let usage: unknown

  const result = (): StreamResult => ({
    content,
    toolCalls: toWireCalls(calls),
    usage
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
    collect(calls, delta.tool_calls)
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
    const message = whole?.choices?.[0]?.message
    if (message) {
      if (typeof message.content === 'string') content = message.content
      collect(calls, message.tool_calls)
    }
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
