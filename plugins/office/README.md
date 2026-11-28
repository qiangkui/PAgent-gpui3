# Office 文档操作

让模型直接操作本机已经打开的 Word / Excel：通过 Office COM 修改当前文档，改动实时显示在窗口中，不走“重新生成 docx/xlsx 文件”的路线。思路参考 `copylee711/dsh-office`，并借用 DSH 内置 Office 技能对“真实应用优先”的取舍。

## 支持环境

- Windows 10/11；
- Microsoft Word / Excel / PowerPoint（推荐），或 WPS 文字/表格/演示（尽力兼容）；
- 插件页启用 `office`，每次工具调用都走 PAgent 的审批卡片；
- 不需要额外 Python 包或 npm 依赖，脚本使用 Windows PowerShell 的 COM。

## 工具

| 工具 | 作用 |
|---|---|
| `office_open` | 打开绝对路径文件（Word / Excel / PowerPoint）；文件不存在时新建；已打开则复用 |
| `office_status` | 列出当前 Word/Excel/PowerPoint 文档、路径、是否已保存、是否由本插件打开 |
| `office_read` | 读取 Word 段落/表格/标题大纲/字数页数/批注/文档属性，Excel 区域值与公式、各表范围、末行末列、名称、图表与表格对象，或 PowerPoint 每页的形状/文字/表格/备注 |
| `office_find` | 在 Word 段落、Excel 单元格或 PowerPoint 幻灯片形状/备注里搜索文字（支持正则），返回定位信息与命中片段 |
| `office_edit` | 批量执行 Word/Excel/PowerPoint 修改，不自动保存 |
| `office_save` | 原地保存、另存为，或导出 PDF |
| `office_close` | 关闭本插件打开/创建的文档；用户原本打开的文档默认拒绝 |
| `office_help` | 查看 Word/Excel/PowerPoint 支持的 `office_edit` 操作和字段 |

## 推荐流程

```text
office_open → office_find / office_read → office_edit → office_read 验证 → office_save
```

- `office_open` 和 `office_edit` 的路径必须是绝对路径；
- 文档大时先用 `office_find` 定位，再按段落号/区域读取，不要整篇读；
- `office_edit` 不会自动保存，确认内容后调用 `office_save`；
- 修改现有文档前先 `office_read` 取段落号、表格号或工作表/区域；
- 不确定操作名和字段时先 `office_help`，不要猜；
- 删除/覆盖/关闭等有风险的动作仍需用户审批。

## Word 操作

`office_edit` 支持：

- `insert_paragraphs`：追加段落，支持 Title/Heading 1..6/List Bullet 等样式、粗体、斜体、字号、字体、颜色、对齐；
- `set_text`：按段落号改写，`expect` 校验段落开头，避免文档变化后改错位置；
- `replace_text`：查找替换；
- `insert_table` / `set_cell` / `format_table`：插入和修改表格；
- `format_text`：修改某段格式；
- `insert_image`：插入图片（绝对路径）；
- `page_break`、`update_fields`；
- `page_setup`：页边距、纸张（A3/A4/A5/B5/Letter/Legal）、方向，作用于全部节；
- `header_footer`：页眉/页脚文字，或 `page_numbers` 写入「页码 / 总页数」域；
- `insert_toc`：插入或刷新目录（配合 `update_fields`）；
- `section_break`、`insert_footnote`、`insert_hyperlink`、`insert_comment`；
- `set_properties`：标题/主题/作者/关键词/备注/类别；
- `delete_paragraph`（必须带 `expect`）、`delete_table`。

格式字段（`insert_paragraphs` / `set_text` / `format_text` 通用）：粗体、斜体、字号、字体、颜色、对齐，以及 `space_before` / `space_after`（磅）、`line_spacing`（倍数）、`indent_left` / `indent_first`（厘米）、`keep_with_next`。

段落号来自 `office_read` 的 `paragraphs[].index`，表格号来自 `tables[].index`。

## Excel 操作

`office_edit` 支持：

- `write_range`：二维值/公式写入，字符串以 `=` 开头写公式；`dates` 默认 `auto`，把 `YYYY-MM-DD` / `YYYY-MM-DD HH:MM[:SS]` 写成**真日期**（可排序、可参与公式），`dates: "text"` 则先设为文本格式再写入，Excel 不会自动转成日期；
- `fill_formula`：把公式填满一列/一行；
- `format_range`：粗体、斜体、字号、字体、数字格式、字体色、填充色、换行、列宽、行高、对齐；
- `autofit`；
- `insert_rows` / `delete_rows` / `insert_columns` / `delete_columns`；
- `clear_range`：清除内容，`formats=true` 连格式一起清；
- `sort`：按区域内列字母排序（升/降序，可声明是否有表头）；
- `autofilter`：开启或取消筛选；
- `merge_cells` / `unmerge_cells`；
- `conditional_format`：大于/小于/等于/介于/包含某文字的条件格式，以及三色色阶；
- `validate_list`：单元格下拉列表；
- `add_comment` / `add_hyperlink`；
- `define_name`：定义名称（同名覆盖）；
- `add_sheet` / `rename_sheet` / `delete_sheet`；
- `add_chart`：柱状、条形、折线、饼图、圆环、面积、雷达；
- `freeze`；
- `tab_color`：工作表标签色；
- `protect_sheet`：保护/取消保护工作表。

工作表省略时使用当前活动工作表；颜色使用 `RRGGBB`。

## PowerPoint 操作

`office_edit` 支持：

- `add_slide`：按版式（title / title_content / two_content / blank）新建页，可指定插入位置；
- `set_text` / `format_text`：按占位符（`title` / `body` / `subtitle` / 编号）或形状编号改写文字与格式（粗体、斜体、字号、字体、颜色、对齐、项目符号）；
- `add_textbox`：任意位置的文本框；
- `add_bullets`：一次写入多条项目符号；
- `add_table`：插入表格（表头加粗）；
- `add_image`：插入图片（绝对路径），不给宽高时保留原尺寸；
- `add_shape`：矩形 / 圆角矩形 / 菱形 / 椭圆 / 箭头 / V 形 / 直线，可设填充、边框、文字；
- `move_slide` / `duplicate_slide` / `delete_slide`（`expect` 必填）/ `set_layout`；
- `set_notes`：写备注；
- `set_background`：页背景色；
- `set_slide_size`：页面尺寸（厘米/毫米/英寸，默认 33.87×19.05 = 16:9）。

坐标、字号、页面尺寸的单位都是**磅**（1 厘米 ≈ 28.35 磅）；页号与形状号来自 `office_read` 的 `slides[].index` 与 `shapes[].index`。

## 读取回来的内容

Word：`paragraphs[].index/text/style/outline`、`headings[]`（标题大纲）、`tables[]`、`words`、`pages`、`sections`、`inline_shapes`、`comment_count`/`comments[]`（作者、内容、所指文字）、`properties`（标题/主题/作者/关键词/类别）。

Excel：`values`/`formulas`（日期默认是 `yyyy-MM-dd HH:mm[:ss]` 文本，`dates:"serial"` 可拿原始序列号）、`sheets`、`sheet_info[]`（每张表的范围、行列数、是否隐藏）、`used_range`、`last_row`/`last_column`、`names[]`（用户定义名称）、`charts[]`、`tables[]`（ListObject）、`date1904`。

PowerPoint：`slides[]`（每页 `layout`、`shapes[]`、`shapes_total`、`notes`）、`slide_count`、`slide_width`/`slide_height`（磅）、`properties`；每个形状带 `index`、`name`、`type`、`placeholder`、`text`、位置尺寸，表格形状另带 `table.data`。

## 日期

Excel 内部把日期存成序列号（`2026-01-05` = 46027），插件在三个环节统一处理：

- **读**：`office_read` 默认把日期格式的单元格返回成 `yyyy-MM-dd`，带时间的补 `HH:mm`（秒为 0 时省略）或 `HH:mm:ss`；`dates: "serial"` 返回原始序列号，`date1904` 会说明工作簿用的是 1904 日期系统。判断依据是单元格自己的 `NumberFormat`（`0.00`、`General`、`#,##0.00_);[Red](#,##0.00)` 这类仍是数字，`yyyy"年"m"月"d"日"`、`[$-804]yyyy/m/d h:mm` 才算日期）。
- **写**：`write_range` 遇到 `YYYY-MM-DD`、`YYYY/MM/DD`（可带 `HH:MM[:SS]`）的字符串就写成真日期并设好数字格式；`dates: "text"` 则把单元格先设成文本格式再写入，避免 Excel 自行解析。`1/2/2026`、`2026-1-5` 这种含糊写法一律按文本处理。
- **搜**：`office_find` 用同样的显示文本匹配，所以 `query: "2026-01"` 能命中日期格；命中片段里显示的也是 `2026-01-05`，不是 46027。

读取和搜索都逐格判断格式（整块 `Range.NumberFormat` 在 PowerShell 5.1 上会返回 `DBNull`）：只有数值且落在序列号范围内的单元格才会多读一次格式，其它单元格不多付这次 COM 调用。`office_find` 的扫描量受 `max_rows` / `max_cols` 限制，超出时结果里 `truncated` 为 true。

## 安全边界

- 只在本机操作，不联网；
- 插件不会把文档上传到任何地方；
- `office_find` 只读，不修改文档；
- `office_close` 只关闭本插件打开/创建的文档；用户原本打开的文档需要显式 `allow_user_doc=true`；
- 有未保存修改时必须明确 `save: true` 或 `save: false`；
- `delete_paragraph` 必须同时给 `expect`（段落开头文字），`delete_slide` 必须给 `expect`（该页上的一段文字）；`set_text` / `format_text` / `insert_comment` 的 `expect` 可选但建议提供；
- 支持 `dry_run` 做路径/文档/操作数量检查；
- 文档所有权写在 `%TEMP%\paagent-office\state.json`，不写回文档内容。

## 与 dsh-office 的差距

当前是面向 PAgent 的核心子集：

- 只支持 Word / Excel / PowerPoint 三件套，不含 Outlook / Visio；
- 不含虚拟光标、迷你卡片、逐字写入、整页渲染检查；
- 不含 Word 原生公式、交叉引用、样式批量管理；
- Excel 图表使用 Office 原生图表，但没有数据透视表、切片器；
- 不含 Excel 工作表复制/移动、分组大纲、页面设置；
- PowerPoint 不含图表/SmartArt/动画/切换、幻灯片母版与版式编辑，也不做屏幕放映；
- `office_read` 限制一次返回的数据量，超长工作表请指定 `range` 或分页读取；
- WPS 兼容取决于本机 ProgID 和对象模型，未做微软 Office 同等级验收。

## 实现说明

- 桥接脚本 `office_bridge.ps1` **必须保留 UTF-8 BOM**：Windows PowerShell 5.1 对无 BOM 文件按 ANSI 解析，中文提示和正则会被截断；
- Word 的 `BuiltInDocumentProperties` 经 PowerShell 适配层会抛空引用，脚本改用原始 IDispatch（`[System.__ComObject]::InvokeMember`）读写文档属性；
- Excel 的 `Range` 是可枚举对象，`$x = if (...) { $range }` 会把它展开成单元格数组，脚本一律用显式 `if/else` 赋值；
- Excel 整块 range 的 `Range.NumberFormat` / `Range.Text` 经 PowerShell 5.1 会变成 `DBNull`（`Value2` 不会），所以日期判定只能逐格读取格式；日期换算本身在 `office_dates.ps1`，可被测试单独加载；
- PowerPoint 的所有布尔值都是 msoTrue/msoFalse（-1/0），`Mso-Bool` 负责转换；备注页上每个形状的 `Type` 都是 14，备注正文要用 `PlaceholderFormat.Type = 2` 找；`Application.Visible` 只能为真，`visible:false` 通过 `Presentations.Open(..., WithWindow:=0)` 实现无窗口；
- `dsh-flow/tests/office-plugin.test.ts` 会校验 `office_help` 与两个 `Run-*-Op` 的操作清单一致、每个工具都有桥接 action、`plugin.json` 有效、脚本带 BOM——改操作时记得同步帮助文本。