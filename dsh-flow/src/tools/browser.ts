/**
 * `browser`: a headless Chromium the assistant drives over DevTools.
 *
 * The engine speaks the Chrome DevTools Protocol directly to the Chromium that
 * ships with Windows (Edge) or to Chrome when it is installed. There is no
 * Playwright, no Puppeteer and no downloaded browser: the whole protocol client
 * is the ~200 lines below, using Node's own `WebSocket` and `fetch`.
 *
 * The tool exposes a small, agent-shaped surface instead of the whole protocol:
 * `open` / `goto` navigate, `snapshot` lists what a person could click with a
 * stable `[n]` reference, `click` / `type` / `press` / `select` act through real
 * input events (so the page behaves as if a person used it), `text` and `eval`
 * read, `console` and `network` report, `screenshot` hands the picture to the
 * model (a whole page, or one element by `ref` / `selector`), `downloads` and
 * `wait_download` watch the folder a page wrote into, and `close` ends the
 * session.
 *
 * Two constraints shape the code:
 *
 * - The browser cannot run inside the write sandbox: Chromium refuses a
 *   low-integrity token (`CreateProcessAsUser` fails), so it starts with a
 *   private profile under the conversation's `.dsh-flow/browser` instead. It
 *   never touches the person's own browser profile. A website may still download a
 *   file — the model needs one to hand over — but it lands in the conversation's
 *   own `.dsh-flow/downloads`, never on the desktop.
 * - Edge updates its program folder while Windows runs, and an existing binary
 *   can look missing for a moment. Starting it is therefore retried.
 */

import { spawn, execFileSync } from 'node:child_process'
import type { ChildProcess } from 'node:child_process'
import { existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from 'node:fs'
import fs from 'node:fs/promises'
import path from 'node:path'
import { bytesText, saveImageCopy } from './image.ts'
import {
  BROWSER_LAUNCH_ATTEMPTS,
  BROWSER_LAUNCH_MS,
  BROWSER_TIMEOUT_MS,
  MAX_BROWSER_EVAL,
  MAX_BROWSER_LOG,
  MAX_BROWSER_NODES,
  MAX_BROWSER_TEXT,
  MAX_IMAGE_BYTES
} from './limits.ts'
import type { ToolFile, ToolImage, ToolResult, ToolScope } from './types.ts'
import { wildcard } from './walk.ts'

/** One message on the DevTools socket. */
interface CdpMessage {
  id?: number
  method?: string
  params?: Record<string, unknown>
  result?: Record<string, unknown>
  error?: { message?: string }
  sessionId?: string
}

/** A pending command, resolved when its reply arrives. */
interface PendingCommand {
  resolve: (value: Record<string, unknown>) => void
  reject: (error: Error) => void
  timer: NodeJS.Timeout
}

/** What one snapshot lists: the reference, its role, its name. */
interface SnapshotNode {
  ref: number
  role: string
  name: string
}

/** Everything the tool keeps between calls, and between questions, of one chat. */
export interface BrowserRuntime {
  /** The process we launched; null when this request rejoined an older session. */
  child: ChildProcess | null
  socket: Cdp
  /** Session id of the attached page, used by every page command. */
  sessionId: string
  targetId: string
  /** Absolute path of the profile, kept for the conversation rather than one question. */
  profile: string
  /** The `session.json` that lets the next request rejoin this browser. */
  statePath: string
  /** DevTools socket this session talks to. */
  endpoint: string
  /** True once the socket closed: the page can no longer be talked to. */
  closed: boolean
  /** Element references the last snapshot handed out, `[n]` → DOM node id. */
  refs: Map<number, number>
  /** Console lines and exceptions, newest last. */
  console: string[]
  /** Requests by id, for the `network` report. */
  requests: Map<string, { method: string; url: string; status: string; type: string }>
  /** Loads that failed, for the `network` report. */
  failures: string[]
  /** Where the page is now. */
  url: string
  /** What the page calls itself. */
  title: string
}

/** A program that speaks the protocol, and how it was found. */
interface BrowserBinary {
  file: string
  /** Human label for the transcript: "Edge 158.0.4370.33" and the like. */
  label: string
}

/** Every install folder a Chromium may live in, most likely first. */
const CHROMIUM_ROOTS: Array<{ root: string; exe: string; label: string }> = [
  {
    root: 'C:\\Program Files (x86)\\Microsoft\\Edge\\Application',
    exe: 'msedge.exe',
    label: 'Edge'
  },
  { root: 'C:\\Program Files\\Microsoft\\Edge\\Application', exe: 'msedge.exe', label: 'Edge' },
  {
    root: 'C:\\Program Files\\Google\\Chrome\\Application',
    exe: 'chrome.exe',
    label: 'Chrome'
  },
  {
    root: 'C:\\Program Files (x86)\\Google\\Chrome\\Application',
    exe: 'chrome.exe',
    label: 'Chrome'
  }
]

/**
 * Launchers inside one install folder: the newest versioned binary first, the
 * stable entry last.
 *
 * The stable `Application\msedge.exe` is a **launcher** that hands the command
 * line to the real browser and exits. Starting that one makes the process we hold
 * disappear a moment later while Edge keeps running — the first defect this tool
 * shipped with: the session looked gone, the next call started a second browser,
 * and every later call was talking to a blank page. The versioned binary stays
 * for the whole session, so it is preferred whenever it exists.
 * @param root - the `Application` folder of one install.
 * @param exe - `msedge.exe` or `chrome.exe`.
 * @param label - what a person calls the program.
 * @returns candidates in the order they should be tried.
 */
export function candidateLaunchers(root: string, exe: string, label: string): BrowserBinary[] {
  const found: BrowserBinary[] = []
  for (const version of listVersions(root)) {
    const file = path.join(root, version, exe)
    if (existsSync(file)) found.push({ file, label: `${label} ${version}` })
  }
  const stable = path.join(root, exe)
  if (existsSync(stable)) found.push({ file: stable, label })
  return found
}

/**
 * Find a Chromium to drive.
 *
 * `DSH_BROWSER_PATH` wins, so a person can point at a portable build. Otherwise
 * each install is tried versioned-first (see {@link candidateLaunchers}), then
 * the Chromium Playwright may have downloaded for another tool.
 * @returns the program plus a label, or null when nothing was found.
 */
export function findBrowser(): BrowserBinary | null {
  const override = process.env.DSH_BROWSER_PATH?.trim()
  if (override && existsSync(override)) {
    return { file: override, label: path.basename(override).replace(/\.exe$/i, '') }
  }
  for (const install of CHROMIUM_ROOTS) {
    const candidates = candidateLaunchers(install.root, install.exe, install.label)
    const versioned = candidates.find((candidate) => candidate.label !== install.label)
    if (versioned) return versioned
    if (candidates.length > 0) return candidates[candidates.length - 1]
  }
  const cached = playwrightChromium()
  if (cached !== '') return { file: cached, label: 'Chromium (playwright)' }
  return null
}

/** Version-like folder names under `root`, newest first. */
export function listVersions(root: string): string[] {
  try {
    return readdirSync(root)
      .filter((name) => /^\d+\.\d+/.test(name) && !name.includes('deleting'))
      .sort((left, right) => right.localeCompare(left, undefined, { numeric: true }))
  } catch {
    return []
  }
}

/** A Chromium another tool downloaded, if any: last resort before giving up. */
function playwrightChromium(): string {
  const cache = process.env.LOCALAPPDATA
  if (!cache) return ''
  const root = path.join(cache, 'ms-playwright')
  try {
    for (const entry of readdirSync(root).filter((name) => name.startsWith('chromium-'))) {
      const file = path.join(root, entry, 'chrome-win', 'chrome.exe')
      if (existsSync(file)) return file
    }
  } catch {
    return ''
  }
  return ''
}

/**
 * Start the browser and wait for its debugging port.
 *
 * The launch is retried, because Edge replaces its program folder while the
 * machine runs and the binary can report as missing for a moment.
 * @param binary - the program to start.
 * @param profile - private profile directory, emptied by the caller.
 * @returns the running child and its DevTools endpoint.
 */
async function launchBrowser(
  binary: BrowserBinary,
  profile: string
): Promise<{ child: ChildProcess; endpoint: string }> {
  let last = ''
  for (let attempt = 1; attempt <= BROWSER_LAUNCH_ATTEMPTS; attempt += 1) {
    const child = spawn(
      binary.file,
      [
        '--headless=new',
        '--no-sandbox',
        '--disable-gpu',
        '--disable-dev-shm-usage',
        '--disable-extensions',
        '--disable-sync',
        '--disable-background-networking',
        '--disable-component-update',
        '--disable-default-apps',
        '--disable-client-side-phishing-detection',
        '--no-service-autorun',
        '--no-first-run',
        '--no-default-browser-check',
        '--mute-audio',
        '--hide-scrollbars',
        '--remote-debugging-address=127.0.0.1',
        '--remote-debugging-port=0',
        `--user-data-dir=${profile}`,
        'about:blank'
      ],
      { stdio: 'ignore', windowsHide: true, detached: true }
    )
    const endpoint = await waitForEndpoint(child, profile).catch((error: Error) => {
      last = error.message
      return ''
    })
    if (endpoint !== '') return { child, endpoint }
    // A failed attempt leaves nothing to reuse: kill whatever started (the real
    // browser may be a grandchild of the launcher) and retry.
    try {
      child.kill()
    } catch {
      // Already gone.
    }
    killByProfile(profile)
    if (last === '') last = '浏览器没有打开调试端口'
    if (attempt < BROWSER_LAUNCH_ATTEMPTS) await sleep(400)
  }
  throw new Error(
    `无法启动浏览器：${last}。` +
      '可以设置环境变量 DSH_BROWSER_PATH 指向 chrome.exe 或 msedge.exe。'
  )
}

/**
 * Read the port Chromium wrote, then hand back the browser WebSocket URL.
 *
 * The file is written in place, so a read can hit it mid-write (`EBUSY` on the
 * machine this was tested on) — that is a retry, not a failure. The launcher that
 * Microsoft ships exits after handing the command line over, so a dead child is
 * only fatal once the port file has had a moment to appear.
 */
async function waitForEndpoint(child: ChildProcess, profile: string): Promise<string> {
  const file = path.join(profile, 'DevToolsActivePort')
  const started = Date.now()
  const deadline = started + BROWSER_LAUNCH_MS
  while (Date.now() < deadline) {
    // Two lines: the port, then the browser target's path. The bare port is not a
    // WebSocket endpoint, so the path has to come along.
    let content = ''
    try {
      content = readFileSync(file, 'utf8')
    } catch {
      content = ''
    }
    if (content !== '') {
      const [portLine, pathLine] = content.split('\n')
      const port = portLine.trim()
      if (/^\d+$/.test(port)) {
        const route = pathLine?.trim() ?? ''
        return `ws://127.0.0.1:${port}${route.startsWith('/') ? route : ''}`
      }
    }
    // A launcher may exit after handing off; give the real browser a moment to
    // write the file before treating the exit as a failure.
    if (child.exitCode !== null && Date.now() - started > 1_500) {
      throw new Error(`浏览器启动后立即退出（退出码 ${child.exitCode}）`)
    }
    await sleep(120)
  }
  throw new Error('浏览器在 12 秒内没有打开调试端口')
}

/**
 * Stop every browser process using one profile folder.
 *
 * The process a launcher spawns is not the one doing the browsing, so
 * `child.kill()` alone can leave Edge running: it keeps the profile locked (the
 * next start then fails with `EBUSY` on `DevToolsActivePort`) and keeps showing
 * up in the task manager. Matching on the profile path — unique per session —
 * kills exactly this session's processes, never the person's own browser.
 * @param profile - the session's private profile folder.
 */
export function killByProfile(profile: string): void {
  if (process.platform !== 'win32') return
  const pattern = profile.replace(/'/g, "''")
  const script =
    "$targets = @(Get-CimInstance Win32_Process -Filter \"Name='msedge.exe' OR Name='chrome.exe'\" " +
    `| Where-Object { $_.CommandLine -like '*${pattern}*' }); ` +
    'foreach ($item in $targets) { Stop-Process -Id $item.ProcessId -Force -ErrorAction SilentlyContinue }'
  try {
    execFileSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', script], {
      windowsHide: true,
      stdio: 'ignore',
      timeout: 8_000
    })
  } catch {
    // Best effort: a browser that refuses to die is not worth failing a tool call.
  }
}

/**
 * Remove profile folders left behind by earlier questions.
 *
 * A question that ends without `close` (the round cap, Stop, a crash) leaves its
 * profile on disk: Chromium writes thousands of files into it. They are useless
 * once the process is gone, and they are all under the conversation's own
 * `.dsh-flow/browser`, so they are swept whenever a new session starts.
 * @param keep - the profile of the session just starting, left alone.
 */
export async function sweepProfiles(root: string, keep?: string): Promise<number> {
  let names: string[] = []
  try {
    names = readdirSync(root).filter((name) => name.startsWith('p-'))
  } catch {
    return 0
  }
  let removed = 0
  for (const name of names) {
    const dir = path.join(root, name)
    if (keep !== undefined && path.resolve(dir) === path.resolve(keep)) continue
    await fs.rm(dir, { recursive: true, force: true }).catch(() => undefined)
    if (!existsSync(dir)) removed += 1
  }
  await fs.rmdir(root).catch(() => undefined)
  return removed
}

/** Sleep for a moment, used by the launch loop and by polling actions. */
export function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

/**
 * The DevTools connection: one socket, numbered commands, named events.
 *
 * Flat sessions are used (`Target.attachToTarget` with `flatten: true`), so page
 * commands carry the session id and share the browser socket.
 */
export class Cdp {
  private readonly socket: WebSocket
  private nextId = 1
  private readonly pending = new Map<number, PendingCommand>()
  private readonly handlers = new Map<string, Set<(params: Record<string, unknown>) => void>>()
  private readonly closedHandlers = new Set<() => void>()
  private closed = false
  /** Session id every page command is sent with. */
  sessionId = ''

  constructor(endpoint: string) {
    this.socket = new WebSocket(endpoint)
    this.socket.addEventListener('message', (event) => this.receive(String(event.data)))
    // A socket error is always followed by a close, which fails the pending
    // commands; without this listener an error event would crash the process.
    this.socket.addEventListener('error', () => undefined)
    this.socket.addEventListener('close', () => {
      this.failAll('浏览器连接已断开')
      for (const handler of this.closedHandlers) handler()
      this.closedHandlers.clear()
    })
  }

  /** Resolve once the socket is open. */
  async ready(timeoutMs = BROWSER_TIMEOUT_MS): Promise<void> {
    if (this.socket.readyState === WebSocket.OPEN) return
    await new Promise<void>((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error('连接浏览器调试端口超时')), timeoutMs)
      this.socket.addEventListener(
        'open',
        () => {
          clearTimeout(timer)
          resolve()
        },
        { once: true }
      )
      this.socket.addEventListener(
        'error',
        () => {
          clearTimeout(timer)
          reject(new Error('无法连接浏览器调试端口'))
        },
        { once: true }
      )
    })
  }

  /** Send one command and wait for its reply. */
  async send(
    method: string,
    params: Record<string, unknown> = {},
    timeoutMs = BROWSER_TIMEOUT_MS,
    targetSession?: string | null
  ): Promise<Record<string, unknown>> {
    if (this.closed) throw new Error('浏览器已经关闭')
    const id = this.nextId
    this.nextId += 1
    const message: CdpMessage = { id, method, params }
    // `null` means the browser itself rather than the attached page, which is how
    // Browser.* commands are addressed.
    const session = targetSession === undefined ? this.sessionId : (targetSession ?? '')
    if (session !== '') message.sessionId = session
    return await new Promise<Record<string, unknown>>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id)
        reject(new Error(`浏览器没有在 ${Math.round(timeoutMs / 1000)} 秒内回复 ${method}`))
      }, timeoutMs)
      this.pending.set(id, { resolve, reject, timer })
      try {
        this.socket.send(JSON.stringify(message))
      } catch (error) {
        clearTimeout(timer)
        this.pending.delete(id)
        reject(error instanceof Error ? error : new Error(String(error)))
      }
    })
  }

  /** Listen to one protocol event; the returned function stops listening. */
  on(method: string, handler: (params: Record<string, unknown>) => void): () => void {
    const set = this.handlers.get(method) ?? new Set<(params: Record<string, unknown>) => void>()
    set.add(handler)
    this.handlers.set(method, set)
    return () => {
      set.delete(handler)
    }
  }

  /** Run one callback when the socket goes away, however that happens. */
  onClose(handler: () => void): void {
    if (this.closed) {
      handler()
      return
    }
    this.closedHandlers.add(handler)
  }

  /** Close the socket; pending commands fail rather than hang. */
  close(): void {
    this.closed = true
    this.failAll('浏览器已经关闭')
    try {
      this.socket.close()
    } catch {
      // Already closed.
    }
  }

  private receive(raw: string): void {
    let message: CdpMessage
    try {
      message = JSON.parse(raw) as CdpMessage
    } catch {
      return
    }
    if (typeof message.id === 'number') {
      const pending = this.pending.get(message.id)
      if (!pending) return
      this.pending.delete(message.id)
      clearTimeout(pending.timer)
      if (message.error) pending.reject(new Error(message.error.message ?? '调试命令失败'))
      else pending.resolve(message.result ?? {})
      return
    }
    if (typeof message.method === 'string') {
      for (const handler of this.handlers.get(message.method) ?? []) {
        handler(message.params ?? {})
      }
    }
  }

  private failAll(reason: string): void {
    for (const pending of this.pending.values()) {
      clearTimeout(pending.timer)
      pending.reject(new Error(reason))
    }
    this.pending.clear()
  }
}

/** Keys a `press` call may name, with what Chromium expects for each. */
const KEY_PRESSES: Record<string, { key: string; code: string; keyCode: number; text?: string }> = {
  enter: { key: 'Enter', code: 'Enter', keyCode: 13, text: '\r' },
  tab: { key: 'Tab', code: 'Tab', keyCode: 9 },
  escape: { key: 'Escape', code: 'Escape', keyCode: 27 },
  esc: { key: 'Escape', code: 'Escape', keyCode: 27 },
  backspace: { key: 'Backspace', code: 'Backspace', keyCode: 8 },
  delete: { key: 'Delete', code: 'Delete', keyCode: 46 },
  space: { key: ' ', code: 'Space', keyCode: 32, text: ' ' },
  arrowup: { key: 'ArrowUp', code: 'ArrowUp', keyCode: 38 },
  arrowdown: { key: 'ArrowDown', code: 'ArrowDown', keyCode: 40 },
  arrowleft: { key: 'ArrowLeft', code: 'ArrowLeft', keyCode: 37 },
  arrowright: { key: 'ArrowRight', code: 'ArrowRight', keyCode: 39 },
  home: { key: 'Home', code: 'Home', keyCode: 36 },
  end: { key: 'End', code: 'End', keyCode: 35 },
  pageup: { key: 'PageUp', code: 'PageUp', keyCode: 33 },
  pagedown: { key: 'PageDown', code: 'PageDown', keyCode: 34 }
}

/**
 * Only http(s) and the blank page are navigated to.
 *
 * `file:` would let a page read the disk, and `javascript:` / `data:` are how a
 * link hides a script; both are refused with the reason instead of a card.
 * @param input - the URL the model asked for.
 * @returns the URL to navigate to.
 */
export function browserUrl(input: unknown): string {
  const text = typeof input === 'string' ? input.trim() : ''
  if (text === '') throw new Error('缺少 url')
  if (text === 'about:blank') return text
  let parsed: URL
  try {
    parsed = new URL(text)
  } catch {
    throw new Error(`不是完整 URL：${text}（要带 http:// 或 https://）`)
  }
  if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') {
    throw new Error(`browser 只打开 http / https 页面，拒绝 ${parsed.protocol}`)
  }
  return parsed.toString()
}

/** The session kept between calls inside one question. */
let runtime: BrowserRuntime | null = null

/**
 * Set when a session ended without `close` — a crash, the updater, a killed
 * process. The next call says so, because its page is a fresh `about:blank` and a
 * model that is not told will describe the wrong page.
 */
let lostSession = false

/** Take the "the browser had to be restarted" note, once. */
function takeLostNote(): string {
  if (!lostSession) return ''
  lostSession = false
  return '（上一次的浏览器已经断开，这一问重新启动了一个：页面回到 about:blank，需要重新 open / goto。）\n'
}

/** Set when this request rejoined a browser an earlier question started. */
let resumedSession = false

/**
 * Take the "this page is the one from before" note, once.
 *
 * The model should know the page carried over: it may be logged in, scrolled,
 * or half-filled, and a model that assumes a fresh page wastes a call.
 */
function takeResumedNote(): string {
  if (!resumedSession) return ''
  resumedSession = false
  return '（已接上上一问打开的浏览器：页面与登录状态都还在，用 snapshot / text 看现在的样子；要换页用 goto。）\n'
}

/** True once a browser is running and its socket is still open. */
export function browserActive(): boolean {
  return runtime !== null && !runtime.closed
}

/** One browser session as written to disk, so the next request can rejoin it. */
interface SavedSession {
  /** Browser-level DevTools socket. */
  endpoint: string
  /** Profile folder, kept across questions so a login survives. */
  profile: string
  /** The page target the question was last looking at. */
  pageTarget: string
  /** Milliseconds since the epoch, for the idle timeout. */
  lastUsedAt: number
}

/** The file inside a conversation that remembers its running browser. */
const SESSION_FILE = 'session.json'

/**
 * How long an unused browser is kept.
 *
 * A browser that outlives one question is what makes「下一问还能看到这个页面」work; it
 * must not live forever either, so a session untouched for this long is replaced
 * on the next question. `DSH_BROWSER_IDLE_MIN` changes it (minutes).
 */
export function browserIdleMs(): number {
  const minutes = Number(process.env.DSH_BROWSER_IDLE_MIN ?? '30')
  const safe = Number.isFinite(minutes) && minutes > 0 ? minutes : 30
  return safe * 60_000
}

/** Where one conversation keeps its browser: profile, state file, old profiles. */
export function browserDir(workDir: string): string {
  return path.join(workDir, '.dsh-flow', 'browser')
}

/**
 * Where the browser writes files a site hands it.
 *
 * Inside the conversation rather than on the desktop: a download is the model's
 * material, and it only reaches the reader when the model calls `show`.
 * @param workDir - the conversation folder.
 * @returns the folder, which is created when the browser starts.
 */
export function downloadsDir(workDir: string): string {
  return path.join(workDir, '.dsh-flow', 'downloads')
}

/**
 * Read the remembered session.
 * @param dir - the conversation's `browser` folder.
 * @returns the session, or null when there is none or it cannot be parsed.
 */
export function readSavedSession(dir: string): SavedSession | null {
  try {
    const text = readFileSync(path.join(dir, SESSION_FILE), 'utf8')
    const parsed = JSON.parse(text) as Partial<SavedSession>
    if (
      typeof parsed.endpoint !== 'string' ||
      parsed.endpoint === '' ||
      typeof parsed.profile !== 'string' ||
      parsed.profile === '' ||
      typeof parsed.lastUsedAt !== 'number'
    ) {
      return null
    }
    return {
      endpoint: parsed.endpoint,
      profile: parsed.profile,
      pageTarget: typeof parsed.pageTarget === 'string' ? parsed.pageTarget : '',
      lastUsedAt: parsed.lastUsedAt
    }
  } catch {
    return null
  }
}

/**
 * Write the pointer the next request reads.
 *
 * Synchronous on purpose: the process may call it from its exit handler, where
 * an await would never run.
 * @param dir - the conversation's `browser` folder.
 * @param saved - what to remember.
 */
export function writeSavedSession(dir: string, saved: SavedSession): void {
  try {
    mkdirSync(dir, { recursive: true })
    writeFileSync(path.join(dir, SESSION_FILE), JSON.stringify(saved), 'utf8')
  } catch {
    // Losing the pointer only costs one browser restart.
  }
}

/** Forget the pointer, because the browser behind it is gone or being replaced. */
export async function clearSavedSession(dir: string): Promise<void> {
  await fs.rm(path.join(dir, SESSION_FILE), { force: true }).catch(() => undefined)
}

/**
 * Enable the domains every action needs, and point downloads at `downloadDir`.
 *
 * Downloads are allowed because the model needs the files a site hands it — an
 * invoice, a CSV, a PDF — but they are written into the conversation's own folder,
 * never the person's desktop, and the browser names the file it saved.
 * @param cdp - the browser socket.
 * @param downloadDir - where a download lands; created here.
 */
async function enablePage(cdp: Cdp, downloadDir: string): Promise<void> {
  await fs.mkdir(downloadDir, { recursive: true }).catch(() => undefined)
  await cdp.send('Page.enable')
  await cdp.send('Runtime.enable')
  await cdp.send('Log.enable')
  await cdp.send('DOM.enable')
  await cdp.send('Network.enable', { maxTotalBufferSize: 20_000_000 })
  await cdp.send(
    'Browser.setDownloadBehavior',
    { behavior: 'allow', downloadPath: downloadDir },
    BROWSER_TIMEOUT_MS,
    null
  )
}

/**
 * Rejoin a browser another request started.
 *
 * The engine runs one process per question, so the only way a page survives into
 * the next question is to leave the browser running and remember where its
 * DevTools socket is. Attaching also re-attaches to the same page, so a page the
 * model had scrolled or logged into is still there.
 * @param workDir - the conversation folder.
 * @param dir - the conversation's `browser` folder.
 * @param saved - what the previous request wrote.
 * @returns the session, or null when the browser is gone.
 */
async function attachToSaved(
  workDir: string,
  dir: string,
  saved: SavedSession
): Promise<BrowserRuntime | null> {
  const cdp = new Cdp(saved.endpoint)
  try {
    await cdp.ready(4_000)
    const listed = await cdp.send('Target.getTargets', {}, 5_000, null)
    const targets = Array.isArray(listed.targetInfos) ? listed.targetInfos : []
    const pages = targets.filter((item) => (item as Record<string, unknown>).type === 'page')
    const same = pages.find(
      (item) => String((item as Record<string, unknown>).targetId) === saved.pageTarget
    )
    const chosen = same ?? pages[0]
    let targetId = chosen === undefined ? '' : String((chosen as Record<string, unknown>).targetId)
    if (targetId === '') {
      const created = await cdp.send('Target.createTarget', { url: 'about:blank' }, 10_000)
      targetId = String(created.targetId ?? '')
      if (targetId === '') throw new Error('浏览器没有可用的页面')
    }
    const attached = await cdp.send('Target.attachToTarget', { targetId, flatten: true })
    cdp.sessionId = String(attached.sessionId ?? '')
    if (cdp.sessionId === '') throw new Error('浏览器没有交出页面会话')
    await enablePage(cdp, downloadsDir(workDir))
    const session: BrowserRuntime = {
      child: null,
      socket: cdp,
      sessionId: cdp.sessionId,
      targetId,
      endpoint: saved.endpoint,
      profile: saved.profile,
      statePath: path.join(dir, SESSION_FILE),
      closed: false,
      refs: new Map(),
      console: [],
      requests: new Map(),
      failures: [],
      url: '',
      title: ''
    }
    collectSessionEvents(cdp, session)
    cdp.onClose(() => {
      session.closed = true
      if (runtime === session) lostSession = true
    })
    // What the page is now; the model reads this before deciding what to do.
    await pageInfo(session).catch(() => undefined)
    return session
  } catch {
    cdp.close()
    return null
  }
}

/**
 * Start the browser, or hand back the one already running.
 *
 * The profile is private to this question and lives under the conversation's own
 * `.dsh-flow/browser`, so nothing the person has open in Edge is visible here and
 * nothing this session writes outlives it.
 * @param workDir - the conversation folder.
 * @returns the running session.
 */
async function ensureBrowser(workDir: string): Promise<BrowserRuntime> {
  // The page lives in the socket, not in the process we launched: Microsoft's
  // launcher exits after handing the command line to the real browser, and tying
  // the session to that process made the next call start a second browser while
  // every later call talked to its blank page.
  if (runtime !== null && !runtime.closed) return runtime
  if (runtime !== null) await closeBrowser(true)
  const dir = browserDir(workDir)
  // A conversation keeps one browser across its questions, so a page a previous
  // question logged into is still there when the next question asks about it. The
  // engine lives for one question, so the pointer lives in session.json.
  const saved = readSavedSession(dir)
  if (saved !== null) {
    if (Date.now() - saved.lastUsedAt > browserIdleMs()) {
      killByProfile(saved.profile)
      await clearSavedSession(dir)
    } else {
      const attached = await attachToSaved(workDir, dir, saved)
      if (attached !== null) {
        runtime = attached
        resumedSession = true
        writeSavedSession(dir, {
          endpoint: attached.endpoint,
          profile: attached.profile,
          pageTarget: attached.targetId,
          lastUsedAt: Date.now()
        })
        return attached
      }
      // The pointer is stale (the browser died, the machine slept): drop it and
      // start a fresh one below.
      killByProfile(saved.profile)
      await clearSavedSession(dir)
    }
  }
  const binary = findBrowser()
  if (binary === null) {
    throw new Error(
      '没有找到可用的浏览器：browser 需要本机安装 Edge 或 Chrome。' +
        '也可以设置环境变量 DSH_BROWSER_PATH 指向 chrome.exe / msedge.exe。'
    )
  }
  // One profile per conversation, not per question: cookies and logins survive
  // until `close` (or the idle timeout) throws the browser away.
  const profile = path.join(dir, 'profile')
  await fs.mkdir(profile, { recursive: true }).catch(() => undefined)
  // Profiles from the old per-question layout are useless now, and a stale
  // DevToolsActivePort from the browser we just killed would be read as this
  // launch's endpoint — the connection then fails against a dead port.
  await sweepProfiles(dir, profile).catch(() => 0)
  await fs.rm(path.join(profile, 'DevToolsActivePort'), { force: true }).catch(() => undefined)
  const { child, endpoint } = await launchBrowser(binary, profile)
  const cdp = new Cdp(endpoint)
  let targetId = ''
  try {
    await cdp.ready()
    const created = await cdp.send('Target.createTarget', { url: 'about:blank' })
    targetId = String(created.targetId ?? '')
    if (targetId === '') throw new Error('浏览器没有创建页面')
    const attached = await cdp.send('Target.attachToTarget', { targetId, flatten: true })
    cdp.sessionId = String(attached.sessionId ?? '')
    if (cdp.sessionId === '') throw new Error('浏览器没有交出页面会话')
    await enablePage(cdp, downloadsDir(workDir))
  } catch (error) {
    cdp.close()
    try {
      child.kill()
    } catch {
      // Already gone.
    }
    throw error instanceof Error ? error : new Error(String(error))
  }
  const session: BrowserRuntime = {
    child,
    endpoint,
    socket: cdp,
    sessionId: cdp.sessionId,
    targetId,
    profile,
    statePath: path.join(dir, SESSION_FILE),
    closed: false,
    refs: new Map(),
    console: [],
    requests: new Map(),
    failures: [],
    url: 'about:blank',
    title: ''
  }
  collectSessionEvents(cdp, session)
  // A dead socket is the only thing that ends a session; the launcher exiting
  // does not, because the browser it started keeps running.
  cdp.onClose(() => {
    session.closed = true
    if (runtime === session) lostSession = true
  })
  // The browser outlives this process — the next question rejoins it — so no exit
  // handler kills it. The handler only leaves a fresh timestamp for the idle
  // timeout, and the child is detached so it does not die with the engine.
  child.unref()
  const pointer: SavedSession = {
    endpoint,
    profile,
    pageTarget: targetId,
    lastUsedAt: Date.now()
  }
  writeSavedSession(dir, pointer)
  process.once('exit', () => {
    writeSavedSession(dir, { ...pointer, lastUsedAt: Date.now() })
  })
  runtime = session
  return session
}


/** Text of one `console.*` argument, as the page logged it. */
function argumentText(value: unknown): string {
  if (value === null || typeof value !== 'object') return String(value)
  const record = value as Record<string, unknown>
  if (typeof record.value === 'string') return record.value
  if (typeof record.value === 'number' || typeof record.value === 'boolean') return String(record.value)
  if (typeof record.description === 'string') return record.description
  const preview = record.preview as Record<string, unknown> | undefined
  const properties = Array.isArray(preview?.properties) ? preview.properties : []
  if (properties.length === 0) {
    return typeof record.className === 'string' ? record.className : '[object]'
  }
  const pairs = properties.map((item) => {
    const property = item as Record<string, unknown>
    const name = String(property.name)
    const value = property.value
    return `${name}: ${typeof value === 'string' || typeof value === 'number' || typeof value === 'boolean' ? String(value) : '…'}`
  })
  return `{ ${pairs.join(', ')} }`
}

/** Keep a bounded list: the newest lines are the interesting ones. */
function pushLine(list: string[], line: string): void {
  const text = line.trim()
  if (text === '') return
  list.push(text)
  if (list.length > MAX_BROWSER_LOG * 2) list.splice(0, list.length - MAX_BROWSER_LOG)
}

/** A URL shortened for a report line. */
export function shortUrl(url: string, limit = 90): string {
  if (url.length <= limit) return url
  return `${url.slice(0, limit - 1)}…`
}

/**
 * Collect what the page says and loads.
 *
 * Console output, exceptions, network traffic and navigation all arrive as
 * events; keeping them here is what `console` / `network` report afterwards.
 */
function collectSessionEvents(cdp: Cdp, session: BrowserRuntime): void {
  cdp.on('Runtime.consoleAPICalled', (params) => {
    const type = typeof params.type === 'string' ? params.type : 'log'
    const args = Array.isArray(params.args) ? params.args : []
    pushLine(session.console, `${type}: ${args.map(argumentText).join(' ')}`)
  })
  cdp.on('Runtime.exceptionThrown', (params) => {
    const details = params.exceptionDetails as Record<string, unknown> | undefined
    const exception = details?.exception as Record<string, unknown> | undefined
    const text =
      typeof exception?.description === 'string'
        ? exception.description
        : typeof details?.text === 'string'
          ? details.text
          : '脚本异常'
    pushLine(session.console, `exception: ${text.split('\n')[0]}`)
  })
  cdp.on('Log.entryAdded', (params) => {
    const entry = params.entry as Record<string, unknown> | undefined
    if (!entry) return
    const level = typeof entry.level === 'string' ? entry.level : 'info'
    const text = typeof entry.text === 'string' ? entry.text : ''
    pushLine(session.console, `${level}: ${text}`)
  })
  cdp.on('Network.requestWillBeSent', (params) => {
    const id = typeof params.requestId === 'string' ? params.requestId : ''
    const request = params.request as Record<string, unknown> | undefined
    if (id === '' || !request) return
    session.requests.set(id, {
      method: typeof request.method === 'string' ? request.method : 'GET',
      url: typeof request.url === 'string' ? request.url : '',
      status: '',
      type: typeof params.type === 'string' ? params.type : ''
    })
    if (session.requests.size > 400) {
      const oldest = session.requests.keys().next().value
      if (oldest !== undefined) session.requests.delete(oldest)
    }
  })
  cdp.on('Network.responseReceived', (params) => {
    const id = typeof params.requestId === 'string' ? params.requestId : ''
    const entry = session.requests.get(id)
    if (!entry) return
    const response = params.response as Record<string, unknown> | undefined
    entry.status = String(response?.status ?? '')
  })
  cdp.on('Network.loadingFailed', (params) => {
    const id = typeof params.requestId === 'string' ? params.requestId : ''
    const entry = session.requests.get(id)
    const why = typeof params.errorText === 'string' ? params.errorText : '加载失败'
    pushLine(
      session.failures,
      entry === undefined ? why : `${entry.method} ${shortUrl(entry.url)} — ${why}`
    )
  })
  cdp.on('Page.frameNavigated', (params) => {
    const frame = params.frame as Record<string, unknown> | undefined
    if (!frame || frame.parentId !== undefined) return
    session.url = typeof frame.url === 'string' ? frame.url : session.url
    // References belong to the page that handed them out.
    session.refs.clear()
  })
}

/** One file in the download folder, as the folder sees it. */
interface DownloadEntry {
  /** File name on disk; a duplicate lands as `name (1).ext`. */
  name: string
  /** Size in bytes. */
  size: number
  /** Change time, in milliseconds, so a rewrite counts as new. */
  mtime: number
}

/**
 * Read the download folder.
 *
 * The folder is the only reliable report: the protocol's download events have come
 * and gone across browser versions, while a file that was handed over is simply
 * there. A missing folder is an empty list, not an error.
 * @param dir - the conversation's `downloads` folder.
 * @returns every file in it, unsorted.
 */
async function folderEntries(dir: string): Promise<DownloadEntry[]> {
  const names = await fs.readdir(dir).catch(() => [] as string[])
  const entries: DownloadEntry[] = []
  for (const name of names) {
    const info = await fs.stat(path.join(dir, name)).catch(() => null)
    if (info === null || !info.isFile()) continue
    entries.push({ name, size: info.size, mtime: info.mtimeMs })
  }
  return entries
}

/** A value that changes whenever a file is rewritten. */
function stamp(entry: DownloadEntry): string {
  return `${entry.size}:${Math.round(entry.mtime)}`
}

/**
 * Wait for a download this call started, and word the lines that name it.
 *
 * A click returns before Chrome has written the file, so the result would say
 * nothing and the model would ask `downloads` a second later. Chrome writes a
 * `.crdownload` while the bytes are still arriving, which is what this waits on;
 * the wait is bounded so a slow file is reported as unfinished instead of failing
 * the call.
 * @param dir - the conversation's `downloads` folder.
 * @param before - what the folder held when the action started.
 * @param waitMs - longest time to wait for a file still arriving.
 * @returns one line per file that is new or rewritten, unfinished ones included.
 */
async function settleDownloads(
  dir: string,
  before: Map<string, string>,
  waitMs = 8_000
): Promise<string[]> {
  const deadline = Date.now() + waitMs
  // A download started by this action shows up within a few frames; the grace
  // period is what keeps an action that downloaded nothing from paying the wait.
  const grace = Date.now() + 300
  for (;;) {
    const entries = await folderEntries(dir)
    const partial = entries.some((entry) => entry.name.toLowerCase().endsWith('.crdownload'))
    const fresh = entries.filter(
      (entry) =>
        !entry.name.toLowerCase().endsWith('.crdownload') && before.get(entry.name) !== stamp(entry)
    )
    if (!partial && (fresh.length > 0 || Date.now() >= grace)) {
      return fresh.map(
        (entry) => `刚下载了 ${entry.name}（${bytesText(entry.size)}）→ ${path.join(dir, entry.name)}`
      )
    }
    if (Date.now() >= deadline) {
      return fresh.map(
        (entry) => `下载还没完成：${entry.name}（稍后用 downloads 看结果）`
      )
    }
    await sleep(150)
  }
}

/**
 * List what the browser has downloaded into this conversation.
 *
 * The folder is the authority, not the events: a session that resumed after a
 * second question still has the first question's files there, and a duplicate name
 * lands as `name (1).ext`, which only the folder knows.
 * @param workDir - the conversation folder.
 * @returns the report the model reads, plus the same files as data (a workflow
 *   step needs paths, not prose).
 */
async function listDownloads(workDir: string): Promise<{ text: string; files: ToolFile[] }> {
  const dir = downloadsDir(workDir)
  const files = (await folderEntries(dir)).filter(
    // A `.crdownload` is a file still being written; listing it would look like a
    // failed download the model then tries to read.
    (entry) => !entry.name.toLowerCase().endsWith('.crdownload')
  )
  files.sort((left, right) => right.mtime - left.mtime)
  if (files.length === 0) {
    return {
      text: `下载目录里还没有文件：${dir}\n（在网页上点下载链接，或让页面触发下载，文件会落在这里。）`,
      files: []
    }
  }
  const shown = files.slice(0, 30)
  const lines = [`下载目录：${dir}`, `共 ${files.length} 个文件，最新在前：`]
  for (const file of shown) {
    lines.push(`- ${path.join(dir, file.name)}（${bytesText(file.size)}）`)
  }
  if (files.length > shown.length) lines.push(`…（还有 ${files.length - shown.length} 个）`)
  return {
    text: lines.join('\n'),
    files: shown.map((file) => ({
      name: file.name,
      path: path.join(dir, file.name),
      bytes: file.size
    }))
  }
}

/**
 * Wait for one download to appear and stop growing.
 *
 * A click that starts a download returns before Chrome has written the file, so
 * a step that needs the result cannot simply read the folder afterwards. Files
 * that were already there must not count either: `since` is a timestamp the
 * caller took before clicking, which is how a second run's
 * `使用单位导出 (1).xls` is not mistaken for the new result.
 * @param workDir - the conversation folder holding `downloads/`.
 * @param args - `name` or `pattern`, optional `since` (epoch ms), `timeoutMs`.
 * @returns the newest matching file, or why nothing arrived.
 */
async function waitDownload(workDir: string, args: Record<string, unknown>): Promise<ToolResult> {
  const dir = downloadsDir(workDir)
  const name = stringArg(args, 'name')
  const pattern = stringArg(args, 'pattern')
  const since = intArg(args, 'since')
  const timeout = clamp(intArg(args, 'timeoutMs') ?? 15_000, 500, 120_000)
  const matcher = pattern === '' ? null : wildcard(pattern)
  const wanted =
    name !== '' ? `名为 ${name} 的` : pattern !== '' ? `匹配 ${pattern} 的` : '新的'
  const deadline = Date.now() + timeout
  let last: { entry: DownloadEntry; stamp: string } | null = null
  for (;;) {
    const entries = await folderEntries(dir)
    const partial = entries.some((entry) => entry.name.toLowerCase().endsWith('.crdownload'))
    const matches = entries
      .filter((entry) => !entry.name.toLowerCase().endsWith('.crdownload'))
      .filter((entry) => since === undefined || Math.round(entry.mtime) >= since)
      .filter((entry) =>
        name !== '' ? entry.name === name : matcher === null ? true : matcher.test(entry.name)
      )
      .sort((left, right) => right.mtime - left.mtime)
    const newest = matches[0]
    if (newest !== undefined && !partial) {
      // Two reads a moment apart agree: the file is written, not still growing.
      if (last !== null && last.entry.name === newest.name && last.stamp === stamp(newest)) {
        const absolute = path.join(dir, newest.name)
        return {
          ok: true,
          text: `下载完成：${newest.name}（${bytesText(newest.size)}）→ ${absolute}`,
          files: [{ name: newest.name, path: absolute, bytes: newest.size }]
        }
      }
      last = { entry: newest, stamp: stamp(newest) }
    }
    if (Date.now() >= deadline) {
      const detail =
        newest === undefined
          ? `目录里没有${wanted}文件`
          : `文件还没写完（${newest.name}，${bytesText(newest.size)}）`
      return { ok: false, text: `等待下载超时（${timeout} ms）：${detail}。目录：${dir}` }
    }
    await sleep(250)
  }
}

/**
 * Run one expression in the page and return its value.
 * @param session - the running browser.
 * @param expression - JavaScript, evaluated in the page.
 * @param awaitPromise - wait for a returned promise to settle.
 * @returns the value, as JSON.
 */
export async function evaluate(
  session: BrowserRuntime,
  expression: string,
  awaitPromise = false
): Promise<unknown> {
  const result = await session.socket.send('Runtime.evaluate', {
    expression,
    awaitPromise,
    returnByValue: true,
    userGesture: true
  })
  const details = result.exceptionDetails as Record<string, unknown> | undefined
  if (details) {
    const exception = details.exception as Record<string, unknown> | undefined
    const text =
      typeof exception?.description === 'string'
        ? exception.description
        : typeof details.text === 'string'
          ? details.text
          : '脚本异常'
    throw new Error(text.split('\n')[0])
  }
  const remote = result.result as Record<string, unknown> | undefined
  return remote?.value
}

/** Where the page is now and what it calls itself. */
async function pageInfo(session: BrowserRuntime): Promise<{ url: string; title: string }> {
  const info = (await evaluate(session, '({ url: location.href, title: document.title })')) as
    | { url?: string; title?: string }
    | undefined
  session.url = info?.url ?? session.url
  session.title = info?.title ?? ''
  return { url: session.url, title: session.title }
}

/**
 * Navigate and wait until the page finished loading.
 *
 * `document.readyState` is polled rather than waiting for `Page.loadEventFired`:
 * that event can fire before a listener is installed, and polling also covers a
 * page that never fires it.
 * @param session - the running browser.
 * @param url - an http(s) URL already validated by {@link browserUrl}.
 * @param waitMs - extra settle time for scripts the page runs after load.
 */
export async function gotoPage(session: BrowserRuntime, url: string, waitMs = 250): Promise<void> {
  session.refs.clear()
  await session.socket.send('Page.navigate', { url })
  const deadline = Date.now() + BROWSER_TIMEOUT_MS
  while (Date.now() < deadline) {
    const state = await evaluate(session, 'document.readyState').catch(() => 'loading')
    if (state === 'complete') break
    await sleep(120)
  }
  if (waitMs > 0) await sleep(waitMs)
}

/**
 * Close a browser this process never started.
 *
 * `close` may arrive in a request whose engine has not touched the browser yet
 * (the previous question left it running). The pointer in `session.json` is then
 * the only way to shut it down, and leaving it behind would mean the next
 * question reopens a browser the person asked to close.
 * @param workDir - the conversation folder.
 * @returns whether a browser was running.
 */
export async function closeSavedBrowser(workDir: string): Promise<boolean> {
  const dir = browserDir(workDir)
  const saved = readSavedSession(dir)
  if (saved === null) return false
  const cdp = new Cdp(saved.endpoint)
  try {
    await cdp.ready(3_000)
    await cdp.send('Browser.close', {}, 3_000, null)
  } catch {
    // Already gone; the kill below covers it.
  }
  cdp.close()
  killByProfile(saved.profile)
  await clearSavedSession(dir)
  for (let attempt = 0; attempt < 10; attempt += 1) {
    await fs.rm(saved.profile, { recursive: true, force: true }).catch(() => undefined)
    if (!existsSync(saved.profile)) break
    await sleep(100)
  }
  await fs.rmdir(dir).catch(() => undefined)
  return true
}

/**
 * Let go of the socket without closing the browser.
 *
 * The browser is meant to outlive the question, but its DevTools socket would
 * keep this process — and the window reading its pipe — alive forever. Closing
 * the socket leaves the browser (and its page) running for the next question to
 * rejoin; the pointer in `session.json` stays valid.
 * @returns whether there was a session to detach from.
 */
export async function detachBrowser(): Promise<boolean> {
  const session = runtime
  if (session === null) return false
  runtime = null
  resumedSession = false
  writeSavedSession(path.dirname(session.statePath), {
    endpoint: session.endpoint,
    profile: session.profile,
    pageTarget: session.targetId,
    lastUsedAt: Date.now()
  })
  // `session.closed` stays false while closing: this is not a crash, and the
  // next process must not be told the browser was lost.
  session.socket.close()
  return true
}

/**
 * Close the browser.
 *
 * `keepProfile` is used when only the socket died: the login state is still worth
 * keeping, so the profile stays for the restart the next action will do. A plain
 * `close` is a reset: the profile and the pointer both go.
 * @param keepProfile - leave the profile folder on disk.
 * @returns whether there was a session to close.
 */
export async function closeBrowser(keepProfile = false): Promise<boolean> {
  const session = runtime
  if (session === null) return false
  runtime = null
  resumedSession = false
  session.closed = true
  try {
    await session.socket.send('Browser.close', {}, 3_000, null)
  } catch {
    // The socket may already be gone; the kill below covers it.
  }
  session.socket.close()
  try {
    session.child?.kill()
  } catch {
    // Already gone.
  }
  // The launcher may have handed off to a process we never held, so the profile
  // path is what identifies this session's browser.
  killByProfile(session.profile)
  await clearSavedSession(path.dirname(session.statePath))
  if (!keepProfile) {
    for (let attempt = 0; attempt < 10; attempt += 1) {
      await fs.rm(session.profile, { recursive: true, force: true }).catch(() => undefined)
      if (!existsSync(session.profile)) break
      await sleep(100)
    }
    // The folder only holds this profile; drop it when it is empty so a
    // conversation keeps no stray directory after an explicit `close`.
    await fs.rmdir(path.dirname(session.profile)).catch(() => undefined)
  }
  return true
}


/** Roles a person can act on, so they get a `[n]` reference. */
const INTERACTIVE_ROLES = new Set([
  'button',
  'link',
  'textbox',
  'searchbox',
  'checkbox',
  'radio',
  'combobox',
  'listbox',
  'option',
  'slider',
  'switch',
  'tab',
  'menuitem',
  'menuitemcheckbox',
  'menuitemradio',
  'treeitem',
  'spinbutton',
  'disclosuretriangle'
])

/** Roles that only repeat body text, which the `text` action already sends. */
const QUIET_ROLES = new Set(['StaticText', 'InlineTextBox', 'generic', 'none', 'presentation'])

/** One accessibility node, as the protocol hands it over. */
function axText(node: Record<string, unknown>, key: string): string {
  const value = node[key] as Record<string, unknown> | undefined
  const text = value?.value
  return typeof text === 'string' ? text.trim() : ''
}

/** One named property of an accessibility node, as a string. */
function axProperty(node: Record<string, unknown>, name: string): string {
  const list = Array.isArray(node.properties) ? node.properties : []
  for (const item of list) {
    const property = item as Record<string, unknown>
    if (property.name !== name) continue
    const value = property.value as Record<string, unknown> | undefined
    const inner = value?.value
    if (typeof inner === 'string' || typeof inner === 'number' || typeof inner === 'boolean') {
      return String(inner)
    }
  }
  return ''
}

/** Cut a name to one line and a readable length. */
export function clip(text: string, limit: number): string {
  const single = text.replace(/\s+/g, ' ').trim()
  return single.length <= limit ? single : `${single.slice(0, limit - 1)}…`
}

/** What the node currently holds: a textbox's value, a checkbox's state. */
function axState(node: Record<string, unknown>): string {
  const value = axText(node, 'value')
  const checked = axProperty(node, 'checked')
  const parts: string[] = []
  if (value !== '') parts.push(`值="${clip(value, 60)}"`)
  if (checked !== '') parts.push(`checked=${checked}`)
  const disabled = axProperty(node, 'disabled')
  if (disabled === 'true') parts.push('禁用')
  return parts.length === 0 ? '' : ` ${parts.join(' ')}`
}

/**
 * List what a person could act on, with stable references.
 *
 * References are handed out per snapshot and cleared by navigation, so `[3]` in
 * a later call can never quietly mean a different element.
 * @param session - the running browser.
 * @returns the listing plus how many rows it has.
 */
export async function snapshot(session: BrowserRuntime): Promise<{ text: string; rows: number }> {
  const result = await session.socket.send('Accessibility.getFullAXTree', {})
  const nodes = Array.isArray(result.nodes) ? result.nodes : []
  session.refs.clear()
  const lines: string[] = []
  let ref = 0
  for (const raw of nodes) {
    if (lines.length >= MAX_BROWSER_NODES) break
    const node = (raw ?? {}) as Record<string, unknown>
    if (node.ignored === true) continue
    const role = axText(node, 'role')
    if (role === '' || QUIET_ROLES.has(role)) continue
    const name = axText(node, 'name')
    const backend = typeof node.backendDOMNodeId === 'number' ? node.backendDOMNodeId : undefined
    const interactive = INTERACTIVE_ROLES.has(role)
    if (!interactive && name === '') continue
    if (interactive && backend !== undefined) {
      ref += 1
      session.refs.set(ref, backend)
      lines.push(`[${ref}] ${role} "${clip(name, 120)}"${axState(node)}`)
    } else {
      lines.push(`${role} "${clip(name, 120)}"`)
    }
  }
  return { text: lines.join('\n'), rows: lines.length }
}

/**
 * Turn one `[n]` reference into something the protocol can act on.
 * @param session - the running browser.
 * @param ref - the number a snapshot handed out.
 * @returns the page object and its DOM node id.
 */
export async function resolveNode(
  session: BrowserRuntime,
  ref: number
): Promise<{ objectId: string; backendNodeId: number }> {
  const backendNodeId = session.refs.get(ref)
  if (backendNodeId === undefined) {
    throw new Error(`没有编号 [${ref}]：先调用 snapshot，用最新一次列出的编号`)
  }
  const resolved = await session.socket.send('DOM.resolveNode', { backendNodeId })
  const object = resolved.object as Record<string, unknown> | undefined
  const objectId = typeof object?.objectId === 'string' ? object.objectId : ''
  if (objectId === '') {
    throw new Error(`编号 [${ref}] 已经失效（页面可能变了）：重新 snapshot 再试`)
  }
  return { objectId, backendNodeId }
}

/** Centre of an element found by CSS selector, after scrolling it into view. */
export async function selectorPoint(
  session: BrowserRuntime,
  selector: string
): Promise<{ x: number; y: number }> {
  const spot = (await evaluate(
    session,
    `(() => {
      const el = document.querySelector(${JSON.stringify(selector)});
      if (!el) return null;
      el.scrollIntoView({ block: 'center', inline: 'center' });
      const box = el.getBoundingClientRect();
      return { x: box.left + box.width / 2, y: box.top + box.height / 2 };
    })()`
  )) as { x?: number; y?: number } | null | undefined
  if (!spot || typeof spot.x !== 'number' || typeof spot.y !== 'number') {
    throw new Error(`找不到元素：${selector}`)
  }
  return { x: spot.x, y: spot.y }
}
/**
 * Border box of an element found by CSS selector, in page coordinates.
 *
 * `screenshot` needs a box rather than a point: a captcha is a picture of one
 * element, not of whatever viewport it happens to sit in. The element is
 * scrolled into view first, so a capture beyond the viewport still has it laid
 * out where the box says it is.
 * @param session - the running browser.
 * @param selector - CSS selector.
 * @returns the box, ready to use as a capture clip.
 */
async function selectorBox(
  session: BrowserRuntime,
  selector: string
): Promise<{ x: number; y: number; width: number; height: number }> {
  const found = await session.socket.send('Runtime.evaluate', {
    expression: `(() => {
      const el = document.querySelector(${JSON.stringify(selector)});
      if (!el) return null;
      el.scrollIntoView({ block: 'center', inline: 'center' });
      return el;
    })()`,
    objectGroup: 'shot'
  })
  try {
    const object = found.result as Record<string, unknown> | undefined
    const objectId = typeof object?.objectId === 'string' ? object.objectId : ''
    if (objectId === '') throw new Error(`找不到元素：${selector}`)
    const model = await session.socket.send('DOM.getBoxModel', { objectId })
    const box = (model.model ?? {}) as Record<string, unknown>
    const quad = Array.isArray(box.border) ? (box.border as number[]) : []
    if (quad.length < 8) throw new Error(`元素 ${selector} 没有可截图的大小（可能不可见）`)
    const xs = [quad[0], quad[2], quad[4], quad[6]]
    const ys = [quad[1], quad[3], quad[5], quad[7]]
    // `DOM.getBoxModel` reports viewport-relative coordinates here, while
    // `Page.captureScreenshot`'s clip is in page coordinates. Without the
    // scroll offset an element below the fold is captured as whatever happens
    // to sit at the same viewport position — a blank slice of the page.
    const scrolled = (await evaluate(
      session,
      '({ x: window.scrollX, y: window.scrollY })'
    )) as { x?: number; y?: number } | undefined
    const x = Math.min(...xs) + (scrolled?.x ?? 0)
    const y = Math.min(...ys) + (scrolled?.y ?? 0)
    const width = Math.max(...xs) - Math.min(...xs)
    const height = Math.max(...ys) - Math.min(...ys)
    if (width <= 0 || height <= 0) throw new Error(`元素 ${selector} 没有可截图的大小（可能不可见）`)
    return { x, y, width, height }
  } finally {
    await session.socket.send('Runtime.releaseObjectGroup', { objectGroup: 'shot' }).catch(
      () => undefined
    )
  }
}

/** The viewport a page is shown at, unless the call asks for another size. */
async function setViewport(
  session: BrowserRuntime,
  width: number | undefined,
  height: number | undefined
): Promise<void> {
  if (width === undefined && height === undefined) return
  await session.socket.send('Emulation.setDeviceMetricsOverride', {
    width: width ?? 1280,
    height: height ?? 800,
    deviceScaleFactor: 1,
    mobile: false
  })
}

/** Where to click: a `[n]` reference or a CSS selector, in CSS pixels. */
async function hitPoint(session: BrowserRuntime, args: Record<string, unknown>): Promise<{ x: number; y: number }> {
  const ref = intArg(args, 'ref')
  if (ref !== undefined) {
    const node = await resolveNode(session, ref)
    await session.socket.send('DOM.scrollIntoViewIfNeeded', { objectId: node.objectId }).catch(
      () => undefined
    )
    const model = await session.socket.send('DOM.getBoxModel', { objectId: node.objectId })
    const box = model.model as Record<string, unknown> | undefined
    const quad = Array.isArray(box?.border) ? (box?.border as number[]) : []
    if (quad.length < 8) throw new Error(`编号 [${ref}] 没有可点击的位置（元素可能不可见）`)
    return {
      x: (quad[0] + quad[2] + quad[4] + quad[6]) / 4,
      y: (quad[1] + quad[3] + quad[5] + quad[7]) / 4
    }
  }
  const selector = stringArg(args, 'selector')
  if (selector === '') throw new Error('需要 ref（snapshot 的编号）或 selector')
  return await selectorPoint(session, selector)
}

/** Press and release one mouse button at a point. */
async function clickAt(
  session: BrowserRuntime,
  point: { x: number; y: number },
  button: string,
  clickCount: number
): Promise<void> {
  await session.socket.send('Input.dispatchMouseEvent', {
    type: 'mouseMoved',
    x: point.x,
    y: point.y,
    button: 'none'
  })
  for (let count = 1; count <= clickCount; count += 1) {
    await session.socket.send('Input.dispatchMouseEvent', {
      type: 'mousePressed',
      x: point.x,
      y: point.y,
      button,
      clickCount: count
    })
    await session.socket.send('Input.dispatchMouseEvent', {
      type: 'mouseReleased',
      x: point.x,
      y: point.y,
      button,
      clickCount: count
    })
  }
  await sleep(120)
}

/** Focus an element and type into it, the way a person would. */
async function typeInto(
  session: BrowserRuntime,
  args: Record<string, unknown>,
  text: string
): Promise<string> {
  const ref = intArg(args, 'ref')
  if (ref !== undefined) {
    const node = await resolveNode(session, ref)
    await session.socket.send('DOM.focus', { objectId: node.objectId })
  } else {
    const selector = stringArg(args, 'selector')
    if (selector === '') throw new Error('需要 ref（snapshot 的编号）或 selector')
    await session.socket.send('Runtime.evaluate', {
      expression: `document.querySelector(${JSON.stringify(selector)})?.focus()`
    })
  }
  await session.socket.send('Input.insertText', { text })
  if (args.submit === true) {
    await pressKey(session, 'enter')
  }
  return `${text.length} 个字符`
}

/** Press one key, with the fields Chromium requires for a real key event. */
async function pressKey(session: BrowserRuntime, key: string): Promise<void> {
  const name = key.trim().toLowerCase()
  const press = KEY_PRESSES[name]
  if (!press) {
    throw new Error(`press 支持 ${Object.keys(KEY_PRESSES).map((item) => item).join(' / ')}`)
  }
  await session.socket.send('Input.dispatchKeyEvent', {
    type: 'rawKeyDown',
    key: press.key,
    code: press.code,
    windowsVirtualKeyCode: press.keyCode,
    nativeVirtualKeyCode: press.keyCode
  })
  if (press.text !== undefined) {
    await session.socket.send('Input.dispatchKeyEvent', { type: 'char', text: press.text })
  }
  await session.socket.send('Input.dispatchKeyEvent', {
    type: 'keyUp',
    key: press.key,
    code: press.code,
    windowsVirtualKeyCode: press.keyCode,
    nativeVirtualKeyCode: press.keyCode
  })
  await sleep(60)
}

/** Choose values in a `<select>`, firing the events a page listens for. */
async function selectOption(
  session: BrowserRuntime,
  args: Record<string, unknown>
): Promise<string> {
  const values = Array.isArray(args.values)
    ? args.values.map((value) => String(value))
    : typeof args.value === 'string'
      ? [args.value]
      : []
  if (values.length === 0) throw new Error('select 需要 values（一个或多个选项值）')
  const ref = intArg(args, 'ref')
  let objectId = ''
  let selector = ''
  if (ref !== undefined) {
    objectId = (await resolveNode(session, ref)).objectId
  } else {
    selector = stringArg(args, 'selector')
    if (selector === '') throw new Error('需要 ref（snapshot 的编号）或 selector')
  }
  const declaration = `function(values) {
    if (!this || this.tagName !== 'SELECT') return false;
    for (const option of Array.from(this.options)) {
      option.selected = values.includes(option.value) || values.includes(option.textContent.trim());
    }
    this.dispatchEvent(new Event('input', { bubbles: true }));
    this.dispatchEvent(new Event('change', { bubbles: true }));
    return true;
  }`
  const result = objectId
    ? await session.socket.send('Runtime.callFunctionOn', {
        objectId,
        functionDeclaration: declaration,
        arguments: [{ value: values }],
        returnByValue: true
      })
    : undefined
  const done =
    objectId !== ''
      ? ((result?.result as Record<string, unknown> | undefined)?.value as boolean | undefined)
      : ((await evaluate(
          session,
          `(() => { const fn = ${declaration}; const el = document.querySelector(${JSON.stringify(selector)}); return el ? fn.call(el, ${JSON.stringify(values)}) : false; })()`
        )) as boolean | undefined)
  if (done !== true) throw new Error('这个元素不是 <select>')
  return `${values.length} 个选项`
}

/** One integer argument, or undefined when absent. */
function intArg(args: Record<string, unknown>, name: string): number | undefined {
  const value = args[name]
  if (typeof value === 'number' && Number.isFinite(value)) return Math.trunc(value)
  if (typeof value === 'string' && /^-?\d+$/.test(value.trim())) return Number(value.trim())
  return undefined
}

/** One string argument, trimmed; empty when absent. */
function stringArg(args: Record<string, unknown>, name: string): string {
  const value = args[name]
  return typeof value === 'string' ? value.trim() : ''
}

/** Keep a number inside a range. */
function clamp(value: number, low: number, high: number): number {
  return Math.min(high, Math.max(low, value))
}

/**
 * Wait until text appears, a selector matches, or a reference is still attached.
 * @param session - the running browser.
 * @param args - `text` / `selector` / `ref`, plus `timeoutMs`.
 * @returns what appeared, once it did.
 */
async function waitForPage(
  session: BrowserRuntime,
  args: Record<string, unknown>
): Promise<string> {
  const timeout = clamp(intArg(args, 'timeoutMs') ?? 10_000, 200, 60_000)
  const text = stringArg(args, 'text')
  const selector = stringArg(args, 'selector')
  const ref = intArg(args, 'ref')
  if (text === '' && selector === '' && ref === undefined) {
    throw new Error('wait_for 需要 text、selector 或 ref 之一')
  }
  const target = ref !== undefined ? `编号 [${ref}]` : text !== '' ? `文本「${clip(text, 40)}」` : selector
  const deadline = Date.now() + timeout
  const started = Date.now()
  while (Date.now() < deadline) {
    let seen = false
    if (ref !== undefined) {
      const known = session.refs.get(ref)
      if (known !== undefined) {
        const object = await session.socket
          .send('DOM.resolveNode', { backendNodeId: known })
          .then((result) => (result.object as Record<string, unknown> | undefined)?.objectId)
          .catch(() => undefined)
        if (typeof object === 'string' && object !== '') {
          const alive = await session.socket
            .send('Runtime.callFunctionOn', {
              objectId: object,
              functionDeclaration: 'function() { return this.isConnected === true; }',
              returnByValue: true
            })
            .then((result) => (result.result as Record<string, unknown> | undefined)?.value)
            .catch(() => false)
          seen = alive === true
        }
      }
    } else if (selector !== '') {
      seen = (await evaluate(session, `!!document.querySelector(${JSON.stringify(selector)})`).catch(
        () => false
      )) === true
    } else {
      seen =
        (await evaluate(
          session,
          `(document.body ? document.body.innerText : '').includes(${JSON.stringify(text)})`
        ).catch(() => false)) === true
    }
    if (seen) return `${target} 已出现（${Date.now() - started} ms）`
    await sleep(200)
  }
  throw new Error(`等待超时（${timeout} ms）：${target} 没有出现`)
}

/**
 * Capture the page (or one element) and hand the picture to the model.
 *
 * The shot is scaled so its long edge fits `maxEdge`: providers bill images by
 * area, and a 4000-pixel-wide page would cost more tokens than the answer.
 * @param session - the running browser.
 * @param workDir - the conversation folder that keeps `shots/`.
 * @param args - `fullPage`, `ref` or `selector`, `format`, `quality`, `maxEdge`.
 * @returns the picture plus where it was saved.
 */
async function captureShot(
  session: BrowserRuntime,
  workDir: string,
  args: Record<string, unknown>
): Promise<ToolResult> {
  const format = stringArg(args, 'format').toLowerCase() === 'png' ? 'png' : 'jpg'
  const quality = clamp(intArg(args, 'quality') ?? 70, 30, 100)
  const maxEdge = clamp(intArg(args, 'maxEdge') ?? 1400, 320, 3_000)
  const fullPage = args.fullPage === true
  const ref = intArg(args, 'ref')
  const selector = stringArg(args, 'selector')
  let clip: Record<string, number> | undefined
  if (ref !== undefined) {
    const node = await resolveNode(session, ref)
    const model = await session.socket.send('DOM.getBoxModel', { objectId: node.objectId })
    const box = (model.model ?? {}) as Record<string, unknown>
    const quad = Array.isArray(box.border) ? (box.border as number[]) : []
    if (quad.length >= 8) {
      const xs = [quad[0], quad[2], quad[4], quad[6]]
      const ys = [quad[1], quad[3], quad[5], quad[7]]
      // Same page-vs-viewport correction as `selectorBox`: the clip is in page
      // coordinates, the box model arrives relative to the current scroll.
      const scrolled = (await evaluate(
        session,
        '({ x: window.scrollX, y: window.scrollY })'
      )) as { x?: number; y?: number } | undefined
      const x = Math.min(...xs) + (scrolled?.x ?? 0)
      const y = Math.min(...ys) + (scrolled?.y ?? 0)
      const width = Math.max(...xs) - Math.min(...xs)
      const height = Math.max(...ys) - Math.min(...ys)
      if (width > 0 && height > 0) clip = { x, y, width, height, scale: 1 }
    }
  } else if (selector !== '') {
    clip = { ...(await selectorBox(session, selector)), scale: 1 }
  } else {
    const metrics = await session.socket.send('Page.getLayoutMetrics')
    const content = (metrics.cssContentSize ?? {}) as { width?: number; height?: number }
    const view = (metrics.cssLayoutViewport ?? {}) as {
      clientWidth?: number
      clientHeight?: number
    }
    const width = Math.ceil(fullPage ? (content.width ?? 0) : (view.clientWidth ?? 0))
    const height = Math.ceil(fullPage ? (content.height ?? 0) : (view.clientHeight ?? 0))
    if (width > 0 && height > 0) clip = { x: 0, y: 0, width, height, scale: 1 }
  }
  if (clip) {
    const longest = Math.max(clip.width, clip.height)
    if (longest > maxEdge) clip.scale = Number((maxEdge / longest).toFixed(4))
  }
  const params: Record<string, unknown> = {
    // The protocol calls JPEG "jpeg"; the tool's argument keeps the shorter name.
    format: format === 'png' ? 'png' : 'jpeg',
    captureBeyondViewport: fullPage || ref !== undefined || selector !== ''
  }
  if (format === 'jpg') params.quality = quality
  if (clip) params.clip = clip
  const shot = await session.socket.send('Page.captureScreenshot', params, 30_000)
  const data = typeof shot.data === 'string' ? shot.data : ''
  if (data === '') throw new Error('浏览器没有返回截图')
  const bytes = Buffer.from(data, 'base64')
  if (bytes.length > MAX_IMAGE_BYTES) {
    throw new Error(
      `截图太大（${bytesText(bytes.length)}，上限 ${MAX_IMAGE_BYTES / (1024 * 1024)} MB）；把 maxEdge 调小再试。`
    )
  }
  const saved = await saveImageCopy(workDir, bytes, format === 'png' ? 'png' : 'jpg')
  const image: ToolImage = {
    mime: format === 'png' ? 'image/png' : 'image/jpeg',
    data,
    ...(saved === null ? {} : { path: saved.absolute })
  }
  const area = fullPage
    ? '整页'
    : ref !== undefined
      ? `元素 [${ref}]`
      : selector !== ''
        ? `元素 ${selector}`
        : '视口'
  const size = clip ? `，${Math.round(clip.width)}×${Math.round(clip.height)}` : ''
  const lines = [
    `已截图（${area}${size}，${format.toUpperCase()}，${bytesText(bytes.length)}）`,
    saved === null ? '没有保存到工作区（只附在这条消息里）' : `保存到 ${saved.label}`,
    '这张图已附在下一条消息里；下一次提问会再带上它。'
  ]
  return { ok: true, text: lines.join('\n'), image }
}

/**
 * Run one `browser` call.
 *
 * One browser serves the whole question: the first call starts it, later calls
 * reuse the same page, and `close` ends it (the engine also kills it when the
 * question is over). Everything a page shows or loads is kept for `console` and
 * `network` between calls.
 * @param workDir - the conversation folder; the profile lives under `.dsh-flow`.
 * @param args - `action` plus what that action needs.
 * @returns text for the model, and a picture for `screenshot`.
 */
export async function runBrowser(workDir: string, args: Record<string, unknown>): Promise<ToolResult> {
  const action = (stringArg(args, 'action') || 'snapshot').toLowerCase()
  if (action === 'close') {
    const closed = (await closeBrowser()) || (await closeSavedBrowser(workDir))
    // An explicit close is not a lost session: the next question starts clean and
    // needs no warning.
    lostSession = false
    return { ok: true, text: closed ? '已关闭浏览器。' : '浏览器没有在运行。' }
  }
  // The folder answers even when no browser is running: files downloaded in an
  // earlier question are still there.
  if (action === 'downloads') {
    const listed = await listDownloads(workDir)
    return { ok: true, text: listed.text, files: listed.files }
  }
  // Waiting for a file is a fact about the folder rather than about a session,
  // so a step can wait even when the browser has already closed.
  if (action === 'wait_download') return await waitDownload(workDir, args)
  try {
    return await runBrowserOnce(workDir, action, args)
  } catch (error) {
    if (!browserDied(error)) throw error
    // The browser died mid-question — a crash, the updater, a process the person
    // stopped. Rebuilding it and repeating the action is what keeps a long answer
    // from ending over a browser that can simply be restarted.
    await closeBrowser(true)
    lostSession = true
    return await runBrowserOnce(workDir, action, args)
  }
}

/** Errors that mean the browser is gone, not that the action was wrong. */
export function browserDied(error: unknown): boolean {
  const message = error instanceof Error ? error.message : String(error)
  return /浏览器连接已断开|浏览器已经关闭|无法连接浏览器调试端口|没有在 \d+ 秒内回复 Page\.navigate/.test(
    message
  )
}

/**
 * Run one action against the session, starting it if needed.
 * @param workDir - the conversation folder.
 * @param action - the lowercased action.
 * @param args - the rest of the call.
 * @returns what the model reads.
 */
async function runBrowserOnce(
  workDir: string,
  action: string,
  args: Record<string, unknown>
): Promise<ToolResult> {
  const session = await ensureBrowser(workDir)
  const resumed = takeResumedNote()
  const note = resumed + takeLostNote()
  // Downloads are detected by comparing the folder around the action: the
  // protocol's download events are not available in every browser version.
  const dir = downloadsDir(workDir)
  const before = new Map((await folderEntries(dir)).map((entry) => [entry.name, stamp(entry)]))
  const result = await actOnPage(session, action, args, workDir)
  // A click that starts a download returns before the file is written; the line
  // naming it belongs in this result, not in whatever the model asks next.
  const arrived = await settleDownloads(dir, before)
  // A model that reads「about:blank（页面没有文字）」on its own may decide the driver
  // is broken; say what to do instead. This is what happened in the field: the
  // session had silently restarted, and every later call reported a blank page.
  const blank =
    session.url === 'about:blank' && !['open', 'goto', 'close'].includes(action)
      ? '页面还停在 about:blank：先 open / goto 一个网址。\n'
      : ''
  const prefix = `${note}${arrived.length === 0 ? '' : `${arrived.join('\n')}\n`}${blank}`
  if (prefix === '') return result
  return { ...result, text: `${prefix}${result.text}` }
}

/** The actions themselves; `session` is live and attached. */
async function actOnPage(
  session: BrowserRuntime,
  action: string,
  args: Record<string, unknown>,
  workDir: string
): Promise<ToolResult> {
  switch (action) {
    case 'open':
    case 'goto': {
      const url = browserUrl(args.url)
      const width = intArg(args, 'width')
      const height = intArg(args, 'height')
      await setViewport(
        session,
        width === undefined ? undefined : clamp(width, 320, 2_400),
        height === undefined ? undefined : clamp(height, 240, 2_000)
      )
      await gotoPage(session, url, clamp(intArg(args, 'waitMs') ?? 250, 0, 10_000))
      const info = await pageInfo(session)
      const body = String(
        (await evaluate(session, '(document.body ? document.body.innerText : "").trim()')) ?? ''
      )
      const title = info.title === '' ? '' : `（${clip(info.title, 80)}）`
      return {
        ok: true,
        text: [
          `已打开 ${info.url}${title}`,
          body === '' ? '（页面没有文字）' : clip(body, MAX_BROWSER_TEXT)
        ].join('\n')
      }
    }
    case 'snapshot': {
      const listing = await snapshot(session)
      const info = await pageInfo(session)
      const head = `${info.title === '' ? '' : `${clip(info.title, 80)} — `}${shortUrl(info.url)}`
      return {
        ok: true,
        text: [
          head,
          listing.text === '' ? '（没有列出可交互元素）' : listing.text,
          `共 ${listing.rows} 行；用 [编号] 点击或输入，导航后编号会重新分配。`
        ].join('\n')
      }
    }
    case 'text': {
      const info = await pageInfo(session)
      const body = String(
        (await evaluate(session, '(document.body ? document.body.innerText : "").trim()')) ?? ''
      )
      if (body === '') return { ok: true, text: `${shortUrl(info.url)}（页面没有文字）` }
      const shown = clip(body, MAX_BROWSER_TEXT)
      return {
        ok: true,
        text: [
          shortUrl(info.url),
          body.length > MAX_BROWSER_TEXT
            ? `${shown}（原文 ${body.length} 字符，已截断）`
            : shown
        ].join('\n')
      }
    }
    case 'click': {
      const point = await hitPoint(session, args)
      const button = stringArg(args, 'button').toLowerCase() === 'right' ? 'right' : 'left'
      const clicks = clamp(intArg(args, 'clickCount') ?? 1, 1, 3)
      await clickAt(session, point, button, clicks)
      const info = await pageInfo(session)
      return {
        ok: true,
        text: `已${button === 'right' ? '右键' : '点击'}（${Math.round(point.x)},${Math.round(point.y)}），现在在 ${shortUrl(info.url)}；用 snapshot 看当前可点的元素，用 screenshot 看画面。`
      }
    }
    case 'type': {
      const text = typeof args.text === 'string' ? args.text : ''
      if (text === '') throw new Error('type 需要 text')
      const how = await typeInto(session, args, text)
      return {
        ok: true,
        text: `已输入 ${how}${args.submit === true ? '，并回车提交' : ''}。`
      }
    }
    case 'press': {
      const key = stringArg(args, 'key')
      if (key === '') throw new Error('press 需要 key（enter / tab / escape / arrowup 等）')
      await pressKey(session, key)
      return { ok: true, text: `已按键 ${key}。` }
    }
    case 'select': {
      const how = await selectOption(session, args)
      return { ok: true, text: `已选择 ${how}。` }
    }
    case 'eval': {
      const expression = typeof args.expression === 'string' ? args.expression.trim() : ''
      if (expression === '') throw new Error('eval 需要 expression')
      if (expression.length > MAX_BROWSER_EVAL) {
        throw new Error(`表达式太长（${expression.length} 字符，上限 ${MAX_BROWSER_EVAL}）`)
      }
      const value = await evaluate(session, expression, true)
      const text =
        typeof value === 'string' ? value : value === undefined ? '' : JSON.stringify(value, null, 2)
      return { ok: true, text: clip(text === '' ? '（没有返回值）' : text, MAX_BROWSER_TEXT), value }
    }
    case 'wait_for':
      return { ok: true, text: await waitForPage(session, args) }
    case 'console': {
      // Give the last lines a moment to arrive before reporting.
      await sleep(120)
      const info = await pageInfo(session)
      const lines = session.console.slice(-MAX_BROWSER_LOG)
      return {
        ok: true,
        text: [
          `${shortUrl(info.url)} 的控制台（最近 ${lines.length} 条）：`,
          lines.length === 0 ? '（没有输出）' : lines.join('\n')
        ].join('\n')
      }
    }
    case 'network': {
      const rows = [...session.requests.values()]
        .slice(-MAX_BROWSER_LOG)
        .map(
          (entry) =>
            `${entry.status === '' ? '…' : entry.status} ${entry.method} ${entry.type === '' ? '-' : entry.type} ${shortUrl(entry.url)}`
        )
      const failures = session.failures.slice(-10)
      const lines = [`最近 ${rows.length} 个请求：`, rows.length === 0 ? '（没有请求）' : rows.join('\n')]
      if (failures.length > 0) lines.push('失败的请求：', failures.join('\n'))
      return { ok: true, text: lines.join('\n') }
    }
    case 'screenshot':
      return await captureShot(session, workDir, args)
    default:
      throw new Error(
        `未知动作：${action}（open / goto / snapshot / text / click / type / press / select / eval / wait_for / console / network / screenshot / downloads / wait_download / close）`
      )
  }
}

/**
 * Whether one browser action needs a confirmation card.
 *
 * Reading a page changes nothing — `snapshot`, `text`, `console`, `network`,
 * `screenshot`, `wait_for` and `close` run like `read_image`. Navigation and
 * anything that touches the page (`open`, `goto`, `click`, `type`, `press`,
 * `select`, `eval`) goes through the card, the same way `http` does.
 * @param action - the `action` argument, already lowercased.
 * @returns true when the person should see a card first.
 */
export function browserNeedsCard(action: string): boolean {
  return ['open', 'goto', 'click', 'type', 'press', 'select', 'eval'].includes(action)
}

/**
 * One line describing what a browser call will do, for its approval card.
 * @param args - the call's arguments.
 * @returns the sentence the card shows.
 */
export function browserSummary(args: Record<string, unknown>): string {
  const action = stringArg(args, 'action').toLowerCase()
  const ref = intArg(args, 'ref')
  const selector = stringArg(args, 'selector')
  const who = ref !== undefined ? `编号 [${ref}]` : selector !== '' ? selector : ''
  switch (action) {
    case 'open':
    case 'goto': {
      const url = stringArg(args, 'url')
      return `${action === 'open' ? '打开页面' : '跳转到'}：${url === '' ? '（缺少 url）' : url}`
    }
    case 'click':
      return `点击 ${who === '' ? '（缺少 ref / selector）' : who}`
    case 'type': {
      const text = typeof args.text === 'string' ? args.text : ''
      const where = who === '' ? '当前焦点' : who
      return `在 ${where} 输入：${clip(text, 80)}${args.submit === true ? '（并回车提交）' : ''}`
    }
    case 'press':
      return `按键：${stringArg(args, 'key')}`
    case 'select': {
      const values = Array.isArray(args.values) ? args.values.map((value) => String(value)) : []
      return `在 ${who === '' ? '当前元素' : who} 选择：${values.length === 0 ? '（缺少 values）' : values.join(', ')}`
    }
    case 'eval': {
      const expression = typeof args.expression === 'string' ? args.expression : ''
      return `在页面执行 JavaScript：${clip(expression, 120)}`
    }
    default:
      return `浏览器动作 ${action}`
  }
}