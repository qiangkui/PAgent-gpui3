/**
 * Manifest loading and static validation.
 *
 * Loading is deliberately strict: a manifest that would fail at run time is
 * rejected here with a positioned message, so a generated manifest can be
 * checked without executing anything.
 */

import { existsSync, readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { validateSchema } from './schema.ts'
import { checkTemplateSyntax } from './template.ts'
import type {
  AiSpec, InputSpec, MapSpec, RunSpec, ServiceSpec, Step, UseSpec, Workflow
} from './types.ts'
import { parseYaml } from './yaml.ts'

/** Manifest entry as written, before normalization. */
interface RawStep {
  /** Step id. */
  id?: unknown
  /** Human-readable purpose. */
  desc?: unknown
  /** Condition expression. */
  when?: unknown
  /** Always skip. */
  skip?: unknown
  /** Keep the run going on failure. */
  allowFailure?: unknown
  /** Run until the first success. */
  once?: unknown
  /** Extra attempts. */
  retries?: unknown
  /** Delay between attempts. */
  retryDelayMs?: unknown
  /** Variable receiving the primary value. */
  save?: unknown
  /** Why this step must ask a model. */
  why?: unknown
  /** Command form. */
  run?: unknown
  /** Assignment form. */
  set?: unknown
  /** Fan-out form. */
  map?: unknown
  /** Model-call form. */
  ai?: unknown
  /** Sub-flow form. */
  use?: unknown
}

/** A loaded manifest plus the directory its paths resolve against. */
export interface LoadedWorkflow {
  /** The validated manifest. */
  workflow: Workflow
  /** Absolute path of the manifest file. */
  file: string
  /** Absolute path of the manifest directory, used for `cwd`. */
  dir: string
  /** Non-fatal findings worth showing the user. */
  warnings: string[]
}

const ALLOWED_STEP_KEYS = new Set([
  'id', 'desc', 'when', 'skip', 'once', 'allowFailure', 'retries', 'retryDelayMs', 'save', 'why', 'run', 'set', 'map', 'ai', 'use',
])

/**
 * Normalize `once`: absent / false means always run, `true` keys by step id,
 * and a non-empty string is a custom key shared between steps.
 * @param raw - the manifest value.
 * @param id - step id, for messages.
 * @param file - manifest path, for messages.
 * @returns the normalized value, or undefined when the step always runs.
 */
function normalizeOnce(raw: unknown, id: string, file: string): boolean | string | undefined {
  if (raw === undefined || raw === null || raw === false) return undefined
  if (raw === true) return true
  if (typeof raw === 'string') {
    const key = raw.trim()
    if (key === '') throw new Error(`${file}: step "${id}" has an empty "once"`)
    return key
  }
  throw new Error(`${file}: step "${id}" has a non-boolean "once"`)
}

/**
 * Read and validate a manifest.
 * @param file - path to a `.yml`, `.yaml`, or `.json` manifest.
 * @returns the validated workflow and its resolution directory.
 */
export function loadWorkflow(file: string): LoadedWorkflow {
  const absolute = resolve(file)
  const text = readFileSync(absolute, 'utf8')
  const raw = absolute.endsWith('.json') ? JSON.parse(text) : parseYaml(text)
  const warnings: string[] = []
  const workflow = normalizeWorkflow(raw, absolute, warnings)
  return { workflow, file: absolute, dir: dirname(absolute), warnings }
}

/**
 * Validate a manifest object and normalize it.
 * @param raw - the parsed manifest.
 * @param file - the manifest path, used in messages.
 * @param warnings - collector for non-fatal findings.
 * @returns the normalized workflow.
 */
function normalizeWorkflow(raw: unknown, file: string, warnings: string[]): Workflow {
  if (!isRecord(raw)) throw new Error(`${file}: manifest must be a mapping at the top level`)
  requireString(raw.name, `${file}: "name" is required`)
  requireString(raw.description, `${file}: "description" is required`)
  const version = raw.version === undefined ? 1 : raw.version
  if (version !== 1) throw new Error(`${file}: unsupported "version" ${String(version)} (supported: 1)`)

  const inputs = normalizeInputs(raw.inputs, file)
  const vars = isRecord(raw.vars) ? raw.vars : {}
  const defaults = normalizeDefaults(raw.defaults, file)
  const services = normalizeServices(raw.services, file, warnings)
  if (!Array.isArray(raw.steps) || raw.steps.length === 0) throw new Error(`${file}: "steps" must be a non-empty list`)

  const steps = normalizeSteps(raw.steps, file, warnings, '')
  return {
    name: raw.name as string,
    description: raw.description as string,
    version: 1,
    inputs,
    vars,
    defaults,
    services,
    steps,
  }
}

/**
 * Normalize the top-level `services` map.
 * @param raw - the `services` value.
 * @param file - manifest path, for messages.
 * @param warnings - collector for non-fatal findings.
 * @returns the services, or undefined when none are declared.
 */
function normalizeServices(
  raw: unknown,
  file: string,
  warnings: string[]
): Workflow['services'] {
  if (raw === undefined || raw === null) return undefined
  if (!isRecord(raw)) throw new Error(`${file}: "services" must be a mapping`)
  const out: NonNullable<Workflow['services']> = {}
  for (const [name, value] of Object.entries(raw)) {
    if (!isRecord(value)) throw new Error(`${file}: service "${name}" must be a mapping`)
    for (const key of Object.keys(value)) {
      if (key !== 'run' && key !== 'ready') {
        warnings.push(`${file}: service "${name}" has unknown key "${key}"`)
      }
    }
    if (value.run === undefined || value.run === null) {
      throw new Error(`${file}: service "${name}" needs "run"`)
    }
    out[name] = {
      run: normalizeRun(value.run, `services.${name}`, file),
      ready: normalizeReady(value.ready, name, file),
    }
  }
  return out
}

/**
 * Normalize one service's readiness probe.
 * @param raw - the `ready` value: a URL string or a mapping.
 * @param name - service name, for messages.
 * @param file - manifest path, for messages.
 * @returns the probe, or undefined when none is declared.
 */
function normalizeReady(raw: unknown, name: string, file: string): ServiceSpec['ready'] {
  if (raw === undefined || raw === null) return undefined
  if (typeof raw === 'string') return { url: checkReadyUrl(raw, name, file) }
  if (!isRecord(raw)) throw new Error(`${file}: service "${name}" "ready" must be a URL or a mapping`)
  for (const key of Object.keys(raw)) {
    if (!['url', 'timeoutMs', 'intervalMs'].includes(key)) {
      throw new Error(`${file}: service "${name}" has unknown "ready.${key}"`)
    }
  }
  if (typeof raw.url !== 'string' || raw.url.trim() === '') {
    throw new Error(`${file}: service "${name}" "ready.url" is required`)
  }
  const timeoutMs = optionalNumber(raw.timeoutMs, `${file}: service "${name}" has a non-numeric "ready.timeoutMs"`)
  if (timeoutMs !== undefined && timeoutMs < 1) {
    throw new Error(`${file}: service "${name}" "ready.timeoutMs" must be positive`)
  }
  const intervalMs = optionalNumber(raw.intervalMs, `${file}: service "${name}" has a non-numeric "ready.intervalMs"`)
  if (intervalMs !== undefined && intervalMs < 1) {
    throw new Error(`${file}: service "${name}" "ready.intervalMs" must be positive`)
  }
  return { url: checkReadyUrl(raw.url, name, file), timeoutMs, intervalMs }
}

/** Keep the probe to HTTP(S) but allow template placeholders in the host/port. */
function checkReadyUrl(url: string, name: string, file: string): string {
  const trimmed = url.trim()
  if (!/^https?:\/\//i.test(trimmed)) {
    throw new Error(`${file}: service "${name}" "ready.url" must start with http:// or https://`)
  }
  return trimmed
}

/**
 * Validate the declared inputs.
 * @param raw - the `inputs` value.
 * @param file - manifest path, for messages.
 * @returns the normalized inputs.
 */
function normalizeInputs(raw: unknown, file: string): Record<string, InputSpec> {
  if (raw === undefined || raw === null) return {}
  if (!isRecord(raw)) throw new Error(`${file}: "inputs" must be a mapping`)
  const out: Record<string, InputSpec> = {}
  for (const [name, spec] of Object.entries(raw)) {
    if (!isRecord(spec)) throw new Error(`${file}: input "${name}" must be a mapping`)
    const secret = spec.secret === true
    if (secret && spec.default !== undefined && spec.default !== null) {
      // A default in the manifest is the very leak this flag exists to stop:
      // the file is committed, and the value would also travel on a command line.
      throw new Error(`${file}: input "${name}" 标了 secret: true，就不能再写 default（密码不进清单）`)
    }
    out[name] = {
      description: typeof spec.description === 'string' ? spec.description : undefined,
      required: spec.required === true,
      default: spec.default,
      secret: secret ? true : undefined,
    }
  }
  return out
}

/**
 * Validate the shared run defaults.
 * @param raw - the `defaults` value.
 * @param file - manifest path, for messages.
 * @returns the normalized defaults.
 */
function normalizeDefaults(raw: unknown, file: string): Workflow['defaults'] {
  if (raw === undefined || raw === null) return {}
  if (!isRecord(raw)) throw new Error(`${file}: "defaults" must be a mapping`)
  const run = raw.run
  if (run === undefined || run === null) return {}
  if (!isRecord(run)) throw new Error(`${file}: "defaults.run" must be a mapping`)
  return {
    cwd: optionalString(run.cwd, `${file}: "defaults.run.cwd" must be a string`),
    timeoutMs: optionalNumber(run.timeoutMs, `${file}: "defaults.run.timeoutMs" must be a number`),
    env: optionalEnv(run.env, `${file}: "defaults.run.env" must map names to strings`),
  }
}

/**
 * Normalize one step list.
 * @param raw - the step list.
 * @param file - manifest path, for messages.
 * @param warnings - collector for non-fatal findings.
 * @param prefix - id prefix used inside `map` bodies, for messages.
 * @returns the normalized steps.
 */
function normalizeSteps(raw: unknown[], file: string, warnings: string[], prefix: string): Step[] {
  const seen = new Set<string>()
  return raw.map((entry, position) => {
    const step = normalizeStep(entry, file, warnings, `${prefix}steps[${position}]`)
    if (seen.has(step.id)) throw new Error(`${file}: duplicate step id "${step.id}" in ${prefix || 'root steps'}`)
    seen.add(step.id)
    return step
  })
}

/**
 * Normalize a single step.
 * @param raw - the step entry.
 * @param file - manifest path, for messages.
 * @param warnings - collector for non-fatal findings.
 * @param where - human-readable position, for messages.
 * @returns the normalized step.
 */
function normalizeStep(raw: unknown, file: string, warnings: string[], where: string): Step {
  if (!isRecord(raw)) throw new Error(`${file}: ${where} must be a mapping`)
  const step = raw as RawStep
  for (const key of Object.keys(raw)) {
    if (!ALLOWED_STEP_KEYS.has(key)) warnings.push(`${file}: ${where} has unknown key "${key}" (ignored)`)
  }
  requireString(step.id, `${file}: ${where} needs an "id"`)
  const id = step.id as string
  if (!/^[A-Za-z0-9_-]+$/.test(id)) throw new Error(`${file}: step id "${id}" may only contain letters, digits, "_" and "-"`)

  const kinds = [step.run, step.set, step.map, step.ai, step.use].filter(value => value !== undefined && value !== null)
  if (kinds.length === 0) {
    throw new Error(`${file}: step "${id}" needs exactly one of "run", "set", "map", "ai", or "use"`)
  }
  if (kinds.length > 1) {
    throw new Error(`${file}: step "${id}" may only use one of "run", "set", "map", "ai", or "use"`)
  }

  const when = optionalString(step.when, `${file}: step "${id}" has a non-string "when"`)
  if (when !== undefined) checkTemplateSyntax(`{{ ${when} }}`)

  const base: Step = {
    id,
    desc: optionalString(step.desc, `${file}: step "${id}" has a non-string "desc"`),
    when,
    skip: step.skip === true,
    once: normalizeOnce(step.once, id, file),
    allowFailure: step.allowFailure === true,
    retries: optionalNumber(step.retries, `${file}: step "${id}" has a non-numeric "retries"`),
    retryDelayMs: optionalNumber(step.retryDelayMs, `${file}: step "${id}" has a non-numeric "retryDelayMs"`),
    save: optionalString(step.save, `${file}: step "${id}" has a non-string "save"`),
    why: optionalString(step.why, `${file}: step "${id}" has a non-string "why"`),
  }

  if (step.run !== undefined && step.run !== null) return { ...base, run: normalizeRun(step.run, id, file) }
  if (step.set !== undefined && step.set !== null) {
    if (!isRecord(step.set)) throw new Error(`${file}: step "${id}" "set" must be a mapping`)
    for (const value of Object.values(step.set)) {
      if (typeof value === 'string') checkTemplateSyntax(value)
    }
    return { ...base, set: step.set }
  }
  if (step.ai !== undefined && step.ai !== null) return { ...base, ai: normalizeAi(step.ai, id, file, warnings) }
  if (step.use !== undefined && step.use !== null) return { ...base, use: normalizeUse(step.use, id, file, warnings) }
  return { ...base, map: normalizeMap(step.map, id, file, warnings) }
}

/**
 * Normalize a `use` step: a path, or a mapping with `manifest` and `inputs`.
 *
 * A manifest path without a template is checked on disk here, so a renamed or
 * mistyped sub-flow is a `validate` error rather than a mid-run surprise.
 * @param raw - the `use` value.
 * @param id - step id, for messages.
 * @param file - manifest path, for messages.
 * @param warnings - collector for non-fatal findings.
 * @returns the normalized sub-flow call.
 */
function normalizeUse(raw: unknown, id: string, file: string, warnings: string[]): UseSpec {
  if (typeof raw === 'string') return useSpec(raw.trim(), undefined, id, file)
  if (!isRecord(raw)) throw new Error(`${file}: step "${id}" "use" must be a path or a mapping`)
  for (const key of Object.keys(raw)) {
    if (key !== 'manifest' && key !== 'inputs') warnings.push(`${file}: step "${id}" has unknown "use.${key}"`)
  }
  if (typeof raw.manifest !== 'string' || raw.manifest.trim() === '') {
    throw new Error(`${file}: step "${id}" "use.manifest" is required`)
  }
  let inputs: Record<string, unknown> | undefined
  if (raw.inputs !== undefined && raw.inputs !== null) {
    if (!isRecord(raw.inputs)) throw new Error(`${file}: step "${id}" "use.inputs" must be a mapping`)
    inputs = raw.inputs
    for (const value of Object.values(inputs)) {
      if (typeof value === 'string') checkTemplateSyntax(value)
    }
  }
  return useSpec(raw.manifest.trim(), inputs, id, file)
}

/** One checked `use` spec. */
function useSpec(
  manifest: string,
  inputs: Record<string, unknown> | undefined,
  id: string,
  file: string
): UseSpec {
  checkTemplateSyntax(manifest)
  if (!manifest.includes('{{')) {
    const target = resolve(dirname(file), manifest)
    if (!existsSync(target)) {
      throw new Error(`${file}: step "${id}" uses a manifest that does not exist: ${manifest}`)
    }
  }
  return { manifest, inputs }
}

/**
 * Normalize the `run` form into a command spec.
 * @param raw - the `run` value.
 * @param id - owning step id, for messages.
 * @param file - manifest path, for messages.
 * @returns the command spec.
 */
function normalizeRun(raw: unknown, id: string, file: string): RunSpec {
  if (typeof raw === 'string') {
    if (raw.trim() === '') throw new Error(`${file}: step "${id}" has an empty "run" command`)
    for (const template of [raw]) checkTemplateSyntax(template)
    return { command: raw, shell: true }
  }
  if (Array.isArray(raw)) {
    if (raw.length === 0) throw new Error(`${file}: step "${id}" has an empty "run" list`)
    const command = raw.map(entry => {
      if (typeof entry !== 'string') throw new Error(`${file}: step "${id}" "run" list entries must be strings`)
      checkTemplateSyntax(entry)
      return entry
    })
    return { command }
  }
  if (!isRecord(raw)) throw new Error(`${file}: step "${id}" "run" must be a string, a list, or a mapping`)
  const command = raw.command
  if (typeof command === 'string') checkTemplateSyntax(command)
  else if (Array.isArray(command)) {
    for (const entry of command) {
      if (typeof entry !== 'string') throw new Error(`${file}: step "${id}" "run.command" list entries must be strings`)
      checkTemplateSyntax(entry)
    }
  } else throw new Error(`${file}: step "${id}" "run.command" must be a string or a list`)
  const cwd = optionalString(raw.cwd, `${file}: step "${id}" has a non-string "run.cwd"`)
  if (cwd !== undefined) checkTemplateSyntax(cwd)
  return {
    command,
    cwd,
    env: optionalEnv(raw.env, `${file}: step "${id}" "run.env" must map names to strings`),
    timeoutMs: optionalNumber(raw.timeoutMs, `${file}: step "${id}" has a non-numeric "run.timeoutMs"`),
    shell: raw.shell === true || typeof raw.shell === 'string' ? raw.shell as boolean | string : undefined,
    textOnly: raw.textOnly === true,
  }
}

/**
 * Normalize the `map` form.
 * @param raw - the `map` value.
 * @param id - owning step id, for messages.
 * @param file - manifest path, for messages.
 * @param warnings - collector for non-fatal findings.
 * @returns the fan-out spec.
 */
function normalizeMap(raw: unknown, id: string, file: string, warnings: string[]): MapSpec {
  if (!isRecord(raw)) throw new Error(`${file}: step "${id}" "map" must be a mapping`)
  requireString(raw.over, `${file}: step "${id}" "map.over" is required`)
  checkTemplateSyntax(`{{ ${raw.over as string} }}`)
  if (!Array.isArray(raw.steps) || raw.steps.length === 0) {
    throw new Error(`${file}: step "${id}" "map.steps" must be a non-empty list`)
  }
  const concurrency = optionalNumber(raw.concurrency, `${file}: step "${id}" has a non-numeric "map.concurrency"`)
  if (concurrency !== undefined && concurrency < 1) throw new Error(`${file}: step "${id}" "map.concurrency" must be >= 1`)
  return {
    over: raw.over as string,
    as: optionalString(raw.as, `${file}: step "${id}" has a non-string "map.as"`),
    concurrency,
    steps: normalizeSteps(raw.steps, file, warnings, `map "${id}" `),
  }
}

/** Keys understood inside an `ai` step. */
const ALLOWED_AI_KEYS = new Set([
  'prompt', 'system', 'schema', 'model', 'temperature', 'maxTokens', 'backend', 'command', 'timeoutMs',
  'images', 'cache',
])

/**
 * Normalize the `ai` form.
 * @param raw - the `ai` value.
 * @param id - owning step id, for messages.
 * @param file - manifest path, for messages.
 * @param warnings - collector for non-fatal findings.
 * @returns the model-call spec.
 */
function normalizeAi(raw: unknown, id: string, file: string, warnings: string[]): AiSpec {
  if (!isRecord(raw)) throw new Error(`${file}: step "${id}" "ai" must be a mapping`)
  for (const key of Object.keys(raw)) {
    if (!ALLOWED_AI_KEYS.has(key)) warnings.push(`${file}: step "${id}" "ai" has unknown key "${key}" (ignored)`)
  }
  requireString(raw.prompt, `${file}: step "${id}" "ai.prompt" is required`)
  const prompt = raw.prompt as string
  checkTemplateSyntax(prompt)
  const system = optionalString(raw.system, `${file}: step "${id}" has a non-string "ai.system"`)
  if (system !== undefined) checkTemplateSyntax(system)

  let schema: Record<string, unknown> | undefined
  if (raw.schema !== undefined && raw.schema !== null) {
    const problems = validateSchema(raw.schema, `${file}: step "${id}" ai.schema`)
    if (problems.length > 0) throw new Error(problems.join('\n'))
    schema = raw.schema as Record<string, unknown>
  }

  const backend = raw.backend
  if (backend !== undefined && backend !== null && backend !== 'http' && backend !== 'cli') {
    throw new Error(`${file}: step "${id}" "ai.backend" must be "http" or "cli"`)
  }
  const command = normalizeCommand(raw.command, id, file)
  if (backend === 'cli' && command === undefined) {
    throw new Error(`${file}: step "${id}" uses "ai.backend: cli" and needs "ai.command"`)
  }
  if (command !== undefined && backend !== 'cli') {
    warnings.push(`${file}: step "${id}" has "ai.command" but its backend is not "cli" (ignored)`)
  }

  const temperature = optionalNumber(raw.temperature, `${file}: step "${id}" has a non-numeric "ai.temperature"`)
  if (temperature !== undefined && (temperature < 0 || temperature > 2)) {
    throw new Error(`${file}: step "${id}" "ai.temperature" must be between 0 and 2`)
  }
  const maxTokens = optionalNumber(raw.maxTokens, `${file}: step "${id}" has a non-numeric "ai.maxTokens"`)
  if (maxTokens !== undefined && (!Number.isInteger(maxTokens) || maxTokens < 1)) {
    throw new Error(`${file}: step "${id}" "ai.maxTokens" must be a positive integer`)
  }
  const timeoutMs = optionalNumber(raw.timeoutMs, `${file}: step "${id}" has a non-numeric "ai.timeoutMs"`)
  if (timeoutMs !== undefined && timeoutMs < 1) throw new Error(`${file}: step "${id}" "ai.timeoutMs" must be positive`)

  const images = normalizeAiImages(raw.images, id, file)
  const cache = normalizeAiCache(raw.cache, id, file)
  if (cache === false) {
    warnings.push(
      `${file}: step "${id}" 的 ai.cache 是 false：每次运行都会调用模型，记录不复用（验证码这类每次都变的判断点就该这样）`,
    )
  }

  return {
    prompt,
    system,
    schema,
    model: optionalString(raw.model, `${file}: step "${id}" has a non-string "ai.model"`),
    temperature,
    maxTokens,
    images,
    cache,
    backend: backend as AiSpec['backend'],
    command,
    timeoutMs,
  }
}

/**
 * Normalize `ai.images`.
 * @param raw - the manifest value.
 * @param id - owning step id, for messages.
 * @param file - manifest path, for messages.
 * @returns the list of path templates, or undefined.
 */
function normalizeAiImages(raw: unknown, id: string, file: string): string[] | undefined {
  if (raw === undefined || raw === null) return undefined
  if (!Array.isArray(raw)) throw new Error(`${file}: step "${id}" "ai.images" must be a list of paths`)
  if (raw.length === 0) throw new Error(`${file}: step "${id}" "ai.images" is an empty list`)
  for (const entry of raw) {
    if (typeof entry !== 'string' || entry.trim() === '') {
      throw new Error(`${file}: step "${id}" "ai.images" entries must be non-empty strings`)
    }
    checkTemplateSyntax(entry)
  }
  return raw as string[]
}

/**
 * Normalize `ai.cache`.
 *
 * Only `true` and `false` are accepted: leaving it out means "replay when the
 * question is unchanged", which is the engine default and the reason a settled
 * workflow costs nothing.
 * @param raw - the manifest value.
 * @param id - owning step id, for messages.
 * @param file - manifest path, for messages.
 * @returns the flag, or undefined when the step follows the default.
 */
function normalizeAiCache(raw: unknown, id: string, file: string): boolean | undefined {
  if (raw === undefined || raw === null) return undefined
  if (raw !== true && raw !== false) throw new Error(`${file}: step "${id}" "ai.cache" must be true or false`)
  return raw
}

/**
 * Normalize an external backend command.
 * @param raw - the `ai.command` value.
 * @param id - owning step id, for messages.
 * @param file - manifest path, for messages.
 * @returns the command, or undefined.
 */
function normalizeCommand(raw: unknown, id: string, file: string): string | string[] | undefined {
  if (raw === undefined || raw === null) return undefined
  if (typeof raw === 'string') {
    if (raw.trim() === '') throw new Error(`${file}: step "${id}" has an empty "ai.command"`)
    return raw
  }
  if (Array.isArray(raw)) {
    if (raw.length === 0) throw new Error(`${file}: step "${id}" has an empty "ai.command" list`)
    for (const entry of raw) {
      if (typeof entry !== 'string') throw new Error(`${file}: step "${id}" "ai.command" list entries must be strings`)
    }
    return raw as string[]
  }
  throw new Error(`${file}: step "${id}" "ai.command" must be a string or a list`)
}

/**
 * Report `ai` steps that do not say what pins them down.
 *
 * A model call without a `why` is a judgment nobody explained, and one without
 * a `schema` has no contract, so it cannot be swapped for a program later. Both
 * are errors under the default strict mode; `--no-strict` is the escape hatch
 * while drafting a manifest.
 * @param workflow - the loaded workflow.
 * @returns one message per missing field, empty when everything is declared.
 */
export function strictFindings(workflow: Workflow): string[] {
  const findings: string[] = []
  const walk = (steps: Step[]): void => {
    for (const step of steps) {
      if (step.ai !== undefined) {
        if (step.why === undefined || step.why.trim() === '') {
          findings.push(`step "${step.id}": ai 步骤需要 "why" 说明为什么必须问模型`)
        }
        if (step.ai.schema === undefined) {
          findings.push(`step "${step.id}": ai 步骤需要 "schema" 约束输出形状`)
        }
      }
      if (step.map !== undefined) walk(step.map.steps)
    }
  }
  walk(workflow.steps)
  return findings
}

/**
 * Require a non-empty string.
 * @param value - candidate value.
 * @param message - error message when the value is not a string.
 */
function requireString(value: unknown, message: string): void {
  if (typeof value !== 'string' || value.trim() === '') throw new Error(message)
}

/**
 * Accept a string or undefined.
 * @param value - candidate value.
 * @param message - error message when the value is present but not a string.
 * @returns the string, or undefined.
 */
function optionalString(value: unknown, message: string): string | undefined {
  if (value === undefined || value === null) return undefined
  if (typeof value !== 'string') throw new Error(message)
  return value
}

/**
 * Accept a number or undefined.
 * @param value - candidate value.
 * @param message - error message when the value is present but not a number.
 * @returns the number, or undefined.
 */
function optionalNumber(value: unknown, message: string): number | undefined {
  if (value === undefined || value === null) return undefined
  if (typeof value !== 'number' || Number.isNaN(value)) throw new Error(message)
  return value
}

/**
 * Accept an environment mapping or undefined.
 * @param value - candidate value.
 * @param message - error message when the mapping is malformed.
 * @returns the mapping, or undefined.
 */
function optionalEnv(value: unknown, message: string): Record<string, string> | undefined {
  if (value === undefined || value === null) return undefined
  if (!isRecord(value)) throw new Error(message)
  const out: Record<string, string> = {}
  for (const [key, entry] of Object.entries(value)) {
    if (typeof entry !== 'string') throw new Error(message)
    out[key] = entry
  }
  return out
}

/**
 * Report whether a value is a plain object.
 * @param value - candidate value.
 * @returns true for non-array objects.
 */
function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}
