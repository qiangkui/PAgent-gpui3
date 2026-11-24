/** Execution tools: shell commands and Python / Node snippets. */

import { spawn } from 'node:child_process'
import { promises as fs } from 'node:fs'
import { describeRuntime, resolveProjectRuntime } from '../runtime.ts'
import type { ToolResult } from './types.ts'
import {
  MAX_CODE_CHARS, MAX_SHELL_CHARS, MAX_SHELL_COMMAND, MAX_SHELL_TIMEOUT_MS, MAX_STDIN_CHARS,
  SHELL_TIMEOUT_MS
} from './limits.ts'
import {
  decodeOutput, decodeText, keepBytes, safeEnd, safeStart, stripAnsi, trimUtf8Tail
} from './text.ts'
import { intArg, textArg } from './paths.ts'
import { commandEnv, killTree } from './process.ts'
import type { Sandbox } from './sandbox.ts'
import { outsidePaths, resolveCommandDir, sandboxRefusal } from './sandbox.ts'
import { startBackground } from './tasks.ts'

/** `shell`: run one command in the conversation folder, with limits. */
export async function runShell(
  workDir: string,
  scopeRoot: string | null,
  args: Record<string, unknown>,
  onOutput?: (text: string) => void,
  sandbox: Sandbox | null = null
): Promise<ToolResult> {
  const command = textArg(args, 'command')
  if (command.trim() === '') throw new Error('command 不能为空')
  if (command.length > MAX_SHELL_COMMAND) {
    throw new Error(`命令太长（${command.length} 字符，上限 ${MAX_SHELL_COMMAND}）`)
  }
  if (args.cwd !== undefined && typeof args.cwd !== 'string') {
    throw new Error('参数 cwd 不是字符串')
  }
  const cwdArg = typeof args.cwd === 'string' && args.cwd.trim() ? args.cwd : '.'
  const cwd = await resolveCommandDir(workDir, scopeRoot, sandbox, cwdArg)
  const info = await fs.stat(cwd).catch(() => null)
  if (!info) throw new Error(`找不到运行目录：${cwdArg}`)
  if (!info.isDirectory()) throw new Error(`运行目录不是文件夹：${cwdArg}`)
  await refuseOutside(command, cwd, sandbox)
  const stdin = inputOf(args)
  if (args.background === true) {
    // A background task has no stdin: nothing is left to feed it once this
    // answer ends, and a half-open pipe would only make it wait forever.
    if (stdin !== '') throw new Error('后台任务不支持 stdin；需要输入的命令请去掉 background')
    const name = typeof args.name === 'string' ? args.name.trim().slice(0, 60) : ''
    return await startBackground(workDir, command, cwd, cwdArg, name)
  }
  const requested = intArg(args, 'timeoutMs') ?? SHELL_TIMEOUT_MS
  const timeoutMs = Math.min(Math.max(requested, 100), MAX_SHELL_TIMEOUT_MS)
  return await spawnProcess(
    { file: command, args: [], shell: true, label: command },
    cwd,
    cwdArg,
    timeoutMs,
    onOutput,
    stdin
  )
}

/** `python` / `node`: run one code snippet without a shell in between. */
export async function runCode(
  workDir: string,
  scopeRoot: string | null,
  args: Record<string, unknown>,
  language: 'python' | 'node',
  onOutput?: (text: string) => void,
  sandbox: Sandbox | null = null
): Promise<ToolResult> {
  const code = textArg(args, 'code')
  if (code.trim() === '') throw new Error('code 不能为空')
  if (code.length > MAX_CODE_CHARS) {
    throw new Error(`代码太长（${code.length} 字符，上限 ${MAX_CODE_CHARS}）`)
  }
  if (args.cwd !== undefined && typeof args.cwd !== 'string') {
    throw new Error('参数 cwd 不是字符串')
  }
  const cwdArg = typeof args.cwd === 'string' && args.cwd.trim() ? args.cwd : '.'
  const cwd = await resolveCommandDir(workDir, scopeRoot, sandbox, cwdArg)
  const info = await fs.stat(cwd).catch(() => null)
  if (!info) throw new Error(`找不到运行目录：${cwdArg}`)
  if (!info.isDirectory()) throw new Error(`运行目录不是文件夹：${cwdArg}`)
  await refuseOutside(code, cwd, sandbox)
  const stdin = inputOf(args)
  // A project interpreter beats PATH, and its bin folder joins PATH so the
  // snippet can call tools the project installed.
  const project = resolveProjectRuntime(language, cwd)
  const resolved = project.resolved
  if (!resolved) {
    throw new Error(
      `${language} 不在 PATH 上（${describeRuntime(language)}），运行目录也没有可用的 .venv`
    )
  }
  const flag = language === 'python' ? '-c' : '-e'
  const requested = intArg(args, 'timeoutMs') ?? SHELL_TIMEOUT_MS
  const timeoutMs = Math.min(Math.max(requested, 100), MAX_SHELL_TIMEOUT_MS)
  // The snippet is one argv entry: no shell quoting, and no temp file to clean.
  return await spawnProcess(
    {
      file: resolved.file,
      args: [...resolved.prefixArgs, flag, code],
      shell: false,
      label: project.local ? `${language} ${flag}（项目 .venv）` : `${language} ${flag}`,
      pathPrepend: project.pathPrepend
    },
    cwd,
    cwdArg,
    timeoutMs,
    onOutput,
    stdin
  )
}

/** The text to feed the command's stdin, empty when it reads nothing. */
function inputOf(args: Record<string, unknown>): string {
  const value = args.stdin
  if (value === undefined || value === null) return ''
  if (typeof value !== 'string') throw new Error('参数 stdin 不是字符串')
  if (value.length > MAX_STDIN_CHARS) {
    throw new Error(`stdin 太长（${value.length} 字符，上限 ${MAX_STDIN_CHARS}）`)
  }
  return value
}

/** Refuse a command that names a path outside the sandbox. */
async function refuseOutside(
  source: string,
  cwd: string,
  sandbox: Sandbox | null
): Promise<void> {
  if (sandbox === null) return
  const outside = await outsidePaths(source, cwd, sandbox)
  if (outside.length > 0) throw new Error(sandboxRefusal(outside, sandbox))
}

/** How much of each stream is kept before the tail is lost. */
export const MAX_SHELL_BUFFER = 400_000

/** How often live output is forwarded, so a chatty command cannot flood the UI. */
const LIVE_OUTPUT_INTERVAL_MS = 200

/** Newest bytes kept for the live row; enough for the last few lines. */
const MAX_LIVE_BYTES = 8 * 1024

/** What to start: a shell line, or a resolved runtime with its own arguments. */
interface SpawnSpec {
  /** Executable (or the whole line when `shell` is true). */
  file: string
  /** Arguments passed straight to the executable. */
  args: string[]
  /** Run through the platform shell, which is how `shell` works. */
  shell: boolean
  /** Text shown after `$` in the result. */
  label: string
  /** Folders placed in front of this command's PATH. */
  pathPrepend?: string[]
}

/**
 * Start one process and wait for it, killing the whole tree on timeout.
 * @param spec - executable, arguments and how to start them.
 * @param cwd - resolved working directory.
 * @param cwdArg - the directory as the model wrote it, for the answer.
 * @param timeoutMs - how long it may run.
 * @param onOutput - receives throttled stdout / stderr while it runs.
 * @param input - text for stdin; it is closed straight after, so a program
 * reading to EOF finishes instead of waiting for a person who is not there.
 * @returns the exit code plus bounded output.
 */
function spawnProcess(
  spec: SpawnSpec,
  cwd: string,
  cwdArg: string,
  timeoutMs: number,
  onOutput?: (text: string) => void,
  input = ''
): Promise<ToolResult> {
  return new Promise((resolve) => {
    const child = spawn(spec.file, spec.args, {
      cwd,
      shell: spec.shell,
      windowsHide: true,
      // Staying in the engine's group lets the window's stop kill it too.
      stdio: [input === '' ? 'ignore' : 'pipe', 'pipe', 'pipe'],
      env: commandEnv(spec.pathPrepend ?? [])
    })
    if (input !== '') child.stdin?.end(input)
    let failure = ''
    let timedOut = false
    let settled = false
    let live = Buffer.alloc(0)
    let liveAt = Date.now()
    let liveEncoding: 'utf-8' | 'gb18030' | null = null
    const outChunks: Buffer[] = []
    const errChunks: Buffer[] = []
    let outSize = 0
    let errSize = 0

    /** Decode the newest line; the row only ever shows that one. */
    const decodeLive = (bytes: Buffer): string => {
      if (
        bytes.length >= 2 &&
        ((bytes[0] === 0xff && bytes[1] === 0xfe) ||
          (bytes[0] === 0xfe && bytes[1] === 0xff))
      ) {
        return decodeText(bytes)
      }
      const newline = bytes.lastIndexOf(0x0a)
      const line = newline >= 0 && newline + 1 < bytes.length ? bytes.subarray(newline + 1) : bytes
      // Learn the stream's encoding from the first non-ASCII line; pure ASCII
      // is valid in both, so it must not decide.
      if (!line.some((byte) => byte >= 0x80)) return line.toString('utf8')
      if (liveEncoding === null) {
        try {
          new TextDecoder('utf-8', { fatal: true }).decode(trimUtf8Tail(line))
          liveEncoding = 'utf-8'
        } catch (error) {
          liveEncoding = 'gb18030'
          if (process.env.DSH_LIVE_DEBUG) console.error('strict failed', String(error), line.toString('hex'), trimUtf8Tail(line).toString('hex'))
        }
        if (process.env.DSH_LIVE_DEBUG) console.error('learn', liveEncoding, line.toString('hex'))
      }
      let start = 0
      if (liveEncoding === 'utf-8') {
        while (start < line.length && (line[start] & 0xc0) === 0x80) start += 1
      }
      return new TextDecoder(liveEncoding).decode(line.subarray(start))
    }
    /**
     * Bytes at the end that start a character a later chunk completes.
     *
     * A live row is a preview, so holding a few bytes back is invisible; decoding
     * them now would paint a replacement character that never belonged to the
     * stream.
     */
    const pendingTail = (bytes: Buffer): number => {
      let index = bytes.length - 1
      let continuations = 0
      while (index >= 0 && (bytes[index] & 0xc0) === 0x80 && continuations < 3) {
        index -= 1
        continuations += 1
      }
      if (index < 0) return 0
      const lead = bytes[index]
      const need = lead >= 0xf0 ? 4 : lead >= 0xe0 ? 3 : lead >= 0xc0 ? 2 : 1
      const have = bytes.length - index
      return have < need ? have : 0
    }
    /** Forward what arrived since the last tick, at most a few times a second. */
    const flush = (force = false): void => {
      if (!onOutput || live.length === 0) return
      if (!force && Date.now() - liveAt < LIVE_OUTPUT_INTERVAL_MS) return
      const all = live
      // Keep an unfinished character for the next chunk; the final flush takes
      // everything, since nothing else is coming.
      const keep = force ? 0 : pendingTail(all)
      const bytes = keep === 0 ? all : all.subarray(0, all.length - keep)
      live = keep === 0 ? Buffer.alloc(0) : Buffer.from(all.subarray(all.length - keep))
      liveAt = Date.now()
      if (bytes.length === 0) return
      const text = stripAnsi(decodeLive(bytes))
      if (text.trim() === '') return
      try {
        onOutput(text)
      } catch {
        // The window may have gone away; the command still has to finish.
      }
    }
    /** Keep the newest bytes if one burst is huge. */
    const keepLive = (chunk: Buffer): void => {
      const kept =
        chunk.length <= MAX_LIVE_BYTES ? chunk : chunk.subarray(chunk.length - MAX_LIVE_BYTES)
      const next = Buffer.concat([live, kept])
      live = next.length <= MAX_LIVE_BYTES ? next : next.subarray(next.length - MAX_LIVE_BYTES)
      flush()
    }
    const timer = setTimeout(() => {
      timedOut = true
      killTree(child)
    }, timeoutMs)
    child.stdout?.on('data', (chunk: Buffer | string) => {
      const bytes = typeof chunk === 'string' ? Buffer.from(chunk) : chunk
      outSize = keepBytes(outChunks, outSize, bytes, MAX_SHELL_BUFFER)
      keepLive(bytes)
    })
    child.stderr?.on('data', (chunk: Buffer | string) => {
      const bytes = typeof chunk === 'string' ? Buffer.from(chunk) : chunk
      errSize = keepBytes(errChunks, errSize, bytes, MAX_SHELL_BUFFER)
      keepLive(bytes)
    })
    const finish = (code: number | null, signal: NodeJS.Signals | null): void => {
      if (settled) return
      settled = true
      clearTimeout(timer)
      flush(true)
      const stdout = decodeOutput(Buffer.concat(outChunks))
      const stderr = decodeOutput(Buffer.concat(errChunks))
      resolve(shellResult(spec.label, cwdArg, timeoutMs, code, signal, timedOut, failure, stdout, stderr))
    }
    child.on('close', finish)
    child.on('error', (error) => {
      failure = error.message
      finish(null, null)
    })
  })
}

/** The tool result: what ran, how it ended, and bounded output. */
function shellResult(
  command: string,
  cwdArg: string,
  timeoutMs: number,
  code: number | null,
  signal: NodeJS.Signals | null,
  timedOut: boolean,
  failure: string,
  stdout: string,
  stderr: string
): ToolResult {
  const head = timedOut
    ? `命令超时（${timeoutMs} ms），已终止`
    : failure !== ''
      ? `命令启动失败：${failure}`
      : signal !== null
        ? `命令被信号 ${signal} 终止`
        : `退出码 ${code ?? '?'}`
  const shown = [stripAnsi(stdout).trimEnd(), stripAnsi(stderr).trimEnd()]
    .filter((part) => part !== '')
    .join('\n[stderr]\n')
  const lines = [head, `$ ${command}`]
  if (cwdArg !== '.') lines.push(`（工作目录：${cwdArg}）`)
  lines.push(shown === '' ? '（没有输出）' : trimShell(shown))
  return {
    ok: !timedOut && failure === '' && code === 0,
    text: lines.join('\n')
  }
}

/** Keep the head and tail of long output, where failures usually are. */
function trimShell(text: string): string {
  if (text.length <= MAX_SHELL_CHARS) return text
  const half = Math.floor((MAX_SHELL_CHARS - 40) / 2)
  const tail = text.slice(safeStart(text, text.length - half))
  return `${text.slice(0, safeEnd(text, half))}\n…（省略 ${text.length - half * 2} 字符）\n${tail}`
}
