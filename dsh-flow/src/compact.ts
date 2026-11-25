/**
 * Multi-layer context compaction: prune -> summarize -> deterministic fallback.
 *
 * Every round resends the whole transcript, so a long question eventually runs
 * past the model's window. Three layers run in order of cost:
 *
 * 1. **prune** - rewrite each oversized tool result as head + marker + tail.
 *    Deterministic, no model call, `tool_call_id` pairing untouched.
 * 2. **summarize** - hand the oldest whole turns to the model and replace them
 *    with one `<compacted-summary>` checkpoint. The LLM fallback that keeps the
 *    work instead of throwing it away.
 * 3. **drop** - when the summarizer fails, replace that range with a note so the
 *    question still goes out with the newest turns intact.
 *
 * Pressure is measured with {@link estimateTokens}, the same CJK/ASCII rule the
 * window uses, so both sides agree on how full the window is.
 */

import { streamCompletion } from './stream.ts'

/** Head/middle/tail budgets for one oversized tool result. */
export interface PruneLimits {
  /** Results at or under this many characters are left alone. */
  thresholdChars: number
  /** Characters kept from the start. */
  headChars: number
  /** Characters kept from the end, where `read` puts its line count. */
  tailChars: number
}

/** Head/tail pruning for one oversized tool result. */
export const PRUNE_LIMITS: PruneLimits = {
  thresholdChars: 8_192,
  headChars: 4_096,
  tailChars: 1_024
}

/** What a pruned middle is replaced with. */
export const PRUNE_MARKER = '\n\n[... middle of tool result pruned ...]\n\n'

/** Pressure and retention budget of one compaction pass. */
export interface CompactLimits {
  /** Prompt tokens above which compaction runs; zero disables it. */
  pressureTokens: number
  /** Recent tokens always kept verbatim. */
  retainTokens: number
  /** Upper bound on one generated checkpoint, in tokens. */
  maxSummaryTokens: number
}

/**
 * Derive the budgets from the model window.
 *
 * 0.8 leaves a fifth of the window for this round's answer and tool results;
 * 0.16 is the recent tail that is never summarized, so a follow-up still has the
 * last few exchanges word for word.
 * @param windowTokens - the model window, from the window table.
 * @returns the budgets for one pass.
 */
export function compactLimits(windowTokens: number): CompactLimits {
  const window = Number.isFinite(windowTokens) && windowTokens > 0 ? windowTokens : 128_000
  return {
    pressureTokens: Math.round(window * 0.8),
    retainTokens: Math.round(window * 0.16),
    maxSummaryTokens: 8_192
  }
}

/**
 * Rough token count, matching the window estimate.
 *
 * A real tokenizer would be a large dependency for a number that only decides
 * when to compact. CJK characters count as one token each, ASCII as one per four.
 * @param text - the text to measure.
 * @returns the estimated tokens.
 */
export function estimateTokens(text: string): number {
  let tokens = 0
  let ascii = 0
  for (let index = 0; index < text.length; index += 1) {
    if (text.charCodeAt(index) < 128) {
      ascii += 1
    } else {
      tokens += Math.ceil(ascii / 4) + 1
      ascii = 0
    }
  }
  return tokens + Math.ceil(ascii / 4)
}

/** One wire message, as this module needs to see it. */
export interface CompactMessage {
  role: 'system' | 'user' | 'assistant' | 'tool'
  content: string | Array<Record<string, unknown>>
  tool_calls?: unknown
  tool_call_id?: string
}

/** Which layer changed the transcript. */
export type CompactLayer = 'none' | 'prune' | 'summary' | 'drop'

/** What one pass did, for the note the window shows. */
export interface CompactReport {
  /** The decisive layer; `none` when the transcript already fit. */
  layer: CompactLayer
  /** Estimated prompt tokens before the pass. */
  before: number
  /** Estimated prompt tokens after it. */
  after: number
  /** Tool results pruned or dropped. */
  pruned: number
  /** Messages the last layer replaced. */
  turns: number
  /** True when the replacement came from the model. */
  summarized: boolean
  /** True when the transcript changed at all, so the window annotates once. */
  changed: boolean
}

/** Slice by code point so a retained boundary never splits a surrogate pair. */
function slicePoints(text: string, start: number, end: number): string {
  return Array.from(text).slice(start, end).join('')
}

/** How many characters a message holds, ignoring non-text parts. */
function contentChars(content: string | Array<Record<string, unknown>>): number {
  if (typeof content === 'string') return content.length
  let chars = 0
  for (const part of content) {
    if (typeof part.text === 'string') chars += part.text.length
  }
  return chars
}

/** How many tokens a message costs; images and files get a fixed estimate. */
export function messageTokens(content: string | Array<Record<string, unknown>>): number {
  if (typeof content === 'string') return estimateTokens(content)
  let tokens = 0
  for (const part of content) {
    if (typeof part.text === 'string') tokens += estimateTokens(part.text)
    else if (part.type === 'image_url') tokens += 256
    else if (part.type === 'file') tokens += 512
  }
  return tokens
}

/**
 * How many tokens one wire message costs, its tool-call arguments included.
 *
 * The arguments of an assistant turn ride in `tool_calls`, not in `content`: a
 * single `write` may carry hundreds of thousands of characters, and every later
 * round resends them. Measuring only `content` made the pressure estimate blind
 * to the largest messages, so a transcript dominated by one such call never
 * triggered compaction and the endpoint kept refusing it.
 * @param message - the message to measure.
 * @returns estimated tokens.
 */
export function wireMessageTokens(message: CompactMessage): number {
  let tokens = messageTokens(message.content)
  if (message.tool_calls !== undefined) {
    try {
      tokens += estimateTokens(JSON.stringify(message.tool_calls))
    } catch {
      // A value that cannot be serialised never reaches the wire either.
    }
  }
  return tokens
}

/**
 * Measure a whole transcript.
 * @param messages - the messages that would be sent.
 * @returns estimated prompt tokens.
 */
export function transcriptTokens(messages: readonly CompactMessage[]): number {
  let tokens = 0
  for (const message of messages) tokens += wireMessageTokens(message)
  return tokens
}

/**
 * Layer 1: rewrite one oversized tool result as head + marker + tail.
 *
 * Both ends survive on purpose: the head is what the model asked for, and the
 * tail is where `read` puts its line count and where commands put their errors.
 * @param text - what the tool returned.
 * @param limits - character budgets; defaults to {@link PRUNE_LIMITS}.
 * @returns the rewritten text, or `text` itself when it already fits.
 */
export function pruneResult(text: string, limits: PruneLimits = PRUNE_LIMITS): string {
  if (text.length <= limits.thresholdChars) return text
  const total = Array.from(text).length
  const head = slicePoints(text, 0, Math.max(0, limits.headChars))
  const tail = slicePoints(text, Math.max(0, total - Math.max(0, limits.tailChars)), total)
  return `${head}${PRUNE_MARKER}${tail}`
}

/**
 * Layer 1 over a transcript.
 * @param messages - messages to rewrite in place.
 * @param limits - character budgets.
 * @returns how many results were rewritten.
 */
export function pruneToolResults(
  messages: CompactMessage[],
  limits: PruneLimits = PRUNE_LIMITS
): number {
  let pruned = 0
  for (const message of messages) {
    if (message.role !== 'tool' || typeof message.content !== 'string') continue
    const next = pruneResult(message.content, limits)
    if (next === message.content) continue
    message.content = next
    pruned += 1
  }
  return pruned
}

/**
 * The longest range of whole turns a summary may replace.
 *
 * The range starts after the system head, stops at the newest question, keeps
 * `retainTokens` of recent turns verbatim, and always ends at a `user` message,
 * so an assistant tool call is never separated from its results.
 * @param messages - the transcript.
 * @param limits - pressure and retention budgets.
 * @returns the index range `[start, end)`, or `null` when nothing is safe.
 */
export function selectSummaryRange(
  messages: readonly CompactMessage[],
  limits: CompactLimits
): { start: number; end: number } | null {
  let first = 0
  while (first < messages.length && messages[first]!.role === 'system') first += 1
  if (first >= messages.length - 1) return null

  // The newest question and everything after it is what this round is about.
  let newestQuestion = messages.length
  for (let index = messages.length - 1; index >= first; index -= 1) {
    if (messages[index]!.role === 'user') {
      newestQuestion = index
      break
    }
  }
  if (newestQuestion <= first) return null

  let kept = 0
  let keepFrom = newestQuestion
  for (let index = newestQuestion - 1; index >= first; index -= 1) {
    // Tool-call arguments count as well: a turn carrying one huge `write` is
    // exactly what the retained tail must not shelter from compaction.
    kept += wireMessageTokens(messages[index]!)
    keepFrom = index
    if (kept >= limits.retainTokens) break
  }
  // Back up to a turn boundary: only a user message may start the kept tail.
  while (keepFrom > first && messages[keepFrom]!.role !== 'user') keepFrom -= 1
  if (keepFrom <= first) return null
  return { start: first, end: keepFrom }
}

/** Opener of the checkpoint a summary lands in. */
export const CHECKPOINT_OPEN = '<compacted-summary>'

/** Closer of that checkpoint. */
export const CHECKPOINT_CLOSE = '</compacted-summary>'

/**
 * Framing that makes a checkpoint read as established background rather than a
 * new instruction.
 */
export const CHECKPOINT_PREAMBLE =
  'This is an automatically generated checkpoint condensing an earlier span of the conversation to free up context. Treat the captured context as established background and build on it without restating it. Continue the task directly from the messages that follow, without acknowledging this checkpoint.'

/** What replaces a range when the summarizer cannot run. */
export const DROPPED_RANGE_NOTE =
  '（更早的对话已省略以免占满上下文；需要细节时请问用户重述，或重新读取相关文件。）'

/** The directive handed to the model with the span to condense. */
export const SUMMARY_INSTRUCTION = [
  'Condense the conversation above into a structured checkpoint that lets another model resume the work with no loss of essential context.',
  '',
  'Output EXACTLY the Markdown structure below: keep every section, in order. Use terse bullets, not prose. Write "(none)" for an empty section - never drop a section.',
  '',
  '## Primary Request and Intent',
  '- [the original and evolving goals; quote verbatim where the wording matters]',
  '',
  '## Key Technical Concepts',
  '- [technologies, frameworks, patterns and conventions in play]',
  '',
  '## Files and Code',
  '- [exact path: why it matters, key changes or snippets]',
  '',
  '## Errors and Fixes',
  '- [error: how it was resolved, plus any related feedback]',
  '',
  '## Pending Jobs',
  '- [explicitly requested work not yet completed]',
  '',
  '## Current Work',
  '- [precisely what was in progress at this checkpoint]',
  '',
  '## Next Step',
  '- [the single next action, or "(none)"]',
  '',
  '## Critical Context',
  '- [decisions and rationale, constraints, preferences, open questions]',
  '',
  'Rules:',
  '- Preserve exact file paths, commands, error strings, identifiers, numbers, signatures and syntax fragments.',
  '- Capture user feedback and explicit instructions faithfully, especially corrections.',
  '- Do NOT mention this summarization request or that the context was compacted.',
  '- Output only the checkpoint text: do not call any tool.',
  `- If the span already contains a ${CHECKPOINT_OPEN} block, it is a PRIOR checkpoint: preserve still-true facts, drop stale ones, and merge newer information into one consolidated summary under the same structure.`,
  '- Reply in the language the conversation used.'
].join('\n')

/** How one span is summarized. */
export interface SummaryTarget {
  /** Endpoint base, without the request path. */
  baseUrl: string
  /** Key sent as the bearer token. */
  apiKey: string
  /** Model id, for the request and for error messages the caller shows. */
  model: string
  /** Request timeout, in milliseconds. */
  timeoutMs?: number
  /** Upper bound on the checkpoint length. */
  maxTokens: number
  /** Stub for tests. */
  fetchImpl?: typeof fetch
}

/** Pull the readable text of one message so the span can be replayed. */
function flatContent(content: string | Array<Record<string, unknown>>): string {
  if (typeof content === 'string') return content
  const parts: string[] = []
  for (const part of content) {
    if (typeof part.text === 'string') parts.push(part.text)
    else if (part.type === 'image_url') parts.push('[image]')
    else if (part.type === 'file') parts.push('[file]')
  }
  return parts.join('\n')
}

/**
 * Layer 2: ask the model for a checkpoint over the selected range.
 *
 * The range is replayed as one user message with the directive after it, so the
 * summarizer reads the same words the assistant did. A failure returns `null`;
 * the caller then drops the range instead, which is why a broken endpoint can
 * never fail the question.
 * @param messages - the transcript.
 * @param range - the range to condense.
 * @param target - endpoint, key and model for the auxiliary call.
 * @returns the checkpoint text, or `null` when the call failed.
 */
export async function summarizeRange(
  messages: readonly CompactMessage[],
  range: { start: number; end: number },
  target: SummaryTarget
): Promise<string | null> {
  const lines: string[] = []
  for (const message of messages.slice(range.start, range.end)) {
    if (message.role === 'system') continue
    const who =
      message.role === 'assistant' ? 'assistant' : message.role === 'tool' ? 'tool' : 'user'
    // The arguments a turn sent belong to the span too: without them a summary
    // cannot say what a `write` actually put on disk. The per-entry cap keeps
    // the auxiliary call bounded even so.
    const calls =
      message.tool_calls === undefined ? '' : `\n工具调用：${JSON.stringify(message.tool_calls)}`
    const raw = `${flatContent(message.content)}${calls}`.trim()
    if (raw === '') continue
    const body = pruneResult(raw, {
      thresholdChars: 2_000,
      headChars: 1_400,
      tailChars: 400
    })
    lines.push(`### ${who}\n${body.trim()}`)
  }
  if (lines.length === 0) return null
  const body: Record<string, unknown> = {
    model: target.model,
    messages: [{ role: 'user', content: `${lines.join('\n\n')}\n\n${SUMMARY_INSTRUCTION}` }],
    stream: true,
    stream_options: { include_usage: true },
    max_tokens: target.maxTokens
  }
  // The checkpoint has to fit its own budget. DeepSeek thinks by default, and
  // that thinking would spend the same output tokens without adding a word of
  // it; only DeepSeek is told, so a stricter gateway never sees the key.
  if (target.baseUrl.toLowerCase().includes('deepseek')) body.thinking = { type: 'disabled' }
  try {
    const answer = await streamCompletion({
      url: `${target.baseUrl}/chat/completions`,
      apiKey: target.apiKey,
      label: target.model,
      body,
      fetchImpl: target.fetchImpl,
      timeoutMs: target.timeoutMs
    })
    const text = answer.content.trim()
    return text === '' ? null : text
  } catch {
    return null
  }
}

/**
 * Replace a range with the checkpoint that condenses it.
 * @param messages - the transcript, rewritten in place.
 * @param range - the range to replace.
 * @param summary - checkpoint text.
 */
export function applyCheckpoint(
  messages: CompactMessage[],
  range: { start: number; end: number },
  summary: string
): void {
  messages.splice(range.start, range.end - range.start, {
    role: 'user',
    content: `${CHECKPOINT_PREAMBLE}\n\n${CHECKPOINT_OPEN}\n${summary.trim()}\n${CHECKPOINT_CLOSE}`
  })
}

/**
 * Layer 3: replace a range with a note when no checkpoint can be made.
 * @param messages - the transcript, rewritten in place.
 * @param range - the range to replace.
 * @param note - what replaces it.
 */
export function dropRange(
  messages: CompactMessage[],
  range: { start: number; end: number },
  note: string = DROPPED_RANGE_NOTE
): void {
  messages.splice(range.start, range.end - range.start, { role: 'user', content: note })
}

/** Everything one compaction pass needs. */
export interface CompactContext {
  /** Budgets derived from the model window. */
  limits: CompactLimits
  /** Endpoint and model for the LLM fallback. */
  target: SummaryTarget
  /**
   * Compact even below the pressure line. Used when the provider rejected a
   * request for being too long, where the estimate was plainly wrong.
   */
  force?: boolean
}

/**
 * Run the layers in order of cost until the transcript fits again.
 *
 * Pruning is free, so it always runs first; the summarizer only runs when the
 * transcript is still over the pressure line, and the drop note only when no
 * checkpoint could be made. Below the pressure line nothing is touched, so a
 * short question is never annotated or re-summarized.
 * @param messages - the transcript, rewritten in place.
 * @param context - budgets and the summarizer target.
 * @returns what the pass did.
 */
export async function compactTranscript(
  messages: CompactMessage[],
  context: CompactContext
): Promise<CompactReport> {
  const report: CompactReport = {
    layer: 'none',
    before: transcriptTokens(messages),
    after: 0,
    pruned: 0,
    turns: 0,
    summarized: false,
    changed: false
  }
  if (
    !context.force &&
    (context.limits.pressureTokens <= 0 || report.before <= context.limits.pressureTokens)
  ) {
    report.after = report.before
    return report
  }
  report.pruned = pruneToolResults(messages)
  if (report.pruned > 0) {
    report.layer = 'prune'
    report.changed = true
  }
  let tokens = transcriptTokens(messages)
  if (!context.force && tokens <= context.limits.pressureTokens) {
    report.after = tokens
    return report
  }
  const range = selectSummaryRange(messages, context.limits)
  if (range === null) {
    report.after = tokens
    return report
  }
  report.turns = range.end - range.start
  const summary = await summarizeRange(messages, range, context.target)
  if (summary !== null) {
    applyCheckpoint(messages, range, summary)
    report.layer = 'summary'
    report.summarized = true
  } else {
    dropRange(messages, range)
    report.layer = 'drop'
  }
  report.changed = true
  report.after = transcriptTokens(messages)
  return report
}

/**
 * Keep one tool result inside the model share of the context.
 *
 * Both ends survive: the head is what the model asked for, and the tail is where
 * `read` puts its line count and where commands put their errors.
 * @param text - what the tool returned.
 * @param limit - characters the message may hold.
 * @param note - marker placed where the middle was cut.
 * @returns the message text, unchanged when it already fits.
 */
export function fitForModel(
  text: string,
  limit: number,
  note = '\n…（结果过长，中间已省略；请缩小范围或用 offset / limit 重试）'
): string {
  if (text.length <= limit) return text
  const room = limit - note.length
  // A budget smaller than the note itself has no room for a tail; keeping the
  // head bounded is better than slicing with a negative length.
  if (room <= 0) return slicePoints(text, 0, Math.max(0, limit))
  const head = Math.ceil(room * 0.7)
  return `${slicePoints(text, 0, head)}${note}\n${slicePoints(text, Math.max(0, Array.from(text).length - (room - head)), Array.from(text).length)}`
}

/** Characters one message may hold before its middle is cut. */
export function clipChars(text: string, limit: number): string {
  if (contentChars(text) <= limit) return text
  return `${slicePoints(text, 0, limit)}\n…（已截断）`
}

/** Why the last layer ran, for the note the window prints. */
export function compactionNote(report: CompactReport): string {
  if (!report.changed) return ''
  const saved = Math.max(0, report.before - report.after)
  if (report.layer === 'summary') {
    return `上下文接近窗口上限，已把最早的 ${report.turns} 条消息压缩成摘要（约省 ${saved} tokens）。`
  }
  if (report.layer === 'drop') {
    return `上下文接近窗口上限，摘要调用失败，已省略最早的 ${report.turns} 条消息（约省 ${saved} tokens）。`
  }
  return `上下文接近窗口上限，已精简 ${report.pruned} 段过长的工具结果（约省 ${saved} tokens）。`
}
