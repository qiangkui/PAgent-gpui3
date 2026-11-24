import { execFile } from 'node:child_process'
import { promisify } from 'node:util'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const run = promisify(execFile)
const here = dirname(fileURLToPath(import.meta.url))
const script = join(here, 'win32_input.py')

const pythonCandidates = [
  { command: process.env.PYTHON || 'python', prefix: [] },
  { command: 'py', prefix: ['-3'] }
]

async function callBridge(payload) {
  let lastError = null
  for (const candidate of pythonCandidates) {
    try {
      const argv = [...candidate.prefix, script, JSON.stringify(payload)]
      const { stdout } = await run(candidate.command, argv, {
        cwd: here,
        windowsHide: true,
        timeout: 30_000,
        maxBuffer: 4 * 1024 * 1024,
        encoding: 'utf8'
      })
      const text = stdout.trim()
      if (text === '') throw new Error('输入脚本没有返回内容')
      const parsed = JSON.parse(text)
      if (!parsed || parsed.ok !== true) throw new Error(String(parsed?.error ?? '输入操作失败'))
      return parsed
    } catch (error) {
      lastError = error
      if (error?.code !== 'ENOENT') break
    }
  }
  const message = lastError instanceof Error ? lastError.message : String(lastError)
  throw new Error(`无法运行 Python 输入桥：${message}`)
}

function render(result) {
  return JSON.stringify(result, null, 2)
}

function register(api, spec) {
  const { action, ...schema } = spec
  api.registerTool(schema, async (args) => {
    const payload = { action }
    for (const [key, value] of Object.entries(args ?? {})) payload[key] = value
    return render(await callBridge(payload))
  })
}

export function activate(api) {
  register(api, {
    action: 'focus',
    name: 'win32_focus',
    description: '把指定 Windows 窗口恢复并切到前台。输入文字/按键前通常先调用它；失败时返回明确错误。',
    parameters: {
      type: 'object',
      properties: {
        handle: { type: 'string', description: '窗口句柄，例如 0x201D2' },
        dry_run: { type: 'boolean', description: '只验证，不切换窗口' }
      },
      required: ['handle'],
      additionalProperties: false
    }
  })

  register(api, {
    action: 'click',
    name: 'win32_click',
    description: '用 SendInput 点击鼠标。给定 handle + x/y 时按该窗口客户区坐标；只给 x/y 时按屏幕坐标；都不给时点击当前鼠标位置。支持左/右/中键与双击。',
    parameters: {
      type: 'object',
      properties: {
        handle: { type: 'string', description: '目标窗口句柄；与 x/y 同用时坐标按客户区解释' },
        x: { type: 'integer', description: '横坐标；省略则用当前鼠标位置' },
        y: { type: 'integer', description: '纵坐标；省略则用当前鼠标位置' },
        button: {
          type: 'string',
          enum: ['left', 'right', 'middle'],
          description: '鼠标键，默认 left'
        },
        double: { type: 'boolean', description: '是否双击，默认 false' },
        activate: { type: 'boolean', description: '点击前是否先切到前台，默认 false' },
        dry_run: { type: 'boolean', description: '只校验坐标并返回计划，不点击' }
      },
      additionalProperties: false
    }
  })
register(api, {
    action: 'scroll',
    name: 'win32_scroll',
    description: '滚动鼠标滚轮。clicks 正数向上/向右，负数向下/向左；一个单位是一格（120）。可选 x/y 先把鼠标移到目标点。',
    parameters: {
      type: 'object',
      properties: {
        handle: { type: 'string', description: '目标窗口句柄，可选；与 x/y 同用时坐标按客户区解释' },
        clicks: {
          type: 'integer',
          minimum: -100,
          maximum: 100,
          description: '滚动格数；正数向上/向右，负数向下/向左'
        },
        horizontal: { type: 'boolean', description: '是否横向滚动，默认 false' },
        x: { type: 'integer', description: '可选横坐标' },
        y: { type: 'integer', description: '可选纵坐标' },
        dry_run: { type: 'boolean', description: '只校验并返回计划，不滚动' }
      },
      required: ['clicks'],
      additionalProperties: false
    }
  })

  register(api, {
    action: 'type',
    name: 'win32_type',
    description: '用 SendInput 的 Unicode 模式输入文字，支持中文；\\n 转为 Enter，\\t 转为 Tab。提供 handle 时会先尝试切到前台，失败则不输入。',
    parameters: {
      type: 'object',
      properties: {
        handle: { type: 'string', description: '目标窗口句柄，可选；提供时先聚焦' },
        text: { type: 'string', description: '要输入的文字，最多 5000 字符' },
        dry_run: { type: 'boolean', description: '只校验并返回计划，不输入' }
      },
      required: ['text'],
      additionalProperties: false
    }
  })

  register(api, {
    action: 'keys',
    name: 'win32_keys',
    description: '发送一组按键组合，例如 ["CTRL","S"]、["ENTER"]、["TAB"]。支持修饰键、功能键、方向键和单个字母/数字。提供 handle 时会先尝试切到前台。',
    parameters: {
      type: 'object',
      properties: {
        handle: { type: 'string', description: '目标窗口句柄，可选；提供时先聚焦' },
        keys: {
          type: 'array',
          minItems: 1,
          maxItems: 5,
          items: { type: 'string' },
          description: '按顺序按下的键，例如 ["CTRL","SHIFT","S"]'
        },
        dry_run: { type: 'boolean', description: '只校验并返回计划，不按键' }
      },
      required: ['keys'],
      additionalProperties: false
    }
  })
}