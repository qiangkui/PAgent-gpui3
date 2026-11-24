/** Spawning and killing the processes the tools start. */

import { spawn } from 'node:child_process'
import type { ChildProcess } from 'node:child_process'

import { delimiter } from 'node:path'

/** Variables a command never inherits, so a tool call cannot print the key. */
const SECRET_ENV = /(api[_-]?key|token|secret|password|credential)/i

/**
 * The environment a command sees: no keys, tokens or passwords.
 * @param pathPrepend - folders to put in front of PATH, project bins included.
 * @returns a copy of the environment with the secrets removed.
 */
export function commandEnv(pathPrepend: readonly string[] = []): Record<string, string | undefined> {
  const env: Record<string, string | undefined> = {}
  for (const [key, value] of Object.entries(process.env)) {
    if (value === undefined || SECRET_ENV.test(key)) continue
    env[key] = value
  }
  if (pathPrepend.length === 0) return env
  // Windows spells the variable as `Path`; keep whichever name this process has.
  const pathKey = Object.keys(env).find((key) => key.toUpperCase() === 'PATH')
  const current = pathKey === undefined ? '' : env[pathKey] ?? ''
  const joined = [...pathPrepend, ...(current === '' ? [] : [current])].join(delimiter)
  if (pathKey !== undefined) {
    env[pathKey] = joined
  } else {
    env.PATH = joined
  }
  return env
}

/**
 * Kill a process and, on Windows, anything it started.
 *
 * A background task has no handle to kill through, so the pid is the only
 * common way in; a group kill covers what the platform supports.
 * @param pid - the process to end.
 */
export function killPid(pid: number): void {
  if (process.platform === 'win32') {
    const killer = spawn('taskkill', ['/pid', String(pid), '/t', '/f'], {
      stdio: 'ignore',
      windowsHide: true
    })
    killer.on('error', () => {
      // Nothing else to try; the task row still reports the pid as gone later.
    })
    return
  }
  // A detached task leads its own group, so the negative pid takes the tree.
  try {
    process.kill(-pid, 'SIGKILL')
  } catch {
    try {
      process.kill(pid, 'SIGKILL')
    } catch {
      // Already gone.
    }
  }
}

/** Kill a running child and, on Windows, anything it started. */
export function killTree(child: ChildProcess): void {
  if (child.pid === undefined) {
    child.kill()
    return
  }
  killPid(child.pid)
}

/**
 * Whether a pid is still running.
 *
 * A background task outlives the engine that started it, so its stored pid is
 * the only evidence left; signal 0 asks the system without touching it.
 * @param pid - the process to ask about.
 */
export function isRunning(pid: number): boolean {
  if (!Number.isInteger(pid) || pid <= 0) return false
  try {
    process.kill(pid, 0)
    return true
  } catch (error) {
    // A process owned by another user is alive but not signalable.
    return (error as NodeJS.ErrnoException).code === 'EPERM'
  }
}
