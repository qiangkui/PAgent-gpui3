/**
 * `read_image` and `screenshot`: letting the model see a picture.
 *
 * Tool messages carry text, so a tool returns the image beside its text and the
 * agent loop attaches it to the next message. That is what makes "look at the
 * file you just changed" and "here is what is on screen" possible at all: the
 * text-only tools could describe a UI but never check one.
 */

import { promises as fs } from 'node:fs'
import path from 'node:path'
import type { ToolImage, ToolResult, ToolScope } from './types.ts'
import { MAX_IMAGE_BYTES, MAX_IMAGE_PIXELS } from './limits.ts'
import { looksLikePdf } from './pdf.ts'
import { intArg, resolveReadable, resolveScoped, textArg } from './paths.ts'
import { isSensitivePath, sensitiveRefusal } from './secrets.ts'
import { killTree } from './process.ts'
import { spawnTool } from './spawn.ts'

/** Bitmap formats a provider accepts, recognised by their first bytes. */
const MAGIC: Array<{ mime: string; match: (bytes: Buffer) => boolean }> = [
  {
    mime: 'image/png',
    match: (bytes) =>
      bytes.length > 8 && bytes[0] === 0x89 && bytes.subarray(1, 4).toString('latin1') === 'PNG'
  },
  {
    mime: 'image/jpeg',
    match: (bytes) =>
      bytes.length > 3 && bytes[0] === 0xff && bytes[1] === 0xd8 && bytes[2] === 0xff
  },
  { mime: 'image/gif', match: (bytes) => bytes.subarray(0, 4).toString('latin1') === 'GIF8' },
  {
    mime: 'image/webp',
    match: (bytes) =>
      bytes.length > 12 &&
      bytes.subarray(0, 4).toString('latin1') === 'RIFF' &&
      bytes.subarray(8, 12).toString('latin1') === 'WEBP'
  },
  { mime: 'image/bmp', match: (bytes) => bytes.subarray(0, 2).toString('latin1') === 'BM' }
]

/**
 * Read one image file and hand it to the model.
 *
 * Text files already have `read`; this is the same tool for pictures, and it
 * refuses anything that is not a bitmap rather than sending mojibake to a
 * vision model.
 * @param workDir - relative paths resolve from here.
 * @param scopeRoot - writable root; `null` means the whole computer.
 * @param args - `path` of the image.
 * @param readOnlyRoots - extra folders that may be read.
 * @returns the image plus a one-line note for the transcript.
 */
export async function readImage(
  workDir: string,
  scopeRoot: ToolScope,
  args: Record<string, unknown>,
  readOnlyRoots: string[]
): Promise<ToolResult> {
  const requested = textArg(args, 'path')
  const target = await resolveReadable(workDir, scopeRoot, requested, readOnlyRoots)
  // A picture of a key file would be just as revealing as reading it.
  if (isSensitivePath(requested) || isSensitivePath(target)) {
    throw new Error(sensitiveRefusal(requested))
  }
  const info = await fs.stat(target)
  if (!info.isFile()) throw new Error(`不是文件：${requested}`)
  if (info.size > MAX_IMAGE_BYTES) {
    throw new Error(
      `图片太大（${bytesText(info.size)}，上限 ${MAX_IMAGE_BYTES / (1024 * 1024)} MB）：${requested}`
    )
  }
  const bytes = await fs.readFile(target)
  assertPixelCap(bytes, requested)
  const image = { ...imageOf(bytes, requested), path: target }
  return {
    ok: true,
    text: `已读取图片 ${requested}（${image.mime}，${bytesText(info.size)}），这张图已附在下一条消息里。`,
    image
  }
}

/** Sniff the format from the bytes, or refuse the file. */
function imageOf(bytes: Buffer, requested: string): ToolImage {
  const found = MAGIC.find((entry) => entry.match(bytes))
  if (!found) {
    // A PDF is the one binary a person is likely to aim at this tool; it has a
    // text extractor of its own, and saying so beats "not a bitmap".
    if (looksLikePdf(bytes) || /\.pdf$/i.test(requested)) {
      throw new Error(
        `这是 PDF，不是位图；用 read 抽取文字（read 支持 pages: "3-7"），或把某一页渲染成 PNG 再看：${requested}`
      )
    }
    throw new Error(
      `不是位图图片（支持 PNG / JPEG / GIF / WebP / BMP；SVG 需要先转成位图）：${requested}`
    )
  }
  return { mime: found.mime, data: bytes.toString('base64') }
}

/** One picture's pixel size, when the format lets it be read from the header. */
interface PixelSize {
  width: number
  height: number
}

/**
 * Refuse a picture whose area a vision model would reject anyway.
 *
 * Providers bill and cap images by area, and a well-compressed PNG can hold far
 * more pixels than the 5 MB byte cap suggests. Refusing with a hint beats sending
 * it and getting the whole request rejected; a format whose header does not say
 * the size is left alone rather than guessed at.
 * @param bytes - the file's bytes.
 * @param requested - path as the model wrote it, for the message.
 */
function assertPixelCap(bytes: Buffer, requested: string): void {
  const size = dimensionsOf(bytes)
  if (size === null || size.width <= 0 || size.height <= 0) return
  const pixels = size.width * size.height
  if (pixels <= MAX_IMAGE_PIXELS) return
  throw new Error(
    `${requested} 是 ${size.width}×${size.height} 像素（约 ${Math.round(pixels / 1_000_000)} MP），` +
      `超过图片上限 ${MAX_IMAGE_PIXELS / 1_000_000} MP：视觉模型按面积计费与限额。` +
      `先用 shell 缩小，或截图时用 maxEdge。`
  )
}

/** Read the pixel size from the header of the formats `read_image` accepts. */
function dimensionsOf(bytes: Buffer): PixelSize | null {
  const head = bytes.subarray(0, 16).toString('latin1')
  if (bytes.length >= 24 && head.startsWith('\u0089PNG')) {
    return { width: bytes.readUInt32BE(16), height: bytes.readUInt32BE(20) }
  }
  if (bytes.length >= 10 && (head.startsWith('GIF87a') || head.startsWith('GIF89a'))) {
    return { width: bytes.readUInt16LE(6), height: bytes.readUInt16LE(8) }
  }
  if (bytes.length >= 26 && head.startsWith('BM')) {
    return {
      width: Math.abs(bytes.readInt32LE(18)),
      height: Math.abs(bytes.readInt32LE(22))
    }
  }
  if (head.startsWith('\u00ff\u00d8\u00ff')) return jpegSize(bytes)
  if (bytes.length >= 30 && head.startsWith('RIFF') && head.slice(8, 12) === 'WEBP') {
    return webpSize(bytes)
  }
  return null
}

/** Scan a JPEG for its start-of-frame marker, which carries the size. */
function jpegSize(bytes: Buffer): PixelSize | null {
  let offset = 2
  while (offset + 9 < bytes.length) {
    if (bytes[offset] !== 0xff) {
      offset += 1
      continue
    }
    const marker = bytes[offset + 1] ?? 0
    // No length field: fill bytes, restart markers, end of image, start of scan.
    if (marker === 0x01 || (marker >= 0xd0 && marker <= 0xd9)) {
      offset += 2
      continue
    }
    if (marker === 0xda) return null
    const length = bytes.readUInt16BE(offset + 2)
    if (length < 2) return null
    const isFrame = marker >= 0xc0 && marker <= 0xcf
    const excluded = marker === 0xc4 || marker === 0xc8 || marker === 0xcc
    if (isFrame && !excluded) {
      return { height: bytes.readUInt16BE(offset + 5), width: bytes.readUInt16BE(offset + 7) }
    }
    offset += 2 + length
  }
  return null
}

/** Read a WebP canvas size, whichever of its three encodings it uses. */
function webpSize(bytes: Buffer): PixelSize | null {
  const chunk = bytes.subarray(12, 16).toString('latin1')
  if (chunk === 'VP8X' && bytes.length >= 30) {
    return { width: 1 + bytes.readUIntLE(24, 3), height: 1 + bytes.readUIntLE(27, 3) }
  }
  if (chunk === 'VP8 ' && bytes.length >= 30) {
    return {
      width: bytes.readUInt16LE(26) & 0x3fff,
      height: bytes.readUInt16LE(28) & 0x3fff
    }
  }
  if (chunk === 'VP8L' && bytes.length >= 25) {
    const bits = bytes.readUInt32LE(21)
    return { width: 1 + (bits & 0x3fff), height: 1 + ((bits >>> 14) & 0x3fff) }
  }
  return null
}

/** How long one capture may take before it is given up on. */
const SCREENSHOT_TIMEOUT_MS = 25_000

/** Longest edge a capture is kept at when the call names no `maxEdge`. */
const DEFAULT_MAX_EDGE = 1_600

/** One rectangle cropped out of the capture area, in capture pixels. */
export interface ShotCrop {
  /** Distance from the area's left edge. */
  x: number
  /** Distance from the area's top edge. */
  y: number
  /** Width of the part to keep. */
  width: number
  /** Height of the part to keep. */
  height: number
}

/**
 * The crop a call asked for, or null for the whole capture area.
 *
 * All four numbers must be given, and the size must be positive: a partial
 * rectangle would silently mean a different area than the person picked, so it is
 * ignored rather than guessed. The capture script clips the result to the area
 * itself, because a window may be smaller than the numbers suggest.
 * @param args - the call's arguments.
 * @returns the rectangle, relative to the capture area.
 */
export function shotCrop(args: Record<string, unknown>): ShotCrop | null {
  const offset = (key: string): number => {
    const value = intArg(args, key)
    return value !== undefined && value >= 0 ? value : -1
  }
  const size = (key: string): number => {
    const value = intArg(args, key)
    return value !== undefined && value > 0 ? value : -1
  }
  const x = offset('x')
  const y = offset('y')
  const width = size('width')
  const height = size('height')
  if (x < 0 || y < 0 || width <= 0 || height <= 0) return null
  return { x, y, width, height }
}

/**
 * The PowerShell program that takes the picture.
 *
 * `System.Drawing` is part of Windows itself, so this needs no dependency the
 * engine does not already have. `SetProcessDPIAware` is what makes the sizes
 * physical rather than scaled, or a 150% display would get a capture of the top
 * left corner only. A capture is scaled down and encoded as JPEG by default:
 * a full-screen PNG is around 200 KB, which is 270 KB of base64 on every round
 * it stays in the conversation.
 */
const CAPTURE = [
  'Add-Type -AssemblyName System.Windows.Forms, System.Drawing',
  "Add-Type -TypeDefinition 'using System; using System.Runtime.InteropServices; public class DshWin { [DllImport(\"user32.dll\")] public static extern IntPtr GetForegroundWindow(); [DllImport(\"user32.dll\")] public static extern bool GetWindowRect(IntPtr h, out RECT r); [DllImport(\"user32.dll\")] public static extern bool SetProcessDPIAware(); [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left; public int Top; public int Right; public int Bottom; } }'",
  '[DshWin]::SetProcessDPIAware() | Out-Null',
  '$region = $env:DSH_SHOT_REGION',
  'if ($region -eq "all") {',
  '  $v = [System.Windows.Forms.SystemInformation]::VirtualScreen',
  '  $rect = [System.Drawing.Rectangle]::new($v.Left, $v.Top, $v.Width, $v.Height)',
  '} else {',
  '  $r = [DshWin+RECT]::new()',
  '  $h = [DshWin]::GetForegroundWindow()',
  '  if ($h -ne [IntPtr]::Zero -and [DshWin]::GetWindowRect($h, [ref]$r) -and ($r.Right -gt $r.Left) -and ($r.Bottom -gt $r.Top)) {',
  '    $rect = [System.Drawing.Rectangle]::new($r.Left, $r.Top, ($r.Right - $r.Left), ($r.Bottom - $r.Top))',
  '  } else {',
  '    $v = [System.Windows.Forms.SystemInformation]::VirtualScreen',
  '    $rect = [System.Drawing.Rectangle]::new($v.Left, $v.Top, $v.Width, $v.Height)',
  '  }',
  '}',
  'if ($rect.Width -le 0 -or $rect.Height -le 0) { throw "capture area is empty" }',
  // A crop is relative to the area chosen above (the window or the screen), so a
  // person can point at one control without the rest of the window coming along.
  // It is clipped to that area, and an empty result is an error rather than a
  // zero-sized picture.
  '$crop = $env:DSH_SHOT_CROP',
  'if ($crop) {',
  '  $parts = $crop -split ","',
  '  $cx = [int]$parts[0]',
  '  $cy = [int]$parts[1]',
  '  $cw = [int]$parts[2]',
  '  $chh = [int]$parts[3]',
  '  $x0 = [Math]::Max($rect.Left + $cx, $rect.Left)',
  '  $y0 = [Math]::Max($rect.Top + $cy, $rect.Top)',
  '  $x1 = [Math]::Min($rect.Left + $cx + $cw, $rect.Right)',
  '  $y1 = [Math]::Min($rect.Top + $cy + $chh, $rect.Bottom)',
  '  if (($x1 -le $x0) -or ($y1 -le $y0)) { throw "crop area is outside the capture area" }',
  '  $rect = [System.Drawing.Rectangle]::new($x0, $y0, ($x1 - $x0), ($y1 - $y0))',
  '}',
  '$bmp = [System.Drawing.Bitmap]::new($rect.Width, $rect.Height)',
  '$g = [System.Drawing.Graphics]::FromImage($bmp)',
  '$g.CopyFromScreen($rect.Left, $rect.Top, 0, 0, $rect.Size)',
  '$g.Dispose()',
  '$cap = "{0}x{1}" -f $rect.Width, $rect.Height',
  '$max = [int]$env:DSH_SHOT_MAX',
  'if ($max -gt 0 -and ($rect.Width -gt $max -or $rect.Height -gt $max)) {',
  '  $scale = [Math]::Min($max / $rect.Width, $max / $rect.Height)',
  '  $w = [int][Math]::Round($rect.Width * $scale)',
  '  $h = [int][Math]::Round($rect.Height * $scale)',
  '  $small = [System.Drawing.Bitmap]::new([Math]::Max($w, 1), [Math]::Max($h, 1))',
  '  $sg = [System.Drawing.Graphics]::FromImage($small)',
  '  $sg.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic',
  '  $sg.DrawImage($bmp, 0, 0, $small.Width, $small.Height)',
  '  $sg.Dispose()',
  '  $bmp.Dispose()',
  '  $bmp = $small',
  '}',
  '$fin = "{0}x{1}" -f $bmp.Width, $bmp.Height',
  'if ($env:DSH_SHOT_FORMAT -eq "png") {',
  '  $bmp.Save($env:DSH_SHOT_OUT, [System.Drawing.Imaging.ImageFormat]::Png)',
  '} else {',
  '  $codec = [System.Drawing.Imaging.ImageCodecInfo]::GetImageEncoders() | Where-Object { $_.MimeType -eq "image/jpeg" }',
  '  $params = [System.Drawing.Imaging.EncoderParameters]::new(1)',
  '  $params.Param[0] = [System.Drawing.Imaging.EncoderParameter]::new([System.Drawing.Imaging.Encoder]::Quality, [long]$env:DSH_SHOT_QUALITY)',
  '  $bmp.Save($env:DSH_SHOT_OUT, $codec, $params)',
  '}',
  '$bmp.Dispose()',
  'Write-Output ("{0}|{1}|{2}|{3}" -f $cap, $fin, $region, $env:DSH_SHOT_FORMAT)'
].join('\n')

/**
 * Capture the screen and hand it to the model.
 *
 * The picture is kept under the conversation's own `.dsh-flow/shots` unless the
 * call names a `path`: the step row shows where it went, and the next question
 * can show the same picture again from that file. A read-only sandbox that
 * refuses the write only means the image lives in this message and not on disk.
 * @param workDir - the conversation folder, which holds `shots/`.
 * @param scopeRoot - writable root; `null` means the whole computer.
 * @param args - `region` (`active` / `all`), `maxEdge`, `format`, `quality`, `path`,
 * and the optional crop `x` / `y` / `width` / `height`.
 * @returns the image plus where it was saved.
 */
export async function screenshot(
  workDir: string,
  scopeRoot: ToolScope,
  args: Record<string, unknown>
): Promise<ToolResult> {
  if (process.platform !== 'win32') {
    throw new Error('screenshot 只在 Windows 上可用；其它平台先截图成文件，再用 read_image 读它。')
  }
  // The foreground window is the default: a full-desktop capture would hand the
  // model every window the person happens to have open, which is not theirs to
  // send.
  const region =
    typeof args.region === 'string' && args.region.trim().toLowerCase() === 'all' ? 'all' : 'active'
  const crop = shotCrop(args)
  const maxEdge = clamp(intArg(args, 'maxEdge') ?? DEFAULT_MAX_EDGE, 320, 4_000)
  const format =
    typeof args.format === 'string' && args.format.trim().toLowerCase() === 'png' ? 'png' : 'jpg'
  const quality = clamp(intArg(args, 'quality') ?? 80, 30, 100)
  const extension = format === 'png' ? 'png' : 'jpg'
  // The capture file goes under the conversation's sandbox temp folder rather
  // than the system temp: a low-integrity child can only write where the jail
  // grants it, and that folder is exactly where it grants one.
  const tempDir = path.join(workDir, '.dsh-flow', 'sandbox-tmp')
  await fs.mkdir(tempDir, { recursive: true }).catch(() => undefined)
  const temporary = path.join(tempDir, `shot-${process.pid}-${Date.now()}.${extension}`)
  const output = await runCapture(temporary, region, maxEdge, format, quality, workDir, crop)
  const info = await fs.stat(temporary).catch(() => null)
  if (!info?.isFile() || info.size === 0) {
    await fs.unlink(temporary).catch(() => undefined)
    const why =
      output
        .split('\n')
        .map((line) => line.trim())
        .find((line) => line !== '') ?? ''
    throw new Error(why === '' ? '截图失败：PowerShell 没有生成图片' : `截图失败：${why}`)
  }
  const bytes = await fs.readFile(temporary)
  // The temporary copy has done its job once the bytes are in memory.
  await fs.unlink(temporary).catch(() => undefined)
  if (bytes.length > MAX_IMAGE_BYTES) {
    throw new Error(
      `截图太大（${bytesText(bytes.length)}，上限 ${MAX_IMAGE_BYTES / (1024 * 1024)} MB）；把 maxEdge 调小或改用 jpg 再试。`
    )
  }
  const [captured = '', final = ''] = (output.split('\n')[0] ?? '').split('|')
  const wanted = typeof args.path === 'string' ? args.path.trim() : ''
  const name = `shot-${stamp()}.${extension}`
  // A read-only conversation keeps the picture in the sandbox temp folder even
  // when the call named a path: writing into the workspace would contradict
  // 「仅可查看」. The note below says so instead of pretending it was saved there.
  const readonly = process.env.DSH_SANDBOX_READONLY === '1'
  const saved = readonly
    ? await saveShot(workDir, name, bytes)
    : wanted === ''
      ? await saveShot(workDir, name, bytes)
      : await saveShotAt(workDir, scopeRoot, wanted, bytes)
  const how = region === 'all' ? '整屏' : '前台窗口'
  const area = crop === null ? how : `${how}，裁切 ${crop.width}×${crop.height} @ (${crop.x},${crop.y})`
  const size =
    captured === '' ? '' : `，${captured}${final === '' || final === captured ? '' : ` → ${final}`}`
  const lines = [`已截图（${area}${size}，${format.toUpperCase()}，${bytesText(bytes.length)}）`]
  if (readonly && wanted !== '') {
    lines.push(`只读模式：没有写到 ${wanted}`)
  }
  lines.push(saved === null ? '没有保存到工作区（只附在这条消息里）' : `保存到 ${saved.label}`)
  lines.push('这张图已附在下一条消息里；下一次提问会再带上它。')
  return {
    ok: true,
    text: lines.join('\n'),
    image: {
      mime: format === 'png' ? 'image/png' : 'image/jpeg',
      data: bytes.toString('base64'),
      ...(saved === null ? {} : { path: saved.absolute })
    }
  }
}

/**
 * Run the capture script and hand back what it printed.
 *
 * It goes through `spawnTool`, so a shell command and a screenshot run under the
 * same low-integrity jail; the picture is written where that jail grants a write
 * (the conversation's sandbox temp folder).
 */
function runCapture(
  out: string,
  region: string,
  maxEdge: number,
  format: string,
  quality: number,
  cwd: string,
  crop: ShotCrop | null = null
): Promise<string> {
  return new Promise((resolve) => {
    const child = spawnTool(
      {
        file: 'powershell',
        args: ['-NoProfile', '-NonInteractive', '-Command', CAPTURE],
        shell: false
      },
      cwd,
      {
        capture: true,
        env: {
          DSH_SHOT_OUT: out,
          DSH_SHOT_REGION: region,
          DSH_SHOT_MAX: String(maxEdge),
          DSH_SHOT_FORMAT: format,
          DSH_SHOT_QUALITY: String(quality),
          // Empty means the whole area; the script only crops when it is set.
          DSH_SHOT_CROP: crop === null ? '' : `${crop.x},${crop.y},${crop.width},${crop.height}`
        }
      }
    )
    let output = ''
    const timer = setTimeout(() => killTree(child), SCREENSHOT_TIMEOUT_MS)
    const finish = (): void => {
      clearTimeout(timer)
      resolve(output.trim())
    }
    child.stdout?.setEncoding('utf8').on('data', (chunk: string) => {
      output += chunk
    })
    child.stderr?.setEncoding('utf8').on('data', (chunk: string) => {
      output += chunk
    })
    child.on('error', (error) => {
      output += `\n${error.message}`
      finish()
    })
    child.on('close', finish)
  })
}

/** A saved picture: what the transcript shows, and what the file is. */
export interface SavedShot {
  /** Path as the step row shows it, relative when it went to `shots/`. */
  label: string
  /** Absolute path, for reading the same picture back later. */
  absolute: string
}

/**
 * Keep one copy of a picture under the conversation's shots folder.
 *
 * Used by tools that produce an image themselves (the browser tool), so the
 * "where do screenshots go" policy — `.dsh-flow/shots`, or the sandbox temp
 * folder in a read-only conversation — lives in exactly one place.
 * @param workDir - the conversation folder.
 * @param bytes - the encoded image.
 * @param extension - `png` or `jpg`, which decides the file name.
 * @returns where it was saved, or null when the write was refused.
 */
export async function saveImageCopy(
  workDir: string,
  bytes: Buffer,
  extension: 'png' | 'jpg'
): Promise<SavedShot | null> {
  return await saveShot(workDir, `shot-${stamp()}.${extension}`, bytes)
}

/** Keep a copy under `.dsh-flow/shots`, or report that it could not. */
async function saveShot(workDir: string, name: string, bytes: Buffer): Promise<SavedShot | null> {
  const { dir, label } = shotsDir(workDir)
  const target = path.join(dir, name)
  try {
    await fs.mkdir(dir, { recursive: true })
    await fs.writeFile(target, bytes)
    return { label: `${label}/${name}`, absolute: target }
  } catch {
    return null
  }
}

/**
 * Where a capture is kept.
 *
 * A read-only conversation must not write into its workspace — the promise of
 * 「仅可查看」 is that nothing on disk changes — so the copy goes to the
 * low-integrity temp folder the sandbox already owns. The picture itself still
 * reaches the model either way.
 * @param workDir - the conversation folder.
 * @returns the absolute folder and the label the step row shows.
 */
export function shotsDir(workDir: string): { dir: string; label: string } {
  if (process.env.DSH_SANDBOX_READONLY === '1') {
    const workspace = process.env.DSH_SANDBOX_WORKSPACE || workDir
    return {
      dir: path.join(workspace, '.dsh-flow', 'sandbox-tmp', 'shots'),
      label: '.dsh-flow/sandbox-tmp/shots'
    }
  }
  return { dir: path.join(workDir, '.dsh-flow', 'shots'), label: '.dsh-flow/shots' }
}

/** Save to the path the call named, inside the writable scope. */
async function saveShotAt(
  workDir: string,
  scopeRoot: ToolScope,
  wanted: string,
  bytes: Buffer
): Promise<SavedShot | null> {
  try {
    const target = await resolveScoped(workDir, scopeRoot, wanted)
    await fs.mkdir(path.dirname(target), { recursive: true })
    await fs.writeFile(target, bytes)
    return { label: wanted, absolute: target }
  } catch {
    return null
  }
}

/** A local timestamp for a file name: `20261126-153045-120`. */
function stamp(): string {
  const now = new Date()
  const pad = (value: number): string => String(value).padStart(2, '0')
  const millis = String(now.getMilliseconds()).padStart(3, '0')
  return (
    `${now.getFullYear()}${pad(now.getMonth() + 1)}${pad(now.getDate())}` +
    `-${pad(now.getHours())}${pad(now.getMinutes())}${pad(now.getSeconds())}-${millis}`
  )
}

/** A byte count as something a person reads at a glance. */
export function bytesText(bytes: number): string {
  if (bytes < 1024) return `${bytes} 字节`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}

/** Keep a number inside the range a call may ask for. */
function clamp(value: number, low: number, high: number): number {
  return Math.min(Math.max(value, low), high)
}