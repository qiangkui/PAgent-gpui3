/**
 * Derive a workflow skeleton from a recorded dsh session.
 *
 * A session log holds what an assistant actually did: the commands it ran, the
 * tools it called, and the prose where it decided something. Only the commands
 * are turned into steps — they are the part that can be replayed exactly.
 * Judgment points and tool calls the runner cannot reproduce become `# TODO`
 * comments, so a human decides what to promote to an `ai:` step or a program.
 * Nothing is guessed: a derived manifest is a draft, never a finished workflow.
 */

/** How to name the derived workflow. */
export interface DeriveOptions {
  /** Kebab-case name; defaults to the session id. */
  name?: string
  /** One-line description; defaults to a sentence naming the session. */
  description?: string
}

/** What a derivation produced. */
export interface DeriveSummary {
  /** Session id from the log header, when present. */
  sessionId: string
  /** Working directory recorded in the header, when present. */
  cwd: string
  /** Number of `run` steps written. */
  runs: number
  /** Judgment points and tool calls left as TODO comments. */
  todos: string[]
  /** Later turns that were not derived, because one workflow is one run. */
  ignoredTurns: number
  /** Lines that were not a JSON event. */
  skippedLines: number
}

/** The generated manifest plus what it contains. */
export interface DeriveReport {
  /** Complete `flow.yml` text. */
  manifest: string
  /** Counts and TODO entries, for the CLI summary. */
  summary: DeriveSummary
}

/** One step written to the manifest. */
interface DerivedStep {
  /** Step id. */
  id: string
  /** What the step does, usually the prose that preceded the command. */
  desc: string
  /** Shell command line, or program plus arguments. */
  command: string | string[]
}

/** A session event, flattened out of its envelope. */
interface SessionEvent {
  /** Event type, for example `tool/call`. */
  type: string
  /** Event payload. */
  data: Record<string, unknown>
}

/** Session header fields this module reads. */
interface SessionHeader {
  /** Session id. */
  id?: string
  /** Working directory the session ran in. */
  cwd?: string
}

/**
 * Derive a workflow skeleton from session JSONL text.
 * @param source - the session file, one JSON record per line.
 * @param options - name and description overrides.
 * @returns the manifest text and a summary of what was kept or left as TODO.
 */
export function deriveWorkflow(source: string, options: DeriveOptions = {}): DeriveReport {
  const { header, events, skipped } = readSession(source)
  const sessionId = header.id ?? ''
  const name = options.name ?? slug(sessionId === '' ? 'derived-flow' : sessionId)
  const description = options.description ?? `从会话 ${sessionId || '(未知)'} 派生的骨架（待 review）`

  const steps: DerivedStep[] = []
  const todos: string[] = []
  const used = new Set<string>()
  let pending = ''
  let deriving = true
  let ignoredTurns = 0

  for (const event of events) {
    if (event.type === 'turn/start') {
      const turn = typeof event.data.turn === 'number' ? event.data.turn : 1
      if (turn > 1) {
        // One workflow is one run; a later question is a different flow until a
        // human says otherwise, so it is reported instead of guessed.
        deriving = false
        ignoredTurns += 1
      }
      continue
    }
    if (!deriving) continue

    if (event.type === 'assistant/message') {
      const text = messageText(event.data.message)
      if (text !== '') pending = text
      continue
    }
    if (event.type !== 'tool/call') continue

    const tool = text(event.data.name) || 'tool'
    const command = commandOf(event.data.arguments)
    if (command !== null) {
      steps.push({ id: idFor(tool, used), desc: headline(pending) || `运行 ${tool}`, command })
      pending = ''
      continue
    }
    const intent = pending === '' ? '' : `（原会话意图：${headline(pending, 100)}）`
    todos.push(`${tool}：${summariseArguments(event.data.arguments)}${intent}`)
    pending = ''
  }

  if (pending !== '') todos.push(`判断/结论：${headline(pending, 120)}`)

  const summary: DeriveSummary = {
    sessionId,
    cwd: header.cwd ?? '',
    runs: steps.length,
    todos,
    ignoredTurns,
    skippedLines: skipped,
  }
  return { manifest: buildManifest(name, description, steps, todos, summary), summary }
}

/**
 * Split the log into its header and events.
 *
 * Accepts both the wrapped record (`{type:"session_event", event:{…}}`) and a
 * bare event, because hand-made fixtures are often written the short way.
 * @param source - the session text.
 * @returns the header, the events, and how many lines could not be read.
 */
function readSession(source: string): { header: SessionHeader, events: SessionEvent[], skipped: number } {
  const header: SessionHeader = {}
  const events: SessionEvent[] = []
  let skipped = 0

  for (const raw of source.split(/\r?\n/)) {
    const line = raw.trim()
    if (line === '') continue
    let record: unknown
    try {
      record = JSON.parse(line)
    } catch {
      skipped += 1
      continue
    }
    if (!isRecord(record)) {
      skipped += 1
      continue
    }
    if (record.type === 'session') {
      const id = text(record.id)
      const cwd = text(record.cwd)
      if (id !== '') header.id = id
      if (cwd !== '') header.cwd = cwd
      continue
    }
    const event = isRecord(record.event) ? record.event : record
    if (typeof event.type !== 'string') {
      skipped += 1
      continue
    }
    events.push({ type: event.type, data: isRecord(event.data) ? event.data : {} })
  }

  return { header, events, skipped }
}

/**
 * Collect the text blocks of an assistant message.
 * @param message - the `assistant/message` payload.
 * @returns the prose, joined with newlines.
 */
function messageText(message: unknown): string {
  if (!isRecord(message)) return ''
  const content = message.content
  if (typeof content === 'string') return content.trim()
  if (!Array.isArray(content)) return ''
  const parts: string[] = []
  for (const block of content) {
    if (isRecord(block) && typeof block.text === 'string' && block.text.trim() !== '') parts.push(block.text.trim())
  }
  return parts.join('\n').trim()
}

/**
 * Read a tool call's command, when it carries one.
 * @param raw - the call's `arguments`, as a JSON string or an object.
 * @returns the command, or null when the call is not a command runner.
 */
function commandOf(raw: unknown): string | string[] | null {
  const args = parseArguments(raw)
  if (args === null) return null
  for (const key of ['command', 'cmd']) {
    const value = args[key]
    if (typeof value === 'string' && value.trim() !== '') return value
    if (Array.isArray(value) && value.length > 0 && value.every(entry => typeof entry === 'string')) {
      return value as string[]
    }
  }
  return null
}

/**
 * Parse a tool call's arguments.
 * @param raw - JSON text or an already parsed object.
 * @returns the object, or null.
 */
function parseArguments(raw: unknown): Record<string, unknown> | null {
  if (typeof raw === 'string') {
    const trimmed = raw.trim()
    if (trimmed === '') return null
    try {
      const parsed: unknown = JSON.parse(trimmed)
      return isRecord(parsed) ? parsed : null
    } catch {
      return null
    }
  }
  return isRecord(raw) ? raw : null
}

/**
 * One-line summary of the arguments a TODO points at.
 * @param raw - JSON text or an object.
 * @returns a short description.
 */
function summariseArguments(raw: unknown): string {
  const args = parseArguments(raw)
  if (args === null) return text(raw) || '(无参数)'
  const entries = Object.entries(args)
    .filter(([, value]) => typeof value === 'string' || typeof value === 'number' || typeof value === 'boolean')
    .slice(0, 3)
    .map(([key, value]) => `${key}=${String(value)}`)
  return entries.length === 0 ? '(无参数)' : headline(entries.join(', '), 100)
}

/**
 * First non-empty line of prose, shortened for a step description.
 * @param value - the prose.
 * @param limit - maximum characters.
 * @returns the line.
 */
function headline(value: string, limit = 72): string {
  const line = value.split(/\r?\n/).map(entry => entry.trim()).find(entry => entry !== '') ?? ''
  return line.length <= limit ? line : `${line.slice(0, limit - 1)}…`
}

/**
 * Make a step id out of a tool name, keeping it unique.
 * @param name - the tool name.
 * @param used - ids already taken.
 * @returns the id.
 */
function idFor(name: string, used: Set<string>): string {
  const base = slug(name) || 'step'
  let candidate = base
  let index = 2
  while (used.has(candidate)) {
    candidate = `${base}-${index}`
    index += 1
  }
  used.add(candidate)
  return candidate
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
 * Build the manifest text.
 * @param name - workflow name.
 * @param description - workflow description.
 * @param steps - replayable steps.
 * @param todos - what was deliberately not derived.
 * @param summary - counts for the header comment.
 * @returns the YAML.
 */
function buildManifest(
  name: string,
  description: string,
  steps: DerivedStep[],
  todos: string[],
  summary: DeriveSummary,
): string {
  const lines: string[] = []
  lines.push(`# 由 dsh 会话 ${summary.sessionId || '(未命名)'} 派生：只是骨架，review 之后再运行。`)
  lines.push(`# 原会话 cwd：${summary.cwd || '(未记录)'}`)
  lines.push(`# ${steps.length} 个命令已转成 run 步骤；${todos.length} 处保留为 TODO。`)
  if (summary.ignoredTurns > 0) {
    lines.push(`# 后 ${summary.ignoredTurns} 轮的提问没有派生：一条流程只跑一轮，需要的话另建清单。`)
  }
  if (summary.skippedLines > 0) lines.push(`# 跳过了 ${summary.skippedLines} 行无法解析的记录。`)
  lines.push('')
  lines.push(`name: ${quote(name)}`)
  lines.push(`description: ${quote(description)}`)
  lines.push('version: 1')
  lines.push('')
  lines.push('steps:')

  if (steps.length === 0) {
    lines.push('  # TODO: 这个会话里没有可自动重放的命令，按文件末尾的判断点手工补步骤。')
    lines.push('  - id: todo')
    lines.push('    set:')
    lines.push(`      todo: ${quote('TODO — 用真实步骤替换')}`)
  }

  for (const step of steps) {
    lines.push(`  - id: ${step.id}`)
    lines.push(`    desc: ${quote(step.desc)}`)
    if (typeof step.command === 'string') {
      const body = step.command.replace(/\r\n/g, '\n').split('\n')
      while (body.length > 0 && body[body.length - 1] === '') body.pop()
      if (body.length === 1) {
        lines.push(`    run: ${quote(body[0]!)}`)
      } else {
        lines.push('    run: |')
        for (const line of body) lines.push(line === '' ? '' : `      ${line}`)
      }
      continue
    }
    lines.push('    run:')
    for (const part of step.command) lines.push(`      - ${quote(part)}`)
  }

  if (todos.length > 0) {
    lines.push('')
    lines.push('# TODO（派生时刻意没有推断的部分）：')
    for (const todo of todos) lines.push(`#   - ${todo.replace(/\r?\n/g, ' ')}`)
  }
  return `${lines.join('\n')}\n`
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
 * Accept a string, ignoring anything else.
 * @param value - the value.
 * @returns the trimmed string, or an empty string.
 */
function text(value: unknown): string {
  return typeof value === 'string' ? value.trim() : ''
}

/**
 * Report whether a value is a plain object.
 * @param value - candidate.
 * @returns true for non-array objects.
 */
function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}
