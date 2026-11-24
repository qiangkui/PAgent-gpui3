# Office 文档操作

让模型直接操作本机已经打开的 Word / Excel：通过 Office COM 修改当前文档，改动实时显示在窗口中，不走“重新生成 docx/xlsx 文件”的路线。思路参考 `copylee711/dsh-office`，并借用 DSH 内置 Office 技能对“真实应用优先”的取舍。

## 支持环境

- Windows 10/11；
- Microsoft Word / Excel（推荐），或 WPS 文字/表格（尽力兼容）；
- 插件页启用 `office`，每次工具调用都走 PAgent 的审批卡片；
- 不需要额外 Python 包或 npm 依赖，脚本使用 Windows PowerShell 的 COM。

## 工具

| 工具 | 作用 |
|---|---|
| `office_open` | 打开绝对路径文件；文件不存在时新建；已打开则复用 |
| `office_status` | 列出当前 Word/Excel 文档、路径、是否已保存、是否由本插件打开 |
| `office_read` | 读取 Word 段落/表格，或 Excel 工作表/区域的值与公式 |
| `office_edit` | 批量执行 Word/Excel 修改，不自动保存 |
| `office_save` | 原地保存、另存为，或导出 PDF |
| `office_close` | 关闭本插件打开/创建的文档；用户原本打开的文档默认拒绝 |
| `office_help` | 查看 Word/Excel 支持的 `office_edit` 操作和字段 |

## 推荐流程

```text
office_open → office_read → office_edit → office_read 验证 → office_save
```

- `office_open` 和 `office_edit` 的路径必须是绝对路径；
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
- `page_break`、`update_fields`。

段落号来自 `office_read` 的 `paragraphs[].index`，表格号来自 `tables[].index`。

## Excel 操作

`office_edit` 支持：

- `write_range`：二维值/公式写入，字符串以 `=` 开头写公式；
- `fill_formula`：把公式填满一列/一行；
- `format_range`：粗体、斜体、字号、字体、数字格式、字体色、填充色、换行、列宽、行高、对齐；
- `autofit`；
- `insert_rows` / `delete_rows`；
- `add_sheet` / `rename_sheet` / `delete_sheet`；
- `add_chart`：柱状、条形、折线、饼图、圆环、面积、雷达；
- `freeze`。

工作表省略时使用当前活动工作表；颜色使用 `RRGGBB`。

## 安全边界

- 只在本机操作，不联网；
- 插件不会把文档上传到任何地方；
- `office_close` 只关闭本插件打开/创建的文档；用户原本打开的文档需要显式 `allow_user_doc=true`；
- 有未保存修改时必须明确 `save: true` 或 `save: false`；
- 支持 `dry_run` 做路径/文档/操作数量检查；
- 文档所有权写在 `%TEMP%\paagent-office\state.json`，不写回文档内容。

## 与 dsh-office 的差距

当前是面向 PAgent 的核心子集：

- 只支持 Word / Excel，不含 PowerPoint；
- 不含虚拟光标、迷你卡片、逐字写入、整页渲染检查；
- 不含 Word 原生公式/脚注/目录/页面设置等高级排版操作；
- Excel 图表使用 Office 原生图表，但没有数据透视表、条件格式等高级分析操作；
- `office_read` 限制一次返回的数据量，超长工作表请指定 `range` 或分页读取；
- WPS 兼容取决于本机 ProgID 和对象模型，未做微软 Office 同等级验收。