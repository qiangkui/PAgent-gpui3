/**
 * `wait_for`: hold until something is actually ready.
 *
 * Starting a dev server is only half of the loop; the next command has to know
 * when it answers. Without this the model guesses with `sleep` and either keeps
 * going too early or waits far too long.
 */

import net from 'node:net'
import type { ToolResult } from './types.ts'
import { readDecodedTail, taskLog } from './tasks.ts'
import { MAX_TASK_OUTPUT } from './limits.ts'
import { intArg, textArg } from './paths.ts'
import { safeStart } from './text.ts'

/** How long a wait lasts when the call names no timeout. */
const DEFAULT_TIMEOUT_MS = 30_000

/** Longest wait one call may ask for. */
const MAX_TIMEOUT_MS = 120_000

/** How much of a task log one poll reads: the newest lines are what a pattern matches. */
const WAIT_LOG_BYTES = 8 * 1024 * 1024

/** How often a target is checked when the call names no interval. */
const DEFAULT_INTERVAL_MS = 500

/** Longest time one probe may take, so a black hole does not eat the wait. */
const PROBE_TIMEOUT_MS = 2_000

/** How many matching lines of a task log the answer quotes. */
const MAX_MATCHES = 5

/**
 * Wait for a port, a URL, or a line in a background task's output.
 *
 * Waiting for a task to *finish* is not here: `task action=wait` already does
 * that, with the exit code and a timeout, so a second copy would only give the
 * model two ways to ask the same question.
 * @param workDir - the conversation folder, which holds task records.
 * @param args - `target` plus that target's own arguments.
 * @returns what became ready, and how long it took.
 */
export async function waitFor(
  workDir: string,
  args: Record<string, unknown>
): Promise<ToolResult> {
  const target = textArg(args, 'target').trim().toLowerCase()
  const timeoutMs = clamp(intArg(args, 'timeoutMs') ?? DEFAULT_TIMEOUT_MS, 500, MAX_TIMEOUT_MS)
  const intervalMs = clamp(intArg(args, 'intervalMs') ?? DEFAULT_INTERVAL_MS, 100, 5_000)
  const started = Date.now()
  const deadline = started + timeoutMs
  if (target === 'port') {
    return await waitPort(args, deadline, intervalMs, timeoutMs, started)
  }
  if (target === 'url') {
    return await waitUrl(args, deadline, intervalMs, timeoutMs, started)
  }
  if (target === 'output') {
    return await waitOutput(workDir, args, deadline, intervalMs, timeoutMs, started)
  }
  throw new Error(
    `target 只能是 port、url 或 output；等后台任务结束请用 task 的 action=wait`
  )
}

/** Wait until a TCP port accepts a connection. */
async function waitPort(
  args: Record<string, unknown>,
  deadline: number,
  intervalMs: number,
  timeoutMs: number,
  started: number
): Promise<ToolResult> {
  const host = typeof args.host === 'string' && args.host.trim() ? args.host.trim() : '127.0.0.1'
  const port = intArg(args, 'port')
  if (port === undefined || port < 1 || port > 65_535) {
    throw new Error('缺少参数 port（1–65535）')
  }
  const where = `${host}:${port}`
  for (;;) {
    if (await portOpen(host, port)) {
      return { ok: true, text: `已就绪：${where} 接受连接（等待 ${waited(started)}）` }
    }
    if (Date.now() >= deadline) break
    await sleep(intervalMs)
  }
  throw new Error(`等待超时：${where} 在 ${timeoutMs} ms 内没有接受连接`)
}

/** Wait until a URL answers, whatever the status, as long as it answers. */
async function waitUrl(
  args: Record<string, unknown>,
  deadline: number,
  intervalMs: number,
  timeoutMs: number,
  started: number
): Promise<ToolResult> {
  const url = textArg(args, 'url').trim()
  if (!/^https?:\/\//i.test(url)) throw new Error(`url 要带 http:// 或 https://：${url}`)
  let last = ''
  for (;;) {
    try {
      const response = await fetch(url, {
        method: 'GET',
        redirect: 'manual',
        signal: AbortSignal.timeout(PROBE_TIMEOUT_MS)
      })
      await response.body?.cancel().catch(() => undefined)
      // A 404 from a dev server still means the server is up; only a 5xx and a
      // transport failure say it is not ready yet.
      if (response.status < 500) {
        return {
          ok: true,
          text: `已就绪：${url} → ${response.status}（等待 ${waited(started)}）`
        }
      }
      last = `HTTP ${response.status}`
    } catch (error) {
      last = error instanceof Error ? error.message : String(error)
    }
    if (Date.now() >= deadline) break
    await sleep(intervalMs)
  }
  throw new Error(
    `等待超时：${url} 在 ${timeoutMs} ms 内没有应答${last === '' ? '' : `（最后：${last}）`}`
  )
}

/** Wait until a background task's log holds a line. */
async function waitOutput(
  workDir: string,
  args: Record<string, unknown>,
  deadline: number,
  intervalMs: number,
  timeoutMs: number,
  started: number
): Promise<ToolResult> {
  const pattern = textArg(args, 'pattern').trim()
  if (pattern === '') throw new Error('缺少参数 pattern（要在输出里等的文本）')
  const task = await taskLog(workDir, textArg(args, 'id'))
  const wanted = pattern.toLowerCase()
  let text = ''
  for (;;) {
    // The tail is decoded like a command's output: a log written before the
    // wrapper started writing UTF-8 is still in the program's own code page, and a
    // pattern with Chinese in it would otherwise never match. Only the last few
    // megabytes are read, so polling a server that has been running for days does
    // not pull its whole log into memory every interval.
    text = await readDecodedTail(task.log, WAIT_LOG_BYTES)
    const lines = text.split('\n').filter((line) => line.toLowerCase().includes(wanted))
    if (lines.length > 0) {
      const shown = lines.slice(-MAX_MATCHES).map((line) => line.trim())
      return {
        ok: true,
        text: `已就绪：${task.id} 的输出里出现「${pattern}」（等待 ${waited(started)}）\n${shown.join('\n')}`
      }
    }
    // A task that already exited will never print the line; say so now instead
    // of making the model wait out the whole timeout.
    if (!task.running) {
      const tail = tailOf(text)
      throw new Error(
        `${task.id} 已经结束，输出里没有「${pattern}」${tail === '' ? '' : `；输出末尾：\n${tail}`}`
      )
    }
    if (Date.now() >= deadline) break
    await sleep(intervalMs)
  }
  const tail = tailOf(text)
  throw new Error(
    `等待超时：${task.id} 在 ${timeoutMs} ms 内没有输出「${pattern}」${tail === '' ? '' : `；输出末尾：\n${tail}`}`
  )
}

/** Whether one TCP connection succeeds within the probe timeout. */
function portOpen(host: string, port: number): Promise<boolean> {
  return new Promise<boolean>((resolve) => {
    const socket = net.connect({ host, port })
    const finish = (open: boolean): void => {
      socket.destroy()
      resolve(open)
    }
    socket.setTimeout(PROBE_TIMEOUT_MS)
    socket.once('connect', () => finish(true))
    socket.once('timeout', () => finish(false))
    socket.once('error', () => finish(false))
  })
}

/** The last lines of a log, so a timeout says what the task did print. */
function tailOf(text: string): string {
  const trimmed = text.trimEnd()
  if (trimmed === '') return ''
  const tail =
    trimmed.length > MAX_TASK_OUTPUT
      ? trimmed.slice(safeStart(trimmed, trimmed.length - MAX_TASK_OUTPUT))
      : trimmed
  return tail.split('\n').slice(-10).join('\n')
}

/** Seconds spent so far, with one decimal, for the answer. */
function waited(started: number): string {
  return `${((Date.now() - started) / 1000).toFixed(1)} 秒`
}

/** Keep an interval or timeout inside the range the tool can honour. */
function clamp(value: number, low: number, high: number): number {
  return Math.min(Math.max(value, low), high)
}

/** Wait one interval. */
function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}