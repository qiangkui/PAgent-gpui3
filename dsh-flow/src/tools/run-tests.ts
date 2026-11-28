/**
 * `run_tests`: run the project's tests and hand the failures back as data.
 *
 * `shell` can already run a test command, but its answer is raw output: the
 * model then re-reads hundreds of lines to find two failing cases, and a
 * truncated tail can hide exactly the failure it needed. This runs the same
 * command and pulls the failures out first, for the four runners the projects
 * here actually use.
 */

import { existsSync, promises as fs } from 'node:fs'
import path from 'node:path'
import type { ToolResult, ToolScope } from './types.ts'
import { LONG_RUN_TIMEOUT_MS, MAX_SHELL_TIMEOUT_MS } from './limits.ts'
import { resolveCommandDir } from './sandbox.ts'
import type { Sandbox } from './sandbox.ts'
import { outsidePaths, sandboxRefusal } from './sandbox.ts'
import { intArg, textArg } from './paths.ts'
import { captureCommand } from './capture.ts'
import type { CaptureOutcome } from './capture.ts'
import { dotnetProject, gradleCommand, hasGradle, hasMaven } from './project.ts'

/** One failing case, as the answer lists it. */
export interface TestFailure {
  /** Test name as the runner printed it. */
  name: string
  /** One line saying why it failed; empty when the runner printed no detail. */
  message: string
}

/** What one parsed run found. */
export interface TestReport {
  /** Passed cases, when the runner reported a count. */
  passed?: number
  /** Failed cases, when the runner reported a count. */
  failedCount?: number
  /** The failures themselves, capped. */
  failures: TestFailure[]
  /** One line naming the runner and its summary. */
  summary: string
}

/** How many failures the answer lists before it stops. */
const MAX_FAILURES = 20

/** How much raw output is quoted after the list. */
const MAX_TAIL = 6_000

/** One planned run: what to start and which parser reads it. */
export interface TestPlan {
  /** Executable to start. */
  file: string
  /** Arguments after it. */
  args: string[]
  /** The command line shown to the person and the model. */
  label: string
  /** True when the command needs the platform shell (npm's `.cmd` shim). */
  shell: boolean
  /** Which parser reads its output. */
  kind: 'cargo' | 'pytest' | 'npm' | 'go' | 'dotnet' | 'maven' | 'gradle' | 'generic'
}

/**
 * Run the project's tests and summarize the failures.
 * @param workDir - relative paths resolve from here.
 * @param scopeRoot - writable root; null means the whole computer.
 * @param args - optional `command`, `filter`, `cwd` and `timeoutMs`.
 * @param sandbox - folders a command may reach, or null to skip the check.
 * @returns the exit code, the counts the runner reported, and the failures.
 */
export async function runTests(
  workDir: string,
  scopeRoot: ToolScope,
  args: Record<string, unknown>,
  sandbox: Sandbox | null = null,
  onOutput?: (text: string) => void
): Promise<ToolResult> {
  // A test run writes build output. In 「仅可查看」 that promise wins over running
  // the suite, and the refusal has to say why rather than surface as EACCES.
  if (process.env.DSH_SANDBOX_READONLY === '1') {
    throw new Error('仅可查看模式不跑测试：测试会写构建产物。切到「工作区内修改」再试。')
  }
  const cwdArg = typeof args.cwd === 'string' && args.cwd.trim() ? args.cwd.trim() : '.'
  const cwd = await resolveCommandDir(workDir, scopeRoot, sandbox, cwdArg)
  const info = await fs.stat(cwd).catch(() => null)
  if (!info?.isDirectory()) throw new Error(`找不到运行目录：${cwdArg}`)
  const plan = await testPlan(cwd, args)
  if (plan === null) {
    throw new Error(
      '没有识别到测试入口（Cargo.toml / go.mod / .sln / .csproj / pom.xml / build.gradle(.kts) / package.json 的 test 脚本 / pytest）；用 command 给一条具体命令，例如 "node --test"。'
    )
  }
  // A command line the sandbox can inspect: it names its own program and args,
  // so an executor that points outside the workspace is refused before it runs.
  const commandLine = [plan.file, ...plan.args].join(' ')
  if (sandbox !== null) {
    const outside = await outsidePaths(commandLine, cwd, sandbox)
    if (outside.length > 0) throw new Error(sandboxRefusal(outside, sandbox))
  }
  const requested = intArg(args, 'timeoutMs') ?? LONG_RUN_TIMEOUT_MS
  const timeoutMs = Math.min(Math.max(requested, 1_000), MAX_SHELL_TIMEOUT_MS)
  const outcome = await captureCommand(plan, cwd, timeoutMs, onOutput)
  const report = parseReport(plan.kind, outcome.text)
  return { ok: outcome.code === 0 && !outcome.timedOut, text: answer(plan, cwdArg, outcome, report) }
}

/**
 * Work out what to run, from an explicit command or the files in `cwd`.
 * @param cwd - resolved run directory.
 * @param args - the call's arguments.
 * @returns the plan, or null when nothing was recognised.
 */
export async function testPlan(
  cwd: string,
  args: Record<string, unknown>
): Promise<TestPlan | null> {
  const given = typeof args.command === 'string' ? args.command.trim() : ''
  if (given !== '') {
    return { file: given, args: [], label: given, shell: true, kind: 'generic' }
  }
  const filter = typeof args.filter === 'string' ? args.filter.trim() : ''
  if (existsSync(path.join(cwd, 'Cargo.toml'))) {
    const args: string[] = ['test']
    if (filter !== '') args.push(filter)
    return { file: 'cargo', args, label: ['cargo', ...args].join(' '), shell: false, kind: 'cargo' }
  }
  const scripts = await packageScripts(cwd)
  if (existsSync(path.join(cwd, 'go.mod'))) {
    // A Go module's tests are always `go test`; `-run` takes a name regex, which
    // is what `filter` means for every other runner too.
    const args: string[] = ['test']
    if (filter !== '') args.push('-run', filter)
    args.push('./...')
    return { file: 'go', args, label: ['go', ...args].join(' '), shell: false, kind: 'go' }
  }
  const solution = dotnetProject(cwd)
  if (solution !== null) {
    // `dotnet test` builds the solution first; naming it keeps a folder with two
    // solutions from picking the wrong one. `--filter` takes the same name
    // expression every other runner's filter means.
    const args: string[] = ['test', '--nologo', solution]
    if (filter !== '') args.push('--filter', `FullyQualifiedName~${filter}`)
    return { file: 'dotnet', args, label: ['dotnet', ...args].join(' '), shell: false, kind: 'dotnet' }
  }
  if (hasMaven(cwd)) {
    // Batch mode without `-q`: Surefire prints its `Tests run: …` counts at INFO
    // level, and `-q` would hide exactly the figures the answer reports. `-B` only
    // drops colour and the download progress bar, so the tail stays readable.
    const args: string[] = filter === '' ? ['-B', 'test'] : ['-B', 'test', `-Dtest=${filter}`]
    return { file: 'mvn', args, label: ['mvn', ...args].join(' '), shell: true, kind: 'maven' }
  }
  if (hasGradle(cwd)) {
    // The wrapper is preferred; `--console=plain` drops the progress bars and
    // colour codes so the failure lines stay parseable. `--tests` takes a class
    // or method pattern, which is what `filter` means everywhere else.
    const launcher = gradleCommand(cwd)
    const args: string[] = ['test', '--console=plain']
    if (filter !== '') args.push('--tests', filter)
    return { file: launcher, args, label: [launcher, ...args].join(' '), shell: true, kind: 'gradle' }
  }
  if (scripts.includes('test')) {
    // `npm test -- <filter>` is how npm forwards an argument to the script, and
    // jest / vitest honour it. A script that ignores arguments still runs its
    // whole suite, so this can only narrow, never broaden.
    const args = filter === '' ? ['test'] : ['test', '--', filter]
    return { file: 'npm', args, label: ['npm', ...args].join(' '), shell: true, kind: 'npm' }
  }
  const pytest =
    existsSync(path.join(cwd, 'pytest.ini')) ||
    existsSync(path.join(cwd, 'conftest.py')) ||
    existsSync(path.join(cwd, 'tests'))
  if (pytest) {
    const args: string[] = ['-m', 'pytest']
    if (filter !== '') args.push('-k', filter)
    return { file: 'python', args, label: ['python', ...args].join(' '), shell: false, kind: 'pytest' }
  }
  return null
}

/** The script names a package.json declares. */
async function packageScripts(cwd: string): Promise<string[]> {
  const file = path.join(cwd, 'package.json')
  if (!existsSync(file)) return []
  try {
    const parsed = JSON.parse(await fs.readFile(file, 'utf8')) as { scripts?: unknown }
    const scripts = parsed.scripts
    if (!scripts || typeof scripts !== 'object') return []
    return Object.keys(scripts as Record<string, unknown>)
  } catch {
    return []
  }
}

/** How much captured output is kept before the middle is dropped. */

/**
 * Parse one run with the parser that matches its runner.
 * @param kind - which runner produced the output.
 * @param output - captured stdout and stderr.
 * @returns the counts the runner reported and the failures it named.
 */
export function parseReport(kind: TestPlan['kind'], output: string): TestReport {
  if (kind === 'cargo') return parseCargo(output)
  if (kind === 'pytest') return parsePytest(output)
  if (kind === 'npm') return parseNpm(output)
  if (kind === 'go') return parseGo(output)
  if (kind === 'dotnet') return parseDotnet(output)
  if (kind === 'maven') return parseMaven(output)
  if (kind === 'gradle') return parseGradle(output)
  return parseGeneric(output)
}

/** Cargo's `test result:` lines and `---- name stdout ----` blocks. */
export function parseCargo(output: string): TestReport {
  const failures: TestFailure[] = []
  let passed = 0
  let failed = 0
  let counted = false
  for (const match of output.matchAll(/test result: (?:ok|FAILED)\. (\d+) passed; (\d+) failed/g)) {
    counted = true
    passed += Number(match[1])
    failed += Number(match[2])
  }
  for (const block of output.matchAll(/---- ([^\n]+) stdout ----\n([\s\S]*?)(?=\n\n|$)/g)) {
    const name = block[1].trim()
    const body = block[2]
      .split('\n')
      .map((line) => line.trim())
      .filter((line) => line !== '')
    const message = body.find((line) => /panicked|assertion|error/i.test(line)) ?? body[0] ?? ''
    addFailure(failures, name, message)
  }
  if (failures.length === 0) {
    for (const match of output.matchAll(/^test (\S+) \.\.\. FAILED$/gm)) {
      addFailure(failures, match[1], '')
    }
  }
  return {
    passed: counted ? passed : undefined,
    failedCount: counted ? failed : undefined,
    failures,
    summary: counted ? `${passed} 通过 / ${failed} 失败` : 'cargo test（未报告统计）'
  }
}

/** Pytest's `FAILED path::test - reason` lines and its last summary line. */
export function parsePytest(output: string): TestReport {
  const failures: TestFailure[] = []
  for (const match of output.matchAll(/^FAILED (\S+?)(?: - (.*))?$/gm)) {
    addFailure(failures, match[1], match[2] ?? '')
  }
  for (const match of output.matchAll(/^ERROR (\S+?)(?: - (.*))?$/gm)) {
    addFailure(failures, match[1], match[2] ?? '')
  }
  const passed = lastNumber(output, /(\d+) passed/g)
  const failed = lastNumber(output, /(\d+) failed/g)
  const errors = lastNumber(output, /(\d+) errors?/g)
  const failedCount =
    failed === undefined && errors === undefined ? undefined : (failed ?? 0) + (errors ?? 0)
  return {
    passed,
    failedCount,
    failures,
    summary:
      passed === undefined && failedCount === undefined
        ? 'pytest（未报告统计）'
        : `${passed ?? 0} 通过 / ${failedCount ?? 0} 失败`
  }
}

/** TAP, vitest, jest and node's own reporter, whichever one the script uses. */
export function parseNpm(output: string): TestReport {
  const failures: TestFailure[] = []
  for (const match of output.matchAll(/^\s*not ok \d+ - (.+)$/gm)) {
    addFailure(failures, match[1], '')
  }
  for (const match of output.matchAll(/^\s*[✖×✗] (.+)$/gm)) {
    // Node prints `✖ divides (2.4ms)`: the duration is not part of the name, and
    // `✖ failing tests:` is the heading of the list rather than a case.
    const name = match[1].replace(/\s+\(\d+(?:\.\d+)?ms\)\s*$/, '').trim()
    if (/(?:failing|cancelled|skipped|todo)\s+tests?:?$/i.test(name)) continue
    addFailure(failures, name, '')
  }
  for (const match of output.matchAll(/^\s*● (.+)$/gm)) {
    addFailure(failures, match[1], '')
  }
  // `node --test` writes `ℹ pass 12` / `ℹ fail 1`; TAP writes `# pass 12`.
  // Without the first form the built-in runner looked like it never counted.
  const tally = counters(output)
  const passed = tally.pass ?? lastNumber(output, /(\d+) passing/g) ?? lastNumber(output, /(\d+) passed/g)
  const failed =
    tally.fail ??
    lastNumber(output, /# fail (\d+)/g) ??
    lastNumber(output, /(\d+) failing/g) ??
    lastNumber(output, /(\d+) failed/g) ??
    lastNumber(output, /Tests:\s+(\d+) failed/g)
  return {
    passed,
    failedCount: failed,
    failures,
    summary:
      passed === undefined && failed === undefined
        ? 'npm test（未报告统计）'
        : `${passed ?? 0} 通过 / ${failed ?? 0} 失败`
  }
}

/**
 * The `ℹ key N` / `# key N` counters a TAP or node summary prints.
 *
 * The last value wins, because a monorepo prints one block per package and the
 * figure at the end is the total.
 * @param output - the captured output.
 * @returns the counters that were found, keyed by name.
 */
function counters(output: string): Record<string, number> {
  const found: Record<string, number> = {}
  const pattern = /^\s*(?:ℹ|#)\s*(tests|pass|fail|cancelled|skipped|todo)\s+(\d+)\s*$/gm
  for (const match of output.matchAll(pattern)) {
    found[match[1] ?? ''] = Number(match[2])
  }
  return found
}

/**
 * Go's `--- FAIL:` blocks and its per-package summary.
 *
 * Without `-v` Go prints nothing for a passing test, so the pass count is per
 * package (`ok  pkg`) while the failure count is per test — the summary says
 * both, which is more honest than inventing a passed-test figure.
 * @param output - captured stdout and stderr.
 * @returns the failures, the package counts and a one-line summary.
 */
export function parseGo(output: string): TestReport {
  const failures: TestFailure[] = []
  const lines = output.split('\n')
  for (let index = 0; index < lines.length; index += 1) {
    const head = /^--- FAIL: (\S+)/.exec((lines[index] ?? '').trim())
    if (head === null) continue
    // The detail is the indented line right after the headline:
    // `--- FAIL: TestAdd (0.00s)` then `    add_test.go:12: got 1, want 2`.
    let message = ''
    for (let next = index + 1; next < lines.length; next += 1) {
      const line = lines[next] ?? ''
      if (line.trim() === '') continue
      if (!/^\s{2,}/.test(line)) break
      message = line.trim()
      break
    }
    addFailure(failures, head[1] ?? '', message)
  }
  let passedPackages = 0
  let failedPackages = 0
  for (const line of lines) {
    if (/^ok\s+\S+/.test(line)) passedPackages += 1
    else if (/^FAIL\s+\S+/.test(line)) failedPackages += 1
  }
  const counted = passedPackages > 0 || failedPackages > 0 || failures.length > 0
  return {
    passed: counted ? passedPackages : undefined,
    failedCount: counted ? failures.length : undefined,
    failures,
    summary: counted
      ? `${passedPackages} 个包通过 / ${failures.length} 个用例失败`
      : 'go test（未报告统计）'
  }
}

/**
 * `dotnet test`'s `Failed Name [12 ms]` blocks and its `Failed: 1, Passed: 2`
 * summary line.
 *
 * The reason lives under an `Error Message:` line that may be followed by a
 * stack trace, so only the first non-empty line after it is kept.
 * @param output - captured stdout and stderr.
 * @returns the failures, the counts and a one-line summary.
 */
export function parseDotnet(output: string): TestReport {
  const failures: TestFailure[] = []
  const lines = output.split('\n')
  for (let index = 0; index < lines.length; index += 1) {
    const head = /^\s*Failed\s+(\S+)/.exec(lines[index] ?? '')
    if (head === null) continue
    let message = ''
    for (let next = index + 1; next < Math.min(lines.length, index + 8); next += 1) {
      const line = (lines[next] ?? '').trim()
      if (line === '') continue
      const inline = /^Error Message:\s*(.*)$/i.exec(line)
      if (inline !== null) {
        message = inline[1]?.trim() ?? ''
        if (message === '') message = (lines[next + 1] ?? '').trim()
        break
      }
      // The next case starts before any reason was printed.
      if (/^\s*(Failed|Passed|Skipped)!?\s+\S/.test(line)) break
    }
    addFailure(failures, head[1] ?? '', message)
  }
  let passed: number | undefined
  let failed: number | undefined
  // `Failed!  - Failed:     1, Passed:     2, Skipped:     0, Total:     3`
  for (const match of output.matchAll(/Failed:\s+(\d+),\s+Passed:\s+(\d+)/g)) {
    failed = Number(match[1])
    passed = Number(match[2])
  }
  return {
    passed,
    failedCount: failed,
    failures,
    summary:
      passed === undefined && failed === undefined
        ? 'dotnet test（未报告统计）'
        : `${passed ?? 0} 通过 / ${failed ?? 0} 失败`
  }
}

/**
 * Maven Surefire's `Tests run: 3, Failures: 1, Errors: 0` lines and its
 * `[ERROR] com.example.FooTest.bar:42 expected…` case lines.
 *
 * Surefire repeats the summary once per module, so the last one wins: it is the
 * module that ran most recently. A failure and an error are both cases that did
 * not pass, which is what the answer counts.
 * @param output - captured stdout and stderr.
 * @returns the failures, the counts and a one-line summary.
 */
export function parseMaven(output: string): TestReport {
  const failures: TestFailure[] = []
  let passed: number | undefined
  let failed: number | undefined
  for (const match of output.matchAll(
    /Tests run:\s+(\d+),\s+Failures:\s+(\d+),\s+Errors:\s+(\d+),\s+Skipped:\s+(\d+)/g
  )) {
    const total = Number(match[1])
    const fail = Number(match[2])
    const errors = Number(match[3])
    const skipped = Number(match[4])
    failed = fail + errors
    passed = Math.max(0, total - fail - errors - skipped)
  }
  for (const match of output.matchAll(/^\[ERROR\]\s+([\w.$]+:\d+)(?:\s+(.*))?$/gm)) {
    addFailure(failures, match[1] ?? '', match[2] ?? '')
  }
  return {
    passed,
    failedCount: failed,
    failures,
    summary:
      passed === undefined && failed === undefined
        ? 'mvn test（未报告统计）'
        : `${passed ?? 0} 通过 / ${failed ?? 0} 失败`
  }
}

/**
 * Gradle's `MathTest > divides() FAILED` lines and its `N tests completed` count.
 *
 * The console reporter prints the case name with its class in front, then an
 * indented line holding the exception; that line is the failure's reason.
 * @param output - captured stdout and stderr.
 * @returns the failures, the counts and a one-line summary.
 */
export function parseGradle(output: string): TestReport {
  const failures: TestFailure[] = []
  const lines = output.split('\n')
  for (let index = 0; index < lines.length; index += 1) {
    const head = /^\s*(\S.*?\s>\s.+?)\s+FAILED\s*$/.exec(lines[index] ?? '')
    if (head === null) continue
    let message = ''
    for (let next = index + 1; next < Math.min(lines.length, index + 4); next += 1) {
      const line = lines[next] ?? ''
      if (line.trim() === '') continue
      // The exception is indented under the case; anything flush left is the next
      // case or the build's own output.
      if (!/^\s{2,}/.test(line)) break
      message = line.trim()
      break
    }
    addFailure(failures, (head[1] ?? '').trim(), message)
  }
  let passed: number | undefined
  let failed: number | undefined
  // `3 tests completed, 1 failed`; a passing run prints the count without the
  // second figure, and an older Gradle may add `, 1 skipped`.
  for (const match of output.matchAll(/(\d+) tests? completed(?:,\s*(\d+) failed)?/g)) {
    const total = Number(match[1])
    const bad = match[2] === undefined ? 0 : Number(match[2])
    failed = bad
    passed = Math.max(0, total - bad)
  }
  return {
    passed,
    failedCount: failed,
    failures,
    summary:
      passed === undefined && failed === undefined
        ? 'gradle test（未报告统计）'
        : `${passed ?? 0} 通过 / ${failed ?? 0} 失败`
  }
}

/** Last resort: every line that reads like a failure, without counts. */
export function parseGeneric(output: string): TestReport {
  const failures: TestFailure[] = []
  for (const line of output.split('\n')) {
    const trimmed = line.trim()
    if (trimmed === '' || trimmed.startsWith('#')) continue
    if (!/(fail|error|✖|✗|not ok)/i.test(trimmed)) continue
    // Counts are a summary, not a case: `1 failed, 2 passed` would otherwise be
    // listed as a failing test of its own.
    if (/^\d+ (passed|failed)/i.test(trimmed)) continue
    const name = trimmed
      .replace(/^not ok \d+ - /i, '')
      .replace(/^[✖×✗]\s*/, '')
      .replace(/^(FAIL|ERROR)\s+/, '')
    addFailure(failures, clip(name, 160), '')
  }
  return {
    failures,
    summary: failures.length === 0 ? '没有失败行' : `看起来有 ${failures.length} 条失败行`
  }
}

/** Record one failure, skipping duplicates and keeping the text short. */
function addFailure(failures: TestFailure[], name: string, message: string): void {
  const clean = name.trim()
  if (clean === '') return
  if (failures.some((failure) => failure.name === clean)) return
  failures.push({ name: clip(clean, 160), message: oneLine(clip(message, 300)) })
}

/** The last number a pattern matched, so a summary line wins over an early one. */
function lastNumber(text: string, pattern: RegExp): number | undefined {
  let found: number | undefined
  for (const match of text.matchAll(pattern)) found = Number(match[1])
  return found
}

/** One line out of a multi-line detail block. */
function oneLine(text: string): string {
  return (
    text
      .split('\n')
      .map((line) => line.trim())
      .find((line) => line !== '') ?? ''
  )
}

/** Keep a string inside a budget, with an ellipsis when it was cut. */
function clip(text: string, limit: number): string {
  return text.length <= limit ? text : `${text.slice(0, limit)}…`
}

/** The answer: what ran, the counts, the failures, then the output's tail. */
function answer(plan: TestPlan, cwdArg: string, outcome: CaptureOutcome, report: TestReport): string {
  const lines = [`$ ${plan.label}`, `退出码 ${outcome.code}${outcome.timedOut ? '（超时被结束）' : ''}`]
  if (cwdArg !== '.') lines.push(`（工作目录：${cwdArg}）`)
  lines.push(`统计：${report.summary}`)
  if (report.failures.length > 0) {
    const shown = report.failures.slice(0, MAX_FAILURES)
    lines.push(`失败（${report.failures.length} 条，列出 ${shown.length} 条）：`)
    for (const failure of shown) {
      lines.push(
        failure.message === '' ? `- ${failure.name}` : `- ${failure.name} — ${failure.message}`
      )
    }
    if (report.failures.length > shown.length) {
      lines.push(`…（还有 ${report.failures.length - shown.length} 条未列出）`)
    }
  } else if (outcome.code !== 0) {
    lines.push('没有从输出里解析出失败用例；下面是输出末尾。')
  }
  const tail = outcome.text.trim()
  if (tail !== '') {
    lines.push('输出末尾：')
    lines.push(
      tail.length > MAX_TAIL ? `…（只显示最后 ${MAX_TAIL} 字符）\n${tail.slice(-MAX_TAIL)}` : tail
    )
  }
  return lines.join('\n')
}