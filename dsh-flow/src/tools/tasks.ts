/**
 * Background commands: start one with `shell`, then read or end it with `task`.
 *
 * A background task outlives the engine that started it — that is the point, a
 * dev server has to keep running after the answer is written — so the record on
 * disk is the only handle left. Each task is one `<id>.json` plus its `<id>.log`
 * under the conversation's `.dsh-flow/tasks` folder.
 */

import { spawn } from 'node:child_process'
import { promises as fs } from 'node:fs'
import { mkdirSync, writeFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import path from 'node:path'
import type { ToolResult } from './types.ts'
import { commandEnv, isRunning, killPid } from './process.ts'
import { intArg, textArg } from './paths.ts'
import { safeStart } from './text.ts'
import { MAX_TASK_LINES, MAX_TASK_OUTPUT } from './limits.ts'

/** One background task as it is kept on disk. */
interface TaskRecord {
  /** `task-1`, `task-2`… unique inside one conversation folder. */
  id: string
  /** The command line the person approved. */
  command: string
  /** Working directory it runs in, absolute. */
  cwd: string
  /** Process id, or 0 when it never started. */
  pid: number
  /** ISO timestamp of the start. */
  startedAt: string
  /** File the command's stdout and stderr go to, absolute. */
  log: string
  /** `stopped` once `task stop` ended it; absent while it is meant to run. */
  stopped?: boolean
}

/** Folder holding one conversation's background tasks. */
function tasksDir(workDir: string): string {
  return path.join(workDir, '.dsh-flow', 'tasks')
}

/** Where one task's output goes. */
function logPath(workDir: string, id: string): string {
  return path.join(tasksDir(workDir), `${id}.log`)
}

/**
 * Start a command that keeps running after the answer.
 * @param workDir - conversation folder, which holds the task records.
 * @param command - the command line, run through the platform shell.
 * @param cwd - resolved working directory.
 * @param cwdArg - the directory as the model wrote it, for the answer.
 * @param name - what to call the task in its row, optional.
 * @returns the id, the pid and how to read it later.
 */
export async function startBackground(
  workDir: string,
  command: string,
  cwd: string,
  cwdArg: string,
  name = ''
): Promise<ToolResult> {
  const dir = tasksDir(workDir)
  await fs.mkdir(dir, { recursive: true })
  const id = `task-${await nextNumber(dir)}`
  const helper = process.env.DSH_SANDBOX_HELPER
  const workspace = process.env.DSH_SANDBOX_WORKSPACE || workDir
  const readonly = process.env.DSH_SANDBOX_READONLY === '1'
  // A read-only sandbox cannot write the task folder, so its log lives in the
  // Low-integrity temp folder instead; the record still points at it.
  const log = helper && readonly
    ? path.join(workspace, '.dsh-flow', 'sandbox-tmp', `${id}.log`)
    : logPath(workDir, id)
  // A detached process cannot hand a file handle to its child on Windows, so a
  // small wrapper runs the command and writes the log itself; it is the process
  // whose pid is recorded, and the tree kill reaches the command from there.
  const wrapper = fileURLToPath(new URL('./task-run.ts', import.meta.url))
  const env = commandEnv()
  let child
  if (helper && process.platform === 'win32') {
    const temp = path.join(workspace, '.dsh-flow', 'sandbox-tmp')
    mkdirSync(path.dirname(log), { recursive: true })
    const specPath = path.join(dir, `sandbox-${id}.json`)
    const payload = {
      cwd,
      workspace,
      readonly,
      file: process.execPath,
      args: [wrapper, log, cwd, command]
    }
    writeFileSync(specPath, JSON.stringify(payload), 'utf8')
    env.TEMP = temp
    env.TMP = temp
    child = spawn(helper, ['--spec', specPath], {
      cwd,
      windowsHide: true,
      detached: true,
      stdio: 'ignore',
      env
    })
  } else {
    child = spawn(process.execPath, [wrapper, log, cwd, command], {
      cwd,
      windowsHide: true,
      // Detached so the answer can end while it keeps running; its own group is
      // also what `killPid` ends.
      detached: true,
      stdio: 'ignore',
      env
    })
  }
  const pid = child.pid ?? 0
  child.on('error', () => {
    // Recorded below with pid 0; an empty log is the only trace left.
  })
  child.unref()
  const record: TaskRecord = {
    id,
    command,
    cwd,
    pid,
    startedAt: new Date().toISOString(),
    log
  }
  await fs.writeFile(path.join(dir, `${id}.json`), JSON.stringify(record, null, 2))
  if (pid === 0) {
    return { ok: false, text: `后台任务 ${id} 没能启动：${command}` }
  }
  const lines = [
    `已在后台启动 ${id}${name === '' ? '' : `（${name}）`}，pid ${pid}`,
    `$ ${command}`
  ]
  if (cwdArg !== '.') lines.push(`（工作目录：${cwdArg}）`)
  lines.push(
    `它会在这次问答结束后继续运行；用 task 工具 action=output id=${id} 看输出，action=stop 结束。`,
    `日志：${log}`
  )
  return { ok: true, text: lines.join('\n') }
}

/** The next free `task-N` number in one folder. */
async function nextNumber(dir: string): Promise<number> {
  const names = await fs.readdir(dir).catch(() => [] as string[])
  let highest = 0
  for (const name of names) {
    const match = /^task-(\d+)\.json$/.exec(name)
    if (match) highest = Math.max(highest, Number(match[1]))
  }
  return highest + 1
}

/** Read every task record in one conversation folder, oldest id first. */
async function readRecords(workDir: string): Promise<TaskRecord[]> {
  const dir = tasksDir(workDir)
  const names = await fs.readdir(dir).catch(() => [] as string[])
  const records: TaskRecord[] = []
  for (const name of names.filter((entry) => entry.endsWith('.json'))) {
    const text = await fs.readFile(path.join(dir, name), 'utf8').catch(() => '')
    try {
      const record = JSON.parse(text) as TaskRecord
      if (typeof record.id === 'string' && typeof record.command === 'string') {
        records.push(record)
      }
    } catch {
      // A half-written record is not worth failing a whole listing over.
    }
  }
  return records.sort((left, right) => left.id.localeCompare(right.id, 'en', { numeric: true }))
}

/** `running`, `exited`, or `stopped`. */
function state(record: TaskRecord): 'running' | 'exited' | 'stopped' {
  if (record.stopped) return 'stopped'
  return isRunning(record.pid) ? 'running' : 'exited'
}

/** One line describing a task, without its output. */
function describeTask(record: TaskRecord): string {
  const status = state(record)
  const when = record.startedAt.replace('T', ' ').slice(0, 19)
  const name = record.command.length > 80 ? `${record.command.slice(0, 80)}…` : record.command
  return `${record.id} [${status}] pid ${record.pid} · ${when} · $ ${name}`
}

/**
 * The `task` tool: list background tasks, read one's output, or end one.
 * @param workDir - conversation folder holding the records.
 * @param args - `action` plus `id` for output / stop.
 * @returns the listing, the tail of a log, or what was stopped.
 */
export async function runTask(workDir: string, args: Record<string, unknown>): Promise<ToolResult> {
  const action = textArg(args, 'action').trim().toLowerCase()
  const records = await readRecords(workDir)
  if (action === 'list') {
    if (records.length === 0) {
      return { ok: true, text: '这个对话还没有后台任务；用 shell 的 background=true 启动一个。' }
    }
    const shown = records.slice(-20).map(describeTask)
    return { ok: true, text: `后台任务 ${records.length} 个（最新 20 个）：\n${shown.join('\n')}` }
  }
  if (action === 'output') {
    const record = pick(records, textArg(args, 'id'))
    const requested = intArg(args, 'lines') ?? 60
    const lines = Math.min(Math.max(requested, 1), MAX_TASK_LINES)
    const text = await fs.readFile(record.log, 'utf8').catch(() => '')
    const tail =
      text.length > MAX_TASK_OUTPUT ? text.slice(safeStart(text, text.length - MAX_TASK_OUTPUT)) : text
    const shown = tail.split('\n').slice(-lines).join('\n').trimEnd()
    const head = `${record.id} [${state(record)}] pid ${record.pid}\n$ ${record.command}`
    const body = shown === '' ? '（还没有输出）' : shown
    return { ok: true, text: `${head}\n${body}` }
  }
  if (action === 'stop' || action === 'stopall') {
    const targets =
      action === 'stopall' ? records.filter((record) => state(record) === 'running') : [pick(records, textArg(args, 'id'))]
    if (targets.length === 0) return { ok: true, text: '没有正在运行的后台任务。' }
    const stopped: string[] = []
    for (const record of targets) {
      if (state(record) === 'running') killPid(record.pid)
      record.stopped = true
      await fs
        .writeFile(path.join(tasksDir(workDir), `${record.id}.json`), JSON.stringify(record, null, 2))
        .catch(() => undefined)
      stopped.push(`${record.id}（pid ${record.pid}）`)
    }
    return { ok: true, text: `已结束后台任务：${stopped.join('、')}` }
  }
  throw new Error('action 必须是 list、output、stop 或 stopAll')
}

/** The record a call named, or a readable error. */
function pick(records: TaskRecord[], id: string): TaskRecord {
  const wanted = id.trim().toLowerCase()
  if (wanted === '') throw new Error('缺少参数 id（例如 task-1）')
  const found = records.find((record) => record.id.toLowerCase() === wanted)
  if (!found) {
    const known = records.map((record) => record.id).join('、')
    throw new Error(known === '' ? `没有这个后台任务：${id}` : `没有这个后台任务：${id}（现有：${known}）`)
  }
  return found
}
