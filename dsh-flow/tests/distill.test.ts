import assert from 'node:assert/strict'
import { join } from 'node:path'
import { test } from 'node:test'
import { distillChat } from '../src/distill.ts'
import { runWorkflow } from '../src/engine.ts'
import { loadWorkflow } from '../src/load.ts'
import { makeProject } from './helpers.ts'

/**
 * Build a saved transcript.
 * @param messages - the `messages` list.
 * @param extra - header overrides.
 * @returns the `chat.json` text.
 */
function chat(messages: unknown[], extra: Record<string, unknown> = {}): string {
  return JSON.stringify({ title: '导出使用单位', dir: 'chat-9', ...extra, messages }, null, 2)
}

/**
 * An assistant message.
 * @param text - the prose.
 * @returns the row.
 */
function say(text: string): unknown {
  return { role: 'assistant', text }
}

/**
 * One tool row, in the shape the window writes.
 * @param name - tool name.
 * @param detail - what the display line showed after ` · `.
 * @param args - arguments, stored as JSON text.
 * @param extra - field overrides (`failed`, `imagePath`, `files`, …).
 * @returns the row.
 */
function tool(
  name: string,
  detail: string,
  args: Record<string, unknown>,
  extra: Record<string, unknown> = {},
): unknown {
  return {
    role: 'tool',
    text: `${name} · ${detail}`,
    input: JSON.stringify(args),
    output: 'ok',
    failed: false,
    elapsedMs: 120,
    ...extra,
  }
}

/** A shell command that works on every platform a test runs on. */
const COMMAND = 'node -e "console.log(1)"'

/**
 * Write a draft and load it back.
 * @param draft - the manifest text.
 * @returns the manifest's directory and the loaded workflow.
 */
function loadDraft(draft: string): { dir: string, loaded: ReturnType<typeof loadWorkflow> } {
  const dir = makeProject({ 'flow.draft.yml': draft })
  return { dir, loaded: loadWorkflow(join(dir, 'flow.draft.yml')) }
}

test('keeps real actions, merges a retry and drops exploration and probing', async () => {
  const report = distillChat(chat([
    { role: 'user', text: '把使用单位导出跑一遍' },
    { role: 'thinking', text: '先看看目录里有什么' },
    say('先找脚本'),
    tool('list', '. — 17 项', { path: '.' }),
    tool('glob', 'scripts/*.py — 1 个文件', { pattern: 'scripts/*.py' }),
    tool('read', 'scripts/parse_units.py — 36 行', { path: 'scripts/parse_units.py' }),
    say('跑解析'),
    tool('shell', '解析脚本', { command: COMMAND }, { failed: true, output: '第一次失败' }),
    tool('shell', '解析脚本', { command: COMMAND }),
    say('打开登录页'),
    tool('browser', 'open https://maers.adrs.org.cn/console/login.ftl', { action: 'open', url: 'https://maers.adrs.org.cn/console/login.ftl' }),
    tool('browser', 'eval — {"ready":true}', { action: 'eval', expression: 'document.title' }),
    tool(
      'browser',
      'screenshot — 100×40',
      { action: 'screenshot', selector: '#yzmImg' },
      { imagePath: 'E:/work/.dsh-flow/shots/captcha.png' },
    ),
    say('交付文件'),
    tool('show', '使用单位导出.xls', { paths: ['out/使用单位导出.xls'] }, { files: ['out/使用单位导出.xls'] }),
    say('导出完成'),
  ]))

  assert.equal(report.summary.tools, 9)
  assert.equal(report.summary.steps, 4)
  assert.equal(report.summary.commands, 1)
  assert.equal(report.summary.placeholders, 3)
  assert.equal(report.summary.merged, 1)
  assert.equal(report.summary.dropped, 4)
  assert.equal(report.summary.explores, 3)
  assert.equal(report.summary.probes, 1)
  assert.equal(report.summary.judgments, 2)
  assert.equal(report.summary.repeated, 0)

  const verdicts = report.actions.map(action => action.verdict)
  assert.deepEqual(verdicts, ['dropped', 'dropped', 'dropped', 'step', 'merged', 'step', 'dropped', 'step', 'step'])
  assert.equal(report.actions[4]?.stepId, 'shell')
  assert.match(report.report, /explore：`list` ×1/)
  assert.match(report.report, /`shell` ×2（失败 1 次）→ 1 个 run 步骤 \+ `retries: 1`/)

  const { loaded } = loadDraft(report.draft)
  assert.deepEqual(loaded.workflow.steps.map(step => step.id), ['shell', 'browser-open', 'browser-screenshot', 'show'])
  assert.equal(loaded.workflow.steps[0]?.retries, 1)
  assert.equal(loaded.workflow.steps[1]?.skip, true)
  assert.equal(loaded.workflow.steps[3]?.skip, true)
  assert.match(report.draft, /todo: "show out\/使用单位导出\.xls"/)

  const summary = await runWorkflow({ loaded, inputs: {}, log: () => {} })
  assert.equal(summary.status, 'ok', summary.error)
  assert.equal(summary.steps.shell?.exitCode, 0)
  assert.equal(summary.steps['browser-open']?.status, 'skipped')
})

test('an explicit name is kept as written, Chinese included', () => {
  const report = distillChat(chat([
    tool('shell', '命令', { command: 'echo one' }),
  ]), { name: '使用单位导出（草稿）' })
  assert.match(report.draft, /name: "使用单位导出（草稿）"/)
})

test('distills the last turn by default and an earlier one on request', () => {
  const source = chat([
    { role: 'user', text: '第一轮' },
    tool('shell', 'echo one', { command: 'echo one' }),
    say('第一轮结束'),
    { role: 'user', text: '第二轮' },
    tool('shell', 'echo two', { command: 'echo two' }),
  ])

  const last = distillChat(source)
  assert.equal(last.summary.turn, 2)
  assert.equal(last.summary.turns, 2)
  assert.equal(last.summary.tools, 1)
  assert.match(last.draft, /echo two/)
  assert.ok(!last.draft.includes('echo one'), last.draft)

  const first = distillChat(source, { turn: 1 })
  assert.equal(first.summary.turn, 1)
  assert.match(first.draft, /echo one/)
  assert.ok(!first.draft.includes('echo two'), first.draft)
})

test('a turn with no actions still yields a validate-able placeholder', () => {
  const report = distillChat(chat([
    { role: 'user', text: '这次只聊天' },
    say('没有需要执行的动作'),
  ]))
  assert.equal(report.summary.steps, 0)
  assert.equal(report.summary.judgments, 1)
  assert.match(report.draft, /- id: todo/)
  assert.match(report.report, /（这一轮没有工具调用）/)

  const { loaded } = loadDraft(report.draft)
  assert.equal(loaded.workflow.steps.length, 1)
  assert.equal(loaded.workflow.steps[0]?.id, 'todo')
})

test('keeps a multi-line command as one block scalar', () => {
  const report = distillChat(chat([
    tool('shell', '两行命令', { command: 'echo one\necho two' }),
  ]))
  assert.match(report.draft, /run: \|/)
  const { loaded } = loadDraft(report.draft)
  const command = loaded.workflow.steps[0]?.run?.command
  assert.equal(typeof command, 'string')
  assert.ok((command as string).startsWith('echo one\necho two'), command as string)
})

test('reports a repeat separated by another step instead of merging it', () => {
  const report = distillChat(chat([
    tool('shell', '第一次', { command: 'echo one' }),
    tool('edit', '改脚本', { path: 'scripts/a.py' }),
    tool('shell', '第二次', { command: 'echo one' }),
  ]))
  assert.equal(report.summary.steps, 3)
  assert.equal(report.summary.merged, 0)
  assert.equal(report.summary.repeated, 1)
  assert.match(report.report, /同名同参但不相邻/)
  assert.match(report.draft, /同名同参被其它步骤隔开/)
})

test('does not run a command whose args were truncated', () => {
  const long = `node -e "${'x'.repeat(8100)}"`
  const report = distillChat(chat([
    { role: 'tool', text: 'shell · 长命令', input: long, output: '…', failed: false },
  ]))
  assert.equal(report.summary.steps, 1)
  assert.equal(report.summary.commands, 0)
  assert.equal(report.summary.truncated, 1)
  assert.equal(report.actions[0]?.args, null)
  assert.match(report.draft, /skip: true/)
  assert.match(report.draft, /回看原始调用/)
})

test('drops a viewport screenshot but keeps an element screenshot', () => {
  const report = distillChat(chat([
    tool('browser', 'screenshot — 754×487', { action: 'screenshot', format: 'png' }, { imagePath: 'a.png' }),
    tool(
      'browser',
      'screenshot — 100×40',
      { action: 'screenshot', selector: '#yzmImg' },
      { imagePath: 'b.png' },
    ),
  ]))
  assert.equal(report.summary.steps, 1)
  assert.equal(report.summary.dropped, 1)
  assert.deepEqual(report.actions.map(action => action.verdict), ['dropped', 'step'])
  assert.equal(report.actions[1]?.stepId, 'browser-screenshot')
})