import assert from 'node:assert/strict'
import { spawn, spawnSync } from 'node:child_process'
import { existsSync, readFileSync } from 'node:fs'
import { mkdir, mkdtemp, readFile } from 'node:fs/promises'
import { createServer } from 'node:http'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { test } from 'node:test'
import { makeProject, nodeEcho } from './helpers.ts'

const CLI = fileURLToPath(new URL('../src/cli.ts', import.meta.url))

/**
 * Invoke the CLI.
 * @param args - arguments after the script path.
 * @returns exit code and output.
 */
function cli(args: string[]): { code: number, out: string, stdout: string } {
  const result = spawnSync(process.execPath, [CLI, ...args], { encoding: 'utf8' })
  return {
    code: result.status ?? -1,
    out: `${result.stdout ?? ''}${result.stderr ?? ''}`,
    stdout: result.stdout ?? '',
  }
}

/**
 * Invoke the CLI, writing `stdin` and waiting without blocking the event loop.
 * @param args - arguments after the script path.
 * @param stdin - text written to the child's stdin, which is then closed.
 * @param cwd - working directory for the child, when the call depends on it.
 * @returns exit code and merged output.
 */
async function cliAsync(args: string[], stdin: string, cwd?: string): Promise<{ code: number, out: string }> {
  const child = spawn(process.execPath, [CLI, ...args], { cwd, stdio: ['pipe', 'pipe', 'pipe'] })
  let out = ''
  child.stdout.setEncoding('utf8').on('data', chunk => {
    out += chunk
  })
  child.stderr.setEncoding('utf8').on('data', chunk => {
    out += chunk
  })
  child.stdin.end(stdin)
  const code = await new Promise<number>(resolve => {
    child.on('close', value => resolve(value ?? -1))
  })
  return { code, out }
}

test('validate reports ok and explains the plan', () => {
  const dir = makeProject({ 'flow.yml': `name: demo\ndescription: d\nsteps:\n  - id: a\n    run: [node, -e, '0']\n` })
  const validated = cli(['validate', `${dir}/flow.yml`])
  assert.equal(validated.code, 0)
  assert.match(validated.out, /ok: demo/)

  const explained = cli(['explain', `${dir}/flow.yml`])
  assert.equal(explained.code, 0)
  assert.match(explained.out, /100% executed without a model/)
})

test('validate accepts a shell command line', () => {
  const dir = makeProject({ 'flow.yml': 'name: demo\ndescription: d\nsteps:\n  - id: a\n    run: echo hello\n' })
  const validated = cli(['validate', `${dir}/flow.yml`])
  assert.equal(validated.code, 0, validated.out)
  const ran = cli(['run', `${dir}/flow.yml`])
  assert.equal(ran.code, 0, ran.out)
  assert.match(ran.out, /\[OK  \] a/)
})

test('validate exits non-zero when a runtime is missing', () => {
  const dir = makeProject({ 'flow.yml': 'name: demo\ndescription: d\nsteps:\n  - id: a\n    run: [no-such-runtime-xyz, --version]\n' })
  const result = cli(['validate', `${dir}/flow.yml`])
  assert.equal(result.code, 2)
  assert.match(result.out, /not found/)
})

test('run exits 1 and names the failing step', () => {
  const dir = makeProject({
    'flow.yml': 'name: demo\ndescription: d\nsteps:\n  - id: boom\n    run: [node, -e, \'process.exit(2)\']\n',
  })
  const result = cli(['run', `${dir}/flow.yml`])
  assert.equal(result.code, 1)
  assert.match(result.out, /\[FAIL\] boom/)
  assert.match(result.out, /report: .*\.html/)
})

test('--dry renders without executing', () => {
  const dir = makeProject({
    'flow.yml': `name: demo
description: d
steps:
  - id: side
    run: ${nodeEcho('{ran:true}')}
  - id: keep
    set:
      ok: 1
`,
  })
  const dry = cli(['run', `${dir}/flow.yml`, '--dry', '--json'])
  assert.equal(dry.code, 0)
  const summary = JSON.parse(dry.stdout.slice(dry.stdout.indexOf('{'))) as { steps: Record<string, { status: string, error?: string }> }
  assert.equal(summary.steps.side?.status, 'skipped')
  assert.equal(summary.steps.side?.error, 'dry run')
  assert.equal(summary.steps.keep?.status, 'ok')
})

test('init scaffolds a workflow that runs', () => {
  const dir = makeProject({})
  const created = cli(['init', `${dir}/sample`, '--lang', 'node'])
  assert.equal(created.code, 0)
  const ran = cli(['run', `${dir}/sample/flow.yml`, '--input', 'who=you'])
  assert.equal(ran.code, 0)
  assert.match(ran.out, /\[OK  \] greet/)
})

const AI_STEP = `  - id: classify
    why: 需要读懂自由文本才能归类
    save: verdict
    ai:
      prompt: "分类：{{ inputs.text }}"
      schema:
        type: object
        required: [category]
        properties:
          category:
            type: string
`

test('run reruns a once step only with --force', () => {
  const dir = makeProject({
    'flow.yml': `name: once-cli
description: d
steps:
  - id: mark
    once: true
    run: [node, steps/mark.mjs]
`,
    'steps/mark.mjs': "import { appendFileSync } from 'node:fs'\nappendFileSync('count.txt', 'x')\n",
  })
  const manifest = join(dir, 'flow.yml')
  const first = cli(['run', manifest])
  const second = cli(['run', manifest])
  const forced = cli(['run', manifest, '--force'])
  assert.equal(first.code, 0, first.out)
  assert.equal(second.code, 0, second.out)
  assert.equal(forced.code, 0, forced.out)
  assert.match(second.out, /SKIP/)
  assert.equal(readFileSync(join(dir, 'count.txt'), 'utf8'), 'xx')
})

test('validate enforces why and schema on ai steps unless --no-strict', () => {
  const dir = makeProject({
    'flow.yml': `name: demo
description: d
inputs:
  text:
    default: hi
steps:
  - id: loose
    ai:
      prompt: hi
`,
  })
  const strict = cli(['validate', `${dir}/flow.yml`])
  assert.equal(strict.code, 2)
  assert.match(strict.out, /需要 "why"/)
  assert.match(strict.out, /需要 "schema"/)

  const lax = cli(['validate', `${dir}/flow.yml`, '--no-strict'])
  assert.equal(lax.code, 0)
  assert.match(lax.out, /1 ai step/)
})

test('explain reports the ai dependency ratio and each reason', () => {
  const dir = makeProject({
    'flow.yml': `name: demo
description: d
inputs:
  text:
    default: hi
steps:
${AI_STEP}  - id: show
    set:
      line: "{{ vars.verdict.category }}"
`,
  })
  const explained = cli(['explain', `${dir}/flow.yml`])
  assert.equal(explained.code, 0)
  assert.match(explained.out, /run 0, set 1, map 0, ai 1/)
  assert.match(explained.out, /50% executed without a model/)
  assert.match(explained.out, /ai 判断点 \(1\/2\)/)
  assert.match(explained.out, /需要读懂自由文本才能归类/)
})

test('run --level manual replays a recorded decision without a model', () => {
  const dir = makeProject({
    'flow.yml': `name: demo
description: d
inputs:
  text:
    default: hi
steps:
  - id: classify
    why: 判断点
    save: verdict
    ai:
      prompt: classify
      backend: cli
      command: ${nodeEcho('{text: JSON.stringify({category:"bug"})}')}
      schema:
        type: object
        required: [category]
        properties:
          category:
            type: string
  - id: show
    set:
      line: "{{ vars.verdict.category }}"
`,
  })
  const drafted = cli(['run', `${dir}/flow.yml`, '--level', 'draft'])
  assert.equal(drafted.code, 0)
  assert.match(drafted.out, /\[OK  \] classify/)
  assert.match(drafted.out, /1 ai \(draft\)/)

  const manual = cli(['run', `${dir}/flow.yml`, '--level', 'manual'])
  assert.equal(manual.code, 0)
  assert.match(manual.out, /复用记录/)

  const other = makeProject({
    'flow.yml': `name: demo
description: d
steps:
  - id: classify
    why: 判断点
    ai:
      prompt: classify
      schema:
        type: object
`,
  })
  const missing = cli(['run', `${other}/flow.yml`, '--level', 'manual'])
  assert.equal(missing.code, 1)
  assert.match(missing.out, /manual 档不调用模型/)
})

const CREDENTIAL_MANIFEST = `name: creds
description: d
steps:
  - id: classify
    why: 判断点
    ai:
      prompt: classify
      schema:
        type: object
        required: [category]
        properties:
          category:
            type: string
`

/**
 * Answer the first model call with one `write` tool call, then with "done".
 * @param args - arguments the model is told to pass to `write`.
 * @returns the port to point the bridge at, and a closer.
 */
async function writeToolServer(args: Record<string, unknown>): Promise<{
  port: number
  close: () => Promise<void>
}> {
  let round = 0
  const server = createServer((request, response) => {
    request.resume()
    request.on('end', () => {
      round += 1
      response.writeHead(200, { 'content-type': 'application/json' })
      if (round === 1) {
        response.end(JSON.stringify({
          choices: [{
            message: {
              content: null,
              tool_calls: [{
                id: 'call-1',
                type: 'function',
                function: { name: 'write', arguments: JSON.stringify(args) }
              }]
            }
          }]
        }))
      } else {
        response.end(JSON.stringify({ choices: [{ message: { content: 'done' } }] }))
      }
    })
  })
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve))
  const address = server.address()
  const port = typeof address === 'object' && address !== null ? address.port : 0
  return {
    port,
    close: async () => {
      await new Promise<void>(resolve => server.close(() => resolve()))
    }
  }
}

test('interactive ai waits for a write approval on the same pipe', async () => {
  let round = 0
  const server = createServer((request, response) => {
    request.resume()
    request.on('end', () => {
      round += 1
      response.writeHead(200, { 'content-type': 'application/json' })
      if (round === 1) {
        response.end(JSON.stringify({
          choices: [{
            message: {
              content: null,
              tool_calls: [{
                id: 'call-1',
                type: 'function',
                function: {
                  name: 'write',
                  arguments: JSON.stringify({ path: 'note.txt', content: 'hello\n' })
                }
              }]
            }
          }]
        }))
      } else {
        response.end(JSON.stringify({ choices: [{ message: { content: 'written' } }] }))
      }
    })
  })
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve))
  const address = server.address()
  const port = typeof address === 'object' && address !== null ? address.port : 0
  const dir = await mkdtemp(join(tmpdir(), 'dsh-approve-'))
  const child = spawn(process.execPath, [CLI, 'ai'], {
    cwd: dir,
    stdio: ['pipe', 'pipe', 'pipe']
  })

  try {
    let out = ''
    let resolveApproval: (value: Record<string, unknown>) => void = () => undefined
    const asked = new Promise<Record<string, unknown>>(resolve => {
      resolveApproval = resolve
    })
    let pending = ''
    child.stdout.setEncoding('utf8').on('data', chunk => {
      pending += chunk
      let newline = pending.indexOf('\n')
      while (newline >= 0) {
        const line = pending.slice(0, newline).trim()
        pending = pending.slice(newline + 1)
        newline = pending.indexOf('\n')
        if (!line) continue
        out += `${line}\n`
        const value = JSON.parse(line) as Record<string, unknown>
        if (value.type === 'approval') resolveApproval(value)
      }
    })
    child.stderr.setEncoding('utf8').on('data', chunk => {
      out += chunk
    })

    child.stdin.write(`${JSON.stringify({
      prompt: 'write note.txt',
      apiKey: 'sk-test',
      baseUrl: `http://127.0.0.1:${port}`,
      model: 'm',
      tools: true,
      interactive: true,
      confirmWrites: true
    })}\n`)

    const question = await Promise.race([
      asked,
      new Promise<never>((_, reject) => {
        setTimeout(() => reject(new Error(`no approval arrived: ${out}`)), 5000)
      })
    ])
    assert.equal(question.tool, 'write')
    assert.equal(question.path, 'note.txt')
    assert.match(String(question.diff), /^\+ hello$/m)
    assert.equal(question.after, 'hello\n')
    assert.equal(question.existed, false)
    assert.equal(question.undoable, true)
    child.stdin.write(`${JSON.stringify({
      type: 'approval',
      call: question.call,
      allow: true
    })}\n`)

    const code = await new Promise<number>(resolve => {
      child.on('close', value => resolve(value ?? -1))
    })
    assert.equal(code, 0, out)
    assert.equal(await readFile(join(dir, 'note.txt'), 'utf8'), 'hello\n')
    assert.match(out, /written/)
  } finally {
    child.kill()
    await new Promise<void>(resolve => server.close(() => resolve()))
  }
})

test('ai writes straight away when the window allows it', async () => {
  let round = 0
  const server = createServer((request, response) => {
    request.resume()
    request.on('end', () => {
      round += 1
      response.writeHead(200, { 'content-type': 'application/json' })
      if (round === 1) {
        response.end(JSON.stringify({
          choices: [{
            message: {
              content: null,
              tool_calls: [{
                id: 'call-1',
                type: 'function',
                function: {
                  name: 'write',
                  arguments: JSON.stringify({ path: 'note.txt', content: 'hello\n' })
                }
              }]
            }
          }]
        }))
      } else {
        response.end(JSON.stringify({ choices: [{ message: { content: 'written' } }] }))
      }
    })
  })
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve))
  const address = server.address()
  const port = typeof address === 'object' && address !== null ? address.port : 0
  const dir = await mkdtemp(join(tmpdir(), 'dsh-direct-'))

  try {
    const { code, out } = await cliAsync(
      ['ai'],
      `${JSON.stringify({
        prompt: 'write note.txt',
        apiKey: 'sk-test',
        baseUrl: `http://127.0.0.1:${port}`,
        model: 'm',
        tools: true,
        interactive: true,
        confirmWrites: false
      })}\n`,
      dir
    )
    assert.equal(code, 0, out)
    assert.equal(await readFile(join(dir, 'note.txt'), 'utf8'), 'hello\n')
    assert.match(out, /"type":"approval","auto":true/)
    assert.match(out, /written/)
  } finally {
    await new Promise<void>(resolve => server.close(() => resolve()))
  }
})

test('ai always asks before a command, even when writes are direct', async () => {
  let round = 0
  const command = `node -e "require('fs').writeFileSync('ran.txt','yes')"`
  const server = createServer((request, response) => {
    request.resume()
    request.on('end', () => {
      round += 1
      response.writeHead(200, { 'content-type': 'application/json' })
      if (round === 1) {
        response.end(JSON.stringify({
          choices: [{
            message: {
              content: null,
              tool_calls: [{
                id: 'call-1',
                type: 'function',
                function: {
                  name: 'shell',
                  arguments: JSON.stringify({ command })
                }
              }]
            }
          }]
        }))
      } else {
        response.end(JSON.stringify({ choices: [{ message: { content: 'ran' } }] }))
      }
    })
  })
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve))
  const address = server.address()
  const port = typeof address === 'object' && address !== null ? address.port : 0
  const dir = await mkdtemp(join(tmpdir(), 'dsh-command-'))
  const child = spawn(process.execPath, [CLI, 'ai'], {
    cwd: dir,
    stdio: ['pipe', 'pipe', 'pipe']
  })

  try {
    let out = ''
    let resolveApproval: (value: Record<string, unknown>) => void = () => undefined
    const asked = new Promise<Record<string, unknown>>(resolve => {
      resolveApproval = resolve
    })
    let pending = ''
    child.stdout.setEncoding('utf8').on('data', chunk => {
      pending += chunk
      let newline = pending.indexOf('\n')
      while (newline >= 0) {
        const line = pending.slice(0, newline).trim()
        pending = pending.slice(newline + 1)
        newline = pending.indexOf('\n')
        if (!line) continue
        out += `${line}\n`
        const value = JSON.parse(line) as Record<string, unknown>
        if (value.type === 'approval') resolveApproval(value)
      }
    })
    child.stderr.setEncoding('utf8').on('data', chunk => {
      out += chunk
    })

    child.stdin.write(`${JSON.stringify({
      prompt: 'run a command',
      apiKey: 'sk-test',
      baseUrl: `http://127.0.0.1:${port}`,
      model: 'm',
      tools: true,
      interactive: true,
      confirmWrites: false
    })}\n`)

    const question = await Promise.race([
      asked,
      new Promise<never>((_, reject) => {
        setTimeout(() => reject(new Error(`no approval arrived: ${out}`)), 5000)
      })
    ])
    assert.equal(question.tool, 'shell')
    assert.equal(question.path, '')
    assert.equal(question.auto, false)
    assert.equal(question.command, command)
    assert.match(String(question.diff), /^\$ node -e/)
    child.stdin.write(`${JSON.stringify({
      type: 'approval',
      call: question.call,
      allow: true
    })}\n`)

    const code = await new Promise<number>(resolve => {
      child.on('close', value => resolve(value ?? -1))
    })
    assert.equal(code, 0, out)
    assert.equal(await readFile(join(dir, 'ran.txt'), 'utf8'), 'yes')
    assert.match(out, /"tool":"shell"/)
    assert.match(out, /"ok":true/)
    assert.match(out, /ran/)
  } finally {
    child.kill()
    await new Promise<void>(resolve => server.close(() => resolve()))
  }
})

test('workspace scope allows siblings but refuses paths outside it', async () => {
  const scope = await mkdtemp(join(tmpdir(), 'dsh-scope-'))
  const chat = join(scope, 'workspace', 'chat-1')
  await mkdir(chat, { recursive: true })

  const inside = await writeToolServer({ path: '../../shared.txt', content: 'inside\n' })
  try {
    const { code, out } = await cliAsync(
      ['ai'],
      `${JSON.stringify({
        prompt: 'write shared.txt',
        apiKey: 'sk-test',
        baseUrl: `http://127.0.0.1:${inside.port}`,
        model: 'm',
        tools: true,
        interactive: true,
        confirmWrites: false,
        fileScope: 'workspace',
        scopeRoot: scope
      })}\n`,
      chat
    )
    assert.equal(code, 0, out)
    assert.equal(await readFile(join(scope, 'shared.txt'), 'utf8'), 'inside\n')
  } finally {
    await inside.close()
  }

  const outside = join(tmpdir(), `dsh-outside-${Date.now()}.txt`)
  const refused = await writeToolServer({ path: outside, content: 'no' })
  try {
    const { code, out } = await cliAsync(
      ['ai'],
      `${JSON.stringify({
        prompt: 'write outside',
        apiKey: 'sk-test',
        baseUrl: `http://127.0.0.1:${refused.port}`,
        model: 'm',
        tools: true,
        interactive: true,
        confirmWrites: false,
        fileScope: 'workspace',
        scopeRoot: scope
      })}\n`,
      chat
    )
    assert.equal(code, 0, out)
    assert.match(out, /"ok":false/)
    assert.match(out, /超出工作区/)
    assert.ok(!existsSync(outside), 'nothing may be written outside the workspace')
  } finally {
    await refused.close()
  }
})

test('full scope writes any path', async () => {
  const work = await mkdtemp(join(tmpdir(), 'dsh-full-'))
  const target = join(await mkdtemp(join(tmpdir(), 'dsh-target-')), 'deep', 'note.txt')
  const server = await writeToolServer({ path: target, content: 'wide\n' })
  try {
    const { code, out } = await cliAsync(
      ['ai'],
      `${JSON.stringify({
        prompt: 'write anywhere',
        apiKey: 'sk-test',
        baseUrl: `http://127.0.0.1:${server.port}`,
        model: 'm',
        tools: true,
        interactive: true,
        confirmWrites: false,
        fileScope: 'full'
      })}\n`,
      work
    )
    assert.equal(code, 0, out)
    assert.equal(await readFile(target, 'utf8'), 'wide\n')
  } finally {
    await server.close()
  }
})

test('--credentials-stdin gives ai steps the endpoint and keeps the key out of the output', async () => {
  let authorization = ''
  let received = ''
  const server = createServer((request, response) => {
    authorization = String(request.headers.authorization ?? '')
    request.setEncoding('utf8')
    request.on('data', chunk => {
      received += chunk
    })
    request.on('end', () => {
      response.writeHead(200, { 'content-type': 'application/json' })
      response.end(JSON.stringify({
        choices: [{ message: { content: '{"category":"bug"}' } }],
        usage: { total_tokens: 3 },
      }))
    })
  })
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve))
  const address = server.address()
  const port = typeof address === 'object' && address !== null ? address.port : 0

  try {
    const dir = makeProject({ 'flow.yml': CREDENTIAL_MANIFEST })
    const result = await cliAsync(
      ['run', `${dir}/flow.yml`, '--level', 'draft', '--credentials-stdin'],
      JSON.stringify({
        apiKey: 'sk-test-123',
        baseUrl: `http://127.0.0.1:${port}`,
        model: 'test-model',
      }),
    )
    assert.equal(result.code, 0, result.out)
    assert.equal(authorization, 'Bearer sk-test-123')
    assert.ok(!result.out.includes('sk-test-123'), 'the key must not reach the output')
    assert.match(result.out, /\[OK  \] classify/)
    assert.equal((JSON.parse(received) as { model?: string }).model, 'test-model')
  } finally {
    await new Promise<void>(resolve => server.close(() => resolve()))
  }
})

test('ai carries image attachments as content parts', async () => {
  let received = ''
  const server = createServer((request, response) => {
    request.setEncoding('utf8')
    request.on('data', chunk => {
      received += chunk
    })
    request.on('end', () => {
      response.writeHead(200, { 'content-type': 'application/json' })
      response.end(JSON.stringify({ choices: [{ message: { content: 'ok' } }] }))
    })
  })
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve))
  const address = server.address()
  const port = typeof address === 'object' && address !== null ? address.port : 0
  try {
    const result = await cliAsync(
      ['ai'],
      JSON.stringify({
        prompt: '看看这张图',
        apiKey: 'sk-test',
        baseUrl: `http://127.0.0.1:${port}`,
        model: 'test-model',
        attachments: [
          { name: 'a.png', kind: 'image', mimeType: 'image/png', data: 'aGk=' }
        ]
      }),
    )
    assert.equal(result.code, 0, result.out)
    const body = JSON.parse(received) as {
      messages: Array<{ role: string, content: unknown }>
    }
    assert.deepEqual(body.messages[0]!.content, [
      { type: 'text', text: '看看这张图' },
      { type: 'image_url', image_url: { url: 'data:image/png;base64,aGk=' } }
    ])
  } finally {
    await new Promise<void>(resolve => server.close(() => resolve()))
  }
})

test('ai without tools still reports the round counts', async () => {
  const server = createServer((request, response) => {
    request.resume()
    request.on('end', () => {
      response.writeHead(200, { 'content-type': 'application/json' })
      response.end(JSON.stringify({
        choices: [{ message: { content: 'ok' } }],
        usage: {
          prompt_tokens: 120,
          completion_tokens: 8,
          total_tokens: 128,
          prompt_tokens_details: { cached_tokens: 40 }
        }
      }))
    })
  })
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve))
  const address = server.address()
  const port = typeof address === 'object' && address !== null ? address.port : 0
  try {
    const result = await cliAsync(
      ['ai'],
      JSON.stringify({
        prompt: '你好',
        apiKey: 'sk-test',
        baseUrl: `http://127.0.0.1:${port}`,
        model: 'm',
        tools: false
      }),
    )
    assert.equal(result.code, 0, result.out)
    const last = result.out.trim().split('\n').filter(line => line.startsWith('{')).pop() ?? '{}'
    const answer = JSON.parse(last) as {
      rounds?: number
      roundTokens?: number[][]
      cacheHits?: number
    }
    // One round with no tool calls, carrying the split the window needs to show
    // that request's prompt and answer beside its bill.
    assert.equal(answer.rounds, 1)
    assert.deepEqual(answer.roundTokens, [[120, 8, 0]])
    assert.equal(answer.cacheHits, 40)
  } finally {
    await new Promise<void>(resolve => server.close(() => resolve()))
  }
})

test('models probes the configured model when the endpoint has no /models', async () => {
  const server = createServer((request, response) => {
    const url = request.url ?? ''
    request.resume()
    if (url.endsWith('/models')) {
      response.writeHead(404, { 'content-type': 'application/json' })
      response.end(JSON.stringify({ error: { message: 'not found' } }))
      return
    }
    response.writeHead(200, { 'content-type': 'application/json' })
    response.end(JSON.stringify({ choices: [{ message: { content: 'ok' } }] }))
  })
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve))
  const address = server.address()
  const port = typeof address === 'object' && address !== null ? address.port : 0
  try {
    const result = await cliAsync(
      ['models'],
      JSON.stringify({
        apiKey: 'sk-test',
        baseUrl: `http://127.0.0.1:${port}`,
        model: 'probe-model',
      }),
    )
    assert.equal(result.code, 0, result.out)
    const last = result.out.trim().split('\n').at(-1) ?? ''
    const parsed = JSON.parse(last) as { models?: string[], degraded?: boolean, reason?: string }
    assert.deepEqual(parsed.models, ['probe-model'])
    assert.equal(parsed.degraded, true)
    assert.match(parsed.reason ?? '', /404|not found/)
  } finally {
    await new Promise<void>(resolve => server.close(() => resolve()))
  }
})

test('--credentials-stdin rejects a payload without a key', async () => {
  const dir = makeProject({ 'flow.yml': CREDENTIAL_MANIFEST })
  const result = await cliAsync(
    ['run', `${dir}/flow.yml`, '--level', 'draft', '--credentials-stdin'],
    JSON.stringify({ baseUrl: 'http://127.0.0.1:1' }),
  )
  assert.equal(result.code, 1)
  assert.match(result.out, /缺少 apiKey/)
})

test('derive writes a reviewed-later skeleton next to the session', () => {
  const dir = makeProject({
    'session.jsonl': [
      JSON.stringify({ type: 'session', version: 0, id: 'cli-session', cwd: 'E:/work' }),
      JSON.stringify({
        type: 'session_event',
        event: {
          type: 'tool/call',
          seq: 2,
          time: 0,
          data: { callId: 'c2', name: 'bash', arguments: JSON.stringify({ command: 'node -e console.log(5)' }) },
        },
      }),
      JSON.stringify({
        type: 'session_event',
        event: {
          type: 'tool/call',
          seq: 3,
          time: 0,
          data: { callId: 'c3', name: 'read', arguments: JSON.stringify({ path: 'a.txt' }) },
        },
      }),
    ].join('\n'),
  })
  const derived = cli(['derive', `${dir}/session.jsonl`, '--out', `${dir}/derived`, '--name', 'from-session'])
  assert.equal(derived.code, 0, derived.out)
  const file = join(dir, 'derived', 'flow.yml')
  const text = readFileSync(file, 'utf8')
  assert.match(text, /name: "from-session"/)
  assert.match(text, /read：path=a\.txt/)

  const validated = cli(['validate', file])
  assert.equal(validated.code, 0, validated.out)
  const ran = cli(['run', file])
  assert.equal(ran.code, 0, ran.out)
  assert.match(ran.out, /\[OK  \] bash/)
})
