/**
 * DeepSeek's server tools: web search and URL reading.
 *
 * The model decides whether to go online; the search and the page reads run on
 * DeepSeek's servers, so nothing here executes a tool. The client declares the
 * tools in the request and then reads what the provider did from
 * `server_tool_calls` (with `annotations` for citations), turning it into the
 * same `step` the local tools produce so the transcript shows it without a
 * second event type.
 *
 * Field names below follow the published Chat Completions shape; everything is
 * read tolerantly, so a gateway that renames or flattens a field still yields a
 * usable row instead of an empty one.
 */

/** One server tool call the provider ran, kept raw for storage and replay. */
export type ServerCall = Record<string, unknown>

/** The declared tools that turn the server side on. */
export function serverToolDeclarations(): Array<Record<string, unknown>> {
  return [{ type: 'deepseek_web_search' }, { type: 'deepseek_read_url' }]
}

/** One row the transcript shows, shaped like an engine step. */
export interface WebStep {
  /** `web_search` for a search, `read_url` for a page read. */
  tool: string
  /** What was searched for, or which page was opened. */
  target: string
  /** False when the provider reported the call as failed. */
  ok: boolean
  /** One-line summary: how many sources came back, or the error code. */
  detail: string
  /** The queries or the URL, as JSON, matching a tool call's arguments. */
  input: string
  /** The sources or the page, one per line, for the expandable body. */
  output: string
  /** Stable id for this call, so a later event can replace the same row. */
  call: string
}

/** What the whole question's server tools add up to. */
export interface WebSearchReport {
  /** Searches the provider ran, `deep thinking` ones included. */
  searches: number
  /** Pages read, whether by a search or from a link in the question. */
  fetches: number
  /** The pages that came back, de-duplicated by URL, for a source list. */
  sources: Array<{ title: string; url: string }>
}

/** Read one value as a record, or nothing when it is not an object. */
function record(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null
}

/** Read one value as a string, trimmed; empty when it is not one. */
function text(value: unknown): string {
  return typeof value === 'string' ? value.trim() : ''
}

/** Read one value as a list; empty when it is not an array. */
function list(value: unknown): unknown[] {
  return Array.isArray(value) ? value : []
}

/** The search queries a call carried, or none. */
function queriesOf(call: Record<string, unknown>): string[] {
  const action = record(call.action) ?? record(call.input) ?? {}
  return list(action.queries ?? call.queries)
    .map(text)
    .filter((query) => query !== '')
}

/**
 * Expand one `server_tool_calls` entry into the calls it carries.
 *
 * The provider groups calls by thinking step: an entry is
 * `{ index, reasoning_content, calls: [ … ] }`, and only the inner objects have
 * `kind` / `action` / `sources`. Some gateways send the calls flat instead, so
 * both shapes are accepted.
 * @param entry - one raw entry of the array.
 * @returns the call objects, in order.
 */
function callsIn(entry: unknown): Record<string, unknown>[] {
  const outer = record(entry)
  if (!outer) return []
  const nested = list(outer.calls)
  if (nested.length === 0) return [outer]
  const flat: Record<string, unknown>[] = []
  for (const item of nested) {
    const call = record(item)
    if (call) flat.push(call)
  }
  return flat
}

/** Whether a call searched (any kind) rather than opened a page. */
function isSearch(call: Record<string, unknown>): boolean {
  // `action` is what actually happened: a deep-thinking step reports
  // `kind: deep_search` but may open a page, which is a read, not a search.
  const type = text(call.action && record(call.action)?.type)
  if (type === 'search' || type === 'image_search') return true
  if (type === 'open_page' || type === 'find_in_page' || type === 'read_link') return false
  const kind = text(call.kind)
  if (kind !== '') return kind.includes('search')
  return !text(call.type).toLowerCase().includes('read')
}

/** The URL a call opened: its action's target, or the page it read. */
function urlOf(call: Record<string, unknown>): string {
  const action = record(call.action) ?? {}
  return (
    text(action.url) ||
    text(call.url) ||
    text(record(call.page)?.url) ||
    queriesOf(call)[0] ||
    ''
  )
}

/** One source as the provider listed it, with whatever display fields it has. */
function sourceOf(value: unknown): { title: string; url: string } | null {
  if (typeof value === 'string') {
    const url = value.trim()
    return url === '' ? null : { title: url, url }
  }
  const entry = record(value)
  if (!entry) return null
  // Different gateways spell the two fields differently; any of these is a link
  // the transcript can show and open.
  const url = text(entry.url) || text(entry.link) || text(entry.href)
  const title = text(entry.title) || text(entry.name) || text(entry.text) || url
  return url === '' ? null : { title, url }
}

/** The sources one call returned, from whichever list the provider filled. */
function sourcesOf(call: Record<string, unknown>): Array<{ title: string; url: string }> {
  const action = record(call.action) ?? {}
  // "The results of a search" has been seen as `sources` in the Messages format
  // and as `results` in the Responses format; both are read, and either list may
  // sit on the call or beside its action.
  const lists = [call.sources, call.results, call.items, call.pages, action.sources, action.results]
  const sources: Array<{ title: string; url: string }> = []
  for (const batch of lists) {
    for (const entry of list(batch)) {
      const source = sourceOf(entry)
      if (source && !sources.some((seen) => seen.url === source.url)) sources.push(source)
    }
  }
  const page = sourceOf(record(call.page))
  if (page && !sources.some((source) => source.url === page.url)) sources.unshift(page)
  return sources
}

/** Whether the provider marked a call as failed. */
function failedOf(call: Record<string, unknown>): boolean {
  const status = text(call.status)
  if (status !== '' && status !== 'completed') return true
  return record(call.error) !== null && text(record(call.error)?.code) !== ''
}

/** The error code or message a failed call carried. */
function errorOf(call: Record<string, unknown>): string {
  const error = record(call.error)
  if (!error) return ''
  return text(error.code) || text(error.message)
}

/**
 * One live row for a search or page read that has just started.
 *
 * The provider tells us `call_started` before it searches; showing that as a
 * running tool row is the only progress a server-side tool can offer.
 */
export interface WebEventRow {
  /** Id the finished row will use, so the live row is replaced, not duplicated. */
  call: string
  /** `web_search` or `read_url`. */
  tool: string
  /** What is being searched for, or the page being opened. */
  target: string
}

/**
 * Read one `server_tool_event` into a live row.
 * @param event - the provider's event, as it arrived.
 * @param prefix - id prefix for this round.
 * @param index - 1-based position of the call within the round, matching the
 *   order the finished calls arrive in.
 * @returns the row, or `null` for events that are not a call starting.
 */
export function webEventRow(event: unknown, prefix: string, index: number): WebEventRow | null {
  const entry = record(event)
  if (!entry || text(entry.type) !== 'call_started') return null
  const outer = record(entry.call)
  if (!outer) return null
  // The event carries one call; a step-shaped payload is unwrapped to its first.
  const call = callsIn(outer)[0]
  if (!call) return null
  const search = isSearch(call)
  const queries = queriesOf(call)
  return {
    // The position in the round, not the provider's id: the finished calls may
    // or may not carry one, and the order is the only thing both events agree
    // on. This is what lets the finished row replace the running one instead of
    // leaving an empty row behind.
    call: `${prefix}-${index}`,
    tool: search ? 'web_search' : 'read_url',
    target: search ? queries.join(' · ') || '联网搜索' : urlOf(call) || '读取页面'
  }
}

/**
 * Turn the provider's server-tool calls into transcript rows.
 *
 * One row per call, plus one fallback row when only citations came back: a UI
 * that shows nothing after the model searched reads as if it never did.
 * @param calls - the raw `server_tool_calls` entries of one round.
 * @param annotations - the raw `annotations` entries of the same round.
 * @param prefix - id prefix for this round, so rows stay unique across rounds.
 * @returns the rows, oldest first.
 */
export function webSteps(
  calls: readonly unknown[],
  annotations: readonly unknown[],
  prefix: string
): WebStep[] {
  const steps: WebStep[] = []
  // Every entry may be a step holding several calls, or a call itself.
  const flat = calls.flatMap(callsIn)
  flat.forEach((call, index) => {
    const search = isSearch(call)
    const queries = queriesOf(call)
    const sources = sourcesOf(call)
    const failed = failedOf(call)
    const error = errorOf(call)
    const target = search ? queries.join(' · ') || '联网搜索' : urlOf(call) || '读取页面'
    const detail = failed
      ? error === ''
        ? '调用失败'
        : `失败：${error}`
      : search
        ? `${sources.length} 个来源`
        : sources.length > 0
          ? sources[0].title
          : '已读取'
    steps.push({
      tool: search ? 'web_search' : 'read_url',
      target,
      ok: !failed,
      detail,
      input: JSON.stringify(search ? { queries } : { url: urlOf(call) }),
      output: failed ? error : sources.map((source) => `${source.title} — ${source.url}`).join('\n'),
      call: `${prefix}-${index + 1}`
    })
  })
  if (steps.length > 0) return steps
  const cited = annotations
    .map((raw) => {
      const entry = record(raw)
      return entry && text(entry.type) === 'url_citation' ? sourceOf(entry) : null
    })
    .filter((source): source is { title: string; url: string } => source !== null)
  if (cited.length === 0) return steps
  steps.push({
    tool: 'web_search',
    target: '联网搜索',
    ok: true,
    detail: `${cited.length} 个来源`,
    input: '{}',
    output: cited.map((source) => `${source.title} — ${source.url}`).join('\n'),
    call: `${prefix}-citations`
  })
  return steps
}

/**
 * Add up what one question's server tools did.
 * @param calls - every round's `server_tool_calls`, oldest first.
 * @param annotations - every round's `annotations`.
 * @returns the counts and the de-duplicated source list.
 */
export function webReport(
  calls: readonly unknown[],
  annotations: readonly unknown[] = []
): WebSearchReport {
  let searches = 0
  let fetches = 0
  const sources: Array<{ title: string; url: string }> = []
  const seen = new Set<string>()
  const keep = (source: { title: string; url: string }): void => {
    if (seen.has(source.url)) return
    seen.add(source.url)
    sources.push(source)
  }
  for (const raw of calls) {
    for (const call of callsIn(raw)) {
      if (isSearch(call)) searches += 1
      else fetches += 1
      for (const source of sourcesOf(call)) keep(source)
    }
  }
  for (const raw of annotations) {
    const entry = record(raw)
    if (!entry || text(entry.type) !== 'url_citation') continue
    const source = sourceOf(entry)
    if (source) keep(source)
  }
  return { searches, fetches, sources }
}