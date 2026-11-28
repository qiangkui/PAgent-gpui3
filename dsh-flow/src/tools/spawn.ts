/**
 * Starting a child process the way every tool that runs one starts it.
 *
 * `shell`, `python`, `node`, background tasks, `run_tests` and `diagnostics` all
 * end up here, so the Windows sandbox helper is applied in exactly one place.
 * Before this existed, `run_tests` / `diagnostics` spawned their own processes
 * and so ran a project's build scripts *outside* the low-integrity jail — while
 * the very same command line through `shell` was jailed.
 */

import { spawn } from 'node:child_process'
import type { ChildProcess } from 'node:child_process'
import { existsSync, mkdirSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { commandEnv } from './process.ts'

/** What to start: a resolved program, or a whole line for the platform shell. */
export interface SpawnSpec {
  /** Executable (or the whole command line when `shell` is true). */
  file: string
  /** Arguments passed straight to the executable. */
  args: string[]
  /** Run through the platform shell, which is how `shell` works. */
  shell: boolean
  /** Folders placed in front of this command's PATH. */
  pathPrepend?: string[]
}

/** How the child's streams are wired. */
export interface SpawnOptions {
  /** True to hand the child pipes for stdout and stderr. */
  capture: boolean
  /** True to hand it a stdin pipe; false leaves stdin closed. */
  stdin?: boolean
  /** Start in its own group, so a later stop can end it. */
  detached?: boolean
  /**
   * Extra variables for this child only.
   *
   * They are merged over the inherited environment after the secrets are gone,
   * so a caller can pass `GIT_PAGER` or a capture path without re-adding a key.
   */
  env?: Record<string, string | undefined>
}

/**
 * Resolve a bare executable against PATH for CreateProcessAsUser.
 *
 * The sandbox helper passes the application name explicitly; Windows does not
 * search PATH for that form, so `node` / `python` from a runtime resolver must
 * become an absolute path first.
 * @param file - the executable as the caller named it.
 * @param prepend - folders to search before PATH.
 * @returns an absolute path when one was found, the input otherwise.
 */
export function absoluteFile(file: string, prepend: readonly string[] = []): string {
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
 * Start one process, jailed through the sandbox helper when the window gave one.
 *
 * The helper (a Windows low-integrity token + Job Object) runs the same program
 * with the same stdio, so callers cannot tell which path was taken; the spec
 * file it reads is written next to the conversation's `.dsh-flow` folder.
 * @param spec - executable, arguments and how to start them.
 * @param cwd - resolved working directory.
 * @param options - stream wiring and whether to start a detached group.
 * @returns the child process, not yet awaited.
 */
export function spawnTool(spec: SpawnSpec, cwd: string, options: SpawnOptions): ChildProcess {
  const helper = process.env.DSH_SANDBOX_HELPER
  const workspace = process.env.DSH_SANDBOX_WORKSPACE || cwd
  const env: Record<string, string | undefined> = {
    ...commandEnv(spec.pathPrepend ?? []),
    ...(options.env ?? {})
  }
  const stdio: Array<'pipe' | 'ignore'> = [
    options.stdin === true ? 'pipe' : 'ignore',
    options.capture ? 'pipe' : 'ignore',
    options.capture ? 'pipe' : 'ignore'
  ]
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
      file: spec.shell
        ? process.env.ComSpec || 'cmd.exe'
        : absoluteFile(spec.file, spec.pathPrepend ?? []),
      args: spec.shell ? ['/d', '/s', '/c', spec.file] : spec.args
    }
    writeFileSync(specPath, JSON.stringify(payload), 'utf8')
    // The helper's own temporary folder has to be inside the jail, or a tool
    // that writes a temp file would be refused by the low-integrity token.
    const temp = path.join(dir, 'sandbox-tmp')
    env.TEMP = temp
    env.TMP = temp
    return spawn(helper, ['--spec', specPath], {
      cwd,
      windowsHide: true,
      detached: options.detached === true,
      stdio,
      env
    })
  }
  return spawn(spec.file, spec.args, {
    cwd,
    windowsHide: true,
    shell: spec.shell,
    detached: options.detached === true,
    stdio,
    env
  })
}