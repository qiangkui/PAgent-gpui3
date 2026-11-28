/**
 * `show`: put the files a task produced in front of the person.
 *
 * The model decides what belongs here — a report, a chart, an archive — and the
 * window draws each one as a card under the tool row: click opens it with the
 * system handler, the folder button reveals it. Without this a finished answer
 * only says「完成了」, and the reader has to find the file in the workspace by hand.
 */

import { promises as fs } from 'node:fs'
import path from 'node:path'
import type { ToolFile, ToolResult, ToolScope } from './types.ts'
import { resolveReadable } from './paths.ts'
import { isSensitivePath, sensitiveRefusal } from './secrets.ts'

/** How many files one call may show; more than this is a folder dump. */
export const MAX_SHOWN_FILES = 12

/**
 * The paths one call asked for: `paths` (the documented form) or a single `path`.
 * @param args - the call's arguments.
 * @returns the requested paths, in the order the model wrote them.
 */
function requestedPaths(args: Record<string, unknown>): string[] {
  const list = Array.isArray(args.paths) ? args.paths : []
  const paths = list
    .filter((item): item is string => typeof item === 'string')
    .map((item) => item.trim())
    .filter((item) => item !== '')
  if (paths.length > 0) return paths
  // A model sometimes sends the one file as `path`; refusing that would only cost
  // another round trip to say the same thing.
  const single = typeof args.path === 'string' ? args.path.trim() : ''
  return single === '' ? [] : [single]
}

/** A file size the card can show: 980 B, 12.3 KB, 4.7 MB. */
function sizeText(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}

/**
 * Resolve one requested file and describe it, or say why it cannot be shown.
 * @param workDir - the conversation folder.
 * @param scopeRoot - writable root; null means the whole computer.
 * @param requested - the path the model wrote.
 * @param readOnlyRoots - extra folders that may be read.
 * @returns the file, or the reason it was left out.
 */
async function shownFile(
  workDir: string,
  scopeRoot: ToolScope,
  requested: string,
  readOnlyRoots: string[]
): Promise<ToolFile | string> {
  if (isSensitivePath(requested)) return `${requested}：${sensitiveRefusal(requested)}`
  const target = await resolveReadable(workDir, scopeRoot, requested, readOnlyRoots)
  const info = await fs.stat(target).catch(() => null)
  if (info === null) return `${requested}：文件不存在`
  if (info.isDirectory()) return `${requested}：这是文件夹，只能展示文件`
  if (isSensitivePath(target)) return `${requested}：${sensitiveRefusal(requested)}`
  return { name: path.basename(target), path: target, bytes: info.size }
}

/**
 * Run one `show` call.
 *
 * Nothing is written and nothing is sent: the files already exist, and the only
 * effect is that the window draws them. Reading the file is not needed either, so
 * a 2 GB archive costs nothing to show.
 * @param workDir - the conversation folder; relative paths resolve from here.
 * @param scopeRoot - writable root; null means the whole computer.
 * @param args - `paths` (or `path`) plus an optional `note`.
 * @param readOnlyRoots - extra folders that may be read.
 * @returns the text for the model, and the cards the window draws.
 */
export async function showFiles(
  workDir: string,
  scopeRoot: ToolScope,
  args: Record<string, unknown>,
  readOnlyRoots: string[]
): Promise<ToolResult> {
  const requested = requestedPaths(args)
  if (requested.length === 0) throw new Error('show 需要 paths（要展示的文件路径）')
  if (requested.length > MAX_SHOWN_FILES) {
    throw new Error(`一次最多展示 ${MAX_SHOWN_FILES} 个文件（这次给了 ${requested.length} 个）`)
  }
  const files: ToolFile[] = []
  const refused: string[] = []
  // One failure must not swallow the others: five files with one typo should show
  // four and say which one is missing.
  for (const entry of requested) {
    const result = await shownFile(workDir, scopeRoot, entry, readOnlyRoots)
    if (typeof result === 'string') refused.push(result)
    else if (files.some((kept) => kept.path === result.path)) continue
    else files.push(result)
  }
  if (files.length === 0) throw new Error(`没有可展示的文件：${refused.join('；')}`)
  const note = typeof args.note === 'string' ? args.note.trim() : ''
  const lines = [`已在对话里展示 ${files.length} 个文件（用户能直接点开）：`]
  for (const file of files) lines.push(`- ${file.name}（${sizeText(file.bytes)}）`)
  if (note !== '') lines.push(note)
  if (refused.length > 0) lines.push(`没有展示：${refused.join('；')}`)
  return { ok: true, text: lines.join('\n'), files }
}