/**
 * Workspace walking, ignore rules and the roots the search tools walk.
 *
 * Two things changed here once the tools were audited. Ignore rules used to be
 * read from the root `.gitignore` only, so a monorepo's per-package rules were
 * ignored and generated folders were searched; and a walk could descend into a
 * sibling conversation folder, because every conversation is a folder directly
 * under the same workspace. Both are handled here, in one place.
 */

import { promises as fs } from 'node:fs'
import path from 'node:path'
import { MAX_WALK_FILES } from './limits.ts'
import { absolutePath, referenceRoots, resolveScoped } from './paths.ts'
import type { ToolScope } from './types.ts'

/** Directories never listed by `list`, and never walked by the search tools. */
export const SKIPPED = new Set(['.git', 'node_modules', 'target', 'dist', '.dsh-flow'])

/**
 * Folders that hold generated output or installed packages.
 *
 * They are left out of walks only — `list` still shows them, because "is
 * `.venv` here" is exactly the question `env` and a person ask.
 */
const HEAVY = new Set([
  '.venv', 'venv', '__pycache__', '.next', '.nuxt', 'build', 'out', 'vendor',
  '.gradle', '.idea', '.cache', 'coverage', '.tox', '.mypy_cache', '.pytest_cache'
])

/** Whether the search tools leave a folder out, by name. */
export function skippedDir(name: string): boolean {
  return SKIPPED.has(name) || HEAVY.has(name)
}

/** One rule from a `.gitignore`: a path pattern plus whether it re-includes. */
export interface IgnoreRule {
  pattern: RegExp
  negate: boolean
}

/** One set of rules read from one folder; `base` is its path from the walk root. */
export interface IgnoreLayer {
  /** Folder the rules were read in, forward slashes; `''` for the root. */
  base: string
  rules: IgnoreRule[]
}

/**
 * The rules that apply to a walk, oldest first.
 *
 * `.gitignore` is the file everyone means, but `.ignore` covers the same ground
 * for tools that are not git, and `.git/info/exclude` is a repository-local
 * ignore list that never appears in a diff. All three are honoured, at the root
 * and in every folder the walk enters.
 * @param dir - absolute folder to read the files in.
 * @returns the rules, or an empty array when the folder has none.
 */
async function readIgnoreFiles(dir: string): Promise<IgnoreRule[]> {
  const rules: IgnoreRule[] = []
  for (const name of ['.gitignore', '.ignore']) {
    const text = await fs.readFile(path.join(dir, name), 'utf8').catch(() => null)
    if (text !== null) rules.push(...ignoreRules(text))
  }
  return rules
}

/**
 * The rules a walk starts with: the root's own files plus the repository's
 * `.git/info/exclude`.
 * @param root - the folder the walk starts in.
 * @returns one layer, or none when the root declares no rules.
 */
export async function baseIgnoreLayers(root: string): Promise<IgnoreLayer[]> {
  const rules = await readIgnoreFiles(root)
  const excluded = await fs
    .readFile(path.join(root, '.git', 'info', 'exclude'), 'utf8')
    .catch(() => null)
  if (excluded !== null) rules.push(...ignoreRules(excluded))
  return rules.length === 0 ? [] : [{ base: '', rules }]
}

/**
 * The rules for one folder below the walk root, after its parents'.
 * @param root - the folder the walk started in.
 * @param dir - the folder being entered, relative to `root`.
 * @param parent - rules inherited from above.
 * @returns the layers that apply inside `dir`.
 */
export async function deeperIgnoreLayers(
  root: string,
  dir: string,
  parent: IgnoreLayer[]
): Promise<IgnoreLayer[]> {
  const rules = await readIgnoreFiles(path.join(root, dir))
  return rules.length === 0 ? parent : [...parent, { base: dir, rules }]
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

/**
 * Whether a relative path is hidden by a stack of rules.
 *
 * A rule from a deeper `.gitignore` is matched against the path relative to that
 * folder, the way git does it, and the last match wins so a nested file can
 * re-include something its parent ignored.
 * @param layers - rules from the root down, in read order.
 * @param target - path relative to the walk root.
 * @returns true when the path should be left out.
 */
export function isIgnoredBy(layers: IgnoreLayer[], target: string): boolean {
  let ignored = false
  for (const layer of layers) {
    const local = localTo(layer.base, target)
    if (local === null) continue
    for (const rule of layer.rules) {
      if (rule.pattern.test(local)) ignored = !rule.negate
    }
  }
  return ignored
}

/** A path relative to `base`, or null when it is not under it. */
function localTo(base: string, target: string): string | null {
  if (base === '') return target
  if (target === base) return ''
  return target.startsWith(`${base}/`) ? target.slice(base.length + 1) : null
}

/** What one walk found, and whether it stopped before covering the folder. */
export interface WalkResult {
  files: string[]
  truncated: boolean
}

/** What a walk leaves out on top of the ignore rules. */
export interface WalkOptions {
  /** Top-level folder names to skip, such as sibling conversation folders. */
  skip?: readonly string[]
}

/**
 * Every file under one folder, as forward-slash relative paths.
 *
 * Symlinked *files* are followed (a linked source file is still source), while
 * symlinked folders are not — following those could walk in circles.
 * @param root - absolute folder to walk.
 * @param options - top-level folders to leave out.
 * @returns the files and whether the file cap stopped the walk.
 */
export async function walk(root: string, options: WalkOptions = {}): Promise<WalkResult> {
  const skip = new Set((options.skip ?? []).map((name) => name.toLowerCase()))
  const files: string[] = []
  const pending: Array<{ dir: string; layers: IgnoreLayer[] }> = [
    { dir: '.', layers: await baseIgnoreLayers(root) }
  ]
  let truncated = false
  while (pending.length > 0 && files.length < MAX_WALK_FILES) {
    const current = pending.shift()
    if (current === undefined) break
    const layers =
      current.dir === '.' ? current.layers : await deeperIgnoreLayers(root, current.dir, current.layers)
    let entries: Array<{
      name: string
      isDirectory: () => boolean
      isFile: () => boolean
      isSymbolicLink: () => boolean
    }>
    try {
      entries = await fs.readdir(path.join(root, current.dir), { withFileTypes: true })
    } catch {
      continue
    }
    for (const entry of entries) {
      const relative = current.dir === '.' ? entry.name : `${current.dir}/${entry.name}`
      if (entry.isDirectory()) {
        // A sibling conversation folder is only skipped at the top level: a
        // folder with the same name deeper down belongs to the project.
        if (current.dir === '.' && skip.has(entry.name.toLowerCase())) continue
        if (skippedDir(entry.name) || isIgnoredBy(layers, relative)) continue
        pending.push({ dir: relative, layers })
        continue
      }
      const isFile = entry.isFile()
        ? true
        : entry.isSymbolicLink() &&
          (await fs
            .stat(path.join(root, relative))
            .then((info) => info.isFile())
            .catch(() => false))
      if (!isFile || isIgnoredBy(layers, relative)) continue
      files.push(relative)
      if (files.length >= MAX_WALK_FILES) {
        truncated = true
        break
      }
    }
  }
  // Folders still queued when the cap hit were never looked at.
  return { files: files.sort(), truncated: truncated || pending.length > 0 }
}

/** One place a search starts: an absolute folder and how to label its hits. */
export interface SearchRoot {
  /** Absolute folder the walk starts in, already resolved. */
  root: string
  /** Folder the labels are relative to, and made absolute against. */
  labelBase: string
  /** Path from `labelBase` down to `root`, forward slashes; may be empty. */
  prefix: string
  /** True when labels have to be absolute paths. */
  absolute: boolean
}

/**
 * The folders one `glob` / `grep` / `replace` call searches.
 *
 * Without a `path` that is the workspace (or the conversation folder under full
 * access) plus every read-only reference root. With one, the same candidates are
 * tried with that folder appended, and the first that exists wins.
 * @param workDir - the conversation folder.
 * @param scopeRoot - writable root; null means the conversation folder.
 * @param requested - the call's `path`, empty for the whole workspace.
 * @param readOnlyRoots - engine and workflow folders that may be read.
 * @returns the places to walk, in order.
 */
export async function searchRoots(
  workDir: string,
  scopeRoot: ToolScope,
  requested: string,
  readOnlyRoots: string[]
): Promise<SearchRoot[]> {
  const base = scopeRoot === null ? workDir : path.resolve(scopeRoot)
  const absolute = base !== path.resolve(workDir)
  const wanted = requested.trim()
  const roots: SearchRoot[] = []
  if (wanted === '' || wanted === '.') {
    roots.push({ root: base, labelBase: base, prefix: '', absolute })
    // The reference folders are searched whole; their hits are named absolute
    // because they live outside the work folder.
    for (const reference of referenceRoots(base, readOnlyRoots)) {
      roots.push({ root: reference, labelBase: reference, prefix: '', absolute: true })
    }
    return roots
  }
  const consider = async (from: string, asAbsolute: boolean): Promise<void> => {
    try {
      // Full access means an absolute `path` is the caller's choice, the same way
      // `read` allows it; everything else is resolved inside the root it names.
      const target =
        scopeRoot === null && path.isAbsolute(wanted)
          ? path.resolve(wanted)
          : await resolveScoped(from, from, wanted)
      const info = await fs.stat(target).catch(() => null)
      if (!info?.isDirectory()) return
      roots.push({
        root: target,
        labelBase: from,
        prefix: path.relative(from, target).replace(/\\/g, '/'),
        absolute: asAbsolute
      })
    } catch {
      // Not under this root; the next candidate is tried.
    }
  }
  await consider(base, absolute)
  for (const reference of referenceRoots(base, readOnlyRoots)) await consider(reference, true)
  if (roots.length === 0) throw new Error(`找不到文件夹：${requested}`)
  return roots
}

/** How one hit is named: relative inside the work folder, else an absolute path. */
export function labelOf(place: SearchRoot, relative: string): string {
  return place.absolute ? absolutePath(place.labelBase, relative) : relative
}

/**
 * Whether a search place is the workspace itself.
 *
 * Only there does the sibling-conversation skip list apply: a `path` that names
 * a folder deeper down is an explicit choice by the model.
 * @param place - one root a search walks.
 * @param workDir - the conversation folder.
 * @param scopeRoot - writable root; null means the conversation folder.
 */
export function isTopPlace(place: SearchRoot, workDir: string, scopeRoot: ToolScope): boolean {
  const base = scopeRoot === null ? workDir : path.resolve(scopeRoot)
  return place.prefix === '' && path.resolve(place.root) === path.resolve(base)
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
