import assert from 'node:assert/strict'
import { execFileSync, spawn } from 'node:child_process'
import { createServer } from 'node:http'
import type { IncomingMessage, Server, ServerResponse } from 'node:http'
import { existsSync } from 'node:fs'
import { mkdir, mkdtemp, readFile, truncate, writeFile } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { test } from 'node:test'
import {
  MAX_CODE_CHARS,
  MAX_EDIT_BYTES,
  MAX_GREP_BYTES,
  MAX_HTTP_CHARS,
  MAX_HTTP_REDIRECTS,
  MAX_IMAGE_BYTES,
  MAX_READ_BYTES,
  MAX_RESULTS,
  MAX_SHELL_CHARS,
  TOOLS,
  previewTool,
  previewToolScoped,
  runTool,
  runToolScoped,
  wildcard
} from '../src/tools.ts'
import { resolveProjectRuntime, resolveRuntime } from '../src/runtime.ts'
import { commandEnv } from '../src/tools/process.ts'
import type { Sandbox } from '../src/tools/sandbox.ts'
import { runAgent } from '../src/agent.ts'
import type { ApprovalRequest } from '../src/agent.ts'
import { DOT_PNG, makeProject, stubFetch } from './helpers.ts'
import {
  parseCargo,
  parseDotnet,
  parseGeneric,
  parseGo,
  parseGradle,
  parseMaven,
  parseNpm,
  parsePytest,
  testPlan
} from '../src/tools/run-tests.ts'
import {
  diagnosticPlan,
  parseCargoDiagnostics,
  parseTextDiagnostics
} from '../src/tools/diagnostics.ts'
import { captureCommand } from '../src/tools/capture.ts'
import { shotsDir, shotCrop } from '../src/tools/image.ts'
import { replaceAsciiBytes } from '../src/tools/files.ts'
import { credentialMentions } from '../src/tools/secrets.ts'
import {
  formatPdfReading,
  missingPdfToolMessage,
  parsePageRange,
  readPdfText,
  usePdfRunner
} from '../src/tools/pdf.ts'
import { MAX_PDF_CHARS, MAX_PDF_PAGES } from '../src/tools/limits.ts'

/**
 * A throw-away working directory.
 * @returns the absolute path.
 */
async function workspace(): Promise<string> {
  return await mkdtemp(path.join(tmpdir(), 'dsh-tools-'))
}

/**
 * A throw-away HTTP server on a random localhost port.
 * @param handler - what every request answers.
 * @returns the server and the base URL to call it with.
 */
async function localServer(
  handler: (req: IncomingMessage, res: ServerResponse) => void
): Promise<{ server: Server; base: string }> {
  const server = createServer(handler)
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve))
  const address = server.address()
  const port = typeof address === 'object' && address !== null ? address.port : 0
  return { server, base: `http://127.0.0.1:${port}` }
}

test('declares every tool the loop can run', () => {
  assert.deepEqual(
    TOOLS.map((tool) => tool.function.name),
    [
      'read', 'write', 'edit', 'replace', 'move', 'copy', 'mkdir', 'delete', 'list', 'glob',
      'tree', 'grep', 'symbols', 'git', 'git_write', 'shell', 'task', 'wait_for', 'diagnostics',
      'run_tests', 'python', 'node', 'http', 'env', 'read_image', 'screenshot', 'show', 'browser'
    ]
  )
})

test('describes every action and wait target the engine implements', () => {
  /** The description of one property, as the model sees it. */
  const property = (tool: string, name: string): string => {
    const found = TOOLS.find((entry) => entry.function.name === tool)
    assert.ok(found, `没有 ${tool}`)
    const properties = (found.function.parameters as { properties: Record<string, { description?: string }> })
      .properties
    assert.ok(properties[name], `${tool} 没有 ${name}`)
    return properties[name]?.description ?? ''
  }
  // `runTask` handles these; the schema is what the model reads, so a missing
  // one makes the action unreachable even though the code supports it.
  for (const action of ['list', 'output', 'input', 'close', 'wait', 'stop', 'stopAll']) {
    assert.match(property('task', 'action'), new RegExp(action), `task 的 action 没提到 ${action}`)
  }
  // Same for `waitFor`, which waits for a port, a page or a task's log line;
  // waiting for the task to *finish* is `task action=wait`, and the schema says
  // so instead of offering a second way to ask the same question.
  for (const target of ['port', 'url', 'output']) {
    assert.match(property('wait_for', 'target'), new RegExp(target), `wait_for 没提到 ${target}`)
  }
  assert.doesNotMatch(property('wait_for', 'target'), /exit/)
  const waitTool = TOOLS.find((entry) => entry.function.name === 'wait_for')
  assert.ok(waitTool)
  assert.match(waitTool.function.description, /action=wait/)
})

test('runs a command and reports its exit code', async () => {
  const root = await workspace()
  const result = await runTool(root, {
    id: '1',
    name: 'shell',
    args: { command: `node -e "process.stdout.write('shell-ok')"` }
  })
  assert.equal(result.ok, true, result.text)
  assert.match(result.text, /^退出码 0/)
  assert.match(result.text, /shell-ok/)
  assert.match(result.text, /\$ node -e/)
})

test('runs a node snippet without shell quoting', async () => {
  const root = await workspace()
  const result = await runTool(root, {
    id: '1',
    name: 'node',
    args: { code: `const text = 'a "quoted" $value'\nprocess.stdout.write(text)` }
  })
  assert.equal(result.ok, true, result.text)
  assert.match(result.text, /^退出码 0/)
  assert.match(result.text, /\$ node -e/)
  assert.match(result.text, /a "quoted" \$value/)
})

test('runs a python snippet when an interpreter exists', async (t) => {
  if (!resolveRuntime('python')) {
    t.skip('这台机器上没有 python')
    return
  }
  const root = await workspace()
  const result = await runTool(root, {
    id: '1',
    name: 'python',
    args: { code: 'print("py-ok")' }
  })
  assert.equal(result.ok, true, result.text)
  assert.match(result.text, /\$ python -c/)
  assert.match(result.text, /py-ok/)
})

test('runs a snippet in a nested folder and refuses to leave the scope', async () => {
  const root = await workspace()
  await mkdir(path.join(root, 'sub'))
  const inside = await runTool(root, {
    id: '1',
    name: 'node',
    args: { code: 'process.stdout.write(process.cwd())', cwd: 'sub' }
  })
  assert.equal(inside.ok, true, inside.text)
  assert.match(inside.text, /sub/)
  const outside = await runTool(root, {
    id: '2',
    name: 'node',
    args: { code: 'process.stdout.write("nope")', cwd: '..' }
  })
  assert.equal(outside.ok, false)
  assert.match(outside.text, /工作区/)
})

test('refuses code that is too long', async () => {
  const root = await workspace()
  const result = await runTool(root, {
    id: '1',
    name: 'node',
    args: { code: `// ${'x'.repeat(MAX_CODE_CHARS)}` }
  })
  assert.equal(result.ok, false)
  assert.match(result.text, /代码太长/)
})

test('indexes declarations and filters them by name', async () => {
  const root = await workspace()
  await mkdir(path.join(root, 'src'))
  await writeFile(
    path.join(root, 'src', 'app.ts'),
    'export function helper() {}\nclass Widget {}\n'
  )
  await writeFile(path.join(root, 'src', 'main.rs'), 'pub struct App {}\nfn main() {}\n')
  await writeFile(path.join(root, 'notes.txt'), 'not code')
  await writeFile(path.join(root, 'clap.py'), 'class Runner:\n    pass\n')

  const all = await runTool(root, { id: '1', name: 'symbols', args: {} })
  assert.equal(all.ok, true, all.text)
  assert.match(all.text, /索引 3 个文件 \/ 5 个符号/, all.text)
  assert.ok(existsSync(path.join(root, '.dsh-flow', 'index.json')), 'the cache is written')

  const found = await runTool(root, { id: '2', name: 'symbols', args: { query: 'helper' } })
  assert.equal(found.ok, true, found.text)
  assert.match(found.text, /src\/app\.ts:1: function helper/)
  assert.doesNotMatch(found.text, /main\.rs/)

  const scoped = await runTool(root, { id: '3', name: 'symbols', args: { path: 'src' } })
  assert.match(scoped.text, /main\.rs/)
  assert.doesNotMatch(scoped.text, /clap\.py/)

  const caseInsensitive = await runTool(root, {
    id: '4',
    name: 'symbols',
    args: { query: 'WIDGET' }
  })
  assert.match(caseInsensitive.text, /class Widget/)
})

test('symbols lists declarations and whole-word references apart', async () => {
  const root = await workspace()
  await mkdir(path.join(root, 'src'))
  await writeFile(
    path.join(root, 'src', 'app.ts'),
    'export function helper() {\n  return 1\n}\nconst helperish = 2\n'
  )
  await writeFile(
    path.join(root, 'src', 'main.ts'),
    'import { helper } from "./app.ts"\n\nhelper()\nhelperX()\n'
  )

  const result = await runTool(root, {
    id: '1',
    name: 'symbols',
    args: { query: 'helper', find: 'references' }
  })
  assert.equal(result.ok, true, result.text)
  assert.match(result.text, /声明（1）：\nsrc\/app\.ts:1: export function helper/)
  assert.match(result.text, /引用（2）：/)
  assert.match(result.text, /src\/main\.ts:1: import \{ helper \}/)
  assert.match(result.text, /src\/main\.ts:3: helper\(\)/)
  assert.doesNotMatch(result.text, /helperish/)
  assert.doesNotMatch(result.text, /helperX/)
})

test('symbols references report nothing, need a name, and respect path', async () => {
  const root = await workspace()
  await mkdir(path.join(root, 'src'))
  await mkdir(path.join(root, 'lib'))
  await writeFile(path.join(root, 'src', 'a.ts'), 'const target = 1\ntarget\n')
  await writeFile(path.join(root, 'lib', 'b.ts'), 'const target = 2\n')

  const missing = await runTool(root, {
    id: '1',
    name: 'symbols',
    args: { query: 'absent', find: 'references' }
  })
  assert.equal(missing.ok, true, missing.text)
  assert.match(missing.text, /没有找到 absent/)

  const noQuery = await runTool(root, {
    id: '2',
    name: 'symbols',
    args: { find: 'references' }
  })
  assert.equal(noQuery.ok, false)
  assert.match(noQuery.text, /标识符/)

  const badFind = await runTool(root, { id: '3', name: 'symbols', args: { find: 'uses' } })
  assert.equal(badFind.ok, false)
  assert.match(badFind.text, /find/)

  const scoped = await runTool(root, {
    id: '4',
    name: 'symbols',
    args: { query: 'target', find: 'references', path: 'lib' }
  })
  assert.match(scoped.text, /lib\/b\.ts:1/)
  assert.doesNotMatch(scoped.text, /src\/a\.ts/)
})

test('symbols references stop at the limit', async () => {
  const root = await workspace()
  const body = Array.from({ length: 6 }, () => 'hit()').join('\n')
  await writeFile(path.join(root, 'many.ts'), `function hit() {}\n${body}\n`)

  const result = await runTool(root, {
    id: '1',
    name: 'symbols',
    args: { query: 'hit', find: 'references', limit: 3 }
  })
  assert.equal(result.ok, true, result.text)
  assert.match(result.text, /声明（1）：/)
  assert.match(result.text, /还有 4 处未显示/)
})

test('symbols reuses its cache until a refresh is asked for', async () => {  const root = await workspace()
  await writeFile(path.join(root, 'main.go'), 'func Before() {}\n')
  await runTool(root, { id: '1', name: 'symbols', args: {} })

  await writeFile(path.join(root, 'extra.py'), 'def fresh(): pass\n')
  const stale = await runTool(root, { id: '2', name: 'symbols', args: { query: 'fresh' } })
  assert.match(stale.text, /没有匹配/)

  const rebuilt = await runTool(root, {
    id: '3',
    name: 'symbols',
    args: { query: 'fresh', refresh: true }
  })
  assert.match(rebuilt.text, /extra\.py:1: fn fresh/)
})

test('a project .venv wins over PATH, and node bins join PATH', async () => {
  const root = await workspace()
  const binDir = process.platform === 'win32' ? 'Scripts' : 'bin'
  const exeName = process.platform === 'win32' ? 'python.exe' : 'python'
  const venvBin = path.join(root, '.venv', binDir)
  await mkdir(venvBin, { recursive: true })
  const venvExe = path.join(venvBin, exeName)
  await writeFile(venvExe, '')

  const python = resolveProjectRuntime('python', root)
  assert.equal(python.local, true)
  assert.equal(python.resolved?.file, venvExe)
  assert.ok(python.pathPrepend.includes(venvBin), python.pathPrepend.join(';'))

  const nodeBin = path.join(root, 'node_modules', '.bin')
  await mkdir(nodeBin, { recursive: true })
  const node = resolveProjectRuntime('node', root)
  assert.equal(node.local, false)
  assert.ok(node.pathPrepend.includes(nodeBin), node.pathPrepend.join(';'))
})

test('project bins go in front of PATH and keys stay out', () => {
  process.env.DSH_TEST_API_KEY = 'hidden'
  try {
    const env = commandEnv([path.join('proj', 'node_modules', '.bin')])
    const key = Object.keys(env).find((name) => name.toUpperCase() === 'PATH')
    assert.ok(key !== undefined)
    assert.equal((env[key] ?? '').split(path.delimiter)[0], path.join('proj', 'node_modules', '.bin'))
    assert.equal(env.DSH_TEST_API_KEY, undefined)
  } finally {
    delete process.env.DSH_TEST_API_KEY
  }
})

test('a node snippet sees the project bin folder on PATH', async () => {
  const root = await workspace()
  const bin = path.join(root, 'node_modules', '.bin')
  await mkdir(bin, { recursive: true })
  const result = await runTool(root, {
    id: '1',
    name: 'node',
    args: {
      code: "console.log((process.env.PATH ?? '').split(require('node:path').delimiter)[0])"
    }
  })
  assert.equal(result.ok, true, result.text)
  assert.ok(result.text.includes(bin), result.text)
})

test('http reads a page and reports the status and headers', async (t) => {
  const { server, base } = await localServer((_req, res) => {
    res.writeHead(200, { 'content-type': 'text/plain; charset=utf-8' })
    res.end('hello 网络')
  })
  t.after(() => server.close())
  const result = await runTool(await workspace(), {
    id: '1',
    name: 'http',
    args: { url: `${base}/page` }
  })
  assert.equal(result.ok, true, result.text)
  assert.match(result.text, new RegExp(`^GET http://127\\.0\\.0\\.1:\\d+/page → 200 OK`))
  assert.match(result.text, /content-type: text\/plain/)
  assert.match(result.text, /hello 网络/)
})

test('http posts a body and marks a 4xx as a failed call', async (t) => {
  const seen: string[] = []
  const { server, base } = await localServer((req, res) => {
    let body = ''
    req.on('data', (chunk: Buffer) => {
      body += chunk.toString('utf8')
    })
    req.on('end', () => {
      seen.push(`${req.method} ${body}`)
      res.writeHead(req.url === '/missing' ? 404 : 200, { 'content-type': 'text/plain' })
      res.end('got it')
    })
  })
  t.after(() => server.close())
  const root = await workspace()
  const posted = await runTool(root, {
    id: '1',
    name: 'http',
    args: { url: `${base}/echo`, method: 'POST', body: 'ping' }
  })
  assert.equal(posted.ok, true, posted.text)
  assert.deepEqual(seen, ['POST ping'])

  const missing = await runTool(root, {
    id: '2',
    name: 'http',
    args: { url: `${base}/missing` }
  })
  assert.equal(missing.ok, false)
  assert.match(missing.text, /→ 404/)
})

test('http sends a credential named by the environment and never prints it', async (t) => {
  const seen: Array<{ auth?: string; key?: string }> = []
  const { server, base } = await localServer((req, res) => {
    seen.push({
      auth: req.headers.authorization,
      key: req.headers['x-api-key'] as string | undefined
    })
    res.writeHead(200, { 'content-type': 'text/plain' })
    res.end('ok')
  })
  t.after(() => server.close())
  process.env.DSH_TEST_TOKEN = 's3cret-value'
  t.after(() => delete process.env.DSH_TEST_TOKEN)
  const root = await workspace()

  const bearer = await runTool(root, {
    id: '1',
    name: 'http',
    args: { url: `${base}/me`, auth: { env: 'DSH_TEST_TOKEN' } }
  })
  assert.equal(bearer.ok, true, bearer.text)
  assert.equal(seen[0]?.auth, 'Bearer s3cret-value', 'Authorization defaults to Bearer')
  assert.match(bearer.text, /环境变量 DSH_TEST_TOKEN/)
  assert.doesNotMatch(bearer.text, /s3cret-value/, 'the value never reaches the transcript')

  // Another header, sent as the variable holds it.
  const raw = await runTool(root, {
    id: '2',
    name: 'http',
    args: {
      url: `${base}/me`,
      auth: { env: 'DSH_TEST_TOKEN', header: 'X-Api-Key', scheme: 'raw' }
    }
  })
  assert.equal(raw.ok, true, raw.text)
  assert.equal(seen[1]?.key, 's3cret-value')

  // Basic base64-encodes `user:pass`, which is what a .netrc-style endpoint wants.
  process.env.DSH_TEST_BASIC = 'user:pass'
  t.after(() => delete process.env.DSH_TEST_BASIC)
  const basic = await runTool(root, {
    id: '3',
    name: 'http',
    args: { url: `${base}/me`, auth: { env: 'DSH_TEST_BASIC', scheme: 'basic' } }
  })
  assert.equal(basic.ok, true, basic.text)
  assert.equal(seen[2]?.auth, `Basic ${Buffer.from('user:pass').toString('base64')}`)
})

test('http refuses a credential variable that is not set and a typed key', async (t) => {
  let hit = 0
  const { server, base } = await localServer((_req, res) => {
    hit += 1
    res.writeHead(200, { 'content-type': 'text/plain' })
    res.end('ok')
  })
  t.after(() => server.close())
  const root = await workspace()

  const missing = await runTool(root, {
    id: '1',
    name: 'http',
    args: { url: `${base}/x`, auth: { env: 'DSH_MISSING_TOKEN' } }
  })
  assert.equal(missing.ok, false)
  assert.match(missing.text, /DSH_MISSING_TOKEN/)
  assert.match(missing.text, /没有设置/)
  assert.equal(hit, 0, 'the request never went out')

  // A key typed into `headers` stays refused: only the variable path is allowed.
  const typed = await runTool(root, {
    id: '2',
    name: 'http',
    args: { url: `${base}/x`, headers: { authorization: 'Bearer abc' } }
  })
  assert.equal(typed.ok, false)
  assert.match(typed.text, /不发 authorization/)
  assert.equal(hit, 0)

  // And `auth` cannot smuggle it out under a transport header.
  const reserved = await runTool(root, {
    id: '3',
    name: 'http',
    args: { url: `${base}/x`, auth: { env: 'DSH_TEST_TOKEN', header: 'Content-Length' } }
  })
  assert.equal(reserved.ok, false)
  assert.match(reserved.text, /auth\.header 不能是 Content-Length/)
  assert.equal(hit, 0)
})

test('a credential follows only its own host across redirects', async (t) => {
  const elsewhere: Array<string | undefined> = []
  const sameHost: Array<string | undefined> = []
  const other = await localServer((req, res) => {
    elsewhere.push(req.headers.authorization)
    res.writeHead(200, { 'content-type': 'text/plain' })
    res.end('ok')
  })
  const origin = await localServer((req, res) => {
    if (req.url === '/landed') {
      sameHost.push(req.headers.authorization)
      res.writeHead(200, { 'content-type': 'text/plain' })
      res.end('ok')
      return
    }
    if (req.url === '/keep') {
      res.writeHead(302, { location: '/landed' })
      res.end()
      return
    }
    res.writeHead(302, { location: `${other.base}/landed` })
    res.end()
  })
  t.after(() => {
    origin.server.close()
    other.server.close()
  })
  process.env.DSH_TEST_TOKEN = 's3cret-value'
  t.after(() => delete process.env.DSH_TEST_TOKEN)
  const root = await workspace()

  // Same host and port: the credential stays.
  const kept = await runTool(root, {
    id: '1',
    name: 'http',
    args: { url: `${origin.base}/keep`, auth: { env: 'DSH_TEST_TOKEN' } }
  })
  assert.equal(kept.ok, true, kept.text)
  assert.equal(sameHost[0], 'Bearer s3cret-value')

  // Another port is another origin: it must not receive the key, and the answer
  // has to say so rather than let a 401 read as a wrong key.
  const crossed = await runTool(root, {
    id: '2',
    name: 'http',
    args: { url: `${origin.base}/away`, auth: { env: 'DSH_TEST_TOKEN' } }
  })
  assert.equal(crossed.ok, true, crossed.text)
  assert.equal(elsewhere[0], undefined)
  assert.match(crossed.text, /没有带上 Authorization/)
})

test('an http call with auth is marked secret for the approval card', async () => {
  const asked: ApprovalRequest[] = []
  const call = {
    id: 'c1',
    type: 'function',
    function: {
      name: 'http',
      arguments: JSON.stringify({
        url: 'https://example.test/api',
        auth: { env: 'DSH_TEST_TOKEN' }
      })
    }
  }
  const stub = stubFetch([
    { choices: [{ message: { content: null, tool_calls: [call] } }] },
    { choices: [{ message: { content: 'refused' } }] }
  ])
  const result = await runAgent(
    { prompt: 'call the api', apiKey: 'sk-test', allowedTools: ['http'] },
    {
      fetchImpl: stub.fetchImpl,
      env: {},
      root: await workspace(),
      confirmActions: async (request) => {
        asked.push(request)
        return false
      }
    }
  )
  assert.equal(asked.length, 1)
  assert.equal(asked[0]?.secret, true)
  assert.match(asked[0]?.diff ?? '', /会发送凭据/)
  assert.doesNotMatch(asked[0]?.diff ?? '', /s3cret/, 'the value is never in the card')
  assert.equal(result.text, 'refused')
})

test('http speaks the other methods and rejects a body on a GET', async (t) => {
  const seen: string[] = []
  const { server, base } = await localServer((req, res) => {
    let body = ''
    req.on('data', (chunk: Buffer) => {
      body += chunk.toString('utf8')
    })
    req.on('end', () => {
      seen.push(`${req.method} ${body}`)
      res.writeHead(200, { 'content-type': 'text/plain' })
      res.end('ok')
    })
  })
  t.after(() => server.close())
  const root = await workspace()
  const put = await runTool(root, {
    id: '1',
    name: 'http',
    args: { url: `${base}/item`, method: 'PUT', body: '{"a":1}' }
  })
  assert.equal(put.ok, true, put.text)
  const removed = await runTool(root, {
    id: '2',
    name: 'http',
    args: { url: `${base}/item`, method: 'DELETE' }
  })
  assert.equal(removed.ok, true, removed.text)
  const head = await runTool(root, {
    id: '3',
    name: 'http',
    args: { url: `${base}/item`, method: 'HEAD' }
  })
  assert.equal(head.ok, true, head.text)
  assert.deepEqual(seen, ['PUT {"a":1}', 'DELETE ', 'HEAD '])

  const wrong = await runTool(root, {
    id: '4',
    name: 'http',
    args: { url: `${base}/item`, method: 'GET', body: 'nope' }
  })
  assert.equal(wrong.ok, false)
  assert.match(wrong.text, /不带正文/)
})

test('write says so when it changes a GBK file to UTF-8', async () => {
  const root = await workspace()
  // 中文 in GBK, the way a Chinese Windows editor may have saved it.
  await writeFile(path.join(root, 'legacy.txt'), Buffer.from([0xd6, 0xd0, 0xce, 0xc4, 0x0a]))
  const result = await runTool(root, {
    id: '1',
    name: 'write',
    args: { path: 'legacy.txt', content: '中文\n' }
  })
  assert.equal(result.ok, true, result.text)
  assert.match(result.text, /不是 UTF-8/)
  const after = await readFile(path.join(root, 'legacy.txt'))
  assert.deepEqual([...after], [...Buffer.from('中文\n')])
})

test('http saves a binary body instead of decoding it', async (t) => {
  const hits: string[] = []
  // The bytes of a tiny PNG are not valid UTF-8 at all, which is the case a
  // text body would have mangled.
  const png = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0xff, 0x00, 0x7f])
  const { server, base } = await localServer((req, res) => {
    hits.push(req.url ?? '')
    res.writeHead(200, { 'content-type': 'image/png' })
    res.end(png)
  })
  t.after(() => server.close())
  const root = await workspace()
  const saved = await runTool(root, {
    id: '1',
    name: 'http',
    args: { url: `${base}/dot.png`, saveTo: 'assets/dot.png' }
  })
  assert.equal(saved.ok, true, saved.text)
  assert.match(saved.text, /正文已保存到 assets\/dot\.png（11 字节，image\/png）/)
  assert.match(saved.text, /正文没有放进对话/)
  const onDisk = await readFile(path.join(root, 'assets', 'dot.png'))
  assert.deepEqual([...onDisk], [...png])

  // A target outside the workspace fails before the request is sent.
  const outside = await runTool(root, {
    id: '2',
    name: 'http',
    args: { url: `${base}/dot.png`, saveTo: '../escaped.png' }
  })
  assert.equal(outside.ok, false)
  assert.match(outside.text, /路径超出工作区/)
  assert.deepEqual(hits, ['/dot.png'])
})

test('http follows a redirect and names the hops it took', async (t) => {
  const { server, base } = await localServer((req, res) => {
    if (req.url === '/next') {
      res.writeHead(200, { 'content-type': 'text/plain; charset=utf-8' })
      res.end('landed')
      return
    }
    res.writeHead(302, { location: '/next' })
    res.end()
  })
  t.after(() => server.close())
  const result = await runTool(await workspace(), {
    id: '1',
    name: 'http',
    args: { url: `${base}/from` }
  })
  assert.equal(result.ok, true, result.text)
  assert.match(result.text, /→ 200 OK（跟随 1 次重定向）/)
  assert.match(result.text, new RegExp(`# 重定向 302 → ${base}/next`))
  assert.match(result.text, /landed/)
})

test('http stops after the redirect cap instead of looping', async (t) => {
  const { server, base } = await localServer((_req, res) => {
    res.writeHead(302, { location: '/again' })
    res.end()
  })
  t.after(() => server.close())
  const result = await runTool(await workspace(), {
    id: '1',
    name: 'http',
    args: { url: `${base}/loop` }
  })
  assert.equal(result.ok, false)
  assert.match(result.text, new RegExp(`重定向超过 ${MAX_HTTP_REDIRECTS} 次`))
})

test('http hands a page over as text instead of markup', async (t) => {
  const page = [
    '<!doctype html><html><head><title>文档</title>',
    '<style>body { color: red }</style></head>',
    '<body><h1>快速开始</h1><script>console.log("noise")</script>',
    '<p>先装依赖 &amp; 再跑测试。</p><ul><li>第一项</li><li>第二项</li></ul></body></html>'
  ].join('')
  const { server, base } = await localServer((_req, res) => {
    res.writeHead(200, { 'content-type': 'text/html; charset=utf-8' })
    res.end(page)
  })
  t.after(() => server.close())
  const result = await runTool(await workspace(), {
    id: '1',
    name: 'http',
    args: { url: `${base}/doc` }
  })
  assert.equal(result.ok, true, result.text)
  assert.match(result.text, /已转成文本/)
  assert.match(result.text, /快速开始/)
  assert.match(result.text, /先装依赖 & 再跑测试。/)
  assert.match(result.text, /第一项/)
  // Tags, scripts and styles are markup, not something to reason about.
  assert.doesNotMatch(result.text, /<h1>|console\.log|color: red/)
})

test('http clips a long body instead of flooding the context', async (t) => {
  const { server, base } = await localServer((_req, res) => {
    res.writeHead(200, { 'content-type': 'text/plain' })
    res.end('x'.repeat(MAX_HTTP_CHARS + 2_000))
  })
  t.after(() => server.close())
  const result = await runTool(await workspace(), {
    id: '1',
    name: 'http',
    args: { url: `${base}/big` }
  })
  assert.equal(result.ok, true, result.text)
  assert.match(result.text, new RegExp(`只显示前 ${MAX_HTTP_CHARS} 字符`))
  assert.ok(result.text.length < MAX_HTTP_CHARS + 400, 'the body is clipped')
})

test('http refuses other schemes, credentials and key-bearing headers', async () => {
  const root = await workspace()
  const refused = [
    { url: 'file:///c:/windows/win.ini' },
    { url: 'http://user:pass@example.com/' },
    { url: 'not a url' },
    { url: '' },
    { url: 'http://example.com/', headers: { Authorization: 'Bearer x' } },
    { url: 'http://example.com/', headers: { cookie: 'a=1' } },
    { url: 'http://example.com/', body: 'x' },
    { url: 'http://example.com/', method: 'DELETE' }
  ]
  for (const args of refused) {
    const result = await runTool(root, { id: '1', name: 'http', args })
    assert.equal(result.ok, false, JSON.stringify(args))
  }
})

test('asks before a request, and refuses it without a decision bridge', async () => {
  const root = await workspace()
  const stub = stubFetch([
    {
      choices: [
        {
          message: {
            content: null,
            tool_calls: [
              {
                id: 'call-1',
                type: 'function',
                function: {
                  name: 'http',
                  arguments: JSON.stringify({ url: 'http://127.0.0.1:1/none' })
                }
              }
            ]
          }
        }
      ]
    },
    { choices: [{ message: { content: 'ok' } }] }
  ])
  const asked: ApprovalRequest[] = []
  await runAgent(
    { prompt: '看个接口', apiKey: 'sk-test' },
    {
      fetchImpl: stub.fetchImpl,
      env: {},
      root,
      confirmActions: async (question) => {
        asked.push(question)
        return false
      }
    }
  )
  assert.equal(asked.length, 1)
  assert.equal(asked[0]!.tool, 'http')
  assert.match(asked[0]!.diff, /^\$ http GET http:\/\/127\.0\.0\.1:1\/none/)

  const silent = stubFetch([
    {
      choices: [
        {
          message: {
            content: null,
            tool_calls: [
              {
                id: 'call-2',
                type: 'function',
                function: {
                  name: 'http',
                  arguments: JSON.stringify({ url: 'http://127.0.0.1:1/none' })
                }
              }
            ]
          }
        }
      ]
    },
    { choices: [{ message: { content: 'ok' } }] }
  ])
  const result = await runAgent(
    { prompt: '看个接口', apiKey: 'sk-test' },
    { fetchImpl: silent.fetchImpl, env: {}, root }
  )
  assert.match(result.text, /ok/)
})

test('reports a non-zero exit as a failed call', async () => {
  const root = await workspace()
  const result = await runTool(root, {
    id: '1',
    name: 'shell',
    args: { command: 'node -e "process.exit(3)"' }
  })
  assert.equal(result.ok, false)
  assert.match(result.text, /^退出码 3/)
})

test('runs in a nested folder and refuses to leave the scope', async () => {
  const root = await workspace()
  await mkdir(path.join(root, 'sub'))
  const inside = await runTool(root, {
    id: '1',
    name: 'shell',
    args: { command: 'node -e "process.stdout.write(process.cwd())"', cwd: 'sub' }
  })
  assert.equal(inside.ok, true, inside.text)
  assert.match(inside.text, /sub/)

  const outside = await runTool(root, {
    id: '2',
    name: 'shell',
    args: { command: 'node -e "process.exit(0)"', cwd: '..' }
  })
  assert.equal(outside.ok, false)

  const missing = await runTool(root, {
    id: '3',
    name: 'shell',
    args: { command: 'node -e "process.exit(0)"', cwd: 'nope' }
  })
  assert.equal(missing.ok, false)
  assert.match(missing.text, /找不到运行目录/)
})

test('stops a command that runs too long', async () => {
  const root = await workspace()
  const result = await runTool(root, {
    id: '1',
    name: 'shell',
    args: { command: 'node -e "setTimeout(() => {}, 10000)"', timeoutMs: 300 }
  })
  assert.equal(result.ok, false)
  assert.match(result.text, /超时/)
})

test('feeds stdin once and closes it', async () => {
  const root = await workspace()
  const result = await runTool(root, {
    id: '1',
    name: 'shell',
    args: {
      command: `node -e "let all='';process.stdin.on('data',(chunk)=>all+=chunk).on('end',()=>process.stdout.write('got:'+all.trim()))"`,
      stdin: 'hello'
    }
  })
  assert.equal(result.ok, true, result.text)
  assert.match(result.text, /got:hello/)
})

test('runs a command in the background, reads it, and ends it', async () => {
  const root = await workspace()
  const started = await runTool(root, {
    id: '1',
    name: 'shell',
    args: {
      command: `node -e "let n=0;setInterval(() => console.log('tick'+(++n)), 100)"`,
      background: true,
      name: 'ticker'
    }
  })
  assert.equal(started.ok, true, started.text)
  assert.match(started.text, /已在后台启动 task-1（ticker）/)
  try {
    // The log is written by the task itself. Poll for the first tick instead of
    // sleeping a fixed time: under a loaded test run, starting `node` and its
    // timer can take longer than any single guess, which used to fail here.
    let output = { ok: false, text: '' }
    const deadline = Date.now() + 10_000
    while (Date.now() < deadline && !/tick\d/.test(output.text)) {
      await new Promise((resolve) => setTimeout(resolve, 100))
      output = await runTool(root, {
        id: '3',
        name: 'task',
        args: { action: 'output', id: 'task-1', lines: 5 }
      })
    }
    assert.equal(output.ok, true, output.text)
    assert.match(output.text, /tick\d/)
    const listed = await runTool(root, { id: '2', name: 'task', args: { action: 'list' } })
    assert.equal(listed.ok, true, listed.text)
    assert.match(listed.text, /task-1 \[running\]/)
  } finally {
    const stopped = await runTool(root, {
      id: '4',
      name: 'task',
      args: { action: 'stop', id: 'task-1' }
    })
    assert.equal(stopped.ok, true, stopped.text)
    assert.match(stopped.text, /已结束后台任务：task-1/)
  }
  const after = await runTool(root, { id: '5', name: 'task', args: { action: 'list' } })
  assert.match(after.text, /task-1 \[stopped\]/)
})

test('feeds a background task stdin, at start and later', async () => {
  const root = await workspace()
  const started = await runTool(root, {
    id: '1',
    name: 'shell',
    args: {
      command: `node -e "process.stdin.setEncoding('utf8');process.stdin.on('data',d=>process.stdout.write('got:'+d))"`,
      background: true,
      name: 'reader',
      stdin: 'first\n'
    }
  })
  assert.equal(started.ok, true, started.text)
  try {
    // The wrapper polls its control file, so give delivery a few seconds rather
    // than guessing one fixed sleep.
    const seen = async (text: string): Promise<boolean> => {
      const deadline = Date.now() + 8_000
      while (Date.now() < deadline) {
        const output = await runTool(root, {
          id: '2',
          name: 'task',
          args: { action: 'output', id: 'task-1' }
        })
        if (output.text.includes(text)) return true
        await new Promise((resolve) => setTimeout(resolve, 120))
      }
      return false
    }
    assert.ok(await seen('got:first'), 'the stdin given at start was delivered')

    const fed = await runTool(root, {
      id: '3',
      name: 'task',
      args: { action: 'input', id: 'task-1', text: 'second' }
    })
    assert.equal(fed.ok, true, fed.text)
    assert.match(fed.text, /已向 task-1 写入输入/)
    assert.ok(await seen('got:second'), 'the later input was delivered')
  } finally {
    await runTool(root, { id: '4', name: 'task', args: { action: 'stop', id: 'task-1' } })
  }
})

test('closes a background task stdin so it can finish', async () => {
  const root = await workspace()
  const started = await runTool(root, {
    id: '1',
    name: 'shell',
    args: {
      command: `node -e "process.stdin.on('end', () => { console.log('eof-seen'); process.exit(0) })"`,
      background: true,
      name: 'waiter'
    }
  })
  assert.equal(started.ok, true, started.text)
  try {
    const closed = await runTool(root, {
      id: '2',
      name: 'task',
      args: { action: 'close', id: 'task-1' }
    })
    assert.equal(closed.ok, true, closed.text)
    assert.match(closed.text, /已通知 task-1 关闭 stdin/)

    // Waiting for the record is what tells the caller it really ran to EOF.
    const waited = await runTool(root, {
      id: '3',
      name: 'task',
      args: { action: 'wait', id: 'task-1', timeoutMs: 20_000 }
    })
    assert.equal(waited.ok, true, waited.text)
    assert.match(waited.text, /task-1 已结束/)
    const output = await runTool(root, { id: '4', name: 'task', args: { action: 'output', id: 'task-1' } })
    assert.match(output.text, /eof-seen/)
  } finally {
    await runTool(root, { id: '5', name: 'task', args: { action: 'stop', id: 'task-1' } })
  }
})

test('refuses input for a task that already ended', async () => {
  const root = await workspace()
  const tasks = path.join(root, '.dsh-flow', 'tasks')
  await mkdir(tasks, { recursive: true })
  const log = path.join(tasks, 'task-1.log')
  await writeFile(log, '', 'utf8')
  await writeFile(
    path.join(tasks, 'task-1.json'),
    JSON.stringify({ id: 'task-1', command: 'x', cwd: root, pid: 0, startedAt: '', log }),
    'utf8'
  )
  const result = await runTool(root, {
    id: '1',
    name: 'task',
    args: { action: 'input', id: 'task-1', text: 'x' }
  })
  assert.equal(result.ok, false)
  assert.match(result.text, /已经不在运行/)
})

test('names the tasks it knows when asked for an unknown one', async () => {
  const root = await workspace()
  const missing = await runTool(root, { id: '1', name: 'task', args: { action: 'output', id: 'task-9' } })
  assert.equal(missing.ok, false)
  assert.match(missing.text, /没有这个后台任务：task-9/)
  const badAction = await runTool(root, { id: '2', name: 'task', args: { action: 'nope' } })
  assert.equal(badAction.ok, false)
  assert.match(badAction.text, /action 必须是/)
})

test('refuses a command that names a path outside the sandbox', async () => {
  const root = await workspace()
  const sandbox = { roots: [root] }
  // The temporary folder above the workspace exists but is not allowlisted.
  const outside = path.dirname(root)
  const escaped = await runToolScoped(
    root,
    root,
    { id: '1', name: 'shell', args: { command: `node -e "process.stdout.write('x')" "${outside}"` } },
    [],
    undefined,
    sandbox
  )
  assert.equal(escaped.ok, false, escaped.text)
  assert.match(escaped.text, /沙箱拒绝了这条命令/)
  assert.match(escaped.text, /设置 → 文件 → 命令允许的路径/)

  // A URL is not a place on disk, so it must not trip the guard.
  const url = await runToolScoped(
    root,
    root,
    {
      id: '2',
      name: 'node',
      args: { code: `const target = 'https://example.com/a'\nprocess.stdout.write(target)` }
    },
    [],
    undefined,
    sandbox
  )
  assert.equal(url.ok, true, url.text)
})

test('lets a command reach an allowlisted folder', async () => {
  const root = await workspace()
  const extra = await workspace()
  const note = path.join(extra, 'note.txt')
  await writeFile(note, 'allowed')
  const call = async (sandbox: Sandbox) =>
    await runToolScoped(
      root,
      root,
      {
        id: '1',
        name: 'shell',
        args: {
          command: `node -e "process.stdout.write(require('node:fs').readFileSync(process.argv[1], 'utf8'))" "${note}"`
        }
      },
      [],
      undefined,
      sandbox
    )
  const refused = await call({ roots: [root] })
  assert.equal(refused.ok, false, refused.text)
  assert.match(refused.text, /沙箱拒绝/)
  const allowed = await call({ roots: [root, extra] })
  assert.equal(allowed.ok, true, allowed.text)
  assert.match(allowed.text, /allowed/)

  // The allowlisted folder is also a place a command may stand in.
  const inside = async (sandbox: Sandbox) =>
    await runToolScoped(
      root,
      root,
      {
        id: '2',
        name: 'shell',
        args: { command: `node -e "process.stdout.write(process.cwd())"`, cwd: extra }
      },
      [],
      undefined,
      sandbox
    )
  const blockedCwd = await inside({ roots: [root] })
  assert.equal(blockedCwd.ok, false, blockedCwd.text)
  const allowedCwd = await inside({ roots: [root, extra] })
  assert.equal(allowedCwd.ok, true, allowedCwd.text)

  // A `..` step that stays inside the roots is fine; one that leaves them is not.
  await mkdir(path.join(root, 'sub'), { recursive: true })
  const upOne = async (command: string) =>
    await runToolScoped(
      root,
      root,
      { id: '3', name: 'shell', args: { command, cwd: 'sub' } },
      [],
      undefined,
      { roots: [root] }
    )
  const insideParent = await upOne('cd .. && echo inside')
  assert.equal(insideParent.ok, true, insideParent.text)
  const leaving = await upOne('cd ../.. && echo outside')
  assert.equal(leaving.ok, false, leaving.text)
  assert.match(leaving.text, /沙箱拒绝/)
})

test('streams shell output while the command is still running', async () => {
  const root = await workspace()
  const chunks: string[] = []
  const result = await runToolScoped(
    root,
    root,
    {
      id: '1',
      name: 'shell',
      args: {
        command: `node -e "process.stdout.write('first\\n'); setTimeout(() => process.stdout.write('second\\n'), 250)"`
      }
    },
    [],
    (text) => chunks.push(text)
  )
  assert.equal(result.ok, true, result.text)
  assert.ok(chunks.length > 0, 'live output arrived before the command finished')
  assert.match(chunks.join(''), /first/)
  assert.match(chunks.join(''), /second/)
  assert.match(result.text, /first/)
  assert.match(result.text, /second/)
})

test('decodes a multi-byte character split across output chunks', async () => {
  const root = await workspace()
  const chunks: string[] = []
  const result = await runToolScoped(
    root,
    root,
    {
      id: '1',
      name: 'shell',
      args: {
        command: `node -e "process.stdout.write(Buffer.from([0xE4,0xB8])); setTimeout(() => process.stdout.write(Buffer.from([0xAD])), 40)"`
      }
    },
    [],
    (text) => chunks.push(text)
  )
  assert.equal(result.ok, true, result.text)
  assert.match(result.text, /中/)
  assert.doesNotMatch(result.text, /\uFFFD/, 'no replacement characters')
  assert.doesNotMatch(chunks.join(''), /\uFFFD/, 'live output is decoded across chunks too')
})

test('decodes GBK output from a Windows command', async () => {
  const root = await workspace()
  const chunks: string[] = []
  const result = await runToolScoped(
    root,
    root,
    {
      id: '1',
      name: 'shell',
      args: {
        command: `node -e "process.stdout.write('name: '); setTimeout(() => process.stdout.write(Buffer.from([0xD6,0xD0,0xCE,0xC4])), 30)"`
      }
    },
    [],
    (text) => chunks.push(text)
  )
  assert.equal(result.ok, true, result.text)
  assert.match(result.text, /name: 中文/)
  assert.doesNotMatch(result.text, /\uFFFD/)
  assert.match(chunks.join(''), /中文/, 'live output falls back to GB18030')
  assert.doesNotMatch(chunks.join(''), /\uFFFD/)
})

test('a GBK pair split by a live flush is not broken', async () => {
  const root = await workspace()
  const chunks: string[] = []
  // The GBK bytes arrive after the live-flush interval has elapsed, so this
  // chunk goes through the preview decoder without the final flush: that path
  // used to hold back the last byte as a UTF-8 lead and paint a replacement
  // character where the second half of the character should be.
  const result = await runToolScoped(
    root,
    root,
    {
      id: '1',
      name: 'shell',
      args: {
        command: `node -e "process.stdout.write('name: '); setTimeout(() => process.stdout.write(Buffer.from([0xD6,0xD0,0xCE,0xC4])), 260)"`
      }
    },
    [],
    (text) => chunks.push(text)
  )
  assert.equal(result.ok, true, result.text)
  assert.match(result.text, /name: 中文/)
  assert.match(chunks.join(''), /中文/, `live output was ${JSON.stringify(chunks.join(''))}`)
  assert.doesNotMatch(chunks.join(''), /\uFFFD/, 'no replacement characters in the preview')
})

test('reads a GBK file as text instead of mojibake', async () => {
  const root = await workspace()
  await writeFile(path.join(root, 'gbk.txt'), Buffer.from([0xd6, 0xd0, 0xce, 0xc4, 0x0a]))
  const result = await runToolScoped(root, root, { id: '1', name: 'read', args: { path: 'gbk.txt' } }, [])
  assert.equal(result.ok, true, result.text)
  assert.match(result.text, /1\t中文/)
  assert.match(result.text, /GB18030/)
  const edit = await runToolScoped(
    root,
    root,
    { id: '2', name: 'edit', args: { path: 'gbk.txt', old: '中文', new: 'English' } },
    []
  )
  assert.equal(edit.ok, false)
  assert.match(edit.text, /不是 UTF-8/)
  assert.deepEqual(await readFile(path.join(root, 'gbk.txt')), Buffer.from([0xd6, 0xd0, 0xce, 0xc4, 0x0a]))
})

test('decodes a stream that mixes UTF-8 and GBK lines', async () => {
  const root = await workspace()
  const command = `node -e "process.stdout.write(Buffer.concat([Buffer.from([0xE4,0xB8,0xAD,0xE6,0x96,0x87]), Buffer.from('\\n')])); process.stdout.write(Buffer.concat([Buffer.from([0xD6,0xD0,0xCE,0xC4]), Buffer.from('\\n')]))"`
  const result = await runToolScoped(root, root, { id: '1', name: 'shell', args: { command } }, [])
  assert.equal(result.ok, true, result.text)
  const lines = result.text.split('\n').filter((line) => line.includes('中文'))
  assert.equal(lines.length, 2, result.text)
  assert.doesNotMatch(result.text, /\uFFFD/)
})

test('writes PowerShell scripts with a BOM so 5.1 reads them as UTF-8', async () => {
  const root = await workspace()
  const written = await runToolScoped(
    root,
    root,
    { id: '1', name: 'write', args: { path: 'read_sysinfo.ps1', content: "Write-Output '系统信息'" } },
    []
  )
  assert.equal(written.ok, true, written.text)
  const bytes = await readFile(path.join(root, 'read_sysinfo.ps1'))
  assert.deepEqual([...bytes.subarray(0, 3)], [0xef, 0xbb, 0xbf])
  assert.match(bytes.toString('utf8'), /系统信息/)
  const ascii = await runToolScoped(
    root,
    root,
    { id: '2', name: 'write', args: { path: 'plain.ps1', content: "Write-Output 'hi'" } },
    []
  )
  assert.equal(ascii.ok, true, ascii.text)
  const plain = await readFile(path.join(root, 'plain.ps1'))
  assert.deepEqual([...plain.subarray(0, 1)], [0x57], 'ASCII-only scripts keep no BOM')
})

test('adds the BOM when an edit puts Chinese into a PowerShell script', async () => {
  const root = await workspace()
  await writeFile(path.join(root, 'edit.ps1'), "Write-Output 'hi'\n", 'utf8')
  const edited = await runToolScoped(
    root,
    root,
    { id: '1', name: 'edit', args: { path: 'edit.ps1', old: 'hi', new: '系统信息' } },
    []
  )
  assert.equal(edited.ok, true, edited.text)
  const bytes = await readFile(path.join(root, 'edit.ps1'))
  assert.deepEqual([...bytes.subarray(0, 3)], [0xef, 0xbb, 0xbf])
  assert.match(bytes.toString('utf8'), /系统信息/)
  const again = await runToolScoped(
    root,
    root,
    { id: '2', name: 'edit', args: { path: 'edit.ps1', old: '系统信息', new: '系统信息2' } },
    []
  )
  assert.equal(again.ok, true, again.text)
  const twice = await readFile(path.join(root, 'edit.ps1'))
  assert.deepEqual([...twice.subarray(0, 6)], [0xef, 0xbb, 0xbf, 0x57, 0x72, 0x69], 'one BOM only')
})

test('keeps only the ends of long command output', async () => {
  const root = await workspace()
  const result = await runTool(root, {
    id: '1',
    name: 'shell',
    args: { command: `node -e "process.stdout.write('x'.repeat(40000))"` }
  })
  assert.equal(result.ok, true, result.text)
  assert.match(result.text, /省略/)
  assert.ok(result.text.length <= MAX_SHELL_CHARS + 200, `${result.text.length} characters`)
})

test('does not hand secrets to a command', async () => {
  const root = await workspace()
  process.env.DSH_TEST_API_KEY = 'secret-value'
  try {
    const result = await runTool(root, {
      id: '1',
      name: 'shell',
      args: { command: `node -e "process.stdout.write(process.env.DSH_TEST_API_KEY || 'unset')"` }
    })
    assert.equal(result.ok, true, result.text)
    assert.match(result.text, /unset/)
    assert.doesNotMatch(result.text, /secret-value/)
  } finally {
    delete process.env.DSH_TEST_API_KEY
  }
})

test('refuses an empty command', async () => {
  const root = await workspace()
  const result = await runTool(root, { id: '1', name: 'shell', args: { command: '  ' } })
  assert.equal(result.ok, false)
  assert.match(result.text, /不能为空/)
})

test('reads a file with line numbers', async () => {
  const root = await workspace()
  await writeFile(path.join(root, 'a.txt'), 'one\ntwo\nthree\n', 'utf8')
  const result = await runTool(root, { id: '1', name: 'read', args: { path: 'a.txt' } })
  assert.equal(result.ok, true)
  assert.match(result.text, /^1\tone\n2\ttwo/)
})

test('reads an empty file without inventing a line', async () => {
  const root = await workspace()
  await writeFile(path.join(root, 'empty.txt'), '', 'utf8')
  const result = await runTool(root, { id: '1', name: 'read', args: { path: 'empty.txt' } })
  assert.equal(result.ok, true)
  assert.equal(result.text, '（文件是空的）')
})

test('explains an offset past the end instead of claiming more lines', async () => {
  const root = await workspace()
  await writeFile(path.join(root, 'short.txt'), 'one\ntwo\nthree', 'utf8')
  const result = await runTool(root, {
    id: '1',
    name: 'read',
    args: { path: 'short.txt', offset: 9 }
  })
  assert.equal(result.ok, true)
  assert.match(result.text, /一共 3 行/)
  assert.doesNotMatch(result.text, /已显示到 9 行/)
})

test('writes a file and creates its directory', async () => {
  const root = await workspace()
  const result = await runTool(root, {
    id: '1',
    name: 'write',
    args: { path: 'nested/b.txt', content: 'hello' }
  })
  assert.equal(result.ok, true)
  assert.equal(await readFile(path.join(root, 'nested', 'b.txt'), 'utf8'), 'hello')
})

test('refuses to edit a file past the size cap without offering a diff', async () => {
  const root = await workspace()
  const big = Buffer.alloc(MAX_EDIT_BYTES + 1, 0x41)
  await writeFile(path.join(root, 'huge.txt'), big)
  const preview = await previewTool(root, {
    id: '1',
    name: 'edit',
    args: { path: 'huge.txt', old: 'A', new: 'B' }
  })
  assert.equal(preview, null, 'no confirmation card for a doomed edit')

  const edit = await runTool(root, {
    id: '2',
    name: 'edit',
    args: { path: 'huge.txt', old: 'A', new: 'B' }
  })
  assert.equal(edit.ok, false)
  assert.match(edit.text, /编辑上限/)
  assert.deepEqual(await readFile(path.join(root, 'huge.txt')), big)
})

test('counts only the lines a character-clipped read really shows', async () => {
  const root = await workspace()
  const width = 'x'.repeat(1000)
  const lines = Array.from({ length: 400 }, () => width)
  await writeFile(path.join(root, 'wide.txt'), lines.join('\n'), 'utf8')

  const result = await runTool(root, { id: '1', name: 'read', args: { path: 'wide.txt' } })
  assert.equal(result.ok, true)
  assert.ok(result.text.length <= 200_500, `${result.text.length} characters`)
  const tail = /共 400 行，已显示到 (\d+) 行/.exec(result.text)
  assert.ok(tail, result.text.slice(-120))
  const shown = Number(tail[1])
  assert.ok(shown > 0 && shown < 400, `claimed ${shown} lines`)
  const lastLine = result.text.split('\n').find((line) => line.startsWith(`${shown}\t`))
  assert.ok(lastLine, 'the last line it claims is in the output')
  assert.ok(lastLine.endsWith(width), 'that line is not cut in half')
})

test('refuses a file past the read size cap instead of loading it', async () => {
  const root = await workspace()
  await writeFile(path.join(root, 'huge.log'), '')
  await truncate(path.join(root, 'huge.log'), MAX_READ_BYTES + 1)
  const result = await runTool(root, { id: '1', name: 'read', args: { path: 'huge.log' } })
  assert.equal(result.ok, false)
  assert.match(result.text, /文件太大/)
  assert.match(result.text, /grep/)
})

test('refuses to read or edit a binary file', async () => {
  const root = await workspace()
  const bytes = Buffer.from([0x00, 0x01, 0x02, 0x41])
  await writeFile(path.join(root, 'bin.dat'), bytes)
  const read = await runTool(root, { id: '1', name: 'read', args: { path: 'bin.dat' } })
  assert.equal(read.ok, false)
  assert.match(read.text, /二进制/)

  const edit = await runTool(root, {
    id: '2',
    name: 'edit',
    args: { path: 'bin.dat', old: 'A', new: 'B' }
  })
  assert.equal(edit.ok, false)
  assert.match(edit.text, /二进制/)
  assert.deepEqual(await readFile(path.join(root, 'bin.dat')), bytes)
})

test('edits one unique occurrence', async () => {
  const root = await workspace()
  await writeFile(path.join(root, 'c.txt'), 'name: old\n', 'utf8')
  const result = await runTool(root, {
    id: '1',
    name: 'edit',
    args: { path: 'c.txt', old: 'old', new: 'new' }
  })
  assert.equal(result.ok, true)
  assert.equal(await readFile(path.join(root, 'c.txt'), 'utf8'), 'name: new\n')
})

test('matches a CRLF file when the model writes LF, and keeps CRLF', async () => {
  const root = await workspace()
  await writeFile(path.join(root, 'crlf.txt'), 'one\r\ntwo\r\n', 'utf8')
  const result = await runTool(root, {
    id: '1',
    name: 'edit',
    args: { path: 'crlf.txt', old: 'one\ntwo', new: 'three\nfour' }
  })
  assert.equal(result.ok, true, result.text)
  assert.equal(await readFile(path.join(root, 'crlf.txt'), 'utf8'), 'three\r\nfour\r\n')
  assert.doesNotMatch(result.text, /找不到/)
})

test('replaceAll changes every occurrence while the default still requires one', async () => {
  const root = await workspace()
  await writeFile(path.join(root, 'many.txt'), 'a\nb\na\n', 'utf8')

  const refused = await runTool(root, {
    id: '1',
    name: 'edit',
    args: { path: 'many.txt', old: 'a', new: 'z' }
  })
  assert.equal(refused.ok, false)
  assert.match(refused.text, /不止一次/)

  const all = await runTool(root, {
    id: '2',
    name: 'edit',
    args: { path: 'many.txt', old: 'a', new: 'z', replaceAll: true }
  })
  assert.equal(all.ok, true, all.text)
  assert.equal(await readFile(path.join(root, 'many.txt'), 'utf8'), 'z\nb\nz\n')
})

test('edits several places in one call', async () => {
  const root = await workspace()
  await writeFile(path.join(root, 'batch.txt'), 'alpha\nbeta\ngamma\n', 'utf8')
  const result = await runTool(root, {
    id: '1',
    name: 'edit',
    args: {
      path: 'batch.txt',
      edits: [
        { old: 'alpha', new: 'ALPHA' },
        { old: 'gamma', new: 'GAMMA' }
      ]
    }
  })
  assert.equal(result.ok, true, result.text)
  assert.match(result.text, /2 处/)
  assert.equal(await readFile(path.join(root, 'batch.txt'), 'utf8'), 'ALPHA\nbeta\nGAMMA\n')
})

test('a batch edit stops before writing when one target is missing', async () => {
  const root = await workspace()
  await writeFile(path.join(root, 'batch.txt'), 'alpha\nbeta\n', 'utf8')
  const preview = await previewTool(root, {
    id: '1',
    name: 'edit',
    args: {
      path: 'batch.txt',
      edits: [
        { old: 'alpha', new: 'ALPHA' },
        { old: 'missing', new: 'x' }
      ]
    }
  })
  assert.equal(preview, null, 'a partial batch is not offered')

  const result = await runTool(root, {
    id: '2',
    name: 'edit',
    args: {
      path: 'batch.txt',
      edits: [
        { old: 'alpha', new: 'ALPHA' },
        { old: 'missing', new: 'x' }
      ]
    }
  })
  assert.equal(result.ok, false)
  assert.match(result.text, /第 2 处/)
  assert.equal(await readFile(path.join(root, 'batch.txt'), 'utf8'), 'alpha\nbeta\n')
})

test('previews a batch edit as one diff', async () => {
  const root = await workspace()
  await writeFile(path.join(root, 'batch.txt'), 'alpha\nbeta\n', 'utf8')
  const preview = await previewTool(root, {
    id: '1',
    name: 'edit',
    args: {
      path: 'batch.txt',
      edits: [
        { old: 'alpha', new: 'ALPHA' },
        { old: 'beta', new: 'BETA' }
      ]
    }
  })
  assert.ok(preview)
  assert.match(preview.diff, /^- alpha$/m)
  assert.match(preview.diff, /^- beta$/m)
  assert.match(preview.diff, /^\+ ALPHA$/m)
  assert.match(preview.diff, /^\+ BETA$/m)
  assert.equal(preview.after, 'ALPHA\nBETA\n')
})

test('edits by line number without touching the other lines', async () => {
  const root = await workspace()
  await writeFile(path.join(root, 'lines.txt'), 'one\ntwo\nthree\nfour\n', 'utf8')
  const result = await runTool(root, {
    id: '1',
    name: 'edit',
    args: { path: 'lines.txt', startLine: 2, endLine: 3, content: 'TWO\nTHREE' }
  })
  assert.equal(result.ok, true, result.text)
  assert.match(result.text, /第 2–3 行/)
  assert.equal(await readFile(path.join(root, 'lines.txt'), 'utf8'), 'one\nTWO\nTHREE\nfour\n')
})

test('a line edit deletes lines, keeps CRLF, and can insert', async () => {
  const root = await workspace()
  await writeFile(path.join(root, 'crlf.txt'), 'a\r\nb\r\nc\r\n', 'utf8')
  const removed = await runTool(root, {
    id: '1',
    name: 'edit',
    args: { path: 'crlf.txt', startLine: 2, content: '' }
  })
  assert.equal(removed.ok, true, removed.text)
  assert.match(removed.text, /删除/)
  assert.equal(await readFile(path.join(root, 'crlf.txt'), 'utf8'), 'a\r\nc\r\n')

  const added = await runTool(root, {
    id: '2',
    name: 'edit',
    args: { path: 'crlf.txt', insertAfter: 1, content: 'x\ny' }
  })
  assert.equal(added.ok, true, added.text)
  assert.equal(await readFile(path.join(root, 'crlf.txt'), 'utf8'), 'a\r\nx\r\ny\r\nc\r\n')
})

test('inserts at the top and after the last line', async () => {
  const root = await workspace()
  await writeFile(path.join(root, 't.txt'), 'b\n', 'utf8')
  const top = await runTool(root, {
    id: '1',
    name: 'edit',
    args: { path: 't.txt', insertAfter: 0, content: 'a' }
  })
  assert.equal(top.ok, true, top.text)
  assert.match(top.text, /文件开头/)
  assert.equal(await readFile(path.join(root, 't.txt'), 'utf8'), 'a\nb\n')

  const end = await runTool(root, {
    id: '2',
    name: 'edit',
    args: { path: 't.txt', insertAfter: 2, content: 'c' }
  })
  assert.equal(end.ok, true, end.text)
  assert.equal(await readFile(path.join(root, 't.txt'), 'utf8'), 'a\nb\nc\n')
})

test('refuses to mix text and line addressing in one edit', async () => {
  const root = await workspace()
  await writeFile(path.join(root, 'm.txt'), 'x\n', 'utf8')
  const mixed = await runTool(root, {
    id: '1',
    name: 'edit',
    args: { path: 'm.txt', old: 'x', new: 'y', startLine: 1, content: 'z' }
  })
  assert.equal(mixed.ok, false)
  assert.match(mixed.text, /二选一/)
  assert.equal(await readFile(path.join(root, 'm.txt'), 'utf8'), 'x\n')
})

test('copies a binary file byte for byte and refuses a silent overwrite', async () => {
  const root = await workspace()
  const bytes = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x00, 0xff])
  await writeFile(path.join(root, 'logo.png'), bytes)
  const copied = await runTool(root, {
    id: '1',
    name: 'copy',
    args: { from: 'logo.png', to: 'assets/logo.png' }
  })
  assert.equal(copied.ok, true, copied.text)
  assert.deepEqual(await readFile(path.join(root, 'assets', 'logo.png')), bytes)

  const again = await runTool(root, {
    id: '2',
    name: 'copy',
    args: { from: 'logo.png', to: 'assets/logo.png' }
  })
  assert.equal(again.ok, false)
  assert.match(again.text, /目标已存在/)

  const replaced = await runTool(root, {
    id: '3',
    name: 'copy',
    args: { from: 'logo.png', to: 'assets/logo.png', overwrite: true }
  })
  assert.equal(replaced.ok, true, replaced.text)
})

test('creates a folder and says when it is already there', async () => {
  const root = await workspace()
  const made = await runTool(root, { id: '1', name: 'mkdir', args: { path: 'out/reports' } })
  assert.equal(made.ok, true, made.text)
  assert.ok(existsSync(path.join(root, 'out', 'reports')))

  const again = await runTool(root, { id: '2', name: 'mkdir', args: { path: 'out/reports' } })
  assert.equal(again.ok, true, again.text)
  assert.match(again.text, /已存在/)
})

test('reads an image and hands it back for the model', async () => {
  const root = await workspace()
  await writeFile(path.join(root, 'dot.png'), DOT_PNG)
  const result = await runTool(root, { id: '1', name: 'read_image', args: { path: 'dot.png' } })
  assert.equal(result.ok, true, result.text)
  assert.equal(result.image?.mime, 'image/png')
  assert.ok((result.image?.data.length ?? 0) > 20, 'the bytes travel base64 encoded')
  assert.match(result.text, /附在下一条消息/)
})

test('a read-only conversation keeps its screenshot out of the workspace', () => {
  const before = process.env.DSH_SANDBOX_READONLY
  const workspaceBefore = process.env.DSH_SANDBOX_WORKSPACE
  const normal = shotsDir('C:/work/chat-1')
  assert.match(normal.dir.replace(/\\/g, '/'), /chat-1\/\.dsh-flow\/shots$/)
  assert.equal(normal.label, '.dsh-flow/shots')
  try {
    process.env.DSH_SANDBOX_READONLY = '1'
    process.env.DSH_SANDBOX_WORKSPACE = 'C:/work'
    const readOnly = shotsDir('C:/work/chat-1')
    // The promise of 「仅可查看」 is that the workspace itself stays untouched.
    assert.match(readOnly.dir.replace(/\\/g, '/'), /\.dsh-flow\/sandbox-tmp\/shots$/)
    assert.equal(readOnly.label, '.dsh-flow/sandbox-tmp/shots')
  } finally {
    if (before === undefined) delete process.env.DSH_SANDBOX_READONLY
    else process.env.DSH_SANDBOX_READONLY = before
    if (workspaceBefore === undefined) delete process.env.DSH_SANDBOX_WORKSPACE
    else process.env.DSH_SANDBOX_WORKSPACE = workspaceBefore
  }
})

test('refuses a picture whose area would blow the vision limit', async () => {
  const root = await workspace()
  // A PNG header (signature + IHDR) is enough: the size is read before the pixels
  // are, so the file can stay tiny while claiming to be 9000×9000.
  const wide = Buffer.alloc(32)
  Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]).copy(wide, 0)
  wide.writeUInt32BE(13, 8)
  wide.write('IHDR', 12, 'latin1')
  wide.writeUInt32BE(9_000, 16)
  wide.writeUInt32BE(9_000, 20)
  await writeFile(path.join(root, 'huge.png'), wide)
  const tooBig = await runTool(root, {
    id: '1',
    name: 'read_image',
    args: { path: 'huge.png' }
  })
  assert.equal(tooBig.ok, false)
  assert.match(tooBig.text, /9000×9000/)
  assert.match(tooBig.text, /超过图片上限/)
})

test('refuses a text file and an oversized image', async () => {
  const root = await workspace()
  await writeFile(path.join(root, 'notes.txt'), 'hello', 'utf8')
  const text = await runTool(root, { id: '1', name: 'read_image', args: { path: 'notes.txt' } })
  assert.equal(text.ok, false)
  assert.match(text.text, /不是位图/)

  const big = Buffer.concat([DOT_PNG, Buffer.alloc(MAX_IMAGE_BYTES + 1)])
  await writeFile(path.join(root, 'big.png'), big)
  const huge = await runTool(root, { id: '2', name: 'read_image', args: { path: 'big.png' } })
  assert.equal(huge.ok, false)
  assert.match(huge.text, /图片太大/)
})

test('pages accepts one page or a range and refuses nonsense', () => {
  assert.deepEqual(parsePageRange(''), { first: 1, last: MAX_PDF_PAGES })
  assert.deepEqual(parsePageRange('3'), { first: 3, last: 3 })
  assert.deepEqual(parsePageRange(' 3 - 7 '), { first: 3, last: 7 })
  assert.throws(() => parsePageRange('0'), /从 1 开始/)
  assert.throws(() => parsePageRange('7-3'), /结束页/)
  assert.throws(() => parsePageRange('x'), /3 或 3-7/)
  assert.throws(() => parsePageRange('1-99'), /最多读 40 页/)
})

test('the missing-extractor message names both ways to fix it', () => {
  const text = missingPdfToolMessage()
  assert.match(text, /pdftotext/)
  assert.match(text, /pip install pypdf/)
  assert.match(text, /附件/)
})

test('read extracts a PDF and honours the page range', async () => {
  const root = await workspace()
  await writeFile(path.join(root, 'spec.pdf'), '%PDF-1.4\n1 0 obj\n')
  const calls: Array<[number, number]> = []
  /** An extractor that behaves like a document of `total` pages. */
  const document = (total: number) => async (_file: string, first: number, last: number) => {
    calls.push([first, last])
    const pages: string[] = []
    for (let page = first; page <= Math.min(last, total); page += 1) pages.push(`page ${page} text`)
    return { text: pages.join('\f'), total, via: 'stub' }
  }
  try {
    usePdfRunner(document(5))
    const all = await runTool(root, { id: '1', name: 'read', args: { path: 'spec.pdf' } })
    assert.equal(all.ok, true, all.text)
    assert.match(all.text, /第 1–5 页，共 5 页/)
    assert.match(all.text, /—— 第 1 页 ——/)
    assert.match(all.text, /page 1 text/)
    assert.match(all.text, /—— 第 5 页 ——/)
    assert.doesNotMatch(all.text, /没读/, 'a five-page document is read whole')
    assert.deepEqual(calls[0], [1, MAX_PDF_PAGES])

    const ranged = await runTool(root, {
      id: '2',
      name: 'read',
      args: { path: 'spec.pdf', pages: '2-2' }
    })
    assert.equal(ranged.ok, true, ranged.text)
    assert.deepEqual(calls[1], [2, 2])
    assert.match(ranged.text, /第 2–2 页/)
    assert.match(ranged.text, /page 2 text/)

    // A document longer than the page cap says how much is left.
    usePdfRunner(document(100))
    const long = await runTool(root, { id: '3', name: 'read', args: { path: 'spec.pdf' } })
    assert.equal(long.ok, true, long.text)
    assert.match(long.text, /第 1–40 页，共 100 页/)
    assert.match(long.text, /还有 60 页没读/)
    assert.match(long.text, /pages: "41-80"/)
  } finally {
    usePdfRunner(null)
  }
})

test('a scanned PDF says the pages hold no text', async () => {
  const root = await workspace()
  await writeFile(path.join(root, 'scan.pdf'), '%PDF-1.4\n')
  usePdfRunner(async () => ({ text: '\f', total: 2, via: 'stub' }))
  try {
    const result = await runTool(root, { id: '1', name: 'read', args: { path: 'scan.pdf' } })
    assert.equal(result.ok, true, result.text)
    assert.match(result.text, /没有可提取的文字/)
    assert.match(result.text, /read_image|附件/)
  } finally {
    usePdfRunner(null)
  }
})

test('a long PDF page is cut at the character cap', async () => {
  const root = await workspace()
  await writeFile(path.join(root, 'big.pdf'), '%PDF-1.4\n')
  usePdfRunner(async () => ({ text: 'x'.repeat(MAX_PDF_CHARS + 100), via: 'stub' }))
  try {
    const result = await runTool(root, { id: '1', name: 'read', args: { path: 'big.pdf' } })
    assert.equal(result.ok, true, result.text)
    assert.match(result.text, /这一页没显示完/)
    assert.match(result.text, /后面的页没读/)
    // The cap holds: the answer stays near the limit instead of carrying the page.
    assert.ok(result.text.length < MAX_PDF_CHARS + 400, `${result.text.length}`)
  } finally {
    usePdfRunner(null)
  }
})

test('grep skips a PDF even when its first bytes look like text', async () => {
  const root = await workspace()
  // A header followed by plain ASCII and no NUL in the first 8 KB: the byte guard
  // alone would have scanned this as text and matched inside binary noise.
  const body = Array.from({ length: 40 }, () => 'needle-inside-pdf').join('\n')
  await writeFile(path.join(root, 'inside.pdf'), `%PDF-1.4\n${body}\n`)
  await writeFile(path.join(root, 'notes.txt'), 'needle-here\n')
  const result = await runTool(root, { id: '1', name: 'grep', args: { pattern: 'needle' } })
  assert.equal(result.ok, true, result.text)
  assert.match(result.text, /notes\.txt/)
  assert.doesNotMatch(result.text, /inside\.pdf/)
})

test('read_image points a PDF at read', async () => {
  const root = await workspace()
  await writeFile(path.join(root, 'doc.pdf'), '%PDF-1.4\n')
  const result = await runTool(root, { id: '1', name: 'read_image', args: { path: 'doc.pdf' } })
  assert.equal(result.ok, false)
  assert.match(result.text, /这是 PDF/)
  assert.match(result.text, /pages/)
})

test('a command may not read a credential file', async () => {
  // The scan looks at whole tokens, `--flag=value` and quoted strings, and leaves
  // prose alone: a commit message that says "secret" is not a file.
  assert.deepEqual(credentialMentions('type .env'), ['.env'])
  assert.deepEqual(credentialMentions('node --env-file=.env server.js'), ['.env'])
  assert.deepEqual(credentialMentions("python -c \"print(open('.env').read())\""), ['.env'])
  assert.deepEqual(credentialMentions('copy .env.example .env.template'), [])
  assert.deepEqual(credentialMentions('git commit -m "fix secret handling"'), [])
  assert.deepEqual(credentialMentions('npm run build'), [])
  assert.deepEqual(credentialMentions('type %USERPROFILE%\\.ssh\\id_rsa'), ['id_rsa'])

  const root = await workspace()
  const refused = await runTool(root, { id: '1', name: 'shell', args: { command: 'echo .env' } })
  assert.equal(refused.ok, false)
  assert.match(refused.text, /凭据文件（\.env）/)

  // An example file holds no secret and still runs.
  const allowed = await runTool(root, {
    id: '2',
    name: 'shell',
    args: { command: 'echo .env.example' }
  })
  assert.equal(allowed.ok, true, allowed.text)

  const snippet = await runTool(root, {
    id: '3',
    name: 'python',
    args: { code: "print(open('.env').read())" }
  })
  assert.equal(snippet.ok, false)
  assert.match(snippet.text, /凭据文件/)

  // A background task is refused before anything is started.
  const background = await runTool(root, {
    id: '4',
    name: 'shell',
    args: { command: 'type .env', background: true }
  })
  assert.equal(background.ok, false)
  assert.match(background.text, /没有执行/)
})

test('reports runtimes and the project markers it can see', async () => {
  const root = await workspace()
  await writeFile(
    path.join(root, 'package.json'),
    JSON.stringify({ scripts: { dev: 'vite', test: 'node --test' } }),
    'utf8'
  )
  // A JVM project next to the Node one: the report names it so the model knows
  // which runner `run_tests` would pick.
  await writeFile(path.join(root, 'pom.xml'), '<project />\n', 'utf8')
  const result = await runTool(root, { id: '1', name: 'env', args: {} })
  assert.equal(result.ok, true, result.text)
  assert.match(result.text, /平台：/)
  assert.match(result.text, /node v?\d/)
  assert.match(result.text, /package\.json/)
  assert.match(result.text, /dev、test/)
  assert.match(result.text, /pom\.xml：Maven 项目/)
})

test('replaces the same text across matching files and names what it skipped', async () => {
  const root = await workspace()
  await mkdir(path.join(root, 'src'), { recursive: true })
  await writeFile(path.join(root, 'src', 'a.ts'), 'const x = old\n', 'utf8')
  await writeFile(path.join(root, 'src', 'b.ts'), 'old()\nold()\n', 'utf8')
  await writeFile(path.join(root, 'src', 'crlf.ts'), 'old\r\n', 'utf8')
  await writeFile(path.join(root, 'src', 'c.md'), 'old\n', 'utf8')

  const result = await runTool(root, {
    id: '1',
    name: 'replace',
    args: { pattern: 'src/**/*.ts', old: 'old', new: 'new' }
  })
  assert.equal(result.ok, true, result.text)
  assert.equal(await readFile(path.join(root, 'src', 'a.ts'), 'utf8'), 'const x = new\n')
  assert.equal(
    await readFile(path.join(root, 'src', 'b.ts'), 'utf8'),
    'old()\nold()\n',
    'a file with two hits is skipped by default'
  )
  assert.equal(await readFile(path.join(root, 'src', 'crlf.ts'), 'utf8'), 'new\r\n')
  assert.equal(await readFile(path.join(root, 'src', 'c.md'), 'utf8'), 'old\n')
  assert.match(result.text, /已替换 2 个文件，共 2 处/)
  assert.match(result.text, /src\/b\.ts：出现 2 次（不唯一）/, result.text)
})

test('replaceAll rewrites every hit in every matching file', async () => {
  const root = await workspace()
  await writeFile(path.join(root, 'a.ts'), 'old old\n', 'utf8')
  await writeFile(path.join(root, 'b.ts'), 'old\n', 'utf8')
  const result = await runTool(root, {
    id: '1',
    name: 'replace',
    args: { pattern: '*.ts', old: 'old', new: 'new', replaceAll: true }
  })
  assert.equal(result.ok, true, result.text)
  assert.equal(await readFile(path.join(root, 'a.ts'), 'utf8'), 'new new\n')
  assert.equal(await readFile(path.join(root, 'b.ts'), 'utf8'), 'new\n')
  assert.match(result.text, /已替换 2 个文件，共 3 处/)
})

test('a batch replace skips binary and oversized files', async () => {
  const root = await workspace()
  await writeFile(
    path.join(root, 'bin.dat'),
    Buffer.concat([Buffer.from([0x00]), Buffer.from('old')])
  )
  const big = path.join(root, 'big.dat')
  await writeFile(big, 'old', 'utf8')
  await truncate(big, MAX_EDIT_BYTES + 1)

  const result = await runTool(root, {
    id: '1',
    name: 'replace',
    args: { pattern: '*.dat', old: 'old', new: 'new' }
  })
  assert.equal(result.ok, true, result.text)
  assert.match(result.text, /bin\.dat：二进制/)
  assert.match(result.text, /big\.dat：超过 2 MB/)
  assert.deepEqual(
    await readFile(path.join(root, 'bin.dat')),
    Buffer.concat([Buffer.from([0x00]), Buffer.from('old')])
  )
  const kept = await readFile(big)
  assert.equal(kept.length, MAX_EDIT_BYTES + 1)
  assert.equal(kept.subarray(0, 3).toString(), 'old')
})

test('previews a batch replace without writing and without an undo pair', async () => {
  const root = await workspace()
  await writeFile(path.join(root, 'a.ts'), 'old\n', 'utf8')
  await writeFile(path.join(root, 'b.ts'), 'old\n', 'utf8')
  const preview = await previewTool(root, {
    id: '1',
    name: 'replace',
    args: { pattern: '*.ts', old: 'old', new: 'new' }
  })
  assert.ok(preview)
  assert.match(preview.diff, /将修改 2 个文件，共 2 处/)
  assert.match(preview.diff, /- a\.ts：1 处/)
  assert.equal(preview.undoable, false)
  assert.equal(await readFile(path.join(root, 'a.ts'), 'utf8'), 'old\n')
  assert.equal(await readFile(path.join(root, 'b.ts'), 'utf8'), 'old\n')
})

test('replace dryRun shows the plan and writes nothing', async () => {
  const root = await workspace()
  await writeFile(path.join(root, 'a.txt'), 'old\n', 'utf8')
  const preview = await runTool(root, {
    id: '1',
    name: 'replace',
    args: { pattern: '*.txt', old: 'old', new: 'new', dryRun: true }
  })
  assert.equal(preview.ok, true, preview.text)
  assert.match(preview.text, /预览，未写盘/)
  assert.match(preview.text, /a\.txt：1 处/)
  // The file still holds what it did before the preview.
  assert.equal(await readFile(path.join(root, 'a.txt'), 'utf8'), 'old\n')

  const real = await runTool(root, {
    id: '2',
    name: 'replace',
    args: { pattern: '*.txt', old: 'old', new: 'new' }
  })
  assert.equal(real.ok, true, real.text)
  assert.match(real.text, /已替换 1 个文件/)
  assert.equal(await readFile(path.join(root, 'a.txt'), 'utf8'), 'new\n')
})

test('write warns when it turns a UTF-16 file into UTF-8', async () => {
  const root = await workspace()
  const file = path.join(root, 'wide.txt')
  await writeFile(
    file,
    Buffer.concat([Buffer.from([0xff, 0xfe]), Buffer.from('a = 1\n', 'utf16le')])
  )
  const result = await runTool(root, {
    id: '1',
    name: 'write',
    args: { path: 'wide.txt', content: 'a = 2\n' }
  })
  assert.equal(result.ok, true, result.text)
  assert.match(result.text, /不是 UTF-8/)
  assert.equal(await readFile(file, 'utf8'), 'a = 2\n')
})

test('write hints that a small change to a long file could use edit', async () => {
  const root = await workspace()
  const lines = Array.from({ length: 60 }, (_, index) => `line ${index}`)
  await writeFile(path.join(root, 'long.txt'), `${lines.join('\n')}\n`, 'utf8')
  const edited = [...lines]
  edited[30] = 'line 30 changed'
  const small = await runTool(root, {
    id: '1',
    name: 'write',
    args: { path: 'long.txt', content: `${edited.join('\n')}\n` }
  })
  assert.equal(small.ok, true, small.text)
  assert.match(small.text, /用 edit 只发差异/)

  // A rewrite that changes every line is not a small edit: no hint.
  const fresh = Array.from({ length: 60 }, (_, index) => `other ${index}`).join('\n') + '\n'
  const rewritten = await runTool(root, {
    id: '2',
    name: 'write',
    args: { path: 'long.txt', content: fresh }
  })
  assert.equal(rewritten.ok, true, rewritten.text)
  assert.doesNotMatch(rewritten.text, /用 edit 只发差异/)
})

test('a screenshot crop needs all four positive numbers', () => {
  assert.deepEqual(shotCrop({ x: 10, y: 20, width: 300, height: 200 }), {
    x: 10,
    y: 20,
    width: 300,
    height: 200
  })
  // Zero is a valid offset but not a valid size.
  assert.deepEqual(shotCrop({ x: 0, y: 0, width: 1, height: 1 }), {
    x: 0,
    y: 0,
    width: 1,
    height: 1
  })
  assert.equal(shotCrop({ x: 10, y: 20, width: 300 }), null, 'a partial rectangle is ignored')
  assert.equal(shotCrop({ x: -1, y: 0, width: 10, height: 10 }), null)
  assert.equal(shotCrop({ x: 0, y: 0, width: 0, height: 10 }), null)
  assert.equal(shotCrop({}), null)
  // A numeric string is accepted, like every other integer argument; text throws
  // the same way it does everywhere else.
  assert.deepEqual(shotCrop({ x: '10', y: 0, width: 10, height: 10 }), {
    x: 10,
    y: 0,
    width: 10,
    height: 10
  })
  assert.throws(() => shotCrop({ x: 'left', y: 0, width: 10, height: 10 }), /不是数字/)
})

test('replaceAsciiBytes counts whole characters only', () => {
  // B0 41 looks like a two-byte character whose trail byte is the letter A; a
  // plain Buffer.indexOf('A') would find two places, this finds one.
  const bytes = Buffer.concat([Buffer.from([0xb0, 0x41]), Buffer.from(' A A', 'latin1')])
  const one = replaceAsciiBytes(bytes, 'A', 'B', false)
  assert.equal(one.count, 1)
  assert.deepEqual([...one.bytes.subarray(0, 2)], [0xb0, 0x41], 'the trail byte is untouched')
  assert.equal(one.bytes.subarray(2).toString('latin1'), ' B A')
  const all = replaceAsciiBytes(bytes, 'A', 'B', true)
  assert.equal(all.count, 2)
  assert.deepEqual([...all.bytes.subarray(0, 2)], [0xb0, 0x41])
  assert.equal(all.bytes.subarray(2).toString('latin1'), ' B B')
})

test('edit changes a GBK file only where the bytes are ASCII', async () => {
  const root = await workspace()
  const file = path.join(root, 'gbk.txt')
  // 你好 in GBK is C4 E3 BA C3.
  const chinese = Buffer.from([0xc4, 0xe3, 0xba, 0xc3])
  await writeFile(file, Buffer.concat([chinese, Buffer.from(' = 1\n', 'latin1')]))
  const result = await runTool(root, {
    id: '1',
    name: 'edit',
    args: { path: 'gbk.txt', old: '= 1', new: '= 2' }
  })
  assert.equal(result.ok, true, result.text)
  assert.match(result.text, /非 UTF-8/)
  const after = await readFile(file)
  assert.deepEqual([...after.subarray(0, 4)], [0xc4, 0xe3, 0xba, 0xc3], 'the Chinese bytes survive')
  assert.equal(after.subarray(4).toString('latin1'), ' = 2\n')
})

test('a GBK file refuses text that would need a GBK encoder', async () => {
  const root = await workspace()
  const file = path.join(root, 'gbk2.txt')
  await writeFile(file, Buffer.from([0xc4, 0xe3, 0xba, 0xc3]), 'latin1')
  const result = await runTool(root, {
    id: '1',
    name: 'edit',
    args: { path: 'gbk2.txt', old: '你好', new: '大家好' }
  })
  assert.equal(result.ok, false)
  assert.match(result.text, /ASCII/)
  // The file is left exactly as it was.
  assert.deepEqual([...(await readFile(file))], [0xc4, 0xe3, 0xba, 0xc3])
})

test('a GBK file accepts an ASCII line replacement', async () => {
  const root = await workspace()
  const file = path.join(root, 'gbk3.txt')
  const chinese = Buffer.from([0xc4, 0xe3, 0xba, 0xc3])
  await writeFile(file, Buffer.concat([chinese, Buffer.from('\nprint(1)\n', 'latin1')]))
  const result = await runTool(root, {
    id: '1',
    name: 'edit',
    args: { path: 'gbk3.txt', startLine: 2, endLine: 2, content: 'print(2)' }
  })
  assert.equal(result.ok, true, result.text)
  const after = await readFile(file)
  assert.deepEqual([...after.subarray(0, 4)], [0xc4, 0xe3, 0xba, 0xc3])
  assert.equal(after.subarray(4).toString('latin1'), '\nprint(2)\n')
})

test('edit refuses a binary file and a UTF-16 file', async () => {
  const root = await workspace()
  const binary = path.join(root, 'fake.png')
  const png = Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x00, 0x00]),
    Buffer.from('IDAT-SOMETHING', 'latin1'),
    Buffer.from([0x00, 0xff])
  ])
  await writeFile(binary, png)
  const refused = await runTool(root, {
    id: '1',
    name: 'edit',
    args: { path: 'fake.png', old: 'IDAT', new: 'XXXX' }
  })
  assert.equal(refused.ok, false)
  assert.match(refused.text, /二进制/)
  assert.deepEqual([...(await readFile(binary))], [...png], 'a binary file is left untouched')

  // UTF-16 is text, but its bytes are not ASCII: say so instead of corrupting it.
  const wide = path.join(root, 'wide.txt')
  const utf16 = Buffer.concat([Buffer.from([0xff, 0xfe]), Buffer.from('a = 1\n', 'utf16le')])
  await writeFile(wide, utf16)
  const wideResult = await runTool(root, {
    id: '2',
    name: 'edit',
    args: { path: 'wide.txt', old: 'a = 1', new: 'a = 2' }
  })
  assert.equal(wideResult.ok, false)
  assert.match(wideResult.text, /UTF-16/)
  assert.deepEqual([...(await readFile(wide))], [...utf16])
})

test('a GBK edit follows the file line endings', async () => {
  const root = await workspace()
  const file = path.join(root, 'gbk-crlf.txt')
  const chinese = Buffer.from([0xc4, 0xe3, 0xba, 0xc3])
  // CRLF after the Chinese line, and the model asks with a bare \n.
  await writeFile(file, Buffer.concat([chinese, Buffer.from('\r\na = 1\r\nb = 2\r\n', 'latin1')]))
  const result = await runTool(root, {
    id: '1',
    name: 'edit',
    args: { path: 'gbk-crlf.txt', old: 'a = 1\nb = 2', new: 'a = 9\nb = 8' }
  })
  assert.equal(result.ok, true, result.text)
  const after = await readFile(file)
  assert.deepEqual([...after.subarray(0, 4)], [0xc4, 0xe3, 0xba, 0xc3])
  assert.equal(after.subarray(4).toString('latin1'), '\r\na = 9\r\nb = 8\r\n')
})

test('the small-edit hint works on a CRLF file too', async () => {
  const root = await workspace()
  const file = path.join(root, 'crlf.txt')
  const lines = Array.from({ length: 60 }, (_, index) => `line ${index}`)
  await writeFile(file, `${lines.join('\r\n')}\r\n`, 'utf8')
  const edited = [...lines]
  edited[10] = 'line 10 changed'
  const result = await runTool(root, {
    id: '1',
    name: 'write',
    args: { path: 'crlf.txt', content: `${edited.join('\n')}\n` }
  })
  assert.equal(result.ok, true, result.text)
  assert.match(result.text, /用 edit 只发差异/)
})

test('task output decodes a log written in the OEM code page', async () => {
  const root = await workspace()
  const dir = path.join(root, '.dsh-flow', 'tasks')
  await mkdir(dir, { recursive: true })
  const log = path.join(dir, 'task-1.log')
  // 构建完成 in GBK, followed by a newline.
  await writeFile(
    log,
    Buffer.from([0xb9, 0xb9, 0xbd, 0xa8, 0xcd, 0xea, 0xb3, 0xc9, 0x0a])
  )
  await writeFile(
    path.join(dir, 'task-1.json'),
    JSON.stringify({
      id: 'task-1',
      command: 'gradlew build',
      cwd: root,
      pid: 0,
      startedAt: new Date(0).toISOString(),
      log
    }),
    'utf8'
  )
  const result = await runTool(root, {
    id: '1',
    name: 'task',
    args: { action: 'output', id: 'task-1' }
  })
  assert.equal(result.ok, true, result.text)
  assert.match(result.text, /构建完成/)
})

test('a background log is written as UTF-8 whatever the command printed', async () => {
  const root = await workspace()
  const log = path.join(root, 'task.log')
  const script = path.join(root, 'print-gbk.js')
  // 构建完成 in GBK, written in two pieces so the wrapper has to join a split
  // character before decoding it.
  await writeFile(
    script,
    [
      'const bytes = Buffer.from([0xb9, 0xb9, 0xbd, 0xa8, 0xcd, 0xea, 0xb3, 0xc9, 0x0a])',
      'process.stdout.write(bytes.subarray(0, 3))',
      'setTimeout(() => process.stdout.write(bytes.subarray(3)), 30)'
    ].join('\n'),
    'utf8'
  )
  const wrapper = path.join(
    path.dirname(fileURLToPath(import.meta.url)),
    '..',
    'src',
    'tools',
    'task-run.ts'
  )
  const command = `"${process.execPath}" "${script}"`
  await new Promise<void>((resolve, reject) => {
    const child = spawn(process.execPath, [wrapper, log, root, command], { stdio: 'ignore' })
    child.on('error', reject)
    child.on('close', () => resolve())
  })
  const text = await readFile(log, 'utf8')
  assert.match(text, /构建完成/, 'the GBK line was decoded before it reached the log')
  assert.match(text, /\[退出码 0\]/)
})

test('a batch replace with nothing to do says so', async () => {
  const root = await workspace()
  const result = await runTool(root, {
    id: '1',
    name: 'replace',
    args: { pattern: 'missing/**/*.ts', old: 'old', new: 'new' }
  })
  assert.equal(result.ok, true, result.text)
  assert.match(result.text, /没有匹配/)
  assert.equal(
    await previewTool(root, {
      id: '2',
      name: 'replace',
      args: { pattern: 'missing/**/*.ts', old: 'old', new: 'new' }
    }),
    null,
    'nothing to approve when no file would change'
  )
})

test('moves a file without overwriting the target', async () => {
  const root = await workspace()
  await writeFile(path.join(root, 'from.txt'), 'body\n', 'utf8')
  const moved = await runTool(root, {
    id: '1',
    name: 'move',
    args: { from: 'from.txt', to: 'nested/to.txt' }
  })
  assert.equal(moved.ok, true, moved.text)
  assert.match(moved.text, /已移动 from\.txt → nested\/to\.txt/)
  assert.equal(await readFile(path.join(root, 'nested', 'to.txt'), 'utf8'), 'body\n')
  await assert.rejects(readFile(path.join(root, 'from.txt')))

  await writeFile(path.join(root, 'other.txt'), 'other\n', 'utf8')
  const clash = await runTool(root, {
    id: '2',
    name: 'move',
    args: { from: 'other.txt', to: 'nested/to.txt' }
  })
  assert.equal(clash.ok, false)
  assert.match(clash.text, /目标已存在/)
  assert.equal(await readFile(path.join(root, 'other.txt'), 'utf8'), 'other\n')
})

test('a folder is deleted only when the call asks for it', async () => {
  const root = await workspace()
  await mkdir(path.join(root, 'build', 'nested'), { recursive: true })
  await writeFile(path.join(root, 'build', 'out.txt'), 'x\n', 'utf8')
  await writeFile(path.join(root, 'build', 'nested', 'more.txt'), 'y\n', 'utf8')

  const refused = await runTool(root, { id: '1', name: 'delete', args: { path: 'build' } })
  assert.equal(refused.ok, false)
  assert.match(refused.text, /recursive: true/)

  const deleted = await runTool(root, {
    id: '2',
    name: 'delete',
    args: { path: 'build', recursive: true }
  })
  assert.equal(deleted.ok, true, deleted.text)
  assert.match(deleted.text, /已删除目录 build（2 个文件）/)
  await assert.rejects(readFile(path.join(root, 'build', 'out.txt')))
})

test('deletes one file but refuses a folder', async () => {
  const root = await workspace()
  await writeFile(path.join(root, 'gone.txt'), 'bye\n', 'utf8')
  const deleted = await runTool(root, { id: '1', name: 'delete', args: { path: 'gone.txt' } })
  assert.equal(deleted.ok, true, deleted.text)
  assert.match(deleted.text, /已删除 gone\.txt/)
  await assert.rejects(readFile(path.join(root, 'gone.txt')))

  await mkdir(path.join(root, 'keep'))
  const folder = await runTool(root, { id: '2', name: 'delete', args: { path: 'keep' } })
  assert.equal(folder.ok, false)
  assert.match(folder.text, /这是一个文件夹/)

  const outside = await runTool(root, { id: '3', name: 'delete', args: { path: '..' } })
  assert.equal(outside.ok, false)
})

test('refuses an edit that is not unique', async () => {
  const root = await workspace()
  await writeFile(path.join(root, 'd.txt'), 'x x', 'utf8')
  const result = await runTool(root, {
    id: '1',
    name: 'edit',
    args: { path: 'd.txt', old: 'x', new: 'y' }
  })
  assert.equal(result.ok, false)
  assert.match(result.text, /不止一次/)
})

test('points a read at a folder to list', async () => {
  const root = await workspace()
  await mkdir(path.join(root, 'src'))
  const result = await runTool(root, { id: '1', name: 'read', args: { path: 'src' } })
  assert.equal(result.ok, false)
  assert.match(result.text, /list/)
})

test('drops a byte-order mark when reading', async () => {
  const root = await workspace()
  await writeFile(path.join(root, 'bom.txt'), '\uFEFFfirst line\n', 'utf8')
  const result = await runTool(root, { id: '1', name: 'read', args: { path: 'bom.txt' } })
  assert.equal(result.ok, true)
  assert.equal(result.text.split('\n')[0], '1\tfirst line')
})

test('keeps every path inside the working directory', async () => {
  const root = await workspace()
  for (const target of ['../escape.txt', '..\\escape.txt', 'C:\\Windows\\win.ini', '/etc/passwd']) {
    const result = await runTool(root, { id: '1', name: 'read', args: { path: target } })
    assert.equal(result.ok, false, target)
  }
  const written = await runTool(root, {
    id: '1',
    name: 'write',
    args: { path: '../escape.txt', content: 'nope' }
  })
  assert.equal(written.ok, false)
})

test('previews a write without touching the file', async () => {
  const root = await workspace()
  await writeFile(path.join(root, 'a.txt'), 'one\ntwo\n', 'utf8')
  const preview = await previewTool(root, {
    id: '1',
    name: 'write',
    args: { path: 'a.txt', content: 'one\nthree\n' }
  })
  assert.ok(preview)
  assert.match(preview.diff, /^- two$/m)
  assert.match(preview.diff, /^\+ three$/m)
  assert.equal(preview.before, 'one\ntwo\n')
  assert.equal(preview.after, 'one\nthree\n')
  assert.equal(preview.existed, true)
  assert.equal(preview.undoable, true)
  assert.equal(await readFile(path.join(root, 'a.txt'), 'utf8'), 'one\ntwo\n')
})

test('previews an edit and skips an unchanged write', async () => {
  const root = await workspace()
  await writeFile(path.join(root, 'b.txt'), 'name: old\n', 'utf8')
  const preview = await previewTool(root, {
    id: '1',
    name: 'edit',
    args: { path: 'b.txt', old: 'old', new: 'new' }
  })
  assert.ok(preview)
  assert.match(preview.diff, /^\+ name: new$/m)
  assert.equal(preview.before, 'name: old\n')
  assert.equal(preview.after, 'name: new\n')

  const unchanged = await previewTool(root, {
    id: '2',
    name: 'write',
    args: { path: 'b.txt', content: 'name: old\n' }
  })
  assert.equal(unchanged, null)
})

test('reads a reference folder but never writes there', async () => {
  const root = await workspace()
  const reference = await workspace()
  await writeFile(path.join(reference, 'flow.yml'), 'name: demo\n', 'utf8')

  const read = await runTool(
    root,
    { id: '1', name: 'read', args: { path: path.join(reference, 'flow.yml') } },
    [reference]
  )
  assert.equal(read.ok, true)
  assert.match(read.text, /name: demo/)

  const write = await runTool(
    root,
    { id: '2', name: 'write', args: { path: path.join(reference, 'x.txt'), content: 'no' } },
    [reference]
  )
  assert.equal(write.ok, false)
})

test('globs and greps reference folders', async () => {
  const root = await workspace()
  const reference = await workspace()
  await writeFile(path.join(reference, 'flow.yml'), 'name: demo\n', 'utf8')

  const globbed = await runTool(root, { id: '1', name: 'glob', args: { pattern: '*.yml' } }, [
    reference
  ])
  assert.equal(globbed.ok, true)
  assert.match(globbed.text.replace(/\\/g, '/'), /flow\.yml$/)

  const grepped = await runTool(root, { id: '2', name: 'grep', args: { pattern: 'demo' } }, [
    reference
  ])
  assert.equal(grepped.ok, true)
  assert.match(grepped.text, /flow\.yml:1: name: demo$/)
})

test('globs files relative to the working directory', async () => {
  const root = await workspace()
  await writeFile(path.join(root, 'one.ts'), '', 'utf8')
  await writeFile(path.join(root, 'two.json'), '', 'utf8')
  const result = await runTool(root, { id: '1', name: 'glob', args: { pattern: '*.ts' } })
  assert.equal(result.ok, true)
  assert.equal(result.text, 'one.ts')
})

test('lists direct children with folders first', async () => {
  const root = await workspace()
  await mkdir(path.join(root, 'src'))
  await mkdir(path.join(root, 'node_modules'))
  await writeFile(path.join(root, 'zeta.txt'), '', 'utf8')
  await writeFile(path.join(root, 'alpha.txt'), '', 'utf8')
  const result = await runTool(root, { id: '1', name: 'list', args: {} })
  assert.equal(result.ok, true)
  assert.equal(result.text, 'src/\nalpha.txt\nzeta.txt')
})

test('lists a nested folder and refuses to leave the working directory', async () => {
  const root = await workspace()
  await mkdir(path.join(root, 'src'))
  await writeFile(path.join(root, 'src', 'a.ts'), '', 'utf8')

  const nested = await runTool(root, { id: '1', name: 'list', args: { path: 'src' } })
  assert.equal(nested.ok, true)
  assert.equal(nested.text, 'a.ts')

  const empty = await runTool(root, { id: '2', name: 'list', args: { path: 'missing' } })
  assert.equal(empty.ok, false)
  assert.match(empty.text, /读不了这个文件夹/)

  const outside = await runTool(root, { id: '3', name: 'list', args: { path: '..' } })
  assert.equal(outside.ok, false)
})

test('lists a reference folder', async () => {
  const root = await workspace()
  const reference = await workspace()
  await writeFile(path.join(reference, 'flow.yml'), 'name: demo\n', 'utf8')
  const result = await runTool(
    root,
    { id: '1', name: 'list', args: { path: reference } },
    [reference]
  )
  assert.equal(result.ok, true)
  assert.match(result.text, /flow\.yml/)
})

test('trees a folder to the requested depth and skips ignored folders', async () => {
  const root = await workspace()
  await mkdir(path.join(root, 'src', 'app'), { recursive: true })
  await mkdir(path.join(root, 'node_modules', 'pkg'), { recursive: true })
  await writeFile(path.join(root, 'src', 'main.ts'), 'x', 'utf8')
  await writeFile(path.join(root, 'src', 'app', 'view.ts'), 'x', 'utf8')
  await writeFile(path.join(root, 'README.md'), 'x', 'utf8')

  const shallow = await runTool(root, {
    id: '1',
    name: 'tree',
    args: { path: '.', depth: 1 }
  })
  assert.equal(shallow.ok, true, shallow.text)
  assert.match(shallow.text, /src\//)
  assert.match(shallow.text, /README\.md/)
  assert.doesNotMatch(shallow.text, /app\//, 'depth 1 lists direct children only')

  const deep = await runTool(root, {
    id: '2',
    name: 'tree',
    args: { path: '.', depth: 3 }
  })
  assert.equal(deep.ok, true, deep.text)
  assert.match(deep.text, /  app\//)
  assert.match(deep.text, /    view\.ts/)
  assert.doesNotMatch(deep.text, /node_modules/)
})

test('trees the workspace root by default in workspace scope', async () => {
  const scope = await workspace()
  const chat = path.join(scope, 'workspace', 'chat-1')
  await mkdir(chat, { recursive: true })
  await writeFile(path.join(scope, 'root.txt'), 'x', 'utf8')
  await writeFile(path.join(chat, 'own.txt'), 'x', 'utf8')

  const result = await runToolScoped(chat, scope, {
    id: '1',
    name: 'tree',
    args: { depth: 1 }
  })
  assert.equal(result.ok, true, result.text)
  assert.match(result.text, /root\.txt/)
  assert.doesNotMatch(result.text, /own\.txt/)
})

test('runs only read-only git subcommands', async () => {
  const root = await workspace()
  execFileSync('git', ['init', '-q'], { cwd: root })
  await writeFile(path.join(root, 'a.txt'), 'x', 'utf8')

  const status = await runTool(root, {
    id: '1',
    name: 'git',
    args: { command: 'status', args: ['--short'] }
  })
  assert.equal(status.ok, true, status.text)
  assert.match(status.text, /\?\? a\.txt/)

  const refused = await runTool(root, { id: '2', name: 'git', args: { command: 'branch' } })
  assert.equal(refused.ok, false)
  assert.match(refused.text, /只允许只读子命令/)

  const output = await runTool(root, {
    id: '3',
    name: 'git',
    args: { command: 'diff', args: ['--output=leak.txt'] }
  })
  assert.equal(output.ok, false)
  assert.match(output.text, /不接受这个参数/)

  const outside = await runTool(root, {
    id: '4',
    name: 'git',
    args: { command: 'diff', args: ['--no-index', '..'] }
  })
  assert.equal(outside.ok, false)
  assert.match(outside.text, /不接受这个参数/)

  const diff = await runTool(root, {
    id: '5',
    name: 'git',
    args: { command: 'diff', args: ['--stat'] }
  })
  assert.equal(diff.ok, true, diff.text)
})

test('honours .gitignore in glob and grep', async () => {
  const root = await workspace()
  await writeFile(
    path.join(root, '.gitignore'),
    '# ignored by default\nsecret.txt\nbuild/\n*.log\n!keep.log\nsrc/*.tmp\n',
    'utf8'
  )
  await writeFile(path.join(root, 'keep.txt'), 'visible\n', 'utf8')
  await writeFile(path.join(root, 'secret.txt'), 'HIDDEN\n', 'utf8')
  await writeFile(path.join(root, 'keep.log'), 'keep\n', 'utf8')
  await writeFile(path.join(root, 'other.log'), 'HIDDEN\n', 'utf8')
  await mkdir(path.join(root, 'build'))
  await writeFile(path.join(root, 'build', 'out.txt'), 'HIDDEN\n', 'utf8')
  await mkdir(path.join(root, 'src'))
  await mkdir(path.join(root, 'src', 'deep'))
  await writeFile(path.join(root, 'src', 'a.tmp'), 'HIDDEN\n', 'utf8')
  await writeFile(path.join(root, 'src', 'deep', 'b.tmp'), 'visible\n', 'utf8')

  const globbed = await runTool(root, { id: '1', name: 'glob', args: { pattern: '**/*' } })
  assert.equal(globbed.ok, true, globbed.text)
  const lines = globbed.text.split('\n')
  assert.ok(lines.includes('keep.txt'), globbed.text)
  assert.ok(lines.includes('keep.log'), 'a negated rule re-includes it')
  assert.ok(lines.includes('src/deep/b.tmp'), 'an anchored rule stays at its depth')
  assert.ok(!lines.includes('secret.txt'), globbed.text)
  assert.ok(!lines.includes('other.log'), globbed.text)
  assert.ok(!lines.some((line) => line.startsWith('build/')), globbed.text)
  assert.ok(!lines.includes('src/a.tmp'), globbed.text)

  const grepped = await runTool(root, { id: '2', name: 'grep', args: { pattern: 'HIDDEN' } })
  assert.equal(grepped.ok, true, grepped.text)
  assert.match(grepped.text, /没有匹配/)
})

test('a nested .gitignore applies to its own folder', async () => {
  const root = await workspace()
  await writeFile(path.join(root, '.gitignore'), 'root-only.txt\n', 'utf8')
  await mkdir(path.join(root, 'packages', 'app'), { recursive: true })
  await writeFile(path.join(root, 'packages', 'app', '.gitignore'), 'generated/\n', 'utf8')
  await writeFile(path.join(root, 'packages', 'app', 'index.ts'), 'export const a = 1\n', 'utf8')
  await mkdir(path.join(root, 'packages', 'app', 'generated'))
  await writeFile(path.join(root, 'packages', 'app', 'generated', 'out.ts'), 'GENERATED\n', 'utf8')

  const globbed = await runTool(root, { id: '1', name: 'glob', args: { pattern: '**/*.ts' } })
  assert.equal(globbed.ok, true, globbed.text)
  assert.match(globbed.text, /packages\/app\/index\.ts/)
  assert.doesNotMatch(globbed.text, /generated\/out\.ts/)

  // The same rule has to hold for a content search, not only for the listing.
  const grepped = await runTool(root, { id: '2', name: 'grep', args: { pattern: 'GENERATED' } })
  assert.match(grepped.text, /没有匹配/)
})

test('glob and grep can be narrowed to one folder', async () => {
  const root = await workspace()
  await mkdir(path.join(root, 'src'))
  await mkdir(path.join(root, 'tests'))
  await writeFile(path.join(root, 'src', 'a.ts'), 'export const inSrc = 1\n', 'utf8')
  await writeFile(path.join(root, 'tests', 'a.ts'), 'export const inTests = 1\n', 'utf8')

  const globbed = await runTool(root, {
    id: '1',
    name: 'glob',
    args: { pattern: '**/*.ts', path: 'src' }
  })
  assert.equal(globbed.ok, true, globbed.text)
  assert.match(globbed.text, /src\/a\.ts/)
  assert.doesNotMatch(globbed.text, /tests\/a\.ts/)

  const grepped = await runTool(root, {
    id: '2',
    name: 'grep',
    args: { pattern: 'inTests', path: 'src' }
  })
  assert.match(grepped.text, /没有匹配/)

  const missing = await runTool(root, { id: '3', name: 'grep', args: { pattern: 'x', path: 'nope' } })
  assert.equal(missing.ok, false, missing.text)
  assert.match(missing.text, /找不到文件夹/)
})

test('searches leave out the sibling folders the window names', async () => {
  const root = await workspace()
  // `root` plays the workspace; `root/chat-1` is this conversation and
  // `root/chat-2` is another one, whose files must stay out of the results.
  await mkdir(path.join(root, 'chat-1'))
  await mkdir(path.join(root, 'chat-2'))
  await writeFile(path.join(root, 'chat-1', 'mine.ts'), 'export const mine = 1\n', 'utf8')
  await writeFile(path.join(root, 'chat-2', 'theirs.ts'), 'export const theirs = 1\n', 'utf8')
  const skip = ['chat-2']

  const globbed = await runToolScoped(
    path.join(root, 'chat-1'),
    root,
    { id: '1', name: 'glob', args: { pattern: '**/*.ts' } },
    [],
    undefined,
    null,
    skip
  )
  assert.equal(globbed.ok, true, globbed.text)
  assert.match(globbed.text, /chat-1\/mine\.ts/)
  assert.doesNotMatch(globbed.text, /chat-2/)

  const grepped = await runToolScoped(
    path.join(root, 'chat-1'),
    root,
    { id: '2', name: 'grep', args: { pattern: 'export' } },
    [],
    undefined,
    null,
    skip
  )
  assert.match(grepped.text, /mine/)
  assert.doesNotMatch(grepped.text, /theirs/)

  const tree = await runToolScoped(
    path.join(root, 'chat-1'),
    root,
    { id: '3', name: 'tree', args: { depth: 2 } },
    [],
    undefined,
    null,
    skip
  )
  assert.match(tree.text, /chat-1\//)
  assert.doesNotMatch(tree.text, /chat-2/)
})

test('a written file invalidates the symbol index', async () => {
  const root = await workspace()
  await writeFile(path.join(root, 'a.ts'), 'export function oldName() {}\n', 'utf8')
  const first = await runTool(root, { id: '1', name: 'symbols', args: { query: 'oldName' } })
  assert.equal(first.ok, true, first.text)
  assert.match(first.text, /oldName/)

  // The index is trusted for ten minutes; a write has to drop it, or the next
  // question would be handed line numbers from before the change.
  await runTool(root, {
    id: '2',
    name: 'write',
    args: { path: 'a.ts', content: 'export function newName() {}\n' }
  })
  const second = await runTool(root, { id: '3', name: 'symbols', args: { query: 'newName' } })
  assert.equal(second.ok, true, second.text)
  assert.match(second.text, /newName/)
  const stale = await runTool(root, { id: '4', name: 'symbols', args: { query: 'oldName' } })
  assert.match(stale.text, /没有匹配/)
})

test('greps a GBK file instead of silently missing it', async () => {
  const root = await workspace()
  // 中文 in the OEM code page, the way a Chinese Windows editor may save it.
  await writeFile(path.join(root, 'legacy.txt'), Buffer.from([0xd6, 0xd0, 0xce, 0xc4, 0x0a]))
  const hit = await runTool(root, { id: '1', name: 'grep', args: { pattern: '中文' } })
  assert.equal(hit.ok, true, hit.text)
  assert.match(hit.text, /legacy\.txt:1/)
})

test('grep caps how much one file contributes', async () => {
  const root = await workspace()
  const many = Array.from({ length: 30 }, (_, index) => `hit ${index}`).join('\n')
  await writeFile(path.join(root, 'many.txt'), `${many}\n`, 'utf8')
  await writeFile(path.join(root, 'one.txt'), 'hit once\n', 'utf8')

  const result = await runTool(root, {
    id: '1',
    name: 'grep',
    args: { pattern: 'hit', perFile: 5 }
  })
  assert.equal(result.ok, true, result.text)
  const fromMany = result.text.split('\n').filter((line) => line.startsWith('many.txt:'))
  assert.equal(fromMany.length, 5, result.text)
  // The other file still shows up: one noisy file cannot take the whole answer.
  assert.match(result.text, /one\.txt:1:/)
  assert.match(result.text, /单个文件最多列 5 行/)
})

test('greps file contents with line numbers', async () => {
  const root = await workspace()
  await writeFile(path.join(root, 'e.txt'), 'alpha\nbeta\n', 'utf8')
  const result = await runTool(root, { id: '1', name: 'grep', args: { pattern: 'bet' } })
  assert.equal(result.ok, true)
  assert.match(result.text, /^e\.txt:2: beta$/)
})

test('greps case-insensitively and can show context lines', async () => {
  const root = await workspace()
  await writeFile(path.join(root, 'f.txt'), 'alpha\nbeta\nALPHA\ngamma\n', 'utf8')
  const result = await runTool(root, {
    id: '1',
    name: 'grep',
    args: { pattern: 'alpha', ignoreCase: true, context: 1 }
  })
  assert.equal(result.ok, true, result.text)
  assert.match(result.text, /f\.txt:1: alpha/)
  assert.match(result.text, /f\.txt:2- beta/, 'the line after a hit is shown as context')
  assert.match(result.text, /f\.txt:3: ALPHA/, 'ignoreCase finds the upper-case hit')
  assert.match(result.text, /f\.txt:4- gamma/)
})

test('stops grepping after its read budget instead of scanning everything', async () => {
  const root = await workspace()
  const almostHalf = Math.ceil(MAX_GREP_BYTES / 2) + 1024
  await writeFile(path.join(root, 'a.txt'), `${'x'.repeat(almostHalf)}ZEBRA\n`, 'utf8')
  await writeFile(path.join(root, 'b.txt'), `${'y'.repeat(almostHalf)}ZEBRA\n`, 'utf8')
  const result = await runTool(root, { id: '1', name: 'grep', args: { pattern: 'ZEBRA' } })
  assert.equal(result.ok, true, result.text)
  assert.match(result.text, /a\.txt:1: /)
  assert.doesNotMatch(result.text, /b\.txt/)
  assert.match(result.text, /已读满/)
  assert.ok(result.text.length < 21_000, `${result.text.length} characters`)
})

test('says how many glob matches it could not show', async () => {
  const root = await workspace()
  for (let index = 0; index < MAX_RESULTS + 2; index += 1) {
    await writeFile(path.join(root, `f${index}.txt`), '', 'utf8')
  }
  const result = await runTool(root, { id: '1', name: 'glob', args: { pattern: '*.txt' } })
  assert.equal(result.ok, true)
  const lines = result.text.split('\n')
  assert.equal(lines.length, MAX_RESULTS + 1)
  assert.match(lines[MAX_RESULTS], /还有 2 个未显示/)
})

test('writes anywhere inside the workspace scope', async () => {
  const scope = await workspace()
  const chat = path.join(scope, 'workspace', 'chat-1')
  await runToolScoped(chat, scope, {
    id: '1',
    name: 'write',
    args: { path: 'note.txt', content: 'inside\n' }
  })
  assert.equal(await readFile(path.join(chat, 'note.txt'), 'utf8'), 'inside\n')

  const sibling = await runToolScoped(chat, scope, {
    id: '2',
    name: 'write',
    args: { path: '../../shared.txt', content: 'sibling\n' }
  })
  assert.equal(sibling.ok, true, sibling.text)
  assert.equal(await readFile(path.join(scope, 'shared.txt'), 'utf8'), 'sibling\n')

  const outside = await runToolScoped(chat, scope, {
    id: '3',
    name: 'write',
    args: { path: path.join(tmpdir(), `dsh-outside-${Date.now()}.txt`), content: 'no' }
  })
  assert.equal(outside.ok, false)
})

test('writes any path in full scope', async () => {
  const work = await workspace()
  const target = path.join(await workspace(), 'deep', 'note.txt')
  const result = await runToolScoped(work, null, {
    id: '1',
    name: 'write',
    args: { path: target, content: 'everywhere\n' }
  })
  assert.equal(result.ok, true, result.text)
  assert.equal(await readFile(target, 'utf8'), 'everywhere\n')

  const preview = await previewToolScoped(work, null, {
    id: '2',
    name: 'write',
    args: { path: target, content: 'changed\n' }
  })
  assert.ok(preview)
  assert.equal(preview.before, 'everywhere\n')
  assert.equal(preview.after, 'changed\n')
  assert.equal(preview.undoable, true)
})

test('globs the workspace instead of the conversation folder', async () => {
  const scope = await workspace()
  const chat = path.join(scope, 'workspace', 'chat-1')
  await mkdir(chat, { recursive: true })
  await writeFile(path.join(chat, 'local.ts'), '', 'utf8')
  await writeFile(path.join(scope, 'shared.ts'), '', 'utf8')
  const result = await runToolScoped(chat, scope, { id: '1', name: 'glob', args: { pattern: '**/*.ts' } })
  assert.equal(result.ok, true)
  const lines = result.text.split('\n')
  assert.ok(
    lines.some((line) => line.endsWith('shared.ts')),
    result.text
  )
  assert.ok(
    lines.some((line) => line.endsWith('local.ts')),
    result.text
  )
})

test('searches a reference folder once even when it is listed twice', async () => {
  const scope = await workspace()
  const chat = path.join(scope, 'chat-1')
  await mkdir(chat, { recursive: true })
  const reference = await workspace()
  await writeFile(path.join(reference, 'needle.txt'), 'NEEDLE\n', 'utf8')
  const result = await runToolScoped(
    chat,
    scope,
    { id: '1', name: 'grep', args: { pattern: 'NEEDLE' } },
    [reference, reference, path.join(reference, 'nested')]
  )
  assert.equal(result.ok, true, result.text)
  const hits = result.text.split('\n').filter((line) => line.includes('needle.txt'))
  assert.equal(hits.length, 1, result.text)
})

test('commits, branches and stashes through git_write', async () => {
  const root = await workspace()
  execFileSync('git', ['init'], { cwd: root, stdio: 'ignore' })
  execFileSync('git', ['config', 'user.email', 'a@b.c'], { cwd: root })
  execFileSync('git', ['config', 'user.name', 'tester'], { cwd: root })
  await writeFile(path.join(root, 'a.txt'), 'one\n', 'utf8')

  const committed = await runTool(root, {
    id: '1',
    name: 'git_write',
    args: { action: 'commit', all: true, message: 'add a' }
  })
  assert.equal(committed.ok, true, committed.text)
  assert.match(committed.text, /\$ git commit -m add a/)
  const log = execFileSync('git', ['log', '--oneline'], { cwd: root, encoding: 'utf8' })
  assert.match(log, /add a/)

  const branch = await runTool(root, {
    id: '2',
    name: 'git_write',
    args: { action: 'branch', name: 'feat/x' }
  })
  assert.equal(branch.ok, true, branch.text)
  const branches = execFileSync('git', ['branch', '--list'], { cwd: root, encoding: 'utf8' })
  assert.match(branches, /feat\/x/)

  await writeFile(path.join(root, 'b.txt'), 'two\n', 'utf8')
  const stashed = await runTool(root, {
    id: '3',
    name: 'git_write',
    args: { action: 'stash', includeUntracked: true, message: 'wip' }
  })
  assert.equal(stashed.ok, true, stashed.text)
  const list = await runTool(root, {
    id: '4',
    name: 'git_write',
    args: { action: 'stash', stashAction: 'list' }
  })
  assert.equal(list.ok, true, list.text)
  assert.match(list.text, /wip/)
})

test('refuses a repository action it cannot express', async () => {
  const root = await workspace()
  execFileSync('git', ['init'], { cwd: root, stdio: 'ignore' })
  const unknown = await runTool(root, {
    id: '1',
    name: 'git_write',
    args: { action: 'reset' }
  })
  assert.equal(unknown.ok, false)
  assert.match(unknown.text, /commit、branch 或 stash/)

  const outside = await workspace()
  const noRepo = await runTool(outside, {
    id: '2',
    name: 'git_write',
    args: { action: 'stash', stashAction: 'list' }
  })
  assert.equal(noRepo.ok, false)
  assert.match(noRepo.text, /不是 git 仓库/)

  // A path that could reach outside the repository never gets to git.
  const escape = await runTool(root, {
    id: '3',
    name: 'git_write',
    args: { action: 'commit', message: 'x', paths: ['../outside.txt'] }
  })
  assert.equal(escape.ok, false)
  assert.match(escape.text, /\.\./)

  const badBranch = await runTool(root, {
    id: '4',
    name: 'git_write',
    args: { action: 'branch', name: '-D main' }
  })
  assert.equal(badBranch.ok, false)
  assert.match(badBranch.text, /- 开头/)
})

test('waits for a port, a url and a task line', async (t) => {
  const { server, base } = await localServer((_req, res) => {
    res.writeHead(200, { 'content-type': 'text/plain' })
    res.end('up')
  })
  t.after(() => server.close())
  const port = Number(new URL(base).port)

  const open = await runTool(await workspace(), {
    id: '1',
    name: 'wait_for',
    args: { target: 'port', port, intervalMs: 100 }
  })
  assert.equal(open.ok, true, open.text)
  assert.match(open.text, /已就绪/)

  const answered = await runTool(await workspace(), {
    id: '2',
    name: 'wait_for',
    args: { target: 'url', url: `${base}/health`, intervalMs: 100 }
  })
  assert.equal(answered.ok, true, answered.text)
  assert.match(answered.text, /→ 200/)

  // A task whose log already holds the line is ready at once, even though the
  // recorded process is gone: the check comes before the running state.
  const root = await workspace()
  const tasks = path.join(root, '.dsh-flow', 'tasks')
  await mkdir(tasks, { recursive: true })
  const log = path.join(tasks, 'task-1.log')
  await writeFile(log, 'starting\nready on 5173\n', 'utf8')
  await writeFile(
    path.join(tasks, 'task-1.json'),
    JSON.stringify({ id: 'task-1', command: 'npm run dev', cwd: root, pid: 0, startedAt: '', log }),
    'utf8'
  )
  const line = await runTool(root, {
    id: '3',
    name: 'wait_for',
    args: { target: 'output', id: 'task-1', pattern: 'ready' }
  })
  assert.equal(line.ok, true, line.text)
  assert.match(line.text, /ready on 5173/)
})

test('waiting for a task to finish belongs to task, not wait_for', async () => {
  // A record whose process is already gone answers at once.
  const root = await workspace()
  const tasks = path.join(root, '.dsh-flow', 'tasks')
  await mkdir(tasks, { recursive: true })
  const log = path.join(tasks, 'task-1.log')
  await writeFile(log, 'done\n[退出码 0]\n', 'utf8')
  await writeFile(
    path.join(tasks, 'task-1.json'),
    JSON.stringify({ id: 'task-1', command: 'x', cwd: root, pid: 0, startedAt: '', log }),
    'utf8'
  )
  const done = await runTool(root, {
    id: '1',
    name: 'task',
    args: { action: 'wait', id: 'task-1', timeoutMs: 700 }
  })
  assert.equal(done.ok, true, done.text)
  assert.match(done.text, /task-1 早已结束（退出码 0）/)

  // `wait_for` no longer offers `exit`: a second way to ask the same question
  // only gives the model something to get wrong.
  const wrong = await runTool(root, {
    id: '2',
    name: 'wait_for',
    args: { target: 'exit', id: 'task-1' }
  })
  assert.equal(wrong.ok, false)
  assert.match(wrong.text, /task 的 action=wait/)

  // And a real task that finishes while `task wait` is running.
  const live = await workspace()
  const started = await runTool(live, {
    id: '3',
    name: 'shell',
    args: { command: `node -e "setTimeout(() => {}, 300)"`, background: true }
  })
  assert.equal(started.ok, true, started.text)
  const waited = await runTool(live, {
    id: '4',
    name: 'task',
    args: { action: 'wait', id: 'task-1', timeoutMs: 20_000 }
  })
  assert.equal(waited.ok, true, waited.text)
  assert.match(waited.text, /task-1 已结束（退出码 0）/)
})

test('a wait that cannot succeed says why instead of hanging', async (t) => {
  const { server, base } = await localServer((_req, res) => {
    res.writeHead(200)
    res.end()
  })
  const port = Number(new URL(base).port)
  await new Promise<void>((resolve) => server.close(() => resolve()))
  const closed = await runTool(await workspace(), {
    id: '1',
    name: 'wait_for',
    args: { target: 'port', port, timeoutMs: 700, intervalMs: 100 }
  })
  assert.equal(closed.ok, false)
  assert.match(closed.text, /等待超时/)

  // A finished task will never print the line, so the wait ends at once.
  const root = await workspace()
  const tasks = path.join(root, '.dsh-flow', 'tasks')
  await mkdir(tasks, { recursive: true })
  const log = path.join(tasks, 'task-1.log')
  await writeFile(log, 'starting\n', 'utf8')
  await writeFile(
    path.join(tasks, 'task-1.json'),
    JSON.stringify({ id: 'task-1', command: 'npm run dev', cwd: root, pid: 0, startedAt: '', log }),
    'utf8'
  )
  const exited = await runTool(root, {
    id: '2',
    name: 'wait_for',
    args: { target: 'output', id: 'task-1', pattern: 'ready', timeoutMs: 700, intervalMs: 100 }
  })
  assert.equal(exited.ok, false)
  assert.match(exited.text, /已经结束/)
})

test('a read-only conversation does not run tests or a compile check', async () => {
  const before = process.env.DSH_SANDBOX_READONLY
  const root = await workspace()
  try {
    process.env.DSH_SANDBOX_READONLY = '1'
    const tests = await runTool(root, { id: '1', name: 'run_tests', args: {} })
    assert.equal(tests.ok, false)
    assert.match(tests.text, /仅可查看模式不跑测试/)
    const check = await runTool(root, { id: '2', name: 'diagnostics', args: {} })
    assert.equal(check.ok, false)
    assert.match(check.text, /仅可查看模式不跑编译检查/)
  } finally {
    if (before === undefined) delete process.env.DSH_SANDBOX_READONLY
    else process.env.DSH_SANDBOX_READONLY = before
  }
})

test('parses cargo failures and their counts', () => {
  const output = [
    'running 2 tests',
    'test math::adds ... ok',
    'test math::divides ... FAILED',
    '',
    'failures:',
    '',
    '---- math::divides stdout ----',
    "thread 'math::divides' panicked at src/math.rs:7:5:",
    'assertion `left == right` failed',
    '',
    'failures:',
    '    math::divides',
    '',
    'test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out'
  ].join('\n')
  const report = parseCargo(output)
  assert.equal(report.passed, 1)
  assert.equal(report.failedCount, 1)
  assert.equal(report.failures[0]?.name, 'math::divides')
  assert.match(report.failures[0]?.message ?? '', /panicked|assertion/)
})

test('parses go failures and its per-package summary', () => {
  const output = [
    '--- FAIL: TestAdd (0.00s)',
    '    add_test.go:12: got 1, want 2',
    '--- FAIL: TestSub (0.00s)',
    '    sub_test.go:20: negative',
    'ok  \texample.com/math\t0.012s',
    'FAIL\texample.com/strings\t0.020s'
  ].join('\n')
  const report = parseGo(output)
  assert.equal(report.failures.length, 2)
  assert.equal(report.failures[0].name, 'TestAdd')
  assert.match(report.failures[0].message, /got 1, want 2/)
  assert.equal(report.summary, '1 个包通过 / 2 个用例失败')
})

test('parses the node test runner summary and skips its heading', () => {
  const output = [
    '✔ adds two numbers (1.2ms)',
    '✖ failing tests:',
    '✖ divides (2.4ms)',
    'ℹ tests 3',
    'ℹ pass 2',
    'ℹ fail 1'
  ].join('\n')
  const report = parseNpm(output)
  assert.equal(report.passed, 2)
  assert.equal(report.failedCount, 1)
  assert.equal(report.failures.length, 1, 'the heading is not a case')
  assert.equal(report.failures[0]?.name, 'divides', 'the duration is not part of the name')
  assert.equal(report.summary, '2 通过 / 1 失败')
})

test('parses dotnet and maven failures with their counts', () => {
  const dotnet = parseDotnet(
    [
      '  Failed MathTests.Divides [12 ms]',
      '  Error Message:',
      '   Assert.Equal() Failure: Expected 2, Actual 3',
      '  Stack Trace:',
      '     at MathTests.Divides() in Math.cs:line 12',
      '',
      'Failed!  - Failed:     1, Passed:     4, Skipped:     0, Total:     5, Duration: 42 ms'
    ].join('\n')
  )
  assert.equal(dotnet.failures.length, 1)
  assert.equal(dotnet.failures[0]?.name, 'MathTests.Divides')
  assert.match(dotnet.failures[0]?.message ?? '', /Expected 2, Actual 3/)
  assert.equal(dotnet.passed, 4)
  assert.equal(dotnet.failedCount, 1)
  assert.equal(dotnet.summary, '4 通过 / 1 失败')

  const maven = parseMaven(
    [
      '[ERROR] Tests run: 3, Failures: 1, Errors: 0, Skipped: 0, Time elapsed: 0.1 s',
      '[ERROR] Failures: ',
      '[ERROR]   MathTest.divides:42 expected:<2> but was:<3>',
      '[INFO] Tests run: 3, Failures: 1, Errors: 0, Skipped: 0'
    ].join('\n')
  )
  assert.equal(maven.failures.length, 1)
  assert.equal(maven.failures[0]?.name, 'MathTest.divides:42')
  assert.match(maven.failures[0]?.message ?? '', /expected/)
  assert.equal(maven.passed, 2)
  assert.equal(maven.failedCount, 1)
  assert.equal(maven.summary, '2 通过 / 1 失败')
})

test('parses mocha and vitest summaries too', () => {
  const mocha = parseNpm(['  5 passing (12ms)', '  1 failing', '  1) math divides:'].join('\n'))
  assert.equal(mocha.passed, 5)
  assert.equal(mocha.failedCount, 1)

  const vitest = parseNpm(['× divides 12ms', 'Tests  1 failed | 4 passed (5)'].join('\n'))
  assert.equal(vitest.passed, 4)
  assert.equal(vitest.failedCount, 1)
  assert.equal(vitest.failures[0]?.name, 'divides 12ms')
})

test('parses gradle failures and its count', () => {
  const output = [
    'MathTest > divides() FAILED',
    '    org.opentest4j.AssertionFailedError at MathTest.java:42',
    'MathTest > adds() FAILED',
    '    org.opentest4j.AssertionFailedError: expected 2',
    '',
    '2 tests completed, 2 failed',
    'BUILD FAILED in 3s'
  ].join('\n')
  const report = parseGradle(output)
  assert.equal(report.failures.length, 2)
  assert.equal(report.failures[0]?.name, 'MathTest > divides()')
  assert.match(report.failures[0]?.message ?? '', /AssertionFailedError at/)
  assert.equal(report.passed, 0)
  assert.equal(report.failedCount, 2)
  assert.equal(report.summary, '0 通过 / 2 失败')
  // A passing run prints the count without a failure figure.
  const clean = parseGradle('3 tests completed')
  assert.equal(clean.passed, 3)
  assert.equal(clean.failedCount, 0)
  assert.equal(clean.failures.length, 0)
})

test('detects gradle projects from their build files', async () => {
  const root = makeProject({ 'build.gradle.kts': 'plugins {}\n' })
  const plan = await testPlan(root, {})
  assert.equal(plan?.kind, 'gradle')
  assert.deepEqual(plan?.args, ['test', '--console=plain'])
  const filtered = await testPlan(root, { filter: 'MathTest' })
  assert.deepEqual(filtered?.args, ['test', '--console=plain', '--tests', 'MathTest'])
  const check = diagnosticPlan(root, {})
  assert.deepEqual(check?.args, ['classes', '--console=plain'])
})

test('parses kotlin diagnostics with file URLs', () => {
  const report = parseTextDiagnostics(
    [
      'e: file:///E:/src/Foo.kt:3:1 Unresolved reference: x',
      'w: file:///E:/src/Bar.kt:8:5 Variable is never used'
    ].join('\n')
  )
  assert.equal(report.errors[0]?.file, 'E:/src/Foo.kt')
  assert.equal(report.errors[0]?.line, 3)
  assert.equal(report.errors[0]?.column, 1)
  assert.match(report.errors[0]?.message ?? '', /Unresolved reference/)
  assert.equal(report.warnings[0]?.file, 'E:/src/Bar.kt')
  assert.equal(report.warnings[0]?.column, 5)
})

test('detects dotnet and maven projects from their build files', async () => {
  const solution = makeProject({ 'App.sln': 'Microsoft Visual Studio Solution File\n' })
  const dotnet = await testPlan(solution, {})
  assert.equal(dotnet?.kind, 'dotnet')
  assert.deepEqual(dotnet?.args, ['test', '--nologo', 'App.sln'])
  const filtered = await testPlan(solution, { filter: 'Math' })
  assert.ok(filtered?.args.includes('--filter'))
  assert.ok(filtered?.args.some((arg) => arg === 'FullyQualifiedName~Math'))

  const project = makeProject({ 'App.csproj': '<Project />\n' })
  assert.equal((await testPlan(project, {}))?.args.at(-1), 'App.csproj')

  const maven = makeProject({ 'pom.xml': '<project />\n' })
  const plain = await testPlan(maven, {})
  assert.equal(plain?.kind, 'maven')
  assert.deepEqual(plain?.args, ['-B', 'test'])
  const narrowed = await testPlan(maven, { filter: 'MathTest' })
  assert.deepEqual(narrowed?.args, ['-B', 'test', '-Dtest=MathTest'])
})

test('detects go test and go vet from the files in a folder', async () => {
  const root = await workspace()
  await writeFile(path.join(root, 'go.mod'), 'module example.com/app\n\ngo 1.23\n', 'utf8')
  const plain = await testPlan(root, {})
  assert.ok(plain)
  assert.equal(plain.label, 'go test ./...')
  const filtered = await testPlan(root, { filter: 'TestAdd' })
  assert.ok(filtered)
  assert.deepEqual(filtered.args, ['test', '-run', 'TestAdd', './...'])
  const check = diagnosticPlan(root, {})
  assert.ok(check)
  assert.equal(check.label, 'go vet ./...')
})

test('parses go vet text diagnostics', () => {
  const report = parseTextDiagnostics(
    ['# example.com/app', './main.go:10:2: fmt.Printf call needs 1 arg but has 2'].join('\n')
  )
  assert.equal(report.errors.length, 1)
  assert.equal(report.errors[0].file, './main.go')
  assert.equal(report.errors[0].line, 10)
  assert.match(report.errors[0].message, /needs 1 arg/)
})

test('parses pytest, TAP and the generic fallback', () => {
  const pytest = parsePytest(
    'FAILED tests/test_a.py::test_x - AssertionError: 1 != 2\n= 1 failed, 3 passed in 0.11s ='
  )
  assert.equal(pytest.failedCount, 1)
  assert.equal(pytest.passed, 3)
  assert.equal(pytest.failures[0]?.name, 'tests/test_a.py::test_x')
  assert.match(String(pytest.failures[0]?.message), /AssertionError/)

  const tap = parseNpm('TAP version 13\nnot ok 1 - adds two numbers\n# fail 1\n1..1\n')
  assert.equal(tap.failedCount, 1)
  assert.equal(tap.failures[0]?.name, 'adds two numbers')

  const generic = parseGeneric(
    'not ok 1 - x\nsome error happened\n# fail 1\n1 failed, 2 passed\n'
  )
  assert.ok(generic.failures.some((failure) => failure.name === 'x'))
  assert.ok(generic.failures.some((failure) => /some error happened/.test(failure.name)))
  assert.ok(!generic.failures.some((failure) => failure.name.startsWith('#')))
  assert.ok(!generic.failures.some((failure) => /^1 failed/.test(failure.name)))
})

test('detects the test entry from the files in a folder', async () => {
  const cargo = makeProject({ 'Cargo.toml': '[package]\nname = "x"\n' })
  assert.equal((await testPlan(cargo, {}))?.kind, 'cargo')
  const npm = makeProject({
    'package.json': JSON.stringify({ scripts: { test: 'node --test' } })
  })
  assert.equal((await testPlan(npm, {}))?.kind, 'npm')
  const pytest = makeProject({ 'pytest.ini': '[pytest]\n' })
  assert.equal((await testPlan(pytest, {}))?.kind, 'pytest')
  const empty = await workspace()
  assert.equal(await testPlan(empty, {}), null)
  assert.equal((await testPlan(empty, { command: 'node --test' }))?.kind, 'generic')
})

test('runs tests and lists the failing case first', async () => {
  const root = await workspace()
  const result = await runTool(root, {
    id: '1',
    name: 'run_tests',
    args: {
      command: `node -e "console.log('not ok 1 - adds two numbers');console.log('# fail 1');process.exit(1)"`
    }
  })
  assert.equal(result.ok, false, result.text)
  assert.match(result.text, /退出码 1/)
  assert.match(result.text, /- adds two numbers/)
  assert.match(result.text, /输出末尾/)
})

test('run_tests says so when there is nothing to detect', async () => {
  const result = await runTool(await workspace(), { id: '1', name: 'run_tests', args: {} })
  assert.equal(result.ok, false)
  assert.match(result.text, /没有识别到测试入口/)
})

test('refuses to read a credential file', async () => {
  const root = await workspace()
  await writeFile(path.join(root, '.env'), 'API_KEY=sk-live\n', 'utf8')
  const read = await runTool(root, { id: '1', name: 'read', args: { path: '.env' } })
  assert.equal(read.ok, false)
  assert.match(read.text, /凭据|密钥/)

  // The example file holds the shape, not the secret.
  await writeFile(path.join(root, '.env.example'), 'API_KEY=\n', 'utf8')
  const example = await runTool(root, { id: '2', name: 'read', args: { path: '.env.example' } })
  assert.equal(example.ok, true, example.text)

  // Searching skips it too, and says so: a matching line would be just as bad.
  const grep = await runTool(root, { id: '3', name: 'grep', args: { pattern: 'API_KEY' } })
  assert.equal(grep.ok, true, grep.text)
  assert.match(grep.text, /已跳过 1 个凭据/)
  assert.doesNotMatch(grep.text, /sk-live/)
})

test('refuses to show a key file as an image', async () => {
  const root = await workspace()
  await writeFile(path.join(root, 'server.pem'), DOT_PNG)
  const result = await runTool(root, { id: '1', name: 'read_image', args: { path: 'server.pem' } })
  assert.equal(result.ok, false)
  assert.match(result.text, /凭据|密钥/)
})

test('parses cargo json diagnostics', () => {
  const entry = (level: string, message: string, file: string, line: number, column: number, code?: string) =>
    JSON.stringify({
      reason: 'compiler-message',
      message: {
        level,
        message,
        ...(code === undefined ? {} : { code: { code } }),
        spans: [{ file_name: file, line_start: line, column_start: column, is_primary: true }]
      }
    })
  const text = [
    entry('error', 'mismatched types', 'src/main.rs', 12, 5, 'E0308'),
    JSON.stringify({ reason: 'compiler-artifact', message: { level: 'error' } }),
    entry('warning', 'unused variable: `x`', 'src/lib.rs', 3, 9),
    '{"reason":"compiler-message"'
  ].join('\n')
  const report = parseCargoDiagnostics(text)
  assert.equal(report.errors.length, 1)
  assert.equal(report.errors[0]?.file, 'src/main.rs')
  assert.equal(report.errors[0]?.line, 12)
  assert.equal(report.errors[0]?.code, 'E0308')
  assert.equal(report.warnings.length, 1)
  assert.equal(report.warnings[0]?.line, 3)
})

test('parses tsc and rustc text diagnostics', () => {
  const text = [
    "src/a.ts(3,5): error TS2322: Type 'string' is not assignable to type 'number'.",
    "src/b.ts(1,1): warning TS6133: 'x' is declared but its value is never read."
  ].join('\n')
  const report = parseTextDiagnostics(text)
  assert.equal(report.errors[0]?.file, 'src/a.ts')
  assert.equal(report.errors[0]?.line, 3)
  assert.equal(report.errors[0]?.code, 'TS2322')
  assert.match(report.errors[0]?.message ?? '', /not assignable/)
  assert.equal(report.warnings[0]?.code, 'TS6133')

  // rustc puts the span on the line after the headline.
  const rust = parseTextDiagnostics(
    [
      'error[E0425]: cannot find value `x` in this scope',
      ' --> src/main.rs:7:9',
      'warning: unused import',
      '  --> src/lib.rs:2:5'
    ].join('\n')
  )
  assert.equal(rust.errors[0]?.file, 'src/main.rs')
  assert.equal(rust.errors[0]?.line, 7)
  assert.equal(rust.errors[0]?.code, 'E0425')
  assert.equal(rust.warnings[0]?.line, 2)
})

test('parses eslint, maven and javac diagnostics', () => {
  const report = parseTextDiagnostics(
    [
      '/repo/src/a.js: line 4, col 9, Error - "x" is not defined. (no-undef)',
      '/repo/src/b.js: line 1, col 1, Warning - Unexpected var. (no-var)',
      '[ERROR] /repo/src/Foo.java:[12,5] cannot find symbol',
      'src/Bar.java:7: error: reached end of file while parsing'
    ].join('\n')
  )
  assert.equal(report.errors.length, 3)
  assert.equal(report.errors[0]?.file, '/repo/src/a.js')
  assert.equal(report.errors[0]?.line, 4)
  assert.equal(report.errors[0]?.column, 9)
  assert.equal(report.errors[0]?.code, 'no-undef')
  assert.match(report.errors[0]?.message ?? '', /is not defined/)
  assert.equal(report.errors[1]?.file, '/repo/src/Foo.java')
  assert.equal(report.errors[1]?.line, 12)
  assert.equal(report.errors[1]?.column, 5)
  assert.equal(report.errors[2]?.file, 'src/Bar.java')
  assert.equal(report.errors[2]?.column, 0, 'javac prints no column')
  assert.equal(report.warnings[0]?.code, 'no-var')

  // Roslyn prints the same `file(line,col): error CODE: message` shape as tsc,
  // and appends the project path in brackets after the message.
  const roslyn = parseTextDiagnostics(
    'Program.cs(12,5): error CS0103: The name \'x\' does not exist in the current context [C:\\app\\App.csproj]'
  )
  assert.equal(roslyn.errors[0]?.code, 'CS0103')
  assert.equal(roslyn.errors[0]?.file, 'Program.cs')
  assert.doesNotMatch(roslyn.errors[0]?.message ?? '', /App\.csproj/)
})

test('detects dotnet, maven and eslint compile entries', () => {
  const solution = makeProject({ 'App.sln': 'Microsoft Visual Studio Solution File\n' })
  assert.deepEqual(diagnosticPlan(solution, {})?.args, [
    'build',
    '--nologo',
    '-v',
    'minimal',
    'App.sln'
  ])
  const maven = makeProject({ 'pom.xml': '<project />\n' })
  assert.equal(diagnosticPlan(maven, {})?.label, 'mvn -q -DskipTests compile')
  const eslint = makeProject({ 'eslint.config.js': 'export default []\n' })
  const plan = diagnosticPlan(eslint, {})
  assert.deepEqual(plan?.args, ['--no-install', 'eslint', '.', '-f', 'compact'])
  assert.equal(plan?.file, 'npx')
})

test('detects the compile entry from the files in a folder', async () => {
  const cargo = makeProject({ 'Cargo.toml': '[package]\nname = "x"\n' })
  assert.equal(diagnosticPlan(cargo, {})?.kind, 'cargo')
  const ts = makeProject({ 'tsconfig.json': '{}' })
  assert.equal(diagnosticPlan(ts, {})?.kind, 'text')
  const empty = await workspace()
  assert.equal(diagnosticPlan(empty, {}), null)
})

test('runs a compile check and lists what it found', async () => {
  const root = await workspace()
  const result = await runTool(root, {
    id: '1',
    name: 'diagnostics',
    args: {
      command: `node -e "console.log('src/a.ts(3,5): error TS2322: bad type');process.exit(1)"`
    }
  })
  assert.equal(result.ok, false, result.text)
  assert.match(result.text, /统计：1 错误 \/ 0 警告/)
  assert.match(result.text, /- src\/a\.ts:3:5 TS2322 — bad type/)
})

test('severity filters the list but keeps both counts', async () => {
  const root = await workspace()
  const result = await runTool(root, {
    id: '1',
    name: 'diagnostics',
    args: {
      severity: 'warning',
      command: `node -e "console.log('src/a.ts(1,1): error TS1: boom');console.log('src/b.ts(2,2): warning TS2: careful');process.exit(1)"`
    }
  })
  assert.match(result.text, /统计：1 错误 \/ 1 警告/)
  assert.doesNotMatch(result.text, /- src\/a\.ts/)
  assert.match(result.text, /- src\/b\.ts:2:2 TS2 — careful/)
})

test('parses cargo json coming from an explicit command', async () => {
  const root = await workspace()
  const result = await runTool(root, {
    id: '1',
    name: 'diagnostics',
    args: {
      format: 'cargo',
      command: `node -e "process.stderr.write('checking crate\\n');console.log(JSON.stringify({reason:'compiler-message',message:{level:'error',code:{code:'E0308'},message:'mismatched types',spans:[{file_name:'src/a.rs',line_start:2,column_start:3,is_primary:true}]}}));process.exit(1)"`
    }
  })
  assert.equal(result.ok, false, result.text)
  assert.match(result.text, /- src\/a\.rs:2:3 E0308 — mismatched types/)
  // stderr is context, not a source of diagnostics: it lands in the tail.
  assert.match(result.text, /checking crate/)
})

test('a test filter reaches npm as a script argument', async () => {
  const root = await workspace()
  await writeFile(path.join(root, 'package.json'), JSON.stringify({ scripts: { test: 'vitest' } }), 'utf8')
  const plain = await testPlan(root, {})
  assert.ok(plain)
  assert.deepEqual(plain.args, ['test'])
  const filtered = await testPlan(root, { filter: 'adds' })
  assert.ok(filtered)
  assert.deepEqual(filtered.args, ['test', '--', 'adds'])
  assert.equal(filtered.label, 'npm test -- adds')
})

test('capture decodes GBK output and forwards lines while running', async () => {
  const root = await workspace()
  // 中文 in the OEM code page: 0xD6D0 0xCEC4. Writing it as bytes proves the
  // capture path no longer hands every chunk to `String()`.
  const plan = {
    file: 'node',
    args: [
      '-e',
      `process.stdout.write(Buffer.from([0xd6,0xd0,0xce,0xc4,0x0a]));` +
        `setTimeout(() => { process.stdout.write('build: ok\\n') }, 120)`
    ],
    label: 'node -e gbk',
    shell: false
  }
  const live: string[] = []
  const outcome = await captureCommand(plan, root, 10_000, (text) => live.push(text))
  assert.equal(outcome.code, 0)
  assert.match(outcome.stdout, /中文/)
  assert.match(outcome.stdout, /build: ok/)
  assert.ok(
    live.some((line) => line.includes('build: ok')),
    `live output should carry the second line: ${JSON.stringify(live)}`
  )
})

test('captures stdout and stderr apart so one cannot corrupt the other', async () => {
  // No shell in between: the payload itself contains quotes, and Windows would
  // read them as the end of the command line.
  const code = `process.stderr.write('progress line\\n');process.stdout.write(JSON.stringify({a:1}));process.exit(3)`
  const result = await captureCommand(
    { file: process.execPath, args: ['-e', code], label: 'node -e', shell: false },
    await workspace(),
    10_000
  )
  assert.equal(result.code, 3)
  assert.match(result.stdout, /"a":1/)
  assert.match(result.stderr, /progress line/)
  // The merge keeps stdout first: a line of progress can never land inside a
  // report line and break its parse.
  assert.ok(result.text.indexOf('"a":1') < result.text.indexOf('progress line'), result.text)
})

test('reports an unknown tool', async () => {
  const root = await workspace()
  const result = await runTool(root, { id: '1', name: 'nope', args: {} })
  assert.equal(result.ok, false)
})

test('wildcards stay inside one directory segment', () => {
  assert.ok(wildcard('*.ts').test('a.ts'))
  assert.ok(!wildcard('*.ts').test('src/a.ts'))
  assert.ok(wildcard('**/*.ts').test('src/deep/a.ts'))
  assert.ok(wildcard('src/*.json').test('src/a.json'))
  assert.ok(!wildcard('src/*.json').test('src/deep/a.json'))
})
