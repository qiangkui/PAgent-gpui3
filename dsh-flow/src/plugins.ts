/**
 * Plugins: JavaScript additions to the tool set, loaded from a folder.
 *
 * A plugin is a folder with `plugin.json` and an entry module that exports
 * `activate(api)`. It may register tools the model can call and prompt
 * fragments the model always sees. Loading happens once per question, so
 * dropping a folder in and asking again is enough — nothing is baked into the
 * program.
 *
 * Plugin code runs in this process, next to the engine: it can do whatever the
 * engine can do. That is why the window only loads plugins someone switched on
 * by hand, and why every plugin tool call goes through the same approval card a
 * `shell` command does.
 */

import { readdir, readFile, stat } from 'node:fs/promises'
import path from 'node:path'
import { pathToFileURL } from 'node:url'
import { TOOLS } from './tools/specs.ts'

/** The API version this engine understands. */
export const PLUGIN_API = 1

/** Longest prompt fragment one plugin may inject. */
const MAX_PROMPT_CHARS = 4_000

/** Most prompts one plugin may inject. */
const MAX_PROMPTS = 5

/** Most tools one plugin may register. */
const MAX_TOOLS = 24

/** Most panels one plugin may declare. */
const MAX_PANELS = 4

/** Most sections one panel may return. */
const MAX_SECTIONS = 6

/** Most rows (or items) one section may return. */
const MAX_PANEL_ITEMS = 40

/** Most columns a table section may have. */
const MAX_PANEL_COLUMNS = 8

/** Longest text one panel entry may carry. */
const MAX_PANEL_TEXT = 4_000

/** How long one panel provider may take before it is skipped. */
const PANEL_TIMEOUT_MS = 3_000

/** Longest description shown to the model for one tool. */
const MAX_DESCRIPTION = 400

/** Tool names the engine already uses; a plugin may not shadow one. */
const BUILTIN = new Set([...TOOLS.map((tool) => tool.function.name), 'mcp'])

/** A tool a plugin registered. */
export interface PluginTool {
  /** Name the model calls. */
  name: string
  /** What it does, as the model reads it. */
  description: string
  /** JSON schema of its arguments. */
  parameters: Record<string, unknown>
  /** The plugin folder the tool came from. */
  plugin: string
  /** Runs it; the returned text goes back to the model. */
  run: (args: Record<string, unknown>, context: PluginContext) => Promise<string> | string
}

/** What a plugin gets when a tool runs. */
export interface PluginContext {
  /** The conversation's working directory. */
  cwd: string
  /** Notes for the engine's stderr, for someone debugging a plugin. */
  log: (message: string) => void
}

/** One prompt fragment a plugin asked to send with every question. */
export interface PluginPrompt {
  /** The plugin folder's name. */
  id: string
  /** The text itself. */
  text: string
}

/** What a panel provider is told about the request being built. */
export interface PanelContext {
  /** The model this question goes to. */
  model: string
  /** The conversation's working directory. */
  cwd: string
  /** Tokens the model's window holds, as the window said; 0 when unknown. */
  contextWindow: number
  /** Characters the whole system message carries. */
  systemChars: number
  /** Characters the plugin prompt fragments carry. */
  injectedChars: number
  /** Characters the tool definitions carry. */
  toolChars: number
  /** The turns this request replays, oldest first. */
  turns: Array<{ role: string; chars: number }>
}

/** One entry of a panel's list. */
export interface PanelItem {
  /** Row label. */
  label: string
  /** Long text, when the row can be opened. */
  text?: string
  /** A number, for a bar section. */
  value?: number
  /** A short aside next to the number. */
  note?: string
}

/** One section of a panel, in one of the shapes the window can draw. */
export interface PanelSection {
  /** Heading shown above the section. */
  title: string
  /** Which shape it is: rows of text, bars, or a table. */
  kind: 'text' | 'bars' | 'table'
  /** Rows for `text` and `bars`. */
  items?: PanelItem[]
  /** Column headings for `table`. */
  columns?: string[]
  /** Cells for `table`, one array per row. */
  rows?: string[][]
}

/** A panel as it goes to the window. */
export interface PanelReport {
  /** Stable id the plugin registered. */
  id: string
  /** Heading the plugin declared. */
  title: string
  /** The plugin folder it came from. */
  plugin: string
  /** Up to four headline numbers. */
  summary: Array<{ label: string; value: string }>
  /** The sections, in the order the plugin returned them. */
  sections: PanelSection[]
}

/** A panel a plugin registered. */
export interface PluginPanel {
  /** Stable id, unique across plugins. */
  id: string
  /** Heading the window shows. */
  title: string
  /** The plugin folder the panel came from. */
  plugin: string
  /** Builds the panel; called once per question, before the first round. */
  provide: (context: PanelContext) => unknown
}

/** A plugin that could not be loaded. */
export interface PluginError {
  /** The plugin folder's name. */
  id: string
  /** Why it was skipped, in a form the window can show. */
  message: string
}

/** A plugin that could not be loaded. */
export interface PluginError {
  /** The plugin folder's name. */
  id: string
  /** Why it was skipped, in a form the window can show. */
  message: string
}

/** Everything the enabled plugins contribute. */
export interface PluginSet {
  /** Tools to offer the model. */
  tools: PluginTool[]
  /** Text to add to the system prompt. */
  prompts: PluginPrompt[]
  /** Panels the window can show next to the context breakdown. */
  panels: PluginPanel[]
  /** Plugins that were skipped, and why. */
  errors: PluginError[]
}

/** What `plugin.json` may say. */
interface Manifest {
  /** Human name; the folder name is the identity. */
  name?: unknown
  /** Version string, for the window's list. */
  version?: unknown
  /** Entry module, relative to the folder. */
  main?: unknown
  /** API version the plugin was written against. */
  api?: unknown
}

/**
 * Load the enabled plugins from a folder.
 *
 * A folder that is missing, unreadable or broken is reported and skipped: one
 * bad plugin must never stop a question from being answered.
 * @param options - the folder, and the plugin folder names that are switched on.
 * @returns the tools and prompts they contribute, plus what failed.
 */
export async function loadPlugins(
  options: { dir?: string; enabled?: string[] } = {}
): Promise<PluginSet> {
  const result: PluginSet = { tools: [], prompts: [], panels: [], errors: [] }
  const dir = (options.dir ?? '').trim()
  const enabled = new Set((options.enabled ?? []).map((entry) => entry.trim()).filter(Boolean))
  if (dir === '' || enabled.size === 0) return result

  let entries: string[]
  try {
    const listed = await readdir(dir, { withFileTypes: true })
    entries = listed.filter((entry) => entry.isDirectory()).map((entry) => entry.name).sort()
  } catch {
    // No plugin folder yet is the normal first run, not a problem to report.
    return result
  }

  for (const id of entries) {
    if (!enabled.has(id)) continue
    try {
      await loadOne(path.join(dir, id), id, result)
    } catch (error) {
      result.errors.push({ id, message: messageOf(error) })
    }
  }
  return result
}

/**
 * Read one plugin folder and collect what it registers.
 * @param folder - absolute path of the plugin.
 * @param id - folder name, used as the plugin's identity.
 * @param into - the set being filled.
 */
async function loadOne(folder: string, id: string, into: PluginSet): Promise<void> {
  const manifestPath = path.join(folder, 'plugin.json')
  let manifest: Manifest
  try {
    manifest = JSON.parse(await readFile(manifestPath, 'utf8')) as Manifest
  } catch (error) {
    throw new Error(`读不到 plugin.json：${messageOf(error)}`)
  }
  const api = typeof manifest.api === 'number' ? manifest.api : 0
  if (api !== PLUGIN_API) {
    throw new Error(`plugin.json 的 api 是 ${api}，这个引擎只支持 ${PLUGIN_API}`)
  }
  const entry = typeof manifest.main === 'string' && manifest.main.trim() ? manifest.main.trim() : 'index.js'
  const file = path.resolve(folder, entry)
  if (!file.startsWith(path.resolve(folder))) {
    throw new Error('plugin.json 的 main 指向了插件目录之外')
  }
  const info = await stat(file).catch(() => null)
  if (!info?.isFile()) {
    throw new Error(`找不到入口文件 ${entry}`)
  }
  const module = (await import(pathToFileURL(file).href)) as { activate?: unknown }
  if (typeof module.activate !== 'function') {
    throw new Error('入口文件没有导出 activate(api)')
  }

  const tools: PluginTool[] = []
  const prompts: string[] = []
  const panels: PluginPanel[] = []
  const taken = new Set(into.panels.map((panel) => panel.id))
  const api1 = {
    /** Offer a tool to the model. */
    registerTool(spec: unknown, run: unknown): void {
      if (tools.length >= MAX_TOOLS) throw new Error(`一个插件最多注册 ${MAX_TOOLS} 个工具`)
      if (typeof run !== 'function') throw new Error('registerTool 的第二个参数必须是函数')
      tools.push(readTool(spec, run as PluginTool['run'], id))
    },
    /** Add text to the system prompt of every question. */
    injectPrompt(text: unknown): void {
      const value = typeof text === 'string' ? text.trim() : ''
      if (value === '') return
      if (prompts.length >= MAX_PROMPTS) throw new Error(`一个插件最多注入 ${MAX_PROMPTS} 段提示`)
      prompts.push(value.slice(0, MAX_PROMPT_CHARS))
    },
    /**
     * Add a panel to the context tab.
     *
     * The provider is called once per question with what the request is made
     * of and returns sections in a fixed set of shapes: the window draws them
     * in its own style, so a panel adds numbers and text, never layout.
     */
    registerPanel(spec: unknown, provide: unknown): void {
      if (panels.length >= MAX_PANELS) throw new Error(`一个插件最多注册 ${MAX_PANELS} 个面板`)
      if (typeof provide !== 'function') throw new Error('registerPanel 的第二个参数必须是函数')
      const panel = readPanel(spec, provide as PluginPanel['provide'], id)
      if (taken.has(panel.id)) throw new Error(`面板 id「${panel.id}」已被占用`)
      taken.add(panel.id)
      panels.push(panel)
    }
  }
  await (module.activate as (api: typeof api1) => unknown)(api1)

  into.tools.push(...tools)
  into.panels.push(...panels)
  for (const text of prompts) into.prompts.push({ id, text })
}

/**
 * Validate one registered tool.
 * @param spec - what the plugin passed to `registerTool`.
 * @param run - the function that runs it.
 * @param plugin - folder name, for the error text and the tool's origin.
 * @returns the tool, ready to offer to the model.
 */
function readTool(
  spec: unknown,
  run: PluginTool['run'],
  plugin: string
): PluginTool {
  if (!spec || typeof spec !== 'object') throw new Error('registerTool 的第一个参数必须是对象')
  const fields = spec as Record<string, unknown>
  const name = typeof fields.name === 'string' ? fields.name.trim() : ''
  if (!/^[a-z][a-z0-9_]{1,40}$/.test(name)) {
    throw new Error(`工具名「${name}」不合法：要小写字母开头，只含小写字母、数字与下划线`)
  }
  if (BUILTIN.has(name)) throw new Error(`工具名「${name}」与内置工具重名`)
  const description = typeof fields.description === 'string' ? fields.description.trim() : ''
  if (description === '') throw new Error(`工具「${name}」缺少 description`)
  const parameters =
    fields.parameters && typeof fields.parameters === 'object'
      ? (fields.parameters as Record<string, unknown>)
      : { type: 'object', properties: {} }
  return {
    name,
    description: description.slice(0, MAX_DESCRIPTION),
    parameters,
    plugin,
    run
  }
}

/**
 * Run one plugin tool and turn anything it throws into a failed call.
 * @param tool - the registered tool.
 * @param args - parsed arguments from the model.
 * @param cwd - the conversation's working directory.
 * @returns whether it worked, and the text handed back to the model.
 */
export async function runPluginTool(
  tool: PluginTool,
  args: Record<string, unknown>,
  cwd: string
): Promise<{ ok: boolean; text: string }> {
  const context: PluginContext = {
    cwd,
    log: (message) => process.stderr.write(`[plugin ${tool.plugin}] ${message}\n`)
  }
  try {
    const text = await tool.run(args, context)
    const value = typeof text === 'string' ? text : String(text ?? '')
    return { ok: true, text: value.trim() === '' ? '（插件没有返回内容）' : value }
  } catch (error) {
    return { ok: false, text: `插件工具 ${tool.name} 执行失败：${messageOf(error)}` }
  }
}

/**
 * Validate one registered panel.
 * @param spec - what the plugin passed to `registerPanel`.
 * @param provide - the function that builds it.
 * @param plugin - folder name, for the error text and the panel's origin.
 * @returns the panel, ready to be asked for content.
 */
function readPanel(
  spec: unknown,
  provide: PluginPanel['provide'],
  plugin: string
): PluginPanel {
  if (!spec || typeof spec !== 'object') throw new Error('registerPanel 的第一个参数必须是对象')
  const fields = spec as Record<string, unknown>
  const id = typeof fields.id === 'string' ? fields.id.trim() : ''
  if (!/^[a-z][a-z0-9_]{1,40}$/.test(id)) {
    throw new Error(`面板 id「${id}」不合法：要小写字母开头，只含小写字母、数字与下划线`)
  }
  const title = clip(fields.title, 60)
  if (title === '') throw new Error(`面板「${id}」缺少 title`)
  return { id, title, plugin, provide }
}

/**
 * Ask every panel for its content, one by one.
 *
 * A panel that throws, hangs or returns nothing displayable is reported and
 * skipped: the context tab still shows its own numbers.
 * @param panels - the panels the enabled plugins registered.
 * @param context - what the request is made of.
 * @returns the panels to show, plus what went wrong.
 */
export async function collectPanels(
  panels: PluginPanel[],
  context: PanelContext
): Promise<{ panels: PanelReport[]; errors: PluginError[] }> {
  const reports: PanelReport[] = []
  const errors: PluginError[] = []
  for (const panel of panels) {
    try {
      const raw = await within(Promise.resolve(panel.provide(context)), PANEL_TIMEOUT_MS)
      const report = readReport(raw, panel)
      if (report === null) {
        errors.push({ id: panel.plugin, message: `面板 ${panel.id} 没有返回可显示的内容` })
        continue
      }
      reports.push(report)
    } catch (error) {
      errors.push({ id: panel.plugin, message: `面板 ${panel.id} 出错：${messageOf(error)}` })
    }
  }
  return { panels: reports, errors }
}

/**
 * Turn whatever a panel provider returned into the shapes the window draws.
 *
 * Anything unrecognised is dropped rather than guessed at, so a typo in a
 * plugin shows up as a missing section instead of a broken tab.
 * @param raw - the provider's return value.
 * @param panel - the panel it came from.
 * @returns the report, or null when nothing displayable came back.
 */
export function readReport(raw: unknown, panel: PluginPanel): PanelReport | null {
  const body = raw && typeof raw === 'object' ? (raw as Record<string, unknown>) : {}
  const summary: Array<{ label: string; value: string }> = []
  if (Array.isArray(body.summary)) {
    for (const entry of body.summary) {
      if (!entry || typeof entry !== 'object') continue
      const fields = entry as Record<string, unknown>
      const label = clip(fields.label, 40)
      const value = clip(fields.value, 40)
      if (label === '' || value === '') continue
      summary.push({ label, value })
      if (summary.length >= 4) break
    }
  }
  const sections: PanelSection[] = []
  if (Array.isArray(body.sections)) {
    for (const entry of body.sections) {
      const section = readSection(entry)
      if (section === null) continue
      sections.push(section)
      if (sections.length >= MAX_SECTIONS) break
    }
  }
  if (summary.length === 0 && sections.length === 0) return null
  return { id: panel.id, title: panel.title, plugin: panel.plugin, summary, sections }
}

/**
 * Read one section, keeping only the fields its shape uses.
 * @param value - what the plugin put in `sections`.
 * @returns the section, or null when it is not one of the three shapes.
 */
function readSection(value: unknown): PanelSection | null {
  if (!value || typeof value !== 'object') return null
  const fields = value as Record<string, unknown>
  const title = clip(fields.title, 60) || '内容'
  const kind = typeof fields.kind === 'string' ? fields.kind : ''
  if (kind === 'table') {
    const columns = Array.isArray(fields.columns)
      ? fields.columns.map((column) => clip(column, 40)).slice(0, MAX_PANEL_COLUMNS)
      : []
    const rows: string[][] = []
    if (Array.isArray(fields.rows)) {
      for (const row of fields.rows.slice(0, MAX_PANEL_ITEMS)) {
        if (!Array.isArray(row)) continue
        const cells = row.slice(0, columns.length || MAX_PANEL_COLUMNS).map((cell) => clip(cell, 200))
        if (cells.length > 0) rows.push(cells)
      }
    }
    if (columns.length === 0 || rows.length === 0) return null
    return { title, kind: 'table', columns, rows }
  }
  if (kind !== 'text' && kind !== 'bars') return null
  const items: PanelItem[] = []
  if (Array.isArray(fields.items)) {
    for (const entry of fields.items.slice(0, MAX_PANEL_ITEMS)) {
      if (!entry || typeof entry !== 'object') continue
      const row = entry as Record<string, unknown>
      const label = clip(row.label, 80)
      const text = clip(row.text, MAX_PANEL_TEXT)
      const value = typeof row.value === 'number' && Number.isFinite(row.value) ? row.value : undefined
      const note = clip(row.note, 80)
      if (kind === 'bars' && value === undefined) continue
      if (label === '' && text === '') continue
      items.push({
        label,
        ...(text === '' ? {} : { text }),
        ...(value === undefined ? {} : { value }),
        ...(note === '' ? {} : { note })
      })
    }
  }
  if (items.length === 0) return null
  return { title, kind: kind === 'bars' ? 'bars' : 'text', items }
}

/**
 * A string of a value, trimmed and cut to `limit` characters.
 * @param value - anything a plugin passed.
 * @param limit - longest the result may be.
 * @returns the text, or an empty string when there is none.
 */
function clip(value: unknown, limit: number): string {
  if (typeof value === 'number' && Number.isFinite(value)) return String(value).slice(0, limit)
  if (typeof value !== 'string') return ''
  return value.trim().slice(0, limit)
}

/**
 * Run a promise against a deadline, so one slow plugin cannot stall a question.
 * @param work - the promise to wait for.
 * @param ms - how long it may take.
 * @returns the promise's value.
 */
function within<T>(work: Promise<T>, ms: number): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined
  const timeout = new Promise<never>((_, reject) => {
    timer = setTimeout(() => reject(new Error(`超过 ${ms} 毫秒`)), ms)
  })
  return Promise.race([work, timeout]).finally(() => {
    if (timer !== undefined) clearTimeout(timer)
  })
}

/**
 * The message of anything thrown.
 * @param error - whatever was caught.
 * @returns its text.
 */
function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}
