import assert from 'node:assert/strict'
import { existsSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { test } from 'node:test'
import { createMockBackend } from '../src/backend.ts'
import type { AiBackend } from '../src/backend.ts'
import { runWorkflow } from '../src/engine.ts'
import type { RunSummary } from '../src/engine.ts'
import { loadWorkflow, strictFindings } from '../src/load.ts'
import type { RunLevel } from '../src/types.ts'
import { DOT_PNG, makeProject, nodeEcho } from './helpers.ts'

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

/**
 * A manifest whose only step reads a picture.
 *
 * `cache` is written only when the test needs it, so the default (`true`, i.e.
 * replay) stays the thing being compared against.
 * @param cache - value for `ai.cache`, when the step should say it.
 * @returns the manifest text.
 */
function visionManifest(cache?: boolean): string {
  const cacheLine = cache === undefined ? '' : `      cache: ${cache}\n`
  return `name: vision
description: read a captcha
steps:
  - id: read
    why: a captcha is a picture judgment no rule can enumerate
    save: code
    ai:
      prompt: read the four characters in the picture
      images: [shot.png]
${cacheLine}      schema:
        type: object
        required: [code]
        properties:
          code:
            type: string
`
}

test('an ai step sends the pictures it names as base64', async () => {
  const dir = makeProject({ 'flow.yml': visionManifest() })
  writeFileSync(join(dir, 'shot.png'), DOT_PNG)
  const backend = createMockBackend(['{"code":"AB12"}'])
  const summary = await runWorkflow({
    loaded: loadWorkflow(join(dir, 'flow.yml')),
    inputs: {},
    backend,
    log: () => {},
  })
  assert.equal(summary.status, 'ok')
  const ask = backend.calls[0]
  assert.equal(ask?.images?.length, 1)
  assert.equal(ask?.images?.[0]?.mime, 'image/png')
  assert.equal(ask?.images?.[0]?.path, join(dir, 'shot.png'))
  assert.equal(ask?.images?.[0]?.data, DOT_PNG.toString('base64'))
})

test('a different picture is a different decision, even when the prompt repeats', async () => {
  // Replaying the recorded answer here would hand back yesterday's captcha, so
  // the image digest is part of the decision key.
  const dir = makeProject({ 'flow.yml': visionManifest() })
  writeFileSync(join(dir, 'shot.png'), DOT_PNG)
  const backend = createMockBackend(['{"code":"AB12"}', '{"code":"CD34"}'])
  const loaded = loadWorkflow(join(dir, 'flow.yml'))
  const first = await runWorkflow({ loaded, inputs: {}, backend, log: () => {} })
  assert.equal(first.status, 'ok')
  assert.equal(first.steps.read?.cached, undefined)

  const other = Buffer.from(DOT_PNG)
  other[other.length - 1] = other[other.length - 1]! ^ 0x01
  writeFileSync(join(dir, 'shot.png'), other)
  const second = await runWorkflow({ loaded, inputs: {}, backend, log: () => {} })
  assert.equal(second.status, 'ok')
  assert.equal(backend.calls.length, 2)
  assert.equal(second.steps.read?.cached, undefined)
})

test('ai.cache: false asks every time and records nothing', async () => {
  const dir = makeProject({ 'flow.yml': visionManifest(false) })
  writeFileSync(join(dir, 'shot.png'), DOT_PNG)
  const backend = createMockBackend(['{"code":"AB12"}', '{"code":"CD34"}'])
  const loaded = loadWorkflow(join(dir, 'flow.yml'))
  await runWorkflow({ loaded, inputs: {}, backend, log: () => {} })
  const second = await runWorkflow({ loaded, inputs: {}, backend, log: () => {} })
  assert.equal(second.status, 'ok')
  assert.equal(backend.calls.length, 2)
  assert.equal(second.steps.read?.cached, undefined)
  // Nothing was written for it: there is no recorded decision to replay, in this
  // run or in any later one.
  assert.equal(existsSync(join(dir, '.dsh-flow', 'answers', 'read')), false)
})

test('manual mode explains why a fresh judgment cannot run', async () => {
  const dir = makeProject({ 'flow.yml': visionManifest(false) })
  writeFileSync(join(dir, 'shot.png'), DOT_PNG)
  const summary = await runWorkflow({
    loaded: loadWorkflow(join(dir, 'flow.yml')),
    inputs: {},
    level: 'manual',
    backend: forbiddenBackend(),
    log: () => {},
  })
  assert.equal(summary.status, 'failed')
  assert.match(summary.steps.read?.error ?? '', /ai\.cache: false/)
})

test('a missing picture fails the step before the model is asked', async () => {
  const dir = makeProject({ 'flow.yml': visionManifest() })
  const backend = createMockBackend(['{"code":"AB12"}'])
  const summary = await runWorkflow({
    loaded: loadWorkflow(join(dir, 'flow.yml')),
    inputs: {},
    backend,
    log: () => {},
  })
  assert.equal(summary.status, 'failed')
  assert.match(summary.steps.read?.error ?? '', /ai\.images/)
  assert.match(summary.steps.read?.error ?? '', /shot\.png/)
  assert.equal(backend.calls.length, 0)
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
