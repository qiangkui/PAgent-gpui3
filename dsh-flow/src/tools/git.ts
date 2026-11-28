/** The read-only git tool. */

import { promises as fs } from 'node:fs'
import type { ToolResult, ToolScope } from './types.ts'
import { GIT_TIMEOUT_MS, MAX_GIT_CHARS } from './limits.ts'
import { decodeOutput, keepBytes, safeEnd, stripAnsi } from './text.ts'
import { resolveScoped, textArg } from './paths.ts'
import { MAX_SHELL_BUFFER } from './exec.ts'
import { killTree } from './process.ts'
import { spawnTool } from './spawn.ts'

/** Subcommands `git` may run: nothing here writes to the repository. */
const GIT_READONLY = new Set([
  'status',
  'diff',
  'log',
  'show',
  'ls-files',
  'rev-parse',
  'shortlog',
  'describe',
  'blame'
])

/** Flags that could make a read-only subcommand write, read outside the repo, or run a program. */
function refusedGitArg(arg: string): boolean {
  if (arg === '-c' || arg === '-O') return true
  return /^--(output|exec|upload-pack|receive-pack|open-files-in-pager|ext-diff|no-index|textconv|filters)(=|$)/.test(
    arg
  )
}

/** `git`: read-only inspection of the repository. */
export async function runGit(
  workDir: string,
  scopeRoot: ToolScope,
  args: Record<string, unknown>
): Promise<ToolResult> {
  const command = textArg(args, 'command').trim()
  if (!GIT_READONLY.has(command)) {
    throw new Error(`只允许只读子命令：${[...GIT_READONLY].join(' / ')}`)
  }
  const extra: string[] = []
  if (args.args !== undefined) {
    if (!Array.isArray(args.args)) throw new Error('参数 args 不是数组')
    for (const item of args.args) {
      if (typeof item !== 'string') throw new Error('参数 args 里只能是字符串')
      if (refusedGitArg(item)) throw new Error(`不接受这个参数：${item}`)
      extra.push(item)
    }
  }
  const cwdArg = typeof args.cwd === 'string' && args.cwd.trim() ? args.cwd : '.'
  const cwd = await resolveScoped(workDir, scopeRoot, cwdArg)
  const info = await fs.stat(cwd).catch(() => null)
  if (!info?.isDirectory()) throw new Error(`找不到运行目录：${cwdArg}`)
  // A repository can configure textconv / external diff drivers, which run
  // programs; a read-only look must not trigger them.
  const filters =
    command === 'blame'
      ? ['--no-textconv']
      : command === 'diff' || command === 'log' || command === 'show'
        ? ['--no-textconv', '--no-ext-diff']
        : []
  // `core.fsmonitor` can name a program that `git status` runs; a read-only
  // look should not start anything from repository config.
  const argv = ['-c', 'core.fsmonitor=false', '--no-pager', command, ...filters, ...extra]
  if (typeof args.path === 'string' && args.path.trim()) argv.push('--', args.path)
  return await spawnGit(argv, cwd, cwdArg)
}

/** Run one git command, killing it on timeout and keeping its output bounded. */
function spawnGit(argv: string[], cwd: string, cwdArg: string): Promise<ToolResult> {
  return new Promise((resolve) => {
    const line = `git ${argv.join(' ')}`
    // Through the same jail as every other spawned tool: a repository can carry
    // `textconv` drivers and hooks, and a read-only look must not run them
    // outside the sandbox. `commandEnv` already drops the pager variables, so
    // they are passed explicitly here.
    const child = spawnTool(
      { file: 'git', args: argv, shell: false },
      cwd,
      {
        capture: true,
        env: { GIT_PAGER: 'cat', GIT_TERMINAL_PROMPT: '0' }
      }
    )
    let timedOut = false
    let settled = false
    const outChunks: Buffer[] = []
    const errChunks: Buffer[] = []
    let outSize = 0
    let errSize = 0
    const timer = setTimeout(() => {
      timedOut = true
      killTree(child)
    }, GIT_TIMEOUT_MS)
    child.stdout?.on('data', (chunk: Buffer | string) => {
      const bytes = typeof chunk === 'string' ? Buffer.from(chunk) : chunk
      outSize = keepBytes(outChunks, outSize, bytes, MAX_SHELL_BUFFER)
    })
    child.stderr?.on('data', (chunk: Buffer | string) => {
      const bytes = typeof chunk === 'string' ? Buffer.from(chunk) : chunk
      errSize = keepBytes(errChunks, errSize, bytes, MAX_SHELL_BUFFER)
    })
    const finish = (code: number | null, signal: NodeJS.Signals | null): void => {
      if (settled) return
      settled = true
      clearTimeout(timer)
      const stdout = decodeOutput(Buffer.concat(outChunks))
      const stderr = decodeOutput(Buffer.concat(errChunks))
      const head = timedOut
        ? `命令超时（${GIT_TIMEOUT_MS} ms），已终止`
        : signal !== null
          ? `命令被信号 ${signal} 终止`
          : `退出码 ${code ?? '?'}`
      const body = [stripAnsi(stdout).trimEnd(), stripAnsi(stderr).trimEnd()]
        .filter((part) => part !== '')
        .join('\n')
      const shown = body.length <= MAX_GIT_CHARS ? body : clipGit(body)
      resolve({
        ok: !timedOut && signal === null && code === 0,
        text: `${head}\n$ ${line}${cwdArg === '.' ? '' : `\n# 工作目录：${cwdArg}`}\n${shown}`
      })
    }
    child.on('close', finish)
    child.on('error', (error) => {
      if (settled) return
      settled = true
      clearTimeout(timer)
      resolve({ ok: false, text: `命令启动失败：${error.message}` })
    })
  })
}

/** Keep both ends of a long git answer, where errors and summaries live. */
function clipGit(text: string): string {
  const note = `\n…（输出过长，只保留前 ${MAX_GIT_CHARS} 字符）`
  return `${text.slice(0, safeEnd(text, MAX_GIT_CHARS))}${note}`
}
