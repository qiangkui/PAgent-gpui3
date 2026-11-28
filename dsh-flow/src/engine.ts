/**
 * Workflow execution.
 *
 * Steps run in order. A `run` step is one program invocation whose result is
 * recorded under `steps.<id>` and, when `save` is set, copied into `vars`. The
 * engine never retries a step whose previous attempt may have had side effects
 * unless the manifest asks for it.
 */

import { createHash, randomUUID } from 'node:crypto'
import { existsSync, mkdirSync, readFileSync, renameSync, statSync, writeFileSync } from 'node:fs'
import { basename, dirname, extname, join, resolve as resolvePath } from 'node:path'
import { fileURLToPath } from 'node:url'
import { createCliBackend, createHttpBackend } from './backend.ts'
import type { AiAsk, AiBackend, AiImage } from './backend.ts'
import { runCommand, stepEnvironment } from './exec.ts'
import { evaluate } from './expr.ts'
import { loadWorkflow } from './load.ts'
import type { LoadedWorkflow } from './load.ts'
import { describeRuntime, resolveRuntime } from './runtime.ts'
import { extractJson, firstIssue } from './schema.ts'
import { startServices, stopServices } from './services.ts'
import type { RunningService } from './services.ts'
import { evaluateCondition, renderDeep, renderString } from './template.ts'
import { isSensitivePath, sensitiveRefusal } from './tools/secrets.ts'
import type {
  AiSpec, MapItemResult, RunLevel, Scope, Step, StepArtifact, StepErrorInfo, StepErrorKind, StepResult,
  TraceRecord
} from './types.ts'

/** Default per-command timeout. */
const DEFAULT_TIMEOUT_MS = 120_000

/** Default fan-out concurrency. */
const DEFAULT_CONCURRENCY = 4

/** How many sub-flows may be nested inside each other. */
const MAX_USE_DEPTH = 5

/** Most pictures one `ai` step may attach, so a loop cannot flood a request. */
const MAX_AI_IMAGES = 3

/** Largest single picture a step may attach, in bytes. */
const MAX_AI_IMAGE_BYTES = 5 * 1024 * 1024

/** Total base64 text one question may carry, in characters. */
const MAX_AI_IMAGES_BASE64 = 12_000_000

/** Picture types an `ai` step may attach, with the MIME type sent to the model. */
const IMAGE_TYPES: Record<string, string> = {
  '.png': 'image/png',
  '.jpg': 'image/jpeg',
  '.jpeg': 'image/jpeg',
  '.webp': 'image/webp',
  '.gif': 'image/gif',
  '.bmp': 'image/bmp',
}

/**
 * This engine's own folder, handed to every `run` step as `DSH_ENGINE_DIR`.
 *
 * Steps that drive the browser call back into `cli.ts`, and a manifest should
 * not have to spell out where the engine was installed.
 */
const ENGINE_DIR = dirname(dirname(fileURLToPath(import.meta.url)))

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
  /**
   * Secret inputs by name.
   *
   * The window sends them on the child's stdin together with the endpoint
   * credentials, so a password never travels on a command line. A step program
   * still sees it as `{{ inputs.name }}`.
   */
  secrets?: Record<string, string>
  /**
   * Scrubber applied to everything the run records (log lines, trace records,
   * captured output, the summary). Built from the secret inputs when absent.
   */
  redact?: (text: string) => string
  /** Directory holding recorded AI decisions. Defaults to `<manifest>/.dsh-flow/answers`. */
  answersDir?: string
  /**
   * Inputs already typed, for a `use` step calling another manifest.
   *
   * The CLI hands over strings and lets `resolveInputs` coerce them; a sub-flow
   * hands over rendered values (a number stays a number), so they are taken as
   * given and only the declared names are checked.
   */
  given?: Record<string, unknown>
  /** How many sub-flows deep this run is; internal, so nesting cannot run away. */
  depth?: number
  /** Manifest files already on the call stack, for cycle detection. */
  chain?: string[]
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
export function resolveInputs(
  loaded: LoadedWorkflow,
  supplied: Record<string, string>,
  secrets: Record<string, string> = {},
): Record<string, unknown> {
  const resolved: Record<string, unknown> = {}
  const declared = loaded.workflow.inputs
  for (const name of Object.keys(supplied)) {
    if (!(name in declared)) {
      throw new Error(`unknown input "${name}" (declared: ${Object.keys(declared).join(', ') || 'none'})`)
    }
    if (declared[name]!.secret === true) {
      throw new Error(
        `input "${name}" 是 secret：不能用 --input 传（会进命令行与日志）。` +
        `用 --secrets-stdin 或环境变量 DSH_SECRET_${name.toUpperCase()}`,
      )
    }
  }
  for (const name of Object.keys(secrets)) {
    if (!(name in declared)) {
      throw new Error(`unknown secret "${name}" (declared: ${Object.keys(declared).join(', ') || 'none'})`)
    }
  }
  for (const [name, spec] of Object.entries(declared)) {
    if (spec.secret === true) {
      const env = process.env[`DSH_SECRET_${name.toUpperCase()}`]
      const value = secrets[name] ?? env
      if (value !== undefined && value !== '') {
        resolved[name] = value
        continue
      }
      if (spec.required) {
        throw new Error(
          `缺少 secret 输入 "${name}"：在界面里填写，或 --secrets-stdin 传 ` +
          `{"secrets":{"${name}":"…"}}，或设置 DSH_SECRET_${name.toUpperCase()}`,
        )
      }
      resolved[name] = undefined
      continue
    }
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
 * Build the replacement applied to everything a run records.
 *
 * A secret in a manifest is only half of the promise: the step it reaches can
 * print it, an error message can quote it, and a trace or an HTML report then
 * holds it forever. One function, applied at the recording points, keeps that
 * from happening — even when a step was careless.
 * @param values - the resolved secret values.
 * @returns the replacement, or undefined when the run has no secrets.
 */
function redactor(values: unknown[]): ((text: string) => string) | undefined {
  const needles = values
    .filter((value): value is string => typeof value === 'string')
    .map((value) => value.trim())
    // One character would rewrite the whole transcript; two is the shortest
    // secret worth protecting.
    .filter((value) => value.length >= 2)
    .sort((left, right) => right.length - left.length)
  if (needles.length === 0) return undefined
  return (text: string): string => {
    let out = text
    for (const needle of needles) out = out.split(needle).join('***')
    return out
  }
}

/** Replace secrets inside a JSON value, depth-first. */
function redactValue(value: unknown, redact: (text: string) => string, depth = 0): unknown {
  if (depth > 6) return value
  if (typeof value === 'string') return redact(value)
  if (Array.isArray(value)) return value.map((entry) => redactValue(entry, redact, depth + 1))
  if (value !== null && typeof value === 'object') {
    const out: Record<string, unknown> = {}
    for (const [key, entry] of Object.entries(value as Record<string, unknown>)) {
      out[key] = redactValue(entry, redact, depth + 1)
    }
    return out
  }
  return value
}

/**
 * Replace every secret inside one step result.
 * @param result - the result to scrub in place.
 * @param redact - the replacement, when the run has secrets.
 */
function redactResult(result: StepResult, redact: ((text: string) => string) | undefined): void {
  if (redact === undefined) return
  for (const key of ['text', 'stdout', 'stderr', 'error', 'summary', 'command'] as const) {
    const value = result[key]
    if (typeof value === 'string') result[key] = redact(value)
  }
  if (result.json !== undefined) result.json = redactValue(result.json, redact)
  if (result.errorInfo !== undefined) {
    const info = { ...result.errorInfo }
    for (const key of ['message', 'hint', 'where', 'expected', 'gotten'] as const) {
      const value = info[key]
      if (typeof value === 'string') info[key] = redact(value)
    }
    result.errorInfo = info
  }
}

/**
 * Resolve the already-typed inputs a sub-flow was handed.
 *
 * Mirrors [`resolveInputs`] but keeps every value as it is: a number stays a
 * number, an object stays an object, so a sub-flow sees what the parent's
 * template produced rather than its string form.
 * @param loaded - the child manifest.
 * @param supplied - rendered values from the caller.
 * @returns the child's inputs, with declared names only.
 */
function resolveGivenInputs(
  loaded: LoadedWorkflow,
  supplied: Record<string, unknown>
): Record<string, unknown> {
  const resolved: Record<string, unknown> = {}
  const declared = loaded.workflow.inputs
  for (const name of Object.keys(supplied)) {
    if (!(name in declared)) {
      throw new Error(
        `unknown input "${name}" for sub-flow "${loaded.workflow.name}" (declared: ${Object.keys(declared).join(', ') || 'none'})`
      )
    }
  }
  for (const [name, spec] of Object.entries(declared)) {
    if (name in supplied) {
      resolved[name] = supplied[name]
      continue
    }
    if (spec.default !== undefined) {
      resolved[name] = spec.default
      continue
    }
    if (spec.required) throw new Error(`missing required input "${name}" for sub-flow "${loaded.workflow.name}"`)
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
  const inputs =
    options.given !== undefined
      ? resolveGivenInputs(options.loaded, options.given)
      : resolveInputs(options.loaded, options.inputs, options.secrets ?? {})
  const secrets = Object.entries(options.loaded.workflow.inputs)
    .filter(([, spec]) => spec.secret === true)
    .map(([name]) => inputs[name])
  const redact = options.redact ?? redactor(secrets)
  // One wrapped copy, so every progress line and trace record this run emits is
  // scrubbed without each call site having to remember.
  const run: RunOptions = {
    ...options,
    redact,
    log: redact === undefined ? options.log : (line: string) => options.log(redact(line)),
  }
  const scope: Scope = {
    inputs,
    vars: { ...run.loaded.workflow.vars },
    steps: {},
    env: process.env,
  }

  run.trace?.({ ts: new Date().toISOString(), type: 'run/start', detail: run.loaded.workflow.name })

  const running: RunningService[] = []
  let serviceError = ''
  let failure: StepFailure | null = null
  try {
    try {
      running.push(
        ...(await startServices({
          services: run.loaded.workflow.services ?? {},
          defaults: run.loaded.workflow.defaults,
          scope,
          dir: run.loaded.dir,
          log: run.log,
          trace: run.trace,
          dryRun: run.dryRun,
        }))
      )
    } catch (error) {
      serviceError = messageOf(error)
    }
    if (serviceError === '') {
      try {
        await executeSteps(run.loaded.workflow.steps, scope, run)
      } catch (error) {
        if (!(error instanceof StepFailure)) throw error
        failure = error
      }
    }
  } finally {
    await stopServices(running, { trace: run.trace, log: run.log })
  }

  const error = serviceError !== '' ? serviceError : failure?.message
  run.trace?.({
    ts: new Date().toISOString(),
    type: 'run/end',
    // The verdict rides on the event itself: a window that only streams events
    // must not have to guess it from the last step.
    status: error === undefined ? 'ok' : 'failed',
    durationMs: Date.now() - startedAt,
    detail: error ?? 'ok',
  })

  // The summary leaves with a scrubbed copy of every step: the trace may be
  // copied into a report or read by the window long after the run is gone.
  const steps: Record<string, StepResult> = {}
  for (const [id, result] of Object.entries(scope.steps)) {
    const copy: StepResult = { ...result }
    redactResult(copy, redact)
    steps[id] = copy
  }

  return {
    runId,
    status: error === undefined ? 'ok' : 'failed',
    steps,
    // `vars` holds what steps saved, which is the same text the steps printed:
    // the report renders it, so it is scrubbed with everything else.
    vars: redact === undefined ? scope.vars : redactValue(scope.vars, redact) as Record<string, unknown>,
    durationMs: Date.now() - startedAt,
    failedStep: failure?.stepId,
    error: error === undefined || redact === undefined ? error : redact(error),
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
    summary: result.summary,
    artifacts: result.artifacts,
    error: result.errorInfo,
  })
  const marker = result.status === 'ok' ? 'OK  ' : result.status === 'skipped' ? 'SKIP' : 'FAIL'
  const detail = result.status === 'ok'
    ? result.kind === 'ai'
      ? `${result.durationMs}ms · ${result.model ?? 'model'}${result.cached === true ? ' · 复用记录' : ''}`
      : `${result.durationMs}ms`
    : result.error ?? ''
  // The step's own one-liner, when it wrote one, is what a reader wants before
  // the timing; it rides after it so existing logs keep their shape.
  const note = result.summary === undefined ? '' : ` · ${result.summary}`
  options.log(`[${marker}] ${step.id}${step.desc ? ` — ${step.desc}` : ''} (${detail})${note}`)
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
    if (step.use !== undefined) return await executeUse(step, scope, options, startedAt)
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
  const env = stepEnvironment({ ...defaults.env, ...rendered.env, DSH_ENGINE_DIR: ENGINE_DIR })
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
  // A step may describe its own outcome (`summary` / `artifacts` / `error`), which
  // is what the window's card renders instead of a wall of stdout.
  const report = liftReport(parsed, options.loaded.dir)
  if (report.summary !== undefined) result.summary = report.summary
  if (report.artifacts !== undefined) result.artifacts = report.artifacts
  if (report.errorInfo !== undefined) result.errorInfo = report.errorInfo
  return result
}

/** Most files one step may hand over. */
const MAX_ARTIFACTS = 12

/** Picture extensions an artifact may claim. */
const ARTIFACT_IMAGES: Record<string, string> = {
  '.png': 'image/png',
  '.jpg': 'image/jpeg',
  '.jpeg': 'image/jpeg',
  '.webp': 'image/webp',
  '.gif': 'image/gif',
}

/** Every failure kind a step may report. */
const ERROR_KINDS: ReadonlySet<string> = new Set([
  'credential', 'config', 'selector-miss', 'regex-miss', 'image-unreadable', 'assert',
  'timeout', 'model', 'engine',
])

/**
 * Read the optional `summary` / `artifacts` / `error` a step wrote.
 *
 * None of it is required: a step that prints plain JSON keeps working, it just
 * shows up with the raw output. What is here is what makes a card readable
 * without the transcript.
 * @param parsed - the step's parsed stdout.
 * @param dir - the manifest folder relative artifact paths resolve against.
 * @returns whatever the step described, each field optional.
 */
function liftReport(
  parsed: unknown,
  dir: string,
): { summary?: string; artifacts?: StepArtifact[]; errorInfo?: StepErrorInfo } {
  if (parsed === null || typeof parsed !== 'object' || Array.isArray(parsed)) return {}
  const record = parsed as Record<string, unknown>
  const out: { summary?: string; artifacts?: StepArtifact[]; errorInfo?: StepErrorInfo } = {}

  const summary = record.summary
  if (typeof summary === 'string' && summary.trim() !== '') {
    out.summary = clip(summary.trim(), 200)
  }

  const artifacts = record.artifacts
  if (Array.isArray(artifacts) && artifacts.length > 0) {
    const kept: StepArtifact[] = []
    for (const entry of artifacts.slice(0, MAX_ARTIFACTS)) {
      if (entry === null || typeof entry !== 'object' || Array.isArray(entry)) continue
      const raw = entry as Record<string, unknown>
      const written = typeof raw.path === 'string' ? raw.path.trim() : ''
      if (written === '') continue
      const absolute = resolvePath(dir, written)
      if (!existsSync(absolute) || !statSync(absolute).isFile()) continue
      if (isSensitivePath(absolute)) continue
      const mime = ARTIFACT_IMAGES[extname(absolute).toLowerCase()]
      const kind = raw.kind === 'image' || (raw.kind === undefined && mime !== undefined) ? 'image' : 'file'
      kept.push({
        kind,
        name: typeof raw.name === 'string' && raw.name.trim() !== '' ? clip(raw.name.trim(), 80) : basename(absolute),
        path: absolute,
        bytes: statSync(absolute).size,
        ...(kind === 'image' && mime !== undefined ? { mime } : {}),
      })
    }
    if (kept.length > 0) out.artifacts = kept
  }

  const error = record.error
  if (error !== null && typeof error === 'object' && !Array.isArray(error)) {
    const raw = error as Record<string, unknown>
    const message = typeof raw.message === 'string' ? raw.message.trim() : ''
    if (message !== '') {
      const reported = typeof raw.kind === 'string' ? raw.kind : ''
      out.errorInfo = {
        kind: (ERROR_KINDS.has(reported) ? reported : 'engine') as StepErrorKind,
        message: clip(message, 300),
        ...(typeof raw.hint === 'string' && raw.hint.trim() !== '' ? { hint: clip(raw.hint.trim(), 200) } : {}),
        ...(typeof raw.where === 'string' && raw.where.trim() !== '' ? { where: clip(raw.where.trim(), 120) } : {}),
        ...(typeof raw.expected === 'string' && raw.expected.trim() !== '' ? { expected: clip(raw.expected.trim(), 120) } : {}),
        ...(typeof raw.gotten === 'string' && raw.gotten.trim() !== '' ? { gotten: clip(raw.gotten.trim(), 120) } : {}),
      }
    }
  }
  return out
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
 * Run another manifest as one step.
 *
 * The child gets its own folder, its own recorded decisions and its own ledger,
 * so a shared login or fetch flow can be developed and settled on its own. Its
 * final `vars` come back as this step's JSON, which is what lets the parent read
 * `{{ steps.<id>.json.<name> }}`; a failed child fails this step.
 * @param step - the step to run.
 * @param scope - the live scope.
 * @param options - run configuration.
 * @param startedAt - attempt start time.
 * @returns the child's variables, or the failure that stopped it.
 */
async function executeUse(
  step: Step,
  scope: Scope,
  options: RunOptions,
  startedAt: number
): Promise<StepResult> {
  const spec = step.use!
  const manifest = resolvePath(options.loaded.dir, renderString(spec.manifest, scope).trim())
  const depth = options.depth ?? 0
  const chain = options.chain ?? [options.loaded.file]
  if (depth >= MAX_USE_DEPTH) {
    throw new Error(`sub-flow nesting is deeper than ${MAX_USE_DEPTH} levels`)
  }
  const key = (path: string): string => resolvePath(path).toLowerCase()
  if (chain.some((seen) => key(seen) === key(manifest))) {
    throw new Error(`sub-flow cycle: ${manifest} is already running`)
  }
  if (options.dryRun) {
    return {
      id: step.id,
      kind: 'use',
      status: 'skipped',
      durationMs: Date.now() - startedAt,
      command: manifest,
      error: 'dry run',
    }
  }
  const loaded = loadWorkflow(manifest)
  const given = renderDeep(spec.inputs ?? {}, scope) as Record<string, unknown>
  const summary = await runWorkflow({
    loaded,
    inputs: {},
    given,
    level: options.level,
    backend: options.backend,
    cache: options.cache,
    aiEnv: options.aiEnv,
    // Secrets stay scrubbed inside the child, and its events/log lines are the
    // parent's: a password typed once must not surface in either transcript.
    redact: options.redact,
    onceLedger: resolvePath(loaded.dir, '.dsh-flow', 'ledger.json'),
    depth: depth + 1,
    chain: [...chain, manifest],
    // The parent's trace stays a single run: child step ids would collide with
    // the parent's in the report, so the child only writes to the log.
    log: (line) => options.log(`  [${step.id}] ${line}`),
  })
  const result: StepResult = {
    id: step.id,
    kind: 'use',
    status: summary.status === 'ok' ? 'ok' : 'failed',
    durationMs: Date.now() - startedAt,
    command: manifest,
    json: summary.vars,
    text: `sub-flow ${loaded.workflow.name}: ${summary.status}`,
  }
  if (summary.status !== 'ok') {
    result.error = `sub-flow ${loaded.workflow.name} failed${summary.failedStep === undefined ? '' : ` at "${summary.failedStep}"`}${summary.error === undefined ? '' : `: ${summary.error}`}`
    // The interesting failure is the child's: a card that says "credential" and
    // points at `read` is worth more than "the sub-flow failed".
    const inner = summary.failedStep === undefined ? undefined : summary.steps[summary.failedStep]
    if (inner?.errorInfo !== undefined) {
      result.errorInfo = {
        ...inner.errorInfo,
        where: `${loaded.workflow.name}:${summary.failedStep}`,
      }
    }
  }
  return result
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
  const ask = buildAsk(spec, scope, options)
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
  // A step whose input changes on every run opts out of the decision cache:
  // replaying yesterday's captcha would only look like a cache hit.
  const cache = options.cache !== false && spec.cache !== false
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
          summary: answerSummary(recorded.text),
          durationMs: Date.now() - startedAt,
        }
      }
      options.log(`  warning: "${step.id}" 的记录已不符合 schema（${settled.error}），需要重新生成`)
    }
  }

  if (level === 'manual') {
    throw new Error(
      cache
        ? `manual 档不调用模型："${step.id}" 没有可复用的判断结果（${relativePath(options.loaded.dir, file)}）。` +
          '先跑一次 --level assist 记录，或临时用 --level draft'
        : `manual 档不调用模型："${step.id}" 标了 ai.cache: false（每次都问模型、从不回放记录），` +
          '这一步没有答案可复用。用 --level assist 或 draft 运行。',
    )
  }

  const backend = options.backend ?? backendFor(spec, options)
  let first: Awaited<ReturnType<AiBackend['ask']>>
  try {
    first = await backend.ask(withSchema(ask))
  } catch (error) {
    // A missing key, a rejected key and a hung endpoint are the most common
    // failures of an ai step; the card should say which one it was.
    return aiFailure(step, startedAt, error)
  }
  let reply = first
  let settled = parseAnswer(spec, first.text)
  if (!settled.ok) {
    options.log(`  ai "${step.id}" 的回答不符合约定（${settled.error}），重问一次`)
    try {
      reply = await backend.ask(withSchema({ ...ask, prompt: repairPrompt(ask.prompt, first.text, settled.error) }))
    } catch (error) {
      return aiFailure(step, startedAt, error)
    }
    const repaired = parseAnswer(spec, reply.text)
    if (!repaired.ok) {
      return aiFailure(step, startedAt, new Error(`ai 步骤 "${step.id}" 的回答仍不符合约定：${repaired.error}`))
    }
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
    summary: answerSummary(reply.text),
    durationMs: Date.now() - startedAt,
  }
}

/**
 * One line from a model answer, for the step card.
 * @param text - the answer.
 * @returns its first non-empty line, clipped.
 */
function answerSummary(text: string): string {
  const line = text.split(/\r?\n/).map((entry) => entry.trim()).find((entry) => entry !== '') ?? ''
  return clip(line, 120)
}

/**
 * Turn a model-side failure into a settled step result.
 *
 * The kinds are the three things that actually happen: no key (or a rejected
 * one), the endpoint or model is not there, and the request timed out. A card
 * that says which one saves a round of guessing.
 * @param step - the failing step.
 * @param startedAt - attempt start time.
 * @param error - what the backend threw.
 * @returns a failed result carrying the kind and a hint.
 */
function aiFailure(step: Step, startedAt: number, error: unknown): StepResult {
  const message = messageOf(error)
  let kind: StepErrorKind = 'model'
  let hint = '检查接入点、模型名与额度；界面「设置 → AI」里能看到上一次的连通性测试结果'
  if (/API Key|缺少|401|403/i.test(message)) {
    kind = 'credential'
    hint = '在界面「设置 → AI」里填写 Key，或设置 DEEPSEEK_API_KEY；不填就不会调用模型'
  } else if (/timeout|timed out|ETIMEDOUT|ECONNRESET/i.test(message)) {
    kind = 'timeout'
    hint = '端点没有在超时内回答；确认网络/代理，或给这一步的 ai.timeoutMs 放宽'
  } else if (/404|模型不存在|unknown model/i.test(message)) {
    hint = '模型 id 写错了，或该 Key 没有这个模型；换一个再试'
  }
  return {
    id: step.id,
    kind: 'ai',
    status: 'failed',
    durationMs: Date.now() - startedAt,
    error: message,
    errorInfo: { kind, message: clip(message, 300), hint },
  }
}

/**
 * Render the prompt and system message of an `ai` step.
 * @param spec - the step's model-call spec.
 * @param scope - the live scope.
 * @returns the question to send.
 */
function buildAsk(spec: AiSpec, scope: Scope, options: RunOptions): AiAsk {
  return {
    prompt: renderString(spec.prompt, scope),
    system: spec.system === undefined ? undefined : renderString(spec.system, scope),
    // `model` is a template as well, so one manifest can point at a vision model
    // for the step that reads a picture and leave the rest to the endpoint.
    model: spec.model === undefined ? undefined : renderString(spec.model, scope).trim() || undefined,
    temperature: spec.temperature,
    maxTokens: spec.maxTokens,
    schema: spec.schema,
    images: readImages(spec.images, scope, options.loaded.dir),
  }
}

/**
 * Read the pictures one `ai` step attaches.
 *
 * Paths are templates, so a step can point at the screenshot the step before it
 * saved. They are read here rather than at request time: a missing file then
 * fails the step with that path in the message, before any tokens are spent.
 * @param paths - the `ai.images` list, unrendered.
 * @param scope - the live scope.
 * @param dir - the manifest folder that relative paths resolve against.
 * @returns the pictures, or undefined when the step attaches none.
 */
function readImages(paths: string[] | undefined, scope: Scope, dir: string): AiImage[] | undefined {
  if (paths === undefined || paths.length === 0) return undefined
  if (paths.length > MAX_AI_IMAGES) {
    throw new Error(`ai.images 最多 ${MAX_AI_IMAGES} 张，这里给了 ${paths.length} 张`)
  }
  const images: AiImage[] = []
  let base64Chars = 0
  for (const template of paths) {
    const requested = renderString(template, scope).trim()
    if (requested === '') throw new Error('ai.images 里有一个空路径')
    const file = resolvePath(dir, requested)
    const mime = IMAGE_TYPES[extname(file).toLowerCase()]
    if (mime === undefined) {
      throw new Error(
        `ai.images 不支持这个文件类型：${requested}（支持 ${Object.keys(IMAGE_TYPES).join(' / ')}）`,
      )
    }
    let bytes: Buffer
    try {
      bytes = readFileSync(file)
    } catch (error) {
      throw new Error(`ai.images 读不到 ${requested}：${messageOf(error)}`)
    }
    if (bytes.length === 0) throw new Error(`ai.images 的图片是空的：${requested}`)
    if (bytes.length > MAX_AI_IMAGE_BYTES) {
      throw new Error(
        `ai.images 的图片太大：${requested}（${Math.round(bytes.length / 1024)} KB，` +
        `上限 ${MAX_AI_IMAGE_BYTES / (1024 * 1024)} MB）`,
      )
    }
    base64Chars += Math.ceil(bytes.length / 3) * 4
    if (base64Chars > MAX_AI_IMAGES_BASE64) {
      throw new Error(
        `ai.images 总量超过 ${MAX_AI_IMAGES_BASE64 / 1_000_000} MB base64：把截图缩小，或分几步问`,
      )
    }
    images.push({ path: file, mime, data: bytes.toString('base64') })
  }
  return images
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
    // Pictures are part of the question: a new screenshot is a new decision,
    // even when the prompt text is word for word the same.
    (ask.images ?? []).map((image) => createHash('sha256').update(image.data).digest('hex')),
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
  if (step.use !== undefined) return 'use'
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
  if (step.use !== undefined) {
    return `use ${renderString(step.use.manifest, scope)}`
  }
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
