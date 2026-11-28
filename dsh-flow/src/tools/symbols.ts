/** Repository symbol index: declarations found by walking the workspace once. */

import { promises as fs } from 'node:fs'
import path from 'node:path'
import type { ToolScope } from './types.ts'
import { MAX_RESULTS, MAX_WALK_FILES } from './limits.ts'
import { intArg } from './paths.ts'
import { walk } from './walk.ts'
import { isSensitivePath } from './secrets.ts'

/** Where the index is cached, relative to the indexed root. */
const CACHE_DIR = '.dsh-flow'
const CACHE_FILE = 'index.json'
/** Bump when the extraction rules or the shape below change. */
const CACHE_VERSION = 4
/** How long a cache is trusted before it is rebuilt. */
const CACHE_TTL_MS = 10 * 60_000
/** Files larger than this are skipped: declarations in them are rare. */
const MAX_INDEX_FILE_BYTES = 512 * 1024
/** Ceiling for one index, so a huge workspace cannot grow without bound. */
const MAX_SYMBOLS = 20_000
/** Characters of one source line kept in a reference hit. */
const MAX_HIT_LINE = 200
/** A reference name has to look like an identifier; anything else is a grep. */
const IDENTIFIER = /^[A-Za-z_$][A-Za-z0-9_$]*$/

/** One declaration found in one file. */
interface SymbolEntry {
  /** Forward-slash path relative to the indexed root. */
  file: string
  /** 1-based line number. */
  line: number
  /** `fn`, `struct`, `class`, `interface`, `type`, … */
  kind: string
  name: string
}

/** What is stored in `.dsh-flow/index.json`. */
interface IndexCache {
  version: number
  builtMs: number
  root: string
  files: number
  truncated: boolean
  /** Top-level folders the walk left out, so a different set rebuilds. */
  skip: string[]
  /** Every indexed file, so reference search scans exactly the same set. */
  paths: string[]
  symbols: SymbolEntry[]
}

/** Languages the index understands, by extension. */
const LANGUAGES = new Map<string, 'rust' | 'ts' | 'py' | 'go'>([
  ['.rs', 'rust'],
  ['.ts', 'ts'],
  ['.tsx', 'ts'],
  ['.js', 'ts'],
  ['.jsx', 'ts'],
  ['.mjs', 'ts'],
  ['.cjs', 'ts'],
  ['.py', 'py'],
  ['.go', 'go']
])

/**
 * `symbols`: search declarations, or every whole-word use of one name.
 *
 * The first call walks the workspace, extracts declarations and caches them in
 * `.dsh-flow/index.json`; later calls reuse that file for ten minutes. A cache
 * that cannot be written (read-only workspace) only costs a rebuild next time.
 * @param workDir - the conversation folder.
 * @param scopeRoot - writable/readable root; null means the conversation only.
 * @param args - query, path, refresh, limit and find.
 * @returns matching declarations or references, or a summary and a hint.
 */
export async function indexSymbols(
  workDir: string,
  scopeRoot: ToolScope,
  args: Record<string, unknown>,
  skip: readonly string[] = []
): Promise<string> {
  const root = scopeRoot === null ? workDir : path.resolve(scopeRoot)
  const query = typeof args.query === 'string' ? args.query.trim() : ''
  if (args.path !== undefined && typeof args.path !== 'string') {
    throw new Error('参数 path 不是字符串')
  }
  const prefix = (typeof args.path === 'string' ? args.path : '')
    .trim()
    .replace(/\\/g, '/')
    .replace(/^\.\//, '')
    .replace(/\/+$/, '')
  if (args.refresh !== undefined && typeof args.refresh !== 'boolean') {
    throw new Error('参数 refresh 不是布尔值')
  }
  if (args.find !== undefined && args.find !== 'references') {
    throw new Error('参数 find 只能是 references')
  }
  const references = args.find === 'references'
  if (references && !IDENTIFIER.test(query)) {
    throw new Error('find: references 需要一个标识符作为 query')
  }
  const refresh = args.refresh === true
  const limit = Math.min(Math.max(intArg(args, 'limit') ?? MAX_RESULTS, 1), MAX_RESULTS)

  const loaded = refresh ? null : await loadCache(root, skip)
  const cache = loaded ?? (await buildIndex(root, skip))
  // A read-only sandbox cannot write the cache; the index still works for this
  // call, and the next one rebuilds instead of breaking the tool.
  if (loaded === null && process.env.DSH_SANDBOX_READONLY !== '1') {
    await saveCache(root, cache)
  }

  if (references) {
    return findReferences(root, cache, query, prefix, limit)
  }

  const symbols = cache.symbols.filter((entry) => {
    if (prefix !== '' && entry.file !== prefix && !entry.file.startsWith(`${prefix}/`)) {
      return false
    }
    return query === '' || entry.name.toLowerCase().includes(query.toLowerCase())
  })
  const summary = indexSummary(cache)

  if (query === '' && prefix === '') {
    return `${summary}。用 query 按名字查找，path 限定目录，refresh: true 重建。`
  }
  if (symbols.length === 0) {
    return `没有匹配 ${query || prefix} 的符号（${summary}）。`
  }
  const shown = symbols
    .slice(0, limit)
    .map((entry) => `${entry.file}:${entry.line}: ${entry.kind} ${entry.name}`)
  const hidden = symbols.length - shown.length
  return `${summary}\n${shown.join('\n')}${
    hidden > 0 ? `\n…（还有 ${hidden} 个未显示，用 query / path 缩小范围）` : ''
  }`
}

/** One line describing what the index covers. */
function indexSummary(cache: IndexCache): string {
  return `索引 ${cache.files} 个文件 / ${cache.symbols.length} 个符号${
    cache.truncated ? `（只扫描了前 ${MAX_WALK_FILES} 个文件）` : ''
  }`
}

/**
 * Find every whole-word use of one identifier in the indexed files.
 *
 * Declarations and other uses are listed apart so the reader sees the
 * definition first; this is still a text search, not type resolution.
 * @param root - the indexed root.
 * @param cache - the current index.
 * @param name - an identifier.
 * @param prefix - optional folder filter, already normalised.
 * @param limit - maximum lines to print.
 * @returns grouped hits, or a "nothing found" line.
 */
async function findReferences(
  root: string,
  cache: IndexCache,
  name: string,
  prefix: string,
  limit: number
): Promise<string> {
  const summary = indexSummary(cache)
  const word = new RegExp(`(?<![A-Za-z0-9_$])${escapeRegExp(name)}(?![A-Za-z0-9_$])`)
  const declarations = new Set(
    cache.symbols
      .filter((entry) => entry.name === name)
      .map((entry) => `${entry.file}:${entry.line}`)
  )
  const decl: string[] = []
  const uses: string[] = []
  let scanned = 0
  // Showing at most `limit` lines means there is no point reading much more.
  const budget = limit * 4
  let total = 0
  for (const relative of cache.paths) {
    if (total >= budget) break
    if (prefix !== '' && relative !== prefix && !relative.startsWith(`${prefix}/`)) continue
    // A reference hit on a key file would print the secret just like `read`.
    if (isSensitivePath(relative)) continue
    const text = await fs.readFile(path.join(root, relative), 'utf8').catch(() => null)
    if (text === null || text.includes('\u0000')) continue
    scanned += 1
    const lines = text.split('\n')
    for (let index = 0; index < lines.length && total < budget; index += 1) {
      const line = lines[index] ?? ''
      if (!word.test(line)) continue
      const hit = `${relative}:${index + 1}: ${squash(line)}`
      if (declarations.has(`${relative}:${index + 1}`)) decl.push(hit)
      else uses.push(hit)
      total += 1
    }
  }
  if (total === 0) {
    return `没有找到 ${name} 的声明或引用（扫描 ${scanned} 个文件；${summary}）。`
  }
  const shown = [...decl.slice(0, limit), ...uses.slice(0, Math.max(limit - decl.length, 0))]
  const hidden = total - shown.length
  const parts: string[] = [summary]
  if (decl.length > 0) {
    parts.push(`声明（${decl.length}）：`, ...decl.slice(0, limit))
  }
  const otherShown = shown.length - Math.min(decl.length, limit)
  if (otherShown > 0) {
    parts.push(`引用（${uses.length}）：`, ...uses.slice(0, otherShown))
  }
  if (hidden > 0) {
    parts.push(`…（还有 ${hidden} 处未显示，用 path 缩小范围或加 limit）`)
  }
  return parts.join('\n')
}

/** Collapse one source line into something that fits on one output line. */
function squash(line: string): string {
  const flat = line.trim().replace(/\s+/g, ' ')
  return flat.length > MAX_HIT_LINE ? `${flat.slice(0, MAX_HIT_LINE)}…` : flat
}

/** Escape a literal string for use inside a regular expression. */
function escapeRegExp(text: string): string {
  return text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
}


/** Read the cache when it is current for this root. */
async function loadCache(root: string, skip: readonly string[] = []): Promise<IndexCache | null> {
  try {
    const file = path.join(root, CACHE_DIR, CACHE_FILE)
    const raw = await fs.readFile(file, 'utf8')
    const parsed = JSON.parse(raw) as IndexCache
    if (parsed.version !== CACHE_VERSION || parsed.root !== root) return null
    if (!Array.isArray(parsed.symbols) || !Array.isArray(parsed.paths)) return null
    // A different sibling-folder list means a different set of files: the cached
    // index would list another conversation's sources, so rebuild it.
    const kept = Array.isArray(parsed.skip) ? parsed.skip : []
    if (kept.length !== skip.length || kept.some((name, index) => name !== skip[index])) {
      return null
    }
    if (Date.now() - parsed.builtMs > CACHE_TTL_MS) return null
    return parsed
  } catch {
    return null
  }
}

/** Walk the root and extract every declaration the supported languages declare. */
async function buildIndex(root: string, skip: readonly string[] = []): Promise<IndexCache> {
  const found = await walk(root, { skip })
  const symbols: SymbolEntry[] = []
  const paths: string[] = []
  let files = 0
  for (const relative of found.files) {
    if (symbols.length >= MAX_SYMBOLS) break
    // Credentials are not source: indexing them would only risk echoing a value.
    if (isSensitivePath(relative)) continue
    const language = LANGUAGES.get(path.extname(relative).toLowerCase())
    if (language === undefined) continue
    const target = path.join(root, relative)
    const info = await fs.stat(target).catch(() => null)
    if (info === null || info.size > MAX_INDEX_FILE_BYTES) continue
    const text = await fs.readFile(target, 'utf8').catch(() => null)
    if (text === null || text.includes('\u0000')) continue
    files += 1
    paths.push(relative)
    symbols.push(...symbolsIn(relative, text, language))
  }
  return {
    version: CACHE_VERSION,
    builtMs: Date.now(),
    root,
    files,
    truncated: found.truncated,
    skip: [...skip],
    paths,
    symbols: symbols.slice(0, MAX_SYMBOLS)
  }
}

/**
 * Forget a cached index after a file the index covers has changed.
 *
 * The index is trusted for ten minutes, so without this a model that edits a
 * file and asks `symbols` again would be handed the line numbers from before its
 * own edit. Best effort: a read-only workspace simply rebuilds next time.
 * @param root - the folder the index belongs to.
 */
export async function invalidateIndex(root: string): Promise<void> {
  await fs.unlink(path.join(root, CACHE_DIR, CACHE_FILE)).catch(() => undefined)
}

/** Write the cache atomically; a read-only workspace just skips it. */
async function saveCache(root: string, cache: IndexCache): Promise<void> {
  try {
    const dir = path.join(root, CACHE_DIR)
    await fs.mkdir(dir, { recursive: true })
    const file = path.join(dir, CACHE_FILE)
    // A unique temporary name: two questions can build the index at the same
    // time, and a shared `.tmp` would let one rename the other's half-written
    // file into place.
    const tmp = `${file}.${process.pid}.${Math.random().toString(36).slice(2)}.tmp`
    await fs.writeFile(tmp, JSON.stringify(cache), 'utf8')
    await fs.rename(tmp, file)
  } catch {
    // The index still works in memory; the next process will rebuild it.
  }
}

/** Extract declarations from one file, line by line. */
function symbolsIn(
  file: string,
  text: string,
  language: 'rust' | 'ts' | 'py' | 'go'
): SymbolEntry[] {
  const entries: SymbolEntry[] = []
  const lines = text.split('\n')
  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index] ?? ''
    // Minified or generated lines are not declarations worth indexing.
    if (line.length > 500) continue
    const hit = matchLine(language, line)
    if (hit !== null) {
      entries.push({ file, line: index + 1, kind: hit.kind, name: hit.name })
    }
  }
  return entries
}

/** One declaration name and kind, if this line starts one. */
function matchLine(
  language: 'rust' | 'ts' | 'py' | 'go',
  line: string
): { kind: string; name: string } | null {
  if (language === 'rust') {
    const hit =
      /^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(fn|struct|enum|trait|impl|mod|const|static|type)\s+([A-Za-z_][A-Za-z0-9_]*)/.exec(
        line
      )
    return hit === null ? null : { kind: hit[1] ?? '', name: hit[2] ?? '' }
  }
  if (language === 'ts') {
    const hit =
      /^\s*(?:export\s+)?(?:default\s+)?(?:declare\s+)?(?:async\s+)?(function|class|interface|type|enum|const|let|var)\s+([A-Za-z_$][A-Za-z0-9_$]*)/.exec(
        line
      )
    return hit === null ? null : { kind: hit[1] ?? '', name: hit[2] ?? '' }
  }
  if (language === 'py') {
    const hit = /^\s*(?:async\s+)?(def|class)\s+([A-Za-z_][A-Za-z0-9_]*)/.exec(line)
    if (hit === null) return null
    return { kind: hit[1] === 'def' ? 'fn' : 'class', name: hit[2] ?? '' }
  }
  const fn = /^\s*func\s+(?:\([^)]*\)\s*)?([A-Za-z_][A-Za-z0-9_]*)/.exec(line)
  if (fn !== null) return { kind: 'fn', name: fn[1] ?? '' }
  const type = /^\s*type\s+([A-Za-z_][A-Za-z0-9_]*)/.exec(line)
  return type === null ? null : { kind: 'type', name: type[1] ?? '' }
}
