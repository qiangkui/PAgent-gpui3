/** Search tools: glob and grep over the workspace. */

import { promises as fs } from 'node:fs'
import path from 'node:path'
import type { ToolScope } from './types.ts'
import {
  MAX_GREP_BYTES, MAX_GREP_CHARS, MAX_GREP_CONTEXT, MAX_GREP_PER_FILE, MAX_RESULTS,
  MAX_WALK_FILES
} from './limits.ts'
import { decodeOutput, safeEnd } from './text.ts'
import { looksLikePdf } from './pdf.ts'
import { intArg, textArg } from './paths.ts'
import type { SearchRoot } from './walk.ts'
import { isTopPlace, labelOf, searchRoots, walk, wildcard } from './walk.ts'
import { isSensitivePath } from './secrets.ts'

/** `glob`: relative paths matching a wildcard pattern. */
export async function globFiles(
  workDir: string,
  scopeRoot: ToolScope,
  args: Record<string, unknown>,
  readOnlyRoots: string[],
  skip: readonly string[] = []
): Promise<string> {
  const pattern = textArg(args, 'pattern')
  if (args.path !== undefined && typeof args.path !== 'string') {
    throw new Error('参数 path 不是字符串')
  }
  const matcher = wildcard(pattern)
  // `path` narrows the walk to one folder, so a big workspace does not have to
  // be scanned (and listed, and truncated) to find one package.
  const places = await searchRoots(workDir, scopeRoot, args.path ?? '', readOnlyRoots)
  const found: string[] = []
  let truncated = false
  for (const place of places) {
    const own = await walk(place.root, {
      skip: isTopPlace(place, workDir, scopeRoot) ? skip : []
    })
    truncated = truncated || own.truncated
    for (const file of own.files) {
      // The pattern is matched against the path from the work folder, so
      // `src/**/*.ts` still means the same thing when `path` is given.
      const relative = place.prefix === '' ? file : `${place.prefix}/${file}`
      if (!matcher.test(relative)) continue
      found.push(labelOf(place, relative))
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
  readOnlyRoots: string[],
  skip: readonly string[] = []
): Promise<string> {
  const pattern = textArg(args, 'pattern')
  if (args.path !== undefined && typeof args.path !== 'string') {
    throw new Error('参数 path 不是字符串')
  }
  const filter = typeof args.glob === 'string' && args.glob ? wildcard(args.glob) : null
  const context = Math.min(Math.max(intArg(args, 'context') ?? 0, 0), MAX_GREP_CONTEXT)
  // Per-file cap: a generated file with hundreds of hits would otherwise take the
  // whole global budget and hide every other file that matched.
  const perFile = Math.min(
    Math.max(intArg(args, 'perFile') ?? MAX_GREP_PER_FILE, 1),
    MAX_RESULTS
  )
  let expression: RegExp
  try {
    expression = new RegExp(pattern, args.ignoreCase === true ? 'i' : '')
  } catch {
    throw new Error(`正则写错了：${pattern}`)
  }
  const places = await searchRoots(workDir, scopeRoot, args.path ?? '', readOnlyRoots)
  const files: Array<{ label: string; target: string }> = []
  let truncated = false
  for (const place of places) {
    const own = await walk(place.root, {
      skip: isTopPlace(place, workDir, scopeRoot) ? skip : []
    })
    truncated = truncated || own.truncated
    for (const file of own.files) {
      const relative = place.prefix === '' ? file : `${place.prefix}/${file}`
      if (filter && !filter.test(relative)) continue
      files.push({ label: labelOf(place, relative), target: path.join(place.root, file) })
    }
  }
  const hits: string[] = []
  let scanned = 0
  let stopped = false
  let skippedSecrets = 0
  let clippedFiles = 0
  for (const entry of files) {
    // Credentials are not searchable: a matching line would put the key into
    // the transcript exactly the way `read` would.
    if (isSensitivePath(entry.label)) {
      skippedSecrets += 1
      continue
    }
    const target = entry.target
    // A cap on hits alone still reads every file it walks; the size budget
    // stops a stray `grep .` in a big workspace from scanning for minutes.
    const size = await fs.stat(target).then((info) => info.size).catch(() => 0)
    if (scanned + size > MAX_GREP_BYTES) {
      stopped = true
      break
    }
    scanned += size
    const bytes = await fs.readFile(target).catch(() => null)
    if (bytes === null) continue
    // A NUL byte means binary, and a PDF is binary whatever its first page looks
    // like; checking the bytes avoids decoding a whole binary file just to find
    // out it is not text. A PDF whose header sits past the search window is
    // still caught when its streams contain a NUL.
    if (bytes.subarray(0, 8_192).includes(0) || looksLikePdf(bytes)) continue
    // GBK sources (Chinese Windows) are read line by line, so a file `read`
    // shows as text is searchable too instead of matching nothing.
    const text = decodeOutput(bytes)
    const lines = text.split('\n')
    const matched: number[] = []
    for (let index = 0; index < lines.length; index += 1) {
      if (expression.test(lines[index])) matched.push(index)
    }
    if (matched.length === 0) continue
    // Context windows of neighbouring hits overlap; one row per line is enough.
    const shown = new Set<number>()
    let stoppedAtLimit = false
    let fileLines = 0
    let clipped = false
    for (const hit of matched) {
      const from = Math.max(0, hit - context)
      const to = Math.min(lines.length - 1, hit + context)
      for (let line = from; line <= to; line += 1) {
        if (shown.has(line)) continue
        if (fileLines >= perFile) {
          clipped = true
          break
        }
        shown.add(line)
        fileLines += 1
        const mark = line === hit ? ': ' : '- '
        hits.push(`${entry.label}:${line + 1}${mark}${lines[line].trim()}`)
        if (hits.length >= MAX_RESULTS) {
          stoppedAtLimit = true
          break
        }
      }
      if (stoppedAtLimit || clipped) break
    }
    if (clipped) clippedFiles += 1
    if (stoppedAtLimit) {
      return `${clipGrep(hits)}\n…（已截断：只显示前 ${MAX_RESULTS} 行，含上下文）`
    }
  }
  const notes: string[] = []
  if (clippedFiles > 0) {
    notes.push(`单个文件最多列 ${perFile} 行，有 ${clippedFiles} 个文件被截断（可调大 perFile）`)
  }
  if (skippedSecrets > 0) notes.push(`已跳过 ${skippedSecrets} 个凭据/密钥类文件`)
  if (stopped) {
    notes.push(`已读满 ${MAX_GREP_BYTES / (1024 * 1024)} MB，剩余文件没有搜索`)
  }
  if (truncated) notes.push(`只扫描了前 ${MAX_WALK_FILES} 个文件`)
  const suffix = notes.length > 0 ? notes.join('；') : ''
  if (hits.length === 0) {
    return `没有匹配 ${pattern} 的内容${suffix ? `（${suffix}）` : ''}`
  }
  return `${clipGrep(hits)}${suffix ? `\n…（${suffix}，用 path / glob 缩小范围后重试）` : ''}`
}

/** Join hits, keeping a few huge matching lines from flooding the reply. */
function clipGrep(hits: string[]): string {
  const text = hits.join('\n')
  return text.length <= MAX_GREP_CHARS
    ? text
    : `${text.slice(0, safeEnd(text, MAX_GREP_CHARS))}\n…（输出过长，已截断）`
}
