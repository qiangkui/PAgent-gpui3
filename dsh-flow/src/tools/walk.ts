/** Workspace walking and .gitignore rules used by search and tree. */

import { promises as fs } from 'node:fs'
import path from 'node:path'
import { MAX_WALK_FILES } from './limits.ts'

/** Directories never walked by `glob` / `grep`. */
export const SKIPPED = new Set(['.git', 'node_modules', 'target', 'dist', '.dsh-flow'])

/** One rule from a `.gitignore`: a path pattern plus whether it re-includes. */
export interface IgnoreRule {
  pattern: RegExp
  negate: boolean
}

/**
 * Read the project's `.gitignore` rules, if it has one.
 * @param root - the folder `glob` / `grep` are walking.
 * @returns the rules, oldest first; empty when there is no file.
 */
export async function ignoreRulesFor(root: string): Promise<IgnoreRule[]> {
  try {
    return ignoreRules(await fs.readFile(path.join(root, '.gitignore'), 'utf8'))
  } catch {
    return []
  }
}

/**
 * Parse `.gitignore` text.
 *
 * Supports comments, blank lines, `!` negation (last match wins), a leading or
 * inner `/` for anchoring, `*`, `?` and `**`. A pattern that matches a folder
 * also hides everything under it; a negated file inside an ignored folder is
 * not reached because the walk prunes that folder.
 * @param text - the file's contents.
 * @returns the compiled rules, in file order.
 */
export function ignoreRules(text: string): IgnoreRule[] {
  const rules: IgnoreRule[] = []
  for (const raw of text.split(/\r?\n/)) {
    // Trailing spaces are ignored unless escaped with a backslash.
    let line = raw.replace(/(^|[^\\])\s+$/, '$1')
    if (line === '' || line.startsWith('#')) continue
    let negate = false
    if (line.startsWith('!')) {
      negate = true
      line = line.slice(1)
    } else if (line.startsWith('\\!') || line.startsWith('\\#')) {
      line = line.slice(1)
    }
    if (line === '') continue
    if (line.endsWith('/')) line = line.slice(0, -1)
    const anchored = line.includes('/')
    if (line.startsWith('/')) line = line.slice(1)
    const source = ignoreSource(line)
    // A match may stop at a folder boundary: ignoring `build` also hides
    // `build/out.txt`.
    const body = anchored ? `^${source}` : `(?:^|/)${source}`
    rules.push({ pattern: new RegExp(`${body}(?:/|$)`, 'i'), negate })
  }
  return rules
}

/** The regular-expression body for one gitignore glob. */
function ignoreSource(pattern: string): string {
  let source = ''
  for (let index = 0; index < pattern.length; index += 1) {
    const char = pattern[index]
    if (char === '*') {
      if (pattern[index + 1] === '*') {
        index += 1
        if (pattern[index + 1] === '/') {
          index += 1
          source += '(?:.*/)?'
        } else {
          source += '.*'
        }
      } else {
        source += '[^/]*'
      }
    } else if (char === '?') {
      source += '[^/]'
    } else if (char === '\\' && index + 1 < pattern.length) {
      index += 1
      source += escapeExpression(pattern[index])
    } else {
      source += escapeExpression(char)
    }
  }
  return source
}

/** Whether a relative path is hidden by the rules. */
export function isIgnored(rules: IgnoreRule[], target: string): boolean {
  let ignored = false
  for (const rule of rules) {
    if (rule.pattern.test(target)) ignored = !rule.negate
  }
  return ignored
}

/** What one walk found, and whether it stopped before covering the folder. */
export interface WalkResult {
  files: string[]
  truncated: boolean
}

/** Every file under the working directory, as forward-slash relative paths. */
export async function walk(root: string): Promise<WalkResult> {
  const rules = await ignoreRulesFor(root)
  const files: string[] = []
  const pending = ['.']
  let truncated = false
  while (pending.length > 0 && files.length < MAX_WALK_FILES) {
    const current = pending.shift() ?? '.'
    let entries: Array<{ name: string; isDirectory: () => boolean; isFile: () => boolean }>
    try {
      entries = await fs.readdir(path.join(root, current), { withFileTypes: true })
    } catch {
      continue
    }
    for (const entry of entries) {
      const relative = current === '.' ? entry.name : `${current}/${entry.name}`
      if (entry.isDirectory()) {
        if (SKIPPED.has(entry.name) || isIgnored(rules, relative)) continue
        pending.push(relative)
      } else if (entry.isFile()) {
        if (isIgnored(rules, relative)) continue
        files.push(relative)
        if (files.length >= MAX_WALK_FILES) {
          truncated = true
          break
        }
      }
    }
  }
  // Folders still queued when the cap hit were never looked at.
  return { files: files.sort(), truncated: truncated || pending.length > 0 }
}

/** Escape one character for a regular expression. */
function escapeExpression(char: string): string {
  return char.replace(/[.+^${}()|[\]\\]/g, '\\$&')
}

/**
 * Turn a wildcard pattern into a regular expression.
 * `**` matches across directories, `*` and `?` stay inside one segment.
 * @param pattern - for example `src/**\/*.ts`.
 * @returns the anchored expression.
 */
export function wildcard(pattern: string): RegExp {
  const normalized = pattern.replace(/\\/g, '/').replace(/^\.\//, '')
  let source = ''
  for (let index = 0; index < normalized.length; index += 1) {
    const char = normalized[index]
    if (char === '*') {
      if (normalized[index + 1] === '*') {
        index += 1
        if (normalized[index + 1] === '/') index += 1
        source += '(?:.*/)?'
      } else {
        source += '[^/]*'
      }
    } else if (char === '?') {
      source += '[^/]'
    } else {
      source += char.replace(/[.+^${}()|[\]\\]/g, '\\$&')
    }
  }
  return new RegExp(`^${source}$`, 'i')
}
