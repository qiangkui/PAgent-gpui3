param([string]$RequestFile = '')

$ErrorActionPreference = 'Stop'
try {
    [Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
} catch {}

function Read-Request {
    if ($RequestFile -eq '' -or -not (Test-Path -LiteralPath $RequestFile)) {
        throw 'missing request file'
    }
    $text = Get-Content -LiteralPath $RequestFile -Raw -Encoding UTF8
    return $text | ConvertFrom-Json
}

function Full-Path([string]$value) {
    return [System.IO.Path]::GetFullPath($value)
}

function Path-Key([string]$kind, [string]$path) {
    return ($kind + '|' + $path.ToLowerInvariant())
}

function Read-State([string]$path) {
    $state = @{ last = $null; owned = @() }
    if ($path -ne '' -and (Test-Path -LiteralPath $path)) {
        try {
            $value = Get-Content -LiteralPath $path -Raw -Encoding UTF8 | ConvertFrom-Json
            if ($null -ne $value.last) { $state.last = $value.last }
            if ($null -ne $value.owned) { $state.owned = @($value.owned) }
        } catch {}
    }
    return $state
}

function Write-State([string]$path, $state) {
    if ($path -eq '') { return }
    $folder = [System.IO.Path]::GetDirectoryName($path)
    if ($folder -ne '' -and -not (Test-Path -LiteralPath $folder)) {
        [void][System.IO.Directory]::CreateDirectory($folder)
    }
    $state | ConvertTo-Json -Depth 6 -Compress | Set-Content -LiteralPath $path -Encoding UTF8
}

function Mark-Owned($state, [string]$kind, [string]$path) {
    $key = Path-Key $kind $path
    if (-not ($state.owned -contains $key)) { $state.owned = @($state.owned) + $key }
}

function Is-Owned($state, [string]$kind, [string]$path) {
    return ($state.owned -contains (Path-Key $kind $path))
}

function Forget-Owned($state, [string]$kind, [string]$path) {
    $key = Path-Key $kind $path
    $state.owned = @($state.owned | Where-Object { $_ -ne $key })
}

function Remember-Last($state, [string]$kind, [string]$path) {
    $state.last = @{ app = $kind; path = $path }
}

function App-From-Path([string]$path) {
    $ext = [System.IO.Path]::GetExtension($path).ToLowerInvariant()
    if ($ext -in @('.docx', '.doc', '.docm')) { return 'word' }
    if ($ext -in @('.xlsx', '.xls', '.xlsm', '.xlsb')) { return 'excel' }
    throw "不支持的文件类型：$ext（只支持 Word .docx/.doc 与 Excel .xlsx/.xls/.xlsm）"
}

function App-Of($request, $state) {
    $kind = ''
    if ($null -ne $request.app) { $kind = ([string]$request.app).ToLowerInvariant() }
    $path = ''
    if ($null -ne $request.path -and [string]$request.path -ne '') { $path = Full-Path ([string]$request.path) }
    if ($kind -eq '' -and $path -ne '') { $kind = App-From-Path $path }
    if ($kind -eq '' -and $null -ne $state.last) { $kind = [string]$state.last.app }
    if ($kind -notin @('word', 'excel')) { throw '缺少 app 或 path，且没有上次操作的文档' }
    if ($path -eq '' -and $null -ne $state.last -and [string]$state.last.app -eq $kind) {
        $path = [string]$state.last.path
    }
    if ($path -ne '') {
        $inferred = App-From-Path $path
        if ($inferred -ne $kind) { throw "文件 $path 属于 $inferred，不是 $kind" }
    }
    return @{ app = $kind; path = $path }
}

function Get-RunningApp([string]$kind) {
    $ids = if ($kind -eq 'word') { @('Word.Application', 'Kwps.Application') } else { @('Excel.Application', 'Ket.Application') }
    foreach ($id in $ids) {
        try {
            $app = [System.Runtime.InteropServices.Marshal]::GetActiveObject($id)
            if ($null -ne $app) { return $app }
        } catch {}
    }
    return $null
}

function Get-App([string]$kind, [bool]$visible) {
    $app = Get-RunningApp $kind
    $attached = $null -ne $app
    if ($null -eq $app) {
        $ids = if ($kind -eq 'word') { @('Word.Application', 'Kwps.Application') } else { @('Excel.Application', 'Ket.Application') }
        foreach ($id in $ids) {
            try { $app = New-Object -ComObject $id } catch {}
            if ($null -ne $app) { break }
        }
        if ($null -eq $app) {
            throw "无法启动 $kind（请确认已安装 Microsoft Office 或 WPS）"
        }
    }
    try {
        if ($kind -eq 'word') {
            $app.Visible = $visible
            $app.DisplayAlerts = 0
        } else {
            $app.Visible = $visible
            $app.DisplayAlerts = $false
        }
    } catch {}
    return @{ app = $app; attached = $attached }
}
function Find-Doc($app, [string]$kind, [string]$path) {
    $wanted = $path.ToLowerInvariant()
    if ($kind -eq 'word') {
        for ($i = 1; $i -le $app.Documents.Count; $i++) {
            try {
                $doc = $app.Documents.Item($i)
                if ($null -ne $doc.FullName -and (Full-Path $doc.FullName).ToLowerInvariant() -eq $wanted) { return $doc }
            } catch {}
        }
    } else {
        for ($i = 1; $i -le $app.Workbooks.Count; $i++) {
            try {
                $book = $app.Workbooks.Item($i)
                if ($null -ne $book.FullName -and (Full-Path $book.FullName).ToLowerInvariant() -eq $wanted) { return $book }
            } catch {}
        }
    }
    return $null
}

function Doc-Info($doc, [string]$kind, [string]$path, [string]$how, $attached) {
    $saved = $false
    try { $saved = [bool]$doc.Saved } catch {}
    $name = ''
    try { $name = [string]$doc.Name } catch {}
    return @{
        app = $kind
        path = $path
        name = $name
        saved = $saved
        how = $how
        attached = [bool]$attached
    }
}

function Open-Document($request, $state) {
    $kind = ''
    if ($null -ne $request.app) { $kind = ([string]$request.app).ToLowerInvariant() }
    $rawPath = ''
    if ($null -ne $request.path -and [string]$request.path -ne '') { $rawPath = [string]$request.path }
    if ($rawPath -eq '') { throw 'office_open 需要绝对文件路径' }
    $path = Full-Path $rawPath
    if ($kind -eq '') { $kind = App-From-Path $path }
    if ($kind -notin @('word', 'excel')) { throw '只支持 Word 与 Excel' }
    $parent = [System.IO.Path]::GetDirectoryName($path)
    if ($parent -ne '' -and -not (Test-Path -LiteralPath $parent)) {
        [void][System.IO.Directory]::CreateDirectory($parent)
    }
    $visible = $true
    if ($request.PSObject.Properties.Name -contains 'visible') { $visible = [bool]$request.visible }
    if ($request.PSObject.Properties.Name -contains 'dry_run' -and [bool]$request.dry_run) {
        return @{ ok = $true; dry_run = $true; app = $kind; path = $path; exists = (Test-Path -LiteralPath $path) }
    }
    $appInfo = Get-App $kind $visible
    $app = $appInfo.app
    $doc = Find-Doc $app $kind $path
    if ($null -ne $doc) {
        $how = 'attached'
    } elseif (Test-Path -LiteralPath $path) {
        if ($kind -eq 'word') {
            $doc = $app.Documents.Open($path, $false, $false, $true)
        } else {
            $doc = $app.Workbooks.Open($path)
        }
        $how = 'opened'
    } else {
        if ($kind -eq 'word') {
            $doc = $app.Documents.Add()
            [void]$doc.SaveAs2($path)
        } else {
            $doc = $app.Workbooks.Add()
            [void]$doc.SaveAs($path)
        }
        $how = 'created'
    }
    if ($how -ne 'attached') { Mark-Owned $state $kind $path }
    if ($how -ne 'attached') {
        try { $doc.Activate() } catch {}
    }
    Remember-Last $state $kind $path
    Write-State $request.state_path $state
    return @{ ok = $true; action = 'open'; doc = (Doc-Info $doc $kind $path $how $appInfo.attached) }
}

function Resolve-Document($request, $state, [bool]$autoOpen = $false) {
    $target = App-Of $request $state
    $appInfo = Get-App $target.app $true
    $app = $appInfo.app
    $doc = $null
    if ($target.path -ne '') {
        $doc = Find-Doc $app $target.app $target.path
        if ($null -eq $doc -and $autoOpen -and (Test-Path -LiteralPath $target.path)) {
            if ($target.app -eq 'word') { $doc = $app.Documents.Open($target.path, $false, $false, $true) }
            else { $doc = $app.Workbooks.Open($target.path) }
            Mark-Owned $state $target.app $target.path
        }
    }
    if ($null -eq $doc) {
        throw '目标文档没有在 Office 中打开；请先调用 office_open'
    }
    Remember-Last $state $target.app $target.path
    Write-State $request.state_path $state
    return @{ app = $target.app; path = $target.path; document = $doc; application = $app; attached = $appInfo.attached }
}
function Has-Prop($obj, [string]$name) {
    return ($null -ne $obj -and $obj.PSObject.Properties.Name -contains $name)
}

function Get-Prop($obj, [string]$name, $fallback) {
    if (Has-Prop $obj $name) { return $obj.$name }
    return $fallback
}

function Trim-Cell([string]$text) {
    return $text.TrimEnd([char]13, [char]10, [char]7, [char]11)
}

function Color-Int([string]$text) {
    $value = $text.Trim().TrimStart('#')
    if ($value.Length -ne 6) { throw "颜色需要 RRGGBB：$text" }
    $r = [Convert]::ToInt32($value.Substring(0, 2), 16)
    $g = [Convert]::ToInt32($value.Substring(2, 2), 16)
    $b = [Convert]::ToInt32($value.Substring(4, 2), 16)
    return ($r + 256 * $g + 65536 * $b)
}

function Word-Style($doc, [string]$name) {
    if ($name -eq '') { return $null }
    $map = @{
        normal = -1; body = -1; title = -63;
        heading1 = -2; 'heading 1' = -2; heading2 = -3; 'heading 2' = -3;
        heading3 = -4; 'heading 3' = -4; heading4 = -5; 'heading 4' = -5;
        heading5 = -6; 'heading 5' = -6; heading6 = -7; 'heading 6' = -7;
        listbullet = -48; 'list bullet' = -48; listnumber = -49; 'list number' = -49;
        quote = -85
    }
    $key = $name.ToLowerInvariant()
    if ($map.ContainsKey($key)) { return $map[$key] }
    return $name
}

function Set-Word-Style($para, $doc, [string]$name) {
    if ($name -eq '') { return }
    $style = Word-Style $doc $name
    try { $para.Style = $style } catch { throw "Word 样式不存在：$name" }
}

function Apply-Word-Format($para, $op) {
    $range = $para.Range
    if (Has-Prop $op 'bold') { $range.Bold = [bool]$op.bold }
    if (Has-Prop $op 'italic') { $range.Italic = [bool]$op.italic }
    if (Has-Prop $op 'font_size') {
        $size = [double]$op.font_size
        if ($size -lt 4 -or $size -gt 200) { throw 'font_size 需在 4..200 之间' }
        $range.Font.Size = $size
    }
    if (Has-Prop $op 'font') { $range.Font.Name = [string]$op.font }
    if (Has-Prop $op 'color') { $range.Font.Color = Color-Int ([string]$op.color) }
    if (Has-Prop $op 'alignment') {
        $align = ([string]$op.alignment).ToLowerInvariant()
        $map = @{ left = 0; center = 1; right = 2; justify = 3; both = 3 }
        if (-not $map.ContainsKey($align)) { throw 'alignment 只能是 left/center/right/justify' }
        $para.Alignment = $map[$align]
    }
}

function Get-Word-Para($doc, $number, $expect) {
    $index = [int]$number
    if ($index -lt 1 -or $index -gt $doc.Paragraphs.Count) { throw "Word 段落编号超出范围：$index" }
    $para = $doc.Paragraphs.Item($index)
    $text = Trim-Cell ([string]$para.Range.Text)
    if ($null -ne $expect -and $text -ne '' -and -not $text.StartsWith([string]$expect, [StringComparison]::Ordinal)) {
        throw "段落 $index 开头不是「$expect」；文档已变化，请重新 office_read"
    }
    return $para
}

function Set-Word-Para-Text($para, [string]$text) {
    $range = $para.Range
    try { [void]$range.MoveEnd(1, -1) } catch {}
    $range.Text = $text
}

function Add-Word-Para($doc, $text, $op, [string]$where = 'end', $anchor = $null, [int]$anchorIndex = 0) {
    $lines = @($text -split "\r?\n")
    $added = @()
    foreach ($line in $lines) {
        if ($where -eq 'start') {
            $doc.Content.InsertParagraphBefore()
            $para = $doc.Paragraphs.Item(1)
        } elseif ($where -eq 'before' -and $null -ne $anchor) {
            $anchor.Range.InsertParagraphBefore()
            $para = $doc.Paragraphs.Item([Math]::Max(1, $anchorIndex))
        } elseif ($where -eq 'after' -and $null -ne $anchor) {
            $anchor.Range.InsertParagraphAfter()
            $para = $doc.Paragraphs.Item($anchorIndex + 1)
        } else {
            $doc.Content.InsertParagraphAfter()
            $para = $doc.Paragraphs.Item($doc.Paragraphs.Count)
        }
        Set-Word-Para-Text $para $line
        if (Has-Prop $op 'style') { Set-Word-Style $para $doc ([string]$op.style) }
        Apply-Word-Format $para $op
        $added += $para
    }
    return $added
}
function Read-Word($request, $target) {
    $doc = $target.document
    $max = 120
    if (Has-Prop $request 'max_paragraphs') { $max = [Math]::Min(400, [Math]::Max(1, [int]$request.max_paragraphs)) }
    $full = $true
    if (Has-Prop $request 'full') { $full = [bool]$request.full }
    $paragraphs = @()
    $count = [Math]::Min($doc.Paragraphs.Count, $max)
    for ($i = 1; $i -le $count; $i++) {
        $para = $doc.Paragraphs.Item($i)
        $text = Trim-Cell ([string]$para.Range.Text)
        if (-not $full -and $text.Length -gt 300) { $text = $text.Substring(0, 300) + '...' }
        $style = ''
        try { $style = [string]$para.Style.NameLocal } catch {}
        $paragraphs += [pscustomobject]@{
            index = $i
            text = $text
            style = $style
        }
    }
    $tables = @()
    $includeTables = $true
    if (Has-Prop $request 'include_tables') { $includeTables = [bool]$request.include_tables }
    if ($includeTables) {
        $tableCount = [Math]::Min($doc.Tables.Count, 20)
        for ($t = 1; $t -le $tableCount; $t++) {
            $table = $doc.Tables.Item($t)
            $rows = @()
            $rowCount = [Math]::Min($table.Rows.Count, 40)
            $colCount = [Math]::Min($table.Columns.Count, 20)
            for ($r = 1; $r -le $rowCount; $r++) {
                $cells = @()
                for ($c = 1; $c -le $colCount; $c++) {
                    $cells += Trim-Cell ([string]$table.Cell($r, $c).Range.Text)
                }
                $rows += ,$cells
            }
            $tables += [pscustomobject]@{
                index = $t
                rows = $rowCount
                columns = $colCount
                data = $rows
            }
        }
    }
    $saved = $false
    try { $saved = [bool]$doc.Saved } catch {}
    return @{
        ok = $true
        action = 'read'
        app = 'word'
        path = $target.path
        name = [string]$doc.Name
        saved = $saved
        paragraph_count = $doc.Paragraphs.Count
        paragraphs = $paragraphs
        truncated = ($doc.Paragraphs.Count -gt $max)
        tables = $tables
    }
}
function Read-Excel($request, $target) {
    $book = $target.document
    $sheets = @()
    foreach ($sheet in $book.Worksheets) { $sheets += [string]$sheet.Name }
    $sheetName = ''
    if (Has-Prop $request 'sheet') { $sheetName = [string]$request.sheet }
    if ($sheetName -eq '') { $sheetName = [string]$book.ActiveSheet.Name }
    $sheet = $book.Worksheets.Item($sheetName)
    $used = $sheet.UsedRange
    $usedAddress = [string]$used.Address()
    $rangeAddress = $usedAddress
    if ((Has-Prop $request 'range') -and [string]$request.range -ne '') { $rangeAddress = [string]$request.range }
    $range = $sheet.Range($rangeAddress)
    $maxRows = 60
    $maxCols = 20
    if (Has-Prop $request 'max_rows') { $maxRows = [Math]::Min(200, [Math]::Max(1, [int]$request.max_rows)) }
    if (Has-Prop $request 'max_cols') { $maxCols = [Math]::Min(40, [Math]::Max(1, [int]$request.max_cols)) }
    $rows = [Math]::Min($range.Rows.Count, $maxRows)
    $cols = [Math]::Min($range.Columns.Count, $maxCols)
    $values = @()
    $formulas = @()
    $withFormulas = $false
    if (Has-Prop $request 'formulas') { $withFormulas = [bool]$request.formulas }
    for ($r = 1; $r -le $rows; $r++) {
        $valueRow = @()
        $formulaRow = @()
        for ($c = 1; $c -le $cols; $c++) {
            $cell = $range.Cells.Item($r, $c)
            $valueRow += $cell.Value2
            if ($withFormulas) { $formulaRow += [string]$cell.Formula }
        }
        $values += ,$valueRow
        if ($withFormulas) { $formulas += ,$formulaRow }
    }
    $saved = $false
    try { $saved = [bool]$book.Saved } catch {}
    return @{
        ok = $true
        action = 'read'
        app = 'excel'
        path = $target.path
        name = [string]$book.Name
        saved = $saved
        sheets = $sheets
        sheet = $sheetName
        used_range = $usedAddress
        range = [string]$range.Address()
        rows = $rows
        columns = $cols
        values = $values
        formulas = if ($withFormulas) { $formulas } else { $null }
        truncated = (($range.Rows.Count -gt $rows) -or ($range.Columns.Count -gt $cols))
    }
}
function Run-Word-Op($doc, $op) {
    $name = [string]$op.op
    switch ($name) {
        'insert_paragraphs' {
            $items = @($op.items)
            if ($items.Count -eq 0) { throw 'insert_paragraphs 需要 items' }
            $where = 'end'
            if (Has-Prop $op 'where') { $where = ([string]$op.where).ToLowerInvariant() }
            $anchor = $null
            $anchorIndex = 0
            if (Has-Prop $op 'para') {
                $anchor = Get-Word-Para $doc $op.para (Get-Prop $op 'expect' $null)
                $anchorIndex = [int]$op.para
                if ($where -eq 'end') { $where = 'after' }
            }
            $made = 0
            foreach ($item in $items) {
                $text = if ($item -is [string]) { [string]$item } else { [string](Get-Prop $item 'text' '') }
                $spec = if ($item -is [string]) { $op } else { $item }
                $added = Add-Word-Para $doc $text $spec $where $anchor $anchorIndex
                $made += @($added).Count
                if ($null -ne $added -and $added.Count -gt 0 -and $where -in @('before', 'after')) {
                    $anchor = $added[$added.Count - 1]
                    $anchorIndex = $anchorIndex + $added.Count
                    $where = 'after'
                }
            }
            return "inserted $made paragraph(s)"
        }
        'set_text' {
            if (-not (Has-Prop $op 'para')) { throw 'set_text 需要 para' }
            $para = Get-Word-Para $doc $op.para (Get-Prop $op 'expect' $null)
            Set-Word-Para-Text $para ([string](Get-Prop $op 'text' ''))
            if ((Has-Prop $op 'bold') -or (Has-Prop $op 'italic') -or (Has-Prop $op 'font_size') -or (Has-Prop $op 'alignment')) {
                Apply-Word-Format $para $op
            }
            return "rewrote paragraph $([int]$op.para)"
        }
        'replace_text' {
            $findText = [string](Get-Prop $op 'find' '')
            if ($findText -eq '') { throw 'replace_text 需要 find' }
            $replace = [string](Get-Prop $op 'replace' '')
            $all = [bool](Get-Prop $op 'all' $true)
            $matchCase = [bool](Get-Prop $op 'match_case' $false)
            $whole = [bool](Get-Prop $op 'match_whole_word' $false)
            $find = $doc.Content.Find
            $find.ClearFormatting()
            $find.Replacement.ClearFormatting()
            $replaceMode = if ($all) { 2 } else { 1 }
            $changed = $find.Execute($findText, $matchCase, $whole, $false, $false, $false, $true, 1, $false, $replace, $replaceMode)
            if (-not $changed) { return 'no match' }
            return 'replaced text'
        }
        'insert_table' {
            $data = @($op.data)
            if ($data.Count -eq 0) { throw 'insert_table 需要 data' }
            $rows = $data.Count
            $cols = @($data[0]).Count
            if ($rows -gt 500 -or $cols -gt 50) { throw '表格过大（最多 500 行、50 列）' }
            $doc.Content.InsertParagraphAfter()
            $table = $doc.Tables.Add($doc.Paragraphs.Item($doc.Paragraphs.Count).Range, $rows, $cols)
            for ($r = 1; $r -le $rows; $r++) {
                $row = @($data[$r - 1])
                for ($c = 1; $c -le $cols; $c++) {
                    $cellText = if ($c -le $row.Count) { [string]$row[$c - 1] } else { '' }
                    $table.Cell($r, $c).Range.Text = $cellText
                }
            }
            $borders = ([string](Get-Prop $op 'borders' 'grid')).ToLowerInvariant()
            if ($borders -eq 'none') { $table.Borders.Enable = $false } else { $table.Borders.Enable = $true }
            if ([bool](Get-Prop $op 'header' $true)) { $table.Rows.Item(1).Range.Bold = $true }
            if ((Has-Prop $op 'caption') -and [string]$op.caption -ne '') {
                $doc.Content.InsertParagraphAfter()
                $caption = $doc.Paragraphs.Item($doc.Paragraphs.Count)
                Set-Word-Para-Text $caption ([string]$op.caption)
                $caption.Alignment = 1
            }
            return "inserted table ${rows}x${cols}"
        }
'set_cell' {
            $tableIndex = [int](Get-Prop $op 'table' 1)
            if ($tableIndex -lt 1 -or $tableIndex -gt $doc.Tables.Count) { throw "表格编号超出范围：$tableIndex" }
            $table = $doc.Tables.Item($tableIndex)
            $row = [int](Get-Prop $op 'row' 0)
            $col = [int](Get-Prop $op 'col' 0)
            if ($row -lt 1 -or $row -gt $table.Rows.Count -or $col -lt 1 -or $col -gt $table.Columns.Count) {
                throw 'set_cell 的行列超出表格范围'
            }
            $table.Cell($row, $col).Range.Text = [string](Get-Prop $op 'text' '')
            return "set table $tableIndex cell $row,$col"
        }
        'format_text' {
            if (-not (Has-Prop $op 'para')) { throw 'format_text 需要 para' }
            $para = Get-Word-Para $doc $op.para (Get-Prop $op 'expect' $null)
            Apply-Word-Format $para $op
            return "formatted paragraph $([int]$op.para)"
        }
        'format_table' {
            $tableIndex = [int](Get-Prop $op 'table' 1)
            if ($tableIndex -lt 1 -or $tableIndex -gt $doc.Tables.Count) { throw "表格编号超出范围：$tableIndex" }
            $table = $doc.Tables.Item($tableIndex)
            if ((Has-Prop $op 'header') -and [bool]$op.header) { $table.Rows.Item(1).Range.Bold = $true }
            if (Has-Prop $op 'bold') { $table.Range.Bold = [bool]$op.bold }
            if (Has-Prop $op 'font_size') { $table.Range.Font.Size = [double]$op.font_size }
            if (Has-Prop $op 'borders') {
                $table.Borders.Enable = (([string]$op.borders).ToLowerInvariant() -ne 'none')
            }
            return "formatted table $tableIndex"
        }
        'insert_image' {
            $path = [string](Get-Prop $op 'path' '')
            if ($path -eq '' -or -not (Test-Path -LiteralPath $path)) { throw "图片不存在：$path" }
            $doc.Content.InsertParagraphAfter()
            $para = $doc.Paragraphs.Item($doc.Paragraphs.Count)
            $shape = $doc.InlineShapes.AddPicture((Full-Path $path), $false, $true, $para.Range)
            if (Has-Prop $op 'width') { $shape.Width = [double]$op.width }
            if (Has-Prop $op 'height') { $shape.Height = [double]$op.height }
            if ((Has-Prop $op 'caption') -and [string]$op.caption -ne '') {
                $doc.Content.InsertParagraphAfter()
                $caption = $doc.Paragraphs.Item($doc.Paragraphs.Count)
                Set-Word-Para-Text $caption ([string]$op.caption)
                $caption.Alignment = 1
            }
            return 'inserted image'
        }
        'page_break' {
            $doc.Content.InsertBreak(7)
            return 'inserted page break'
        }
        'update_fields' {
            $doc.Fields.Update() | Out-Null
            return 'updated fields'
        }
        default { throw "Word 不支持的操作：$name（先调用 office_help app='word'）" }
    }
}
function Normalize-Rows($values) {
    if ($null -eq $values) { return @() }
    if ($values -is [System.Array]) {
        if ($values.Count -gt 0 -and $values[0] -is [System.Array]) { return @($values) }
        return @(,([object[]]$values))
    }
    return @(,([object[]]@($values)))
}

function Write-Excel-Range($sheet, [string]$address, $values) {
    $rows = Normalize-Rows $values
    if ($rows.Count -eq 0) { throw 'write_range 需要 values' }
    $rowsCount = $rows.Count
    $colsCount = 0
    foreach ($row in $rows) { $colsCount = [Math]::Max($colsCount, @($row).Count) }
    if ($rowsCount -gt 5000 -or $colsCount -gt 200) { throw 'write_range 数据过大（最多 5000 行、200 列）' }
    $start = $sheet.Range($address).Cells.Item(1, 1)
    $written = 0
    for ($r = 0; $r -lt $rowsCount; $r++) {
        $row = @($rows[$r])
        for ($c = 0; $c -lt $row.Count; $c++) {
            $cell = $start.Offset($r, $c)
            $value = $row[$c]
            try {
                if ($null -eq $value) {
                    $cell.ClearContents() | Out-Null
                } elseif ($value -is [string] -and $value.StartsWith('=')) {
                    $cell.Formula = $value
                } elseif ($value -is [int] -or $value -is [long] -or $value -is [double] -or $value -is [decimal]) {
                    $cell.Value2 = [double]$value
                } elseif ($value -is [bool]) {
                    $cell.Value2 = [bool]$value
                } else {
                    $cell.Value2 = [string]$value
                }
            } catch {
                throw "write_range failed at row $($r + 1), column $($c + 1), value '$value': $($_.Exception.Message)"
            }
            $written++
        }
    }
    return $written
}

function Format-Excel-Range($sheet, [string]$address, $op) {
    $range = $sheet.Range($address)
    if (Has-Prop $op 'bold') { $range.Font.Bold = [bool]$op.bold }
    if (Has-Prop $op 'italic') { $range.Font.Italic = [bool]$op.italic }
    if (Has-Prop $op 'font_size') { $range.Font.Size = [double]$op.font_size }
    if (Has-Prop $op 'font') { $range.Font.Name = [string]$op.font }
    if (Has-Prop $op 'number_format') { $range.NumberFormat = [string]$op.number_format }
    if (Has-Prop $op 'color') { $range.Font.Color = Color-Int ([string]$op.color) }
    if (Has-Prop $op 'fill') { $range.Interior.Color = Color-Int ([string]$op.fill) }
    if (Has-Prop $op 'wrap') { $range.WrapText = [bool]$op.wrap }
    if (Has-Prop $op 'column_width') { $range.Columns.ColumnWidth = [double]$op.column_width }
    if (Has-Prop $op 'row_height') { $range.Rows.RowHeight = [double]$op.row_height }
    if (Has-Prop $op 'alignment') {
        $align = ([string]$op.alignment).ToLowerInvariant()
        $map = @{ left = -4131; center = -4108; right = -4152; general = -1 }
        if (-not $map.ContainsKey($align)) { throw 'alignment 只能是 left/center/right/general' }
        $range.HorizontalAlignment = $map[$align]
    }
    return 'formatted ' + [string]$range.Address()
}
function Run-Excel-Op($book, $app, $op) {
    $name = [string]$op.op
    $sheetName = [string](Get-Prop $op 'sheet' '')
    $sheet = if ($sheetName -eq '') { $book.ActiveSheet } else { $book.Worksheets.Item($sheetName) }
    switch ($name) {
        'write_range' {
            if (-not (Has-Prop $op 'range')) { throw 'write_range 需要 range' }
            $written = Write-Excel-Range $sheet ([string]$op.range) (Get-Prop $op 'values' $null)
            return "wrote $written cell(s) into $($sheet.Name)!$([string]$op.range)"
        }
        'fill_formula' {
            if (-not (Has-Prop $op 'range') -or -not (Has-Prop $op 'formula')) { throw 'fill_formula 需要 range 与 formula' }
            $range = $sheet.Range([string]$op.range)
            $range.Formula = [string]$op.formula
            $direction = [string](Get-Prop $op 'direction' 'down')
            if ($direction -eq 'right') { $range.FillRight() } else { $range.FillDown() }
            return "filled $([string]$range.Address()) with formula"
        }
        'format_range' {
            if (-not (Has-Prop $op 'range')) { throw 'format_range 需要 range' }
            return Format-Excel-Range $sheet ([string]$op.range) $op
        }
        'autofit' {
            $address = if (Has-Prop $op 'range') { [string]$op.range } else { [string]$sheet.UsedRange.Address() }
            $sheet.Range($address).Columns.AutoFit() | Out-Null
            return "autofit $address"
        }
        'insert_rows' {
            $row = [int](Get-Prop $op 'row' 1)
            $count = [Math]::Max(1, [int](Get-Prop $op 'count' 1))
            $sheet.Rows("$row`:$($row + $count - 1)").Insert() | Out-Null
            return "inserted $count row(s) at $row"
        }
        'delete_rows' {
            $row = [int](Get-Prop $op 'row' 1)
            $count = [Math]::Max(1, [int](Get-Prop $op 'count' 1))
            $sheet.Rows("$row`:$($row + $count - 1)").Delete() | Out-Null
            return "deleted $count row(s) at $row"
        }
        'add_sheet' {
            $newName = [string](Get-Prop $op 'name' '')
            if ($newName -eq '') { throw 'add_sheet 需要 name' }
            $newSheet = $book.Worksheets.Add([System.Reflection.Missing]::Value, $book.Worksheets.Item($book.Worksheets.Count))
            $newSheet.Name = $newName
            return "added sheet $newName"
        }
        'rename_sheet' {
            $from = [string](Get-Prop $op 'sheet' '')
            $to = [string](Get-Prop $op 'name' '')
            if ($from -eq '' -or $to -eq '') { throw 'rename_sheet 需要 sheet 与 name' }
            $book.Worksheets.Item($from).Name = $to
            return "renamed sheet $from to $to"
        }
        'delete_sheet' {
            $from = [string](Get-Prop $op 'sheet' '')
            if ($from -eq '') { throw 'delete_sheet 需要 sheet' }
            if ($book.Worksheets.Count -le 1) { throw '不能删除唯一的工作表' }
            $book.Worksheets.Item($from).Delete()
            return "deleted sheet $from"
        }
        'add_chart' {
            $range = [string](Get-Prop $op 'range' '')
            if ($range -eq '') { throw 'add_chart 需要 range' }
            $type = ([string](Get-Prop $op 'type' 'column')).ToLowerInvariant()
            $types = @{ column = 51; bar = 57; line = 4; pie = 5; doughnut = -4120; area = 1; radar = -4151 }
            if (-not $types.ContainsKey($type)) { throw "不支持的图表类型：$type" }
            $obj = $sheet.Shapes.AddChart2(-1, $types[$type], 10, 10, 420, 260)
            $chart = $obj.Chart
            $chart.SetSourceData($sheet.Range($range))
            if ($type -ne 'column') { $chart.ChartType = $types[$type] }
            if ((Has-Prop $op 'title') -and [string]$op.title -ne '') {
                $chart.HasTitle = $true
                $chart.ChartTitle.Text = [string]$op.title
            }
            return "added $type chart for $range"
        }
        'freeze' {
            $cell = [string](Get-Prop $op 'cell' 'A2')
            $sheet.Activate()
            $app.ActiveWindow.FreezePanes = $false
            $sheet.Range($cell).Select() | Out-Null
            $app.ActiveWindow.FreezePanes = $true
            return "froze panes at $cell"
        }
        default { throw "Excel 不支持的操作：$name（先调用 office_help app='excel'）" }
    }
}
function Edit-Document($request, $state) {
    $target = Resolve-Document $request $state $true
    $ops = @($request.ops)
    if ($ops.Count -eq 0) { throw 'office_edit 需要非空 ops' }
    if ($ops.Count -gt 100) { throw '一次最多 100 个操作' }
    if ((Has-Prop $request 'dry_run') -and [bool]$request.dry_run) {
        return @{ ok = $true; dry_run = $true; app = $target.app; path = $target.path; operations = $ops.Count }
    }
    $done = @()
    $undo = $null
    if ($target.app -eq 'word') {
        try {
            $undo = $target.document.Application.UndoRecord
            $undo.StartCustomRecord()
        } catch { $undo = $null }
    }
    try {
        foreach ($op in $ops) {
            if ($target.app -eq 'word') {
                $done += Run-Word-Op $target.document $op
            } else {
                $done += Run-Excel-Op $target.document $target.application $op
            }
        }
    } catch {
        $finished = $done.Count
        throw "第 $($finished + 1) 项失败，已完成 $finished 项：$($_.Exception.Message)"
    } finally {
        if ($null -ne $undo) { try { $undo.EndCustomRecord() } catch {} }
    }
    Write-State $request.state_path $state
    return @{
        ok = $true
        action = 'edit'
        app = $target.app
        path = $target.path
        done = $done
        count = $ops.Count
        saved = $false
    }
}

function Save-Document($request, $state) {
    $targetRequest = $request.PSObject.Copy()
    $targetRequest.path = $null
    $target = Resolve-Document $targetRequest $state $false
    $doc = $target.document
    if ((Has-Prop $request 'dry_run') -and [bool]$request.dry_run) {
        return @{ ok = $true; dry_run = $true; app = $target.app; path = $target.path; save_as = (Get-Prop $request 'path' '') }
    }
    $savePath = ''
    if ((Has-Prop $request 'path') -and [string]$request.path -ne '') { $savePath = Full-Path ([string]$request.path) }
    if ($savePath -eq '') {
        if ($target.app -eq 'word') { $doc.Save() } else { $doc.Save() }
        $resultPath = $target.path
    } else {
        $ext = [System.IO.Path]::GetExtension($savePath).ToLowerInvariant()
        $parent = [System.IO.Path]::GetDirectoryName($savePath)
        if ($parent -ne '' -and -not (Test-Path -LiteralPath $parent)) {
            [void][System.IO.Directory]::CreateDirectory($parent)
        }
        if ($ext -eq '.pdf') {
            if ($target.app -eq 'word') {
                $doc.ExportAsFixedFormat($savePath, 17)
            } else {
                $doc.ExportAsFixedFormat(0, $savePath)
            }
            $resultPath = $savePath
        } else {
            if ($target.app -eq 'word') {
                [void]$doc.SaveAs2($savePath)
            } else {
                [void]$doc.SaveAs($savePath)
            }
            $oldPath = $target.path
            if (Is-Owned $state $target.app $oldPath) {
                Forget-Owned $state $target.app $oldPath
                Mark-Owned $state $target.app $savePath
            }
            Remember-Last $state $target.app $savePath
            Write-State $request.state_path $state
            $resultPath = $savePath
        }
    }
    $size = 0
    try { $size = (Get-Item -LiteralPath $resultPath).Length } catch {}
    return @{
        ok = $true
        action = 'save'
        app = $target.app
        path = $resultPath
        bytes = $size
    }
}

function Close-Document($request, $state) {
    $target = Resolve-Document $request $state $false
    $doc = $target.document
    $owned = Is-Owned $state $target.app $target.path
    $allowUser = (Has-Prop $request 'allow_user_doc') -and [bool]$request.allow_user_doc
    if (-not $owned -and -not $allowUser) {
        throw '这是用户原本打开的文档；除非传 allow_user_doc=true，否则不关闭'
    }
    $saved = $false
    try { $saved = [bool]$doc.Saved } catch {}
    $save = $null
    if (Has-Prop $request 'save') { $save = [bool]$request.save }
    if (-not $saved -and $null -eq $save) {
        throw '文档有未保存修改：请传 save=true 保存后关闭，或 save=false 放弃修改'
    }
    $doSave = $saved -or ($null -ne $save -and [bool]$save)
    if ($doSave) { if ($target.app -eq 'word') { $doc.Save() } else { $doc.Save() } }
    if ($target.app -eq 'word') { $doc.Close(0) } else { $doc.Close($false) }
    Forget-Owned $state $target.app $target.path
    if ($null -ne $state.last -and ([string]$state.last.path) -eq $target.path) { $state.last = $null }
    Write-State $request.state_path $state
    return @{ ok = $true; action = 'close'; app = $target.app; path = $target.path; saved = $doSave }
}
function Get-Status($state) {
    $result = @()
    foreach ($kind in @('word', 'excel')) {
        $app = Get-RunningApp $kind
        if ($null -eq $app) {
            $result += [pscustomobject]@{ app = $kind; running = $false; version = ''; documents = @() }
            continue
        }
        $documents = @()
        try {
            if ($kind -eq 'word') {
                for ($i = 1; $i -le $app.Documents.Count; $i++) {
                    $doc = $app.Documents.Item($i)
                    $full = ''
                    try { $full = [string]$doc.FullName } catch {}
                    $saved = $false
                    try { $saved = [bool]$doc.Saved } catch {}
                    $documents += [pscustomobject]@{
                        name = [string]$doc.Name
                        path = $full
                        saved = $saved
                        owned = ($full -ne '' -and (Is-Owned $state $kind $full))
                    }
                }
            } else {
                for ($i = 1; $i -le $app.Workbooks.Count; $i++) {
                    $book = $app.Workbooks.Item($i)
                    $full = ''
                    try { $full = [string]$book.FullName } catch {}
                    $saved = $false
                    try { $saved = [bool]$book.Saved } catch {}
                    $documents += [pscustomobject]@{
                        name = [string]$book.Name
                        path = $full
                        saved = $saved
                        owned = ($full -ne '' -and (Is-Owned $state $kind $full))
                    }
                }
            }
        } catch {}
        $version = ''
        try { $version = [string]$app.Version } catch {}
        $result += [pscustomobject]@{ app = $kind; running = $true; version = $version; documents = $documents }
    }
    return @{ ok = $true; action = 'status'; apps = $result }
}

try {
    $request = Read-Request
    $statePath = ''
    if (Has-Prop $request 'state_path') { $statePath = [string]$request.state_path }
    $state = Read-State $statePath
    $action = [string]$request.action
    $result = $null
    switch ($action) {
        'open' { $result = Open-Document $request $state }
        'status' { $result = Get-Status $state }
        'read' {
            $target = Resolve-Document $request $state $true
            $result = if ($target.app -eq 'word') { Read-Word $request $target } else { Read-Excel $request $target }
        }
        'edit' { $result = Edit-Document $request $state }
        'save' { $result = Save-Document $request $state }
        'close' { $result = Close-Document $request $state }
        default { throw "未知操作：$action" }
    }
    if ($null -eq $result) { throw '操作没有返回结果' }
    $result | ConvertTo-Json -Depth 12 -Compress
} catch {
    @{ ok = $false; error = $_.Exception.Message } | ConvertTo-Json -Compress
    exit 0
}
