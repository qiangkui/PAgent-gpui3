/**
 * Workflow execution.
 *
 * Steps run in order. A `run` step is one program invocation whose result is
 * recorded under `steps.<id>` and, when `save` is set, copied into `vars`. The
 * engine never retries a step whose previous attempt may have had side effects
 * unless the manifest asks for it.
 */

import { createHash, randomUUID } from 'node:crypto'
import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from 'node:fs'
import { dirname, join, resolve as resolvePath } from 'node:path'
import { createCliBackend, createHttpBackend } from './backend.ts'
import type { AiAsk, AiBackend } from './backend.ts'
import { runCommand, stepEnvironment } from './exec.ts'
import { evaluate } from './expr.ts'
import type { LoadedWorkflow } from './load.ts'
import { describeRuntime, resolveRuntime } from './runtime.ts'
import { extractJson, firstIssue } from './schema.ts'
import { startServices, stopServices } from './services.ts'
import type { RunningService } from './services.ts'
import { evaluateCondition, renderDeep, renderString } from './template.ts'
import type { AiSpec, MapItemResult, RunLevel, Scope, Step, StepResult, TraceRecord } from './types.ts'

/** Default per-command timeout. */
const DEFAULT_TIMEOUT_MS = 120_000

/** Default fan-out concurrency. */
const DEFAULT_CONCURRENCY = 4

/** One run's `once` entries, keyed by `workflow:key`. */
const ONCE_LEDGERS = new WeakMap<RunOptions, Record<string, string>>()

/**
 * The ledger entries for one run, loaded once and written back after success.
 * @param options - the run configuration.
 * @returns the entries plus a save function, or null when `once` is disabled.
 */
function onceLedgerFor(options: RunOptions): {
  entries: Record<string, string>
  save: () => void
} | null {
  if (!options.onceLedger) return null
  let entries = ONCE_LEDGERS.get(options)
  if (!entries) {
    entries = readOnceLedger(options.onceLedger)
    ONCE_LEDGERS.set(options, entries)
  }
  const path = options.onceLedger
  return { entries, save: () => writeOnceLedger(path, entries) }
}

/** Read the ledger; a missing or broken file means nothing has run. */
function readOnceLedger(path: string): Record<string, string> {
  try {
    const value = JSON.parse(readFileSync(path, 'utf8')) as { entries?: unknown }
    const entries = value.entries
    if (!entries || typeof entries !== 'object' || Array.isArray(entries)) return {}
    const kept: Record<string, string> = {}
    for (const [key, at] of Object.entries(entries)) {
      if (typeof at === 'string') kept[key] = at
    }
    return kept
  } catch {
    return {}
  }
}

/** Replace the ledger in one step, so a crash never leaves half a file. */
function writeOnceLedger(path: string, entries: Record<string, string>): void {
  mkdirSync(dirname(path), { recursive: true })
  const temporary = `${path}.tmp`
  writeFileSync(temporary, JSON.stringify({ version: 1, entries }, null, 2) + '\n', 'utf8')
  renameSync(temporary, path)
}

/** The key a `once` step is remembered under, or null when it always runs. */
function onceKey(step: Step): string | null {
  if (step.once === undefined || step.once === false) return null
  return typeof step.once === 'string' ? step.once : step.id
}

/** How a run is configured. */
export interface RunOptions {
  /** The loaded manifest. */
  loaded: LoadedWorkflow
  /** Raw `--input name=value` pairs from the caller. */
  inputs: Record<string, string>
  /** Render steps without executing them. */
  dryRun?: boolean
  /** How much of the run may call a model. Defaults to `assist`. */
  level?: RunLevel
  /** Backend override, used by contract tests to keep them offline. */
  backend?: AiBackend
  /** Read and write recorded AI decisions. Defaults to true. */
  cache?: boolean
  /**
   * Ledger holding `once` entries, usually `<manifest dir>/.dsh-flow/ledger.json`.
   * Without it, `once` steps always run.
   */
  onceLedger?: string
  /** Ignore the ledger and run `once` steps again, refreshing their entries. */
  forceOnce?: boolean
  /** Endpoint credentials for HTTP backends, layered over the environment. */
  aiEnv?: Record<string, string>
  /** Directory holding recorded AI decisions. Defaults to `<manifest>/.dsh-flow/answers`. */
  answersDir?: string
  /** Progress sink; receives one line per step. */
  log: (line: string) => void
  /** Trace sink; receives one record per event. */
  trace?: (record: TraceRecord) => void
}

/** The settled state of one run. */
export interface RunSummary {
  /** Identifier shared with the trace file. */
  runId: string
  /** Whether every non-tolerant step succeeded. */
  status: 'ok' | 'failed'
  /** Recorded step results, keyed by step id. */
  steps: Record<string, StepResult>
  /** Final variables. */
  vars: Record<string, unknown>
  /** Wall-clock duration. */
  durationMs: number
  /** The step that stopped the run, when it failed. */
  failedStep?: string
  /** Failure message, when the run did not complete. */
  error?: string
}

/** Thrown internally when a non-tolerant step fails. */
class StepFailure extends Error {
  readonly stepId: string

  /**
   * @param stepId - the failing step.
   * @param message - the failure message.
   */
  constructor(stepId: string, message: string) {
    super(message)
    this.stepId = stepId
    this.name = 'StepFailure'
  }
}

/**
 * Resolve declared inputs, applying defaults and rejecting missing required ones.
 * @param loaded - the loaded manifest.
 * @param supplied - raw caller values.
 * @returns the resolved inputs.
 */
export function resolveInputs(loaded: LoadedWorkflow, supplied: Record<string, string>): Record<string, unknown> {
  const resolved: Record<string, unknown> = {}
  const declared = loaded.workflow.inputs
  for (const name of Object.keys(supplied)) {
    if (!(name in declared)) {
      throw new Error(`unknown input "${name}" (declared: ${Object.keys(declared).join(', ') || 'none'})`)
    }
  }
  for (const [name, spec] of Object.entries(declared)) {
    const provided = supplied[name]
    if (provided !== undefined) {
      resolved[name] = provided
      continue
    }
    if (spec.default !== undefined) {
      resolved[name] = spec.default
      continue
    }
    if (spec.required) throw new Error(`missing required input "${name}" (pass --input ${name}=...)`)
    resolved[name] = undefined
  }
  return resolved
}

/**
 * Execute a whole workflow.
 * @param options - run configuration.
 * @returns the settled summary; a step failure is reported in the summary, not thrown.
 */
export async function runWorkflow(options: RunOptions): Promise<RunSummary> {
  const startedAt = Date.now()
  const runId = `${new Date().toISOString().replaceAll(':', '-')}-${randomUUID().slice(0, 8)}`
  const inputs = resolveInputs(options.loaded, options.inputs)
  const scope: Scope = {
    inputs,
    vars: { ...options.loaded.workflow.vars },
    steps: {},
    env: process.env,
  }

  options.trace?.({ ts: new Date().toISOString(), type: 'run/start', detail: options.loaded.workflow.name })

  const running: RunningService[] = []
  let serviceError = ''
  let failure: StepFailure | null = null
  try {
    try {
      running.push(
        ...(await startServices({
          services: options.loaded.workflow.services ?? {},
          defaults: options.loaded.workflow.defaults,
          scope,
          dir: options.loaded.dir,
          log: options.log,
          trace: options.trace,
          dryRun: options.dryRun,
        }))
      )
    } catch (error) {
      serviceError = messageOf(error)
    }
    if (serviceError === '') {
      try {
        await executeSteps(options.loaded.workflow.steps, scope, options)
      } catch (error) {
        if (!(error instanceof StepFailure)) throw error
        failure = error
      }
    }
  } finally {
    await stopServices(running, { trace: options.trace, log: options.log })
  }

  const error = serviceError !== '' ? serviceError : failure?.message
  options.trace?.({
    ts: new Date().toISOString(),
    type: 'run/end',
    detail: error ?? 'ok',
  })

  return {
    runId,
    status: error === undefined ? 'ok' : 'failed',
    steps: scope.steps,
    vars: scope.vars,
    durationMs: Date.now() - startedAt,
    failedStep: failure?.stepId,
    error,
  }
}

/**
 * Execute one step list against a scope.
 * @param steps - the steps to run.
 * @param scope - the live scope mutated as steps settle.
 * @param options - run configuration.
 */
async function executeSteps(steps: Step[], scope: Scope, options: RunOptions): Promise<void> {
  for (const step of steps) {
    const result = await executeStep(step, scope, options)
    scope.steps[step.id] = result
    reportStep(step, result, options)
    if (result.status === 'failed' && !step.allowFailure) throw new StepFailure(step.id, result.error ?? 'step failed')
  }
}

/**
 * Execute one step with retries and its `when` gate.
 * @param step - the step to run.
 * @param scope - the live scope.
 * @param options - run configuration.
 * @returns the recorded result.
 */
async function executeStep(step: Step, scope: Scope, options: RunOptions): Promise<StepResult> {
  const startedAt = Date.now()
  const emit = (record: TraceRecord): void => options.trace?.(record)
  emit({ ts: new Date().toISOString(), type: 'step/start', id: step.id, kind: kindOf(step) })

  if (step.skip) return { id: step.id, kind: kindOf(step), status: 'skipped', durationMs: 0, error: 'skipped (skip: true)' }

  const key = onceKey(step)
  const ledger = key === null ? null : onceLedgerFor(options)
  const entry = key === null ? '' : `${options.loaded.workflow.name}:${key}`
  if (
    ledger !== null &&
    entry !== '' &&
    !options.forceOnce &&
    !options.dryRun &&
    ledger.entries[entry] !== undefined
  ) {
    return {
      id: step.id,
      kind: kindOf(step),
      status: 'skipped',
      durationMs: 0,
      error: `already ran (once: ${key})`
    }
  }

  if (step.when !== undefined) {
    let shouldRun: boolean
    try {
      shouldRun = evaluateCondition(step.when, scope)
    } catch (error) {
      return failureResult(step, startedAt, messageOf(error))
    }
    if (!shouldRun) {
      return { id: step.id, kind: kindOf(step), status: 'skipped', durationMs: 0, error: `condition false: ${step.when}` }
    }
  }

  const attempts = (step.retries ?? 0) + 1
  let result = failureResult(step, startedAt, 'not executed')
  for (let attempt = 1; attempt <= attempts; attempt += 1) {
    result = await executeOnce(step, scope, options)
    if (result.status !== 'failed') break
    if (attempt < attempts) {
      const delay = step.retryDelayMs ?? 250
      options.log(`  retry ${attempt + 1}/${attempts} for "${step.id}" in ${delay}ms`)
      await sleep(delay)
    }
  }

  if (step.save !== undefined && result.status === 'ok') {
    scope.vars[step.save] = primaryValue(result)
  }
  if (ledger !== null && entry !== '' && result.status === 'ok' && !options.dryRun) {
    ledger.entries[entry] = new Date().toISOString()
    ledger.save()
  }
  return result
}

/**
 * Print and trace a settled step.
 * @param step - the executed step.
 * @param result - its result.
 * @param options - run configuration.
 */
function reportStep(step: Step, result: StepResult, options: RunOptions): void {
  options.trace?.({
    ts: new Date().toISOString(),
    type: 'step/end',
    id: step.id,
    kind: result.kind,
    status: result.status,
    durationMs: result.durationMs,
    command: result.command,
    detail: result.error,
  })
  const marker = result.status === 'ok' ? 'OK  ' : result.status === 'skipped' ? 'SKIP' : 'FAIL'
  const detail = result.status === 'ok'
    ? result.kind === 'ai'
      ? `${result.durationMs}ms · ${result.model ?? 'model'}${result.cached === true ? ' · 复用记录' : ''}`
      : `${result.durationMs}ms`
    : result.error ?? ''
  options.log(`[${marker}] ${step.id}${step.desc ? ` — ${step.desc}` : ''} (${detail})`)
  if (result.status === 'ok' && result.kind === 'run' && result.stderr !== undefined && result.stderr.trim() !== '') {
    for (const line of result.stderr.trimEnd().split(/\r?\n/).slice(0, 20)) options.log(`       | ${line}`)
  }
  if (result.status === 'failed' && result.kind === 'run' && result.stderr !== undefined && result.stderr.trim() !== '') {
    for (const line of result.stderr.trimEnd().split(/\r?\n/).slice(0, 20)) options.log(`       ! ${line}`)
  }
}

/**
 * Execute one attempt of a step.
 * @param step - the step to run.
 * @param scope - the live scope.
 * @param options - run configuration.
 * @returns the attempt's result.
 */
async function executeOnce(step: Step, scope: Scope, options: RunOptions): Promise<StepResult> {
  const startedAt = Date.now()
  try {
    if (step.run !== undefined) return await executeRun(step, scope, options, startedAt)
    if (step.set !== undefined) return executeSet(step, scope, startedAt)
    if (step.ai !== undefined) return await executeAi(step, scope, options, startedAt)
    return await executeMap(step, scope, options, startedAt)
  } catch (error) {
    return failureResult(step, startedAt, messageOf(error))
  }
}

/**
 * Run the external program of a `run` step.
 * @param step - the step to run.
 * @param scope - the live scope.
 * @param options - run configuration.
 * @param startedAt - attempt start time.
 * @returns the attempt's result.
 */
async function executeRun(step: Step, scope: Scope, options: RunOptions, startedAt: number): Promise<StepResult> {
  const spec = step.run!
  const defaults = options.loaded.workflow.defaults
  const rendered = renderDeep(spec, scope) as typeof spec
  const parts = Array.isArray(rendered.command) ? rendered.command.slice() : [rendered.command]
  const commandLine = parts.join(' ')
  if (options.dryRun) {
    return {
      id: step.id,
      kind: 'run',
      status: 'skipped',
      durationMs: Date.now() - startedAt,
      command: commandLine,
      error: 'dry run',
    }
  }
  const resolvedFirst = rendered.shell ? null : resolveRuntimeName(parts[0] ?? '')
  if (!rendered.shell && resolvedFirst === null) throw new Error(describeRuntime(parts[0] ?? ''))
  // A shell command is one command line: the shell owns the tokenizing, so the
  // first token must not be resolved as a program.
  const file = rendered.shell ? commandLine : resolvedFirst!.file
  const args = rendered.shell ? [] : [...resolvedFirst!.prefixArgs, ...parts.slice(1)]

  const cwd = resolvePath(options.loaded.dir, rendered.cwd ?? defaults.cwd ?? '.')
  const timeoutMs = rendered.timeoutMs ?? defaults.timeoutMs ?? DEFAULT_TIMEOUT_MS
  const env = stepEnvironment({ ...defaults.env, ...rendered.env })
  const stdin = `${JSON.stringify({
    inputs: scope.inputs,
    vars: scope.vars,
    item: scope.item ?? null,
    index: scope.index ?? null,
    steps: scope.steps,
  }, null, 2)}\n`

  const outcome = await runCommand({ file, args, cwd, env, timeoutMs, stdin, shell: rendered.shell })
  if (outcome.spawnError !== undefined) throw new Error(`cannot start "${commandLine}": ${outcome.spawnError}`)

  const parsed = rendered.textOnly ? undefined : tryParseJson(outcome.stdout)
  const result: StepResult = {
    id: step.id,
    kind: 'run',
    status: outcome.exitCode === 0 ? 'ok' : 'failed',
    exitCode: outcome.exitCode,
    stdout: outcome.stdout,
    stderr: outcome.stderr,
    json: parsed,
    durationMs: Date.now() - startedAt,
    truncated: outcome.truncated,
    command: commandLine,
  }
  if (result.status === 'failed') {
    result.error = `exit code ${outcome.exitCode}${outcome.stderr.trim() === '' ? '' : `: ${firstLine(outcome.stderr)}`}`
  }
  return result
}

/**
 * Apply a `set` step.
 * @param step - the step to apply.
 * @param scope - the live scope.
 * @param startedAt - attempt start time.
 * @returns the attempt's result.
 */
function executeSet(step: Step, scope: Scope, startedAt: number): StepResult {
  const assigned: Record<string, unknown> = {}
  for (const [key, value] of Object.entries(step.set ?? {})) {
    const rendered = renderDeep(value, scope)
    assigned[key] = rendered
    scope.vars[key] = rendered
  }
  return {
    id: step.id,
    kind: 'set',
    status: 'ok',
    durationMs: Date.now() - startedAt,
    json: assigned,
  }
}

/**
 * Ask a model one question and record the answer under `steps.<id>`.
 *
 * The level decides who answers:
 * - `manual` never calls a model; it replays a recorded decision and fails
 *   loudly when there is none.
 * - `assist` replays a recorded decision and only asks the model when the
 *   prompt changed, so a settled workflow costs zero tokens.
 * - `draft` always asks, which is what a new or edited prompt needs.
 * @param step - the step to run.
 * @param scope - the live scope.
 * @param options - run configuration.
 * @param startedAt - attempt start time.
 * @returns the attempt's result.
 */
async function executeAi(step: Step, scope: Scope, options: RunOptions, startedAt: number): Promise<StepResult> {
  const spec = step.ai!
  const level: RunLevel = options.level ?? 'assist'
  const ask = buildAsk(spec, scope)
  if (options.dryRun) {
    return {
      id: step.id,
      kind: 'ai',
      status: 'skipped',
      durationMs: Date.now() - startedAt,
      error: 'dry run',
    }
  }

  const key = answerKey(ask, spec.backend ?? 'http')
  const file = answerFile(options, step.id, key)
  const cache = options.cache !== false
  if (cache && level !== 'draft') {
    const recorded = readAnswer(file, key)
    if (recorded !== null) {
      // A recorded answer is only reusable while it still satisfies the
      // contract; a schema edit must not keep an old shape alive.
      const settled = parseAnswer(spec, recorded.text)
      if (settled.ok) {
        return {
          id: step.id,
          kind: 'ai',
          status: 'ok',
          text: recorded.text,
          json: settled.json,
          model: recorded.model,
          usage: recorded.usage,
          cached: true,
          durationMs: Date.now() - startedAt,
        }
      }
      options.log(`  warning: "${step.id}" 的记录已不符合 schema（${settled.error}），需要重新生成`)
    }
  }

  if (level === 'manual') {
    throw new Error(
      `manual 档不调用模型："${step.id}" 没有可复用的判断结果（${relativePath(options.loaded.dir, file)}）。` +
      '先跑一次 --level assist 记录，或临时用 --level draft',
    )
  }

  const backend = options.backend ?? backendFor(spec, options)
  const first = await backend.ask(withSchema(ask))
  let reply = first
  let settled = parseAnswer(spec, first.text)
  if (!settled.ok) {
    options.log(`  ai "${step.id}" 的回答不符合约定（${settled.error}），重问一次`)
    reply = await backend.ask(withSchema({ ...ask, prompt: repairPrompt(ask.prompt, first.text, settled.error) }))
    const repaired = parseAnswer(spec, reply.text)
    if (!repaired.ok) throw new Error(`ai 步骤 "${step.id}" 的回答仍不符合约定：${repaired.error}`)
    settled = repaired
  }

  if (cache) {
    writeAnswer(options, file, {
      key,
      stepId: step.id,
      at: new Date().toISOString(),
      model: reply.model,
      text: reply.text,
      usage: reply.usage,
    })
  }

  return {
    id: step.id,
    kind: 'ai',
    status: 'ok',
    text: reply.text,
    json: settled.json,
    model: reply.model,
    usage: reply.usage,
    durationMs: Date.now() - startedAt,
  }
}

/**
 * Render the prompt and system message of an `ai` step.
 * @param spec - the step's model-call spec.
 * @param scope - the live scope.
 * @returns the question to send.
 */
function buildAsk(spec: AiSpec, scope: Scope): AiAsk {
  return {
    prompt: renderString(spec.prompt, scope),
    system: spec.system === undefined ? undefined : renderString(spec.system, scope),
    model: spec.model,
    temperature: spec.temperature,
    maxTokens: spec.maxTokens,
    schema: spec.schema,
  }
}

/**
 * Add the schema to the prompt, since a model cannot guess a shape.
 * @param ask - the question.
 * @returns the question with the schema appended.
 */
function withSchema(ask: AiAsk): AiAsk {
  if (ask.schema === undefined) return ask
  return {
    ...ask,
    prompt: `${ask.prompt}\n\n输出必须是一个 JSON 值，并符合下面的 JSON Schema（只输出 JSON，不要解释，不要 Markdown 代码块）：\n${JSON.stringify(ask.schema, null, 2)}`,
  }
}

/**
 * Build the one repair question allowed per step.
 * @param prompt - the original question.
 * @param invalid - what the model answered first.
 * @param error - why that answer was rejected.
 * @returns the repair question.
 */
function repairPrompt(prompt: string, invalid: string, error: string): string {
  return `${prompt}\n\n---\n上一次的回答是：\n${clip(invalid, 2000)}\n\n它不符合要求：${error}\n请只输出符合要求的结果。`
}

/**
 * Interpret a model answer against the step's contract.
 * @param spec - the step's model-call spec.
 * @param text - the answer text.
 * @returns the parsed value, or why it cannot be used.
 */
function parseAnswer(spec: AiSpec, text: string): { ok: true, json?: unknown } | { ok: false, error: string } {
  if (spec.schema === undefined) return { ok: true, json: tryParseJson(text) }
  const parsed = extractJson(text)
  if (!parsed.ok) return { ok: false, error: `回答不是 JSON（${parsed.error}）` }
  const issue = firstIssue(spec.schema, parsed.value)
  if (issue !== null) return { ok: false, error: issue }
  return { ok: true, json: parsed.value }
}

/**
 * Identify a decision by everything that can change it.
 * @param ask - the rendered question.
 * @param backend - the backend that would answer.
 * @returns a stable short key.
 */
function answerKey(ask: AiAsk, backend: string): string {
  const material = JSON.stringify([
    backend,
    ask.prompt,
    ask.system ?? '',
    ask.schema ?? null,
    ask.model ?? '',
    ask.temperature ?? null,
    ask.maxTokens ?? null,
  ])
  return createHash('sha256').update(material).digest('hex').slice(0, 16)
}

/**
 * Path of one recorded decision.
 * @param options - run configuration.
 * @param stepId - owning step.
 * @param key - decision key.
 * @returns the absolute file path.
 */
function answerFile(options: RunOptions, stepId: string, key: string): string {
  const root = options.answersDir ?? join(options.loaded.dir, '.dsh-flow', 'answers')
  return join(root, stepId, `${key}.json`)
}

/** One recorded decision on disk. */
interface RecordedAnswer {
  /** Decision key the record was written for. */
  key: string
  /** Step that asked. */
  stepId: string
  /** ISO timestamp. */
  at: string
  /** Model that answered. */
  model: string
  /** The answer text. */
  text: string
  /** Token accounting, when the provider reported it. */
  usage?: unknown
}

/**
 * Read a recorded decision.
 * @param file - the record path.
 * @param key - expected decision key.
 * @returns the record, or null when absent or written for another prompt.
 */
function readAnswer(file: string, key: string): RecordedAnswer | null {
  if (!existsSync(file)) return null
  try {
    const parsed = JSON.parse(readFileSync(file, 'utf8')) as Partial<RecordedAnswer>
    if (parsed.key !== key || typeof parsed.text !== 'string') return null
    return {
      key,
      stepId: String(parsed.stepId ?? ''),
      at: String(parsed.at ?? ''),
      model: String(parsed.model ?? 'model'),
      text: parsed.text,
      usage: parsed.usage,
    }
  } catch {
    return null
  }
}

/**
 * Store a decision so the next run can replay it.
 * @param options - run configuration.
 * @param file - the record path.
 * @param record - what to write.
 */
function writeAnswer(options: RunOptions, file: string, record: RecordedAnswer): void {
  try {
    mkdirSync(dirname(file), { recursive: true })
    writeFileSync(file, `${JSON.stringify(record, null, 2)}\n`, 'utf8')
  } catch (error) {
    options.log(`  warning: 无法写入判断记录 ${file}：${messageOf(error)}`)
  }
}

/**
 * Pick the backend a step asked for.
 * @param spec - the step's model-call spec.
 * @param options - run configuration.
 * @returns the backend.
 */
function backendFor(spec: AiSpec, options: RunOptions): AiBackend {
  if (spec.backend === 'cli' && spec.command !== undefined) {
    return createCliBackend(spec.command, { cwd: options.loaded.dir, timeoutMs: spec.timeoutMs })
  }
  const env = options.aiEnv === undefined ? undefined : { ...process.env, ...options.aiEnv }
  return createHttpBackend({ timeoutMs: spec.timeoutMs, env })
}

/**
 * Render a path relative to the manifest directory for messages.
 * @param base - the manifest directory.
 * @param path - the absolute path.
 * @returns the relative path with forward slashes.
 */
function relativePath(base: string, path: string): string {
  const normalizedBase = base.endsWith('\\') || base.endsWith('/') ? base : `${base}${process.platform === 'win32' ? '\\' : '/'}`
  return (path.startsWith(normalizedBase) ? path.slice(normalizedBase.length) : path).replaceAll('\\', '/')
}

/**
 * Cap long text kept in messages.
 * @param text - the text.
 * @param limit - maximum characters.
 * @returns the text, truncated when too long.
 */
function clip(text: string, limit: number): string {
  return text.length <= limit ? text : `${text.slice(0, limit)}\n…（已截断）`
}

/**
 * Run the fan-out of a `map` step.
 * @param step - the step to run.
 * @param scope - the live scope.
 * @param options - run configuration.
 * @param startedAt - attempt start time.
 * @returns the attempt's result.
 */
async function executeMap(step: Step, scope: Scope, options: RunOptions, startedAt: number): Promise<StepResult> {
  const spec = step.map!
  const over = evaluate(spec.over, scope)
  if (!Array.isArray(over)) {
    throw new Error(`map "${step.id}": "over" must evaluate to a list, got ${typeName(over)}`)
  }
  const binding = spec.as ?? 'item'
  const concurrency = Math.max(1, spec.concurrency ?? DEFAULT_CONCURRENCY)
  const items: MapItemResult[] = new Array(over.length)
  let cursor = 0
  const worker = async (): Promise<void> => {
    for (;;) {
      const index = cursor
      cursor += 1
      if (index >= over.length) return
      const item = over[index]
      const childScope: Scope = {
        ...scope,
        vars: { ...scope.vars, [binding]: item, index },
        steps: {},
        item,
        index,
        bindings: { ...scope.bindings, [binding]: item },
      }
      let ok = true
      for (const nested of spec.steps) {
        const result = await executeStep(nested, childScope, options)
        childScope.steps[nested.id] = result
        if (result.status === 'failed') {
          ok = false
          if (!nested.allowFailure) break
        }
      }
      items[index] = { index, item, steps: childScope.steps, ok }
    }
  }
  await Promise.all(Array.from({ length: Math.min(concurrency, over.length) }, () => worker()))

  const failed = items.filter(entry => entry !== undefined && !entry.ok).length
  return {
    id: step.id,
    kind: 'map',
    status: failed === 0 ? 'ok' : 'failed',
    items,
    durationMs: Date.now() - startedAt,
    error: failed === 0 ? undefined : `${failed}/${over.length} iteration(s) failed`,
  }
}

/**
 * Resolve the program of a command's first token.
 * @param token - the first token as written in the manifest.
 * @returns the resolution, or null when nothing matches.
 */
function resolveRuntimeName(token: string): { file: string, prefixArgs: string[] } | null {
  return resolveRuntime(token)
}

/**
 * Resolve the value a `save` writes into `vars`.
 * @param result - the settled result.
 * @returns the primary value.
 */
function primaryValue(result: StepResult): unknown {
  if (result.kind === 'map') return result.items
  if (result.kind === 'set') return result.json
  if (result.kind === 'ai') return result.json !== undefined ? result.json : result.text
  return result.json !== undefined ? result.json : result.stdout
}

/**
 * Build a failed result.
 * @param step - the failing step.
 * @param startedAt - attempt start time.
 * @param message - the failure message.
 * @returns the result.
 */
function failureResult(step: Step, startedAt: number, message: string): StepResult {
  return {
    id: step.id,
    kind: kindOf(step),
    status: 'failed',
    durationMs: Date.now() - startedAt,
    error: message,
  }
}

/**
 * Classify a step.
 * @param step - the step to classify.
 * @returns its executor kind.
 */
function kindOf(step: Step): StepResult['kind'] {
  if (step.run !== undefined) return 'run'
  if (step.set !== undefined) return 'set'
  if (step.ai !== undefined) return 'ai'
  return 'map'
}

/**
 * Parse stdout as JSON when possible.
 * @param text - captured stdout.
 * @returns the parsed value, or undefined when it is not JSON.
 */
function tryParseJson(text: string): unknown {
  const trimmed = text.trim()
  if (trimmed === '') return undefined
  if (!(trimmed.startsWith('{') || trimmed.startsWith('[') || trimmed.startsWith('"') || /^-?\d|^true|^false|^null/.test(trimmed))) {
    return undefined
  }
  try {
    return JSON.parse(trimmed)
  } catch (error) {
    if (error instanceof SyntaxError) return undefined
    throw error
  }
}

/**
 * Extract an error message from an unknown thrown value.
 * @param error - the thrown value.
 * @returns the message.
 */
function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

/**
 * First line of text.
 * @param text - the text.
 * @returns the first non-empty line, trimmed.
 */
function firstLine(text: string): string {
  return text.split(/\r?\n/).map(line => line.trim()).find(line => line !== '') ?? ''
}

/**
 * Describe a value's type for messages.
 * @param value - the value.
 * @returns a short type name.
 */
function typeName(value: unknown): string {
  if (value === null) return 'null'
  if (Array.isArray(value)) return 'list'
  return typeof value
}

/**
 * Wait.
 * @param ms - milliseconds to wait.
 * @returns a promise resolved after the delay.
 */
function sleep(ms: number): Promise<void> {
  return new Promise(resolve => {
    setTimeout(resolve, ms)
  })
}

/**
 * Render a step's command for display without running it.
 * @param step - the step to render.
 * @param scope - the live scope.
 * @param loaded - the loaded manifest.
 * @returns the rendered command line, or a description for non-`run` steps.
 */export function describeStep(step: Step, scope: Scope, loaded: LoadedWorkflow): string {
  if (step.run !== undefined) {
    const rendered = renderDeep(step.run, scope) as Step['run']
    const parts = rendered === undefined ? [] : Array.isArray(rendered.command) ? rendered.command : [rendered.command]
    return parts.join(' ')
  }
  if (step.set !== undefined) return `set ${Object.keys(step.set).join(', ')}`
  if (step.ai !== undefined) {
    const spec = step.ai
    const prompt = renderString(spec.prompt, scope).split(/\r?\n/).find(line => line.trim() !== '') ?? ''
    return `ai→${spec.backend ?? 'http'}: ${prompt.trim().slice(0, 72)}`
  }
  return `map over ${step.map?.over ?? ''}`
}

/**
 * Execute one step in isolation, for contract tests.
 * @param step - the step to run.
 * @param scope - a scope built from the fixture input.
 * @param options - run configuration.
 * @returns the settled result.
 */
export async function runOneStep(step: Step, scope: Scope, options: RunOptions): Promise<StepResult> {
  return executeStep(step, scope, options)
}

/**
 * Render the trace file name for a run.
 * @param runId - the run identifier.
 * @returns the file name.
 */
export function traceFileName(runId: string): string {
  return `${runId}.jsonl`
}
