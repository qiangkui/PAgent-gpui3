/** Text decoding, clipping and ANSI stripping shared by the tools. */

/** Move a prefix cut forward if it would split a UTF-16 surrogate pair. */
export function safeEnd(text: string, index: number): number {
  if (index <= 0) return 0
  if (index >= text.length) return text.length
  const code = text.charCodeAt(index)
  return code >= 0xdc00 && code <= 0xdfff ? index + 1 : index
}

/** Move a suffix cut backward if it would split a UTF-16 surrogate pair. */
export function safeStart(text: string, index: number): number {
  if (index <= 0) return 0
  if (index >= text.length) return text.length
  const code = text.charCodeAt(index)
  return code >= 0xdc00 && code <= 0xdfff ? index - 1 : index
}

/**
 * Drop an incomplete UTF-8 sequence at the end of a byte buffer.
 *
 * A capped buffer can end in the middle of a character; strict decoding would
 * then reject a stream that is otherwise perfectly good UTF-8.
 */
export function trimUtf8Tail(buffer: Buffer): Buffer {
  let end = buffer.length
  let start = end - 1
  while (start >= 0 && end - start <= 4 && (buffer[start] & 0xc0) === 0x80) start -= 1
  if (start < 0) return buffer
  const lead = buffer[start]
  const need = lead >= 0xf0 ? 4 : lead >= 0xe0 ? 3 : lead >= 0xc0 ? 2 : 1
  return end - start < need ? buffer.subarray(0, start) : buffer
}

/** Decoded text and the encoding that produced it. */
export interface DecodedText {
  text: string
  encoding: 'utf-8' | 'utf-16' | 'gb18030'
}

/**
 * Decode text without being told its encoding.
 *
 * Windows terminals still hand back GBK for built-ins such as `dir`; strict
 * UTF-8 is tried first, so ordinary UTF-8 (including a character split across
 * pipe chunks) stays correct, and GB18030 is the fallback for the local code
 * page. A UTF-16 BOM is honoured because PowerShell can emit it.
 */
export function decodeTextInfo(buffer: Buffer): DecodedText {
  if (buffer.length >= 2 && buffer[0] === 0xff && buffer[1] === 0xfe) {
    return { text: new TextDecoder('utf-16le').decode(buffer.subarray(2)), encoding: 'utf-16' }
  }
  if (buffer.length >= 2 && buffer[0] === 0xfe && buffer[1] === 0xff) {
    return { text: new TextDecoder('utf-16be').decode(buffer.subarray(2)), encoding: 'utf-16' }
  }
  try {
    return { text: new TextDecoder('utf-8', { fatal: true }).decode(trimUtf8Tail(buffer)), encoding: 'utf-8' }
  } catch {
    try {
      return { text: new TextDecoder('gb18030').decode(buffer), encoding: 'gb18030' }
    } catch {
      return { text: buffer.toString('utf8'), encoding: 'utf-8' }
    }
  }
}

/** [`decodeTextInfo`] for callers that only need the text. */
export function decodeText(buffer: Buffer): string {
  return decodeTextInfo(buffer).text
}

/**
 * Decode a whole command's output, one line at a time.
 *
 * A Windows stream can mix encodings: `cmd` built-ins answer in the OEM code
 * page while a script prints UTF-8, and both land in the same pipe. Checking
 * the whole buffer once would reject it over a single GBK line and then decode
 * every UTF-8 line as GB18030; line by line each line gets its own answer.
 */
export function decodeOutput(buffer: Buffer): string {
  if (
    buffer.length >= 2 &&
    ((buffer[0] === 0xff && buffer[1] === 0xfe) || (buffer[0] === 0xfe && buffer[1] === 0xff))
  ) {
    return decodeText(buffer)
  }
  const parts: string[] = []
  let start = 0
  for (let index = 0; index < buffer.length; index += 1) {
    if (buffer[index] !== 0x0a) continue
    parts.push(decodeLine(buffer.subarray(start, index)))
    start = index + 1
  }
  if (start < buffer.length) parts.push(decodeLine(buffer.subarray(start)))
  return parts.join('\n')
}

/** One line: ASCII as-is, strict UTF-8 when it can be, GB18030 otherwise. */
function decodeLine(bytes: Buffer): string {
  if (!bytes.some((byte) => byte >= 0x80)) return bytes.toString('utf8')
  try {
    return new TextDecoder('utf-8', { fatal: true }).decode(bytes)
  } catch {
    return new TextDecoder('gb18030').decode(bytes)
  }
}

/** Keep the first `limit` bytes of a stream, in chunks (no O(n²) concat). */
export function keepBytes(chunks: Buffer[], size: number, chunk: Buffer, limit: number): number {
  if (size >= limit) return size
  const room = limit - size
  const kept = chunk.length <= room ? chunk : chunk.subarray(0, room)
  chunks.push(kept)
  return size + kept.length
}

/** Colours only add noise to an answer that is read as text. */
const ANSI = /\u001b\[[0-9;?]*[ -/]*[@-~]/g


/** Drop ANSI colour sequences; answers are read, not rendered. */
export function stripAnsi(text: string): string {
  return text.replace(ANSI, '')
}
