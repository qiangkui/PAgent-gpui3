/**
 * Step contract tests.
 *
 * A step is only safe to swap between a program and a model call when its
 * output contract is pinned. Each `fixtures/<step-id>.in.json` feeds a scope and
 * `fixtures/<step-id>.out.json` holds the expected primary output, so a step can
 * be regression-tested without running the whole workflow or touching the
 * network. An `ai` step also needs `fixtures/<step-id>.ai.json` — the model's
 * recorded reply — because a contract test must never depend on a live model.
 */

import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { createMockBackend } from './backend.ts'
import { runOneStep } from './engine.ts'
import type { RunOptions } from './engine.ts'
import type { LoadedWorkflow } from './load.ts'
import type { Scope, Step, StepResult } from './types.ts'

/** Outcome of one step's contract test. */
export interface FixtureOutcome {
  /** Step id. */
  id: string
  /** `passed`, `failed`, or `uncovered` when no fixture exists. */
  state: 'passed' | 'failed' | 'uncovered'
  /** Location of the expected output, when a fixture exists. */
  fixture?: string
  /** What differed, when the comparison failed. */
  detail?: string
}

/** Aggregate contract-test result. */
export interface FixtureReport {
  /** Per-step outcomes. */
  outcomes: FixtureOutcome[]
  /** Counts for the summary line. */
  totals: { passed: number, failed: number, uncovered: number }
}

/** Input fixture shape. */
interface InputFixture {
  /** Input values to expose. */
  inputs?: Record<string, unknown>
  /** Variable values to expose. */
  vars?: Record<string, unknown>
  /** Step results to expose, keyed by id. */
  steps?: Record<string, StepResult>
  /** Fan-out element. */
  item?: unknown
  /** Fan-out index. */
  index?: number
}

/**
 * Run every step's contract test.
 * @param loaded - the loaded manifest.
 * @param options - how to record progress.
 * @param update - rewrite expected outputs from this run.
 * @returns the aggregate report.
 */
export async function runFixtures(
  loaded: LoadedWorkflow,
  options: { log: (line: string) => void, update: boolean, filter?: string },
): Promise<FixtureReport> {
  const fixtureDir = join(loaded.dir, 'fixtures')
  const steps = flattenSteps(loaded.workflow.steps)
  const selected = options.filter === undefined ? steps : steps.filter(step => step.id === options.filter)
  if (selected.length === 0) throw new Error(`no step matches "${options.filter ?? ''}"`)

  const outcomes: FixtureOutcome[] = []
  for (const step of selected) {
    const expectedPath = join(fixtureDir, `${step.id}.out.json`)
    if (!options.update && !existsSync(expectedPath)) {
      outcomes.push({ id: step.id, state: 'uncovered', detail: `no ${step.id}.out.json` })
      continue
    }
    // An `ai` step is tested against a recorded reply, never the network.
    const reply = step.ai === undefined ? undefined : readAiReply(fixtureDir, step.id)
    if (step.ai !== undefined && reply === undefined) {
      const hint = `ai 步骤需要 fixtures/${step.id}.ai.json 记录模型的回复`
      if (options.update) throw new Error(`${step.id}: ${hint}`)
      outcomes.push({ id: step.id, state: 'uncovered', detail: hint })
      continue
    }
    const scope = buildScope(loaded, fixtureDir, step.id)
    const result = await runOneStep(step, scope, testRunOptions(loaded, options.log, reply))
    const actual = primaryOutput(result)
    if (options.update) {
      mkdirSync(fixtureDir, { recursive: true })
      writeFileSync(expectedPath, `${JSON.stringify(actual, null, 2)}\n`, 'utf8')
      options.log(`recorded ${relative(loaded.dir, expectedPath)}`)
      outcomes.push({ id: step.id, state: 'passed', fixture: expectedPath })
      continue
    }
    const expected: unknown = JSON.parse(readFileSync(expectedPath, 'utf8'))
    if (deepEqual(expected, actual)) {
      outcomes.push({ id: step.id, state: 'passed', fixture: expectedPath })
      continue
    }
    outcomes.push({
      id: step.id,
      state: 'failed',
      fixture: expectedPath,
      detail: `expected ${JSON.stringify(expected)}\n  actual ${JSON.stringify(actual)}`,
    })
  }

  const totals = {
    passed: outcomes.filter(outcome => outcome.state === 'passed').length,
    failed: outcomes.filter(outcome => outcome.state === 'failed').length,
    uncovered: outcomes.filter(outcome => outcome.state === 'uncovered').length,
  }
  return { outcomes, totals }
}

/**
 * Build a scope from a fixture input file, falling back to declared defaults.
 * @param loaded - the loaded manifest.
 * @param fixtureDir - the fixture directory.
 * @param stepId - the step under test.
 * @returns the scope.
 */
function buildScope(loaded: LoadedWorkflow, fixtureDir: string, stepId: string): Scope {
  const inputPath = join(fixtureDir, `${stepId}.in.json`)
  const fixture: InputFixture = existsSync(inputPath)
    ? JSON.parse(readFileSync(inputPath, 'utf8')) as InputFixture
    : {}
  const inputs: Record<string, unknown> = {}
  for (const [name, spec] of Object.entries(loaded.workflow.inputs)) inputs[name] = spec.default
  Object.assign(inputs, fixture.inputs ?? {})
  return {
    inputs,
    vars: { ...loaded.workflow.vars, ...fixture.vars },
    steps: fixture.steps ?? {},
    env: process.env,
    item: fixture.item,
    index: fixture.index,
  }
}

/**
 * Build engine options for a contract test.
 * @param loaded - the loaded manifest.
 * @param log - progress sink.
 * @param reply - recorded model answer, for an `ai` step.
 * @returns the run options.
 */
function testRunOptions(loaded: LoadedWorkflow, log: (line: string) => void, reply?: string): RunOptions {
  return {
    loaded,
    inputs: {},
    log,
    // Always ask the recorded reply: a contract test must not read the
    // decision cache, or an old answer would hide a broken prompt.
    level: 'draft',
    backend: reply === undefined ? undefined : createMockBackend([reply]),
    // A recorded fixture reply must never become a run's "decided" answer.
    cache: false,
  }
}

/**
 * Read the recorded model answer of an `ai` step.
 * @param fixtureDir - the fixture directory.
 * @param stepId - the step id.
 * @returns the answer text, or undefined when no fixture exists.
 */
function readAiReply(fixtureDir: string, stepId: string): string | undefined {
  const path = join(fixtureDir, `${stepId}.ai.json`)
  if (!existsSync(path)) return undefined
  const parsed: unknown = JSON.parse(readFileSync(path, 'utf8'))
  if (typeof parsed === 'string') return parsed
  if (parsed !== null && typeof parsed === 'object' && !Array.isArray(parsed)) {
    const record = parsed as Record<string, unknown>
    if (typeof record.text === 'string') return record.text
    if ('json' in record) return JSON.stringify(record.json)
  }
  return JSON.stringify(parsed)
}

/**
 * The value a fixture pins: parsed JSON when present, otherwise raw stdout.
 * @param result - the settled step result.
 * @returns the comparable output.
 */
function primaryOutput(result: StepResult): unknown {
  if (result.status !== 'ok') return { status: result.status, error: result.error }
  if (result.kind === 'map') return result.items
  if (result.kind === 'ai') return result.json !== undefined ? result.json : (result.text ?? null)
  if (result.json !== undefined) return result.json
  return result.stdout ?? null
}

/**
 * Collect every step, including those nested inside `map` bodies.
 * @param steps - the step list to walk.
 * @returns the flattened list.
 */
function flattenSteps(steps: Step[]): Step[] {
  const out: Step[] = []
  for (const step of steps) {
    out.push(step)
    if (step.map !== undefined) out.push(...flattenSteps(step.map.steps))
  }
  return out
}

/**
 * Compare two JSON values structurally.
 * @param left - expected value.
 * @param right - actual value.
 * @returns true when they match.
 */
function deepEqual(left: unknown, right: unknown): boolean {
  if (left === right) return true
  if (typeof left !== typeof right) return false
  if (Array.isArray(left) && Array.isArray(right)) {
    return left.length === right.length && left.every((entry, index) => deepEqual(entry, right[index]))
  }
  if (left && right && typeof left === 'object' && typeof right === 'object') {
    const leftEntries = Object.entries(left as Record<string, unknown>)
    const rightRecord = right as Record<string, unknown>
    if (leftEntries.length !== Object.keys(rightRecord).length) return false
    return leftEntries.every(([key, value]) => deepEqual(value, rightRecord[key]))
  }
  return false
}

/**
 * Render a path relative to a base directory for messages.
 * @param base - the base directory.
 * @param path - the path to render.
 * @returns the relative path with forward slashes.
 */
function relative(base: string, path: string): string {
  return path.startsWith(base) ? path.slice(base.length + 1).replaceAll('\\', '/') : path
}
