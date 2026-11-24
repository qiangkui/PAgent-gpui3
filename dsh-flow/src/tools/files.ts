/** File tools: read, write, edit, move, delete, list, tree and previews. */

import { promises as fs } from 'node:fs'
import path from 'node:path'
import type { ToolCall, ToolPreview, ToolScope } from './types.ts'
import {
  MAX_EDIT_BYTES, MAX_READ_BYTES, MAX_READ_CHARS, MAX_READ_LINES, MAX_RESULTS,
  MAX_TREE_DEPTH, MAX_TREE_ENTRIES, MAX_WRITE_CHARS
} from './limits.ts'
import { decodeTextInfo, safeEnd } from './text.ts'
import {
  absolutePath, intArg, referenceRoots, resolveReadable, resolveScoped, textArg
} from './paths.ts'
import { SKIPPED, ignoreRulesFor, isIgnored, walk, wildcard } from './walk.ts'

/**
 * Describe what a write or edit would change, without touching the file.
 *
 * Returns `null` when the call is not a write, the path is refused, the file
 * cannot be read, or the edit would not apply uniquely: the tool itself then
 * reports the same problem without an approval prompt.
 * @param root - the writable conversation directory.
 * @param call - name plus parsed arguments.
 * @returns the diff, or `null` when there is nothing to preview.
 */
export async function previewTool(
  root: string,
  call: ToolCall
): Promise<ToolPreview | null> {
  return await previewToolScoped(root, root, call)
}

/**
 * Preview a write with the same work directory and writable scope as the tool.
 * @param workDir - relative paths resolve from here.
 * @param scopeRoot - writable root; null means the whole computer.
 * @param call - name plus parsed arguments.
 * @returns the preview, or null when there is nothing to show.
 */
export async function previewToolScoped(
  workDir: string,
  scopeRoot: ToolScope,
  call: ToolCall
): Promise<ToolPreview | null> {
  try {
    if (call.name === 'write') {
      const target = await resolveScoped(workDir, scopeRoot, textArg(call.args, 'path'))
      const content = textArg(call.args, 'content')
      if (content.length > MAX_WRITE_CHARS) return null
      let before = ''
      let existed = true
      try {
        before = await fs.readFile(target, 'utf8')
      } catch {
        existed = false
      }
      return preview(before, content, existed)
    }
    if (call.name === 'edit') {
      const pathArg = textArg(call.args, 'path')
      const target = await resolveScoped(workDir, scopeRoot, pathArg)
      // A file the tool will refuse is not worth a confirmation card.
      const info = await fs.stat(target)
      if (info.size > MAX_EDIT_BYTES) return null
      const before = await fs.readFile(target, 'utf8')
      assertText(before, pathArg)
      const after = applyEdits(before, editPairs(call.args))
      return preview(before, after, true)
    }
    if (call.name === 'replace') {
      return await previewReplace(workDir, scopeRoot, call.args)
    }
  } catch {
    return null
  }
  return null
}

/** Longest before + after text kept for an undo. */
const UNDO_MAX_CHARS = 200_000

/**
 * The diff plus the two versions, when they are small enough to keep.
 * @param before - the file now.
 * @param after - the file the tool would write.
 * @param existed - false when the call creates the file.
 * @returns the preview, or null when nothing changes.
 */
function preview(before: string, after: string, existed: boolean): ToolPreview | null {
  const diff = diffLines(stripBom(before), after)
  if (!diff) return null
  const undoable = before.length + after.length <= UNDO_MAX_CHARS
  return {
    diff,
    before: undoable ? before : undefined,
    after: undoable ? after : undefined,
    existed,
    undoable
  }
}

/** How many lines per side a diff compares before truncating. */
const DIFF_MAX_LINES = 400

/** Longest diff text kept for the approval card. */
const DIFF_MAX_CHARS = 8_000

/**
 * A line diff, with `-` for removed and `+` for added lines.
 * @param before - the file now.
 * @param after - the file the tool would write.
 * @returns the diff, or an empty string when nothing changes.
 */
function diffLines(before: string, after: string): string {
  if (before === after) return ''
  let left = before.split('\n')
  let right = after.split('\n')
  let note = ''
  if (left.length > DIFF_MAX_LINES || right.length > DIFF_MAX_LINES) {
    note = `（文件较长，只比较前 ${DIFF_MAX_LINES} 行）\n`
    left = left.slice(0, DIFF_MAX_LINES)
    right = right.slice(0, DIFF_MAX_LINES)
  }
  const table: number[][] = Array.from({ length: left.length + 1 }, () =>
    new Array<number>(right.length + 1).fill(0)
  )
  for (let i = left.length - 1; i >= 0; i -= 1) {
    for (let j = right.length - 1; j >= 0; j -= 1) {
      table[i][j] =
        left[i] === right[j]
          ? table[i + 1][j + 1] + 1
          : Math.max(table[i + 1][j], table[i][j + 1])
    }
  }
  const lines: string[] = []
  let i = 0
  let j = 0
  while (i < left.length && j < right.length) {
    if (left[i] === right[j]) {
      lines.push(`  ${left[i]}`)
      i += 1
      j += 1
    } else if (table[i + 1][j] >= table[i][j + 1]) {
      lines.push(`- ${left[i]}`)
      i += 1
    } else {
      lines.push(`+ ${right[j]}`)
      j += 1
    }
  }
  while (i < left.length) {
    lines.push(`- ${left[i]}`)
    i += 1
  }
  while (j < right.length) {
    lines.push(`+ ${right[j]}`)
    j += 1
  }
  let text = `--- 修改前\n+++ 修改后\n${note}${lines.join('\n')}`
  if (text.length > DIFF_MAX_CHARS) text = `${text.slice(0, DIFF_MAX_CHARS)}\n…（已截断）`
  return text
}

/** The text without a leading BOM, as `read` shows it. */
function stripBom(text: string): string {
  return text.charCodeAt(0) === 0xfeff ? text.slice(1) : text
}

/** `read`: numbered lines, capped. */
export async function readFile(
  workDir: string,
  scopeRoot: ToolScope,
  args: Record<string, unknown>,
  readOnlyRoots: string[]
): Promise<string> {
  const requested = textArg(args, 'path')
  const target = await resolveReadable(workDir, scopeRoot, requested, readOnlyRoots)
  // `readFile` on a folder fails with a platform-specific errno; point the
  // model at the tool that actually handles folders instead.
  const info = await fs.stat(target)
  if (info.isDirectory()) {
    throw new Error(`这是一个文件夹，用 list 或 tree 查看：${requested}`)
  }
  // Reading first and clipping afterwards still pays for the whole file in
  // memory; a log or database dump would be loaded just to show its first page.
  if (info.size > MAX_READ_BYTES) {
    throw new Error(
      `文件太大（${Math.round(info.size / (1024 * 1024))} MB，上限 ${MAX_READ_BYTES / (1024 * 1024)} MB）；请用 grep 定位或用 shell 分段查看：${requested}`
    )
  }
  const decoded = decodeTextInfo(await fs.readFile(target))
  // Editors on Windows often leave a BOM; the model should see the text, not it.
  const decodedText = decoded.text.charCodeAt(0) === 0xfeff ? decoded.text.slice(1) : decoded.text
  // A GBK source file would otherwise reach the model as mojibake, and every
  // `edit` built on it would look for text that is not there.
  const text =
    decoded.encoding === 'utf-8' ? decodedText : `${decodedText}\n（该文件按 ${decoded.encoding.toUpperCase()} 解码）`
  assertText(text, requested)
  // `''.split('\n')` is one empty line; numbering it would report a line that
  // does not exist and hide the fact that the file is empty.
  if (text === '') return '（文件是空的）'
  const lines = text.split('\n')
  const offset = Math.max(intArg(args, 'offset') ?? 1, 1)
  const limit = Math.min(Math.max(intArg(args, 'limit') ?? MAX_READ_LINES, 1), MAX_READ_LINES)
  // Saying "已显示到 9 行" for a 3-line file would send the model looking for
  // content that does not exist; name the real length instead.
  if (offset > lines.length) {
    return `（一共 ${lines.length} 行，offset ${offset} 超出了末尾；从 1 开始读或把 offset 调小）`
  }
  const slice = lines.slice(offset - 1, offset - 1 + limit)
  // Stop between lines: cutting the joined text at a character count would
  // split a line in half while the tail still says the whole line was shown.
  const parts: string[] = []
  let used = 0
  for (let index = 0; index < slice.length; index += 1) {
    const line = `${offset + index}\t${slice[index]}`
    const cost = line.length + (parts.length === 0 ? 0 : 1)
    if (used + cost > MAX_READ_CHARS) break
    parts.push(line)
    used += cost
  }
  // One line longer than the whole budget still has to show something.
  if (parts.length === 0 && slice.length > 0) {
    parts.push(`${offset}\t${slice[0].slice(0, safeEnd(slice[0], MAX_READ_CHARS))}`)
  }
  const shown = offset - 1 + parts.length
  const tail = shown < lines.length ? `\n…（共 ${lines.length} 行，已显示到 ${shown} 行）` : ''
  return parts.join('\n') + tail
}

/** `write`: create or overwrite, making parent directories. */
export async function writeFile(
  workDir: string,
  scopeRoot: ToolScope,
  args: Record<string, unknown>
): Promise<string> {
  const target = await resolveScoped(workDir, scopeRoot, textArg(args, 'path'))
  const content = textArg(args, 'content')
  if (content.length > MAX_WRITE_CHARS) {
    throw new Error(`内容太长（${content.length} 字符，上限 ${MAX_WRITE_CHARS}）`)
  }
  await fs.mkdir(path.dirname(target), { recursive: true })
  await fs.writeFile(target, scriptText(target, content), 'utf8')
  const shown = path.relative(workDir, target) || target
  return `已写入 ${shown}（${content.length} 字符）`
}

/** One replacement of an `edit` call. */
interface EditPair {
  old: string
  new: string
  /** True when every occurrence should change, not only a unique one. */
  replaceAll: boolean
}

/**
 * The replacements an `edit` call asks for.
 * `edits` wins when present; otherwise `old` / `new` describe one change.
 */
function editPairs(args: Record<string, unknown>): EditPair[] {
  if (args.edits !== undefined) {
    if (!Array.isArray(args.edits)) throw new Error('参数 edits 不是数组')
    if (args.edits.length === 0) throw new Error('edits 不能为空')
    return args.edits.map((item, index) => {
      if (!item || typeof item !== 'object' || Array.isArray(item)) {
        throw new Error(`edits[${index}] 不是对象`)
      }
      const pair = item as Record<string, unknown>
      if (typeof pair.old !== 'string' || typeof pair.new !== 'string') {
        throw new Error(`edits[${index}] 需要 old 和 new 字符串`)
      }
      return { old: pair.old, new: pair.new, replaceAll: pair.replaceAll === true }
    })
  }
  if (args.old === undefined && args.new === undefined) {
    throw new Error('缺少参数 old / new（或用 edits 批量替换）')
  }
  return [
    { old: textArg(args, 'old'), new: textArg(args, 'new'), replaceAll: args.replaceAll === true }
  ]
}

/**
 * Match `old` against `text`, adapting the line endings.
 *
 * Windows checkouts are mostly CRLF while a model writes `\n`, so a plain
 * `indexOf` would refuse a correct replacement. The file's own style wins:
 * CRLF files get CRLF, LF files stay LF.
 * @param text - the file now.
 * @param old - the text the model wants to replace.
 * @returns where the match starts, the exact text matched, and the file's EOL.
 */
function locateEdit(
  text: string,
  old: string
): { at: number; matched: string; eol: '\n' | '\r\n' } {
  const eol: '\n' | '\r\n' = text.includes('\r\n') ? '\r\n' : '\n'
  const direct = text.indexOf(old)
  if (direct >= 0) return { at: direct, matched: old, eol }
  if (eol === '\r\n') {
    const crlf = old.replace(/\r?\n/g, '\r\n')
    const at = text.indexOf(crlf)
    if (at >= 0) return { at, matched: crlf, eol }
  } else {
    const lf = old.replace(/\r\n/g, '\n')
    const at = text.indexOf(lf)
    if (at >= 0) return { at, matched: lf, eol }
  }
  return { at: -1, matched: old, eol }
}

/** The replacement text in the file's own line endings. */
function withLineEndings(text: string, eol: '\n' | '\r\n'): string {
  return eol === '\r\n' ? text.replace(/\r\n|\n/g, '\r\n') : text.replace(/\r\n/g, '\n')
}

/** Apply every replacement in order; each must still be unique unless `replaceAll`. */
function applyEdits(text: string, pairs: EditPair[]): string {
  let result = text
  for (const [index, pair] of pairs.entries()) {
    const where = pairs.length === 1 ? '' : `（第 ${index + 1} 处）`
    if (!pair.old) throw new Error(`old 不能为空${where}`)
    const found = locateEdit(result, pair.old)
    if (found.at < 0) throw new Error(`文件里找不到要替换的文本${where}：${shorten(pair.old)}`)
    const replacement = withLineEndings(pair.new, found.eol)
    if (pair.replaceAll) {
      result = result.split(found.matched).join(replacement)
    } else {
      if (result.indexOf(found.matched, found.at + 1) >= 0) {
        throw new Error(`要替换的文本出现不止一次${where}，请写得更具体或加 replaceAll`)
      }
      result = result.slice(0, found.at) + replacement + result.slice(found.at + found.matched.length)
    }
  }
  return result
}

/** One file a batch replace would change. */
interface ReplaceChange {
  /** Path as the model sees it: relative, or absolute in a wider workspace. */
  label: string
  /** Absolute path written on disk. */
  target: string
  /** Text after the replacement. */
  after: string
  /** How many occurrences changed. */
  count: number
}

/** One file a batch replace leaves alone, and why. */
interface ReplaceSkip {
  label: string
  reason: string
}

/** What a batch replace would do, worked out before anything is written. */
interface ReplacePlan {
  pattern: string
  old: string
  new: string
  replaceAll: boolean
  changes: ReplaceChange[]
  skipped: ReplaceSkip[]
  /** Files that simply do not contain the text; counted, not listed. */
  notFound: number
  truncated: boolean
}

/**
 * Select files and work out every replacement, without writing anything.
 *
 * The workspace is what `glob` searches: relative paths from the writable root,
 * honouring `.gitignore`. Each file is judged on its own, so a binary or
 * non-unique file is skipped without stopping the rest.
 * @param workDir - relative paths resolve from here.
 * @param scopeRoot - writable root; null means the work directory.
 * @param args - `pattern`, `old`, `new` and optional `replaceAll`.
 * @returns what would change and what would be skipped.
 */
async function planReplace(
  workDir: string,
  scopeRoot: ToolScope,
  args: Record<string, unknown>
): Promise<ReplacePlan> {
  const pattern = textArg(args, 'pattern')
  const old = textArg(args, 'old')
  const replacementText = textArg(args, 'new')
  if (old === '') throw new Error('old 不能为空')
  const replaceAll = args.replaceAll === true

  const matcher = wildcard(pattern)
  const base = scopeRoot === null ? workDir : path.resolve(scopeRoot)
  const absolute = base !== path.resolve(workDir)
  const own = await walk(base)
  const matched = own.files.filter((file) => matcher.test(file))
  const truncated = own.truncated || matched.length > MAX_RESULTS
  const changes: ReplaceChange[] = []
  const skipped: ReplaceSkip[] = []
  let notFound = 0

  for (const file of matched.slice(0, MAX_RESULTS)) {
    const target = path.join(base, file)
    const label = absolute ? absolutePath(base, file) : file
    const info = await fs.stat(target).catch(() => null)
    if (!info || !info.isFile()) {
      skipped.push({ label, reason: '不是文件' })
      continue
    }
    if (info.size > MAX_EDIT_BYTES) {
      const limit = MAX_EDIT_BYTES / (1024 * 1024)
      skipped.push({ label, reason: `超过 ${limit} MB（编辑上限）` })
      continue
    }
    let text: string
    try {
      text = new TextDecoder('utf-8', { fatal: true }).decode(await fs.readFile(target))
    } catch {
      skipped.push({ label, reason: '不是 UTF-8 / 二进制' })
      continue
    }
    if (text.includes('\u0000')) {
      skipped.push({ label, reason: '二进制文件' })
      continue
    }
    const found = locateEdit(text, old)
    if (found.at < 0) {
      notFound += 1
      continue
    }
    const count = text.split(found.matched).length - 1
    if (!replaceAll && count > 1) {
      skipped.push({ label, reason: `出现 ${count} 次（不唯一）` })
      continue
    }
    const replacement = withLineEndings(replacementText, found.eol)
    if (found.matched === replacement) {
      skipped.push({ label, reason: '替换前后相同' })
      continue
    }
    const after = replaceAll
      ? text.split(found.matched).join(replacement)
      : text.slice(0, found.at) +
        replacement +
        text.slice(found.at + found.matched.length)
    if (after === text) {
      skipped.push({ label, reason: '替换前后相同' })
      continue
    }
    changes.push({ label, target, after, count })
  }
  return { pattern, old, new: replacementText, replaceAll, changes, skipped, notFound, truncated }
}

/** Write a file in one step, so a failed write leaves the old one whole. */
async function writeTextAtomic(target: string, text: string): Promise<void> {
  const temporary = `${target}.dsh-flow-${process.pid}.tmp`
  await fs.writeFile(temporary, scriptText(target, text), 'utf8')
  try {
    await fs.rename(temporary, target)
  } catch (error) {
    await fs.unlink(temporary).catch(() => undefined)
    throw error
  }
}

/** The file lists a batch replace reports, capped so one call stays readable. */
const REPLACE_LIST = 30

/**
 * `replace`: run the same replacement over every file a glob picks.
 *
 * Every file is planned and written on its own: a file that does not contain
 * the text, is not unique (unless `replaceAll`), is binary or is too large is
 * skipped and named, while the others still change.
 * @param workDir - relative paths resolve from here.
 * @param scopeRoot - writable root; null means the work directory.
 * @param args - `pattern`, `old`, `new` and optional `replaceAll`.
 * @returns what changed, how many places, and what was skipped.
 */
export async function replaceFiles(
  workDir: string,
  scopeRoot: ToolScope,
  args: Record<string, unknown>
): Promise<string> {
  const plan = await planReplace(workDir, scopeRoot, args)
  if (plan.changes.length === 0 && plan.skipped.length === 0 && plan.notFound === 0) {
    return `没有匹配 ${plan.pattern} 的文件`
  }
  const changed: ReplaceChange[] = []
  const failures: ReplaceSkip[] = []
  for (const change of plan.changes) {
    try {
      await writeTextAtomic(change.target, change.after)
      changed.push(change)
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error)
      failures.push({ label: change.label, reason: `写入失败：${message}` })
    }
  }
  const skipped = plan.skipped.concat(failures)
  const total = changed.reduce((sum, change) => sum + change.count, 0)
  const lines: string[] = []
  if (changed.length === 0) {
    lines.push(`没有文件被修改：${plan.pattern}`)
  } else {
    lines.push(`已替换 ${changed.length} 个文件，共 ${total} 处：`)
    for (const change of changed.slice(0, REPLACE_LIST)) {
      lines.push(`- ${change.label}：${change.count} 处`)
    }
    if (changed.length > REPLACE_LIST) {
      lines.push(`…（还有 ${changed.length - REPLACE_LIST} 个文件未列出）`)
    }
  }
  if (skipped.length > 0) {
    lines.push(`未改动 ${skipped.length} 个：`)
    for (const skip of skipped.slice(0, REPLACE_LIST)) {
      lines.push(`- ${skip.label}：${skip.reason}`)
    }
    if (skipped.length > REPLACE_LIST) {
      lines.push(`…（还有 ${skipped.length - REPLACE_LIST} 个未列出）`)
    }
  }
  if (plan.notFound > 0) {
    lines.push(`另有 ${plan.notFound} 个文件没有这段文本`)
  }
  if (plan.truncated) {
    lines.push(`…（只处理了前 ${MAX_RESULTS} 个匹配文件，用更具体的 pattern 缩小范围）`)
  }
  return lines.join('\n')
}

/**
 * The confirmation card for a batch replace: which files, how many places and
 * what is being skipped. No undo pair is kept: one card cannot cover many files.
 */
async function previewReplace(
  workDir: string,
  scopeRoot: ToolScope,
  args: Record<string, unknown>
): Promise<ToolPreview | null> {
  let plan: ReplacePlan
  try {
    plan = await planReplace(workDir, scopeRoot, args)
  } catch {
    return null
  }
  if (plan.changes.length === 0) return null
  const total = plan.changes.reduce((sum, change) => sum + change.count, 0)
  const lines = [
    `批量替换 ${plan.pattern}${plan.replaceAll ? '（替换所有出现）' : '（每个文件必须唯一）'}`,
    `原文：${shorten(plan.old)}`,
    `新文：${shorten(plan.new)}`,
    `将修改 ${plan.changes.length} 个文件，共 ${total} 处：`
  ]
  for (const change of plan.changes.slice(0, REPLACE_LIST)) {
    lines.push(`- ${change.label}：${change.count} 处`)
  }
  if (plan.changes.length > REPLACE_LIST) {
    lines.push(`…（还有 ${plan.changes.length - REPLACE_LIST} 个文件未列出）`)
  }
  if (plan.skipped.length > 0) {
    lines.push(`跳过 ${plan.skipped.length} 个：`)
    for (const skip of plan.skipped.slice(0, REPLACE_LIST)) {
      lines.push(`- ${skip.label}：${skip.reason}`)
    }
    if (plan.skipped.length > REPLACE_LIST) {
      lines.push(`…（还有 ${plan.skipped.length - REPLACE_LIST} 个未列出）`)
    }
  }
  if (plan.notFound > 0) {
    lines.push(`另有 ${plan.notFound} 个文件没有这段文本`)
  }
  if (plan.truncated) {
    lines.push(`…（只处理了前 ${MAX_RESULTS} 个匹配文件）`)
  }
  let diff = lines.join('\n')
  if (diff.length > DIFF_MAX_CHARS) {
    diff = `${diff.slice(0, safeEnd(diff, DIFF_MAX_CHARS))}\n…（已截断）`
  }
  return { diff, existed: true, undoable: false }
}

/** `edit`: replace one or several unique occurrences. */
export async function editFile(
  workDir: string,
  scopeRoot: ToolScope,
  args: Record<string, unknown>
): Promise<string> {
  const pathArg = textArg(args, 'path')
  const target = await resolveScoped(workDir, scopeRoot, pathArg)
  const pairs = editPairs(args)
  const info = await fs.stat(target)
  if (info.size > MAX_EDIT_BYTES) {
    const limit = MAX_EDIT_BYTES / (1024 * 1024)
    throw new Error(
      `文件太大（${info.size} 字节，编辑上限 ${limit} MB）；请用 shell 或分段处理：${pathArg}`
    )
  }
  // Rewriting is only safe when the bytes were UTF-8 to begin with: `applyEdits`
  // would otherwise write the file back in a different encoding, or the model's
  // `old` text would never match what is on disk.
  const bytes = await fs.readFile(target)
  let text: string
  try {
    text = new TextDecoder('utf-8', { fatal: true }).decode(bytes)
  } catch {
    throw new Error(`文件不是 UTF-8，edit 不改写它（先转码或改用 write）：${pathArg}`)
  }
  assertText(text, pathArg)
  const after = applyEdits(text, pairs)
  await fs.writeFile(target, scriptText(target, after), 'utf8')
  const shown = path.relative(workDir, target) || target
  return pairs.length === 1 ? `已修改 ${shown}` : `已修改 ${shown}（${pairs.length} 处）`
}

/**
 * Text as it has to sit on disk.
 *
 * Windows PowerShell (5.1) reads a script without a byte-order mark as ANSI, so
 * a `.ps1` holding Chinese becomes mojibake the moment it runs — the paths and
 * strings inside it turn into `缂婂懏璇℃伓` and everything downstream is wrong.
 * A BOM makes the same file read as UTF-8 everywhere; ASCII-only scripts are
 * left alone so ordinary files keep their byte-for-byte content.
 */
function scriptText(target: string, content: string): string {
  if (!/\.ps(m|d)?1$/i.test(target)) return content
  const body = content.charCodeAt(0) === 0xfeff ? content.slice(1) : content
  return body.length > 0 && /[^\u0000-\u007f]/.test(body) ? `\uFEFF${body}` : content
}

/**
 * Refuse a file that is not text.
 *
 * A NUL byte is the usual sign of a binary file; reading it as UTF-8 would
 * show mojibake, and rewriting it after an `edit` would corrupt it for good.
 */
function assertText(text: string, requested: string): void {
  if (text.includes('\u0000')) {
    throw new Error(`看起来是二进制文件，不按文本处理：${requested}`)
  }
}

/** Keep an error message from quoting a whole file-sized `old` value. */
function shorten(text: string): string {
  const line = text.split('\n')[0]
  return line.length > 80 ? `${line.slice(0, safeEnd(line, 80))}…` : line
}

/** `move`: rename a file, never overwriting an existing target. */
export async function moveFile(
  workDir: string,
  scopeRoot: ToolScope,
  args: Record<string, unknown>
): Promise<string> {
  const fromArg = textArg(args, 'from')
  const toArg = textArg(args, 'to')
  const from = await resolveScoped(workDir, scopeRoot, fromArg)
  const to = await resolveScoped(workDir, scopeRoot, toArg)
  if (!(await fs.stat(from)).isFile()) throw new Error(`只能移动文件：${fromArg}`)
  if (await fs.lstat(to).then(() => true).catch(() => false)) {
    throw new Error(`目标已存在：${toArg}`)
  }
  await fs.mkdir(path.dirname(to), { recursive: true })
  try {
    await fs.rename(from, to)
  } catch (error) {
    // A rename across drives fails; copy then remove the source.
    const code = (error as { code?: string }).code
    if (code !== 'EXDEV') throw error
    await fs.copyFile(from, to)
    await fs.unlink(from)
  }
  return `已移动 ${fromArg} → ${toArg}`
}

/** `delete`: remove one file; folders are refused. */
export async function deleteFile(
  workDir: string,
  scopeRoot: ToolScope,
  args: Record<string, unknown>
): Promise<string> {
  const targetArg = textArg(args, 'path')
  const target = await resolveScoped(workDir, scopeRoot, targetArg)
  const info = await fs.lstat(target)
  if (!info.isFile() && !info.isSymbolicLink()) {
    throw new Error(`只能删除文件；文件夹请手动处理：${targetArg}`)
  }
  await fs.unlink(target)
  return `已删除 ${targetArg}`
}

/** `list`: direct children of one folder, directories first. */
export async function listDir(
  workDir: string,
  scopeRoot: ToolScope,
  args: Record<string, unknown>,
  readOnlyRoots: string[]
): Promise<string> {
  if (args.path !== undefined && typeof args.path !== 'string') {
    throw new Error('参数 path 不是字符串')
  }
  const requested = args.path && args.path.trim() ? args.path : '.'
  const target = await resolveReadable(workDir, scopeRoot, requested, readOnlyRoots)
  let entries: Array<{ name: string; isDirectory: () => boolean }>
  try {
    entries = await fs.readdir(target, { withFileTypes: true })
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    throw new Error(`读不了这个文件夹：${message}`)
  }
  // Same folders `glob` / `grep` skip: listing them would only suggest places
  // the other tools never search.
  const visible = entries.filter((entry) => !SKIPPED.has(entry.name))
  if (visible.length === 0) return '文件夹是空的'
  visible.sort((a, b) => {
    const byKind = Number(b.isDirectory()) - Number(a.isDirectory())
    if (byKind !== 0) return byKind
    return a.name < b.name ? -1 : a.name > b.name ? 1 : 0
  })
  const shown = visible
    .slice(0, MAX_RESULTS)
    .map((entry) => (entry.isDirectory() ? `${entry.name}/` : entry.name))
  const hidden = visible.length - shown.length
  return (
    shown.join('\n') +
    (hidden > 0 ? `\n…（还有 ${hidden} 项未显示，用 glob 缩小范围）` : '')
  )
}

/**
 * Resolve a folder the model asked for: the writable scope first, then the
 * read-only reference roots (engine and workflow folders).
 * @returns the absolute path of an existing folder.
 */
async function resolveFolder(
  workDir: string,
  scopeRoot: ToolScope,
  requested: string,
  readOnlyRoots: string[]
): Promise<string> {
  const candidates: string[] = []
  try {
    candidates.push(await resolveScoped(workDir, scopeRoot, requested))
  } catch (error) {
    if (scopeRoot === null) throw error
  }
  if (scopeRoot !== null) {
    for (const base of readOnlyRoots) {
      try {
        candidates.push(await resolveScoped(base, base, requested))
      } catch {
        // Not under this reference root; try the next one.
      }
    }
  }
  for (const candidate of candidates) {
    const info = await fs.stat(candidate).catch(() => null)
    if (info?.isDirectory()) return candidate
  }
  if (candidates.length > 0) throw new Error(`不是文件夹：${requested}`)
  throw new Error(`找不到文件夹：${requested}`)
}

/**
 * `tree`: a shallow map of one folder, so the model can orient itself without
 * a dozen `list` calls. Ignored folders are left out, like `glob` / `grep`.
 */
export async function treeDir(
  workDir: string,
  scopeRoot: ToolScope,
  args: Record<string, unknown>,
  readOnlyRoots: string[]
): Promise<string> {
  const requested = typeof args.path === 'string' && args.path.trim() ? args.path : '.'
  // In workspace scope `.` means the workspace, like `glob` / `grep` do: the
  // map is for orienting in the project, not for peeking at the chat folder.
  let base: string
  if (requested === '.' && scopeRoot !== null) {
    base = path.resolve(scopeRoot)
    const info = await fs.stat(base).catch(() => null)
    if (!info?.isDirectory()) throw new Error(`工作区不是文件夹：${base}`)
  } else {
    base = await resolveFolder(workDir, scopeRoot, requested, readOnlyRoots)
  }
  const depth = Math.min(Math.max(intArg(args, 'depth') ?? 2, 1), MAX_TREE_DEPTH)
  const limit = Math.min(
    Math.max(intArg(args, 'limit') ?? MAX_TREE_ENTRIES, 1),
    MAX_TREE_ENTRIES
  )
  const rules = await ignoreRulesFor(base)
  const lines: string[] = []
  let truncated = false
  const visit = async (relative: string, level: number): Promise<void> => {
    let entries: Array<{
      name: string
      isDirectory: () => boolean
      isFile: () => boolean
    }>
    try {
      entries = await fs.readdir(path.join(base, relative), { withFileTypes: true })
    } catch {
      return
    }
    entries.sort((a, b) => {
      const byKind = Number(b.isDirectory()) - Number(a.isDirectory())
      if (byKind !== 0) return byKind
      return a.name < b.name ? -1 : a.name > b.name ? 1 : 0
    })
    for (const entry of entries) {
      if (lines.length >= limit) {
        truncated = true
        return
      }
      const child = relative === '.' ? entry.name : `${relative}/${entry.name}`
      if (isIgnored(rules, child)) continue
      if (entry.isDirectory()) {
        if (SKIPPED.has(entry.name)) continue
        lines.push(`${'  '.repeat(level)}${entry.name}/`)
        if (level + 1 < depth) await visit(child, level + 1)
      } else if (entry.isFile()) {
        lines.push(`${'  '.repeat(level)}${entry.name}`)
      }
    }
  }
  await visit('.', 0)
  if (lines.length === 0) return '文件夹是空的'
  const shown =
    path.resolve(base) === path.resolve(workDir) ? '.' : base.replace(/\\/g, '/')
  const notes = [`${shown}（深度 ${depth}，${lines.length} 项）`]
  if (truncated) notes.push(`…（只列了前 ${limit} 项，用 path / depth 缩小范围）`)
  return notes.concat(lines).join('\n')
}
