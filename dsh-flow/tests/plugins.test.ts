import assert from 'node:assert/strict'
import { mkdtemp, mkdir, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { test } from 'node:test'
import { PLUGIN_API, collectPanels, loadPlugins, runPluginTool } from '../src/plugins.ts'

/**
 * A throw-away plugin folder.
 * @param name - folder name of the plugin.
 * @param manifest - what `plugin.json` says.
 * @param source - the entry module's source.
 * @returns the folder the plugins live in.
 */
async function plugin(
  name: string,
  manifest: Record<string, unknown>,
  source: string
): Promise<string> {
  const root = await mkdtemp(path.join(tmpdir(), 'dsh-plugin-'))
  const folder = path.join(root, name)
  await mkdir(folder, { recursive: true })
  await writeFile(path.join(folder, 'plugin.json'), JSON.stringify(manifest), 'utf8')
  await writeFile(path.join(folder, 'index.js'), source, 'utf8')
  return root
}

test('loads an enabled plugin and runs its tool', async () => {
  const dir = await plugin(
    'demo',
    { name: 'Demo', version: '1.0.0', api: PLUGIN_API },
    `export function activate(api) {
       api.registerTool(
         { name: 'plugin_hello', description: '打招呼', parameters: { type: 'object', properties: { who: { type: 'string' } } } },
         (args) => 'hello ' + args.who
       )
       api.injectPrompt('插件提示：先看 README。')
     }`
  )
  const plugins = await loadPlugins({ dir, enabled: ['demo'] })
  assert.equal(plugins.errors.length, 0)
  assert.deepEqual(plugins.tools.map((tool) => tool.name), ['plugin_hello'])
  assert.equal(plugins.tools[0].plugin, 'demo')
  assert.deepEqual(plugins.prompts, [{ id: 'demo', text: '插件提示：先看 README。' }])

  const result = await runPluginTool(plugins.tools[0], { who: 'world' }, dir)
  assert.deepEqual(result, { ok: true, text: 'hello world' })
})

test('leaves a plugin alone until it is switched on', async () => {
  const dir = await plugin(
    'demo',
    { api: PLUGIN_API },
    'export function activate(api) { api.registerTool({ name: "plugin_x", description: "x" }, () => "x") }'
  )
  assert.deepEqual(await loadPlugins({ dir, enabled: [] }), {
    tools: [],
    prompts: [],
    panels: [],
    errors: []
  })
  // An unknown folder name in the enabled list is simply not there.
  assert.equal((await loadPlugins({ dir, enabled: ['nope'] })).tools.length, 0)
  // No folder at all is the normal first run, not an error.
  assert.deepEqual(await loadPlugins({ dir: path.join(dir, 'missing'), enabled: ['demo'] }), {
    tools: [],
    prompts: [],
    panels: [],
    errors: []
  })
})

test('reports why a plugin was skipped instead of failing the question', async () => {
  const wrongApi = await plugin('old', { api: 99 }, 'export function activate() {}')
  const old = await loadPlugins({ dir: wrongApi, enabled: ['old'] })
  assert.equal(old.tools.length, 0)
  assert.match(old.errors[0].message, /只支持/)

  const noActivate = await plugin('empty', { api: PLUGIN_API }, 'export const nothing = 1')
  const empty = await loadPlugins({ dir: noActivate, enabled: ['empty'] })
  assert.match(empty.errors[0].message, /activate/)

  const escaped = await plugin('escape', { api: PLUGIN_API, main: '../../evil.js' }, '')
  const out = await loadPlugins({ dir: escaped, enabled: ['escape'] })
  assert.match(out.errors[0].message, /插件目录之外/)

  const broken = await mkdtemp(path.join(tmpdir(), 'dsh-plugin-'))
  await mkdir(path.join(broken, 'bad'), { recursive: true })
  await writeFile(path.join(broken, 'bad', 'plugin.json'), '{ not json', 'utf8')
  const bad = await loadPlugins({ dir: broken, enabled: ['bad'] })
  assert.match(bad.errors[0].message, /plugin.json/)
})

test('refuses a tool that shadows a built-in or breaks the naming rule', async () => {
  const clash = await plugin(
    'clash',
    { api: PLUGIN_API },
    'export function activate(api) { api.registerTool({ name: "read", description: "x" }, () => "x") }'
  )
  assert.match(
    (await loadPlugins({ dir: clash, enabled: ['clash'] })).errors[0].message,
    /内置工具重名/
  )

  const odd = await plugin(
    'odd',
    { api: PLUGIN_API },
    'export function activate(api) { api.registerTool({ name: "Bad Name", description: "x" }, () => "x") }'
  )
  assert.match((await loadPlugins({ dir: odd, enabled: ['odd'] })).errors[0].message, /不合法/)
})

test('turns a throwing plugin into a failed tool call', async () => {
  const dir = await plugin(
    'boom',
    { api: PLUGIN_API },
    'export function activate(api) { api.registerTool({ name: "plugin_boom", description: "x" }, () => { throw new Error("炸了") }) }'
  )
  const plugins = await loadPlugins({ dir, enabled: ['boom'] })
  const result = await runPluginTool(plugins.tools[0], {}, dir)
  assert.equal(result.ok, false)
  assert.match(result.text, /炸了/)
})

test('a panel gets what the request is made of and comes back in window shapes', async () => {
  const dir = await plugin(
    'panels',
    { api: PLUGIN_API },
    `export function activate(api) {
       api.registerPanel({ id: 'demo_panel', title: '示例面板' }, (context) => ({
         summary: [{ label: '模型', value: context.model }],
         sections: [
           { title: '按类', kind: 'bars', items: [{ label: '系统', value: 12, note: 'chars' }, { label: '无值' }] },
           { title: '明细', kind: 'text', items: [{ label: '轮次', text: '共 ' + context.turns.length + ' 条' }] },
           { title: '表', kind: 'table', columns: ['a', 'b'], rows: [['1', '2']] }
         ]
       }))
     }`
  )
  const plugins = await loadPlugins({ dir, enabled: ['panels'] })
  assert.deepEqual(plugins.errors, [])
  assert.deepEqual(plugins.panels.map((panel) => panel.id), ['demo_panel'])

  const built = await collectPanels(plugins.panels, {
    model: 'demo-model',
    cwd: dir,
    contextWindow: 128000,
    systemChars: 10,
    injectedChars: 0,
    toolChars: 100,
    turns: [{ role: 'user', chars: 5 }]
  })
  assert.deepEqual(built.errors, [])
  assert.equal(built.panels.length, 1)
  const report = built.panels[0]
  assert.equal(report.title, '示例面板')
  assert.equal(report.plugin, 'panels')
  assert.deepEqual(report.summary, [{ label: '模型', value: 'demo-model' }])
  assert.deepEqual(report.sections.map((section) => section.kind), ['bars', 'text', 'table'])
  // A bar without a number cannot be drawn, so it is dropped rather than drawn
  // as zero.
  assert.deepEqual(report.sections[0].items, [{ label: '系统', value: 12, note: 'chars' }])
  assert.equal(report.sections[1].items?.[0]?.text, '共 1 条')
  assert.deepEqual(report.sections[2].rows, [['1', '2']])
})

test('a broken panel is reported and the rest still show', async () => {
  const dir = await plugin(
    'mixed',
    { api: PLUGIN_API },
    `export function activate(api) {
       api.registerPanel({ id: 'boom_panel', title: '会坏的' }, () => { throw new Error('炸了') })
       api.registerPanel({ id: 'empty_panel', title: '空的' }, () => ({ summary: [], sections: [] }))
       api.registerPanel({ id: 'good_panel', title: '好的' }, () => ({ summary: [{ label: 'a', value: '1' }] }))
     }`
  )
  const plugins = await loadPlugins({ dir, enabled: ['mixed'] })
  const built = await collectPanels(plugins.panels, {
    model: 'm',
    cwd: dir,
    contextWindow: 0,
    systemChars: 0,
    injectedChars: 0,
    toolChars: 0,
    turns: []
  })
  assert.deepEqual(built.panels.map((panel) => panel.id), ['good_panel'])
  assert.equal(built.errors.length, 2)
  assert.match(built.errors[0].message, /炸了/)
  assert.match(built.errors[1].message, /没有返回可显示的内容/)
})

test('refuses a panel that breaks the naming rule or repeats an id', async () => {
  const odd = await plugin(
    'odd',
    { api: PLUGIN_API },
    'export function activate(api) { api.registerPanel({ id: "Bad Id", title: "x" }, () => ({})) }'
  )
  assert.match((await loadPlugins({ dir: odd, enabled: ['odd'] })).errors[0].message, /不合法/)

  const untitled = await plugin(
    'untitled',
    { api: PLUGIN_API },
    'export function activate(api) { api.registerPanel({ id: "fine_id" }, () => ({})) }'
  )
  assert.match(
    (await loadPlugins({ dir: untitled, enabled: ['untitled'] })).errors[0].message,
    /缺少 title/
  )

  const twice = await plugin(
    'twice',
    { api: PLUGIN_API },
    `export function activate(api) {
       api.registerPanel({ id: 'same_id', title: 'a' }, () => ({}))
       api.registerPanel({ id: 'same_id', title: 'b' }, () => ({}))
     }`
  )
  assert.match((await loadPlugins({ dir: twice, enabled: ['twice'] })).errors[0].message, /已被占用/)
})
