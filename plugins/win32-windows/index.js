import { execFile } from 'node:child_process'
import { promisify } from 'node:util'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { randomUUID } from 'node:crypto'
import { mkdir, readFile, readdir, stat, unlink, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'

const run = promisify(execFile)
const here = dirname(fileURLToPath(import.meta.url))
const script = join(here, 'win32_windows.py')
const uiaScript = join(here, 'uia_tree.ps1')
const snapshotDir = join(tmpdir(), 'pa-win32-snapshots')
const SNAPSHOT_TTL_MS = 45_000

/**
 * Windows 上的 Python 命令。`python` 可能是 Microsoft Store 别名，因此
 * 退回到 `py -3`；两者都不可用时给出可读错误。
 */
const pythonCandidates = [
  { command: process.env.PYTHON || 'python', prefix: [] },
  { command: 'py', prefix: ['-3'] }
]

/**
 * Run the Python bridge once.
 * @param {Record<string, unknown>} payload
 * @returns {Promise<Record<string, unknown>>}
 */
async function callBridge(payload) {
  let lastError = null
  for (const candidate of pythonCandidates) {
    try {
      const argv = [...candidate.prefix, script, JSON.stringify(payload)]
      const { stdout } = await run(candidate.command, argv, {
        cwd: here,
        windowsHide: true,
        timeout: 30_000,
        maxBuffer: 8 * 1024 * 1024,
        encoding: 'utf8'
      })
      const text = stdout.trim()
      if (text === '') throw new Error('窗口检查脚本没有返回内容')
      const parsed = JSON.parse(text)
      if (!parsed || parsed.ok !== true) {
        throw new Error(String(parsed?.error ?? '窗口检查失败'))
      }
      return parsed
    } catch (error) {
      lastError = error
      if (error?.code !== 'ENOENT') break
    }
  }
  const message = lastError instanceof Error ? lastError.message : String(lastError)
  throw new Error(`无法运行 Python 窗口检查：${message}`)
}

/**
 * @param {Record<string, unknown>} args
 * @returns {{ action: string, payload: Record<string, unknown> }}
 */
function listPayload(args) {
  const parent = typeof args.parent === 'string' ? args.parent.trim() : ''
  const depth = Number.isFinite(Number(args.depth)) ? Number(args.depth) : parent ? 1 : 0
  const limit = Number.isFinite(Number(args.limit)) ? Number(args.limit) : 200
  return {
    action: parent ? 'children' : 'top',
    payload: {
      action: parent ? 'children' : 'top',
      parent,
      depth: Math.min(8, Math.max(1, Math.trunc(depth))),
      include_invisible: args.include_invisible === true,
      title_contains: typeof args.title_contains === 'string' ? args.title_contains : '',
      class_contains: typeof args.class_contains === 'string' ? args.class_contains : '',
      limit: Math.min(500, Math.max(1, Math.trunc(limit)))
    }
  }
}

/**
 * Format one bridge answer for the model.
 * @param {Record<string, unknown>} result
 * @returns {string}
 */
function render(result) {
  return JSON.stringify(result, null, 2)
}

/**
 * Read the UIA tree through the .NET automation client. PowerShell is used
 * instead of a Python COM package so the plugin stays dependency-free.
 */
async function callUia(payload) {
  const { stdout } = await run(
    'powershell.exe',
    [
      '-NoProfile',
      '-NonInteractive',
      '-ExecutionPolicy', 'Bypass',
      '-File', uiaScript,
      JSON.stringify(payload)
    ],
    {
      cwd: here,
      windowsHide: true,
      timeout: 30_000,
      maxBuffer: 8 * 1024 * 1024,
      encoding: 'utf8'
    }
  )
  const text = stdout.trim()
  if (text === '') throw new Error('UIA 脚本没有返回内容')
  const parsed = JSON.parse(text)
  if (!parsed || parsed.ok !== true) throw new Error(String(parsed?.error ?? 'UIA 读取失败'))
  return parsed
}

/** Remove snapshots that are too old to be resolved anyway. */
async function pruneSnapshots() {
  await mkdir(snapshotDir, { recursive: true })
  const now = Date.now()
  const entries = await readdir(snapshotDir).catch(() => [])
  for (const name of entries) {
    const file = join(snapshotDir, name)
    const info = await stat(file).catch(() => null)
    if (info && now - info.mtimeMs > 10 * 60_000) await unlink(file).catch(() => {})
  }
}

/**
 * Persist one observation for the input plugin. The ID is a random UUID; the
 * input plugin only accepts files in this directory, so a model cannot point it
 * at an arbitrary path.
 */
async function saveSnapshot(window, elements) {
  await pruneSnapshots()
  const id = randomUUID()
  const snapshot = {
    id,
    at: Date.now(),
    ttl_ms: SNAPSHOT_TTL_MS,
    window,
    elements
  }
  await writeFile(join(snapshotDir, `${id}.json`), JSON.stringify(snapshot), 'utf8')
  return id
}

export function activate(api) {
  api.registerTool(
    {
      name: 'win32_list_windows',
      description:
        '只读枚举 Windows 桌面窗口/控件。不传 parent 列出顶层窗口；传 parent（十六进制句柄）展开该窗口的子级。返回句柄、标题、类名、位置尺寸、PID、可见性等。通常先看顶层，再用 handle 继续向下查。',
      parameters: {
        type: 'object',
        properties: {
          parent: {
            type: 'string',
            description: '父窗口句柄，例如 0x000000000012A34；省略时枚举顶层窗口'
          },
          depth: {
            type: 'integer',
            minimum: 1,
            maximum: 8,
            description: '从 parent 往下展开几层，默认 1'
          },
          include_invisible: {
            type: 'boolean',
            description: '是否包含不可见窗口/控件，默认 false；找不到目标时可设 true'
          },
          title_contains: {
            type: 'string',
            description: '标题包含的文字，不区分大小写'
          },
          class_contains: {
            type: 'string',
            description: '窗口类名包含的文字，不区分大小写'
          },
          limit: {
            type: 'integer',
            minimum: 1,
            maximum: 500,
            description: '最多返回多少条，默认 200'
          }
        },
        additionalProperties: false
      }
    },
    async (args) => {
      const { payload } = listPayload(args)
      return render(await callBridge(payload))
    }
  )

  api.registerTool(
    {
      name: 'win32_window_info',
      description:
        '按句柄读取一个 Windows 窗口/控件的详细信息：位置、客户区、样式、标题、类名、父子/所有者、根祖先、PID、线程、进程路径、DPI。先用 win32_list_windows 找到句柄。',
      parameters: {
        type: 'object',
        properties: {
          handle: {
            type: 'string',
            description: '窗口句柄，例如 0x000000000012A34'
          }
        },
        required: ['handle'],
        additionalProperties: false
      }
    },
    async (args) => {
      const handle = typeof args.handle === 'string' ? args.handle.trim() : ''
      if (handle === '') throw new Error('缺少 handle')
      return render(await callBridge({ action: 'info', handle }))
    }
  )
api.registerTool(
    {
      name: 'win32_observe',
      description:
        '用 UI Automation 读取一个窗口的元素树（按钮、输入框、菜单等），返回元素编号、名称、控件类型、屏幕坐标/中心点、是否密码框和可用模式。会生成 snapshot_id，后续 win32-input 的点击/输入应传 snapshot + element，避免使用过期坐标。',
      parameters: {
        type: 'object',
        properties: {
          handle: {
            type: 'string',
            description: '目标窗口句柄，先用 win32_list_windows 获取'
          },
          depth: {
            type: 'integer',
            minimum: 1,
            maximum: 6,
            description: 'UIA 树展开层数，默认 3'
          },
          limit: {
            type: 'integer',
            minimum: 1,
            maximum: 500,
            description: '最多返回多少元素，默认 200'
          },
          include_offscreen: {
            type: 'boolean',
            description: '是否包含屏幕外元素，默认 false'
          },
          actionable_only: {
            type: 'boolean',
            description: '只返回可交互或带名称的元素，默认 true'
          },
          name_contains: {
            type: 'string',
            description: '元素名称包含的文字，不区分大小写'
          },
          type_contains: {
            type: 'string',
            description: '控件类型包含的文字，例如 Button / Edit'
          }
        },
        required: ['handle'],
        additionalProperties: false
      }
    },
    async (args) => {
      const handle = typeof args.handle === 'string' ? args.handle.trim() : ''
      if (handle === '') throw new Error('缺少 handle')
      const result = await callUia({
        handle,
        depth: Number.isFinite(Number(args.depth)) ? Number(args.depth) : 3,
        limit: Number.isFinite(Number(args.limit)) ? Number(args.limit) : 200,
        include_offscreen: args.include_offscreen === true,
        actionable_only: args.actionable_only !== false,
        name_contains: typeof args.name_contains === 'string' ? args.name_contains : '',
        type_contains: typeof args.type_contains === 'string' ? args.type_contains : ''
      })
      const snapshot = await saveSnapshot(result.window, result.elements ?? [])
      return render({
        ...result,
        snapshot_id: snapshot,
        snapshot_ttl_ms: 45_000,
        hint:
          '下一步把 snapshot_id 和 element 编号传给 win32-input 的 click/type/scroll；超过 45 秒请重新 observe。'
      })
    }
  )
}