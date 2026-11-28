// The Office plugin is shipped as plain files outside this package, so nothing
// type-checks it or imports it. These checks keep its four moving parts in sync:
// `office_help` (what the model is told), the bridge's switch (what actually
// runs), the tool definitions (what the engine exposes) and the bridge's
// encoding (PowerShell 5.1 parses a BOM-less file as ANSI, which breaks the
// Chinese messages the tools return).
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'
import { loadPlugins } from '../src/plugins.ts'

const root = new URL('../../plugins/office/', import.meta.url)
const text = (name: string): Promise<string> => readFile(new URL(name, root), 'utf8')

/** Indented `'name' {` labels of a switch, up to its `default {` arm. */
function armLabels(lines: string[], from: number, stop: RegExp): string[] {
  const labels: string[] = []
  for (let index = from; index < lines.length; index += 1) {
    if (stop.test(lines[index])) break
    const match = /^        '([a-z_]+)' \{/.exec(lines[index])
    if (match) labels.push(match[1])
  }
  return labels
}

/** Operations `office_edit` accepts for one application. */
function editOps(bridge: string, functionName: string): string[] {
  const lines = bridge.split(/\r?\n/)
  const start = lines.findIndex((line) => line.startsWith(`function ${functionName}`))
  assert.ok(start >= 0, `${functionName} 不在桥接脚本里`)
  return armLabels(lines, start + 1, /^        default \{/)
}

/** Actions the bridge's top-level switch dispatches. */
function bridgeActions(bridge: string): string[] {
  const lines = bridge.split(/\r?\n/)
  const start = lines.findIndex((line) => line.trim() === 'switch ($action) {')
  assert.ok(start >= 0, '桥接脚本缺少 action 分发')
  return armLabels(lines, start + 1, /^    \}/)
}

/** Tool names a help constant lists, one per `- name ...` line. */
function helpOps(help: string): string[] {
  return [...help.matchAll(/^- ([a-z_]+)/gm)].map((match) => match[1])
}

async function readPlugin(): Promise<{
  bridge: string
  source: string
  help: { word: string; excel: string; powerpoint: string }
}> {
  const bridge = await text('office_bridge.ps1')
  const source = await text('index.js')
  const word = /const WORD_HELP = `([\s\S]*?)`/.exec(source)
  const excel = /const EXCEL_HELP = `([\s\S]*?)`/.exec(source)
  const powerpoint = /const POWERPOINT_HELP = `([\s\S]*?)`/.exec(source)
  assert.ok(word && excel && powerpoint, 'index.js 缺少 WORD_HELP / EXCEL_HELP / POWERPOINT_HELP')
  return { bridge, source, help: { word: word[1], excel: excel[1], powerpoint: powerpoint[1] } }
}

test('office_help lists exactly the Word operations the bridge implements', async () => {
  const { bridge, help } = await readPlugin()
  const listed = helpOps(help.word).sort()
  const implemented = editOps(bridge, 'Run-Word-Op').sort()
  assert.deepEqual(listed, implemented, 'Word 帮助与 Run-Word-Op 不一致')
})

test('office_help lists exactly the Excel operations the bridge implements', async () => {
  const { bridge, help } = await readPlugin()
  const listed = helpOps(help.excel).sort()
  const implemented = editOps(bridge, 'Run-Excel-Op').sort()
  assert.deepEqual(listed, implemented, 'Excel 帮助与 Run-Excel-Op 不一致')
})

test('office_help lists exactly the PowerPoint operations the bridge implements', async () => {
  const { bridge, help } = await readPlugin()
  const listed = helpOps(help.powerpoint).sort()
  const implemented = editOps(bridge, 'Run-PowerPoint-Op').sort()
  assert.deepEqual(listed, implemented, 'PowerPoint 帮助与 Run-PowerPoint-Op 不一致')
})

test('every registered office tool has a bridge action', async () => {
  const { bridge, source } = await readPlugin()
  const tools = [...source.matchAll(/name: '(office_[a-z_]+)'/g)].map((match) => match[1])
  assert.ok(tools.length >= 8, `工具太少：${tools.join(', ')}`)
  // `office_help` is answered from the constants above, never by the bridge.
  const local = new Set(['office_help'])
  const actions = new Set(bridgeActions(bridge))
  for (const tool of tools) {
    if (local.has(tool)) continue
    const action = tool.slice('office_'.length)
    assert.ok(actions.has(action), `${tool} 没有对应的桥接 action：${[...actions].join(', ')}`)
  }
  assert.deepEqual([...actions].sort(), tools.filter((tool) => !local.has(tool)).map((tool) => tool.slice(7)).sort())
})

test('the bridge keeps its UTF-8 BOM so PowerShell reads it as UTF-8', async () => {
  const bytes = await readFile(new URL('office_bridge.ps1', root))
  assert.deepEqual([...bytes.subarray(0, 3)], [0xef, 0xbb, 0xbf])
})

test('plugin.json names the entry the plugin actually ships', async () => {
  const manifest = JSON.parse(await text('plugin.json')) as {
    name?: string
    version?: string
    api?: number
    main?: string
  }
  assert.ok(manifest.name && manifest.name.length > 0)
  assert.match(String(manifest.version), /^\d+\.\d+\.\d+$/)
  assert.equal(manifest.api, 1)
  assert.equal(manifest.main, 'index.js')
  await text(String(manifest.main))
})

test('the engine loader accepts the shipped office plugin', async () => {
  const shipped = fileURLToPath(new URL('../../plugins', import.meta.url))
  const loaded = await loadPlugins({ dir: shipped, enabled: ['office'] })
  assert.deepEqual(loaded.errors, [])
  assert.deepEqual(
    loaded.tools.map((tool) => tool.name).sort(),
    [
      'office_close',
      'office_edit',
      'office_find',
      'office_help',
      'office_open',
      'office_read',
      'office_save',
      'office_status'
    ]
  )
  // Every tool's description is what the model sees; a 400-character cut would
  // mean half the guidance never arrives.
  for (const tool of loaded.tools) {
    assert.ok(tool.description.length > 0)
    assert.ok(tool.description.length <= 400, `${tool.name} 的描述超过 400 字会被截断`)
    assert.equal(tool.parameters.type, 'object')
  }
  assert.equal(loaded.prompts.length, 1)
  assert.match(loaded.prompts[0].text, /office_find/)
})