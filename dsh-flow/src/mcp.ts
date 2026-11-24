/**
 * MCP servers: external tool servers spoken to over JSON-RPC.
 *
 * Enabled servers are opened once per question, asked for their tools, and kept
 * alive until the answer is done. The `mcp` tool can also link a server on the
 * spot — by id from the `mcpserver` folder, or by a command / URL — and call
 * that server's tools without the user switching it on first. A server that
 * fails is reported and skipped: one broken server must never stop a question
 * from being answered.
 *
 * Two transports are supported: `stdio` (a child process, newline-delimited
 * JSON-RPC) and streamable HTTP (`POST` with JSON or event-stream replies).
 */

import { spawn, type ChildProcess } from 'node:child_process'
import { mkdir, readdir, readFile, rm, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { createInterface } from 'node:readline'
import type { Writable } from 'node:stream'
import type { McpServerConfig } from './ai.ts'

/** Most tools one server may contribute. */
const MAX_TOOLS = 48

/** Longest description shown to the model for one tool. */
const MAX_DESCRIPTION = 400

/** How long one JSON-RPC request may take. */
const REQUEST_TIMEOUT_MS = 20_000

/** Protocol revision this client speaks. */
const PROTOCOL_VERSION = '2024-11-05'

/** A tool one MCP server offers. */
export interface McpTool {
  /** Name the model calls, prefixed so it cannot shadow a built-in. */
  name: string
  /** What it does, as the model reads it. */
  description: string
  /** JSON schema of its arguments. */
  parameters: Record<string, unknown>
  /** Server id it came from. */
  server: string
  /** Name the server itself knows it by. */
  original: string
  /** Runs it; the returned text goes back to the model. */
  run: (args: Record<string, unknown>) => Promise<{ ok: boolean; text: string }>
}

/** A server that could not be opened. */
export interface McpError {
  /** Server id from the window. */
  id: string
  /** Why it was skipped, in a form the window can show. */
  message: string
}

/** Everything the enabled servers contribute. */
export interface McpSet {
  /** Tools to offer the model. */
  tools: McpTool[]
  /** Servers that were skipped, and why. */
  errors: McpError[]
  /** Stops every server; safe to call more than once. */
  close: () => Promise<void>
}

/** The live connections of one question, plus the `mcp` gateway tool. */
export interface McpHub {
  /** Tools of every server opened so far. */
  readonly tools: McpTool[]
  /** Servers that failed to open, and why. */
  readonly errors: McpError[]
  /** The tool the model calls to list / link / call servers. */
  readonly gateway: McpTool
  /** Open one server; returns its tools and writes the skill files. */
  open: (config: McpServerConfig) => Promise<McpTool[]>
  /** Close every connection. */
  close: () => Promise<void>
}

/** One JSON-RPC peer. */
interface RpcClient {
  request: (method: string, params?: Record<string, unknown>) => Promise<unknown>
  notify: (method: string, params?: Record<string, unknown>) => Promise<void>
  close: () => Promise<void>
}

interface Pending {
  resolve: (value: unknown) => void
  reject: (error: Error) => void
  timer: ReturnType<typeof setTimeout>
}

interface OpenServer {
  id: string
  name: string
  client: RpcClient
  tools: McpTool[]
}

/** A stdio server: one child process, one JSON object per line. */
class StdioClient implements RpcClient {
  private child: ChildProcess
  private stdin: Writable
  private pending = new Map<number, Pending>()
  private next = 1
  private stderr = ''
  private closed = false

  constructor(command: string, args: string[], env: Record<string, string> | undefined, cwd?: string) {
    this.child = spawn(command, args, {
      cwd,
      env: { ...process.env, ...(env ?? {}) },
      stdio: ['pipe', 'pipe', 'pipe'],
      windowsHide: true
    })
    if (!this.child.stdin || !this.child.stdout) {
      throw new Error(`无法打开 ${command} 的管道`)
    }
    this.stdin = this.child.stdin
    const lines = createInterface({ input: this.child.stdout, crlfDelay: Infinity })
    lines.on('line', (line) => this.receive(line))
    this.child.stderr?.on('data', (chunk: Buffer | string) => {
      this.stderr = (this.stderr + chunk.toString()).slice(-4_000)
    })
    this.child.on('error', (error) => this.failAll(`无法启动 ${command}：${error.message}`))
    this.child.on('exit', (code) => {
      const detail = this.stderr.trim() === '' ? '' : `：${this.stderr.trim()}`
      this.failAll(`MCP 服务器已退出（code ${code}）${detail}`)
    })
  }

  /** Send one request and wait for the reply with the same id. */
  request(method: string, params?: Record<string, unknown>): Promise<unknown> {
    if (this.closed) return Promise.reject(new Error('MCP 服务器已关闭'))
    const id = this.next
    this.next += 1
    return new Promise<unknown>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id)
        reject(new Error(`MCP 请求超时：${method}`))
      }, REQUEST_TIMEOUT_MS)
      this.pending.set(id, { resolve, reject, timer })
      this.write({ jsonrpc: '2.0', id, method, ...(params === undefined ? {} : { params }) }, (error) => {
        if (!error) return
        clearTimeout(timer)
        this.pending.delete(id)
        reject(error)
      })
    })
  }

  /** Send a notification; nothing comes back. */
  async notify(method: string, params?: Record<string, unknown>): Promise<void> {
    if (this.closed) return
    await new Promise<void>((resolve, reject) => {
      this.write({ jsonrpc: '2.0', method, ...(params === undefined ? {} : { params }) }, (error) =>
        error ? reject(error) : resolve()
      )
    })
  }

  /** Stop the child and answer every waiting request. */
  async close(): Promise<void> {
    if (this.closed) return
    this.closed = true
    this.failAll('MCP 服务器已关闭')
    try {
      this.stdin.end()
    } catch {
      // The pipe may already be gone.
    }
    this.child.kill()
  }

  private write(message: Record<string, unknown>, done: (error?: Error) => void): void {
    try {
      this.stdin.write(`${JSON.stringify(message)}\n`, (error) => done(error instanceof Error ? error : undefined))
    } catch (error) {
      done(error instanceof Error ? error : new Error(String(error)))
    }
  }

  private receive(line: string): void {
    const text = line.trim()
    if (text === '') return
    let message: Record<string, unknown>
    try {
      message = JSON.parse(text) as Record<string, unknown>
    } catch {
      return
    }
    const id = typeof message.id === 'number' ? message.id : null
    if (id !== null) {
      const pending = this.pending.get(id)
      if (pending) {
        this.pending.delete(id)
        clearTimeout(pending.timer)
        const error = readError(message)
        if (error) pending.reject(error)
        else pending.resolve(message.result)
        return
      }
    }
    // A request from the server (sampling / roots) is answered so it does not
    // wait forever; this client offers neither capability.
    if (typeof message.method === 'string' && message.id !== undefined) {
      this.write(
        {
          jsonrpc: '2.0',
          id: message.id,
          error: { code: -32601, message: `不支持的方法：${message.method}` }
        },
        () => undefined
      )
    }
  }

  private failAll(reason: string): void {
    if (this.pending.size === 0) {
      this.closed = true
      return
    }
    for (const pending of this.pending.values()) {
      clearTimeout(pending.timer)
      pending.reject(new Error(reason))
    }
    this.pending.clear()
    this.closed = true
  }
}

/** A streamable HTTP server: one POST per message. */
class HttpClient implements RpcClient {
  private session = ''
  private next = 1
  private url: string
  private headers: Record<string, string> | undefined

  constructor(url: string, headers: Record<string, string> | undefined) {
    this.url = url
    this.headers = headers
  }

  /** Send one request and read its reply from JSON or an event stream. */
  async request(method: string, params?: Record<string, unknown>): Promise<unknown> {
    const id = this.next
    this.next += 1
    const message = { jsonrpc: '2.0', id, method, ...(params === undefined ? {} : { params }) }
    return await this.post(message, id)
  }

  /** Send a notification; the server answers with 202 in the usual case. */
  async notify(method: string, params?: Record<string, unknown>): Promise<void> {
    await this.post({ jsonrpc: '2.0', method, ...(params === undefined ? {} : { params }) }, undefined)
  }

  /** HTTP has nothing to release; the session is dropped by the server. */
  async close(): Promise<void> {
    return
  }

  private async post(message: Record<string, unknown>, id: number | undefined): Promise<unknown> {
    const controller = new AbortController()
    const timer = setTimeout(() => controller.abort(), REQUEST_TIMEOUT_MS)
    try {
      const headers: Record<string, string> = {
        'content-type': 'application/json',
        accept: 'application/json, text/event-stream',
        ...(this.session === '' ? {} : { 'mcp-session-id': this.session }),
        ...(this.headers ?? {})
      }
      const response = await fetch(this.url, {
        method: 'POST',
        headers,
        body: JSON.stringify(message),
        signal: controller.signal
      })
      const session = response.headers.get('mcp-session-id')
      if (session) this.session = session
      if (!response.ok) {
        throw new Error(`HTTP ${response.status}：${(await response.text()).slice(0, 200)}`)
      }
      if (id === undefined || response.status === 202 || response.status === 204) return undefined
      const type = response.headers.get('content-type') ?? ''
      const payload = type.includes('text/event-stream')
        ? await readEventStream(response, id)
        : ((await response.json()) as unknown)
      const chosen = pickReply(payload, id)
      if (!chosen) throw new Error('MCP 响应里没有这一条请求')
      const error = readError(chosen as Record<string, unknown>)
      if (error) throw error
      return (chosen as Record<string, unknown>).result
    } finally {
      clearTimeout(timer)
    }
  }
}

/** The schema of the `mcp` gateway tool. */
const GATEWAY_PARAMETERS: Record<string, unknown> = {
  type: 'object',
  properties: {
    action: {
      type: 'string',
      enum: ['list', 'link', 'tools', 'call', 'disconnect'],
      description: 'list 看 mcpserver 目录；link 连接；tools 看本次已连接；call 调用工具；disconnect 断开'
    },
    server: {
      type: 'string',
      description: 'link 时引用 mcpserver 里已有的 id；call / disconnect 时指定服务器'
    },
    id: { type: 'string', description: 'link 新服务器时使用的 id；省略时用 server 或 name' },
    name: { type: 'string', description: '显示名' },
    transport: { type: 'string', enum: ['stdio', 'http'], description: '默认按 command / url 推断' },
    command: { type: 'string', description: 'stdio 服务器可执行文件' },
    args: { type: 'array', items: { type: 'string' }, description: 'stdio 命令参数' },
    env: { type: 'object', additionalProperties: { type: 'string' }, description: 'stdio 环境变量' },
    url: { type: 'string', description: 'http 服务器地址' },
    headers: { type: 'object', additionalProperties: { type: 'string' }, description: 'http 请求头' },
    cwd: { type: 'string', description: 'stdio 运行目录' },
    save: { type: 'boolean', description: 'link 时把配置写进 mcpserver/<id>/mcp.json（默认 true）' },
    tool: { type: 'string', description: 'call 时服务器自己的工具名' },
    arguments: { type: 'object', description: 'call 的参数对象' }
  },
  required: ['action']
}

/**
 * Open the enabled servers and return the live hub.
 * @param configs - only servers the window switched on.
 * @param options - the catalog folder used by the `mcp` gateway.
 * @returns the tools, the servers that failed, and the gateway.
 */
export async function createMcpHub(
  configs: McpServerConfig[] = [],
  options: { dir?: string } = {}
): Promise<McpHub> {
  const hub = new McpHubImpl((options.dir ?? '').trim())
  for (const config of configs) {
    const id = (config.id ?? '').trim()
    if (id === '') continue
    try {
      await hub.open(config)
    } catch (error) {
      hub.errors.push({ id, message: messageOf(error) })
    }
  }
  return hub
}

/** The implementation behind {@link McpHub}. */
class McpHubImpl implements McpHub {
  readonly errors: McpError[] = []
  readonly gateway: McpTool
  private servers = new Map<string, OpenServer>()
  private names = new Set<string>()
  private dir: string

  constructor(dir: string) {
    this.dir = dir
    this.gateway = {
      name: 'mcp',
      description:
        '连接并调用 MCP 服务器。list 查看 mcpserver 目录里已有的服务器；link 按 id 连接，或直接给 command / url（会保存配置）；tools 查看本次已连接的服务器与工具；call 调用某个服务器的工具；disconnect 断开。连接在本次提问内保持。',
      parameters: GATEWAY_PARAMETERS,
      server: 'mcp',
      original: 'mcp',
      run: (args) => this.runGateway(args)
    }
  }

  get tools(): McpTool[] {
    return [...this.servers.values()].flatMap((server) => server.tools)
  }

  /** Open one server, collect its tools, and write the skill files. */
  async open(config: McpServerConfig): Promise<McpTool[]> {
    const id = (config.id ?? '').trim()
    if (!isServerId(id)) throw new Error(`服务器 id「${id}」不合法`)
    const existing = this.servers.get(id)
    if (existing) return existing.tools
    const problem = configProblem(config)
    if (problem !== '') throw new Error(problem)
    const client = createClient(config)
    const taken = new Set(this.names)
    const serverTools: McpTool[] = []
    try {
      await client.request('initialize', {
        protocolVersion: PROTOCOL_VERSION,
        capabilities: {},
        clientInfo: { name: 'dsh-flow-ui', version: '1.0.0' }
      })
      await client.notify('notifications/initialized', {})
      let cursor: string | undefined
      let pages = 0
      do {
        const page = (await client.request('tools/list', cursor === undefined ? {} : { cursor })) as {
          tools?: unknown
          nextCursor?: unknown
        }
        for (const raw of Array.isArray(page?.tools) ? page.tools : []) {
          if (serverTools.length >= MAX_TOOLS) break
          const tool = readTool(raw, id, config.name ?? id, client, taken)
          if (tool) serverTools.push(tool)
        }
        cursor = typeof page?.nextCursor === 'string' && page.nextCursor !== '' ? page.nextCursor : undefined
        pages += 1
      } while (cursor !== undefined && pages < 20)
    } catch (error) {
      await client.close().catch(() => undefined)
      throw error
    }
    for (const tool of serverTools) this.names.add(tool.name)
    this.servers.set(id, { id, name: config.name ?? id, client, tools: serverTools })
    await writeSkills(this.dir, id, serverTools).catch((error) => {
      process.stderr.write(`[mcp ${id}] 技能写入失败：${messageOf(error)}\n`)
    })
    return serverTools
  }

  /** Close every connection; safe to call more than once. */
  async close(): Promise<void> {
    const open = [...this.servers.values()]
    this.servers.clear()
    this.names.clear()
    await Promise.all(open.map((server) => server.client.close().catch(() => undefined)))
  }

  /** The gateway's dispatcher. */
  private async runGateway(args: Record<string, unknown>): Promise<{ ok: boolean; text: string }> {
    const action = typeof args.action === 'string' ? args.action.trim().toLowerCase() : ''
    if (action === 'list') return await this.list()
    if (action === 'tools') return this.describeOpen()
    if (action === 'link') return await this.link(args)
    if (action === 'call') return await this.call(args)
    if (action === 'disconnect') return await this.disconnect(args)
    return { ok: false, text: 'action 必须是 list / link / tools / call / disconnect' }
  }

  /** List the catalog plus the servers opened only for this question. */
  private async list(): Promise<{ ok: boolean; text: string }> {
    const catalog = await readCatalog(this.dir)
    const lines: string[] = []
    const seen = new Set<string>()
    for (const server of catalog) {
      seen.add(server.id)
      const state = this.servers.has(server.id) ? '（本次已连接）' : ''
      const problem = server.problem ? `；问题：${server.problem}` : ''
      lines.push(`- ${server.id}：${server.name ?? server.id}（${describeConfig(server)}${problem}）${state}`)
    }
    for (const id of this.servers.keys()) {
      if (!seen.has(id)) lines.push(`- ${id}：本次直接连接（不在 mcpserver 目录里）`)
    }
    if (lines.length === 0) {
      return {
        ok: true,
        text: 'mcpserver 目录里还没有服务器。link 时可以直接给 command 或 url，并把配置保存进去。'
      }
    }
    return { ok: true, text: `MCP 服务器（${lines.length}）：\n${lines.join('\n')}` }
  }

  /** What this question has open. */
  private describeOpen(): { ok: boolean; text: string } {
    if (this.servers.size === 0) {
      return { ok: true, text: '本次提问还没有连接 MCP 服务器。先用 action=link。' }
    }
    const lines = [...this.servers.values()].map((server) => {
      const tools = server.tools.map((tool) => tool.original).join(', ')
      return `- ${server.id}（${server.name}）：${tools === '' ? '没有工具' : tools}`
    })
    return { ok: true, text: `本次已连接：\n${lines.join('\n')}` }
  }

  /** Link by catalog id, or by an inline command / URL and optionally save it. */
  private async link(args: Record<string, unknown>): Promise<{ ok: boolean; text: string }> {
    const inline = firstText(args.command, args.url) !== ''
    const requested = firstText(args.server, args.id, args.name)
    let config: McpServerConfig
    let fromCatalog = false
    if (!inline && firstText(args.server) !== '') {
      const serverId = firstText(args.server)
      const catalog = await readCatalog(this.dir)
      const found = catalog.find((entry) => entry.id === serverId)
      if (!found) {
        const known = catalog.map((entry) => entry.id).join(', ')
        return {
          ok: false,
          text: `mcpserver 目录里没有「${serverId}」。可用：${known === '' ? '（空）' : known}`
        }
      }
      if (found.problem) return { ok: false, text: `服务器「${serverId}」无法使用：${found.problem}` }
      config = found
      fromCatalog = true
    } else {
      if (requested === '') return { ok: false, text: 'link 需要 server（已有 id）、id 或 name' }
      const id = slug(requested)
      if (!isServerId(id)) return { ok: false, text: `服务器 id「${id}」不合法` }
      const transport = firstText(args.transport).toLowerCase() || (firstText(args.url) !== '' ? 'http' : 'stdio')
      config = {
        id,
        name: firstText(args.name) || id,
        transport,
        command: firstText(args.command),
        args: readStringArray(args.args),
        env: readStringMap(args.env),
        url: firstText(args.url),
        headers: readStringMap(args.headers),
        cwd: firstText(args.cwd)
      }
      const problem = configProblem(config)
      if (problem !== '') return { ok: false, text: problem }
    }
    try {
      const tools = await this.open(config)
      const names = tools.map((tool) => `${tool.original}（${tool.description}）`).join('；')
      let note = ''
      if (!fromCatalog && args.save !== false && this.dir !== '') {
        note = await saveManifest(this.dir, config)
      }
      return {
        ok: true,
        text: `已连接 MCP 服务器「${config.name ?? config.id}」（${config.id}）。工具：${names === '' ? '没有工具' : names}。\n调用方式：action=call、server=${config.id}、tool=<工具名>、arguments={…}。${note}`
      }
    } catch (error) {
      return { ok: false, text: `连接 MCP 服务器「${config.id}」失败：${messageOf(error)}` }
    }
  }

  /** Call one tool, opening the server by id if it is not open yet. */
  private async call(args: Record<string, unknown>): Promise<{ ok: boolean; text: string }> {
    const server = firstText(args.server)
    const toolName = firstText(args.tool)
    if (server === '' || toolName === '') return { ok: false, text: 'call 需要 server 与 tool' }
    if (!this.servers.has(server)) {
      const catalog = await readCatalog(this.dir)
      const found = catalog.find((entry) => entry.id === server)
      if (!found) return { ok: false, text: `没有连接「${server}」，mcpserver 目录里也没有它。先 action=link。` }
      if (found.problem) return { ok: false, text: `服务器「${server}」无法使用：${found.problem}` }
      try {
        await this.open(found)
      } catch (error) {
        return { ok: false, text: `连接「${server}」失败：${messageOf(error)}` }
      }
    }
    const open = this.servers.get(server)
    if (!open) return { ok: false, text: `没有连接「${server}」` }
    const tool = open.tools.find((entry) => entry.original === toolName || entry.name === toolName)
    if (!tool) {
      const known = open.tools.map((entry) => entry.original).join(', ')
      return {
        ok: false,
        text: `服务器「${server}」没有工具「${toolName}」。可用：${known === '' ? '（没有工具）' : known}`
      }
    }
    try {
      return await tool.run(readArguments(args.arguments))
    } catch (error) {
      return { ok: false, text: `调用「${server}:${toolName}」失败：${messageOf(error)}` }
    }
  }

  /** Close one server, or all of them. */
  private async disconnect(args: Record<string, unknown>): Promise<{ ok: boolean; text: string }> {
    const server = firstText(args.server)
    if (server !== '') {
      const open = this.servers.get(server)
      if (!open) return { ok: true, text: `没有连接「${server}」` }
      this.servers.delete(server)
      this.rebuildNames()
      await open.client.close().catch(() => undefined)
      return { ok: true, text: `已断开「${server}」` }
    }
    const open = [...this.servers.values()]
    this.servers.clear()
    this.names.clear()
    await Promise.all(open.map((entry) => entry.client.close().catch(() => undefined)))
    return { ok: true, text: '已断开全部 MCP 服务器' }
  }

  private rebuildNames(): void {
    this.names = new Set()
    for (const server of this.servers.values()) {
      for (const tool of server.tools) this.names.add(tool.name)
    }
  }
}

/**
 * Open the enabled servers and collect the tools they offer.
 * @param configs - only servers the window switched on.
 * @returns the tools, the servers that failed, and a close function.
 */
export async function loadMcpServers(configs: McpServerConfig[] = []): Promise<McpSet> {
  const hub = await createMcpHub(configs)
  return {
    tools: hub.tools,
    errors: hub.errors,
    close: () => hub.close()
  }
}

/** Read `mcpserver` manifests: a shared `mcpServers` map and one folder each. */
async function readCatalog(dir: string): Promise<McpServerConfig[]> {
  const root = dir.trim()
  if (root === '') return []
  const found: McpServerConfig[] = []
  const shared = await readObject(path.join(root, 'mcp.json'))
  if (shared) {
    const map = shared.mcpServers
    if (isRecord(map)) {
      for (const [id, value] of Object.entries(map)) {
        const config = catalogConfig(id, value, root, false)
        if (config) found.push(config)
      }
    }
  }
  const entries = await readdir(root, { withFileTypes: true }).catch(() => [])
  for (const entry of entries) {
    if (!entry.isDirectory() || entry.name.startsWith('.')) continue
    const folder = path.join(root, entry.name)
    const value = await readObject(path.join(folder, 'mcp.json'))
    if (!value) continue
    const config = catalogConfig(entry.name, value, folder, true)
    if (config) found.push(config)
  }
  const seen = new Set<string>()
  return found.filter((config) => {
    if (seen.has(config.id)) return false
    seen.add(config.id)
    return true
  })
}

/** One catalog entry, with the same validation the window applies. */
function catalogConfig(
  rawId: string,
  value: unknown,
  base: string,
  defaultCwd: boolean
): McpServerConfig | null {
  if (!isRecord(value)) return null
  const id = rawId.trim()
  if (!isServerId(id)) return null
  const text = (key: string): string => (typeof value[key] === 'string' ? value[key].trim() : '')
  const command = text('command')
  const url = text('url')
  const declared = text('transport') || text('type')
  const transport = (declared === '' ? (url === '' ? 'stdio' : 'http') : declared).toLowerCase()
  const configuredCwd = text('cwd')
  const cwd = defaultCwd
    ? configuredCwd === ''
      ? base
      : path.isAbsolute(configuredCwd)
        ? configuredCwd
        : path.resolve(base, configuredCwd)
    : configuredCwd === ''
      ? undefined
      : configuredCwd
  const config: McpServerConfig = {
    id,
    name: text('name') || id,
    description: text('description'),
    transport,
    command,
    args: readStringArray(value.args),
    env: readStringMap(value.env),
    url,
    headers: readStringMap(value.headers),
    cwd
  }
  const problem = configProblem(config)
  if (problem !== '') config.problem = problem
  return config
}

/** Why a config cannot be opened, or an empty string. */
function configProblem(config: McpServerConfig): string {
  const transport = (config.transport ?? '').toLowerCase()
  if (transport === 'sse') return '暂不支持 sse 传输，请改用 stdio 或 http'
  if (transport !== 'stdio' && transport !== 'http') {
    return `transport 只支持 stdio / http，这里是「${transport}」`
  }
  if (transport === 'stdio' && (config.command ?? '').trim() === '') return '缺少 command'
  if (transport === 'http') {
    const url = (config.url ?? '').trim()
    if (url === '') return '缺少 url'
    if (!/^https?:\/\//i.test(url)) return 'url 必须是 http 或 https'
  }
  return ''
}

/**
 * Write one JSON file per skill under `<dir>/skills/<server id>/`.
 *
 * The folder is the window's source for the card's skill count and the
 * "技能" panel; stale files from an earlier connection are removed so the
 * folder always matches what the server currently offers.
 */
async function writeSkills(dir: string, serverId: string, tools: McpTool[]): Promise<void> {
  if (dir.trim() === '') return
  const folder = path.join(dir, 'skills', serverId)
  await mkdir(folder, { recursive: true })
  const wanted = new Set<string>()
  for (const tool of tools) {
    const base = sanitise(tool.original, 40)
    let file = `${base}.json`
    let suffix = 2
    while (wanted.has(file)) {
      file = `${base}_${suffix}.json`
      suffix += 1
    }
    wanted.add(file)
    const body = {
      server: serverId,
      name: tool.original,
      exposed: tool.name,
      description: tool.description,
      inputSchema: tool.parameters
    }
    const target = path.join(folder, file)
    const content = `${JSON.stringify(body, null, 2)}\n`
    // Reconnecting with the same tools should not rewrite every file.
    const existing = await readFile(target, 'utf8').catch(() => null)
    if (existing !== content) await writeFile(target, content, 'utf8')
  }
  const entries = await readdir(folder, { withFileTypes: true }).catch(() => [])
  for (const entry of entries) {
    if (entry.isFile() && entry.name.endsWith('.json') && !wanted.has(entry.name)) {
      await rm(path.join(folder, entry.name)).catch(() => undefined)
    }
  }
}

/** Write an inline link into `<dir>/<id>/mcp.json`, never overwriting. */
async function saveManifest(dir: string, config: McpServerConfig): Promise<string> {  if (dir.trim() === '' || !isServerId(config.id)) return ''
  const folder = path.join(dir, config.id)
  const file = path.join(folder, 'mcp.json')
  try {
    await mkdir(folder, { recursive: true })
  } catch (error) {
    return `\n连接成功，但无法创建 ${folder}：${messageOf(error)}`
  }
  try {
    await readFile(file)
    return `\n配置已存在，未覆盖：${file}`
  } catch {
    // No file yet: write one below.
  }
  const body = {
    name: config.name,
    description: config.description,
    transport: config.transport,
    command: config.command,
    args: config.args,
    env: config.env,
    url: config.url,
    headers: config.headers,
    cwd: config.cwd
  }
  try {
    await writeFile(file, `${JSON.stringify(body, null, 2)}\n`, 'utf8')
    return `\n配置已保存：${file}`
  } catch (error) {
    return `\n连接成功，但保存配置失败：${messageOf(error)}`
  }
}

/** One line describing the transport and target. */
function describeConfig(config: McpServerConfig): string {
  const transport = (config.transport ?? 'stdio').toLowerCase()
  if (transport === 'stdio') {
    const args = Array.isArray(config.args) ? config.args.join(' ') : ''
    return `stdio · ${(config.command ?? '').trim()}${args === '' ? '' : ` ${args}`}`
  }
  return `${transport} · ${config.url ?? ''}`
}

/** Open one server over the transport its manifest asked for. */
function createClient(config: McpServerConfig): RpcClient {
  const transport = (config.transport ?? (config.url ? 'http' : 'stdio')).toLowerCase()
  if (transport === 'stdio') {
    const command = (config.command ?? '').trim()
    if (command === '') throw new Error('缺少 command')
    const args = Array.isArray(config.args) ? config.args.filter((arg) => typeof arg === 'string') : []
    const env = isRecord(config.env) ? (config.env as Record<string, string>) : undefined
    const cwd = typeof config.cwd === 'string' && config.cwd.trim() !== '' ? config.cwd : undefined
    return new StdioClient(command, args, env, cwd)
  }
  if (transport === 'http') {
    const url = (config.url ?? '').trim()
    if (url === '') throw new Error('缺少 url')
    if (!/^https?:\/\//i.test(url)) throw new Error('url 必须是 http 或 https')
    const headers = isRecord(config.headers) ? (config.headers as Record<string, string>) : undefined
    return new HttpClient(url, headers)
  }
  throw new Error(`暂不支持 ${transport} 传输，请改用 stdio 或 http`)
}

/**
 * Turn one MCP tool definition into something the model can call.
 * @param raw - the server's `tools/list` entry.
 * @param server - server id.
 * @param serverName - name shown in the description.
 * @param client - the connection to call through.
 * @param names - names already used, so two servers cannot collide.
 * @returns the tool, or null when the entry is not usable.
 */
function readTool(
  raw: unknown,
  server: string,
  serverName: string,
  client: RpcClient,
  names: Set<string>
): McpTool | null {
  if (!isRecord(raw)) return null
  const original = typeof raw.name === 'string' ? raw.name.trim() : ''
  if (original === '') return null
  const label = `${sanitise(server, 24)}_${sanitise(original, 40)}`
  let name = `mcp_${label}`.slice(0, 64)
  let suffix = 2
  while (names.has(name)) {
    name = `${`mcp_${label}`.slice(0, 60)}_${suffix}`
    suffix += 1
  }
  names.add(name)
  const description = typeof raw.description === 'string' ? raw.description.trim() : ''
  const parameters = isRecord(raw.inputSchema) ? raw.inputSchema : { type: 'object', properties: {} }
  return {
    name,
    description: `MCP「${serverName}」：${description === '' ? original : description}`.slice(0, MAX_DESCRIPTION),
    parameters,
    server,
    original,
    run: async (args) => {
      const result = (await client.request('tools/call', {
        name: original,
        arguments: args
      })) as Record<string, unknown> | undefined
      return {
        ok: result?.isError !== true,
        text: renderContent(result)
      }
    }
  }
}

/** Flatten an MCP `tools/call` result into text for the model. */
function renderContent(result: unknown): string {
  if (!isRecord(result)) return JSON.stringify(result)
  const content = Array.isArray(result.content) ? result.content : []
  const parts: string[] = []
  for (const item of content) {
    if (!isRecord(item)) continue
    if (item.type === 'text' && typeof item.text === 'string') {
      parts.push(item.text)
    } else if (item.type === 'image') {
      parts.push(`[图片${typeof item.mimeType === 'string' ? ` ${item.mimeType}` : ''}]`)
    } else if (item.type === 'audio') {
      parts.push(`[音频${typeof item.mimeType === 'string' ? ` ${item.mimeType}` : ''}]`)
    } else if (item.type === 'resource') {
      const resource = isRecord(item.resource) ? item.resource : undefined
      if (resource && typeof resource.text === 'string') parts.push(resource.text)
      else parts.push(`[资源${resource && typeof resource.uri === 'string' ? ` ${resource.uri}` : ''}]`)
    } else if (item.type === 'resource_link') {
      parts.push(`[资源链接${typeof item.uri === 'string' ? ` ${item.uri}` : ''}]`)
    } else {
      parts.push(JSON.stringify(item))
    }
  }
  if (parts.length === 0 && result.structuredContent !== undefined) {
    parts.push(JSON.stringify(result.structuredContent))
  }
  const text = parts.join('\n').trim()
  return text === '' ? '(MCP 工具没有返回内容)' : text
}

/**
 * Read `data:` lines until the reply with `id` arrives, then stop reading.
 * @param response - the event-stream response.
 * @param id - request id to wait for.
 * @returns the JSON-RPC message.
 */
async function readEventStream(response: Response, id: number): Promise<unknown> {
  const reader = response.body?.getReader()
  if (!reader) throw new Error('MCP 响应没有可读取的流')
  const decoder = new TextDecoder()
  let buffer = ''
  try {
    for (;;) {
      const { done, value } = await reader.read()
      if (done) break
      buffer += decoder.decode(value, { stream: true })
      let end: number
      while ((end = buffer.indexOf('\n')) >= 0) {
        const line = buffer.slice(0, end).replace(/\r$/, '')
        buffer = buffer.slice(end + 1)
        if (!line.startsWith('data:')) continue
        const text = line.slice(5).trim()
        if (text === '' || text === '[DONE]') continue
        let parsed: unknown
        try {
          parsed = JSON.parse(text)
        } catch {
          continue
        }
        const chosen = pickReply(parsed, id)
        if (chosen) return chosen
      }
    }
    throw new Error('MCP 事件流在回复前结束了')
  } finally {
    await reader.cancel().catch(() => undefined)
  }
}

/** Pick the message with `id` out of one object or an array. */
function pickReply(payload: unknown, id: number): unknown {
  if (Array.isArray(payload)) {
    return payload.find((item) => isRecord(item) && item.id === id)
  }
  if (isRecord(payload)) {
    if (payload.id === id || (payload.id === undefined && (payload.result !== undefined || payload.error !== undefined))) {
      return payload
    }
  }
  return undefined
}

/** A JSON-RPC `error` field as an Error, or null. */
function readError(message: Record<string, unknown>): Error | null {
  const error = message.error
  if (!isRecord(error)) return null
  const code = typeof error.code === 'number' ? error.code : 0
  const text = typeof error.message === 'string' ? error.message : '未知错误'
  return new Error(`MCP 错误 ${code}：${text}`)
}

/** Read a JSON object from a file, or null when it is missing or broken. */
async function readObject(file: string): Promise<Record<string, unknown> | null> {
  try {
    const value = JSON.parse(await readFile(file, 'utf8')) as unknown
    return isRecord(value) ? value : null
  } catch {
    return null
  }
}

/** Whether `value` is a plain object. */
function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
}

/** A tool-name fragment: lowercase letters, digits and underscores. */
function sanitise(text: string, limit: number): string {
  const cleaned = text
    .toLowerCase()
    .replace(/[^a-z0-9_]+/g, '_')
    .replace(/^_+|_+$/g, '')
    .slice(0, limit)
  return cleaned === '' ? 'tool' : cleaned
}

/** A server id: letters, digits, dot, dash, underscore and spaces. */
function isServerId(id: string): boolean {
  return (
    id.length > 0 &&
    id.length <= 64 &&
    !id.startsWith('.') &&
    !id.includes('..') &&
    /^[a-zA-Z0-9._ -]+$/.test(id)
  )
}

/** Turn a display name into a server id. */
function slug(text: string): string {
  return text
    .trim()
    .replace(/[^a-zA-Z0-9._ -]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 64)
}

/** The first non-empty string among the arguments. */
function firstText(...values: unknown[]): string {
  for (const value of values) {
    if (typeof value === 'string' && value.trim() !== '') return value.trim()
  }
  return ''
}

/** A string array, ignoring anything else. */
function readStringArray(value: unknown): string[] {
  return Array.isArray(value) ? value.filter((item): item is string => typeof item === 'string') : []
}

/** A string map, stringifying numbers and booleans. */
function readStringMap(value: unknown): Record<string, string> | undefined {
  if (!isRecord(value)) return undefined
  const result: Record<string, string> = {}
  for (const [key, item] of Object.entries(value)) {
    if (typeof item === 'string') result[key] = item
    else if (typeof item === 'number' || typeof item === 'boolean') result[key] = String(item)
  }
  return result
}

/** `arguments` as an object, accepting a JSON string as well. */
function readArguments(value: unknown): Record<string, unknown> {
  if (isRecord(value)) return value
  if (typeof value === 'string' && value.trim() !== '') {
    try {
      const parsed = JSON.parse(value) as unknown
      if (isRecord(parsed)) return parsed
    } catch {
      // A broken string becomes no arguments; the tool will say what it needs.
    }
  }
  return {}
}

/** An error message as a string. */
function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}
