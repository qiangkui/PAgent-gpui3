#!/usr/bin/env node
/**
 * Command line entry: `run`, `validate`, `explain`, `test`, `init`.
 *
 * Human output goes to stdout and failures to stderr, so `--json` output can be
 * piped without mixing in progress lines.
 */

import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import process from 'node:process'
import { createInterface } from 'node:readline'
import { askModel, listModels } from './ai.ts'
import type { AiRequest } from './ai.ts'
import { roundCount, runAgent } from './agent.ts'
import type { ApprovalRequest } from './agent.ts'
import { cacheHitSize } from './stream.ts'
import { webEventRow, webReport, webSteps } from './web.ts'
import { deriveWorkflow } from './derive.ts'
import { describeStep, resolveInputs, runWorkflow } from './engine.ts'
import type { RunOptions, RunSummary } from './engine.ts'
import { runFixtures } from './fixtures.ts'
import { loadWorkflow, strictFindings } from './load.ts'
import type { LoadedWorkflow } from './load.ts'
import { loadPlugins } from './plugins.ts'
import { compactLimits, estimateTokens, summarizeRange } from './compact.ts'
import { createMcpHub } from './mcp.ts'
import { renderReport } from './report.ts'
import { describeRuntime, resolveRuntime } from './runtime.ts'
import type { RunLevel, Scope, Step, TraceRecord } from './types.ts'

/** Parsed command line. */
interface ParsedArgs {
  /** Subcommand name. */
  command: string
  /** Positional arguments after the subcommand. */
  positionals: string[]
  /** Boolean or valued flags, keyed without leading dashes. */
  flags: Map<string, string | true>
  /** Repeated `--input name=value` pairs. */
  inputs: Record<string, string>
}

const USAGE = `dsh-flow — deterministic-first workflow runner

Usage:
  dsh-flow run <manifest> [options]
  dsh-flow validate <manifest>
  dsh-flow explain <manifest> [--input name=value]...
  dsh-flow test <manifest> [--update] [--filter <step-id>]
  dsh-flow init <directory> [--lang python|node]
  dsh-flow derive <session.jsonl> [--out <dir>] [--name <name>]
  dsh-flow ai                       (one JSON request on stdin, JSON answer on stdout)
  dsh-flow models                   (list a provider's models: JSON in, JSON out)
  dsh-flow mcp-skills               (connect one MCP server and write its skills: JSON in, JSON out)
  dsh-flow compact                  (condense one conversation span: JSON in, JSON out)

Options for run:
  --input name=value   supply a declared input (repeatable)
  --level <mode>       manual | assist | draft (default: assist)
                       manual = never call a model, replay recorded decisions
                       assist = replay recorded decisions, ask only when missing
                       draft  = always ask, then record
  --no-strict          allow ai steps without "why" or "schema" (drafting only)
  --credentials-stdin  read {"apiKey","baseUrl","model"} from stdin for ai steps
  --dry                render steps without executing them
  --report <file>      write a self-contained HTML report
  --json               print the run summary as JSON
  --quiet              only print failures

Manifest: a .yml/.yaml/.json file; steps are real programs receiving JSON on
stdin and writing JSON on stdout, or explicit "ai" steps that ask a model and
validate the answer against a schema. Run "dsh-flow init <dir>" for a working
starting point.
`

/**
 * Run the CLI.
 * @param argv - arguments after the executable.
 * @returns the process exit code.
 */
async function main(argv: string[]): Promise<number> {
  let parsed: ParsedArgs
  try {
    parsed = parseArgs(argv)
  } catch (error) {
    process.stderr.write(`${messageOf(error)}\n\n${USAGE}`)
    return 2
  }
  if (parsed.command === 'help' || parsed.command === '--help' || parsed.command === '-h' || parsed.command === '') {
    process.stdout.write(USAGE)
    return 0
  }
  try {
    if (parsed.command === 'run') return await commandRun(parsed)
    if (parsed.command === 'validate') return commandValidate(parsed)
    if (parsed.command === 'explain') return commandExplain(parsed)
    if (parsed.command === 'test') return await commandTest(parsed)
    if (parsed.command === 'init') return commandInit(parsed)
    if (parsed.command === 'derive') return commandDerive(parsed)
    if (parsed.command === 'ai') return await commandAi()
    if (parsed.command === 'models') return await commandModels()
    if (parsed.command === 'mcp-skills') return await commandMcpSkills()
    if (parsed.command === 'compact') return await commandCompact()
    process.stderr.write(`unknown command "${parsed.command}"\n\n${USAGE}`)
    return 2
  } catch (error) {
    process.stderr.write(`${messageOf(error)}\n`)
    return parsed.command === 'run' ? 1 : 2
  }
}

/**
 * `run`: execute a manifest.
 * @param parsed - the parsed command line.
 * @returns the exit code.
 */
async function commandRun(parsed: ParsedArgs): Promise<number> {
  const loaded = loadWorkflow(requireManifest(parsed))
  const quiet = parsed.flags.has('quiet')
  const asJson = parsed.flags.has('json')
  const level = parseLevel(parsed)
  const strict = !parsed.flags.has('no-strict')
  const log = (line: string): void => {
    if (quiet && !line.includes('FAIL')) return
    // With `--json`, stdout carries only the summary; progress (including
    // service output) goes to stderr, which the window shows in its log pane.
    const sink = asJson ? process.stderr : process.stdout
    sink.write(`${line}\n`)
  }
  for (const warning of loaded.warnings) process.stderr.write(`warning: ${warning}\n`)
  if (strict) {
    for (const finding of strictFindings(loaded.workflow)) process.stderr.write(`warning: ${finding}\n`)
  }

  const runsDir = join(loaded.dir, '.dsh-flow', 'runs')
  const trace: TraceRecord[] = []
  const options: RunOptions = {
    loaded,
    inputs: parsed.inputs,
    dryRun: parsed.flags.has('dry'),
    level,
    onceLedger: join(loaded.dir, '.dsh-flow', 'ledger.json'),
    forceOnce: parsed.flags.has('force'),
    aiEnv: await readCredentials(parsed.flags.has('credentials-stdin')),
    log,
    trace: record => trace.push(record),
  }

  if (!asJson) process.stdout.write(`▶ ${loaded.workflow.name} — ${loaded.workflow.description}\n`)
  const summary = await runWorkflow(options)

  mkdirSync(runsDir, { recursive: true })
  writeFileSync(join(runsDir, `${summary.runId}.jsonl`), trace.map(record => JSON.stringify(record)).join('\n') + '\n', 'utf8')

  const reportFlag = parsed.flags.get('report')
  const reportPath = typeof reportFlag === 'string' ? resolve(reportFlag) : undefined
  const fallbackReport = join(runsDir, `${summary.runId}.html`)
  if (reportPath !== undefined) {
    mkdirSync(dirname(reportPath), { recursive: true })
    writeFileSync(reportPath, renderReport(loaded, summary), 'utf8')
  } else if (summary.status === 'failed') {
    writeFileSync(fallbackReport, renderReport(loaded, summary), 'utf8')
  }

  if (asJson) {
    process.stdout.write(`${JSON.stringify(summary, null, 2)}\n`)
  } else {
    const kinds = countKinds(loaded.workflow.steps)
    const total = kinds.run + kinds.set + kinds.map + kinds.ai
    process.stdout.write(`\n${summary.status === 'ok' ? 'done' : 'FAILED'} in ${summary.durationMs} ms · ${total} steps · ${total - kinds.ai} deterministic · ${kinds.ai} ai (${level})\n`)
    if (reportPath !== undefined) process.stdout.write(`report: ${reportPath}\n`)
    else if (summary.status === 'failed') process.stdout.write(`report: ${fallbackReport}\n`)
  }
  return summary.status === 'ok' ? 0 : 1
}

/**
 * Read the `--level` flag.
 * @param parsed - the parsed command line.
 * @returns the requested level, `assist` when absent.
 */
function parseLevel(parsed: ParsedArgs): RunLevel {
  const raw = parsed.flags.get('level')
  if (raw === undefined) return 'assist'
  if (raw === 'manual' || raw === 'assist' || raw === 'draft') return raw
  throw new Error(`--level must be manual, assist, or draft (got "${String(raw)}")`)
}

/**
 * Read the endpoint credentials the window hands over on stdin.
 *
 * The key never appears on the command line, so it cannot leak through the
 * process list or the log pane. An empty payload (or a missing flag) means the
 * run falls back to the `DEEPSEEK_*` environment variables.
 * @param enabled - whether `--credentials-stdin` was given.
 * @returns environment additions for the HTTP backend, or undefined.
 */
async function readCredentials(enabled: boolean): Promise<Record<string, string> | undefined> {
  if (!enabled) return undefined
  const raw = (await readStdin()).trim()
  if (raw === '') return undefined
  let parsed: unknown
  try {
    parsed = JSON.parse(raw)
  } catch {
    throw new Error('--credentials-stdin 需要一份 JSON：{"apiKey": "…", "baseUrl": "…", "model": "…"}')
  }
  if (!isRecord(parsed)) throw new Error('--credentials-stdin 需要 JSON 对象')
  const apiKey = text(parsed.apiKey)
  if (apiKey === '') throw new Error('--credentials-stdin 缺少 apiKey')
  const env: Record<string, string> = { DEEPSEEK_API_KEY: apiKey }
  const baseUrl = text(parsed.baseUrl)
  if (baseUrl !== '') env.DEEPSEEK_BASE_URL = baseUrl
  const model = text(parsed.model)
  if (model !== '') env.DEEPSEEK_MODEL = model
  return env
}

/**
 * Accept a string field, ignoring anything else.
 * @param value - the value from the payload.
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

/**
 * `validate`: parse and statically check a manifest.
 * @param parsed - the parsed command line.
 * @returns the exit code.
 */
function commandValidate(parsed: ParsedArgs): number {
  const loaded = loadWorkflow(requireManifest(parsed))
  for (const warning of loaded.warnings) process.stdout.write(`warning: ${warning}\n`)
  const findings = parsed.flags.has('no-strict') ? [] : strictFindings(loaded.workflow)
  for (const finding of findings) process.stdout.write(`error: ${finding}\n`)
  const problems = missingRuntimes(loaded)
  for (const problem of problems) process.stdout.write(`error: ${problem}\n`)
  if (problems.length > 0 || findings.length > 0) return 2
  const kinds = countKinds(loaded.workflow.steps)
  const ai = kinds.ai === 0 ? 'no model calls' : `${kinds.ai} ai step(s)`
  process.stdout.write(`ok: ${loaded.workflow.name} (${countSteps(loaded.workflow.steps)} steps, ${Object.keys(loaded.workflow.inputs).length} inputs, ${ai})\n`)
  return 0
}

/**
 * `derive`: turn a recorded dsh session into a workflow skeleton.
 *
 * Only commands become steps; judgment points and un-replayable tool calls stay
 * as TODO comments. The manifest goes to stdout (or `<out>/flow.yml`), and the
 * counts go to stderr so the manifest can be piped.
 * @param parsed - the parsed command line.
 * @returns the exit code.
 */
function commandDerive(parsed: ParsedArgs): number {
  const session = parsed.positionals[0]
  if (session === undefined) throw new Error('derive needs a session JSONL path')
  const source = readFileSync(resolve(session), 'utf8')
  const nameFlag = parsed.flags.get('name')
  const descriptionFlag = parsed.flags.get('description')
  const report = deriveWorkflow(source, {
    name: typeof nameFlag === 'string' ? nameFlag : undefined,
    description: typeof descriptionFlag === 'string' ? descriptionFlag : undefined,
  })

  const { summary } = report
  process.stderr.write(
    `derived ${summary.runs} run · ${summary.todos.length} TODO · ${summary.skippedLines} skipped` +
    `${summary.ignoredTurns === 0 ? '' : ` · ${summary.ignoredTurns} later turn(s) not derived`}\n`,
  )
  for (const todo of summary.todos) process.stderr.write(`  TODO ${todo}\n`)

  const outFlag = parsed.flags.get('out')
  if (typeof outFlag === 'string') {
    const dir = resolve(outFlag)
    mkdirSync(dir, { recursive: true })
    const file = join(dir, 'flow.yml')
    writeFileSync(file, report.manifest, 'utf8')
    process.stdout.write(`${file}\n`)
    return 0
  }
  process.stdout.write(report.manifest)
  return 0
}

/**
 * `explain`: show the step plan and how much of it is deterministic.
 * @param parsed - the parsed command line.
 * @returns the exit code.
 */
function commandExplain(parsed: ParsedArgs): number {
  const loaded = loadWorkflow(requireManifest(parsed))
  const inputs = tryResolveInputs(loaded, parsed.inputs)
  const scope: Scope = { inputs, vars: { ...loaded.workflow.vars }, steps: {}, env: process.env }
  const { workflow } = loaded
  process.stdout.write(`${workflow.name} — ${workflow.description}\n`)
  const declared = Object.entries(workflow.inputs)
  process.stdout.write(`inputs: ${declared.length === 0 ? 'none' : declared.map(([name, spec]) => {
    const tags = [spec.required === true ? 'required' : 'optional']
    if (spec.default !== undefined) tags.push(`default=${JSON.stringify(spec.default)}`)
    return `${name} (${tags.join(', ')})`
  }).join(', ')}\n\n`)
  const lines: string[] = []
  const aiSteps: Step[] = []
  const walk = (steps: Step[], depth: number): void => {
    for (const step of steps) {
      const kind = step.run !== undefined ? 'run' : step.set !== undefined ? 'set' : step.ai !== undefined ? 'ai' : 'map'
      if (step.ai !== undefined) aiSteps.push(step)
      const flags = [
        step.when === undefined ? '' : 'when',
        step.skip === true ? 'skip' : '',
        step.allowFailure === true ? 'allowFailure' : '',
        step.save === undefined ? '' : `save=${step.save}`,
      ].filter(entry => entry !== '').join(' ')
      lines.push(`${'  '.repeat(depth)}${kind.padEnd(4)} ${step.id.padEnd(18)} ${describeStep(step, scope, loaded)}${flags === '' ? '' : `   [${flags}]`}`)
      if (step.map !== undefined) walk(step.map.steps, depth + 1)
    }
  }
  walk(workflow.steps, 0)
  process.stdout.write(`${lines.join('\n')}\n\n`)
  const kinds = countKinds(workflow.steps)
  const total = kinds.run + kinds.set + kinds.map + kinds.ai
  process.stdout.write(
    `steps: ${total} · run ${kinds.run}, set ${kinds.set}, map ${kinds.map}, ai ${kinds.ai} · ` +
    `${percent(total - kinds.ai, total)}% executed without a model\n`,
  )
  if (aiSteps.length > 0) {
    process.stdout.write(`\nai 判断点 (${aiSteps.length}/${total})：\n`)
    for (const step of aiSteps) {
      const why = step.why === undefined ? '缺少 why（严格模式会报错）' : step.why
      process.stdout.write(`  ${step.id} — ${why}\n`)
    }
  }
  return 0
}

/**
 * `test`: run step contract fixtures.
 * @param parsed - the parsed command line.
 * @returns the exit code.
 */
async function commandTest(parsed: ParsedArgs): Promise<number> {
  const loaded = loadWorkflow(requireManifest(parsed))
  const filter = parsed.flags.get('filter')
  const report = await runFixtures(loaded, {
    log: line => process.stdout.write(`${line}\n`),
    update: parsed.flags.has('update'),
    filter: typeof filter === 'string' ? filter : undefined,
  })
  for (const outcome of report.outcomes) {
    if (outcome.state === 'failed') process.stdout.write(`FAIL ${outcome.id}\n     ${outcome.detail ?? ''}\n`)
    else if (outcome.state === 'uncovered') process.stdout.write(`---- ${outcome.id} (${outcome.detail ?? 'no fixture'})\n`)
  }
  process.stdout.write(`fixtures: ${report.totals.passed} passed, ${report.totals.failed} failed, ${report.totals.uncovered} uncovered\n`)
  return report.totals.failed === 0 ? 0 : 1
}

/**
 * `init`: scaffold a working workflow directory.
 * @param parsed - the parsed command line.
 * @returns the exit code.
 */
function commandInit(parsed: ParsedArgs): number {
  const target = parsed.positionals[0]
  if (target === undefined) throw new Error('init needs a target directory')
  const langFlag = parsed.flags.get('lang')
  const lang = typeof langFlag === 'string' ? langFlag : 'python'
  if (lang !== 'python' && lang !== 'node') throw new Error(`--lang must be "python" or "node", got "${lang}"`)
  const dir = resolve(target)
  const name = dir.split(/[\\/]/).filter(part => part !== '').at(-1) ?? 'workflow'
  mkdirSync(join(dir, 'steps'), { recursive: true })
  writeFileSync(join(dir, 'flow.yml'), starterManifest(name, lang), 'utf8')
  writeFileSync(join(dir, 'steps', lang === 'python' ? 'greet.py' : 'greet.mjs'), lang === 'python' ? PYTHON_STEP : NODE_STEP, 'utf8')
  process.stdout.write(`created ${join(dir, 'flow.yml')}\n`)
  process.stdout.write(`next: dsh-flow run ${join(dir, 'flow.yml')} --input who=you\n`)
  return 0
}

/**
 * Build the starter manifest.
 * @param name - workflow name.
 * @param lang - step language.
 * @returns manifest text.
 */
function starterManifest(name: string, lang: 'python' | 'node'): string {
  const program = lang === 'python' ? '[python, steps/greet.py]' : '[node, steps/greet.mjs]'
  return `name: ${name}
description: Greet someone, deterministically
version: 1

inputs:
  who:
    description: Name to greet
    default: world

steps:
  - id: greet
    desc: Build the greeting
    run: ${program}
    save: greeting

  - id: show
    run: ${lang === 'python' ? '[python, -c, "import json,sys; print(json.load(sys.stdin)[\'vars\'][\'greeting\'][\'text\'])"]' : '[node, -e, "let s=\'\';process.stdin.on(\'data\',d=>s+=d).on(\'end\',()=>console.log(JSON.parse(s).vars.greeting.text))"]'}
    when: "steps.greet.exitCode == 0"
`
}

const PYTHON_STEP = `"""Example step: read the JSON context on stdin, write JSON on stdout."""
import json
import sys


def main() -> None:
    context = json.loads(sys.stdin.read() or "{}")
    who = context.get("inputs", {}).get("who") or "world"
    print(json.dumps({"text": f"hello, {who}"}, ensure_ascii=False))


if __name__ == "__main__":
    main()
`

const NODE_STEP = `// Example step: read the JSON context on stdin, write JSON on stdout.
let raw = ''
process.stdin.setEncoding('utf8')
process.stdin.on('data', chunk => { raw += chunk })
process.stdin.on('end', () => {
  const context = JSON.parse(raw === '' ? '{}' : raw)
  const who = context.inputs?.who ?? 'world'
  process.stdout.write(JSON.stringify({ text: \`hello, \${who}\` }) + '\\n')
})
`

/**
 * Report `run` steps whose first token matches no executable.
 * @param loaded - the loaded manifest.
 * @returns one message per missing runtime.
 */
function missingRuntimes(loaded: LoadedWorkflow): string[] {
  const problems: string[] = []
  const walk = (steps: Step[]): void => {
    for (const step of steps) {
      if (step.run !== undefined && !step.run.shell) {
        const parts = Array.isArray(step.run.command) ? step.run.command : [step.run.command]
        const first = parts[0] ?? ''
        if (!first.includes('{{') && resolveRuntime(first) === null) problems.push(`step "${step.id}": ${describeRuntime(first)}`)
      }
      if (step.ai !== undefined && step.ai.backend === 'cli' && step.ai.command !== undefined) {
        const command = step.ai.command
        const first = (Array.isArray(command) ? command[0] ?? '' : command.split(/\s+/)[0] ?? '')
        if (first !== '' && !first.includes('{{') && resolveRuntime(first) === null) {
          problems.push(`step "${step.id}" ai.command: ${describeRuntime(first)}`)
        }
      }
      if (step.map !== undefined) walk(step.map.steps)
    }
  }
  walk(loaded.workflow.steps)
  return problems
}

/**
 * Count steps by executor kind, including nested fan-out bodies.
 * @param steps - the step list.
 * @returns counts.
 */
function countKinds(steps: Step[]): { run: number, set: number, map: number, ai: number } {
  const totals = { run: 0, set: 0, map: 0, ai: 0 }
  const walk = (list: Step[]): void => {
    for (const step of list) {
      if (step.run !== undefined) totals.run += 1
      else if (step.set !== undefined) totals.set += 1
      else if (step.ai !== undefined) totals.ai += 1
      else totals.map += 1
      if (step.map !== undefined) walk(step.map.steps)
    }
  }
  walk(steps)
  return totals
}

/**
 * Total step count, including nested fan-out bodies.
 * @param steps - the step list.
 * @returns the count.
 */
function countSteps(steps: Step[]): number {
  return steps.reduce((total, step) => total + 1 + (step.map === undefined ? 0 : countSteps(step.map.steps)), 0)
}

/**
 * Resolve inputs, tolerating missing required values for `explain`.
 * @param loaded - the loaded manifest.
 * @param supplied - raw caller values.
 * @returns the resolved inputs.
 */
function tryResolveInputs(loaded: LoadedWorkflow, supplied: Record<string, string>): Record<string, unknown> {
  try {
    return resolveInputs(loaded, supplied)
  } catch (error) {
    if (error instanceof Error) return supplied
    throw error
  }
}

/**
 * Format a ratio as a whole percentage.
 * @param part - the numerator.
 * @param total - the denominator.
 * @returns the percentage text.
 */
function percent(part: number, total: number): number {
  return total === 0 ? 0 : Math.round((part / total) * 100)
}

/**
 * Require the manifest positional argument.
 * @param parsed - the parsed command line.
 * @returns the manifest path.
 */
function requireManifest(parsed: ParsedArgs): string {
  const file = parsed.positionals[0]
  if (file === undefined) throw new Error(`${parsed.command} needs a manifest path`)
  return file
}

/**
 * Parse argv into the command, positionals, flags, and inputs.
 * @param argv - arguments after the executable.
 * @returns the parsed command line.
 */
function parseArgs(argv: string[]): ParsedArgs {
  const [command = '', ...rest] = argv
  const positionals: string[] = []
  const flags = new Map<string, string | true>()
  const inputs: Record<string, string> = {}
  for (let i = 0; i < rest.length; i += 1) {
    const token = rest[i]!
    if (!token.startsWith('--')) {
      positionals.push(token)
      continue
    }
    const name = token.slice(2)
    const equals = name.indexOf('=')
    if (equals >= 0) {
      flags.set(name.slice(0, equals), name.slice(equals + 1))
      continue
    }
    const next = rest[i + 1]
    if (name === 'input') {
      if (next === undefined) throw new Error('--input needs name=value')
      const pair = splitPair(next)
      inputs[pair[0]] = pair[1]
      i += 1
      continue
    }
    if (next !== undefined && !next.startsWith('--') && VALUED_FLAGS.has(name)) {
      flags.set(name, next)
      i += 1
      continue
    }
    flags.set(name, true)
  }
  return { command, positionals, flags, inputs }
}

/** Flags that consume the following token as their value. */
const VALUED_FLAGS = new Set(['report', 'filter', 'lang', 'level', 'out', 'name', 'description'])

/**
 * Split `name=value`.
 * @param text - the pair.
 * @returns name and value.
 */
function splitPair(text: string): [string, string] {
  const at = text.indexOf('=')
  if (at <= 0) throw new Error(`expected name=value, got "${text}"`)
  return [text.slice(0, at), text.slice(at + 1)]
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
 * `mcp-skills`: connect one catalog server, write its skills, JSON out.
 *
 * The window uses this when someone presses 「获取技能」 on an MCP card, so the
 * count and the skill panel can be filled without asking the model a question.
 * @returns the exit code.
 */
async function commandMcpSkills(): Promise<number> {
  let raw = ''
  try {
    raw = readFileSync(0, 'utf8')
  } catch {
    // An empty pipe is reported below.
  }
  let request: { dir?: unknown; id?: unknown }
  try {
    request = JSON.parse(raw) as { dir?: unknown; id?: unknown }
  } catch {
    process.stdout.write(`${JSON.stringify({ ok: false, text: 'stdin 不是合法 JSON', tools: [] })}\n`)
    return 2
  }
  const dir = typeof request.dir === 'string' ? request.dir.trim() : ''
  const id = typeof request.id === 'string' ? request.id.trim() : ''
  if (dir === '' || id === '') {
    process.stdout.write(`${JSON.stringify({ ok: false, text: '需要 dir 与 id', tools: [] })}\n`)
    return 2
  }
  const hub = await createMcpHub([], { dir })
  try {
    const result = await hub.gateway.run({ action: 'link', server: id })
    process.stdout.write(
      `${JSON.stringify({
        ok: result.ok,
        text: result.text,
        tools: hub.tools.map((tool) => ({
          name: tool.original,
          exposed: tool.name,
          description: tool.description
        }))
      })}\n`
    )
    return result.ok ? 0 : 1
  } finally {
    await hub.close()
  }
}

/**
 * `ai`: answer one question from stdin, JSON in and JSON out.
 *
 * Answers go to stdout even when they are failures, so the caller only has to
 * parse one stream; the exit code still reports success. The answer arrives
 * piece by piece as `{"type":"delta","text":…}` lines, and with `tools: true`
 * every tool call is reported as its own line before the final answer.
 * @returns the exit code.
 */
async function commandAi(): Promise<number> {
  // One JSON request arrives first; when it says `interactive`, the same pipe
  // then carries `{"type":"approval",…}` decisions for writes.
  const input = createInterface({ input: process.stdin, crlfDelay: Infinity })
  let buffered = ''
  let settled = false
  let resolveRequest: (raw: string | null) => void = () => undefined
  const ready = new Promise<string | null>((resolve) => {
    resolveRequest = resolve
  })
  const approvals = new Map<string, (allow: boolean) => void>()
  const questions = new Map<string, (answer: string) => void>()
  const plans = new Map<string, (approved: boolean) => void>()

  input.on('line', (line) => {
    if (!settled) {
      buffered = buffered === '' ? line : `${buffered}\n${line}`
      if (buffered.trim() === '') return
      try {
        JSON.parse(buffered)
        settled = true
        resolveRequest(buffered)
        buffered = ''
      } catch {
        // Pretty-printed requests span lines; keep reading until one parses.
      }
      return
    }
    let answer: Record<string, unknown> | null = null
    try {
      answer = JSON.parse(line) as Record<string, unknown>
    } catch {
      return
    }
    if (answer?.type !== 'approval') {
      if (answer?.type === 'question') {
        const call = typeof answer.call === 'string' ? answer.call : ''
        const resolve = questions.get(call)
        if (resolve) {
          questions.delete(call)
          resolve(typeof answer.answer === 'string' ? answer.answer : '')
        }
        return
      }
      if (answer?.type === 'plan') {
        const call = typeof answer.call === 'string' ? answer.call : ''
        const resolve = plans.get(call)
        if (resolve) {
          plans.delete(call)
          resolve(answer.approved === true)
        }
        return
      }
      return
    }
    const call = typeof answer.call === 'string' ? answer.call : ''
    const decide = approvals.get(call)
    if (decide) {
      approvals.delete(call)
      decide(answer.allow === true)
    }
  })
  // Closing the pipe without a decision must not let a write through.
  input.on('close', () => {
    if (!settled) {
      settled = true
      resolveRequest(buffered.trim() === '' ? null : buffered)
    }
    for (const decide of approvals.values()) decide(false)
    approvals.clear()
    for (const resolve of questions.values()) resolve('')
    questions.clear()
    for (const resolve of plans.values()) resolve(false)
    plans.clear()
  })

  const raw = await ready
  if (raw === null) {
    process.stdout.write(`${JSON.stringify({ error: 'stdin 是空的' })}\n`)
    input.close()
    return 2
  }
  let request: AiRequest
  try {
    request = JSON.parse(raw) as AiRequest
  } catch {
    process.stdout.write(`${JSON.stringify({ error: 'stdin 不是合法 JSON' })}\n`)
    input.close()
    return 2
  }

  const onDelta = (text: string): void => {
    process.stdout.write(`${JSON.stringify({ type: 'delta', text })}\n`)
  }
  // Thinking reaches the window live so it can be shown while it happens, and
  // the final answer repeats the whole text so a dropped fragment cannot lose it.
  const onReasoning = (text: string): void => {
    process.stdout.write(`${JSON.stringify({ type: 'reasoning', text })}\n`)
  }
  const interactive = request.interactive === true
  const confirmWrites = request.confirmWrites === true
  const ask = async (question: ApprovalRequest): Promise<boolean> => {
    const decision = new Promise<boolean>((resolve) => approvals.set(question.call, resolve))
    process.stdout.write(
      `${JSON.stringify({ type: 'approval', auto: false, ...question })}\n`
    )
    return await decision
  }
  // Writes wait only when the window asked for confirmation; otherwise the
  // event is still reported so the card and its undo data reach the transcript.
  // A one-shot run that asked for confirmation has nobody to ask, so its writes
  // are refused rather than run unapproved (see `requireWriteApproval`).
  const approve = interactive && confirmWrites ? ask : undefined
  // A command or a move / delete always waits, in both modes: it can do far
  // more than one write.
  const confirmActions = interactive ? ask : undefined
  const onApproval =
    interactive && !confirmWrites
      ? (question: ApprovalRequest): void => {
          process.stdout.write(
            `${JSON.stringify({ type: 'approval', auto: true, ...question })}\n`
          )
        }
      : undefined
  // A structured question and a plan review use the same pipe as approvals:
  // they are emitted once and resolved by the matching typed reply.
  const askQuestion = interactive
    ? async (question: {
        call: string
        question: string
        options: string[]
        allowCustom: boolean
      }): Promise<string> => {
        const answer = new Promise<string>((resolve) => questions.set(question.call, resolve))
        process.stdout.write(`${JSON.stringify({ type: 'question', ...question })}\n`)
        return await answer
      }
    : undefined
  const reviewPlan = interactive
    ? async (request: { call: string; plan: string }): Promise<boolean> => {
        const approved = new Promise<boolean>((resolve) => plans.set(request.call, resolve))
        process.stdout.write(`${JSON.stringify({ type: 'plan', ...request })}\n`)
        return await approved
      }
    : undefined
  const onTodo = (items: Array<{ text: string; status: string }>): void => {
    process.stdout.write(`${JSON.stringify({ type: 'todo', items })}\n`)
  }
  const scopeRoot =
    request.fileScope === 'full'
      ? null
      : request.fileScope === 'workspace'
        ? (request.scopeRoot ?? process.cwd())
        : process.cwd()
  // Commands are checked against the workspace plus the folder allowlist; the
  // full-access modes pass no sandbox, which is what makes them unrestricted.
  const allowPaths = (request.allowPaths ?? [])
    .map((entry) => entry.trim())
    .filter((entry) => entry !== '')
    .map((entry) => resolve(process.cwd(), entry))
  const sandbox = scopeRoot === null ? null : { roots: [scopeRoot, ...allowPaths] }
  // Plugins are read once per question, so a folder dropped into place is live
  // on the next one. A plugin that fails to load is reported, never fatal.
  const plugins = request.tools
    ? await loadPlugins({ dir: request.pluginDir, enabled: request.plugins })
    : { tools: [], prompts: [], panels: [], errors: [] }
  for (const failure of plugins.errors) {
    process.stderr.write(`[plugin ${failure.id}] ${failure.message}\n`)
  }
  // MCP servers are opened once per question, after the plugins, so a server
  // that fails costs only its own tools. The hub also offers the `mcp` tool,
  // which can link a server by id or by an inline command / URL.
  const mcp = await createMcpHub(request.tools ? (request.mcpServers ?? []) : [], {
    dir: request.tools ? request.mcpDir : undefined
  })
  for (const failure of mcp.errors) {
    process.stderr.write(`[mcp ${failure.id}] ${failure.message}\n`)
  }

  try {
    if (request.tools) {
      const result = await runAgent(request, {
        root: process.cwd(),
        scopeRoot,
        sandbox,
        readOnlyRoots: request.readOnlyRoots,
        windowTokens: request.windowTokens,
        plugins: plugins.tools,
        panels: plugins.panels,
        mcpTools: [...mcp.tools, ...(request.tools && (request.mcpServers?.length ?? 0) > 0 ? [mcp.gateway] : [])],
        promptInjections: plugins.prompts,
        plan: request.plan === true,
        askUser: askQuestion,
        reviewPlan,
        onTodo,
        approve,
        // Confirmation was asked for, but there is no window to ask through.
        requireWriteApproval: confirmWrites && !interactive,
        onApproval,
        confirmActions,
        onStep: (step) => {
          process.stdout.write(
            `${JSON.stringify({
              type: 'step',
              tool: step.tool,
              target: step.target,
              ok: step.ok,
              detail: step.detail,
              input: step.input,
              output: step.output,
              call: step.call
            })}\n`
          )
        },
        // The strip under the input follows the loop while the answer is still
        // arriving, so the round and the call count go out before each request.
        onProgress: (progress) => {
          process.stdout.write(
            `${JSON.stringify({
              type: 'progress',
              round: progress.round,
              calls: progress.calls
            })}\n`
          )
        },
        onToolStart: (call, tool, target) => {
          process.stdout.write(
            `${JSON.stringify({ type: 'tool_start', call, tool, target })}\n`
          )
        },
        onToolOutput: (call, tool, target, text) => {
          process.stdout.write(
            `${JSON.stringify({ type: 'tool_output', call, tool, target, text })}\n`
          )
        },
        // The window shows how much of the prompt the definitions take, so it
        // needs the very schemas this request carries. Panels are built before
        // the first round and describe this request; a panel that failed is
        // reported on its plugin's card, beside the load failures.
        onTools: (report) => {
          process.stdout.write(
            `${JSON.stringify({
              type: 'tools',
              items: report.items,
              prompts: report.prompts,
              panels: report.panels,
              errors: [...plugins.errors, ...report.panelErrors]
            })}\n`
          )
        },
        onDelta,
        onReasoning
      })
      const answer = { text: result.text, model: result.model, usage: result.usage, context: result.context, rounds: result.rounds, roundTokens: result.roundTokens, roundReasoning: result.roundReasoning, reasoning: result.reasoning, cacheHits: result.cacheHits, webSearch: result.webSearch, serverToolCalls: result.serverCalls, timing: result.timing, compaction: result.compaction }
      process.stdout.write(`${JSON.stringify(answer)}\n`)
      return 0
    }
    const startedAt = Date.now()
    // Live server-tool rows are numbered per question here; the finished rows
    // use the same prefix and order, so each one replaces its running row.
    let liveCalls = 0
    const result = await askModel(request, {
      onDelta,
      onReasoning,
      // One-shot questions have no tool loop either, so a running search is
      // reported here the same way the loop reports a tool it just started.
      onServerEvent: (event) => {
        const row = webEventRow(event, 'web', liveCalls + 1)
        if (row === null) return
        liveCalls += 1
        process.stdout.write(
          `${JSON.stringify({ type: 'tool_start', call: row.call, tool: row.tool, target: row.target })}\n`
        )
      }
    })
    // Server tools run on the provider's side, so no step event came out of a
    // tool loop; the rows are written here, before the answer, so the window
    // shows the search the way it shows a local tool.
    for (const step of webSteps(result.serverCalls, result.annotations, 'web')) {
      process.stdout.write(
        `${JSON.stringify({
          type: 'step',
          tool: step.tool,
          target: step.target,
          ok: step.ok,
          detail: step.detail,
          input: step.input,
          output: step.output,
          call: step.call
        })}\n`
      )
    }
    // One request with no tool loop still reports the same shape the agent
    // does: the window shows the round's prompt and answer beside the bill, so
    // leaving `roundTokens` empty made that request look like it cost nothing.
    const counted = roundCount(result.usage)
    const answer = {
      text: result.text,
      model: result.model,
      usage: result.usage,
      context: result.context,
      rounds: 1,
      roundTokens: counted === null ? [] : [[counted[0], counted[1], 0]],
      // One request, one round: its thinking is the only one there is.
      roundReasoning: [estimateTokens(result.reasoning)],
      reasoning: result.reasoning,
      cacheHits: cacheHitSize(result.usage),
      webSearch: webReport(result.serverCalls, result.annotations),
      serverToolCalls: result.serverCalls,
      // No tool loop, so the whole call was model time; the window still gets
      // the same shape and can show it beside a tool-using request.
      timing: { modelMs: Date.now() - startedAt, toolMs: 0 }
    }
    process.stdout.write(`${JSON.stringify(answer)}\n`)
    return 0
  } catch (error) {
    process.stdout.write(`${JSON.stringify({ error: messageOf(error) })}\n`)
    return 1
  } finally {
    await mcp.close().catch(() => undefined)
    input.close()
  }
}

/**
 * `models`: list what an endpoint offers, JSON in and JSON out.
 *
 * The window uses this so the model versions it shows come from the provider
 * instead of a hard-coded list.
 * @returns the exit code.
 */
async function commandModels(): Promise<number> {
  const raw = await readStdin()
  let request: AiRequest
  try {
    request = JSON.parse(raw) as AiRequest
  } catch {
    process.stdout.write(`${JSON.stringify({ error: 'stdin 不是合法 JSON' })}\n`)
    return 2
  }
  try {
    const result = await listModels(request)
    process.stdout.write(`${JSON.stringify(result)}\n`)
    return 0
  } catch (error) {
    const reason = messageOf(error)
    // Some gateways do not implement `GET /models` even though chat works.
    // Probe the configured model once, so the window can still offer a list
    // instead of leaving the connection unusable.
    const model = (request.model ?? '').trim()
    const key = (request.apiKey ?? '').trim()
    if (model !== '' && key !== '') {
      try {
        await askModel({ ...request, prompt: 'ping', system: '这是一次能力探测，只回复 ok。', maxTokens: 8 })
        process.stdout.write(`${JSON.stringify({ models: [model], degraded: true, reason })}\n`)
        return 0
      } catch {
        // The probe failed too: the original listing error is the useful one.
      }
    }
    process.stdout.write(`${JSON.stringify({ error: reason })}\n`)
    return 1
  }
}

/**
 * `compact`: condense one conversation span into a checkpoint.
 *
 * The window owns the transcript, so it sends the span it wants condensed and
 * gets back the checkpoint text; the last compaction layer is therefore the
 * same code the engine uses for its own rounds. A failure answers `{ error }`
 * and exit code 1, which the window turns into "use the note instead".
 * @returns the exit code.
 */
async function commandCompact(): Promise<number> {
  const raw = await readStdin()
  let request: {
    history?: Array<{ role?: string; text?: string; content?: string }>
    system?: string
    model?: string
    apiKey?: string
    baseUrl?: string
    maxTokens?: number
    timeoutMs?: number
    label?: string
  }
  try {
    request = JSON.parse(raw) as typeof request
  } catch {
    process.stdout.write(`${JSON.stringify({ error: 'stdin 不是合法 JSON' })}\n`)
    return 2
  }
  const turns = (request.history ?? [])
    .map((turn) => {
      const role = turn?.role === 'assistant' ? 'assistant' : 'user'
      const text = (turn?.content ?? turn?.text ?? '').trim()
      return text === '' ? null : { role, text }
    })
    .filter((turn): turn is { role: 'user' | 'assistant'; text: string } => turn !== null)
  if (turns.length === 0) {
    process.stdout.write(`${JSON.stringify({ error: '没有可以压缩的对话内容' })}\n`)
    return 2
  }
  const messages = turns.map((turn) => ({ role: turn.role, content: turn.text }))
  const limits = compactLimits(0)
  const target = {
    baseUrl: (request.baseUrl ?? '').replace(/\/+$/, ''),
    apiKey: request.apiKey ?? '',
    model: request.model ?? '',
    timeoutMs: request.timeoutMs,
    maxTokens: request.maxTokens ?? limits.maxSummaryTokens
  }
  const summary = await summarizeRange(messages, { start: 0, end: messages.length }, target)
  if (summary === null) {
    process.stdout.write(`${JSON.stringify({ error: '摘要调用失败' })}\n`)
    return 1
  }
  process.stdout.write(`${JSON.stringify({ summary, model: target.model })}\n`)
  return 0
}

/**
 * Read all of stdin as text.
 * @returns the input, empty when the caller wrote nothing.
 */
async function readStdin(): Promise<string> {
  const chunks: Buffer[] = []
  for await (const chunk of process.stdin) chunks.push(Buffer.from(chunk as Buffer))
  return Buffer.concat(chunks).toString('utf8')
}

process.exitCode = await main(process.argv.slice(2))
