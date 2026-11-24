import { execFile } from 'node:child_process'
import { promisify } from 'node:util'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { readFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'

const run = promisify(execFile)
const here = dirname(fileURLToPath(import.meta.url))
const script = join(here, 'win32_input.py')
const snapshotDir = join(tmpdir(), 'pa-win32-snapshots')
const SNAPSHOT_TTL_MS = 45_000
const DANGEROUS = /删除|移除|清空|格式化|卸载|永久|支付|付款|购买|下单|结账|转账|汇款|确认交易|退出登录|注销|登出/i

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

function snapshotHandle(value) {
  return String(value ?? '').trim().toUpperCase()
}

async function loadSnapshot(id) {
  const text = String(id ?? '').trim()
  if (!/^[0-9a-f-]{36}$/i.test(text)) {
    throw new Error('snapshot_id 不合法；请先调用 win32_observe')
  }
  let snapshot
  try {
    snapshot = JSON.parse(await readFile(join(snapshotDir, `${text}.json`), 'utf8'))
  } catch {
    throw new Error('找不到观察快照；请先调用 win32_observe')
  }
  const age = Date.now() - Number(snapshot.at || 0)
  if (!Number.isFinite(age) || age > (snapshot.ttl_ms || SNAPSHOT_TTL_MS)) {
    throw new Error('观察快照已过期；请重新调用 win32_observe')
  }
  return snapshot
}

/**
 * Resolve an element index from a fresh observation.
 * @returns {Promise<{snapshot:object, entry:object|null, handle:string}>}
 */
async function resolveTarget(args, { center = true } = {}) {
  const snapshotId = typeof args.snapshot === 'string' ? args.snapshot.trim() : ''
  const rawElement = args.element
  const hasElement = rawElement !== undefined && rawElement !== null
  if (!snapshotId) {
    if (hasElement) throw new Error('使用 element 时必须同时提供 snapshot')
    return { snapshot: null, entry: null, handle: String(args.handle ?? '') }
  }
  const snapshot = await loadSnapshot(snapshotId)
  const snapshotHandleText = snapshotHandle(snapshot.window?.handle)
  if (args.handle && snapshotHandleText !== snapshotHandle(args.handle)) {
    throw new Error('handle 与观察快照的窗口不一致')
  }
  if (!hasElement) {
    return { snapshot, entry: null, handle: snapshot.window.handle }
  }
  const index = Number(rawElement)
  const entry = (snapshot.elements || []).find((item) => Number(item.index) === index)
  if (!entry) throw new Error(`元素编号 ${rawElement} 不在当前快照中；请重新 observe`)
  const out = { ...args, handle: snapshot.window.handle }
  if (center && entry.center) {
    out.x = entry.center.x
    out.y = entry.center.y
  }
  return { snapshot, entry, handle: snapshot.window.handle, args: out }
}

function guardEntry(entry, action, args) {
  if (!entry) return
  const label = String(entry.name || '')
  if (entry.password) {
    if (action === 'type') {
      throw new Error('敏感输入保护：密码框拒绝自动输入，请由用户本人输入')
    }
    if (action === 'click' && args.confirm_password !== true) {
      throw new Error('目标被标记为密码框；确认点击请传 confirm_password=true')
    }
  }
  if (DANGEROUS.test(label) && args.confirm_dangerous !== true) {
    throw new Error(
      `目标「${label}」包含删除/支付/注销等危险词；确认操作请传 confirm_dangerous=true`
    )
  }
}

function register(api, spec, prepare) {
  const { action, ...schema } = spec
  api.registerTool(schema, async (args) => {
    const base = { ...(args ?? {}), action }
    const payload = prepare ? await prepare(base) : base
    return render(await callBridge(payload))
  })
}

export function activate(api) {
  register(api, {
    action: 'focus',
    name: 'win32_focus',
    description: '把指定窗口恢复并切到前台。可传 snapshot+element 聚焦观察到的具体控件所在窗口；输入文字/按键前通常先调用。',
    parameters: {
      type: 'object',
      properties: {
        handle: { type: 'string', description: '窗口句柄，例如 0x201D2' },
        snapshot: { type: 'string', description: 'win32_observe 返回的 snapshot_id' },
        element: { type: 'integer', description: '观察快照中的元素编号' },
        dry_run: { type: 'boolean', description: '只验证，不切换窗口' }
      },
      additionalProperties: false
    }
  }, async (args) => {
    const target = await resolveTarget(args, { center: false })
    if (!target.handle) throw new Error('缺少 handle 或 snapshot')
    return { ...args, handle: target.handle }
  })

  register(api, {
    action: 'click',
    name: 'win32_click',
    description: '用 SendInput 点击鼠标。优先传 snapshot+element 点击 UIA 元素中心；也可给 handle+x/y 客户区坐标、仅 x/y 屏幕坐标，或省略坐标点击当前鼠标位置。密码框/危险词目标需要额外确认参数。',
    parameters: {
      type: 'object',
      properties: {
        snapshot: { type: 'string', description: 'win32_observe 返回的 snapshot_id' },
        element: { type: 'integer', description: '观察快照中的元素编号；优先于 x/y' },
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
        confirm_password: { type: 'boolean', description: '确认目标为密码框仍要点击' },
        confirm_dangerous: { type: 'boolean', description: '确认目标含删除/支付等危险词' },
        dry_run: { type: 'boolean', description: '只校验坐标并返回计划，不点击' }
      },
      additionalProperties: false
    }
  }, async (args) => {
    const target = await resolveTarget(args)
    guardEntry(target.entry, 'click', args)
    if (!target.snapshot && target.entry === null && !target.handle && args.x === undefined) {
      // clicking the current cursor is still allowed; no target needed
    }
    return { ...args, ...(target.args ?? {}), handle: target.handle || args.handle, screen: Boolean(target.snapshot) }
  })
register(api, {
    action: 'scroll',
    name: 'win32_scroll',
    description: '滚动鼠标滚轮。可传 snapshot+element 在元素中心滚动，或 handle+x/y。clicks 正数向上/向右，负数向下/向左；一个单位是一格（120）。',
    parameters: {
      type: 'object',
      properties: {
        snapshot: { type: 'string', description: 'win32_observe 返回的 snapshot_id' },
        element: { type: 'integer', description: '观察快照中的元素编号' },
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
  }, async (args) => {
    const target = await resolveTarget(args)
    return { ...args, ...(target.args ?? {}), handle: target.handle || args.handle, screen: Boolean(target.snapshot) }
  })

  register(api, {
    action: 'type',
    name: 'win32_type',
    description: '用 SendInput 的 Unicode 模式输入文字，支持中文；\\n 转为 Enter，\\t 转为 Tab。推荐先用 win32_observe 找到输入框元素，再传 snapshot+element；密码框拒绝自动输入。',
    parameters: {
      type: 'object',
      properties: {
        snapshot: { type: 'string', description: 'win32_observe 返回的 snapshot_id' },
        element: { type: 'integer', description: '观察快照中的输入框元素编号' },
        handle: { type: 'string', description: '目标窗口句柄，可选；提供时先聚焦' },
        text: { type: 'string', description: '要输入的文字，最多 5000 字符' },
        confirm_dangerous: { type: 'boolean', description: '确认目标含危险词仍要输入' },
        dry_run: { type: 'boolean', description: '只校验并返回计划，不输入' }
      },
      required: ['text'],
      additionalProperties: false
    }
  }, async (args) => {
    const target = await resolveTarget(args)
    guardEntry(target.entry, 'type', args)
    if (target.entry) {
      const type = String(target.entry.control_type || '')
      const inputLike = ['Edit', 'Document', 'Text', 'ComboBox', 'Spinner'].includes(type)
        || (target.entry.patterns || []).includes('Value')
      if (!inputLike) {
        throw new Error(`元素 ${args.element} 是 ${type || '未知'}，不是输入控件；请先选择输入框或单独点击`)
      }
    }
    return { ...args, ...(target.args ?? {}), handle: target.handle || args.handle, screen: Boolean(target.snapshot) }
  })

  register(api, {
    action: 'keys',
    name: 'win32_keys',
    description: '发送一组按键组合，例如 ["CTRL","S"]、["ENTER"]、["TAB"]。可传 snapshot 绑定到新鲜观察窗口；提供 handle 时会先尝试切到前台。',
    parameters: {
      type: 'object',
      properties: {
        snapshot: { type: 'string', description: 'win32_observe 返回的 snapshot_id' },
        handle: { type: 'string', description: '目标窗口句柄，可选；提供时先聚焦' },
        keys: {
          type: 'array',
          minItems: 1,
          maxItems: 5,
          items: { type: 'string' },
          description: '按顺序按下的键，例如 ["CTRL","SHIFT","S"]'
        },
        confirm_dangerous: { type: 'boolean', description: '确认窗口标题含危险词仍要按键' },
        dry_run: { type: 'boolean', description: '只校验并返回计划，不按键' }
      },
      required: ['keys'],
      additionalProperties: false
    }
  }, async (args) => {
    const target = await resolveTarget(args, { center: false })
    if (target.snapshot && DANGEROUS.test(String(target.snapshot.window?.title || '')) && args.confirm_dangerous !== true) {
      throw new Error('窗口标题包含危险词；确认按键请传 confirm_dangerous=true')
    }
    return { ...args, ...(target.args ?? {}), handle: target.handle || args.handle }
  })
register(api, {
    action: 'drag',
    name: 'win32_drag',
    description: '在新鲜观察快照的窗口内拖拽。坐标是与 win32_observe 相同的屏幕物理像素；先用 from 点按下，移动，再在 to 点松开。',
    parameters: {
      type: 'object',
      properties: {
        snapshot: { type: 'string', description: 'win32_observe 返回的 snapshot_id' },
        from_x: { type: 'integer', description: '起点屏幕 x' },
        from_y: { type: 'integer', description: '起点屏幕 y' },
        to_x: { type: 'integer', description: '终点屏幕 x' },
        to_y: { type: 'integer', description: '终点屏幕 y' },
        duration_ms: {
          type: 'integer',
          minimum: 50,
          maximum: 5000,
          description: '拖拽时长，默认 350ms'
        },
        dry_run: { type: 'boolean', description: '只校验并返回计划，不拖拽' }
      },
      required: ['snapshot', 'from_x', 'from_y', 'to_x', 'to_y'],
      additionalProperties: false
    }
  }, async (args) => {
    const target = await resolveTarget(args, { center: false })
    if (!target.snapshot) throw new Error('drag 需要 win32_observe 的 snapshot')
    return { ...args, handle: target.handle }
  })

  api.registerTool(
    {
      name: 'win32_wait',
      description: '等待界面加载/动画完成（本地 sleep），默认 1000ms，最大 60000ms。等待后应重新 win32_observe。',
      parameters: {
        type: 'object',
        properties: {
          ms: { type: 'integer', minimum: 0, maximum: 60000, description: '等待毫秒数' }
        },
        required: ['ms'],
        additionalProperties: false
      }
    },
    async (args) => {
      const ms = Math.max(0, Math.min(Number(args.ms) || 0, 60_000))
      await new Promise((resolve) => setTimeout(resolve, ms))
      return render({ ok: true, action: 'wait', ms })
    }
  )
}