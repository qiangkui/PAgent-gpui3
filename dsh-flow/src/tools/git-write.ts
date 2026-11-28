/**
 * `git_write`: the few repository changes an assistant makes every day.
 *
 * `git` stays read-only — it is the tool the model uses to *look* at a repo, and
 * a free-form write there would be one approval away from `git reset --hard`.
 * This tool is the opposite: three named actions, explicit arguments, and no way
 * to express a destructive one.
 */

import { promises as fs } from 'node:fs'
import type { ToolResult, ToolScope } from './types.ts'
import { GIT_TIMEOUT_MS } from './limits.ts'
import { resolveScoped, textArg } from './paths.ts'
import { killTree } from './process.ts'
import { spawnTool } from './spawn.ts'

/** One `git` run: exit code, stdout and stderr, capped for the transcript. */
interface GitRun {
  code: number
  out: string
  err: string
}

/** Longest message or branch name this tool accepts. */
const MAX_TEXT = 4_000

/** Longest command output kept in the answer. */
const MAX_OUTPUT = 8_000

/**
 * Change the repository: commit, create/switch a branch, or stash.
 * @param workDir - relative paths resolve from here.
 * @param scopeRoot - writable root; null means the whole computer.
 * @param args - `action` plus that action's own arguments.
 * @returns what ran, its exit code and its output.
 */
export async function runGitWrite(
  workDir: string,
  scopeRoot: ToolScope,
  args: Record<string, unknown>
): Promise<ToolResult> {
  const action = textArg(args, 'action').trim().toLowerCase()
  const cwdArg = typeof args.cwd === 'string' && args.cwd.trim() ? args.cwd.trim() : '.'
  const cwd = await resolveScoped(workDir, scopeRoot, cwdArg)
  const info = await fs.stat(cwd).catch(() => null)
  if (!info?.isDirectory()) throw new Error(`找不到运行目录：${cwdArg}`)
  if (!(await isRepository(cwd))) throw new Error(`这个目录不是 git 仓库：${cwdArg}`)

  const steps: Array<{ argv: string[]; output: GitRun }> = []
  const run = async (argv: string[]): Promise<GitRun> => {
    const result = await git(cwd, argv)
    steps.push({ argv, output: result })
    return result
  }
  let failed = ''
  if (action === 'commit') {
    const message = textArg(args, 'message').trim()
    if (message === '') throw new Error('缺少参数 message（提交说明）')
    if (message.length > MAX_TEXT) throw new Error(`提交说明太长（上限 ${MAX_TEXT} 字符）`)
    const paths = pathList(args, 'paths')
    if (args.all === true) {
      const added = await run(['add', '-A'])
      if (added.code !== 0) failed = 'git add -A'
    } else if (paths.length > 0) {
      const added = await run(['add', '--', ...paths])
      if (added.code !== 0) failed = 'git add'
    }
    // Commit only runs when staging worked: a commit after a failed add would
    // silently leave the files the model meant to include out of it.
    if (failed === '') {
      const committed = await run(['commit', '-m', message])
      if (committed.code !== 0) failed = 'git commit'
    }
  } else if (action === 'branch') {
    const name = textArg(args, 'name').trim()
    checkBranchName(name)
    const argv = args.checkout === true ? ['switch', '-c', name] : ['branch', name]
    const created = await run(argv)
    if (created.code !== 0) failed = argv.slice(0, 2).join(' ')
  } else if (action === 'stash') {
    const what =
      typeof args.stashAction === 'string' ? args.stashAction.trim().toLowerCase() : 'push'
    if (what === 'list') {
      const listed = await run(['stash', 'list'])
      if (listed.code !== 0) failed = 'git stash list'
    } else if (what === 'pop') {
      const popped = await run(['stash', 'pop'])
      if (popped.code !== 0) failed = 'git stash pop'
    } else if (what === 'push') {
      const argv = ['stash', 'push']
      if (args.includeUntracked === true) argv.push('-u')
      const message = typeof args.message === 'string' ? args.message.trim() : ''
      if (message !== '') {
        if (message.length > MAX_TEXT) throw new Error(`说明太长（上限 ${MAX_TEXT} 字符）`)
        argv.push('-m', message)
      }
      const stashed = await run(argv)
      if (stashed.code !== 0) failed = 'git stash push'
    } else {
      throw new Error('stashAction 只能是 push、pop 或 list')
    }
  } else {
    throw new Error('action 只能是 commit、branch 或 stash')
  }

  const lines: string[] = []
  for (const step of steps) {
    lines.push(`$ git ${step.argv.join(' ')}`, `退出码 ${step.output.code}`)
    const body = `${step.output.out}${step.output.err}`.trim()
    if (body !== '') lines.push(clip(body))
  }
  return { ok: failed === '', text: lines.join('\n') }
}

/** Whether `cwd` is inside a work tree. */
async function isRepository(cwd: string): Promise<boolean> {
  const result = await git(cwd, ['rev-parse', '--is-inside-work-tree'])
  return result.code === 0 && result.out.trim() === 'true'
}

/**
 * Run one git command without a shell.
 *
 * Pre-commit hooks are left alone: they are the repository's own gate, and
 * skipping them is an explicit human decision rather than a tool default. They
 * do run inside the sandbox, because a hook is project code and a commit is the
 * one place this tool reaches it.
 */
function git(cwd: string, argv: string[]): Promise<GitRun> {
  return new Promise((resolve) => {
    const child = spawnTool(
      { file: 'git', args: argv, shell: false },
      cwd,
      {
        capture: true,
        env: { GIT_PAGER: 'cat', GIT_TERMINAL_PROMPT: '0' }
      }
    )
    let out = ''
    let err = ''
    const timer = setTimeout(() => killTree(child), GIT_TIMEOUT_MS)
    child.stdout?.setEncoding('utf8').on('data', (chunk: string) => {
      if (out.length < MAX_OUTPUT) out += chunk
    })
    child.stderr?.setEncoding('utf8').on('data', (chunk: string) => {
      if (err.length < MAX_OUTPUT) err += chunk
    })
    child.on('error', (error) => {
      clearTimeout(timer)
      resolve({ code: -1, out, err: `${err}${error.message}` })
    })
    child.on('close', (code) => {
      clearTimeout(timer)
      resolve({ code: code ?? -1, out, err })
    })
  })
}

/** The `paths` argument, refused when a path could leave the repository. */
function pathList(args: Record<string, unknown>, name: string): string[] {
  const raw = args[name]
  if (raw === undefined || raw === null) return []
  if (!Array.isArray(raw)) throw new Error(`参数 ${name} 必须是字符串数组`)
  const paths: string[] = []
  for (const entry of raw) {
    if (typeof entry !== 'string') throw new Error(`参数 ${name} 里只能是字符串`)
    const value = entry.trim()
    if (value === '') continue
    if (value.includes('\\') || value.startsWith('/') || /^[A-Za-z]:/.test(value)) {
      throw new Error(`只接受仓库内的相对路径：${entry}`)
    }
    if (value.split('/').includes('..')) throw new Error(`路径不能含 ..：${entry}`)
    paths.push(value)
  }
  return paths
}

/** Branch names git itself would accept, minus the ones that read as flags. */
function checkBranchName(name: string): void {
  if (name === '') throw new Error('缺少参数 name（分支名）')
  if (name.startsWith('-')) throw new Error(`分支名不能以 - 开头：${name}`)
  if (!/^[A-Za-z0-9._/-]+$/.test(name)) {
    throw new Error(`分支名只能用字母、数字、. _ - /：${name}`)
  }
  if (name.endsWith('/') || name.includes('//') || name.includes('..')) {
    throw new Error(`分支名不合法：${name}`)
  }
}

/** Keep the head and tail of a long command output. */
function clip(text: string): string {
  if (text.length <= MAX_OUTPUT) return text
  const head = text.slice(0, MAX_OUTPUT / 2)
  const tail = text.slice(-MAX_OUTPUT / 2)
  return `${head}\n…（共 ${text.length} 字符，中间已省略）\n${tail}`
}