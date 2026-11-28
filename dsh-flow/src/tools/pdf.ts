/**
 * Reading a PDF's text.
 *
 * A PDF is a binary container: its words sit in compressed streams behind font
 * maps, so `read` cannot number its lines the way it numbers a source file. Two
 * external tools can pull the text out, and the engine bundles neither — adding a
 * dependency would change how it is packaged — so both are checked at call time:
 *
 * - Poppler's `pdftotext`, which is fast and keeps the layout;
 * - `pypdf` through Python, which is one `pip install pypdf` away on a machine
 *   that already has Python.
 *
 * When neither is installed the message names both, because "binary file" told the
 * person nothing they could act on.
 */

import path from 'node:path'
import { captureCommand } from './capture.ts'
import { MAX_PDF_CHARS, MAX_PDF_PAGES, PDF_TIMEOUT_MS } from './limits.ts'
import { findExecutable, resolveRuntime } from '../runtime.ts'
import { safeEnd } from './text.ts'

/** The marker every PDF carries; a valid file may have junk before it. */
const PDF_MAGIC = Buffer.from('%PDF-', 'latin1')

/** How far into a file the header may sit, per the PDF specification. */
const PDF_HEADER_WINDOW = 1_024

/**
 * Whether a buffer starts like a PDF.
 *
 * The specification allows up to 1024 bytes of junk before `%PDF-`, and files
 * produced by some mail gateways have it, so the marker is searched in that window
 * rather than required at byte zero.
 * @param bytes - the file's first bytes (a few KB is plenty).
 * @returns true for a PDF, however it was named.
 */
export function looksLikePdf(bytes: Buffer): boolean {
  return bytes.subarray(0, PDF_HEADER_WINDOW).includes(PDF_MAGIC)
}

/** One extractor's output: page texts joined with a form feed. */
export interface PdfExtract {
  /** Pages in order, separated by `\f` (the form feed both tools agree on). */
  text: string
  /** Total pages the document has, when the extractor reported it. */
  total?: number
  /** Which tool produced it. */
  via: string
}

/** Runs one extractor over a page range. */
export type PdfRunner = (file: string, first: number, last: number) => Promise<PdfExtract>

/** What one PDF reading gives the model. */
export interface PdfReading {
  /** Page-marked text, already inside the character cap. */
  text: string
  /** First page shown. */
  first: number
  /** Last page shown. */
  last: number
  /** Total pages, when the extractor knew. */
  total?: number
  /** True when a page range or the character cap left pages out. */
  truncated: boolean
  /** True when the pages held no text at all: a scan needs eyes, not a parser. */
  scanned: boolean
  /** Tool that produced the text. */
  via: string
}

/** An extractor installed by a test, so the suite needs neither real tool. */
let runner: PdfRunner | null = null

/**
 * Replace the extractor, or restore the real one with `null`.
 *
 * Only tests call this: the default runner probes the two real tools, and a
 * machine may have neither, which would make the PDF path untestable.
 * @param replacement - the runner to use, or null for the real one.
 */
export function usePdfRunner(replacement: PdfRunner | null): void {
  runner = replacement
}

/**
 * The page range an `pages` argument asks for.
 * @param value - `3`, `3-7`, or empty for the first {@link MAX_PDF_PAGES} pages.
 * @returns the inclusive range, 1-based.
 */
export function parsePageRange(value: string): { first: number; last: number } {
  const asked = value.trim()
  if (asked === '') return { first: 1, last: MAX_PDF_PAGES }
  const single = /^(\d+)$/.exec(asked)
  if (single !== null) {
    const page = Number(single[1])
    if (page < 1) throw new Error('pages 从 1 开始')
    return { first: page, last: page }
  }
  const range = /^(\d+)\s*-\s*(\d+)$/.exec(asked)
  if (range === null) throw new Error('pages 要写成 3 或 3-7')
  const first = Number(range[1])
  const last = Number(range[2])
  if (first < 1) throw new Error('pages 从 1 开始')
  if (last < first) throw new Error('pages 的结束页不能小于开始页')
  if (last - first + 1 > MAX_PDF_PAGES) {
    throw new Error(`一次最多读 ${MAX_PDF_PAGES} 页；把范围缩小再试`)
  }
  return { first, last }
}

/**
 * What to say when no extractor is installed.
 *
 * Returned as an error message rather than a silent refusal: the person can fix
 * this in one command, and the model has to know which one.
 * @returns the message, naming both installation paths.
 */
export function missingPdfToolMessage(): string {
  return [
    '这台机器上没有 PDF 抽取器，`read` 打不开 PDF 里的文字。装一个再试：',
    '  - Poppler 的 `pdftotext`（推荐：快、保留排版），把它的 bin 目录放进 PATH；',
    '  - 或 Python 的 `pypdf`：`python -m pip install pypdf`。',
    '也可以让用户把这份 PDF 附到聊天里（附件走模型的文档能力），或把某页渲染成图片用 read_image。'
  ].join('\n')
}

/**
 * Read a PDF's text for `read`.
 *
 * Text only: images in the PDF are not extracted, and a scanned page yields
 * nothing, which the caller reports as such instead of as an error.
 * @param file - absolute path to the PDF.
 * @param pages - the call's `pages` argument, empty for the first pages.
 * @returns the marked text and what it did not cover.
 */
export async function readPdfText(file: string, pages: string): Promise<PdfReading> {
  const { first, last } = parsePageRange(pages)
  const raw = await (runner ?? defaultRunner)(file, first, last)
  return assemble(raw, first, last, pages.trim() === '')
}

/**
 * The answer `read` gives for a PDF: what was read, then the page-marked text.
 * @param requested - the path as the model wrote it.
 * @param reading - what the extractor produced.
 * @returns the tool result text.
 */
export function formatPdfReading(requested: string, reading: PdfReading): string {
  const total = reading.total === undefined ? '' : `，共 ${reading.total} 页`
  const lines = [
    `${requested}：PDF，用 ${reading.via} 抽取文字（第 ${reading.first}–${reading.last} 页${total}）`
  ]
  if (reading.scanned) {
    lines.push(
      '这份 PDF 没有可提取的文字（可能是扫描件或纯图片页）：用 read_image 看渲染出来的页，或让用户把它附到聊天里交给视觉模型。'
    )
    return lines.join('\n')
  }
  lines.push(reading.text)
  if (reading.truncated) {
    const rest =
      reading.total !== undefined && reading.last < reading.total
        ? `还有 ${reading.total - reading.last} 页没读`
        : '后面的页没读'
    lines.push(
      `…（${rest}；继续用 pages: "${reading.last + 1}-${reading.last + MAX_PDF_PAGES}"，一次最多 ${MAX_PDF_PAGES} 页）`
    )
  }
  return lines.join('\n')
}

/** The default runner: `pdftotext` when it is on PATH, else `pypdf`. */
async function defaultRunner(file: string, first: number, last: number): Promise<PdfExtract> {
  if (findExecutable('pdftotext')) return await viaPdftotext(file, first, last)
  if (await pypdfReady()) return await viaPypdf(file, first, last)
  throw new Error(missingPdfToolMessage())
}

/** Poppler's `pdftotext`, which keeps the layout and writes text to stdout. */
async function viaPdftotext(file: string, first: number, last: number): Promise<PdfExtract> {
  const outcome = await captureCommand(
    {
      file: 'pdftotext',
      args: ['-layout', '-enc', 'UTF-8', '-f', String(first), '-l', String(last), file, '-'],
      label: `pdftotext -f ${first} -l ${last}`,
      shell: false
    },
    path.dirname(file),
    PDF_TIMEOUT_MS
  )
  if (outcome.timedOut) throw new Error(`pdftotext 超时（${PDF_TIMEOUT_MS / 1000} 秒）：${file}`)
  if (outcome.code !== 0) {
    const why = outcome.stderr.trim() || outcome.stdout.trim() || `退出码 ${outcome.code}`
    throw new Error(`pdftotext 读不了这份 PDF：${firstLine(why)}`)
  }
  return { text: outcome.stdout, via: 'pdftotext' }
}

/** Whether `python -c "import pypdf"` works; asked once per process. */
let pypdfCheck: Promise<boolean> | null = null

function pypdfReady(): Promise<boolean> {
  pypdfCheck ??= probePypdf()
  return pypdfCheck
}

async function probePypdf(): Promise<boolean> {
  if (resolveRuntime('python') === null) return false
  const outcome = await captureCommand(
    {
      file: 'python',
      args: ['-c', 'import pypdf'],
      label: 'python -c import pypdf',
      shell: false
    },
    process.cwd(),
    10_000
  ).catch(() => null)
  return outcome !== null && !outcome.timedOut && outcome.code === 0
}

/**
 * The `pypdf` path: one page at a time, in page order.
 *
 * The script writes the total page count to stderr, because stdout carries the
 * text and mixing the two would put a number in the middle of a paragraph. Both
 * streams are switched to UTF-8 first: Windows Python otherwise prints with the
 * system code page, and a page holding `®` or any character outside GBK makes it
 * fail with `UnicodeEncodeError` before a single word arrives.
 */
const PYPDF_SCRIPT = [
  'import sys',
  'sys.stdout.reconfigure(encoding="utf-8", errors="replace")',
  'sys.stderr.reconfigure(encoding="utf-8", errors="replace")',
  'from pypdf import PdfReader',
  'reader = PdfReader(sys.argv[1])',
  'total = len(reader.pages)',
  'sys.stderr.write("TOTAL:%d\\n" % total)',
  'if reader.is_encrypted:',
  '    sys.stderr.write("ENCRYPTED\\n")',
  '    sys.exit(3)',
  'first = max(1, int(sys.argv[2]))',
  'last = min(total, int(sys.argv[3]))',
  'parts = []',
  'for index in range(first - 1, last):',
  '    parts.append(reader.pages[index].extract_text() or "")',
  'sys.stdout.write("\\f".join(parts))'
].join('\n')

/** `pypdf` through the local Python; slow on big documents but very portable. */
async function viaPypdf(file: string, first: number, last: number): Promise<PdfExtract> {
  const outcome = await captureCommand(
    {
      file: 'python',
      args: ['-c', PYPDF_SCRIPT, file, String(first), String(last)],
      label: `python pypdf -f ${first} -l ${last}`,
      shell: false
    },
    path.dirname(file),
    PDF_TIMEOUT_MS
  )
  if (outcome.timedOut) throw new Error(`pypdf 超时（${PDF_TIMEOUT_MS / 1000} 秒）：${file}`)
  if (/ENCRYPTED/.test(outcome.stderr)) {
    throw new Error('这份 PDF 有密码保护，read 打不开；让用户给出解密后的文件，或用带密码的工具处理')
  }
  if (outcome.code !== 0) {
    const why = lastUsefulLine(outcome.stderr) || `退出码 ${outcome.code}`
    throw new Error(`pypdf 读不了这份 PDF：${why}`)
  }
  const total = Number(/TOTAL:(\d+)/.exec(outcome.stderr)?.[1] ?? NaN)
  return {
    text: outcome.stdout,
    total: Number.isFinite(total) ? total : undefined,
    via: 'pypdf'
  }
}

/**
 * Mark the pages and hold the whole reading inside the character cap.
 * @param raw - the extractor's output.
 * @param first - the first page asked for.
 * @param last - the last page asked for.
 * @param defaulted - true when the call named no range, so "there may be more
 * pages" is worth saying.
 * @returns the marked text and what it covered.
 */
function assemble(raw: PdfExtract, first: number, last: number, defaulted: boolean): PdfReading {
  const blocks = raw.text.split('\f')
  // A trailing form feed leaves one empty block behind; it is not a page.
  while (blocks.length > 1 && (blocks[blocks.length - 1] ?? '').trim() === '') blocks.pop()
  const pages = blocks.map((body, index) => ({ number: first + index, body: body.trimEnd() }))
  const scanned = pages.every((page) => page.body.trim() === '')
  const lines: string[] = []
  let used = 0
  let shown = 0
  let cut = false
  for (const page of pages) {
    const head = `—— 第 ${page.number} 页 ——`
    if (used + head.length + 1 > MAX_PDF_CHARS) {
      cut = true
      break
    }
    lines.push(head)
    used += head.length + 1
    shown += 1
    if (page.body.trim() === '') {
      const empty = '（这一页没有文字）'
      lines.push(empty)
      used += empty.length + 1
      continue
    }
    const room = MAX_PDF_CHARS - used
    if (page.body.length > room) {
      lines.push(`${page.body.slice(0, safeEnd(page.body, room))}…（这一页没显示完）`)
      cut = true
      break
    }
    lines.push(page.body)
    used += page.body.length + 1
  }
  const hiddenByRange =
    raw.total !== undefined ? shown < raw.total : defaulted && shown >= Math.min(last, MAX_PDF_PAGES)
  return {
    text: lines.join('\n'),
    first,
    last: shown === 0 ? first : first + shown - 1,
    total: raw.total,
    truncated: cut || hiddenByRange,
    scanned,
    via: raw.via
  }
}

/** The first non-empty line of an extractor's complaint, kept short. */
function firstLine(text: string): string {
  const line = text.split('\n').map((entry) => entry.trim()).find((entry) => entry !== '') ?? ''
  return line.length > 200 ? `${line.slice(0, 200)}…` : line
}

/**
 * The last line that actually explains a Python failure.
 *
 * A traceback ends with the exception, while the script's own `TOTAL:` and
 * `ENCRYPTED` markers come first; picking the first line would hide the reason
 * behind a page count.
 * @param text - the captured stderr.
 * @returns the message line, kept short.
 */
function lastUsefulLine(text: string): string {
  const lines = text
    .split('\n')
    .map((entry) => entry.trim())
    .filter((entry) => entry !== '' && !/^TOTAL:\d+$/.test(entry) && entry !== 'ENCRYPTED')
  const last = lines[lines.length - 1] ?? ''
  return last.length > 200 ? `${last.slice(0, 200)}…` : last
}