/** `http`: one GET or one bounded POST, so the assistant can read a page or an API. */

import type { ToolResult } from './types.ts'
import {
  HTTP_TIMEOUT_MS, MAX_HTTP_BODY, MAX_HTTP_BYTES, MAX_HTTP_CHARS, MAX_HTTP_HEADERS,
  MAX_HTTP_TIMEOUT_MS
} from './limits.ts'
import { intArg } from './paths.ts'
import { decodeOutput } from './text.ts'

/**
 * Headers the tool refuses to send.
 *
 * The model has no honest reason to attach credentials, and a card cannot show
 * a key without putting it in the transcript, so these are refused instead of
 * confirmed.
 */
const BLOCKED_HEADERS = new Set(['authorization', 'cookie', 'proxy-authorization'])

/** Most request headers one call may set. */
const MAX_REQUEST_HEADERS = 20

/** Longest single request header value kept. */
const MAX_HEADER_VALUE = 2_000

/**
 * Run one request.
 *
 * Redirects are not followed: the answer names the `location` instead, so a
 * public URL cannot quietly lead the request somewhere else.
 * @param args - url, method, headers, body and timeoutMs.
 * @returns the status line, the headers, and the body clipped to a readable size.
 */
export async function runHttp(args: Record<string, unknown>): Promise<ToolResult> {
  const method = methodOf(args)
  const url = urlOf(args)
  const headers = headersOf(args)
  const body = bodyOf(args, method)
  const asked = intArg(args, 'timeoutMs') ?? HTTP_TIMEOUT_MS
  const timeoutMs = Math.min(Math.max(asked, 100), MAX_HTTP_TIMEOUT_MS)
  const controller = new AbortController()
  const timer = setTimeout(() => controller.abort(), timeoutMs)
  let response: Response
  try {
    response = await fetch(url, {
      method,
      headers,
      body: method === 'POST' ? body : undefined,
      redirect: 'manual',
      signal: controller.signal
    })
  } catch (error) {
    if (controller.signal.aborted) throw new Error(`请求超时（${timeoutMs} ms）：${url}`)
    throw new Error(`请求失败：${error instanceof Error ? error.message : String(error)}`)
  } finally {
    clearTimeout(timer)
  }
  const { bytes, truncated } = await readBody(response)
  return { ok: response.status < 400, text: answer(method, url, response, bytes, truncated) }
}

/**
 * The line an approval card shows for a request.
 *
 * Deliberately lenient: the card only has to describe the call, and the tool
 * itself refuses anything invalid once the person has allowed it.
 * @param args - the raw tool arguments.
 * @returns `$ http GET <url>` plus the header names and the body size.
 */
export function httpSummary(args: Record<string, unknown>): string {
  const url = typeof args.url === 'string' ? args.url.trim() : ''
  const method = typeof args.method === 'string' && args.method.trim() !== ''
    ? args.method.trim().toUpperCase()
    : 'GET'
  const lines = [`$ http ${method} ${url === '' ? '（缺少 url）' : url}`]
  const headers = args.headers
  if (headers !== null && typeof headers === 'object' && !Array.isArray(headers)) {
    const entries = Object.entries(headers as Record<string, unknown>)
    if (entries.length > 0) {
      const shown = entries
        .slice(0, MAX_REQUEST_HEADERS)
        .map(([name, value]) => `${name}: ${typeof value === 'string' ? value : '…'}`)
      lines.push(`# 请求头：${shown.join('；')}`)
    }
  }
  if (typeof args.body === 'string' && args.body !== '') {
    lines.push(`# 正文：${args.body.length} 字符`)
  }
  if (method !== 'GET' && method !== 'POST') lines.push('# 方法只支持 GET / POST，这次会失败')
  return lines.join('\n')
}

/** The request method, rejecting anything that is not GET or POST. */
function methodOf(args: Record<string, unknown>): 'GET' | 'POST' {
  if (args.method === undefined || args.method === null) return 'GET'
  if (typeof args.method !== 'string') throw new Error('参数 method 不是字符串')
  const method = args.method.trim().toUpperCase()
  if (method === 'GET' || method === 'POST') return method
  throw new Error(`method 只能是 GET 或 POST（收到 ${method}）`)
}

/** The URL, refusing anything that is not plain http / https. */
function urlOf(args: Record<string, unknown>): string {
  if (typeof args.url !== 'string') throw new Error('缺少参数 url')
  const raw = args.url.trim()
  if (raw === '') throw new Error('url 不能为空')
  let parsed: URL
  try {
    parsed = new URL(raw)
  } catch {
    throw new Error(`url 不是完整地址（要带 http:// 或 https://）：${raw}`)
  }
  if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') {
    throw new Error(`只支持 http / https，收到 ${parsed.protocol}`)
  }
  if (parsed.username !== '' || parsed.password !== '') {
    throw new Error('url 里不要带账号密码；需要认证请用 shell')
  }
  if (parsed.hostname === '') throw new Error('url 缺少主机名')
  return parsed.toString()
}

/** The request headers, refusing the ones that would carry credentials. */
function headersOf(args: Record<string, unknown>): Record<string, string> {
  const headers: Record<string, string> = {}
  const raw = args.headers
  if (raw === undefined || raw === null) return headers
  if (typeof raw !== 'object' || Array.isArray(raw)) throw new Error('参数 headers 必须是对象')
  const entries = Object.entries(raw as Record<string, unknown>)
  if (entries.length > MAX_REQUEST_HEADERS) {
    throw new Error(`请求头太多（${entries.length} 个，上限 ${MAX_REQUEST_HEADERS}）`)
  }
  for (const [name, value] of entries) {
    if (typeof value !== 'string') throw new Error(`请求头 ${name} 的值不是字符串`)
    if (name.trim() === '') throw new Error('请求头名字不能为空')
    if (BLOCKED_HEADERS.has(name.trim().toLowerCase())) {
      throw new Error(`不发 ${name} 这类请求头（可能带密钥）；需要认证请用 shell`)
    }
    if (value.length > MAX_HEADER_VALUE) {
      throw new Error(`请求头 ${name} 太长（上限 ${MAX_HEADER_VALUE} 字符）`)
    }
    headers[name] = value
  }
  return headers
}

/** The POST body; GET has none. */
function bodyOf(args: Record<string, unknown>, method: 'GET' | 'POST'): string | undefined {
  if (args.body === undefined || args.body === null) return undefined
  if (typeof args.body !== 'string') throw new Error('参数 body 不是字符串')
  if (method === 'GET') throw new Error('GET 不带正文；要发正文请用 POST')
  if (args.body.length > MAX_HTTP_BODY) {
    throw new Error(`正文太长（${args.body.length} 字符，上限 ${MAX_HTTP_BODY}）`)
  }
  return args.body
}

/** Read the body up to the byte cap instead of trusting `content-length`. */
async function readBody(response: Response): Promise<{ bytes: Buffer; truncated: boolean }> {
  if (response.body === null) return { bytes: Buffer.alloc(0), truncated: false }
  const reader = response.body.getReader()
  const chunks: Buffer[] = []
  let size = 0
  let truncated = false
  for (;;) {
    const { done, value } = await reader.read()
    if (done) break
    if (value === undefined) continue
    chunks.push(Buffer.from(value))
    size += value.length
    if (size < MAX_HTTP_BYTES) continue
    truncated = true
    await reader.cancel().catch(() => undefined)
    break
  }
  return { bytes: Buffer.concat(chunks).subarray(0, MAX_HTTP_BYTES), truncated }
}

/** The result text: status line, headers, then the clipped body. */
function answer(
  method: string,
  url: string,
  response: Response,
  bytes: Buffer,
  truncated: boolean
): string {
  const status = `${response.status} ${response.statusText}`.trim()
  const lines = [`${method} ${url} → ${status}`]
  const headers = [...response.headers].slice(0, MAX_HTTP_HEADERS)
  for (const [name, value] of headers) {
    lines.push(`${name}: ${value}`)
  }
  if (response.status >= 300 && response.status < 400) {
    const location = response.headers.get('location') ?? '（没有 location）'
    lines.push(`# 没有跟随重定向；location: ${location}`)
  }
  const text = decodeOutput(bytes)
  if (text.trim() === '') {
    lines.push('# 正文为空')
    return lines.join('\n')
  }
  if (text.length > MAX_HTTP_CHARS) {
    lines.push(`# 正文 ${text.length} 字符，只显示前 ${MAX_HTTP_CHARS} 字符`)
    lines.push(text.slice(0, MAX_HTTP_CHARS))
    return lines.join('\n')
  }
  if (truncated) {
    lines.push(`# 正文超过 ${MAX_HTTP_BYTES} 字节，只保留开头`)
  }
  lines.push(text)
  return lines.join('\n')
}
