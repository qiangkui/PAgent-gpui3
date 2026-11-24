/**
 * The path guard a command runs under.
 *
 * `shell`, `python` and `node` can do far more than the file tools, so the
 * window passes the folders a command may reach and the engine refuses one that
 * names anything else. This is a **heuristic**: it reads the command text, it
 * does not confine the process. A path built at run time (`$env:USERPROFILE`,
 * `os.path.expanduser`) gets past it, which is why the confirmation card is
 * still the real gate.
 */

import os from 'node:os'
import path from 'node:path'
import { isOutside, realish, resolveScoped } from './paths.ts'
import type { ToolScope } from './types.ts'

/** The folders a command may run in and refer to. */
export interface Sandbox {
  /** Absolute roots, already resolved; the working directory and its scope. */
  roots: string[]
}

/**
 * `C:\…` and `C:/…`, the shape a Windows absolute path takes.
 *
 * The lookbehind rules out the tail of a word, so the `s:` in `https://x` is
 * read as part of the scheme; the second guard does the same for `//`.
 */
const DRIVE_PATH = /(?<![A-Za-z0-9_])[A-Za-z]:(?![\\/]{2})[\\/][^\s"'`|<>]*/g

/** `\\server\share\…`. */
const UNC_PATH = /\\\\[^\s"'`|<>]+/g

/**
 * `/srv/app`, but not the `//` in a URL or the `/a` in `https://x/a`.
 *
 * The lookbehind skips the separator right after a scheme or another slash.
 */
const UNIX_PATH = /(?<![\w.:/~\\])\/[^\s"'`|<>]*/g

/** `~/notes` and `~\notes`. */
const HOME_PATH = /(?<![\w.])~[\\/][^\s"'`|<>]*/g

/** A `..` step, with everything it walks through. */
const PARENT_PATH = /(?:^|[\s"'`=;&|(])(\.\.[\\/]?[^\s"'`|<>]*)/g

/** How many offending paths one refusal lists. */
const MAX_LISTED_PATHS = 5

/**
 * Where a command runs.
 *
 * `cwd` keeps its meaning under the workspace scope, but an allowlisted folder
 * is also a place a command may stand in; nothing else is.
 * @param workDir - relative paths resolve from here.
 * @param scopeRoot - writable root, or null for full access.
 * @param sandbox - allowed roots, or null when no guard is in force.
 * @param cwdArg - the directory as the model wrote it.
 * @returns the absolute working directory.
 */
export async function resolveCommandDir(
  workDir: string,
  scopeRoot: ToolScope,
  sandbox: Sandbox | null,
  cwdArg: string
): Promise<string> {
  let problem = ''
  try {
    return await resolveScoped(workDir, scopeRoot, cwdArg)
  } catch (error) {
    problem = error instanceof Error ? error.message : String(error)
  }
  for (const root of sandbox?.roots ?? []) {
    try {
      return await resolveScoped(root, root, cwdArg)
    } catch (error) {
      problem = error instanceof Error ? error.message : String(error)
    }
  }
  throw new Error(problem)
}

/**
 * Paths a command names that the sandbox does not cover.
 *
 * @param source - the command line or the snippet about to run.
 * @param cwd - where it will run, already resolved.
 * @param sandbox - allowed roots, or null to skip the check entirely.
 * @returns the offending paths, absolute, oldest mention first; empty is fine.
 */
export async function outsidePaths(
  source: string,
  cwd: string,
  sandbox: Sandbox | null
): Promise<string[]> {
  if (sandbox === null || sandbox.roots.length === 0) return []
  const roots = await Promise.all(sandbox.roots.map((root) => realish(path.resolve(root))))
  const outside: string[] = []
  for (const candidate of candidates(source)) {
    const target = await realish(path.resolve(cwd, expandHome(candidate)))
    if (roots.some((root) => !isOutside(root, target))) continue
    // The same folder may be named twice; one line is enough.
    if (outside.some((seen) => seen.toLowerCase() === target.toLowerCase())) continue
    outside.push(target)
  }
  return outside
}

/**
 * The refusal handed back to the model when a command leaves the sandbox.
 * @param outside - offending paths from {@link outsidePaths}.
 * @param sandbox - the roots that were allowed.
 * @returns a message that says what to change to get past it.
 */
export function sandboxRefusal(outside: string[], sandbox: Sandbox): string {
  const shown = outside
    .slice(0, MAX_LISTED_PATHS)
    .map((target) => `  ${target}`)
    .join('\n')
  const roots = sandbox.roots.map((root) => `  ${root}`).join('\n')
  const more =
    outside.length > MAX_LISTED_PATHS ? `\n…另有 ${outside.length - MAX_LISTED_PATHS} 个` : ''
  return [
    '沙箱拒绝了这条命令：它引用了允许范围之外的路径。',
    shown + more,
    '允许范围：',
    roots,
    '需要时把这些目录加进「设置 → 文件 → 命令允许的路径」，或把工具权限切到「完全权限」。'
  ].join('\n')
}

/** Every path-shaped word in one piece of command text. */
function candidates(source: string): string[] {
  const found: string[] = []
  const add = (match: RegExpMatchArray): void => {
    const text = (match[1] ?? match[0]).replace(/[,;:]+$/, '')
    // A lone separator is punctuation (`a / b`), not a place on disk.
    if (text === '' || text === '/' || text === '\\' || text === '.') return
    found.push(text)
  }
  for (const match of source.matchAll(DRIVE_PATH)) add(match)
  for (const match of source.matchAll(HOME_PATH)) add(match)
  for (const match of source.matchAll(PARENT_PATH)) add(match)
  if (process.platform === 'win32') {
    for (const match of source.matchAll(UNC_PATH)) add(match)
  } else {
    for (const match of source.matchAll(UNIX_PATH)) add(match)
  }
  return found
}

/** Expand a leading `~` the way a shell would, since the sandbox compares paths. */
export function expandHome(target: string): string {
  if (target !== '~' && !target.startsWith('~/') && !target.startsWith('~\\')) return target
  return path.join(os.homedir(), target.slice(1))
}
