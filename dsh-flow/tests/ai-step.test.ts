import assert from 'node:assert/strict'
import { join } from 'node:path'
import { test } from 'node:test'
import { createMockBackend } from '../src/backend.ts'
import type { AiBackend } from '../src/backend.ts'
import { runWorkflow } from '../src/engine.ts'
import type { RunSummary } from '../src/engine.ts'
import { loadWorkflow, strictFindings } from '../src/load.ts'
import type { RunLevel } from '../src/types.ts'
import { makeProject, nodeEcho } from './helpers.ts'

const AI_MANIFEST = `name: triage
description: classify one report
inputs:
  text:
    default: hello
steps:
  - id: classify
    why: 自由文本的归类无法用规则穷举
    save: verdict
    ai:
      prompt: "分类：{{ inputs.text }}"
      system: 你是一个分类器
      temperature: 0
      schema:
        type: object
        required: [category]
        properties:
          category:
            type: string
            enum: [bug, question, other]
`

const GOOD = '{"category":"bug"}'

/**
 * Run a manifest held in a temporary directory.
 * @param manifest - manifest text.
 * @param options - level, backend, inputs and dry-run flag.
 * @returns the summary plus the manifest directory.
 */
async function runManifest(
  manifest: string,
  options: { level?: RunLevel, backend?: AiBackend, inputs?: Record<string, string>, dryRun?: boolean } = {},
): Promise<{ summary: RunSummary, dir: string }> {
  const dir = makeProject({ 'flow.yml': manifest })
  const summary = await runWorkflow({
    loaded: loadWorkflow(join(dir, 'flow.yml')),
    inputs: options.inputs ?? {},
    level: options.level,
    backend: options.backend,
    dryRun: options.dryRun,
    log: () => {},
  })
  return { summary, dir }
}

/** A backend that fails the test if anything asks it a question. */
function forbiddenBackend(): AiBackend {
  return {
    name: 'forbidden',
    async ask() {
      throw new Error('this run must not call a model')
    },
  }
}

test('records a schema-valid answer under steps.<id>.json and vars.<save>', async () => {
  const backend = createMockBackend([GOOD])
  const { summary } = await runManifest(AI_MANIFEST, { backend })
  assert.equal(summary.status, 'ok')
  assert.equal(summary.steps.classify?.kind, 'ai')
  assert.deepEqual(summary.steps.classify?.json, { category: 'bug' })
  assert.equal(summary.steps.classify?.text, GOOD)
  assert.equal(summary.steps.classify?.model, 'mock')
  assert.deepEqual(summary.vars.verdict, { category: 'bug' })
  assert.equal(backend.calls.length, 1)
  assert.equal(backend.calls[0]?.system, '你是一个分类器')
  assert.equal(backend.calls[0]?.temperature, 0)
  assert.match(backend.calls[0]?.prompt ?? '', /分类：hello/)
  assert.match(backend.calls[0]?.prompt ?? '', /JSON Schema/)
})

test('rejects an answer that breaks the schema and repairs it once', async () => {
  const backend = createMockBackend(['{"category":"nonsense"}', '{"category":"question"}'])
  const { summary } = await runManifest(AI_MANIFEST, { level: 'draft', backend })
  assert.equal(summary.status, 'ok')
  assert.deepEqual(summary.steps.classify?.json, { category: 'question' })
  assert.equal(backend.calls.length, 2)
  assert.match(backend.calls[1]?.prompt ?? '', /不符合要求/)
  assert.match(backend.calls[1]?.prompt ?? '', /期望取值/)
})

test('fails with a readable message when the repair still breaks the schema', async () => {
  const backend = createMockBackend(['不是 JSON', '{"category":"nonsense"}'])
  const { summary } = await runManifest(AI_MANIFEST, { level: 'draft', backend })
  assert.equal(summary.status, 'failed')
  assert.equal(summary.failedStep, 'classify')
  assert.match(summary.steps.classify?.error ?? '', /仍不符合约定/)
  assert.match(summary.steps.classify?.error ?? '', /category/)
})

test('manual level never calls a model and explains what is missing', async () => {
  const { summary } = await runManifest(AI_MANIFEST, { level: 'manual', backend: forbiddenBackend() })
  assert.equal(summary.status, 'failed')
  assert.equal(summary.steps.classify?.status, 'failed')
  assert.match(summary.steps.classify?.error ?? '', /manual 档不调用模型/)
  assert.match(summary.steps.classify?.error ?? '', /--level assist/)
})

test('a recorded decision replays at zero calls in manual and assist', async () => {
  const dir = makeProject({ 'flow.yml': AI_MANIFEST })
  const loaded = loadWorkflow(join(dir, 'flow.yml'))
  const drafted = await runWorkflow({ loaded, inputs: {}, level: 'draft', backend: createMockBackend([GOOD]), log: () => {} })
  assert.equal(drafted.status, 'ok')

  for (const level of ['manual', 'assist'] as const) {
    const replayed = await runWorkflow({ loaded, inputs: {}, level, backend: forbiddenBackend(), log: () => {} })
    assert.equal(replayed.status, 'ok', level)
    assert.deepEqual(replayed.steps.classify?.json, { category: 'bug' })
    assert.equal(replayed.steps.classify?.cached, true)
  }
})

test('changing the prompt invalidates the recorded decision', async () => {
  const dir = makeProject({ 'flow.yml': AI_MANIFEST })
  const loaded = loadWorkflow(join(dir, 'flow.yml'))
  await runWorkflow({ loaded, inputs: {}, level: 'draft', backend: createMockBackend([GOOD]), log: () => {} })

  const staleManual = await runWorkflow({ loaded, inputs: { text: 'changed' }, level: 'manual', backend: forbiddenBackend(), log: () => {} })
  assert.equal(staleManual.status, 'failed')
  assert.match(staleManual.steps.classify?.error ?? '', /manual 档不调用模型/)

  const backend = createMockBackend(['{"category":"question"}'])
  const assisted = await runWorkflow({ loaded, inputs: { text: 'changed' }, level: 'assist', backend, log: () => {} })
  assert.equal(assisted.status, 'ok')
  assert.equal(backend.calls.length, 1)
  assert.deepEqual(assisted.steps.classify?.json, { category: 'question' })
})

test('an ai step without a schema keeps text and parses JSON when it can', async () => {
  const manifest = `name: t
description: d
steps:
  - id: prose
    ai:
      prompt: say hi
`
  const { summary } = await runManifest(manifest, { backend: createMockBackend(['hello, 世界']) })
  assert.equal(summary.status, 'ok')
  assert.equal(summary.steps.prose?.text, 'hello, 世界')
  assert.equal(summary.steps.prose?.json, undefined)

  const withJson = await runManifest(manifest, { backend: createMockBackend(['{"ok":true}']) })
  assert.deepEqual(withJson.summary.steps.prose?.json, { ok: true })
})

test('dry run renders an ai step without asking anything', async () => {
  const { summary } = await runManifest(AI_MANIFEST, { dryRun: true, backend: forbiddenBackend() })
  assert.equal(summary.status, 'ok')
  assert.equal(summary.steps.classify?.status, 'skipped')
  assert.equal(summary.steps.classify?.error, 'dry run')
})

test('runs an ai step through an external command', async () => {
  const manifest = `name: t
description: d
steps:
  - id: classify
    why: 外部命令里只有一个模型
    ai:
      backend: cli
      command: ${nodeEcho('{text: JSON.stringify({category:"bug"})}')}
      prompt: classify this
      schema:
        type: object
        required: [category]
        properties:
          category:
            type: string
`
  const { summary } = await runManifest(manifest, { level: 'draft' })
  assert.equal(summary.status, 'ok')
  assert.equal(summary.steps.classify?.model, 'cli')
  assert.deepEqual(summary.steps.classify?.json, { category: 'bug' })
})

test('strict mode flags ai steps without why or schema', async () => {
  const dir = makeProject({
    'flow.yml': `name: t
description: d
steps:
  - id: loose
    ai:
      prompt: hi
`,
  })
  const loaded = loadWorkflow(join(dir, 'flow.yml'))
  const findings = strictFindings(loaded.workflow)
  assert.equal(findings.length, 2)
  assert.match(findings.join('\n'), /需要 "why"/)
  assert.match(findings.join('\n'), /需要 "schema"/)
})

test('rejects malformed ai specs at load time', () => {
  const dir = makeProject({
    'flow.yml': `name: t
description: d
steps:
  - id: bad
    ai:
      prompt: hi
      schema:
        type: object
        properites: {}
`,
  })
  assert.throws(() => loadWorkflow(join(dir, 'flow.yml')), /不支持的 schema 关键字 "properites"/)

  const cli = makeProject({
    'flow.yml': `name: t
description: d
steps:
  - id: bad
    ai:
      prompt: hi
      backend: cli
`,
  })
  assert.throws(() => loadWorkflow(join(cli, 'flow.yml')), /needs "ai.command"/)

  const overlap = makeProject({
    'flow.yml': `name: t
description: d
steps:
  - id: bad
    run: [node, -e, '0']
    ai:
      prompt: hi
`,
  })
  assert.throws(() => loadWorkflow(join(overlap, 'flow.yml')), /may only use one of/)
})
