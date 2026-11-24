import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { createServer } from 'node:net'
import { test } from 'node:test'
import { resolveInputs, runWorkflow } from '../src/engine.ts'
import type { RunSummary } from '../src/engine.ts'
import { loadWorkflow } from '../src/load.ts'
import { stepEnvironment } from '../src/exec.ts'
import { makeProject, nodeEcho } from './helpers.ts'

/** A port nobody is listening on right now. */
async function freePort(): Promise<number> {
  const server = createServer()
  return await new Promise((resolve, reject) => {
    server.once('error', reject)
    server.listen(0, '127.0.0.1', () => {
      const address = server.address() as { port: number }
      server.close(() => resolve(address.port))
    })
  })
}

/** Whether a process id is still alive. */
function alive(pid: number): boolean {
  try {
    process.kill(pid, 0)
    return true
  } catch {
    return false
  }
}

/**
 * Run a manifest held in a temporary directory.
 * @param manifest - manifest text.
 * @param inputs - caller inputs.
 * @returns the summary.
 */
async function run(manifest: string, inputs: Record<string, string> = {}): Promise<RunSummary> {
  const dir = makeProject({ 'flow.yml': manifest })
  return runWorkflow({ loaded: loadWorkflow(`${dir}/flow.yml`), inputs, log: () => {} })
}

test('passes context on stdin and saves the JSON reply into vars', async () => {
  const summary = await run(`name: t
description: d
version: 1
inputs:
  who:
    default: world
steps:
  - id: greet
    run: ${nodeEcho('{text:"hi "+c.inputs.who}')}
    save: greet
  - id: echo
    set:
      line: "{{ vars.greet.text }}!"
`)
  assert.equal(summary.status, 'ok')
  assert.deepEqual(summary.steps.greet?.json, { text: 'hi world' })
  assert.equal(summary.vars.line, 'hi world!')
})

test('keeps a bare template value typed', async () => {
  const summary = await run(`name: t
description: d
steps:
  - id: n
    run: ${nodeEcho('{total:7}')}
    save: data
  - id: copy
    set:
      total: "{{ vars.data.total }}"
`)
  assert.equal(summary.vars.total, 7)
})

test('skips a step whose condition is false', async () => {
  const summary = await run(`name: t
description: d
steps:
  - id: skipped
    when: "1 == 2"
    run: [node, -e, 'process.exit(9)']
  - id: ran
    set:
      ok: true
`)
  assert.equal(summary.steps.skipped?.status, 'skipped')
  assert.equal(summary.status, 'ok')
  assert.equal(summary.steps.ran?.status, 'ok')
})

test('starts a service, waits for readiness, and stops it', async () => {
  const port = await freePort()
  const logs: string[] = []
  const dir = makeProject({
    'flow.yml': `name: services-demo
description: d
inputs:
  port:
    required: true
services:
  api:
    run:
      command: [node, server.mjs]
      env:
        PORT: "{{ inputs.port }}"
    ready:
      url: "http://127.0.0.1:{{ inputs.port }}/health"
      timeoutMs: 8000
      intervalMs: 100
steps:
  - id: probe
    run: [node, steps/probe.mjs, "{{ inputs.port }}"]
    save: reply
`,
    'server.mjs': `import { createServer } from 'node:http'
import { writeFileSync } from 'node:fs'
writeFileSync('service.pid', String(process.pid))
console.log('listening')
createServer((request, response) => {
  response.end(request.url === '/health' ? 'ok' : 'up')
}).listen(Number(process.env.PORT), '127.0.0.1')
`,
    'steps/probe.mjs': `const reply = await fetch('http://127.0.0.1:' + process.argv[2] + '/')
console.log(JSON.stringify({ text: await reply.text() }))
`,
  })
  const summary = await runWorkflow({
    loaded: loadWorkflow(`${dir}/flow.yml`),
    inputs: { port: String(port) },
    log: (line) => logs.push(line),
  })
  assert.equal(summary.status, 'ok', `${summary.error}\n${logs.join('\n')}`)
  assert.deepEqual(summary.steps.probe?.json, { text: 'up' })
  assert.ok(logs.includes('service api ready'), logs.join('\n'))
  assert.ok(logs.some((line) => line.includes('[api] listening')), logs.join('\n'))
  const pid = Number(readFileSync(`${dir}/service.pid`, 'utf8'))
  assert.ok(!alive(pid), 'the service is stopped when the run ends')
})

test('stops the service when a step fails', async () => {
  const port = await freePort()
  const dir = makeProject({
    'flow.yml': `name: services-fail
description: d
inputs:
  port:
    required: true
services:
  api:
    run:
      command: [node, server.mjs]
      env:
        PORT: "{{ inputs.port }}"
    ready: "http://127.0.0.1:{{ inputs.port }}/health"
steps:
  - id: bad
    run: [node, -e, 'process.exit(3)']
`,
    'server.mjs': `import { createServer } from 'node:http'
import { writeFileSync } from 'node:fs'
writeFileSync('service.pid', String(process.pid))
createServer((request, response) => {
  response.end(request.url === '/health' ? 'ok' : 'up')
}).listen(Number(process.env.PORT), '127.0.0.1')
`,
  })
  const summary = await runWorkflow({
    loaded: loadWorkflow(`${dir}/flow.yml`),
    inputs: { port: String(port) },
    log: () => {},
  })
  assert.equal(summary.status, 'failed')
  assert.equal(summary.failedStep, 'bad')
  const pid = Number(readFileSync(`${dir}/service.pid`, 'utf8'))
  assert.ok(!alive(pid), 'the service is stopped after a failure')
})

test('fails before the steps when a service never gets ready', async () => {
  const dir = makeProject({
    'flow.yml': `name: services-broken
description: d
services:
  broken:
    run: [node, -e, 'process.exit(2)']
    ready:
      url: http://127.0.0.1:9/health
      timeoutMs: 1200
      intervalMs: 100
steps:
  - id: never
    run: [node, -e, 'process.exit(0)']
`,
  })
  const summary = await runWorkflow({
    loaded: loadWorkflow(`${dir}/flow.yml`),
    inputs: {},
    log: () => {},
  })
  assert.equal(summary.status, 'failed')
  assert.match(String(summary.error), /service "broken"/)
  assert.equal(summary.steps.never, undefined)
})

test('runs a once step until its first success', async () => {
  const dir = makeProject({
    'flow.yml': `name: once-demo
description: d
steps:
  - id: mark
    once: true
    run: [node, steps/mark.mjs]
`,
    'steps/mark.mjs': "import { appendFileSync } from 'node:fs'\nappendFileSync('count.txt', 'x')\n",
  })
  const loaded = loadWorkflow(`${dir}/flow.yml`)
  const base = {
    loaded,
    inputs: {},
    log: () => {},
    onceLedger: `${dir}/.dsh-flow/ledger.json`,
  }
  const first = await runWorkflow(base)
  const second = await runWorkflow({ ...base })
  const forced = await runWorkflow({ ...base, forceOnce: true })
  assert.equal(first.steps.mark?.status, 'ok')
  assert.equal(second.steps.mark?.status, 'skipped')
  assert.match(String(second.steps.mark?.error), /once/)
  assert.equal(forced.steps.mark?.status, 'ok')
  assert.equal(readFileSync(`${dir}/count.txt`, 'utf8'), 'xx')
})

test('does not record a failed once step', async () => {
  const dir = makeProject({
    'flow.yml': `name: once-fail
description: d
steps:
  - id: bad
    once: true
    allowFailure: true
    run: [node, -e, 'process.exit(3)']
`,
  })
  const loaded = loadWorkflow(`${dir}/flow.yml`)
  const base = {
    loaded,
    inputs: {},
    log: () => {},
    onceLedger: `${dir}/.dsh-flow/ledger.json`,
  }
  const first = await runWorkflow(base)
  const second = await runWorkflow({ ...base })
  assert.equal(first.steps.bad?.status, 'failed')
  assert.equal(second.steps.bad?.status, 'failed', 'a failure is retried next run')
})

test('stops the run at a failing step unless allowFailure is set', async () => {
  const failing = await run(`name: t
description: d
steps:
  - id: bad
    run: [node, -e, 'process.exit(3)']
  - id: never
    set:
      ok: true
`)
  assert.equal(failing.status, 'failed')
  assert.equal(failing.failedStep, 'bad')
  assert.equal(failing.steps.never, undefined)

  const tolerated = await run(`name: t
description: d
steps:
  - id: bad
    allowFailure: true
    run: [node, -e, 'process.exit(3)']
  - id: later
    set:
      ok: true
`)
  assert.equal(tolerated.status, 'ok')
  assert.equal(tolerated.steps.bad?.status, 'failed')
  assert.equal(tolerated.steps.later?.status, 'ok')
})

test('reports a missing runtime before spawning', async () => {
  const summary = await run(`name: t
description: d
steps:
  - id: a
    run: [definitely-not-a-runtime-xyz, --version]
`)
  assert.equal(summary.status, 'failed')
  assert.match(summary.steps.a?.error ?? '', /not found/)
})

test('fans out over a list and records per-item results', async () => {
  const summary = await run(`name: t
description: d
vars:
  items: [1, 2, 3]
steps:
  - id: each
    map:
      over: vars.items
      as: n
      concurrency: 2
      steps:
        - id: keep
          set:
            seen: "{{ n }}"
`)
  const items = summary.steps.each?.items ?? []
  assert.equal(items.length, 3)
  assert.deepEqual(items.map(entry => entry.steps.keep?.json), [{ seen: 1 }, { seen: 2 }, { seen: 3 }])
})

test('marks a fan-out failed when one iteration fails', async () => {
  const summary = await run(`name: t
description: d
vars:
  items: [0, 1]
steps:
  - id: each
    allowFailure: true
    map:
      over: vars.items
      steps:
        - id: gate
          when: "item == 1"
          run: [node, -e, 'process.exit(1)']
`)
  assert.equal(summary.steps.each?.status, 'failed')
})

test('decodes UTF-8 replies without mangling non-ASCII text', async () => {
  const summary = await run(`name: t
description: d
steps:
  - id: zh
    run: ${nodeEcho('{text:"中文 标题"}')}
`)
  assert.deepEqual(summary.steps.zh?.json, { text: '中文 标题' })
})

test('runs a string command through the shell', async () => {
  const summary = await run(`name: t
description: d
steps:
  - id: shell
    run: node -e console.log(7)
`)
  assert.equal(summary.status, 'ok', summary.steps.shell?.error)
  assert.equal(summary.steps.shell?.stdout?.trim(), '7')
})

test('resolves declared inputs and rejects unknown ones', () => {
  const dir = makeProject({
    'flow.yml': 'name: t\ndescription: d\ninputs:\n  need:\n    required: true\nsteps:\n  - id: a\n    set:\n      x: 1\n',
  })
  const loaded = loadWorkflow(`${dir}/flow.yml`)
  assert.deepEqual(resolveInputs(loaded, { need: 'v' }), { need: 'v' })
  assert.throws(() => resolveInputs(loaded, {}), /missing required input "need"/)
  assert.throws(() => resolveInputs(loaded, { other: 'x' }), /unknown input "other"/)
})

test('forces UTF-8 for step programs', () => {
  const env = stepEnvironment({ EXTRA: '1' })
  assert.equal(env.PYTHONIOENCODING, 'utf-8')
  assert.equal(env.PYTHONUTF8, '1')
  assert.equal(env.EXTRA, '1')
})
