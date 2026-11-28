// What a step says about itself: the one-line result, the files it produced, and
// the failure it reported — plus the rule that a secret input never comes back
// out through a log line, a trace record or the summary.
import assert from 'node:assert/strict'
import { writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { test } from 'node:test'
import { runWorkflow } from '../src/engine.ts'
import { loadWorkflow } from '../src/load.ts'
import { DOT_PNG, makeProject } from './helpers.ts'

/**
 * A manifest with one step that prints a fixed JSON document.
 * @param body - the JSON text the step writes.
 * @returns the manifest text.
 */
function printing(body: string): string {
  return `name: report
description: report a result
steps:
  - id: work
    run: [node, -e, 'process.stdout.write(${JSON.stringify(body)})']
`
}

test('a step reports its summary and its artifacts', async () => {
  const dir = makeProject({ 'flow.yml': printing('{"summary":"共 51 条","artifacts":[{"path":"shot.png"}]}') })
  writeFileSync(join(dir, 'shot.png'), DOT_PNG)
  const summary = await runWorkflow({
    loaded: loadWorkflow(join(dir, 'flow.yml')),
    inputs: {},
    log: () => {},
  })
  const step = summary.steps.work
  assert.equal(step?.summary, '共 51 条')
  assert.equal(step?.artifacts?.length, 1)
  assert.equal(step?.artifacts?.[0]?.kind, 'image')
  assert.equal(step?.artifacts?.[0]?.mime, 'image/png')
  assert.equal(step?.artifacts?.[0]?.bytes, DOT_PNG.length)
  assert.equal(step?.artifacts?.[0]?.path, join(dir, 'shot.png'))
})

test('an artifact that is not there is left out instead of breaking the run', async () => {
  const dir = makeProject({ 'flow.yml': printing('{"artifacts":[{"path":"missing.png"}]}') })
  const summary = await runWorkflow({
    loaded: loadWorkflow(join(dir, 'flow.yml')),
    inputs: {},
    log: () => {},
  })
  assert.equal(summary.status, 'ok')
  assert.equal(summary.steps.work?.artifacts, undefined)
})

test('a structured failure keeps its kind, hint and comparison', async () => {
  const dir = makeProject({
    'flow.yml': `name: report
description: fail with detail
steps:
  - id: check
    run: [node, -e, 'process.stdout.write(JSON.stringify({error:{kind:"regex-miss",message:"分页条里没有 «共 N 条记录»",hint:"先点查询",where:"maers_site.py:query",expected:"共 51 条",gotten:"每页：10…"}}));process.exit(1)']
`,
  })
  const summary = await runWorkflow({
    loaded: loadWorkflow(join(dir, 'flow.yml')),
    inputs: {},
    log: () => {},
  })
  assert.equal(summary.status, 'failed')
  const info = summary.steps.check?.errorInfo
  assert.equal(info?.kind, 'regex-miss')
  assert.equal(info?.hint, '先点查询')
  assert.equal(info?.where, 'maers_site.py:query')
  assert.equal(info?.expected, '共 51 条')
  assert.equal(info?.gotten, '每页：10…')
})

test('an unknown failure kind falls back to engine instead of inventing one', async () => {
  const dir = makeProject({
    'flow.yml': printing('{"error":{"kind":"cosmic-rays","message":"something odd"}}'),
  })
  const summary = await runWorkflow({
    loaded: loadWorkflow(join(dir, 'flow.yml')),
    inputs: {},
    log: () => {},
  })
  assert.equal(summary.steps.work?.errorInfo?.kind, 'engine')
})

const SECRET_MANIFEST = `name: secret
description: a password the cards never show
inputs:
  password:
    description: 站点密码
    required: true
    secret: true
steps:
  - id: greet
    save: seen
    run:
      command: [node, -e, 'process.stdout.write(JSON.stringify({summary:"used",echo:process.env.ECHO}))']
      env:
        ECHO: "{{ inputs.password }}"
`

test('a secret input never reaches the summary, the trace or the log', async () => {
  const dir = makeProject({ 'flow.yml': SECRET_MANIFEST })
  const log: string[] = []
  const trace: string[] = []
  const summary = await runWorkflow({
    loaded: loadWorkflow(join(dir, 'flow.yml')),
    inputs: {},
    secrets: { password: 'swordfish' },
    log: (line) => log.push(line),
    trace: (record) => trace.push(JSON.stringify(record)),
  })
  assert.equal(summary.status, 'ok')
  const shown = JSON.stringify([summary, log, trace])
  assert.equal(shown.includes('swordfish'), false, shown)
  // The step really did see the value: it is redacted on the way out, not lost.
  assert.equal(JSON.stringify(summary.steps.greet?.json).includes('***'), true)
})

test('a secret can come from the environment when no map is given', async () => {
  const dir = makeProject({ 'flow.yml': SECRET_MANIFEST })
  const before = process.env.DSH_SECRET_PASSWORD
  process.env.DSH_SECRET_PASSWORD = 'envfish'
  try {
    const summary = await runWorkflow({
      loaded: loadWorkflow(join(dir, 'flow.yml')),
      inputs: {},
      log: () => {},
    })
    assert.equal(summary.status, 'ok')
  } finally {
    if (before === undefined) delete process.env.DSH_SECRET_PASSWORD
    else process.env.DSH_SECRET_PASSWORD = before
  }
})

test('a missing secret says where to put it', async () => {
  const dir = makeProject({ 'flow.yml': SECRET_MANIFEST })
  await assert.rejects(
    runWorkflow({ loaded: loadWorkflow(join(dir, 'flow.yml')), inputs: {}, log: () => {} }),
    /DSH_SECRET_PASSWORD/,
  )
})

test('a secret input refuses a value on the command line', async () => {
  const dir = makeProject({ 'flow.yml': SECRET_MANIFEST })
  await assert.rejects(
    runWorkflow({
      loaded: loadWorkflow(join(dir, 'flow.yml')),
      inputs: { password: 'typed-on-argv' },
      log: () => {},
    }),
    /是 secret/,
  )
})

test('a manifest cannot bake a default into a secret input', () => {
  const dir = makeProject({
    'flow.yml': `name: leaky
description: d
inputs:
  password:
    secret: true
    default: hunter2
steps:
  - id: a
    set:
      x: y
`,
  })
  assert.throws(() => loadWorkflow(join(dir, 'flow.yml')), /secret: true/)
})