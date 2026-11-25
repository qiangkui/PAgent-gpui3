import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { createServer } from 'node:http'
import type { IncomingMessage, Server, ServerResponse } from 'node:http'
import { existsSync } from 'node:fs'
import { mkdir, mkdtemp, readFile, truncate, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { test } from 'node:test'
import {
  MAX_CODE_CHARS,
  MAX_EDIT_BYTES,
  MAX_GREP_BYTES,
  MAX_HTTP_CHARS,
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
import { stubFetch } from './helpers.ts'

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
      'read', 'write', 'edit', 'replace', 'move', 'delete', 'list', 'glob', 'tree', 'grep',
      'symbols', 'git', 'shell', 'task', 'python', 'node', 'http'
    ]
  )
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

test('http does not follow a redirect and names the target', async (t) => {
  const { server, base } = await localServer((_req, res) => {
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
  assert.match(result.text, /302/)
  assert.match(result.text, /location: \/next/)
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
  assert.match(folder.text, /只能删除文件/)

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
