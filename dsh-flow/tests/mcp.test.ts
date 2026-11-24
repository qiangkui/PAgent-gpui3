import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { createServer } from 'node:http'
import { mkdtemp, mkdir, readFile, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { test } from 'node:test'
import { createMcpHub, loadMcpServers } from '../src/mcp.ts'

const CLI = fileURLToPath(new URL('../src/cli.ts', import.meta.url))

/** A dependency-free stdio MCP server, enough for the client tests. */
const STDIO_SERVER = `
process.stdin.setEncoding('utf8')
let buffer = ''
function send(message) { process.stdout.write(JSON.stringify(message) + '\\n') }
function handle(message) {
  if (message.id === undefined) return
  if (message.method === 'initialize') {
    send({ jsonrpc: '2.0', id: message.id, result: {
      protocolVersion: '2024-11-05', capabilities: { tools: {} },
      serverInfo: { name: 'test', version: '1.0.0' }
    } })
    return
  }
  if (message.method === 'tools/list') {
    send({ jsonrpc: '2.0', id: message.id, result: { tools: [
      { name: 'echo', description: '返回输入', inputSchema: {
        type: 'object', properties: { text: { type: 'string' } }, required: ['text']
      } }
    ] } })
    return
  }
  if (message.method === 'tools/call') {
    const text = String((message.params.arguments || {}).text || '')
    send({ jsonrpc: '2.0', id: message.id, result: { content: [{ type: 'text', text }] } })
    return
  }
  send({ jsonrpc: '2.0', id: message.id, error: { code: -32601, message: 'unknown' } })
}
process.stdin.on('data', (chunk) => {
  buffer += chunk
  let end
  while ((end = buffer.indexOf('\\n')) >= 0) {
    const line = buffer.slice(0, end).trim()
    buffer = buffer.slice(end + 1)
    if (line) handle(JSON.parse(line))
  }
})
`

test('opens a stdio server, lists its tools and calls one', async () => {
  const dir = await mkdtemp(path.join(tmpdir(), 'dsh-mcp-'))
  const server = path.join(dir, 'server.mjs')
  await writeFile(server, STDIO_SERVER, 'utf8')
  const set = await loadMcpServers([
    { id: 'demo', name: '演示', transport: 'stdio', command: process.execPath, args: [server] }
  ])
  try {
    assert.deepEqual(set.errors, [])
    assert.deepEqual(set.tools.map((tool) => tool.name), ['mcp_demo_echo'])
    assert.equal(set.tools[0]?.server, 'demo')
    assert.equal(set.tools[0]?.original, 'echo')
    const result = await set.tools[0]!.run({ text: '你好' })
    assert.deepEqual(result, { ok: true, text: '你好' })
  } finally {
    await set.close()
  }
})

test('reports a server with no command and one over an unsupported transport', async () => {
  const set = await loadMcpServers([
    { id: 'empty', transport: 'stdio' },
    { id: 'legacy', transport: 'sse', url: 'http://127.0.0.1:9/sse' }
  ])
  assert.equal(set.tools.length, 0)
  assert.equal(set.errors.length, 2)
  assert.match(set.errors[0]?.message ?? '', /command/)
  assert.match(set.errors[1]?.message ?? '', /sse/)
  await set.close()
})

test('opens a streamable HTTP server', async () => {  const http = createServer((request, response) => {
    let body = ''
    request.setEncoding('utf8')
    request.on('data', (chunk: string) => {
      body += chunk
    })
    request.on('end', () => {
      const message = JSON.parse(body) as Record<string, unknown>
      if (message.id === undefined) {
        response.statusCode = 202
        response.end()
        return
      }
      response.setHeader('content-type', 'application/json')
      const method = message.method
      let result: unknown = {}
      if (method === 'initialize') {
        result = {
          protocolVersion: '2024-11-05',
          capabilities: { tools: {} },
          serverInfo: { name: 'remote', version: '1.0.0' }
        }
      } else if (method === 'tools/list') {
        result = {
          tools: [{ name: 'ping', description: 'ping', inputSchema: { type: 'object', properties: {} } }]
        }
      } else if (method === 'tools/call') {
        result = { content: [{ type: 'text', text: 'pong' }] }
      }
      response.end(JSON.stringify({ jsonrpc: '2.0', id: message.id, result }))
    })
  })
  await new Promise<void>((resolve) => http.listen(0, '127.0.0.1', resolve))
  const address = http.address()
  const port = typeof address === 'object' && address ? address.port : 0
  const set = await loadMcpServers([
    { id: 'remote', transport: 'http', url: `http://127.0.0.1:${port}/mcp` }
  ])
  try {
    assert.deepEqual(set.errors, [])
    assert.deepEqual(set.tools.map((tool) => tool.name), ['mcp_remote_ping'])
    assert.deepEqual(await set.tools[0]!.run({}), { ok: true, text: 'pong' })
  } finally {
    await set.close()
    await new Promise<void>((resolve, reject) =>
      http.close((error) => (error ? reject(error) : resolve()))
    )
  }
})

test('the mcp tool links a catalog server and calls its tool', async () => {
  const dir = await mkdtemp(path.join(tmpdir(), 'dsh-mcp-catalog-'))
  const server = path.join(dir, 'server.mjs')
  await writeFile(server, STDIO_SERVER, 'utf8')
  const folder = path.join(dir, 'demo')
  await mkdir(folder)
  await writeFile(
    path.join(folder, 'mcp.json'),
    JSON.stringify({
      name: '演示',
      transport: 'stdio',
      command: process.execPath,
      args: [server]
    }),
    'utf8'
  )
  const hub = await createMcpHub([], { dir })
  try {
    const listed = await hub.gateway.run({ action: 'list' })
    assert.equal(listed.ok, true)
    assert.match(listed.text, /demo/)

    const linked = await hub.gateway.run({ action: 'link', server: 'demo' })
    assert.equal(linked.ok, true, linked.text)
    assert.match(linked.text, /echo/)

    const called = await hub.gateway.run({
      action: 'call',
      server: 'demo',
      tool: 'echo',
      arguments: { text: '你好' }
    })
    assert.deepEqual(called, { ok: true, text: '你好' })

    const disconnected = await hub.gateway.run({ action: 'disconnect', server: 'demo' })
    assert.equal(disconnected.ok, true)
    assert.deepEqual(hub.tools, [])

    // `call` opens a catalog server by id without a separate `link`.
    const auto = await hub.gateway.run({
      action: 'call',
      server: 'demo',
      tool: 'echo',
      arguments: { text: 'auto' }
    })
    assert.deepEqual(auto, { ok: true, text: 'auto' })
  } finally {
    await hub.close()
  }
})

test('the mcp-skills command fetches one server without a question', async () => {
  const dir = await mkdtemp(path.join(tmpdir(), 'dsh-mcp-skills-cli-'))
  const server = path.join(dir, 'server.mjs')
  await writeFile(server, STDIO_SERVER, 'utf8')
  const folder = path.join(dir, 'demo')
  await mkdir(folder)
  await writeFile(
    path.join(folder, 'mcp.json'),
    JSON.stringify({
      name: '演示',
      transport: 'stdio',
      command: process.execPath,
      args: [server]
    }),
    'utf8'
  )
  const result = spawnSync(process.execPath, [CLI, 'mcp-skills'], {
    input: JSON.stringify({ dir, id: 'demo' }),
    encoding: 'utf8'
  })
  assert.equal(result.status, 0, result.stderr)
  const parsed = JSON.parse(result.stdout.trim()) as {
    ok: boolean
    text: string
    tools: { name: string }[]
  }
  assert.equal(parsed.ok, true, parsed.text)
  assert.deepEqual(parsed.tools.map((tool) => tool.name), ['echo'])
  const skill = await readFile(path.join(dir, 'skills', 'demo', 'echo.json'), 'utf8')
  assert.match(skill, /mcp_demo_echo/)
})

test('an inline link is saved next to the executable folder and can be called', async () => {
  const dir = await mkdtemp(path.join(tmpdir(), 'dsh-mcp-inline-'))
  const server = path.join(dir, 'server.mjs')
  await writeFile(server, STDIO_SERVER, 'utf8')
  const hub = await createMcpHub([], { dir })
  try {
    const linked = await hub.gateway.run({
      action: 'link',
      id: 'inline',
      name: '临时服务器',
      command: process.execPath,
      args: [server]
    })
    assert.equal(linked.ok, true, linked.text)
    const saved = await readFile(path.join(dir, 'inline', 'mcp.json'), 'utf8')
    assert.match(saved, /"command"/)
    const skill = await readFile(path.join(dir, 'skills', 'inline', 'echo.json'), 'utf8')
    assert.match(skill, /"echo"/)
    assert.match(skill, /mcp_inline_echo/)
    const called = await hub.gateway.run({
      action: 'call',
      server: 'inline',
      tool: 'echo',
      arguments: { text: 'saved' }
    })
    assert.deepEqual(called, { ok: true, text: 'saved' })
  } finally {
    await hub.close()
  }
})
