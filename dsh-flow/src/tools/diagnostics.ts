/**
 * `diagnostics`: the compiler's complaints, as data instead of prose.
 *
 * `run_tests` covers "does it work"; this covers "does it compile". A `cargo
 * check` or `tsc --noEmit` answer is hundreds of lines where the model needs the
 * five errors, and a truncated tail can hide exactly them. Both tools speak a
 * machine-readable form — cargo's JSON messages and tsc's `file(line,col)`
 * lines — so the failures come out first and the raw output stays as context.
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
import { dotnetProject, eslintCommand, gradleCommand, hasEslint, hasGradle, hasMaven } from './project.ts'

/** One problem the compiler reported. */
export interface Diagnostic {
  /** File as the compiler named it; relative to the run directory. */
  file: string
  /** 1-based line; zero when the message had no span. */
  line: number
  /** 1-based column; zero when the message had no span. */
  column: number
  /** `error` or `warning`. */
  level: 'error' | 'warning'
  /** Compiler code, for example `E0308`; empty when there is none. */
  code: string
  /** One line of message text. */
  message: string
}

/** Everything one run reported, split so the answer can list errors first. */
export interface DiagnosticReport {
  errors: Diagnostic[]
  warnings: Diagnostic[]
}

/** How many problems are listed by default. */
const DEFAULT_LIMIT = 30

/** Longest list one call may ask for. */
const MAX_LIMIT = 100

/** How much raw output is quoted after the list. */
const MAX_TAIL = 6_000

/** One planned run: what to start and how to read it. */
interface DiagnosticPlan {
  /** Executable to start. */
  file: string
  /** Arguments after it. */
  args: string[]
  /** The command line shown to the person and the model. */
  label: string
  /** True when the command needs the platform shell. */
  shell: boolean
  /** Which parser reads its output. */
  kind: 'cargo' | 'text'
}

/**
 * Run the project's compiler and summarize what it reported.
 * @param workDir - relative paths resolve from here.
 * @param scopeRoot - writable root; null means the whole computer.
 * @param args - optional `command`, `cwd`, `timeoutMs`, `severity` and `limit`.
 * @param sandbox - folders a command may reach, or null to skip the check.
 * @returns the exit code, the counts, and the problems.
 */
export async function runDiagnostics(
  workDir: string,
  scopeRoot: ToolScope,
  args: Record<string, unknown>,
  sandbox: Sandbox | null = null,
  onOutput?: (text: string) => void
): Promise<ToolResult> {
  // Same reason as `run_tests`: a compiler writes its artifacts, which a
  // read-only conversation has promised not to do.
  if (process.env.DSH_SANDBOX_READONLY === '1') {
    throw new Error('仅可查看模式不跑编译检查：它会写构建产物。切到「工作区内修改」再试。')
  }
  const cwdArg = typeof args.cwd === 'string' && args.cwd.trim() ? args.cwd.trim() : '.'
  const cwd = await resolveCommandDir(workDir, scopeRoot, sandbox, cwdArg)
  const info = await fs.stat(cwd).catch(() => null)
  if (!info?.isDirectory()) throw new Error(`找不到运行目录：${cwdArg}`)
  const plan = diagnosticPlan(cwd, args)
  if (plan === null) {
    throw new Error(
      '没有识别到编译入口（Cargo.toml / go.mod / tsconfig.json / .sln / .csproj / pom.xml / build.gradle(.kts) / eslint 配置）；用 command 给一条具体命令，例如 "cargo check --message-format=json" 或 "tsc --noEmit --pretty false"。'
    )
  }
  const commandLine = [plan.file, ...plan.args].join(' ')
  if (sandbox !== null) {
    const outside = await outsidePaths(commandLine, cwd, sandbox)
    if (outside.length > 0) throw new Error(sandboxRefusal(outside, sandbox))
  }
  const asked = intArg(args, 'timeoutMs') ?? LONG_RUN_TIMEOUT_MS
  const timeoutMs = Math.min(Math.max(asked, 1_000), MAX_SHELL_TIMEOUT_MS)
  const outcome = await captureCommand(plan, cwd, timeoutMs, onOutput)
  // Cargo's report is stdout JSON; progress on stderr must not be mixed into it.
  // A text compiler writes wherever it likes, so both streams are read there.
  const report =
    plan.kind === 'cargo'
      ? parseCargoDiagnostics(outcome.stdout)
      : parseTextDiagnostics(outcome.text)
  const severity = typeof args.severity === 'string' ? args.severity.trim().toLowerCase() : 'all'
  const limit = Math.min(Math.max(intArg(args, 'limit') ?? DEFAULT_LIMIT, 1), MAX_LIMIT)
  return {
    ok: outcome.code === 0 && !outcome.timedOut,
    text: answer(plan, cwdArg, outcome, report, severity, limit)
  }
}

/**
 * Work out what to compile, from an explicit command or the files in `cwd`.
 * @param cwd - resolved run directory.
 * @param args - the call's arguments.
 * @returns the plan, or null when nothing was recognised.
 */
export function diagnosticPlan(cwd: string, args: Record<string, unknown>): DiagnosticPlan | null {
  const given = typeof args.command === 'string' ? args.command.trim() : ''
  const asked = typeof args.format === 'string' ? args.format.trim().toLowerCase() : 'auto'
  if (given !== '') {
    // A command that asks cargo for JSON is parsed as JSON: `cargo clippy
    // --message-format=json` is the common case, and the text parser would find
    // nothing in it.
    const kind =
      asked === 'cargo'
        ? 'cargo'
        : asked === 'text'
          ? 'text'
          : /--message-format(?:\s+|=)json\b/i.test(given)
            ? 'cargo'
            : 'text'
    return { file: given, args: [], label: given, shell: true, kind }
  }
  if (existsSync(path.join(cwd, 'Cargo.toml'))) {
    return {
      file: 'cargo',
      args: ['check', '--message-format=json'],
      label: 'cargo check --message-format=json',
      shell: false,
      kind: 'cargo'
    }
  }
  if (existsSync(path.join(cwd, 'go.mod'))) {
    // `go vet` prints `file:line:col: message`, which is the text form below.
    return {
      file: 'go',
      args: ['vet', './...'],
      label: 'go vet ./...',
      shell: false,
      kind: 'text'
    }
  }
  if (existsSync(path.join(cwd, 'tsconfig.json'))) {
    // A project-local tsc is the version the project itself builds with; the one
    // on PATH may be older. Both shims need the shell on Windows.
    const local = process.platform === 'win32'
      ? path.join(cwd, 'node_modules', '.bin', 'tsc.cmd')
      : path.join(cwd, 'node_modules', '.bin', 'tsc')
    const file = existsSync(local) ? local : 'tsc'
    return {
      file,
      args: ['--noEmit', '--pretty', 'false'],
      label: `${file} --noEmit --pretty false`,
      shell: true,
      kind: 'text'
    }
  }
  const solution = dotnetProject(cwd)
  if (solution !== null) {
    // `dotnet build` prints `Program.cs(12,5): error CS0103: …`, which the text
    // parser already reads because tsc uses the same shape.
    return {
      file: 'dotnet',
      args: ['build', '--nologo', '-v', 'minimal', solution],
      label: `dotnet build --nologo ${solution}`,
      shell: false,
      kind: 'text'
    }
  }
  if (hasMaven(cwd)) {
    // `-q` keeps the dependency log off the tail; javac's own lines carry the
    // file, line and column.
    return {
      file: 'mvn',
      args: ['-q', '-DskipTests', 'compile'],
      label: 'mvn -q -DskipTests compile',
      shell: true,
      kind: 'text'
    }
  }
  if (hasGradle(cwd)) {
    // `classes` compiles every source set the project has (Java, Kotlin), which is
    // what "does it compile" means here; `--console=plain` keeps javac's lines.
    const launcher = gradleCommand(cwd)
    return {
      file: launcher,
      args: ['classes', '--console=plain'],
      label: `${launcher} classes --console=plain`,
      shell: true,
      kind: 'text'
    }
  }
  if (hasEslint(cwd)) {
    // The compact format is one line per problem, which is the shape every other
    // linter's output takes; the JSON formatter would need its own parser.
    const local = eslintCommand(cwd)
    const viaNpx = local === 'npx'
    const file = viaNpx ? 'npx' : local
    // `--no-install` turns "eslint is not installed" into an immediate error
    // instead of an interactive "Ok to proceed?" prompt that would just hang
    // until the timeout.
    const args = viaNpx ? ['--no-install', 'eslint', '.', '-f', 'compact'] : ['.', '-f', 'compact']
    return { file, args, label: [file, ...args].join(' '), shell: true, kind: 'text' }
  }
  return null
}

/** `file(line,col): error TS2322: message` — TypeScript's compact form, and
 * `file(line,col): error CS0103: message` from `dotnet build`. Both name the
 * rule as letters + digits, so one pattern reads tsc, Roslyn and msbuild. */
const TSC_LINE = /^(.+?)\((\d+),(\d+)\):\s+(error|warning)\s+([A-Z]{1,4}\d+)?\s*:?\s*(.*)$/

/** `file.js: line 12, col 5, Error - message (rule)` — eslint's compact form. */
const ESLINT_LINE = /^(.+?):\s*line (\d+),\s*col (\d+),\s*(Error|Warning) - (.*?)(?:\s+\(([^)]+)\))?$/

/** `[ERROR] /src/Foo.java:[12,5] message` — Maven's javac summary. */
const MAVEN_LINE = /^\[(ERROR|WARNING)\]\s+(.+?):\[(\d+),(\d+)\]\s*(.*)$/i

/** `src/Foo.java:12: error: message` — javac, and gcc without a column. */
const JAVA_LINE = /^(.+?):(\d+):\s+(error|warning):\s*(.*)$/i

/** `e: file:///E:/src/Foo.kt:3:1 Unresolved reference: x` — kotlinc, and Gradle. */
const KOTLIN_LINE = /^([ew]):\s+(.+?):(\d+):(\d+)\s+(.*)$/

/**
 * `file:line:col: error|warning: message` — eslint, clang, most linters.
 *
 * `go vet` and `go build` print `file:line:col: message` with no level at all,
 * so the level and its colon are optional; a line that names a position is
 * treated as an error unless it says otherwise.
 */
const COLON_LINE = /^(.+?):(\d+):(\d+):\s+(?:(error|warning)\b\s*:?\s*)?(.*)$/i

/** `error[E0308]: message` — rustc's headline, with a span on the next line. */
const HEAD_LINE = /^(error|warning)(?:\[([A-Za-z0-9]+)\])?:\s*(.*)$/

/** `  --> src/main.rs:12:5` — the span that belongs to a headline. */
const ARROW_LINE = /^-->\s+(\S+?):(\d+):(\d+)$/

/**
 * Parse cargo's `--message-format=json` stream.
 * @param text - the captured output, one JSON object per line.
 * @returns errors and warnings with their primary span.
 */
export function parseCargoDiagnostics(text: string): DiagnosticReport {
  const errors: Diagnostic[] = []
  const warnings: Diagnostic[] = []
  for (const raw of text.split('\n')) {
    const line = raw.trim()
    if (!line.startsWith('{')) continue
    let value: unknown
    try {
      value = JSON.parse(line)
    } catch {
      // A line cut by the output cap is not a message worth failing over.
      continue
    }
    const entry = value as { reason?: unknown; message?: Record<string, unknown> }
    if (entry.reason !== 'compiler-message') continue
    const message = entry.message
    if (!message || typeof message !== 'object') continue
    const level = typeof message.level === 'string' ? message.level : ''
    if (level !== 'error' && level !== 'warning') continue
    const span = primarySpan(message.spans)
    const codeValue = message.code as { code?: unknown } | undefined
    addDiagnostic(level === 'error' ? errors : warnings, {
      file: span?.file ?? '',
      line: span?.line ?? 0,
      column: span?.column ?? 0,
      level: level === 'warning' ? 'warning' : 'error',
      code: typeof codeValue?.code === 'string' ? codeValue.code : '',
      message: oneLine(typeof message.message === 'string' ? message.message : '')
    })
  }
  return { errors, warnings }
}

/** The span a compiler message points at, preferring the primary one. */
function primarySpan(raw: unknown): { file: string; line: number; column: number } | null {
  if (!Array.isArray(raw)) return null
  const spans = raw.filter(
    (span): span is Record<string, unknown> => typeof span === 'object' && span !== null
  )
  const chosen = spans.find((span) => span.is_primary === true) ?? spans[0]
  if (chosen === undefined) return null
  return {
    file: typeof chosen.file_name === 'string' ? chosen.file_name : '',
    line: Number(chosen.line_start ?? 0) || 0,
    column: Number(chosen.column_start ?? 0) || 0
  }
}

/**
 * Parse a compiler that prints text: tsc, rustc, and most linters.
 *
 * rustc prints `error[E0308]: …` and then the span on the next line, so a
 * headline is held until its `-->` arrives; anything without a span is dropped
 * rather than listed with a made-up position.
 * @param text - the captured output.
 * @returns errors and warnings with whatever position each named.
 */
export function parseTextDiagnostics(text: string): DiagnosticReport {
  const errors: Diagnostic[] = []
  const warnings: Diagnostic[] = []
  let pending: { level: 'error' | 'warning'; code: string; message: string } | null = null
  for (const raw of text.split('\n')) {
    const line = raw.trim()
    if (line === '') continue
    const kotlin = KOTLIN_LINE.exec(line)
    if (kotlin) {
      // kotlinc marks the level with one letter and often names the file as a
      // `file://` URL; the path is what the model can open.
      const level = kotlin[1] === 'w' ? 'warning' : 'error'
      const file = (kotlin[2] ?? '').replace(/^file:\/\/\/?/, '')
      addDiagnostic(level === 'error' ? errors : warnings, {
        file,
        line: Number(kotlin[3]),
        column: Number(kotlin[4]),
        level,
        code: '',
        message: oneLine(kotlin[5] ?? '')
      })
      pending = null
      continue
    }
    const eslint = ESLINT_LINE.exec(line)
    if (eslint) {
      const level = eslint[4] === 'Warning' ? 'warning' : 'error'
      addDiagnostic(level === 'error' ? errors : warnings, {
        file: eslint[1] ?? '',
        line: Number(eslint[2]),
        column: Number(eslint[3]),
        level,
        code: eslint[6] ?? '',
        message: oneLine(eslint[5] ?? '')
      })
      pending = null
      continue
    }
    const tsc = TSC_LINE.exec(line)
    if (tsc) {
      const level = tsc[4] === 'warning' ? 'warning' : 'error'
      // msbuild appends the project it came from as ` [C:\…\App.csproj]`; that is
      // not part of the message, and it is the one path the model cannot open
      // relative to the run directory.
      const message = oneLine(tsc[6] ?? '').replace(
        /\s*\[[^\]]*\.(?:cs|vb|vcx|fs)proj\]\s*$/i,
        ''
      )
      addDiagnostic(level === 'error' ? errors : warnings, {
        file: tsc[1] ?? '',
        line: Number(tsc[2]),
        column: Number(tsc[3]),
        level,
        code: tsc[5] ?? '',
        message
      })
      pending = null
      continue
    }
    const maven = MAVEN_LINE.exec(line)
    if (maven) {
      const level = maven[1]?.toUpperCase() === 'WARNING' ? 'warning' : 'error'
      addDiagnostic(level === 'error' ? errors : warnings, {
        file: maven[2] ?? '',
        line: Number(maven[3]),
        column: Number(maven[4]),
        level,
        code: '',
        message: oneLine(maven[5] ?? '')
      })
      pending = null
      continue
    }
    const java = JAVA_LINE.exec(line)
    if (java) {
      const level = java[3]?.toLowerCase() === 'warning' ? 'warning' : 'error'
      addDiagnostic(level === 'error' ? errors : warnings, {
        file: java[1] ?? '',
        line: Number(java[2]),
        column: 0,
        level,
        code: '',
        message: oneLine(java[4] ?? '')
      })
      pending = null
      continue
    }
    const colon = COLON_LINE.exec(line)
    if (colon) {
      const level = colon[4]?.toLowerCase() === 'warning' ? 'warning' : 'error'
      const rest = colon[5] ?? ''
      const code = /^([A-Za-z]+\d+)\s*:?\s*(.*)$/.exec(rest)
      addDiagnostic(level === 'error' ? errors : warnings, {
        file: colon[1] ?? '',
        line: Number(colon[2]),
        column: Number(colon[3]),
        level,
        code: code?.[1] ?? '',
        message: oneLine(code?.[2] ?? rest)
      })
      pending = null
      continue
    }
    const arrow = ARROW_LINE.exec(line)
    if (arrow && pending !== null) {
      addDiagnostic(pending.level === 'error' ? errors : warnings, {
        file: arrow[1] ?? '',
        line: Number(arrow[2]),
        column: Number(arrow[3]),
        level: pending.level,
        code: pending.code,
        message: pending.message
      })
      pending = null
      continue
    }
    const head = HEAD_LINE.exec(line)
    if (head) {
      pending = {
        level: head[1] === 'warning' ? 'warning' : 'error',
        code: head[2] ?? '',
        message: oneLine(head[3] ?? '')
      }
    }
  }
  return { errors, warnings }
}

/** Record one problem, skipping duplicates and keeping the text short. */
function addDiagnostic(list: Diagnostic[], diagnostic: Diagnostic): void {
  if (diagnostic.message === '' && diagnostic.file === '') return
  const key = `${diagnostic.file}:${diagnostic.line}:${diagnostic.column}:${diagnostic.code}`
  if (list.some((seen) => `${seen.file}:${seen.line}:${seen.column}:${seen.code}` === key)) return
  list.push({ ...diagnostic, message: clip(diagnostic.message, 200) })
}

/** One line out of a multi-line message. */
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

/** `file:line:col CODE — message`, with the empty pieces left out. */
function formatDiagnostic(diagnostic: Diagnostic): string {
  const where =
    diagnostic.file === ''
      ? ''
      : `${diagnostic.file}${diagnostic.line === 0 ? '' : `:${diagnostic.line}`}${
          diagnostic.column === 0 ? '' : `:${diagnostic.column}`
        }`
  const code = diagnostic.code === '' ? '' : ` ${diagnostic.code}`
  const head = `${where}${code}`.trim()
  if (head === '') return diagnostic.message
  return diagnostic.message === '' ? head : `${head} — ${diagnostic.message}`
}

/** The answer: what ran, the counts, the problems, then the output's tail. */
function answer(
  plan: DiagnosticPlan,
  cwdArg: string,
  outcome: CaptureOutcome,
  report: DiagnosticReport,
  severity: string,
  limit: number
): string {
  const errors = severity !== 'warning' ? report.errors : []
  const warnings = severity !== 'error' ? report.warnings : []
  const lines = [`$ ${plan.label}`, `退出码 ${outcome.code}${outcome.timedOut ? '（超时被结束）' : ''}`]
  if (cwdArg !== '.') lines.push(`（工作目录：${cwdArg}）`)
  lines.push(`统计：${report.errors.length} 错误 / ${report.warnings.length} 警告`)
  const section = (title: string, items: Diagnostic[]): void => {
    if (items.length === 0) return
    const shown = items.slice(0, limit)
    lines.push(`${title}（${items.length} 条，列出 ${shown.length} 条）：`)
    for (const item of shown) lines.push(`- ${formatDiagnostic(item)}`)
    if (items.length > shown.length) {
      lines.push(`…（还有 ${items.length - shown.length} 条未列出）`)
    }
  }
  section('错误', errors)
  section('警告', warnings)
  if (errors.length === 0 && warnings.length === 0) {
    if (severity === 'error' && report.errors.length > 0) {
      lines.push('按 severity=error 过滤后没有错误。')
    } else if (severity === 'warning' && report.warnings.length > 0) {
      lines.push('按 severity=warning 过滤后没有警告。')
    } else {
      lines.push('没有错误或警告。')
    }
  }
  if (outcome.code !== 0 && report.errors.length === 0 && report.warnings.length === 0) {
    lines.push('没有解析出诊断；下面是输出末尾。')
  }
  const tail = tailFor(plan, outcome)
  if (tail !== '') {
    lines.push('输出末尾：')
    lines.push(
      tail.length > MAX_TAIL ? `…（只显示最后 ${MAX_TAIL} 字符）\n${tail.slice(-MAX_TAIL)}` : tail
    )
  }
  return lines.join('\n')
}

/**
 * The output worth quoting.
 *
 * Cargo's JSON is what the list was built from, so repeating it would bury the
 * answer in raw objects; its own notes ("could not compile") are on stderr, so
 * that is what a cargo run quotes. A text compiler writes one stream, and that
 * whole stream is the context.
 * @param plan - which command ran.
 * @param outcome - the captured streams.
 * @returns the tail to show, empty when there is nothing worth it.
 */
function tailFor(plan: DiagnosticPlan, outcome: CaptureOutcome): string {
  if (plan.kind === 'cargo') {
    return outcome.stderr
      .split('\n')
      .filter((line) => !line.trim().startsWith('{"reason":'))
      .join('\n')
      .trim()
  }
  return outcome.text.trim()
}