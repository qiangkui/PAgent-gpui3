import { execFile } from 'node:child_process'
import { promisify } from 'node:util'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const run = promisify(execFile)
const here = dirname(fileURLToPath(import.meta.url))
const script = join(here, 'win32_windows.py')

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
}