/** Search tools: glob and grep over the workspace. */

import { promises as fs } from 'node:fs'
import path from 'node:path'
import type { ToolScope } from './types.ts'
import {
  MAX_GREP_BYTES, MAX_GREP_CHARS, MAX_GREP_CONTEXT, MAX_RESULTS, MAX_WALK_FILES
} from './limits.ts'
import { safeEnd } from './text.ts'
import { absolutePath, intArg, referenceRoots, textArg } from './paths.ts'
import { walk, wildcard } from './walk.ts'

/** `glob`: relative paths matching a wildcard pattern. */
export async function globFiles(
  workDir: string,
  scopeRoot: ToolScope,
  args: Record<string, unknown>,
  readOnlyRoots: string[]
): Promise<string> {
  const pattern = textArg(args, 'pattern')
  const matcher = wildcard(pattern)
  // Workspace scope searches the whole workspace and names hits absolutely, so
  // the model can read them from the conversation folder; full access searches
  // the work directory.
  const base = scopeRoot === null ? workDir : path.resolve(scopeRoot)
  // Same folder means the old relative listing; a wider workspace needs
  // absolute names so `read` can reach them from the conversation folder.
  const absolute = base !== path.resolve(workDir)
  const own = await walk(base)
  const found = own.files
    .filter((file) => matcher.test(file))
    .map((file) => (absolute ? absolutePath(base, file) : file))
  let truncated = own.truncated
  for (const reference of referenceRoots(base, readOnlyRoots)) {
    const extra = await walk(reference)
    truncated = truncated || extra.truncated
    for (const file of extra.files) {
      if (matcher.test(file)) found.push(absolutePath(reference, file))
    }
  }
  const shown = found.slice(0, MAX_RESULTS)
  if (shown.length === 0) {
    return truncated
      ? `没有匹配 ${pattern} 的文件（只扫描了前 ${MAX_WALK_FILES} 个文件，可能还有）`
      : `没有匹配 ${pattern} 的文件`
  }
  // Listing only the first page without saying so reads as "this is all of
  // them"; name the remainder so the model narrows the pattern instead.
  const notes: string[] = []
  const hidden = found.length - shown.length
  if (hidden > 0) notes.push(`还有 ${hidden} 个未显示`)
  if (truncated) notes.push(`只扫描了前 ${MAX_WALK_FILES} 个文件`)
  return shown.join('\n') + (notes.length > 0 ? `\n…（${notes.join('；')}，用更具体的 pattern 缩小范围）` : '')
}

/** `grep`: `path:line: text` for each hit, capped. */
export async function grepFiles(
  workDir: string,
  scopeRoot: ToolScope,
  args: Record<string, unknown>,
  readOnlyRoots: string[]
): Promise<string> {
  const pattern = textArg(args, 'pattern')
  const filter = typeof args.glob === 'string' && args.glob ? wildcard(args.glob) : null
  const context = Math.min(Math.max(intArg(args, 'context') ?? 0, 0), MAX_GREP_CONTEXT)
  let expression: RegExp
  try {
    expression = new RegExp(pattern, args.ignoreCase === true ? 'i' : '')
  } catch {
    throw new Error(`正则写错了：${pattern}`)
  }
  const base = scopeRoot === null ? workDir : path.resolve(scopeRoot)
  const absolute = base !== path.resolve(workDir)
  const own = await walk(base)
  const files = own.files.map((file) => ({
    base,
    file,
    label: absolute ? absolutePath(base, file) : file
  }))
  let truncated = own.truncated
  for (const reference of referenceRoots(base, readOnlyRoots)) {
    const extra = await walk(reference)
    truncated = truncated || extra.truncated
    for (const file of extra.files) {
      files.push({ base: reference, file, label: absolutePath(reference, file) })
    }
  }
  const hits: string[] = []
  let scanned = 0
  let stopped = false
  for (const entry of files) {
    if (filter && !filter.test(entry.file)) continue
    const target = path.join(entry.base, entry.file)
    // A cap on hits alone still reads every file it walks; the size budget
    // stops a stray `grep .` in a big workspace from scanning for minutes.
    const size = await fs.stat(target).then((info) => info.size).catch(() => 0)
    if (scanned + size > MAX_GREP_BYTES) {
      stopped = true
      break
    }
    scanned += size
    let text: string
    try {
      text = await fs.readFile(target, 'utf8')
    } catch {
      continue
    }
    if (text.includes('\u0000')) continue
    const lines = text.split('\n')
    const matched: number[] = []
    for (let index = 0; index < lines.length; index += 1) {
      if (expression.test(lines[index])) matched.push(index)
    }
    if (matched.length === 0) continue
    // Context windows of neighbouring hits overlap; one row per line is enough.
    const shown = new Set<number>()
    let stoppedAtLimit = false
    for (const hit of matched) {
      const from = Math.max(0, hit - context)
      const to = Math.min(lines.length - 1, hit + context)
      for (let line = from; line <= to; line += 1) {
        if (shown.has(line)) continue
        shown.add(line)
        const mark = line === hit ? ': ' : '- '
        hits.push(`${entry.label}:${line + 1}${mark}${lines[line].trim()}`)
        if (hits.length >= MAX_RESULTS) {
          stoppedAtLimit = true
          break
        }
      }
      if (stoppedAtLimit) break
    }
    if (stoppedAtLimit) {
      return `${clipGrep(hits)}\n…（已截断：只显示前 ${MAX_RESULTS} 行，含上下文）`
    }
  }
  const notes: string[] = []
  if (stopped) {
    notes.push(`已读满 ${MAX_GREP_BYTES / (1024 * 1024)} MB，剩余文件没有搜索`)
  }
  if (truncated) notes.push(`只扫描了前 ${MAX_WALK_FILES} 个文件`)
  const suffix = notes.length > 0 ? notes.join('；') : ''
  if (hits.length === 0) {
    return `没有匹配 ${pattern} 的内容${suffix ? `（${suffix}）` : ''}`
  }
  return `${clipGrep(hits)}${suffix ? `\n…（${suffix}，用 glob 缩小范围后重试）` : ''}`
}

/** Join hits, keeping a few huge matching lines from flooding the reply. */
function clipGrep(hits: string[]): string {
  const text = hits.join('\n')
  return text.length <= MAX_GREP_CHARS
    ? text
    : `${text.slice(0, safeEnd(text, MAX_GREP_CHARS))}\n…（输出过长，已截断）`
}
