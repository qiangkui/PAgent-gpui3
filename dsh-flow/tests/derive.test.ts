import assert from 'node:assert/strict'
import { join } from 'node:path'
import { test } from 'node:test'
import { deriveWorkflow } from '../src/derive.ts'
import { runWorkflow } from '../src/engine.ts'
import { loadWorkflow } from '../src/load.ts'
import { makeProject } from './helpers.ts'

/**
 * Build a session log.
 * @param events - session events, written in the wrapped form.
 * @param header - header overrides.
 * @returns JSONL text.
 */
function session(events: unknown[], header: Record<string, unknown> = {}): string {
  const lines = [JSON.stringify({ type: 'session', version: 0, id: 's-1', cwd: 'E:/work', ...header })]
  for (const event of events) lines.push(JSON.stringify({ type: 'session_event', sessionId: 's-1', event }))
  return `${lines.join('\n')}\n`
}

/**
 * An assistant message with one text block.
 * @param seq - event sequence number.
 * @param text - the prose.
 * @returns the event.
 */
function say(seq: number, text: string): unknown {
  return {
    type: 'assistant/message',
    seq,
    time: 0,
    data: { message: { role: 'assistant', content: [{ type: 'text', text }] } },
  }
}

/**
 * A tool call.
 * @param seq - event sequence number.
 * @param name - tool name.
 * @param args - arguments, serialized as the provider sends them.
 * @returns the event.
 */
function call(seq: number, name: string, args: Record<string, unknown>): unknown {
  return {
    type: 'tool/call',
    seq,
    time: 0,
    data: { callId: `c${seq}`, name, arguments: JSON.stringify(args) },
  }
}

/**
 * Write a derived manifest and load it back.
 * @param manifest - the YAML.
 * @returns the loaded workflow plus its directory.
 */
function load(manifest: string): { dir: string, loaded: ReturnType<typeof loadWorkflow> } {
  const dir = makeProject({ 'flow.yml': manifest })
  return { dir, loaded: loadWorkflow(join(dir, 'flow.yml')) }
}

test('turns commands into run steps in order and replays them', async () => {
  const report = deriveWorkflow(session([
    { type: 'turn/start', seq: 1, data: { turn: 1 } },
    say(2, '先打印一行'),
    call(3, 'bash', { command: [process.execPath, '-e', 'console.log("one")'] }),
    call(4, 'shell', { command: 'echo two' }),
    say(5, '看起来没问题'),
  ]), { name: 'derived-test' })

  assert.equal(report.summary.runs, 2)
  assert.equal(report.summary.todos.length, 1)
  assert.match(report.manifest, /name: "derived-test"/)
  assert.match(report.manifest, /- id: bash/)
  assert.match(report.manifest, /desc: "先打印一行"/)

  const { dir, loaded } = load(report.manifest)
  assert.equal(loaded.workflow.steps.length, 2)
  assert.deepEqual(loaded.workflow.steps[0]?.run?.command, [process.execPath, '-e', 'console.log("one")'])
  assert.equal(loaded.workflow.steps[1]?.run?.command, 'echo two')

  const summary = await runWorkflow({ loaded, inputs: {}, log: () => {} })
  assert.equal(summary.status, 'ok', summary.error)
  assert.equal(summary.steps.bash?.stdout?.trim(), 'one')
  assert.equal(summary.steps.shell?.exitCode, 0)
  assert.ok(dir.length > 0)
})

test('keeps a multi-line shell command as one block scalar', () => {
  const report = deriveWorkflow(session([
    call(2, 'bash', { command: 'echo one\necho two' }),
  ]))
  assert.match(report.manifest, /run: \|/)
  const { loaded } = load(report.manifest)
  const command = loaded.workflow.steps[0]?.run?.command
  assert.equal(typeof command, 'string')
  assert.ok((command as string).startsWith('echo one\necho two'), command as string)
})

test('leaves judgment points and other tools as TODO comments', () => {
  const report = deriveWorkflow(session([
    say(1, '先读文件再决定'),
    call(2, 'read', { path: 'a.txt' }),
    say(3, '这个仓库没有问题，可以收工'),
  ]))
  assert.equal(report.summary.runs, 0)
  assert.equal(report.summary.todos.length, 2)
  assert.match(report.summary.todos[0] ?? '', /read：path=a\.txt（原会话意图：先读文件再决定）/)
  assert.match(report.summary.todos[1] ?? '', /判断\/结论：这个仓库没有问题/)

  const { loaded } = load(report.manifest)
  assert.equal(loaded.workflow.steps.length, 1)
  assert.deepEqual(loaded.workflow.steps[0]?.set, { todo: 'TODO — 用真实步骤替换' })
  assert.match(report.manifest, /# TODO（派生时刻意没有推断的部分）：/)
})

test('derives only the first turn and reports the rest', () => {
  const report = deriveWorkflow(session([
    { type: 'turn/start', seq: 1, data: { turn: 1 } },
    call(2, 'bash', { command: 'echo one' }),
    { type: 'turn/start', seq: 3, data: { turn: 2 } },
    call(4, 'bash', { command: 'echo two' }),
    { type: 'turn/start', seq: 5, data: { turn: 3 } },
  ]))
  assert.equal(report.summary.runs, 1)
  assert.equal(report.summary.ignoredTurns, 2)
  assert.ok(!report.manifest.includes('echo two'), report.manifest)
  assert.match(report.manifest, /后 2 轮的提问没有派生/)
})

test('counts unreadable lines without failing', () => {
  const source = `${session([call(2, 'bash', { command: 'echo one' })])}{"broken": \nnot json\n`
  const report = deriveWorkflow(source)
  assert.equal(report.summary.skippedLines, 2)
  assert.equal(report.summary.runs, 1)
  assert.match(report.manifest, /跳过了 2 行/)
})

test('falls back to the session id for the name', () => {
  const report = deriveWorkflow(session([], { id: 'Repo Health Session' }))
  assert.match(report.manifest, /name: "repo-health-session"/)
  assert.equal(report.summary.runs, 0)
})
