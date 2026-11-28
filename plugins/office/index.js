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
- page_setup {margins?:{top?,bottom?,left?,right?}, orientation?:"portrait"|"landscape", paper?:"a4"|"a3"|"a5"|"b5"|"letter"|"legal", unit?:"cm"|"mm"|"in"}  作用于全部节。
- header_footer {header?, footer?, page_numbers?, alignment?:"left"|"center"|"right"}  写页眉页脚文字；page_numbers 写入「页码 / 总页数」域，不能与 footer 同时用。
- insert_toc {levels?}  已有目录就刷新，否则在文档开头插入目录；之后可用 update_fields 更新。
- section_break {}
- insert_footnote {text, para?, expect?}  省略 para 时在文末插入。
- insert_hyperlink {url, text?}  在文末插入一条超链接段落。
- insert_comment {para, text, author?, expect?}
- set_properties {title?, subject?, author?, keywords?, comments?, category?}
- delete_paragraph {para, expect}  expect 必填，防止误删。
- delete_table {table}
段落号从 office_read 的 paragraphs[].index 取；编辑后重新读取再按编号操作。
格式字段（insert_paragraphs / set_text / format_text 通用，可选）：bold、italic、font_size、font、color、alignment、space_before、space_after（磅）、line_spacing（1 为单倍）、indent_left、indent_first（厘米）、keep_with_next。`

const EXCEL_HELP = `Excel office_edit 操作：
- write_range {sheet?, range:"A1", values:[[...],...], dates?: "auto"|"text"} 字符串以 = 开头时写公式；auto（默认）把 YYYY-MM-DD / YYYY-MM-DD HH:mm 写成真日期。
- fill_formula {sheet?, range:"F2:F500", formula:"=D2*E2", direction?: "down"|"right"}
- format_range {sheet?, range, bold?, italic?, font_size?, font?, number_format?, color?, fill?, wrap?, column_width?, row_height?, alignment?:"left"|"center"|"right"|"general"}
- autofit {sheet?, range?}
- insert_rows {sheet?, row, count?}
- delete_rows {sheet?, row, count?}
- insert_columns {sheet?, col:"B", count?}
- delete_columns {sheet?, col:"B", count?}
- clear_range {sheet?, range, formats?}  formats=true 连同格式一起清除。
- sort {sheet?, range, key?:"C", order?:"asc"|"desc", has_header?}  key 是排序区域内的列字母，默认第一列。
- autofilter {sheet?, range?, clear?}  省略 range 用 UsedRange；clear=true 取消筛选。
- merge_cells {sheet?, range}
- unmerge_cells {sheet?, range}
- conditional_format {sheet?, range, rule:"greater"|"less"|"equal"|"between"|"contains"|"colorscale", value?, value2?, fill?, color?, bold?, min_color?, mid_color?, max_color?}
- validate_list {sheet?, range, values:[...], prompt?}  单元格下拉列表，选项不能含逗号。
- add_comment {sheet?, cell:"B2", text}
- add_hyperlink {sheet?, cell:"B2", url, text?}
- define_name {sheet?, name, range}  同名名称会被覆盖。
- add_sheet {name}
- rename_sheet {sheet, name}
- delete_sheet {sheet}  不能删除唯一工作表。
- add_chart {sheet?, range, type?:"column"|"bar"|"line"|"pie"|"doughnut"|"area"|"radar", title?}
- freeze {sheet?, cell?: "A2"}
- tab_color {sheet?, color:"4472C4"|"none"}
- protect_sheet {sheet?, password?, unprotect?}  省略 password 时无密码保护。
sheet 省略时用当前活动工作表；颜色用 RRGGBB。
日期：office_read 默认把日期格式的单元格读成 yyyy-MM-dd（带时间时补 HH:mm[:ss]），dates:"serial" 返回原始序列号；office_find 也按同样文本匹配。`

const POWERPOINT_HELP = `PowerPoint office_edit 操作：
- add_slide {layout?:"title"|"title_content"|"two_content"|"blank", index?}  默认追加到末尾。
- set_text {slide, placeholder?:"title"|"body"|"subtitle"|编号, shape?, text, bold?, italic?, font_size?, font?, color?, alignment?, bullet?}
- format_text {slide, placeholder?, shape?, bold?, italic?, font_size?, font?, color?, alignment?, bullet?}
- add_textbox {slide, text, left?, top?, width?, height?, font_size?, bold?, color?, alignment?}
- add_bullets {slide, placeholder?, shape?, items:[...], font_size?}  每条一项，自动加项目符号。
- add_table {slide, data:[[...],...], left?, top?, width?, height?, header?}
- add_image {slide, path, left?, top?, width?, height?}  path 必须是本机绝对路径；不给宽高时保留图片原尺寸。
- add_shape {slide, shape?:"rectangle"|"rounded_rectangle"|"diamond"|"oval"|"arrow"|"chevron"|"line", left?, top?, width?, height?, fill?, line?, text?}
- move_slide {slide, to}
- duplicate_slide {slide}
- delete_slide {slide, expect}  expect 必填（该页上应有的一段文字），防止误删。
- set_notes {slide, text}
- set_background {slide, color:"RRGGBB"}
- set_slide_size {width?, height?, unit?:"cm"|"mm"|"in"}  单位缺省厘米，默认 33.87×19.05（16:9）。
- set_layout {slide, layout:"title"|"title_content"|"two_content"|"blank"}
slide 与 shape 编号从 office_read 的 slides[].index / shapes[].index 取；占位符省略时用 body，标题用 placeholder:"title"。
坐标与字号单位是磅：1 厘米 ≈ 28.35 磅；默认页面 960×540 磅。`

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
    '处理 Word/Excel/PowerPoint 时优先使用 office_open、office_read、office_edit、office_save；它们通过本机 Office COM 修改当前打开的文档，改动实时可见。文档较大或只想定位某段内容时先用 office_find，不要整篇读取；按 office_read 的段落号/工作表/幻灯片与形状编号操作，不确定操作名先调用 office_help；编辑后重新读取再继续。'
  )

  api.registerTool(
    {
      name: 'office_open',
      description:
        '在 Word、Excel 或 PowerPoint 中打开一个绝对路径文件；文件不存在时新建（按扩展名判断）。文件已经打开则直接复用。后续用 office_read/office_edit/office_save。',
      parameters: {
        type: 'object',
        properties: {
          path: { type: 'string', description: '绝对路径，例如 C:\\work\\报告.docx、C:\\work\\数据.xlsx 或 C:\\work\\汇报.pptx' },
          app: { type: 'string', enum: ['word', 'excel', 'powerpoint'], description: '可选；通常由扩展名判断' },
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
        '读取已打开的 Word 段落、表格、标题大纲、字数/页数与批注，Excel 工作表/区域的值和公式、各表范围、末行末列、定义名称、图表与表格对象（日期默认读成 yyyy-MM-dd），或 PowerPoint 每页的形状/文字/表格/备注。省略 path 时读上次操作的文档。编辑前先读取。',
      parameters: {
        type: 'object',
        properties: {
          path: { type: 'string', description: '绝对路径；省略时用上次操作的文档' },
          app: { type: 'string', enum: ['word', 'excel', 'powerpoint'], description: '可选；通常由 path 判断' },
          sheet: { type: 'string', description: 'Excel 工作表名，省略用活动表' },
          range: { type: 'string', description: 'Excel 的 A1 区域，省略用 UsedRange' },
          formulas: { type: 'boolean', description: 'Excel 是否同时返回公式，默认 false' },
          dates: {
            type: 'string',
            enum: ['text', 'serial'],
            description: 'Excel 日期单元格的返回形式：text（默认，yyyy-MM-dd[ HH:mm[:ss]]）或 serial（原始序列号）'
          },
          max_paragraphs: { type: 'integer', minimum: 1, maximum: 400, description: 'Word 最多读取段落数，默认 120' },
          max_rows: { type: 'integer', minimum: 1, maximum: 200, description: 'Excel 最多读取行数，默认 60' },
          max_cols: { type: 'integer', minimum: 1, maximum: 40, description: 'Excel 最多读取列数，默认 20' },
          include_tables: { type: 'boolean', description: 'Word 是否读取表格，默认 true' },
          full: { type: 'boolean', description: 'Word 是否返回完整段落文字，PowerPoint 是否返回完整形状文字，默认 true' },
          max_slides: { type: 'integer', minimum: 1, maximum: 200, description: 'PowerPoint 最多读取页数，默认 60' },
          max_shapes: { type: 'integer', minimum: 1, maximum: 60, description: 'PowerPoint 每页最多读取形状数，默认 30' },
          include_notes: { type: 'boolean', description: 'PowerPoint 是否读取备注，默认 true' }
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
            dates: args.dates,
            max_paragraphs: args.max_paragraphs,
            max_rows: args.max_rows,
            max_cols: args.max_cols,
            include_tables: args.include_tables !== false,
            full: args.full !== false,
            max_slides: args.max_slides,
            max_shapes: args.max_shapes,
            include_notes: args.include_notes !== false
          },
          120_000
        )
      )
    }
  )
api.registerTool(
    {
      name: 'office_find',
      description:
        '在已打开的 Word 文档、Excel 工作簿或 PowerPoint 演示文稿里搜索文字，返回段落号、工作表!单元格地址或幻灯片/形状与命中片段；用于大文档定位，不必先整篇 office_read。',
      parameters: {
        type: 'object',
        properties: {
          query: { type: 'string', description: '要查找的文字；regex=true 时是正则' },
          path: { type: 'string', description: '绝对路径；省略时用上次操作的文档' },
          app: { type: 'string', enum: ['word', 'excel', 'powerpoint'], description: '可选；通常由 path 判断' },
          sheet: { type: 'string', description: 'Excel 只搜索这张工作表；省略则搜索全部' },
          regex: { type: 'boolean', description: '按正则匹配，默认 false' },
          match_case: { type: 'boolean', description: '区分大小写，默认 false' },
          max_hits: { type: 'integer', minimum: 1, maximum: 200, description: '最多返回多少条命中，默认 50' },
          max_rows: { type: 'integer', minimum: 1, maximum: 2000, description: 'Excel 每张表最多扫描行数，默认 200' },
          max_cols: { type: 'integer', minimum: 1, maximum: 100, description: 'Excel 每张表最多扫描列数，默认 40' },
          max_slides: { type: 'integer', minimum: 1, maximum: 200, description: 'PowerPoint 最多搜索页数，默认 100' }
        },
        required: ['query'],
        additionalProperties: false
      }
    },
    async (args) => {
      const query = String(args.query ?? '').trim()
      if (query === '') throw new Error('缺少 query')
      const path = absoluteOrOmit(args.path)
      return render(
        await callBridge(
          {
            action: 'find',
            query,
            path,
            app: args.app,
            sheet: args.sheet,
            regex: args.regex === true,
            match_case: args.match_case === true,
            max_hits: args.max_hits,
            max_rows: args.max_rows,
            max_cols: args.max_cols,
            max_slides: args.max_slides
          },
          240_000
        )
      )
    }
  )

  api.registerTool(
    {
      name: 'office_edit',
      description:
        '在打开的 Word/Excel/PowerPoint 文档上按顺序执行修改，不自动保存。ops 是操作数组，操作名与字段先调用 office_help 查看；Word 支持段落、表格、格式与页面设置，Excel 支持单元格、排序筛选、条件格式与图表，PowerPoint 支持页、占位符文字、文本框、表格、图片与形状。',
      parameters: {
        type: 'object',
        properties: {
          path: { type: 'string', description: '绝对路径；省略时用上次操作的文档' },
          app: { type: 'string', enum: ['word', 'excel', 'powerpoint'], description: '可选；通常由 path 判断' },
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
        '保存当前文档（Word/Excel/PowerPoint）；省略 path 时原地保存。给 path 时另存为，扩展名 .pdf 导出 PDF 且不改变当前文档路径。编辑后确认内容再保存。',
      parameters: {
        type: 'object',
        properties: {
          path: { type: 'string', description: '绝对路径；省略时原地保存' },
          app: { type: 'string', enum: ['word', 'excel', 'powerpoint'], description: '可选；通常由上次操作判断' },
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
        '关闭本插件打开或创建的 Word/Excel/PowerPoint 文档。用户原本打开的文档默认拒绝关闭；有未保存修改时必须明确 save=true 或 save=false。',
      parameters: {
        type: 'object',
        properties: {
          path: { type: 'string', description: '绝对路径；省略时用上次操作的文档' },
          app: { type: 'string', enum: ['word', 'excel', 'powerpoint'], description: '可选' },
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
      description: '列出 office_edit 在 Word、Excel 或 PowerPoint 中支持的全部操作和字段。不确定操作名时先调用。',
      parameters: {
        type: 'object',
        properties: {
          app: { type: 'string', enum: ['word', 'excel', 'powerpoint'], description: '要查看的应用' }
        },
        required: ['app'],
        additionalProperties: false
      }
    },
    async (args) => {
      const app = String(args.app ?? '')
      const help = app === 'word' ? WORD_HELP : app === 'excel' ? EXCEL_HELP : app === 'powerpoint' ? POWERPOINT_HELP : ''
      if (help === '') throw new Error('app 必须是 word、excel 或 powerpoint')
      return render({ ok: true, app, help })
    }
  )
}