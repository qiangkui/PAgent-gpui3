/** Path resolution and argument reading shared by the tools. */

import { promises as fs } from 'node:fs'
import path from 'node:path'
import type { ToolScope } from './types.ts'

/**
 * Resolve a path from the work directory, refusing to leave the writable scope.
 * Symlinks are followed first, so a link inside the scope cannot point out.
 * @param workDir - relative paths resolve from here.
 * @param scopeRoot - writable root; null means the whole computer.
 * @param target - the path the model asked for.
 * @returns the absolute path.
 */
export async function resolveScoped(
  workDir: string,
  scopeRoot: ToolScope,
  target: string
): Promise<string> {
  const resolved = path.resolve(workDir, target)
  if (scopeRoot === null) {
    // Full access: absolute paths and `..` are the caller's choice.
    return resolved
  }
  const realRoot = await realish(path.resolve(scopeRoot))
  const real = await realish(resolved)
  if (isOutside(realRoot, real)) {
    throw new Error(`路径超出工作区：${target}`)
  }
  return real
}

/**
 * Resolve symlinks in the part of `target` that exists.
 *
 * A file that is about to be created has no `realpath` yet, but its parent may
 * already be a link; following the nearest existing ancestor closes that hole.
 * @param target - an absolute path.
 * @returns the path with every resolvable link followed.
 */
export async function realish(target: string): Promise<string> {
  const tail: string[] = []
  let current = target
  for (;;) {
    const real = await fs.realpath(current).catch(() => null)
    if (real) return path.join(real, ...tail)
    const parent = path.dirname(current)
    if (parent === current) return target
    tail.unshift(path.basename(current))
    current = parent
  }
}

/** Whether `candidate` is outside `base` (same path counts as inside). */
export function isOutside(base: string, candidate: string): boolean {
  const relative = path.relative(base, candidate)
  return (
    relative === '..' || relative.startsWith(`..${path.sep}`) || path.isAbsolute(relative)
  )
}

/**
 * Resolve a path for reading: the conversation folder first, then each
 * read-only reference root.
 * @param root - the writable conversation directory.
 * @param target - the path the model asked for.
 * @param readOnlyRoots - engine and workflow folders the assistant may read.
 * @returns the absolute path of the file.
 */
export async function resolveReadable(
  workDir: string,
  scopeRoot: ToolScope,
  target: string,
  readOnlyRoots: string[]
): Promise<string> {
  try {
    return await resolveScoped(workDir, scopeRoot, target)
  } catch (error) {
    if (scopeRoot === null) throw error
    let problem = error instanceof Error ? error.message : String(error)
    for (const base of readOnlyRoots) {
      try {
        return await resolveScoped(base, base, target)
      } catch (inner) {
        problem = inner instanceof Error ? inner.message : String(inner)
      }
    }
    throw new Error(problem)
  }
}

/**
 * The reference roots worth walking: a folder already inside the writable one
 * would only repeat what `walk(root)` already found.
 * @param root - the writable conversation directory.
 * @param readOnlyRoots - folders the assistant may read.
 * @returns absolute roots not covered by `root`.
 */
export function referenceRoots(root: string, readOnlyRoots: string[]): string[] {
  const writable = path.resolve(root)
  const roots: string[] = []
  for (const dir of readOnlyRoots.map((entry) => path.resolve(entry))) {
    if (!isOutside(writable, dir)) continue
    // The window passes the engine and workflow folders, which can be the same
    // folder (or nested); walking both would report every hit twice.
    if (roots.some((kept) => !isOutside(kept, dir))) continue
    roots.push(dir)
  }
  return roots
}

/** An absolute path the model can hand back to `read`, with forward slashes. */
export function absolutePath(base: string, relative: string): string {
  return path.join(base, relative).replace(/\\/g, '/')
}

/** A required string argument. */
export function textArg(args: Record<string, unknown>, name: string): string {
  const value = args[name]
  if (typeof value !== 'string') throw new Error(`缺少参数 ${name}`)
  return value
}

/** An optional integer argument. */
export function intArg(args: Record<string, unknown>, name: string): number | undefined {
  const value = args[name]
  if (value === undefined || value === null) return undefined
  const parsed = Number(value)
  if (!Number.isFinite(parsed)) throw new Error(`参数 ${name} 不是数字`)
  return Math.trunc(parsed)
}
