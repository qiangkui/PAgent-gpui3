/**
 * Background commands: start one with `shell`, then read or end it with `task`.
 *
 * A background task outlives the engine that started it — that is the point, a
 * dev server has to keep running after the answer is written — so the record on
 * disk is the only handle left. Each task is one `<id>.json` plus its `<id>.log`
 * under the conversation's `.dsh-flow/tasks` folder.
 */

import { promises as fs } from 'node:fs'
import { fileURLToPath } from 'node:url'
import path from 'node:path'
import type { ToolResult } from './types.ts'
import { isRunning, killPid } from './process.ts'
import { intArg, textArg } from './paths.ts'
import { decodeOutput, safeStart } from './text.ts'
import { MAX_TASK_LINES, MAX_TASK_OUTPUT } from './limits.ts'
import { spawnTool } from './spawn.ts'

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

/** How much of a task's log is read back: the answer only shows the last lines. */
const TAIL_BYTES = 4 * MAX_TASK_OUTPUT

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
  name = '',
  stdin = ''
): Promise<ToolResult> {
  const dir = tasksDir(workDir)
  await fs.mkdir(dir, { recursive: true })
  const id = `task-${await nextNumber(dir)}`
  const readonly = process.env.DSH_SANDBOX_READONLY === '1'
  // A read-only sandbox cannot write the task folder, so its log and its input
  // queue live in the Low-integrity temp folder instead; the record still points
  // at them.
  const log = readonly && process.env.DSH_SANDBOX_HELPER
    ? path.join(process.env.DSH_SANDBOX_WORKSPACE || workDir, '.dsh-flow', 'sandbox-tmp', `${id}.log`)
    : logPath(workDir, id)
  // Input is queued in a file the wrapper watches; creating it up front means a
  // task started with `stdin` already has its first lines waiting. The close
  // marker stays absent until `task close` asks for EOF.
  const control = path.join(path.dirname(log), `${id}.stdin`)
  const closeMarker = path.join(path.dirname(log), `${id}.stdin.close`)
  await fs.mkdir(path.dirname(control), { recursive: true })
  await fs.writeFile(control, stdin, 'utf8')
  // A detached process cannot hand a file handle to its child on Windows, so a
  // small wrapper runs the command and writes the log itself; it is the process
  // whose pid is recorded, and the tree kill reaches the command from there.
  const wrapper = fileURLToPath(new URL('./task-run.ts', import.meta.url))
  // Detached so the answer can end while it keeps running; its own group is also
  // what `killPid` ends. `spawnTool` swaps in the sandbox helper when the window
  // provided one, and keeps the same detached group + ignored stdio.
  const child = spawnTool(
    {
      file: process.execPath,
      args: [wrapper, log, cwd, command, control, closeMarker],
      shell: false
    },
    cwd,
    { capture: false, detached: true }
  )
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
    `它会在这次问答结束后继续运行；用 task 工具 action=output id=${id} 看输出，action=input 写输入（只有读取 stdin 的程序收得到），action=stop 结束。`,
    `日志：${log}`
  )
  return { ok: true, text: lines.join('\n') }
}

/**
 * The log file of one recorded task, for tools that watch its output.
 * @param workDir - conversation folder holding the records.
 * @param id - the task id, for example `task-1`.
 * @returns its log path, whether it is still running, and its command.
 */
export async function taskLog(
  workDir: string,
  id: string
): Promise<{ id: string; log: string; running: boolean; command: string }> {
  const record = pick(await readRecords(workDir), id)
  return {
    id: record.id,
    log: record.log,
    running: state(record) === 'running',
    command: record.command
  }
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

/**
 * The end of a log, decoded like a command's output.
 *
 * `shell` decodes its output line by line; a log written to disk has the same
 * problem, so it goes through the same decoder instead of being read as UTF-8.
 * Only the last `bytes` are read, which bounds the memory one poll costs.
 * @param file - the log to read.
 * @param bytes - how much of its end to read.
 * @returns the tail, empty when the file is gone.
 */
export async function readDecodedTail(file: string, bytes: number): Promise<string> {
  const handle = await fs.open(file, 'r').catch(() => null)
  if (handle === null) return ''
  try {
    const info = await handle.stat()
    const start = Math.max(0, info.size - bytes)
    const length = info.size - start
    const buffer = Buffer.alloc(length)
    await handle.read(buffer, 0, length, start)
    return decodeOutput(buffer)
  } catch {
    return ''
  } finally {
    await handle.close().catch(() => undefined)
  }
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
    // Only the tail is read: a server's log can be megabytes, and the answer
    // shows its last lines. The bytes go through `decodeOutput`, because a
    // Windows program may write the OEM code page rather than UTF-8.
    const text = await readDecodedTail(record.log, TAIL_BYTES)
    const tail =
      text.length > MAX_TASK_OUTPUT ? text.slice(safeStart(text, text.length - MAX_TASK_OUTPUT)) : text
    const shown = tail.split('\n').slice(-lines).join('\n').trimEnd()
    const head = `${record.id} [${state(record)}] pid ${record.pid}\n$ ${record.command}`
    const body = shown === '' ? '（还没有输出）' : shown
    return { ok: true, text: `${head}\n${body}` }
  }
  if (action === 'input') {
    const record = pick(records, textArg(args, 'id'))
    const text = textArg(args, 'text')
    if (text === '') throw new Error('缺少参数 text（要写进 stdin 的内容）')
    if (state(record) !== 'running') {
      throw new Error(`${record.id} 已经不在运行，输入不会送达`)
    }
    const control = path.join(path.dirname(record.log), `${record.id}.stdin`)
    await fs.mkdir(path.dirname(control), { recursive: true })
    // A whole line is the unit a prompt expects; the wrapper forwards the bytes
    // as they land, so the trailing break is added here rather than guessed at.
    await fs.appendFile(control, text.endsWith('\n') ? text : `${text}\n`, 'utf8')
    return {
      ok: true,
      text: `已向 ${record.id} 写入输入（${text.length} 字符）。只有读取 stdin 的程序会看到它；用 task output 看反应。`
    }
  }
  if (action === 'close') {
    const record = pick(records, textArg(args, 'id'))
    if (state(record) !== 'running') {
      throw new Error(`${record.id} 已经不在运行，不需要再关闭 stdin`)
    }
    const marker = path.join(path.dirname(record.log), `${record.id}.stdin.close`)
    await fs.writeFile(marker, '1', 'utf8')
    return {
      ok: true,
      text: `已通知 ${record.id} 关闭 stdin：它读到 EOF 后会继续（很多 CLI 等在这一点）。本来就只读一次输入或根本不读 stdin 的程序不会因此改变。`
    }
  }
  if (action === 'wait') {
    const record = pick(records, textArg(args, 'id'))
    if (state(record) !== 'running') {
      const text = await fs.readFile(record.log, 'utf8').catch(() => '')
      const note = exitNote(text)
      return {
        ok: true,
        text: `${record.id} ${record.stopped ? '早已停止' : '早已结束'}${note === '' ? '' : `（${note}）`}。`
      }
    }
    const asked = intArg(args, 'timeoutMs') ?? 30_000
    const timeoutMs = Math.min(Math.max(asked, 500), 120_000)
    const started = Date.now()
    while (state(record) === 'running' && Date.now() - started < timeoutMs) {
      await new Promise((resolve) => setTimeout(resolve, 150))
    }
    if (state(record) === 'running') {
      const waited = ((Date.now() - started) / 1000).toFixed(1)
      throw new Error(
        `${record.id} 还在运行（已等 ${waited} 秒）。用 task stop 结束它，或加大 timeoutMs（上限 120000）。`
      )
    }
    const text = await fs.readFile(record.log, 'utf8').catch(() => '')
    const note = exitNote(text)
    return {
      ok: true,
      text: `${record.id} ${record.stopped ? '已停止' : '已结束'}${note === '' ? '' : `（${note}）`}，用时 ${((Date.now() - started) / 1000).toFixed(1)} 秒。`
    }
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
  throw new Error('action 必须是 list、output、input、close、wait、stop 或 stopAll')
}

/**
 * How a finished background task ended, as its wrapper wrote it to the log.
 * @param text - the task's log.
 * @returns `退出码 N`, `被信号 … 结束`, or empty when the log says neither.
 */
export function exitNote(text: string): string {
  const tail = text.slice(-4_000)
  const exit = /\[退出码 (-?\d+)\]/.exec(tail)
  if (exit) return `退出码 ${exit[1]}`
  const signal = /\[被信号 ([^\]]+?) 结束\]/.exec(tail)
  if (signal) return `被信号 ${signal[1]} 结束`
  const failed = /\[无法启动：([^\]]+)\]/.exec(tail)
  if (failed) return `无法启动：${failed[1]}`
  return ''
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
