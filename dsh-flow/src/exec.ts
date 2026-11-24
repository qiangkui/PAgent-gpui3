/**
 * Subprocess execution for `run` steps.
 *
 * The step protocol is one JSON document on stdin and one JSON document on
 * stdout, always UTF-8. The runner forces UTF-8 through the environment because
 * Windows Python otherwise writes its ANSI code page and the reply stops being
 * parseable.
 */

import { spawn } from 'node:child_process'
import type { ChildProcess } from 'node:child_process'

/** Largest captured output kept per stream. */
const CAPTURE_LIMIT = 4 * 1024 * 1024

/** Default kill grace before a hard stop. */
const KILL_GRACE_MS = 2000

/** One finished command. */
export interface CommandOutcome {
  /** Exit code, or -1 when the process never started or was killed. */
  exitCode: number
  /** Captured standard output. */
  stdout: string
  /** Captured standard error. */
  stderr: string
  /** True when a stream hit the capture cap. */
  truncated: boolean
  /** Human-readable refusal for spawn failures. */
  spawnError?: string
}

/** How to start one command. */
export interface CommandRequest {
  /** Executable path or bare name. */
  file: string
  /** Arguments passed verbatim. */
  args: string[]
  /** Working directory. */
  cwd: string
  /** Extra environment variables layered over the current process. */
  env: Record<string, string>
  /** Kill the process after this many milliseconds. */
  timeoutMs: number
  /** UTF-8 JSON document written to stdin, then stdin is closed. */
  stdin?: string
  /** Shell to use instead of exec'ing `file` directly. */
  shell?: boolean | string
}

/**
 * Run one command to completion.
 * @param request - how to start the command.
 * @returns the captured outcome; spawn failures are reported, never thrown.
 */
export function runCommand(request: CommandRequest): Promise<CommandOutcome> {
  return new Promise(resolve => {
    let settled = false
    const stdoutChunks: Buffer[] = []
    const stderrChunks: Buffer[] = []
    let captured = 0
    let truncated = false

    const child = request.shell
      ? spawn([request.file, ...request.args].join(' '), {
        cwd: request.cwd,
        env: { ...process.env, ...request.env },
        shell: typeof request.shell === 'string' ? request.shell : true,
        stdio: ['pipe', 'pipe', 'pipe'],
        windowsHide: true,
      })
      : spawn(request.file, request.args, {
        cwd: request.cwd,
        env: { ...process.env, ...request.env },
        stdio: ['pipe', 'pipe', 'pipe'],
        windowsHide: true,
      })

    const finish = (outcome: CommandOutcome): void => {
      if (settled) return
      settled = true
      clearTimeout(timer)
      resolve(outcome)
    }

    const collect = (target: Buffer[]) => (chunk: Buffer): void => {
      if (captured >= CAPTURE_LIMIT) {
        truncated = true
        return
      }
      const remaining = CAPTURE_LIMIT - captured
      const slice = chunk.length > remaining ? chunk.subarray(0, remaining) : chunk
      target.push(slice)
      captured += slice.length
      if (slice.length < chunk.length) truncated = true
    }

    child.stdout?.on('data', collect(stdoutChunks))
    child.stderr?.on('data', collect(stderrChunks))

    const timer = setTimeout(() => {
      truncated = truncated || false
      killTree(child)
    }, request.timeoutMs)

    child.on('error', error => {
      finish({
        exitCode: -1,
        stdout: Buffer.concat(stdoutChunks).toString('utf8'),
        stderr: Buffer.concat(stderrChunks).toString('utf8'),
        truncated,
        spawnError: describeSpawnError(error),
      })
    })

    child.on('close', code => {
      finish({
        exitCode: code ?? -1,
        stdout: Buffer.concat(stdoutChunks).toString('utf8'),
        stderr: Buffer.concat(stderrChunks).toString('utf8'),
        truncated,
      })
    })

    if (request.stdin !== undefined) child.stdin?.end(request.stdin)
    else child.stdin?.end()
  })
}

/**
 * Turn a spawn failure into an actionable message.
 * @param error - the error emitted by the child process.
 * @returns the message.
 */
function describeSpawnError(error: Error): string {
  const code = (error as NodeJS.ErrnoException).code
  if (code === 'ENOENT') return 'executable not found (check the runtime name or PATH)'
  if (code === 'EACCES') return 'executable is not runnable (permission denied)'
  return error.message
}

/**
 * Stop a command and its descendants.
 * @param child - the running child process.
 */
function killTree(child: ChildProcess): void {
  const pid = child.pid
  if (pid === undefined) return
  if (process.platform === 'win32') {
    spawn('taskkill', ['/pid', String(pid), '/T', '/F'], { windowsHide: true, stdio: 'ignore' })
    return
  }
  child.kill('SIGTERM')
  const hard = setTimeout(() => child.kill('SIGKILL'), KILL_GRACE_MS)
  hard.unref()
}

/**
 * Build the environment every step program receives.
 * @param extra - manifest-provided variables, which win over the defaults.
 * @returns the environment additions.
 */
export function stepEnvironment(extra: Record<string, string>): Record<string, string> {
  return {
    PYTHONUTF8: '1',
    PYTHONIOENCODING: 'utf-8',
    NO_COLOR: '1',
    ...extra,
  }
}
