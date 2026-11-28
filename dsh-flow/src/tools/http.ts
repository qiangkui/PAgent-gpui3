/** `http`: one GET or one bounded POST, so the assistant can read a page or an API. */

import type { ToolResult, ToolScope } from './types.ts'
import {
  HTTP_TIMEOUT_MS, MAX_HTTP_BODY, MAX_HTTP_BYTES, MAX_HTTP_CHARS, MAX_HTTP_HEADERS,
  MAX_HTTP_REDIRECTS, MAX_HTTP_SAVE_BYTES, MAX_HTTP_TIMEOUT_MS
} from './limits.ts'
import { intArg, resolveScoped } from './paths.ts'
import { decodeOutput } from './text.ts'
import { promises as fs } from 'node:fs'
import path from 'node:path'

/**
 * Headers the tool refuses to send.
 *
 * A key typed into `headers` would land in the transcript and in the provider's
 * logs, and no card can show it without doing the same. Credentials go through
 * `auth` instead, which names an environment variable and never reveals its
 * value.
 */
const BLOCKED_HEADERS = new Set(['authorization', 'cookie', 'proxy-authorization'])

/** The scheme an `auth` value is written with. */
type AuthScheme = 'bearer' | 'basic' | 'raw'

/** One request's credential, resolved from the environment at call time. */
interface HttpAuth {
  /** Header name to send. */
  header: string
  /** Multi-line-free header value, built from the variable. */
  value: string
  /** Environment variable the value came from, for the answer. */
  env: string
}

/** Most request headers one call may set. */
const MAX_REQUEST_HEADERS = 20

/** Longest single request header value kept. */
const MAX_HEADER_VALUE = 2_000

/** Methods the tool speaks. */
type HttpMethod = 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE' | 'HEAD'

/** The methods that carry a request body, so a POST body is refused on a GET. */
const WITH_BODY = new Set<HttpMethod>(['POST', 'PUT', 'PATCH'])

/** Every method the tool accepts. */
const METHODS = new Set<HttpMethod>(['GET', 'POST', 'PUT', 'PATCH', 'DELETE', 'HEAD'])

/** Status codes that mean "look at `location` instead". */
const REDIRECT_STATUS = new Set([301, 302, 303, 307, 308])

/**
 * Run one request.
 *
 * Redirects are followed up to {@link MAX_HTTP_REDIRECTS} hops, and only to
 * another `http` / `https` address without credentials in it: a documentation
 * URL that answers `301` would otherwise reach the model as an empty page. An
 * HTML body is turned into its text, because tags are noise in a transcript.
 * @param args - url, method, headers, body and timeoutMs.
 * @returns the status line, the redirect chain, and the body clipped to a readable size.
 */
export async function runHttp(
  workDir: string,
  scopeRoot: ToolScope,
  args: Record<string, unknown>
): Promise<ToolResult> {
  const url = urlOf(args)
  let method = methodOf(args)
  let headers = headersOf(args)
  let body = bodyOf(args, method)
  const auth = authOf(args)
  // The credential is added after the caller's headers, so a call cannot smuggle
  // one through `headers` and then also name a variable.
  if (auth !== null) headers = { ...headers, [auth.header]: auth.value }
  let credential = auth !== null
  let droppedAt = ''
  if (args.saveTo !== undefined && typeof args.saveTo !== 'string') {
    throw new Error('参数 saveTo 不是字符串')
  }
  const saveTo = typeof args.saveTo === 'string' ? args.saveTo.trim() : ''
  // The target is resolved before the request goes out: a path outside the
  // workspace must fail now, not after the bytes were downloaded.
  const target = saveTo === '' ? null : await resolveScoped(workDir, scopeRoot, saveTo)
  const asked = intArg(args, 'timeoutMs') ?? HTTP_TIMEOUT_MS
  const timeoutMs = Math.min(Math.max(asked, 100), MAX_HTTP_TIMEOUT_MS)
  const controller = new AbortController()
  const timer = setTimeout(() => controller.abort(), timeoutMs)
  let response: Response
  let current = url
  const hops: string[] = []
  try {
    for (let hop = 0; ; hop += 1) {
      response = await send(current, method, headers, body, controller.signal, timeoutMs)
      if (!REDIRECT_STATUS.has(response.status)) break
      const location = response.headers.get('location')
      // A 3xx without a target is the answer, not a step towards one.
      if (location === null || location.trim() === '') break
      const next = redirectTarget(current, location)
      // The body of a hop is never shown; cancelling it releases the socket.
      await response.body?.cancel().catch(() => undefined)
      if (hop >= MAX_HTTP_REDIRECTS) {
        throw new Error(
          `重定向超过 ${MAX_HTTP_REDIRECTS} 次（最后一次指向 ${next}），可能是循环；请直接请求最终地址`
        )
      }
      hops.push(`${response.status} → ${next}`)
      // A credential follows only where it is still the same host: a redirect to
      // another site (or down to plain http) must not hand the key over.
      if (credential && auth !== null && !keepsCredential(current, next)) {
        credential = false
        droppedAt = next
        headers = withoutHeader(headers, auth.header)
      }
      // 303 switches to GET, and so do 301 / 302 on a POST: the offer went
      // through and the reply is a page. 307 / 308 keep the method and body.
      if (
        method !== 'GET' &&
        (response.status === 303 || response.status === 301 || response.status === 302)
      ) {
        method = 'GET'
        body = undefined
        headers = withoutBodyHeaders(headers)
      }
      current = next
    }
  } catch (error) {
    if (controller.signal.aborted) throw new Error(`请求超时（${timeoutMs} ms）：${url}`)
    throw error instanceof Error ? error : new Error(String(error))
  } finally {
    clearTimeout(timer)
  }
  if (target !== null) {
    // A download is bytes, not prose: nothing is decoded, and the answer says
    // where it went so the model can `read_image` or open it next.
    const saved = await saveBody(response, target).catch((error: unknown) => {
      const message = error instanceof Error ? error.message : String(error)
      throw new Error(`保存失败：${message}（${saveTo}）`)
    })
    return {
      ok: response.status < 400,
      text: savedAnswer(method, current, response, saveTo, saved, authNoteOf(auth, credential, droppedAt))
    }
  }
  const { bytes, truncated } = await readBody(response)
  return {
    ok: response.status < 400,
    text: answer(method, current, hops, response, bytes, truncated, authNoteOf(auth, credential, droppedAt))
  }
}

/**
 * What the answer says about the credential.
 *
 * The variable is named; its value never is. A redirect that dropped it is
 * reported too, because the 401 that follows would otherwise read as a wrong key.
 * @param auth - the credential the call asked for, if any.
 * @param kept - whether the final hop still carried it.
 * @param droppedAt - the URL that made it stop following.
 * @returns one line, empty when the call sent no credential.
 */
function authNoteOf(auth: HttpAuth | null, kept: boolean, droppedAt: string): string {
  if (auth === null) return ''
  if (kept) return `# 认证：${auth.header}（来自环境变量 ${auth.env}，值不显示）`
  return `# 认证：重定向到 ${droppedAt} 后没有带上 ${auth.header}（来自环境变量 ${auth.env}）`
}

/**
 * The credential a call asked for, read from the environment.
 *
 * `auth.env` names a variable instead of carrying a value: a value here would sit
 * in the transcript, the approval card and the provider's logs. Bearer is the
 * default for `Authorization`; `basic` base64-encodes what the variable holds, so
 * `user:pass` works; every other header is sent as it is.
 * @param args - the call's arguments.
 * @returns the header to send, or null when the call named no credential.
 */
function authOf(args: Record<string, unknown>): HttpAuth | null {
  const plan = authPlan(args)
  if (plan === null) return null
  const value = process.env[plan.env] ?? ''
  if (value.trim() === '') {
    throw new Error(
      `环境变量 ${plan.env} 没有设置或是空的，这次请求没有发出。请在启动程序前设置它（或写进用户环境变量），再用同样的调用重试。`
    )
  }
  const trimmed = value.replace(/[\r\n]+$/, '')
  if (/[\r\n]/.test(trimmed)) throw new Error(`环境变量 ${plan.env} 的值里不能有换行`)
  const written =
    plan.scheme === 'bearer'
      ? `Bearer ${trimmed.trim()}`
      : plan.scheme === 'basic'
        ? `Basic ${Buffer.from(trimmed, 'utf8').toString('base64')}`
        : trimmed
  if (written.length > MAX_HEADER_VALUE) {
    throw new Error(`环境变量 ${plan.env} 的值太长（上限 ${MAX_HEADER_VALUE} 字符）`)
  }
  return { header: plan.header, value: written, env: plan.env }
}

/** What an `auth` argument asked for, before the variable is read. */
interface AuthPlan {
  header: string
  env: string
  scheme: AuthScheme
}

/** Header names a credential may not be sent under. */
const RESERVED_AUTH_HEADERS = new Set([
  'host',
  'content-length',
  'transfer-encoding',
  'connection',
  'set-cookie'
])

/**
 * Read the `auth` argument without touching the environment.
 *
 * The approval card needs the header and the variable name before the request is
 * allowed, and a missing variable has to be reported by the tool (after the
 * decision) rather than as a broken card.
 * @param args - the call's arguments.
 * @returns the plan, or null when no `auth` was given.
 */
function authPlan(args: Record<string, unknown>): AuthPlan | null {
  const raw = args.auth
  if (raw === undefined || raw === null) return null
  if (typeof raw !== 'object' || Array.isArray(raw)) {
    throw new Error('参数 auth 必须是对象，例如 {"env":"GITHUB_TOKEN"}')
  }
  const spec = raw as Record<string, unknown>
  const env = typeof spec.env === 'string' ? spec.env.trim() : ''
  if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(env)) {
    throw new Error('auth.env 要写成环境变量名，例如 GITHUB_TOKEN（值不要写在这里）')
  }
  const header =
    typeof spec.header === 'string' && spec.header.trim() !== ''
      ? spec.header.trim()
      : 'Authorization'
  if (!/^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/.test(header)) {
    throw new Error(`auth.header 不是合法的请求头名字：${header}`)
  }
  const lower = header.toLowerCase()
  if (lower === 'cookie' || RESERVED_AUTH_HEADERS.has(lower)) {
    throw new Error(`auth.header 不能是 ${header}：它要么不符合这个工具的规则，要么不是用来带凭据的头`)
  }
  const asked = typeof spec.scheme === 'string' ? spec.scheme.trim().toLowerCase() : ''
  if (asked !== '' && asked !== 'bearer' && asked !== 'basic' && asked !== 'raw') {
    throw new Error('auth.scheme 只能是 bearer / basic / raw')
  }
  const scheme: AuthScheme =
    asked === 'bearer' || asked === 'basic' || asked === 'raw'
      ? asked
      : lower === 'authorization'
        ? 'bearer'
        : 'raw'
  return { header, env, scheme }
}

/**
 * Whether a credential may follow a redirect.
 *
 * Same host and port is the rule; upgrading http to https is allowed because it
 * cannot hand the key to the wire, a downgrade is not. Anything else is another
 * site and gets no credential.
 * @param from - the URL that answered with a redirect.
 * @param next - the target it points at.
 * @returns whether the header may be sent again.
 */
function keepsCredential(from: string, next: string): boolean {
  try {
    const a = new URL(from)
    const b = new URL(next)
    if (a.hostname !== b.hostname || a.port !== b.port) return false
    if (a.protocol === 'https:' && b.protocol === 'http:') return false
    return true
  } catch {
    return false
  }
}

/** Copy the headers without one of them, keeping the caller's spelling. */
function withoutHeader(headers: Record<string, string>, name: string): Record<string, string> {
  const wanted = name.trim().toLowerCase()
  const kept: Record<string, string> = {}
  for (const [key, value] of Object.entries(headers)) {
    if (key.trim().toLowerCase() === wanted) continue
    kept[key] = value
  }
  return kept
}

/** What one saved download came to. */
interface SavedBody {
  /** Bytes written to disk. */
  bytes: number
  /** True when the cap stopped the download before it ended. */
  truncated: boolean
}

/**
 * Stream a response into a file, without putting the bytes in the transcript.
 *
 * The stream is read in chunks so a large download never sits in memory; the cap
 * only guards the disk, because nothing here reaches the model.
 * @param response - the final response of the redirect chain.
 * @param target - absolute file to write, parents created.
 * @returns the byte count and whether the cap cut it short.
 */
async function saveBody(response: Response, target: string): Promise<SavedBody> {
  await fs.mkdir(path.dirname(target), { recursive: true })
  if (response.body === null) {
    await fs.writeFile(target, Buffer.alloc(0))
    return { bytes: 0, truncated: false }
  }
  const reader = response.body.getReader()
  const handle = await fs.open(target, 'w')
  let bytes = 0
  let truncated = false
  try {
    for (;;) {
      const { done, value } = await reader.read()
      if (done) break
      if (value === undefined) continue
      const room = MAX_HTTP_SAVE_BYTES - bytes
      if (room <= 0) {
        truncated = true
        await reader.cancel().catch(() => undefined)
        break
      }
      const kept = value.length <= room ? value : value.subarray(0, room)
      if (kept.length > 0) await handle.write(kept)
      bytes += kept.length
      if (bytes >= MAX_HTTP_SAVE_BYTES) {
        truncated = true
        await reader.cancel().catch(() => undefined)
        break
      }
    }
  } finally {
    await handle.close().catch(() => undefined)
  }
  return { bytes, truncated }
}

/** The answer for a request whose body went to disk instead of the transcript. */
function savedAnswer(
  method: string,
  url: string,
  response: Response,
  saveTo: string,
  saved: SavedBody,
  authNote = ''
): string {
  const status = `${response.status} ${response.statusText}`.trim()
  const kind = response.headers.get('content-type') ?? '未知类型'
  const lines = [
    `${method} ${url} → ${status}`,
    `# 正文已保存到 ${saveTo}（${saved.bytes} 字节，${kind}）`
  ]
  if (authNote !== '') lines.push(authNote)
  if (saved.truncated) {
    lines.push(`# 超过 ${MAX_HTTP_SAVE_BYTES} 字节，只保存了开头`)
  }
  lines.push('# 正文没有放进对话；需要时用 read / read_image 打开这个文件')
  return lines.join('\n')
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
  if (typeof args.saveTo === 'string' && args.saveTo.trim() !== '') {
    lines.push(`# 响应会原样保存到：${args.saveTo.trim()}`)
  }
  // The card has to say a credential is going out; the variable is named, its
  // value is not, so nothing secret ever reaches the transcript.
  let plan: AuthPlan | null = null
  try {
    plan = authPlan(args)
  } catch {
    // A malformed `auth` is reported by the tool once the person has decided.
    plan = null
  }
  if (plan !== null) {
    lines.push(
      `# 会发送凭据：${plan.header} ← 环境变量 ${plan.env}（值不会显示，也不会写进对话）`
    )
  }
  lines.push(`# 会跟随最多 ${MAX_HTTP_REDIRECTS} 次 http/https 重定向（换主机后不带凭据）`)
  if (!METHODS.has(method as HttpMethod)) {
    lines.push(`# 方法只支持 ${[...METHODS].join(' / ')}，这次会失败`)
  }
  return lines.join('\n')
}

/** Send one hop, translating transport failures into readable errors. */
async function send(
  url: string,
  method: HttpMethod,
  headers: Record<string, string>,
  body: string | undefined,
  signal: AbortSignal,
  timeoutMs: number
): Promise<Response> {
  try {
    return await fetch(url, {
      method,
      headers,
      body: WITH_BODY.has(method) ? body : undefined,
      redirect: 'manual',
      signal
    })
  } catch (error) {
    if (signal.aborted) throw new Error(`请求超时（${timeoutMs} ms）：${url}`)
    const detail = error instanceof Error ? error.message : String(error)
    throw new Error(`请求失败：${detail}（${url}）`)
  }
}

/**
 * Resolve a `location` header against the URL that answered.
 * @param from - the URL of the hop.
 * @param location - the raw header value.
 * @returns the absolute target, which is always http / https.
 */
function redirectTarget(from: string, location: string): string {
  let next: URL
  try {
    next = new URL(location.trim(), from)
  } catch {
    throw new Error(`重定向目标不是合法地址：${location.trim()}`)
  }
  if (next.protocol !== 'http:' && next.protocol !== 'https:') {
    throw new Error(`重定向到不支持的协议 ${next.protocol}：${next.toString()}`)
  }
  if (next.username !== '' || next.password !== '') {
    throw new Error(`重定向目标带账号密码，不跟随：${next.toString()}`)
  }
  return next.toString()
}

/** Drop request headers that only described a body a redirect got rid of. */
function withoutBodyHeaders(headers: Record<string, string>): Record<string, string> {
  const kept: Record<string, string> = {}
  for (const [name, value] of Object.entries(headers)) {
    const key = name.trim().toLowerCase()
    if (key === 'content-type' || key === 'content-length') continue
    kept[name] = value
  }
  return kept
}

/** The request method, rejecting anything the tool does not speak. */
function methodOf(args: Record<string, unknown>): HttpMethod {
  if (args.method === undefined || args.method === null) return 'GET'
  if (typeof args.method !== 'string') throw new Error('参数 method 不是字符串')
  const method = args.method.trim().toUpperCase() as HttpMethod
  if (METHODS.has(method)) return method
  throw new Error(`method 只能是 ${[...METHODS].join(' / ')}（收到 ${method}）`)
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

/** The request body; only the methods that carry one accept it. */
function bodyOf(args: Record<string, unknown>, method: HttpMethod): string | undefined {
  if (args.body === undefined || args.body === null) return undefined
  if (typeof args.body !== 'string') throw new Error('参数 body 不是字符串')
  if (!WITH_BODY.has(method)) {
    throw new Error(`${method} 不带正文；要发正文请用 POST / PUT / PATCH`)
  }
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

/** The result text: status line, the hops taken, headers, then the clipped body. */
function answer(
  method: string,
  url: string,
  hops: string[],
  response: Response,
  bytes: Buffer,
  truncated: boolean,
  authNote = ''
): string {
  const status = `${response.status} ${response.statusText}`.trim()
  const followed = hops.length === 0 ? '' : `（跟随 ${hops.length} 次重定向）`
  const lines = [`${method} ${url} → ${status}${followed}`]
  for (const hop of hops) lines.push(`# 重定向 ${hop}`)
  if (authNote !== '') lines.push(authNote)
  const headers = [...response.headers].slice(0, MAX_HTTP_HEADERS)
  for (const [name, value] of headers) {
    lines.push(`${name}: ${value}`)
  }
  if (REDIRECT_STATUS.has(response.status)) {
    const location = response.headers.get('location') ?? '（没有 location）'
    lines.push(`# 没有继续跟随；location: ${location}`)
  }
  const raw = decodeOutput(bytes)
  if (raw.trim() === '') {
    lines.push('# 正文为空')
    return lines.join('\n')
  }
  // Tags reach the model as markup noise and eat the character budget, so an
  // HTML answer is handed over as its text.
  const html = looksLikeHtml(raw, response.headers.get('content-type') ?? '')
  const text = html ? tidy(htmlToText(raw)) : raw
  if (html) lines.push('# 正文是 HTML，已转成文本')
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

/** Whether a response body is an HTML document. */
function looksLikeHtml(body: string, contentType: string): boolean {
  if (/html/i.test(contentType)) return true
  const head = body.slice(0, 400).trimStart().toLowerCase()
  return head.startsWith('<!doctype html') || head.startsWith('<html')
}

/**
 * The readable text of an HTML document.
 *
 * Scripts, styles and comments go first — their contents are code, and a
 * `<script>` body would otherwise reach the model as a wall of JavaScript.
 * @param html - the raw document.
 * @returns the text, with block tags turned into line breaks.
 */
function htmlToText(html: string): string {
  return decodeEntities(
    html
      .replace(/<!--[\s\S]*?-->/g, ' ')
      .replace(/<(script|style|noscript|template|svg)\b[\s\S]*?<\/\1\s*>/gi, ' ')
      .replace(/<br\s*\/?>/gi, '\n')
      .replace(
        /<\/?(p|div|li|tr|h[1-6]|section|article|header|footer|pre|blockquote|table|ul|ol|dl|dt|dd|figure|figcaption|form|main|nav|aside)\b[^>]*>/gi,
        '\n'
      )
      .replace(/<[^>]+>/g, ' ')
  )
}

/** Named entities worth decoding; everything else is left as it is. */
const NAMED_ENTITIES: Record<string, string> = {
  amp: '&',
  lt: '<',
  gt: '>',
  quot: '"',
  apos: "'",
  nbsp: ' ',
  mdash: '—',
  ndash: '–',
  hellip: '…',
  lsquo: '‘',
  rsquo: '’',
  ldquo: '“',
  rdquo: '”',
  middot: '·',
  times: '×',
  copy: '©',
  reg: '®'
}

/** Decode the entities a page usually contains, decimal and hex included. */
function decodeEntities(text: string): string {
  return text.replace(/&(#x[0-9a-f]+|#[0-9]+|[a-z]+);/gi, (whole, code: string) => {
    if (/^#x/i.test(code)) {
      const point = Number.parseInt(code.slice(2), 16)
      return Number.isFinite(point) && point > 0 ? String.fromCodePoint(point) : whole
    }
    if (code.startsWith('#')) {
      const point = Number.parseInt(code.slice(1), 10)
      return Number.isFinite(point) && point > 0 ? String.fromCodePoint(point) : whole
    }
    return NAMED_ENTITIES[code.toLowerCase()] ?? whole
  })
}

/** Collapse the blank space HTML leaves behind into one line per paragraph. */
function tidy(text: string): string {
  const kept: string[] = []
  for (const line of text.split('\n')) {
    const clean = line.replace(/[^\S\n]+/g, ' ').trim()
    if (clean === '' && (kept.length === 0 || kept[kept.length - 1] === '')) continue
    kept.push(clean)
  }
  while (kept.length > 0 && kept[kept.length - 1] === '') kept.pop()
  return kept.join('\n')
}
