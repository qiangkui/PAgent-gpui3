import assert from 'node:assert/strict'
import { existsSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import { test } from 'node:test'
import { runFixtures } from '../src/fixtures.ts'
import { loadWorkflow } from '../src/load.ts'
import { makeProject, nodeEcho } from './helpers.ts'

const MANIFEST = `name: fixtures-demo
description: d
steps:
  - id: counted
    run: ${nodeEcho('{files:2}')}
  - id: other
    run: ${nodeEcho('{other:true}')}
`

test('reports a step without a fixture as uncovered', async () => {
  const dir = makeProject({ 'flow.yml': MANIFEST })
  const report = await runFixtures(loadWorkflow(`${dir}/flow.yml`), { log: () => {}, update: false })
  assert.equal(report.totals.uncovered, 2)
  assert.equal(report.totals.failed, 0)
})

test('passes when the recorded output matches', async () => {
  const dir = makeProject({
    'flow.yml': MANIFEST,
    'fixtures/counted.out.json': '{\n  "files": 2\n}\n',
  })
  const report = await runFixtures(loadWorkflow(`${dir}/flow.yml`), { log: () => {}, update: false })
  assert.equal(report.totals.passed, 1)
  assert.equal(report.totals.uncovered, 1)
})

test('fails when the step output drifts from the fixture', async () => {
  const dir = makeProject({
    'flow.yml': MANIFEST,
    'fixtures/counted.out.json': '{\n  "files": 99\n}\n',
  })
  const report = await runFixtures(loadWorkflow(`${dir}/flow.yml`), { log: () => {}, update: false })
  assert.equal(report.totals.failed, 1)
  assert.match(report.outcomes[0]?.detail ?? '', /expected .*99/)
})

test('records outputs with update and feeds inputs from the input fixture', async () => {
  const dir = makeProject({
    'flow.yml': `name: fixtures-demo
description: d
inputs:
  repo:
    default: .
steps:
  - id: echoed
    run: ${nodeEcho('{repo:c.inputs.repo}')}
`,
    'fixtures/echoed.in.json': '{\n  "inputs": {\n    "repo": "from-fixture"\n  }\n}\n',
  })
  const report = await runFixtures(loadWorkflow(`${dir}/flow.yml`), { log: () => {}, update: true })
  assert.equal(report.totals.passed, 1)
  const recorded = JSON.parse(readFileSync(join(dir, 'fixtures', 'echoed.out.json'), 'utf8')) as { repo: string }
  assert.equal(recorded.repo, 'from-fixture')
})

const AI_MANIFEST = `name: fixtures-ai
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

test('an ai step is tested against its recorded reply, not a model', async () => {
  const dir = makeProject({
    'flow.yml': AI_MANIFEST,
    'fixtures/classify.ai.json': JSON.stringify({ text: '{"category":"bug"}' }),
    'fixtures/classify.out.json': JSON.stringify({ category: 'bug' }),
  })
  const report = await runFixtures(loadWorkflow(`${dir}/flow.yml`), { log: () => {}, update: false })
  assert.equal(report.totals.passed, 1)
  assert.equal(report.totals.failed, 0)
  // The fixture reply must not turn into a recorded decision a later run replays.
  assert.equal(existsSync(join(dir, '.dsh-flow', 'answers')), false)
})

test('reports an ai step without a reply fixture as uncovered', async () => {
  const dir = makeProject({
    'flow.yml': AI_MANIFEST,
    'fixtures/classify.out.json': JSON.stringify({ category: 'bug' }),
  })
  const report = await runFixtures(loadWorkflow(`${dir}/flow.yml`), { log: () => {}, update: false })
  assert.equal(report.totals.uncovered, 1)
  assert.match(report.outcomes[0]?.detail ?? '', /classify\.ai\.json/)
})

test('refuses to record an ai step that has no reply fixture', async () => {
  const dir = makeProject({ 'flow.yml': AI_MANIFEST })
  await assert.rejects(runFixtures(loadWorkflow(`${dir}/flow.yml`), { log: () => {}, update: true }), /classify\.ai\.json/)
})
