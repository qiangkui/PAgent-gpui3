/**
 * Run-level long-lived processes.
 *
 * A manifest may declare services (a browser, a database, a local HTTP server)
 * that start before the steps and are stopped when the run settles. Each one is
 * spawned once, its output is forwarded to the run log, and a `ready` probe
 * holds the run until the service actually answers.
 */

import { spawn } from 'node:child_process'
import type { ChildProcess } from 'node:child_process'
import { resolve as resolvePath } from 'node:path'
import { stepEnvironment } from './exec.ts'
import { describeRuntime, resolveRuntime } from './runtime.ts'
import { renderDeep, renderString } from './template.ts'
import type { RunDefaults, Scope, ServiceSpec, TraceRecord } from './types.ts'

/** Default time a service gets to answer its readiness probe. */
const READY_TIMEOUT_MS = 15_000

/** Default delay between readiness probes. */
const READY_INTERVAL_MS = 250

/** Longest log line kept from a service. */
const MAX_LINE_CHARS = 2_000

/** How many last lines are quoted when a service fails to start. */
const TAIL_LINES = 12

/** One service that is up and can be stopped. */
export interface RunningService {
  /** Manifest key. */
  name: string
  /** Stop the process tree and wait for it to go away. */
  stop: () => Promise<void>
}

/** Everything needed to start the services of one run. */
export interface ServiceStart {
  /** Declared services, in manifest order. */
  services: Record<string, ServiceSpec>
  /** Shared run defaults (`cwd`, `env`). */
  defaults: RunDefaults
  /** Live scope used to render commands and env values. */
  scope: Scope
  /** Directory the manifest lives in. */
  dir: string
  /** Run log sink. */
  log: (line: string) => void
  /** Trace sink. */
  trace?: (record: TraceRecord) => void
  /** True when the caller only wants rendered output. */
  dryRun?: boolean
}

/**
 * Start every declared service and wait until each one is ready.
 *
 * On failure the services that already started are stopped before the error is
 * rethrown, so a run never leaks half a stack.
 * @param options - services plus the run context.
 * @returns the running services, in declaration order.
 */
export async function startServices(options: ServiceStart): Promise<RunningService[]> {
  const entries = Object.entries(options.services)
  if (entries.length === 0 || options.dryRun === true) return []
  const running: RunningService[] = []
  try {
    for (const [name, spec] of entries) {
      running.push(await startOne(name, spec, options))
    }
    return running
  } catch (error) {
    await stopServices(running, options)
    throw error
  }
}

/** Stop every service, newest first, and never throw. */
export async function stopServices(
  running: RunningService[],
  options: Pick<ServiceStart, 'trace' | 'log'>
): Promise<void> {
  for (const service of running.slice().reverse()) {
    await service.stop()
    options.trace?.({
      ts: new Date().toISOString(),
      type: 'service/end',
      id: service.name,
      detail: 'stopped'
    })
    options.log(`service ${service.name} stopped`)
  }
}

/** Spawn one service and hold until it is ready. */
async function startOne(
  name: string,
  spec: ServiceSpec,
  options: ServiceStart
): Promise<RunningService> {
  const rendered = renderDeep(spec.run, options.scope) as typeof spec.run
  const parts = Array.isArray(rendered.command) ? rendered.command.slice() : [rendered.command]
  const commandLine = parts.join(' ')
  const resolved = rendered.shell ? null : resolveRuntime(parts[0] ?? '')
  if (!rendered.shell && resolved === null) {
    throw new Error(`service "${name}": ${describeRuntime(parts[0] ?? '')}`)
  }
  const file = rendered.shell ? commandLine : resolved!.file
  const args = rendered.shell ? [] : [...resolved!.prefixArgs, ...parts.slice(1)]
  const cwd = resolvePath(options.dir, rendered.cwd ?? options.defaults.cwd ?? '.')
  const env = stepEnvironment({ ...options.defaults.env, ...rendered.env })

  options.trace?.({
    ts: new Date().toISOString(),
    type: 'service/start',
    id: name,
    command: commandLine
  })
  options.log(`service ${name} starting: ${commandLine}`)

  const child = spawn(file, args, {
    cwd,
    env: { ...process.env, ...env },
    shell: rendered.shell ? true : false,
    stdio: ['ignore', 'pipe', 'pipe'],
    windowsHide: true
  })

  let spawnError = ''
  const tail: string[] = []
  const keep = (line: string): void => {
    const trimmed = line.replace(/\r$/, '').slice(0, MAX_LINE_CHARS)
    if (trimmed.trim() === '') return
    options.log(`[${name}] ${trimmed}`)
    tail.push(trimmed)
    if (tail.length > TAIL_LINES) tail.shift()
  }
  pipeLines(child.stdout, keep)
  pipeLines(child.stderr, keep)

  const spawned = new Promise<boolean>((resolve) => {
    child.once('spawn', () => resolve(true))
    child.once('error', (error) => {
      spawnError = error.message
      resolve(false)
    })
  })
  if (!(await spawned)) {
    await stopChild(child, false)
    throw new Error(`service "${name}" could not start: ${spawnError}`)
  }

  const exited = new Promise<{ code: number | null; signal: NodeJS.Signals | null }>((resolve) => {
    child.once('close', (code, signal) => resolve({ code, signal }))
  })

  const ready =
    spec.ready === undefined
      ? undefined
      : { ...spec.ready, url: renderString(spec.ready.url, options.scope) }
  try {
    await waitReady(name, ready, exited, tail, options.log)
  } catch (error) {
    await stopChild(child, false)
    throw error
  }

  options.trace?.({ ts: new Date().toISOString(), type: 'service/ready', id: name })
  options.log(`service ${name} ready`)
  return {
    name,
    stop: async () => {
      await stopChild(child, false)
    }
  }
}

/** Readiness probe: no probe means "give it a moment, then assume ready". */
async function waitReady(
  name: string,
  ready: ServiceSpec['ready'],
  exited: Promise<{ code: number | null; signal: NodeJS.Signals | null }>,
  tail: string[],
  log: (line: string) => void
): Promise<void> {
  if (ready === undefined) {
    const outcome = await Promise.race([exited, sleep(200).then(() => null)])
    if (outcome !== null) {
      throw new Error(
        `service "${name}" exited before it was ready (${exitLabel(outcome)})${tailHint(tail)}`
      )
    }
    return
  }
  const timeoutMs = ready.timeoutMs ?? READY_TIMEOUT_MS
  const intervalMs = ready.intervalMs ?? READY_INTERVAL_MS
  const deadline = Date.now() + timeoutMs
  while (Date.now() <= deadline) {
    const outcome = await Promise.race([
      exited,
      fetch(ready.url, { signal: AbortSignal.timeout(Math.max(intervalMs, 1_000)) })
        .then((response) => ({ response }))
        .catch(() => null)
    ])
    if (outcome !== null && 'code' in outcome) {
      throw new Error(
        `service "${name}" exited before it was ready (${exitLabel(outcome)})${tailHint(tail)}`
      )
    }
    if (outcome !== null && 'response' in outcome && outcome.response.ok) {
      return
    }
    log(`service ${name} not ready yet`)
    await sleep(intervalMs)
  }
  throw new Error(
    `service "${name}" did not become ready within ${timeoutMs} ms${tailHint(tail)}`
  )
}

/** Format an exit for an error message. */
function exitLabel(outcome: { code: number | null; signal: NodeJS.Signals | null }): string {
  if (outcome.signal !== null) return `signal ${outcome.signal}`
  return `exit ${outcome.code ?? '?'}`
}

/** Quote the last output lines, when there are any. */
function tailHint(tail: string[]): string {
  if (tail.length === 0) return ''
  return `\n最近输出：\n${tail.map((line) => `  ${line}`).join('\n')}`
}

/** Split a stream into lines without dropping a trailing partial line. */
function pipeLines(stream: NodeJS.ReadableStream | null, onLine: (line: string) => void): void {
  if (stream === null) return
  let pending = ''
  stream.setEncoding('utf8')
  stream.on('data', (chunk: string) => {
    pending += chunk
    let newline = pending.indexOf('\n')
    while (newline >= 0) {
      onLine(pending.slice(0, newline))
      pending = pending.slice(newline + 1)
      newline = pending.indexOf('\n')
    }
  })
  stream.on('end', () => {
    if (pending !== '') onLine(pending)
  })
}

/** Stop one child: ask politely on Unix, take the tree down on Windows. */
function stopChild(child: ChildProcess, force: boolean): Promise<void> {
  if (child.exitCode !== null || child.signalCode !== null) return Promise.resolve()
  return new Promise((resolve) => {
    let settled = false
    const finish = (): void => {
      if (settled) return
      settled = true
      resolve()
    }
    child.once('close', finish)
    if (process.platform === 'win32') {
      // Console processes ignore a polite taskkill, so take the tree directly.
      if (child.pid !== undefined) {
        const killer = spawn(
          'taskkill',
          ['/pid', String(child.pid), '/t', '/f'],
          { stdio: 'ignore', windowsHide: true }
        )
        killer.on('error', () => child.kill())
      } else {
        child.kill()
      }
    } else {
      child.kill(force ? 'SIGKILL' : 'SIGTERM')
    }
    const hard = setTimeout(() => {
      if (settled) return
      if (process.platform === 'win32' && child.pid !== undefined) {
        const killer = spawn(
          'taskkill',
          ['/pid', String(child.pid), '/t', '/f'],
          { stdio: 'ignore', windowsHide: true }
        )
        killer.on('error', () => child.kill())
      } else {
        child.kill('SIGKILL')
      }
      setTimeout(finish, 500).unref()
    }, 2_000)
    hard.unref()
  })
}

/** Wait without holding the event loop open. */
function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms).unref())
}
