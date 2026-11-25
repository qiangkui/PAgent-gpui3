/** Execution tools: shell commands and Python / Node snippets. */

import { spawn } from 'node:child_process'
import { promises as fs } from 'node:fs'
import { mkdirSync, writeFileSync } from 'node:fs'
import { existsSync } from 'node:fs'
import path from 'node:path'
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
 * Resolve a bare executable against PATH for CreateProcessAsUser.
 *
 * The sandbox helper passes the application name explicitly; Windows does not
 * search PATH for that form, so `node` / `python` from a runtime resolver must
 * become an absolute path first.
 */
function absoluteFile(file: string, prepend: readonly string[] = []): string {
  if (path.isAbsolute(file)) return file
  const search = process.env.PATH ?? process.env.Path ?? ''
  const names = file.toLowerCase().endsWith('.exe') ? [file] : [file, `${file}.exe`]
  for (const dir of [...prepend, ...search.split(path.delimiter)]) {
    if (dir === '') continue
    for (const name of names) {
      const candidate = path.join(dir, name)
      if (existsSync(candidate)) return candidate
    }
  }
  return file
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
    // The window can hand commands to the Windows sandbox helper. It lowers the
    // child to Low integrity and keeps it in a Job Object; the command sees the
    // same stdio pipes, so the live rows and the timeout path are unchanged.
    const helper = process.env.DSH_SANDBOX_HELPER
    const workspace = process.env.DSH_SANDBOX_WORKSPACE || cwd
    const env = commandEnv(spec.pathPrepend ?? [])
    let child
    if (helper && process.platform === 'win32') {
      const dir = path.join(workspace, '.dsh-flow')
      mkdirSync(dir, { recursive: true })
      const specPath = path.join(
        dir,
        `sandbox-${process.pid}-${Date.now()}-${Math.floor(Math.random() * 1_000_000)}.json`
      )
      const payload = {
        cwd,
        workspace,
        readonly: process.env.DSH_SANDBOX_READONLY === '1',
        file: spec.shell ? process.env.ComSpec || 'cmd.exe' : absoluteFile(spec.file, spec.pathPrepend ?? []),
        args: spec.shell ? ['/d', '/s', '/c', spec.file] : spec.args
      }
      writeFileSync(specPath, JSON.stringify(payload), 'utf8')
      const temp = path.join(dir, 'sandbox-tmp')
      env.TEMP = temp
      env.TMP = temp
      child = spawn(helper, ['--spec', specPath], {
        cwd,
        windowsHide: true,
        stdio: [input === '' ? 'ignore' : 'pipe', 'pipe', 'pipe'],
        env
      })
    } else {
      child = spawn(spec.file, spec.args, {
        cwd,
        shell: spec.shell,
        windowsHide: true,
        // Staying in the engine's group lets the window's stop kill it too.
        stdio: [input === '' ? 'ignore' : 'pipe', 'pipe', 'pipe'],
        env
      })
    }
    if (input !== '') child.stdin?.end(input)
    let failure = ''
    let timedOut = false
    let settled = false
    let live = Buffer.alloc(0)
    let liveAt = Date.now()
    let liveEncoding: 'utf-8' | 'gb18030' | null = null
    // One decoder for the stream, so a character split across two chunks is
    // completed instead of being painted as a replacement character. A fresh
    // decoder per chunk was why a GBK stream's last character could break.
    let liveDecoder: InstanceType<typeof TextDecoder> | null = null
    const outChunks: Buffer[] = []
    const errChunks: Buffer[] = []
    let outSize = 0
    let errSize = 0

    /**
     * Decode one chunk; the row only ever shows its newest line.
     *
     * The decoder is kept for the whole stream and fed every byte, so a
     * character split across two chunks is completed instead of being painted
     * as a replacement character. Handing each chunk to a fresh decoder (and
     * holding back a UTF-8-shaped tail) is what used to lose the last character
     * of a GBK line.
     * @param bytes - what arrived since the last flush.
     * @param force - true on the final flush, when nothing else is coming.
     */
    const decodeLive = (bytes: Buffer, force: boolean): string => {
      if (
        bytes.length >= 2 &&
        ((bytes[0] === 0xff && bytes[1] === 0xfe) ||
          (bytes[0] === 0xfe && bytes[1] === 0xff))
      ) {
        return decodeText(bytes)
      }
      if (liveDecoder === null) {
        // Pure ASCII is valid in both encodings, so it must not decide.
        if (!bytes.some((byte) => byte >= 0x80)) return bytes.toString('utf8')
        try {
          new TextDecoder('utf-8', { fatal: true }).decode(trimUtf8Tail(bytes))
          liveEncoding = 'utf-8'
        } catch {
          liveEncoding = 'gb18030'
        }
        liveDecoder = new TextDecoder(liveEncoding)
      }
      const decoded = liveDecoder.decode(bytes, { stream: !force })
      const newline = decoded.lastIndexOf('\n')
      return newline >= 0 && newline + 1 < decoded.length ? decoded.slice(newline + 1) : decoded
    }
    /** Forward what arrived since the last tick, at most a few times a second. */
    const flush = (force = false): void => {
      if (!onOutput || live.length === 0) return
      if (!force && Date.now() - liveAt < LIVE_OUTPUT_INTERVAL_MS) return
      const bytes = live
      live = Buffer.alloc(0)
      liveAt = Date.now()
      const text = stripAnsi(decodeLive(bytes, force))
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
