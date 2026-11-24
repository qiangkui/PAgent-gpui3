import { execFile } from 'node:child_process'
import { promisify } from 'node:util'
import { dirname, isAbsolute, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { mkdir, unlink, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { randomUUID } from 'node:crypto'

const run = promisify(execFile)
const here = dirname(fileURLToPath(import.meta.url))
const bridge = join(here, 'office_bridge.ps1')
const stateDir = join(tmpdir(), 'paagent-office')
const statePath = join(stateDir, 'state.json')

const WORD_HELP = `Word office_edit 操作：
- insert_paragraphs {items:[{text, style?, bold?, italic?, font_size?, font?, color?, alignment?}], where?: "end"(默认)|"start"|"before"|"after", para?, expect?}
  text 中的换行会拆成多个段落；style 可为 Normal/Title/Heading 1..6/List Bullet/List Number/Quote。
- set_text {para, expect, text, bold?, italic?, font_size?, alignment?} 按段落号改写，expect 是改写前开头文字，用于防止位置过期。
- replace_text {find, replace, all?} 替换正文文字；all 默认 true。
- insert_table {data:[[...],...], header?: true, borders?: "grid"|"none", caption?}
- set_cell {table, row, col, text}
- format_text {para, expect?, bold?, italic?, font_size?, font?, color?, alignment?}
- format_table {table, header?, bold?, font_size?, borders?}
- insert_image {path, width?, height?, caption?}  path 必须是本机绝对路径。
- page_break {}
- update_fields {}
段落号从 office_read 的 paragraphs[].index 取；编辑后重新读取再按编号操作。`

const EXCEL_HELP = `Excel office_edit 操作：
- write_range {sheet?, range:"A1", values:[[...],...]} 字符串以 = 开头时写公式。
- fill_formula {sheet?, range:"F2:F500", formula:"=D2*E2", direction?: "down"|"right"}
- format_range {sheet?, range, bold?, italic?, font_size?, font?, number_format?, color?, fill?, wrap?, column_width?, row_height?, alignment?:"left"|"center"|"right"|"general"}
- autofit {sheet?, range?}
- insert_rows {sheet?, row, count?}
- delete_rows {sheet?, row, count?}
- add_sheet {name}
- rename_sheet {sheet, name}
- delete_sheet {sheet}  不能删除唯一工作表。
- add_chart {sheet?, range, type?:"column"|"bar"|"line"|"pie"|"doughnut"|"area"|"radar", title?}
- freeze {sheet?, cell?: "A2"}
sheet 省略时用当前活动工作表；颜色用 RRGGBB。`

async function callBridge(payload, timeout = 180_000) {
  await mkdir(stateDir, { recursive: true })
  const requestFile = join(stateDir, `request-${randomUUID()}.json`)
  const request = { ...payload, state_path: statePath }
  await writeFile(requestFile, JSON.stringify(request), 'utf8')
  try {
    const { stdout } = await run(
      'powershell.exe',
      [
        '-NoProfile',
        '-NonInteractive',
        '-ExecutionPolicy', 'Bypass',
        '-File', bridge,
        requestFile
      ],
      {
        cwd: here,
        windowsHide: true,
        timeout,
        maxBuffer: 32 * 1024 * 1024,
        encoding: 'utf8'
      }
    )
    const text = stdout.trim()
    if (text === '') throw new Error('Office 脚本没有返回内容')
    const parsed = JSON.parse(text)
    if (!parsed || parsed.ok !== true) throw new Error(String(parsed?.error ?? 'Office 操作失败'))
    return parsed
  } finally {
    await unlink(requestFile).catch(() => {})
  }
}

function render(result) {
  return JSON.stringify(result, null, 2)
}

function absoluteOrOmit(value) {
  if (value === undefined || value === null || String(value).trim() === '') return undefined
  const text = String(value).trim()
  if (!isAbsolute(text)) throw new Error('path 必须是绝对路径')
  return text
}
export function activate(api) {
  api.injectPrompt(
    '处理 Word/Excel 时优先使用 office_open、office_read、office_edit、office_save；它们通过本机 Office COM 修改当前打开的文档，改动实时可见。按 office_read 的段落号/工作表操作，不确定操作名先调用 office_help；编辑后重新读取再继续。'
  )

  api.registerTool(
    {
      name: 'office_open',
      description:
        '在 Word 或 Excel 中打开一个绝对路径文件；文件不存在时新建（按扩展名判断 Word/Excel）。文件已经打开则直接复用。后续用 office_read/office_edit/office_save。',
      parameters: {
        type: 'object',
        properties: {
          path: { type: 'string', description: '绝对路径，例如 C:\\work\\报告.docx 或 C:\\work\\数据.xlsx' },
          app: { type: 'string', enum: ['word', 'excel'], description: '可选；通常由扩展名判断' },
          visible: { type: 'boolean', description: '是否显示 Office 窗口，默认 true' },
          dry_run: { type: 'boolean', description: '只检查路径与应用，不打开文件' }
        },
        required: ['path'],
        additionalProperties: false
      }
    },
    async (args) => {
      const path = absoluteOrOmit(args.path)
      if (!path) throw new Error('缺少 path')
      return render(
        await callBridge(
          { action: 'open', path, app: args.app, visible: args.visible !== false, dry_run: args.dry_run === true },
          120_000
        )
      )
    }
  )

  api.registerTool(
    {
      name: 'office_status',
      description: '列出当前 Word/Excel 已打开的文档、路径、是否保存，以及是否由本插件打开，便于继续或关闭。',
      parameters: { type: 'object', properties: {}, additionalProperties: false }
    },
    async () => render(await callBridge({ action: 'status' }, 60_000))
  )

  api.registerTool(
    {
      name: 'office_read',
      description:
        '读取已打开的 Word 段落与表格，或 Excel 工作表/区域的值和公式；读取的是当前未保存内容。省略 path 时读上次操作的文档。编辑前先读取。',
      parameters: {
        type: 'object',
        properties: {
          path: { type: 'string', description: '绝对路径；省略时用上次操作的文档' },
          app: { type: 'string', enum: ['word', 'excel'], description: '可选；通常由 path 判断' },
          sheet: { type: 'string', description: 'Excel 工作表名，省略用活动表' },
          range: { type: 'string', description: 'Excel 的 A1 区域，省略用 UsedRange' },
          formulas: { type: 'boolean', description: 'Excel 是否同时返回公式，默认 false' },
          max_paragraphs: { type: 'integer', minimum: 1, maximum: 400, description: 'Word 最多读取段落数，默认 120' },
          max_rows: { type: 'integer', minimum: 1, maximum: 200, description: 'Excel 最多读取行数，默认 60' },
          max_cols: { type: 'integer', minimum: 1, maximum: 40, description: 'Excel 最多读取列数，默认 20' },
          include_tables: { type: 'boolean', description: 'Word 是否读取表格，默认 true' },
          full: { type: 'boolean', description: 'Word 是否返回完整段落文字，默认 true' }
        },
        additionalProperties: false
      }
    },
    async (args) => {
      const path = absoluteOrOmit(args.path)
      return render(
        await callBridge(
          {
            action: 'read',
            path,
            app: args.app,
            sheet: args.sheet,
            range: args.range,
            formulas: args.formulas === true,
            max_paragraphs: args.max_paragraphs,
            max_rows: args.max_rows,
            max_cols: args.max_cols,
            include_tables: args.include_tables !== false,
            full: args.full !== false
          },
          120_000
        )
      )
    }
  )
api.registerTool(
    {
      name: 'office_edit',
      description:
        '在打开的 Word/Excel 文档上按顺序执行修改，不自动保存。ops 是操作数组，操作名与字段先调用 office_help 查看；支持段落、表格、单元格、格式、工作表与图表。',
      parameters: {
        type: 'object',
        properties: {
          path: { type: 'string', description: '绝对路径；省略时用上次操作的文档' },
          app: { type: 'string', enum: ['word', 'excel'], description: '可选；通常由 path 判断' },
          ops: {
            type: 'array',
            minItems: 1,
            maxItems: 100,
            description: '按顺序执行的操作，例如 [{"op":"set_text","para":3,"expect":"旧开头","text":"新文本"}]',
            items: {
              type: 'object',
              additionalProperties: true,
              properties: { op: { type: 'string', description: '操作名' } }
            }
          },
          dry_run: { type: 'boolean', description: '只验证文档和操作数量，不修改' }
        },
        required: ['ops'],
        additionalProperties: false
      }
    },
    async (args) => {
      if (!Array.isArray(args.ops) || args.ops.length === 0) throw new Error('ops 不能为空')
      if (args.ops.length > 100) throw new Error('一次最多 100 个操作')
      const path = absoluteOrOmit(args.path)
      return render(
        await callBridge(
          { action: 'edit', path, app: args.app, ops: args.ops, dry_run: args.dry_run === true },
          300_000
        )
      )
    }
  )

  api.registerTool(
    {
      name: 'office_save',
      description:
        '保存当前 Word/Excel 文档；省略 path 时原地保存。给 path 时另存为，扩展名 .pdf 导出 PDF 且不改变当前文档路径。编辑后确认内容再保存。',
      parameters: {
        type: 'object',
        properties: {
          path: { type: 'string', description: '绝对路径；省略时原地保存' },
          app: { type: 'string', enum: ['word', 'excel'], description: '可选；通常由上次操作判断' },
          dry_run: { type: 'boolean', description: '只检查文档，不保存' }
        },
        additionalProperties: false
      }
    },
    async (args) => {
      const path = absoluteOrOmit(args.path)
      return render(
        await callBridge({ action: 'save', path, app: args.app, dry_run: args.dry_run === true }, 180_000)
      )
    }
  )

  api.registerTool(
    {
      name: 'office_close',
      description:
        '关闭本插件打开或创建的 Word/Excel 文档。用户原本打开的文档默认拒绝关闭；有未保存修改时必须明确 save=true 或 save=false。',
      parameters: {
        type: 'object',
        properties: {
          path: { type: 'string', description: '绝对路径；省略时用上次操作的文档' },
          app: { type: 'string', enum: ['word', 'excel'], description: '可选' },
          save: { type: 'boolean', description: '关闭前是否保存；有未保存修改时必须提供' },
          allow_user_doc: { type: 'boolean', description: '允许关闭不是本插件打开的文档，默认 false' }
        },
        additionalProperties: false
      }
    },
    async (args) => {
      const path = absoluteOrOmit(args.path)
      return render(
        await callBridge(
          {
            action: 'close',
            path,
            app: args.app,
            save: args.save,
            allow_user_doc: args.allow_user_doc === true
          },
          120_000
        )
      )
    }
  )

  api.registerTool(
    {
      name: 'office_help',
      description: '列出 office_edit 在 Word 或 Excel 中支持的全部操作和字段。不确定操作名时先调用。',
      parameters: {
        type: 'object',
        properties: {
          app: { type: 'string', enum: ['word', 'excel'], description: '要查看的应用' }
        },
        required: ['app'],
        additionalProperties: false
      }
    },
    async (args) => {
      const app = String(args.app ?? '')
      const help = app === 'word' ? WORD_HELP : app === 'excel' ? EXCEL_HELP : ''
      if (help === '') throw new Error('app 必须是 word 或 excel')
      return render({ ok: true, app, help })
    }
  )
}