/** File tools: read, write, edit, move, delete, list, tree and previews. */

import { promises as fs } from 'node:fs'
import path from 'node:path'
import type { ToolCall, ToolPreview, ToolScope } from './types.ts'
import {
  MAX_EDIT_BYTES, MAX_READ_BYTES, MAX_READ_CHARS, MAX_READ_LINES, MAX_RESULTS,
  MAX_TREE_DEPTH, MAX_TREE_ENTRIES, MAX_WRITE_CHARS
} from './limits.ts'
import { decodeTextInfo, safeEnd } from './text.ts'
import { intArg, referenceRoots, resolveReadable, resolveScoped, textArg } from './paths.ts'
import {
  SKIPPED, baseIgnoreLayers, deeperIgnoreLayers, isIgnoredBy, isTopPlace, labelOf, searchRoots,
  skippedDir, walk, wildcard
} from './walk.ts'
import type { IgnoreLayer } from './walk.ts'
import { isSensitivePath, sensitiveRefusal } from './secrets.ts'
import { invalidateIndex } from './symbols.ts'
import { formatPdfReading, looksLikePdf, readPdfText } from './pdf.ts'

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
  call: ToolCall,
  skip: readonly string[] = []
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
      const after = applyEditCall(before, call.args).after
      return preview(before, after, true)
    }
    if (call.name === 'replace') {
      return await previewReplace(workDir, scopeRoot, call.args, skip)
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
  // A key reaches the provider once and stays in its logs; this asks instead of
  // reading, and the message says what to do instead.
  if (isSensitivePath(requested) || isSensitivePath(target)) {
    throw new Error(sensitiveRefusal(requested))
  }
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
  const bytes = await fs.readFile(target)
  // A PDF's words sit in compressed streams behind font maps, so `read` hands it
  // to an extractor instead of showing binary noise. The extension counts too, so
  // a broken header produces a clear extraction error rather than "binary file".
  if (looksLikePdf(bytes) || path.extname(target).toLowerCase() === '.pdf') {
    const pages = args.pages
    if (pages !== undefined && typeof pages !== 'string' && typeof pages !== 'number') {
      throw new Error('pages 要写成 3 或 3-7')
    }
    // `pages` is a string in the schema; a number is accepted because a model
    // sometimes sends `"pages": 3` for a single page.
    const reading = await readPdfText(target, pages === undefined ? '' : String(pages))
    return formatPdfReading(requested, reading)
  }
  const decoded = decodeTextInfo(bytes)
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
  const before = await inspectExisting(target)
  const notes: string[] = []
  // `write` speaks UTF-8 only. Replacing a GBK source with it would silently
  // change the file's encoding, which every later `edit` then refuses; saying so
  // in the result is the difference between a surprise and a decision.
  if (before?.foreign === true) {
    notes.push('\n（原文件不是 UTF-8，这次按 UTF-8 写回；要保留原编码请先转码或用 shell 处理）')
  } else if (before?.text !== undefined && similarText(before.text, content)) {
    // A whole-file rewrite that only touches a few lines costs the transcript far
    // more than an `edit` would; say so once, after the write has already landed.
    notes.push('\n（这次改动不大：用 edit 只发差异可以省很多上下文；本次已整篇写入）')
  }
  await fs.mkdir(path.dirname(target), { recursive: true })
  await fs.writeFile(target, scriptText(target, content), 'utf8')
  await invalidateIndex(indexRoot(workDir, scopeRoot))
  const shown = path.relative(workDir, target) || target
  return `已写入 ${shown}（${content.length} 字符）${notes.join('')}`
}

/** What a file that is about to be overwritten already holds. */
interface ExistingFile {
  /** The file's text when it decodes as UTF-8. */
  text?: string
  /** True when the file is text but not UTF-8 (GBK, UTF-16, …). */
  foreign: boolean
}

/**
 * Read a file that is about to be overwritten, within the edit size cap.
 *
 * Only files small enough for `edit` are read: a huge log is not a source file,
 * and loading it just to word a hint would cost more than the hint is worth.
 * @param target - absolute path that the write would replace.
 * @returns its text (when UTF-8) or why it cannot be compared; null if absent.
 */
async function inspectExisting(target: string): Promise<ExistingFile | null> {
  const info = await fs.stat(target).catch(() => null)
  if (info === null || !info.isFile() || info.size > MAX_EDIT_BYTES) return null
  const bytes = await fs.readFile(target).catch(() => null)
  if (bytes === null) return null
  // A UTF-16 file is text, and `read` shows it fine: overwriting it in UTF-8 is
  // exactly the change the note has to mention. Without this the NUL-byte guard
  // below would quietly call it binary and say nothing.
  if (
    bytes.length >= 2 &&
    ((bytes[0] === 0xff && bytes[1] === 0xfe) || (bytes[0] === 0xfe && bytes[1] === 0xff))
  ) {
    return { foreign: true }
  }
  if (bytes.includes(0)) return null
  try {
    return { text: new TextDecoder('utf-8', { fatal: true }).decode(bytes), foreign: false }
  } catch {
    return { foreign: true }
  }
}

/**
 * Whether `after` touches only a small part of a large `before`.
 *
 * A cheap line-by-line comparison, not a diff: the answer only decides whether a
 * hint is worth showing, and a false negative costs nothing.
 * @param before - the file's current text.
 * @param after - the text about to be written.
 * @returns true when the file is long and the change is small.
 */
function similarText(before: string, after: string): boolean {
  // Line endings are stripped before comparing: a CRLF file and an LF `content`
  // would otherwise differ on every line, and a genuinely small edit to a
  // Windows checkout would never earn the hint.
  const split = (text: string): string[] => text.split('\n').map((line) => line.replace(/\r$/, ''))
  const oldLines = split(before)
  if (oldLines.length < 40) return false
  const newLines = split(after)
  const shared = Math.min(oldLines.length, newLines.length)
  let changed = Math.abs(oldLines.length - newLines.length)
  for (let index = 0; index < shared; index += 1) {
    if (oldLines[index] !== newLines[index]) changed += 1
  }
  return changed <= Math.max(4, Math.round(Math.max(oldLines.length, newLines.length) * 0.05))
}

/** The folder the symbol index covers, matching `symbols`' own root. */
function indexRoot(workDir: string, scopeRoot: ToolScope): string {
  return scopeRoot === null ? workDir : path.resolve(scopeRoot)
}

/** One replacement of an `edit` call. */
interface EditPair {
  old: string
  new: string
  /** True when every occurrence should change, not only a unique one. */
  replaceAll: boolean
}

/**
 * An `edit` addressed by line number instead of by text.
 *
 * Text matching is the right default — it is unambiguous about what is being
 * replaced — but a model that has just read a file knows the line numbers, and
 * asking it to reproduce a long block exactly is where edits fail. Lines are
 * 1-based and count the same way `read` numbers them.
 */
interface LineEdit {
  /** First line replaced, 1-based. Absent for an insert. */
  start?: number
  /** Last line replaced, inclusive, 1-based. */
  end?: number
  /** Line the content goes after, 1-based; 0 means the top of the file. */
  insertAfter?: number
  /** Replacement or inserted text; an empty string deletes the addressed lines. */
  content: string
}

/** What one `edit` call does to a file. */
interface EditOutcome {
  /** The text the file would hold afterwards. */
  after: string
  /** Where it changed, for the tool result; empty for a plain text edit. */
  what: string
}

/**
 * Apply one `edit` call, whichever way it addressed the file.
 * @param text - the file's text now.
 * @param args - the call's arguments.
 * @returns the new text plus a short description of what changed.
 */
function applyEditCall(text: string, args: Record<string, unknown>): EditOutcome {
  const line = lineEdit(args)
  if (line !== null) return applyLineEdit(text, line)
  const pairs = editPairs(args)
  return { after: applyEdits(text, pairs), what: pairs.length === 1 ? '' : `${pairs.length} 处` }
}

/**
 * The line-addressed edit an `edit` call asks for, or null for a text edit.
 *
 * The two styles cannot be mixed in one call: `old` would then be matched
 * against a file the line edit has already changed, and the order would be
 * guesswork.
 */
function lineEdit(args: Record<string, unknown>): LineEdit | null {
  const hasStart = args.startLine !== undefined
  const hasInsert = args.insertAfter !== undefined
  if (!hasStart && !hasInsert) return null
  if (args.old !== undefined || args.new !== undefined || args.edits !== undefined) {
    throw new Error('edit 不能同时用 old / new / edits 和 startLine / insertAfter，二选一')
  }
  if (hasStart && hasInsert) throw new Error('startLine 和 insertAfter 只能给一个')
  if (typeof args.content !== 'string') {
    throw new Error('按行编辑需要 content 字符串；删除这几行时传空字符串')
  }
  if (args.content.length > MAX_WRITE_CHARS) {
    throw new Error(`内容太长（${args.content.length} 字符，上限 ${MAX_WRITE_CHARS}）`)
  }
  if (hasStart) {
    const start = intArg(args, 'startLine') ?? 0
    const end = intArg(args, 'endLine') ?? start
    if (start < 1) throw new Error('startLine 从 1 开始')
    if (end < start) throw new Error('endLine 不能小于 startLine')
    return { start, end, content: args.content }
  }
  const after = intArg(args, 'insertAfter') ?? -1
  if (after < 0) throw new Error('insertAfter 从 0 开始（0 = 插入到文件开头）')
  return { insertAfter: after, content: args.content }
}

/**
 * Apply a line-addressed edit without touching the rest of the file.
 *
 * Works on character offsets rather than on a `split('\n')` array: that keeps
 * the file's own line endings byte-for-byte, CRLF included, and never rewrites
 * lines the model did not address.
 * @param text - the file's text now.
 * @param edit - the resolved edit.
 * @returns the new text plus where it changed.
 */
function applyLineEdit(text: string, edit: LineEdit): EditOutcome {
  const eol: '\n' | '\r\n' = text.includes('\r\n') ? '\r\n' : '\n'
  const starts: number[] = [0]
  for (let index = 0; index < text.length; index += 1) {
    if (text[index] === '\n') starts.push(index + 1)
  }
  const piece = withLineEndings(edit.content, eol)
  if (edit.insertAfter !== undefined) {
    if (edit.insertAfter > starts.length) {
      throw new Error(`insertAfter 超出：文件共 ${starts.length} 行`)
    }
    const at =
      edit.insertAfter === 0
        ? 0
        : edit.insertAfter < starts.length
          ? starts[edit.insertAfter]
          : text.length
    if (piece.trim() === '') throw new Error('要插入的 content 不能为空')
    // A file whose last line has no trailing break still needs one before the
    // inserted text, or the two lines would run together.
    const lead = at > 0 && text[at - 1] !== '\n' ? eol : ''
    const where = edit.insertAfter === 0 ? '文件开头' : `第 ${edit.insertAfter} 行后`
    return { after: text.slice(0, at) + lead + piece + eol + text.slice(at), what: `${where}插入` }
  }
  const start = edit.start ?? 1
  const end = edit.end ?? start
  if (start > starts.length) throw new Error(`startLine 超出：文件共 ${starts.length} 行`)
  if (end > starts.length) throw new Error(`endLine 超出：文件共 ${starts.length} 行`)
  const begin = starts[start - 1]
  const finish = end < starts.length ? starts[end] : text.length
  const keepsBreak = finish > 0 && text[finish - 1] === '\n'
  const replacement = piece === '' ? '' : piece + (keepsBreak ? eol : '')
  const where = start === end ? `第 ${start} 行` : `第 ${start}–${end} 行`
  return {
    after: text.slice(0, begin) + replacement + text.slice(finish),
    what: `${where} → ${piece === '' ? '删除' : `${piece.split(/\r?\n/).length} 行`}`
  }
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

/** What one byte-level edit does to a file that is not UTF-8. */
interface ByteEditOutcome {
  /** The file's bytes afterwards. */
  bytes: Buffer
  /** What changed, for the tool result; empty for a single text edit. */
  what: string
}

/**
 * Apply an `edit` to a file that is not UTF-8, on its bytes.
 *
 * Node can decode GBK but has no GBK encoder, so decoding-and-rewriting would
 * change the file's encoding. An ASCII-only change needs no encoder: GBK is
 * ASCII-compatible and every ASCII byte is a whole character, so the change is
 * made in the buffer and the rest is copied byte for byte. Text with Chinese in
 * it is refused with a message saying how to proceed — guessing an encoding is
 * how a source file gets corrupted.
 * @param bytes - the file as it is on disk.
 * @param args - the call's arguments.
 * @returns the new bytes plus a short description of what changed.
 */
function applyByteEdit(bytes: Buffer, args: Record<string, unknown>): ByteEditOutcome {
  const line = lineEdit(args)
  if (line !== null) return applyByteLineEdit(bytes, line)
  const pairs = editPairs(args)
  let result = bytes
  for (const [index, pair] of pairs.entries()) {
    const where = pairs.length === 1 ? '' : `（第 ${index + 1} 处）`
    assertAsciiOnly(pair.old, `old${where}`)
    assertAsciiOnly(pair.new, `new${where}`)
    if (pair.old === '') throw new Error(`old 不能为空${where}`)
    // The file's own line endings win, exactly as they do on the UTF-8 path: a
    // GBK source checked out on Windows is CRLF, and the model writes `\n`.
    const variant = byteVariant(pair, result)
    const replaced = replaceAsciiBytes(result, variant.old, variant.new, pair.replaceAll)
    if (replaced.count === 0) {
      throw new Error(`文件里找不到要替换的文本${where}：${shorten(pair.old)}`)
    }
    if (replaced.count > 1 && !pair.replaceAll) {
      throw new Error(`要替换的文本出现不止一次${where}，请写得更具体或加 replaceAll`)
    }
    result = replaced.bytes
  }
  return { bytes: result, what: pairs.length === 1 ? '' : `${pairs.length} 处` }
}

/**
 * The pair tuned to the file's own line endings, for a byte-level edit.
 *
 * A GBK source on Windows is CRLF while a model writes `\n`, so the same `old`
 * that matches a UTF-8 file would otherwise report "找不到" here. Only the line
 * endings change: the text itself still has to match exactly.
 * @param pair - the replacement as the call wrote it.
 * @param bytes - the buffer it will be applied to.
 * @returns the pair with `\n` or `\r\n`, whichever the file uses.
 */
function byteVariant(pair: EditPair, bytes: Buffer): { old: string; new: string } {
  if (!pair.old.includes('\n') && !pair.new.includes('\n')) return pair
  const eol = bytes.includes(Buffer.from('\r\n')) ? '\r\n' : '\n'
  const tune = (text: string): string => text.replace(/\r\n|\n/g, eol)
  return { old: tune(pair.old), new: tune(pair.new) }
}

/**
 * Refuse text that cannot be written in the file's encoding.
 *
 * The file is not UTF-8, so a Chinese `old` would have to be encoded back the same
 * way to be found or written — and the engine has no such encoder. Saying so is
 * better than writing UTF-8 into a GBK file, which every later `read` would show
 * as mojibake.
 * @param text - the argument text.
 * @param label - which argument it is, for the message.
 */
function assertAsciiOnly(text: string, label: string): void {
  if (!/[^\u0000-\u007f]/.test(text)) return
  throw new Error(
    `这个文件不是 UTF-8，只能按 ASCII 原文修改；${label} 里有非 ASCII 字符。先把它转成 UTF-8（例如 shell 里用 iconv），或用 write 整篇重写。`
  )
}

/**
 * Apply a line-addressed `edit` to a file that is not UTF-8, on its bytes.
 *
 * Line starts are found by their `\n` bytes and the replacement is written in the
 * file's own line endings; every line the call did not address is copied
 * untouched, so the encoded text around it keeps its exact bytes.
 * @param bytes - the file as it is on disk.
 * @param edit - the resolved edit.
 * @returns the new bytes plus where it changed.
 */
function applyByteLineEdit(bytes: Buffer, edit: LineEdit): ByteEditOutcome {
  assertAsciiOnly(edit.content, 'content')
  const eol = bytes.includes(Buffer.from('\r\n')) ? Buffer.from('\r\n') : Buffer.from('\n')
  const starts: number[] = [0]
  for (let index = 0; index < bytes.length; index += 1) {
    if (bytes[index] === 0x0a) starts.push(index + 1)
  }
  const piece = Buffer.from(
    eol[0] === 0x0d
      ? edit.content.replace(/\r\n|\n/g, '\r\n')
      : edit.content.replace(/\r\n/g, '\n'),
    'latin1'
  )
  if (edit.insertAfter !== undefined) {
    if (edit.insertAfter > starts.length) {
      throw new Error(`insertAfter 超出：文件共 ${starts.length} 行`)
    }
    const at =
      edit.insertAfter === 0
        ? 0
        : edit.insertAfter < starts.length
          ? (starts[edit.insertAfter] ?? bytes.length)
          : bytes.length
    if (edit.content.trim() === '') throw new Error('要插入的 content 不能为空')
    const lead = at > 0 && bytes[at - 1] !== 0x0a ? eol : Buffer.alloc(0)
    const where = edit.insertAfter === 0 ? '文件开头' : `第 ${edit.insertAfter} 行后`
    return {
      bytes: Buffer.concat([bytes.subarray(0, at), lead, piece, eol, bytes.subarray(at)]),
      what: `${where}插入`
    }
  }
  const startLine = edit.start ?? 1
  const endLine = edit.end ?? startLine
  if (startLine > starts.length) throw new Error(`startLine 超出：文件共 ${starts.length} 行`)
  if (endLine > starts.length) throw new Error(`endLine 超出：文件共 ${starts.length} 行`)
  const begin = starts[startLine - 1] ?? 0
  const finish = endLine < starts.length ? (starts[endLine] ?? bytes.length) : bytes.length
  const keepsBreak = finish > 0 && bytes[finish - 1] === 0x0a
  const replacement =
    piece.length === 0 ? Buffer.alloc(0) : keepsBreak ? Buffer.concat([piece, eol]) : piece
  const where = startLine === endLine ? `第 ${startLine} 行` : `第 ${startLine}–${endLine} 行`
  return {
    bytes: Buffer.concat([bytes.subarray(0, begin), replacement, bytes.subarray(finish)]),
    what: `${where} → ${piece.length === 0 ? '删除' : `${edit.content.split(/\r?\n/).length} 行`}`
  }
}

/**
 * Replace ASCII text inside a buffer, counting only whole characters.
 *
 * The walk steps over characters, so a needle can only match where a character
 * starts. That matters: a GBK trail byte may itself be an ASCII letter, and a
 * plain `Buffer.indexOf` would happily replace half of a Chinese character.
 * @param bytes - the buffer to change.
 * @param old - ASCII text to find.
 * @param replacement - ASCII text to put there.
 * @param all - replace every occurrence instead of the first.
 * @returns the new buffer and how many places changed.
 */
export function replaceAsciiBytes(
  bytes: Buffer,
  old: string,
  replacement: string,
  all: boolean
): { bytes: Buffer; count: number } {
  const needle = Buffer.from(old, 'latin1')
  const put = Buffer.from(replacement, 'latin1')
  const pieces: Buffer[] = []
  let position = 0
  let count = 0
  while (position < bytes.length) {
    const lead = bytes[position] ?? 0
    if (lead < 0x80 && (all || count === 0) && matchesBytes(bytes, position, needle)) {
      pieces.push(put)
      count += 1
      position += needle.length
      continue
    }
    // A lead byte is followed by its trail byte, which is never a character of
    // its own; a lone trail at the end of the file is copied as it is.
    const length = lead < 0x80 ? 1 : 2
    pieces.push(bytes.subarray(position, Math.min(position + length, bytes.length)))
    position += length
  }
  return { bytes: Buffer.concat(pieces), count }
}

/** Whether `needle` sits at `at`, compared byte for byte. */
function matchesBytes(bytes: Buffer, at: number, needle: Buffer): boolean {
  if (at + needle.length > bytes.length) return false
  for (let offset = 0; offset < needle.length; offset += 1) {
    if (bytes[at + offset] !== needle[offset]) return false
  }
  return true
}

/**
 * Refuse a file that must not be changed at byte level.
 *
 * A UTF-16 file is text and `read` shows it, but its bytes interleave NULs with
 * characters, so an ASCII replacement would miss and a byte rewrite would destroy
 * it; saying "convert it first" is the only honest answer. Anything else with a
 * NUL byte is binary — the same rule `read` applies.
 * @param bytes - the file as it is on disk.
 * @param requested - the path as the model wrote it, for the message.
 */
function assertEditableBytes(bytes: Buffer, requested: string): void {
  const utf16 =
    bytes.length >= 2 &&
    ((bytes[0] === 0xff && bytes[1] === 0xfe) || (bytes[0] === 0xfe && bytes[1] === 0xff))
  if (utf16) {
    throw new Error(`文件是 UTF-16 编码，edit 只处理 UTF-8 与非 UTF-8 的 ASCII 原文；先转成 UTF-8：${requested}`)
  }
  if (bytes.includes(0)) {
    throw new Error(
      `看起来是二进制文件（或是没有 BOM 的 UTF-16），不按文本处理：${requested}；要正常改它先转成 UTF-8（例如用 iconv 或 PowerShell 的 Set-Content）`
    )
  }
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
  args: Record<string, unknown>,
  skip: readonly string[] = []
): Promise<ReplacePlan> {
  const pattern = textArg(args, 'pattern')
  if (args.path !== undefined && typeof args.path !== 'string') {
    throw new Error('参数 path 不是字符串')
  }
  const old = textArg(args, 'old')
  const replacementText = textArg(args, 'new')
  if (old === '') throw new Error('old 不能为空')
  const replaceAll = args.replaceAll === true

  const matcher = wildcard(pattern)
  const places = await searchRoots(workDir, scopeRoot, args.path ?? '', [])
  const changes: ReplaceChange[] = []
  const skipped: ReplaceSkip[] = []
  let notFound = 0
  let truncated = false
  let matchedTotal = 0

  for (const place of places) {
    const own = await walk(place.root, {
      skip: isTopPlace(place, workDir, scopeRoot) ? skip : []
    })
    truncated = truncated || own.truncated
    const matched = own.files.filter((file) => {
      const relative = place.prefix === '' ? file : `${place.prefix}/${file}`
      return matcher.test(relative)
    })
    matchedTotal += matched.length
    for (const file of matched) {
      if (changes.length + skipped.length >= MAX_RESULTS) {
        truncated = true
        break
      }
      const relative = place.prefix === '' ? file : `${place.prefix}/${file}`
      const target = path.join(place.root, file)
      const label = labelOf(place, relative)
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
      skipped.push({ label, reason: '不是 UTF-8（批量替换只处理 UTF-8；这一份可用 edit 按 ASCII 原文改）' })
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
  }
  return {
    pattern,
    old,
    new: replacementText,
    replaceAll,
    changes,
    skipped,
    notFound,
    truncated: truncated || matchedTotal > MAX_RESULTS
  }
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
  args: Record<string, unknown>,
  skip: readonly string[] = []
): Promise<string> {
  if (args.dryRun !== undefined && typeof args.dryRun !== 'boolean') {
    throw new Error('参数 dryRun 不是布尔值')
  }
  const plan = await planReplace(workDir, scopeRoot, args, skip)
  if (plan.changes.length === 0 && plan.skipped.length === 0 && plan.notFound === 0) {
    return `没有匹配 ${plan.pattern} 的文件`
  }
  // A preview answers with the same plan the card shows, and writes nothing: the
  // model can check its range before asking for the change.
  if (args.dryRun === true) {
    return `（预览，未写盘）\n${replacePlanText(plan)}`
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
  if (changed.length > 0) await invalidateIndex(indexRoot(workDir, scopeRoot))
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
 * What a batch replace would do, as text.
 *
 * Shared by the confirmation card, the `dryRun` answer and the result, so the
 * model reads the same words the person sees.
 * @param plan - what was worked out without writing anything.
 * @returns the plan, one line per file.
 */
function replacePlanText(plan: ReplacePlan): string {
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
  return lines.join('\n')
}

/**
 * The confirmation card for a batch replace: which files, how many places and
 * what is being skipped. No undo pair is kept: one card cannot cover many files.
 */
async function previewReplace(
  workDir: string,
  scopeRoot: ToolScope,
  args: Record<string, unknown>,
  skip: readonly string[] = []
): Promise<ToolPreview | null> {
  let plan: ReplacePlan
  try {
    plan = await planReplace(workDir, scopeRoot, args, skip)
  } catch {
    return null
  }
  if (plan.changes.length === 0) return null
  let diff = replacePlanText(plan)
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
  const info = await fs.stat(target)
  if (info.size > MAX_EDIT_BYTES) {
    const limit = MAX_EDIT_BYTES / (1024 * 1024)
    throw new Error(
      `文件太大（${info.size} 字节，编辑上限 ${limit} MB）；请用 shell 或分段处理：${pathArg}`
    )
  }
  // Rewriting is only safe when the bytes were UTF-8 to begin with: the edit
  // would otherwise write the file back in a different encoding, or the model's
  // `old` text would never match what is on disk. A GBK file still accepts an
  // ASCII-only change, applied to its bytes without decoding anything.
  const bytes = await fs.readFile(target)
  let text: string | null = null
  try {
    text = new TextDecoder('utf-8', { fatal: true }).decode(bytes)
  } catch {
    text = null
  }
  const shown = path.relative(workDir, target) || target
  if (text === null) {
    // Not UTF-8. Before treating it as GBK text, rule out the two things that
    // are not text in that sense: a UTF-16 file (which `read` shows fine) and a
    // binary file. Editing either at byte level would corrupt it.
    assertEditableBytes(bytes, pathArg)
    const outcome = applyByteEdit(bytes, args)
    await fs.writeFile(target, outcome.bytes)
    await invalidateIndex(indexRoot(workDir, scopeRoot))
    return outcome.what === ''
      ? `已修改 ${shown}（原文非 UTF-8，按 ASCII 替换）`
      : `已修改 ${shown}（原文非 UTF-8，${outcome.what}）`
  }
  assertText(text, pathArg)
  const outcome = applyEditCall(text, args)
  await fs.writeFile(target, scriptText(target, outcome.after), 'utf8')
  await invalidateIndex(indexRoot(workDir, scopeRoot))
  return outcome.what === '' ? `已修改 ${shown}` : `已修改 ${shown}（${outcome.what}）`
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
  await invalidateIndex(indexRoot(workDir, scopeRoot))
  return `已移动 ${fromArg} → ${toArg}`
}

/**
 * `copy`: duplicate one file, byte for byte.
 *
 * `write` only carries text, so until this existed there was no way to copy an
 * icon, a font or any other asset without going through `shell`. The source is
 * read and written by the OS, so binary content survives untouched.
 * @param workDir - relative paths resolve from here.
 * @param scopeRoot - writable root; null means the whole computer.
 * @param args - `from`, `to`, and optional `overwrite`.
 * @returns what was copied and how large it was.
 */
export async function copyFile(
  workDir: string,
  scopeRoot: ToolScope,
  args: Record<string, unknown>
): Promise<string> {
  const fromArg = textArg(args, 'from')
  const toArg = textArg(args, 'to')
  const from = await resolveScoped(workDir, scopeRoot, fromArg)
  const to = await resolveScoped(workDir, scopeRoot, toArg)
  const info = await fs.stat(from).catch(() => null)
  if (!info?.isFile()) throw new Error(`只能复制文件：${fromArg}`)
  const existing = await fs.lstat(to).catch(() => null)
  if (existing) {
    // Overwriting is a separate decision: an accidental `to` that points at an
    // existing file must not silently replace it.
    if (args.overwrite !== true) {
      throw new Error(`目标已存在：${toArg}（确实要覆盖请传 overwrite: true）`)
    }
    if (!existing.isFile()) throw new Error(`目标不是文件，不能覆盖：${toArg}`)
  }
  await fs.mkdir(path.dirname(to), { recursive: true })
  await fs.copyFile(from, to)
  await invalidateIndex(indexRoot(workDir, scopeRoot))
  return `已复制 ${fromArg} → ${toArg}（${sizeText(info.size)}）`
}

/**
 * `mkdir`: create one folder, parents included.
 *
 * `write` creates the folders a file needs, but an empty folder (a placeholder
 * for build output, say) had no way to exist before this.
 * @param workDir - relative paths resolve from here.
 * @param scopeRoot - writable root; null means the whole computer.
 * @param args - `path`.
 * @returns whether it was created or already there.
 */
export async function makeDir(
  workDir: string,
  scopeRoot: ToolScope,
  args: Record<string, unknown>
): Promise<string> {
  const pathArg = textArg(args, 'path')
  if (pathArg.trim() === '') throw new Error('path 不能为空')
  const target = await resolveScoped(workDir, scopeRoot, pathArg)
  const info = await fs.stat(target).catch(() => null)
  if (info?.isDirectory()) return `目录已存在：${pathArg}`
  if (info) throw new Error(`这里已经有一个文件，不能建目录：${pathArg}`)
  await fs.mkdir(target, { recursive: true })
  const shown = path.relative(workDir, target) || target
  return `已创建目录 ${shown}`
}

/** A byte count as something a person reads at a glance. */
function sizeText(bytes: number): string {
  if (bytes < 1024) return `${bytes} 字节`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}

/**
 * `delete`: remove one file, or a whole folder when `recursive` is set.
 *
 * A recursive delete is the one action that can lose a lot of work in a click, so
 * it stays opt-in: without the flag a folder is refused with a message that names
 * the files it holds, and the approval card says how many there are.
 */
export async function deleteFile(
  workDir: string,
  scopeRoot: ToolScope,
  args: Record<string, unknown>
): Promise<string> {
  const targetArg = textArg(args, 'path')
  const target = await resolveScoped(workDir, scopeRoot, targetArg)
  if (args.recursive !== undefined && typeof args.recursive !== 'boolean') {
    throw new Error('参数 recursive 不是布尔值')
  }
  const info = await fs.lstat(target)
  if (info.isDirectory()) {
    if (args.recursive !== true) {
      throw new Error(`这是一个文件夹，不能直接删除：${targetArg}（确实要连内容一起删请传 recursive: true）`)
    }
    const count = await countFiles(target).catch(() => 0)
    await fs.rm(target, { recursive: true, force: false, maxRetries: 2 })
    await invalidateIndex(indexRoot(workDir, scopeRoot))
    return `已删除目录 ${targetArg}（${count} 个文件）`
  }
  if (!info.isFile() && !info.isSymbolicLink()) {
    throw new Error(`只能删除文件或文件夹：${targetArg}`)
  }
  await fs.unlink(target)
  await invalidateIndex(indexRoot(workDir, scopeRoot))
  return `已删除 ${targetArg}`
}

/** How many files a folder holds, for the approval card and the result. */
async function countFiles(dir: string): Promise<number> {
  let total = 0
  const pending = [dir]
  while (pending.length > 0 && total < MAX_RESULTS * 50) {
    const current = pending.pop()
    if (current === undefined) break
    const entries = await fs.readdir(current, { withFileTypes: true }).catch(() => [])
    for (const entry of entries) {
      if (entry.isDirectory()) pending.push(path.join(current, entry.name))
      else total += 1
    }
  }
  return total
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
  readOnlyRoots: string[],
  skip: readonly string[] = []
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
  const skipped = new Set(skip.map((name) => name.toLowerCase()))
  const rootLayers = await baseIgnoreLayers(base)
  const lines: string[] = []
  let truncated = false
  /**
   * One folder of the map.
   * @param relative - folder relative to `base`, `.` for the root.
   * @param level - indentation level.
   * @param layers - ignore rules that apply inside this folder.
   */
  const visit = async (relative: string, level: number, layers: IgnoreLayer[]): Promise<void> => {
    const own =
      relative === '.' ? layers : await deeperIgnoreLayers(base, relative, layers)
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
      // A sibling conversation folder is only skipped where the map starts: the
      // same name deeper down belongs to the project being mapped.
      if (relative === '.' && skipped.has(entry.name.toLowerCase())) continue
      if (entry.isDirectory()) {
        if (skippedDir(entry.name) || isIgnoredBy(own, child)) continue
        lines.push(`${'  '.repeat(level)}${entry.name}/`)
        if (level + 1 < depth) await visit(child, level + 1, own)
      } else if (entry.isFile()) {
        if (isIgnoredBy(own, child)) continue
        lines.push(`${'  '.repeat(level)}${entry.name}`)
      }
    }
  }
  await visit('.', 0, rootLayers)
  if (lines.length === 0) return '文件夹是空的'
  const shown =
    path.resolve(base) === path.resolve(workDir) ? '.' : base.replace(/\\/g, '/')
  const notes = [`${shown}（深度 ${depth}，${lines.length} 项）`]
  if (truncated) notes.push(`…（只列了前 ${limit} 项，用 path / depth 缩小范围）`)
  return notes.concat(lines).join('\n')
}
