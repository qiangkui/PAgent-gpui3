param([string]$RequestFile = '')

$ErrorActionPreference = 'Stop'
try {
    [Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
} catch {}

# Date, format and serial-number helpers: a separate file so tests can load them
# without running this script's request loop.
. (Join-Path $PSScriptRoot 'office_dates.ps1')

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
    if ($ext -in @('.pptx', '.ppt', '.pptm', '.ppsx', '.ppsm')) { return 'powerpoint' }
    throw "不支持的文件类型：$ext（支持 Word .docx/.doc、Excel .xlsx/.xls/.xlsm、PowerPoint .pptx/.ppt/.pptm）"
}

function App-Of($request, $state) {
    $kind = ''
    if ($null -ne $request.app) { $kind = ([string]$request.app).ToLowerInvariant() }
    $path = ''
    if ($null -ne $request.path -and [string]$request.path -ne '') { $path = Full-Path ([string]$request.path) }
    if ($kind -eq '' -and $path -ne '') { $kind = App-From-Path $path }
    if ($kind -eq '' -and $null -ne $state.last) { $kind = [string]$state.last.app }
    if ($kind -notin @('word', 'excel', 'powerpoint')) { throw '缺少 app 或 path，且没有上次操作的文档' }
    if ($path -eq '' -and $null -ne $state.last -and [string]$state.last.app -eq $kind) {
        $path = [string]$state.last.path
    }
    if ($path -ne '') {
        $inferred = App-From-Path $path
        if ($inferred -ne $kind) { throw "文件 $path 属于 $inferred，不是 $kind" }
    }
    return @{ app = $kind; path = $path }
}

function App-ProgIds([string]$kind) {
    if ($kind -eq 'word') { return @('Word.Application', 'Kwps.Application') }
    if ($kind -eq 'excel') { return @('Excel.Application', 'Ket.Application') }
    return @('PowerPoint.Application', 'Kwpp.Application')
}

function Get-RunningApp([string]$kind) {
    foreach ($id in (App-ProgIds $kind)) {
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
        foreach ($id in (App-ProgIds $kind)) {
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
        } elseif ($kind -eq 'powerpoint') {
            # ppAlertsNone = 1; PowerPoint has no hidden-window mode, so `visible`
            # only decides whether the presentation gets a window (see Open-Document).
            $app.DisplayAlerts = 1
            if ($visible) { $app.Visible = -1 }
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
    } elseif ($kind -eq 'powerpoint') {
        for ($i = 1; $i -le $app.Presentations.Count; $i++) {
            try {
                $pres = $app.Presentations.Item($i)
                if ($null -ne $pres.FullName -and (Full-Path $pres.FullName).ToLowerInvariant() -eq $wanted) { return $pres }
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
    $info = @{
        app = $kind
        path = $path
        name = $name
        saved = $saved
        how = $how
        attached = [bool]$attached
    }
    if ($kind -eq 'powerpoint') {
        try { $info.slides = [int]$doc.Slides.Count } catch { $info.slides = 0 }
    }
    return $info
}

function Open-Document($request, $state) {
    $kind = ''
    if ($null -ne $request.app) { $kind = ([string]$request.app).ToLowerInvariant() }
    $rawPath = ''
    if ($null -ne $request.path -and [string]$request.path -ne '') { $rawPath = [string]$request.path }
    if ($rawPath -eq '') { throw 'office_open 需要绝对文件路径' }
    $path = Full-Path $rawPath
    if ($kind -eq '') { $kind = App-From-Path $path }
    if ($kind -notin @('word', 'excel', 'powerpoint')) { throw '只支持 Word、Excel 与 PowerPoint' }
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
    # msoTrue = -1, msoFalse = 0; a windowless presentation is how PowerPoint hides.
    $withWindow = if ($visible) { -1 } else { 0 }
    $doc = Find-Doc $app $kind $path
    if ($null -ne $doc) {
        $how = 'attached'
    } elseif (Test-Path -LiteralPath $path) {
        if ($kind -eq 'word') {
            $doc = $app.Documents.Open($path, $false, $false, $true)
        } elseif ($kind -eq 'powerpoint') {
            $doc = $app.Presentations.Open($path, 0, 0, $withWindow)
        } else {
            $doc = $app.Workbooks.Open($path)
        }
        $how = 'opened'
    } else {
        if ($kind -eq 'word') {
            $doc = $app.Documents.Add()
            [void]$doc.SaveAs2($path)
        } elseif ($kind -eq 'powerpoint') {
            $doc = $app.Presentations.Add($withWindow)
            [void]$doc.SaveAs($path, 24)
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
            elseif ($target.app -eq 'powerpoint') { $doc = $app.Presentations.Open($target.path, 0, 0, -1) }
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

function Length-Points([double]$value, [string]$unit) {
    switch ($unit.ToLowerInvariant()) {
        'cm' { return $value * 28.3464567 }
        'mm' { return $value * 2.83464567 }
        'in' { return $value * 72 }
        default { return $value }
    }
}

function Word-Align([string]$name) {
    $map = @{ left = 0; center = 1; right = 2; justify = 3; both = 3 }
    $key = $name.ToLowerInvariant()
    if (-not $map.ContainsKey($key)) { throw 'alignment 只能是 left/center/right/justify' }
    return $map[$key]
}

function Get-Doc-Property($doc, [string]$name) {
    try {
        $get = [System.Reflection.BindingFlags]::GetProperty
        # Word's DocumentProperties collection cannot be adapted by PowerShell ("Object
        # reference not set…"), so both sides go through raw IDispatch.
        $bag = [System.__ComObject].InvokeMember('BuiltInDocumentProperties', $get, $null, $doc, $null)
        if ($null -eq $bag) { return '' }
        $prop = [System.__ComObject].InvokeMember('Item', $get, $null, $bag, @($name))
        if ($null -eq $prop) { return '' }
        return [string][System.__ComObject].InvokeMember('Value', $get, $null, $prop, $null)
    } catch {
        return ''
    }
}

function Set-Doc-Property($doc, [string]$name, [string]$value) {
    $get = [System.Reflection.BindingFlags]::GetProperty
    $set = [System.Reflection.BindingFlags]::SetProperty
    $bag = [System.__ComObject].InvokeMember('BuiltInDocumentProperties', $get, $null, $doc, $null)
    if ($null -eq $bag) { throw 'Word 没有返回文档属性集合' }
    $prop = [System.__ComObject].InvokeMember('Item', $get, $null, $bag, @($name))
    if ($null -eq $prop) { throw "文档属性不存在：$name" }
    try {
        [void][System.__ComObject].InvokeMember('Value', $set, $null, $prop, @($value))
    } catch {
        throw "文档属性 $name 无法写入：$($_.Exception.Message)"
    }
}

function Snippet([string]$text, [string]$query) {
    if ($text.Length -le 120) { return $text }
    $at = $text.IndexOf($query, [StringComparison]::OrdinalIgnoreCase)
    if ($at -lt 0) { return $text.Substring(0, 120) + '...' }
    $from = [Math]::Max(0, $at - 40)
    $length = [Math]::Min($text.Length - $from, 120)
    $prefix = if ($from -gt 0) { '...' } else { '' }
    $suffix = if (($from + $length) -lt $text.Length) { '...' } else { '' }
    return $prefix + $text.Substring($from, $length) + $suffix
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
    if (Has-Prop $op 'alignment') { $para.Alignment = Word-Align ([string]$op.alignment) }
    if (Has-Prop $op 'space_before') { $para.SpaceBefore = [double]$op.space_before }
    if (Has-Prop $op 'space_after') { $para.SpaceAfter = [double]$op.space_after }
    if (Has-Prop $op 'line_spacing') {
        $multiple = [double]$op.line_spacing
        if ($multiple -lt 0.5 -or $multiple -gt 5) { throw 'line_spacing 需在 0.5..5 之间（1 为单倍行距）' }
        $para.LineSpacingRule = 5
        $para.LineSpacing = 12 * $multiple
    }
    if (Has-Prop $op 'indent_left') { $para.LeftIndent = Length-Points ([double]$op.indent_left) 'cm' }
    if (Has-Prop $op 'indent_first') { $para.FirstLineIndent = Length-Points ([double]$op.indent_first) 'cm' }
    if (Has-Prop $op 'keep_with_next') { $para.KeepWithNext = [bool]$op.keep_with_next }
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
    $headings = @()
    $count = [Math]::Min($doc.Paragraphs.Count, $max)
    for ($i = 1; $i -le $count; $i++) {
        $para = $doc.Paragraphs.Item($i)
        $text = Trim-Cell ([string]$para.Range.Text)
        if (-not $full -and $text.Length -gt 300) { $text = $text.Substring(0, 300) + '...' }
        $style = ''
        try { $style = [string]$para.Style.NameLocal } catch {}
        # OutlineLevel is 1..9 for headings and 10 for body text; it follows the
        # style's outline level, so it reads the same in every UI language.
        $level = 0
        try { $level = [int]$para.OutlineLevel } catch {}
        if ($level -lt 1 -or $level -gt 9) { $level = 0 }
        $paragraphs += [pscustomobject]@{
            index = $i
            text = $text
            style = $style
            outline = $level
        }
        if ($level -ge 1 -and $text -ne '' -and $headings.Count -lt 100) {
            $headings += [pscustomobject]@{ index = $i; level = $level; text = $text }
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
    # wdStatisticWords = 0, wdStatisticPages = 2; both are cheap properties of
    # the open document (Word recomputes pages in the background).
    $words = 0
    try { $words = [int]$doc.ComputeStatistics(0) } catch {}
    $pages = 0
    try { $pages = [int]$doc.ComputeStatistics(2) } catch {}
    $shapes = 0
    try { $shapes = [int]$doc.InlineShapes.Count } catch {}
    $commentCount = 0
    $comments = @()
    try {
        $commentCount = [int]$doc.Comments.Count
        $commentLimit = [Math]::Min($commentCount, 20)
        for ($c = 1; $c -le $commentLimit; $c++) {
            $comment = $doc.Comments.Item($c)
            $comments += [pscustomobject]@{
                author = [string]$comment.Author
                text = Trim-Cell ([string]$comment.Range.Text)
                scope = Snippet (Trim-Cell ([string]$comment.Scope.Text)) ''
            }
        }
    } catch {}
    return @{
        ok = $true
        action = 'read'
        app = 'word'
        path = $target.path
        name = [string]$doc.Name
        saved = $saved
        paragraph_count = $doc.Paragraphs.Count
        paragraphs = $paragraphs
        headings = $headings
        truncated = ($doc.Paragraphs.Count -gt $max)
        tables = $tables
        words = $words
        pages = $pages
        sections = $doc.Sections.Count
        inline_shapes = $shapes
        comment_count = $commentCount
        comments = $comments
        properties = [pscustomobject]@{
            title = Get-Doc-Property $doc 'Title'
            subject = Get-Doc-Property $doc 'Subject'
            author = Get-Doc-Property $doc 'Author'
            keywords = Get-Doc-Property $doc 'Keywords'
            category = Get-Doc-Property $doc 'Category'
        }
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
    # `dates` is `text` by default: a model reading 46027 cannot tell a date from a
    # quantity. `serial` keeps the raw numbers for anyone who wants them.
    $dateText = $true
    if (Has-Prop $request 'dates') { $dateText = ([string]$request.dates).ToLowerInvariant() -ne 'serial' }
    $date1904 = $false
    try { $date1904 = [bool]$book.Date1904 } catch {}
    # Excel's `Range.NumberFormat` comes back as DBNull through PowerShell 5.1, so
    # the format is read per cell — the loop already holds that cell for its value.
    # Only numbers that could be serials (a day count or a time fraction) are asked.
    for ($r = 1; $r -le $rows; $r++) {
        $valueRow = @()
        $formulaRow = @()
        for ($c = 1; $c -le $cols; $c++) {
            $cell = $range.Cells.Item($r, $c)
            $value = $cell.Value2
            if ($dateText -and ($value -is [double]) -and ([double]$value -gt 0) -and ([double]$value -le 2958465)) {
                $format = ''
                try { $format = [string]$cell.NumberFormat } catch {}
                if (Test-Date-Format $format) {
                    $text = Format-Serial-Date ([double]$value) $date1904
                    if ($null -ne $text) { $value = $text }
                }
            }
            $valueRow += $value
            if ($withFormulas) { $formulaRow += [string]$cell.Formula }
        }
        $values += ,$valueRow
        if ($withFormulas) { $formulas += ,$formulaRow }
    }
    $saved = $false
    try { $saved = [bool]$book.Saved } catch {}
    # Every sheet's extent, in one pass: the model plans a read/write with these
    # instead of guessing ranges or scanning blindly.
    $sheetInfo = @()
    $sheetLimit = [Math]::Min([int]$book.Worksheets.Count, 30)
    for ($s = 1; $s -le $sheetLimit; $s++) {
        try {
            $ws = $book.Worksheets.Item($s)
            $wsUsed = $ws.UsedRange
            $sheetInfo += [pscustomobject]@{
                name = [string]$ws.Name
                used_range = [string]$wsUsed.Address()
                rows = [int]$wsUsed.Rows.Count
                columns = [int]$wsUsed.Columns.Count
                hidden = ([int]$ws.Visible -ne -1)
            }
        } catch {}
    }
    $names = @()
    foreach ($item in $book.Names) {
        try {
            $full = [string]$item.Name
            # Excel keeps its own names (_FilterDatabase, Print_Area, ...) next to
            # the user's; they start with an underscore after the sheet prefix.
            $local = if ($full.Contains('!')) { $full.Substring($full.IndexOf('!') + 1) } else { $full }
            if ($local.StartsWith('_')) { continue }
            $names += [pscustomobject]@{ name = $full; refers_to = [string]$item.RefersTo }
        } catch {}
    }
    $charts = @()
    try {
        $chartObjects = $sheet.ChartObjects()
        $chartLimit = [Math]::Min([int]$chartObjects.Count, 20)
        for ($c = 1; $c -le $chartLimit; $c++) {
            $chartObject = $chartObjects.Item($c)
            $charts += [pscustomobject]@{
                name = [string]$chartObject.Name
                type = [int]$chartObject.Chart.ChartType
            }
        }
    } catch {}
    $listObjects = @()
    try {
        foreach ($table in $sheet.ListObjects) {
            $listObjects += [pscustomobject]@{ name = [string]$table.Name; range = [string]$table.Range.Address() }
        }
    } catch {}
    return @{
        ok = $true
        action = 'read'
        app = 'excel'
        path = $target.path
        name = [string]$book.Name
        saved = $saved
        sheets = $sheets
        sheet_info = $sheetInfo
        sheet = $sheetName
        used_range = $usedAddress
        last_row = ([int]$used.Row + [int]$used.Rows.Count - 1)
        last_column = ([int]$used.Column + [int]$used.Columns.Count - 1)
        range = [string]$range.Address()
        rows = $rows
        columns = $cols
        values = $values
        formulas = if ($withFormulas) { $formulas } else { $null }
        dates = if ($dateText) { 'text' } else { 'serial' }
        date1904 = $date1904
        names = $names
        charts = $charts
        tables = $listObjects
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
        'page_setup' {
            $unit = [string](Get-Prop $op 'unit' 'cm')
            $sections = $doc.Sections
            for ($i = 1; $i -le $sections.Count; $i++) {
                $page = $sections.Item($i).PageSetup
                if (Has-Prop $op 'orientation') {
                    $value = ([string]$op.orientation).ToLowerInvariant()
                    $map = @{ portrait = 0; landscape = 1 }
                    if (-not $map.ContainsKey($value)) { throw 'orientation 只能是 portrait 或 landscape' }
                    $page.Orientation = $map[$value]
                }
                if (Has-Prop $op 'paper') {
                    $value = ([string]$op.paper).ToLowerInvariant()
                    $map = @{ letter = 1; legal = 5; a3 = 6; a4 = 7; a5 = 11; b5 = 13 }
                    if (-not $map.ContainsKey($value)) { throw 'paper 支持 a3/a4/a5/b5/letter/legal' }
                    $page.PaperSize = $map[$value]
                }
                if (Has-Prop $op 'margins') {
                    $margins = $op.margins
                    if (Has-Prop $margins 'top') { $page.TopMargin = Length-Points ([double]$margins.top) $unit }
                    if (Has-Prop $margins 'bottom') { $page.BottomMargin = Length-Points ([double]$margins.bottom) $unit }
                    if (Has-Prop $margins 'left') { $page.LeftMargin = Length-Points ([double]$margins.left) $unit }
                    if (Has-Prop $margins 'right') { $page.RightMargin = Length-Points ([double]$margins.right) $unit }
                }
            }
            return "set page layout for $($sections.Count) section(s)"
        }
        'header_footer' {
            if ((Has-Prop $op 'footer') -and (Has-Prop $op 'page_numbers') -and [bool]$op.page_numbers) {
                throw 'footer 文本与 page_numbers 不能同时使用'
            }
            $section = $doc.Sections.Item(1)
            $touched = @()
            $align = Word-Align ([string](Get-Prop $op 'alignment' 'center'))
            if (Has-Prop $op 'header') {
                $range = $section.Headers.Item(1).Range
                $range.Text = [string]$op.header
                $range.Font.Size = 9
                $range.ParagraphFormat.Alignment = $align
                $touched += 'header'
            }
            if (Has-Prop $op 'footer') {
                $range = $section.Footers.Item(1).Range
                $range.Text = [string]$op.footer
                $range.Font.Size = 9
                $range.ParagraphFormat.Alignment = $align
                $touched += 'footer'
            }
            if ((Has-Prop $op 'page_numbers') -and [bool]$op.page_numbers) {
                $footer = $section.Footers.Item(1)
                $footer.Range.Text = ''
                $footer.Range.ParagraphFormat.Alignment = 1
                $at = $footer.Range
                $at.Collapse(0)
                [void]$at.Fields.Add($at, 33, $null, $false)
                $at = $footer.Range
                $at.Collapse(0)
                $at.InsertAfter(' / ')
                $at = $footer.Range
                $at.Collapse(0)
                [void]$at.Fields.Add($at, 26, $null, $false)
                $footer.Range.Font.Size = 9
                $touched += 'page_numbers'
            }
            if ($touched.Count -eq 0) { throw 'header_footer 需要 header、footer 或 page_numbers' }
            return 'set ' + ($touched -join ', ')
        }
        'insert_toc' {
            $levels = [Math]::Min(6, [Math]::Max(1, [int](Get-Prop $op 'levels' 3)))
            if ($doc.TablesOfContents.Count -gt 0) {
                $doc.TablesOfContents.Item(1).Update()
                return 'updated the existing table of contents'
            }
            [void]$doc.Content.InsertParagraphBefore()
            [void]$doc.TablesOfContents.Add($doc.Paragraphs.Item(1).Range, $true, 1, $levels)
            return "inserted a table of contents (levels 1..$levels)"
        }
        'section_break' {
            $doc.Content.InsertBreak(2) | Out-Null
            return "inserted a section break (now $($doc.Sections.Count) section(s))"
        }
        'insert_footnote' {
            $text = [string](Get-Prop $op 'text' '')
            if ($text -eq '') { throw 'insert_footnote 需要 text' }
            if (Has-Prop $op 'para') {
                $anchor = Get-Word-Para $doc $op.para (Get-Prop $op 'expect' $null)
                $range = $anchor.Range
            } else {
                $doc.Content.InsertParagraphAfter() | Out-Null
                $range = $doc.Paragraphs.Item($doc.Paragraphs.Count).Range
            }
            [void]$doc.Footnotes.Add($range, '', $text)
            return 'inserted a footnote'
        }
        'insert_hyperlink' {
            $url = [string](Get-Prop $op 'url' '')
            if ($url -eq '') { throw 'insert_hyperlink 需要 url' }
            $text = [string](Get-Prop $op 'text' $url)
            $doc.Content.InsertParagraphAfter() | Out-Null
            $range = $doc.Paragraphs.Item($doc.Paragraphs.Count).Range
            try { [void]$range.MoveEnd(1, -1) } catch {}
            $range.Text = ''
            [void]$doc.Hyperlinks.Add($range, $url, '', $text, $text)
            return "inserted a hyperlink to $url"
        }
        'insert_comment' {
            if (-not (Has-Prop $op 'para')) { throw 'insert_comment 需要 para' }
            $text = [string](Get-Prop $op 'text' '')
            if ($text -eq '') { throw 'insert_comment 需要 text' }
            $para = Get-Word-Para $doc $op.para (Get-Prop $op 'expect' $null)
            $comment = $doc.Comments.Add($para.Range, $text)
            if (Has-Prop $op 'author') { $comment.Author = [string]$op.author }
            return "commented paragraph $([int]$op.para)"
        }
        'set_properties' {
            $fields = @{
                title = 'Title'; subject = 'Subject'; author = 'Author'
                keywords = 'Keywords'; comments = 'Comments'; category = 'Category'
            }
            $changed = @()
            foreach ($key in $fields.Keys) {
                if (Has-Prop $op $key) {
                    Set-Doc-Property $doc $fields[$key] ([string]$op.$key)
                    $changed += $key
                }
            }
            if ($changed.Count -eq 0) { throw 'set_properties 需要至少一个属性（title/subject/author/keywords/comments/category）' }
            return 'set ' + ($changed -join ', ')
        }
        'delete_paragraph' {
            if (-not (Has-Prop $op 'para')) { throw 'delete_paragraph 需要 para' }
            if (-not (Has-Prop $op 'expect')) { throw 'delete_paragraph 需要 expect（段落开头文字）以防误删' }
            $para = Get-Word-Para $doc $op.para ([string]$op.expect)
            $para.Range.Delete() | Out-Null
            return "deleted paragraph $([int]$op.para)"
        }
        'delete_table' {
            $tableIndex = [int](Get-Prop $op 'table' 1)
            if ($tableIndex -lt 1 -or $tableIndex -gt $doc.Tables.Count) { throw "表格编号超出范围：$tableIndex" }
            $doc.Tables.Item($tableIndex).Delete()
            return "deleted table $tableIndex"
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

function Write-Excel-Range($sheet, [string]$address, $values, [bool]$asDates = $true) {
    $rows = Normalize-Rows $values
    if ($rows.Count -eq 0) { throw 'write_range 需要 values' }
    $rowsCount = $rows.Count
    $colsCount = 0
    foreach ($row in $rows) { $colsCount = [Math]::Max($colsCount, @($row).Count) }
    if ($rowsCount -gt 5000 -or $colsCount -gt 200) { throw 'write_range 数据过大（最多 5000 行、200 列）' }
    $start = $sheet.Range($address).Cells.Item(1, 1)
    $written = 0
    $dated = 0
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
                } elseif ($value -is [string]) {
                    # "2026-01-05" as a real date, so sorting, filters and formulas
                    # treat it like Excel does. Anything else stays text.
                    $asDate = ConvertTo-Date-Cell ([string]$value)
                    if (($null -ne $asDate) -and $asDates) {
                        $cell.NumberFormat = [string]$asDate.format
                        $cell.Value2 = [double]$asDate.serial
                        $dated++
                    } elseif ($null -ne $asDate) {
                        # dates:"text" — Excel parses a date-shaped string assigned to
                        # Value2, so the cell must be text before the value lands.
                        $cell.NumberFormat = '@'
                        $cell.Value2 = [string]$value
                    } else {
                        $cell.Value2 = [string]$value
                    }
                } else {
                    $cell.Value2 = [string]$value
                }
            } catch {
                throw "write_range failed at row $($r + 1), column $($c + 1), value '$value': $($_.Exception.Message)"
            }
            $written++
        }
    }
    return @{ cells = $written; dates = $dated }
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
            $asDates = ([string](Get-Prop $op 'dates' 'auto')).ToLowerInvariant() -ne 'text'
            $written = Write-Excel-Range $sheet ([string]$op.range) (Get-Prop $op 'values' $null) $asDates
            $suffix = ''
            if ($written.dates -gt 0) { $suffix = "（其中 $($written.dates) 个写成日期）" }
            return "wrote $($written.cells) cell(s) into $($sheet.Name)!$([string]$op.range)$suffix"
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
        'merge_cells' {
            if (-not (Has-Prop $op 'range')) { throw 'merge_cells 需要 range' }
            $range = $sheet.Range([string]$op.range)
            $range.Merge() | Out-Null
            return 'merged ' + [string]$range.Address()
        }
        'unmerge_cells' {
            if (-not (Has-Prop $op 'range')) { throw 'unmerge_cells 需要 range' }
            $range = $sheet.Range([string]$op.range)
            $range.UnMerge() | Out-Null
            return 'unmerged ' + [string]$range.Address()
        }
        'insert_columns' {
            $col = [string](Get-Prop $op 'col' '')
            if ($col -eq '') { throw 'insert_columns 需要 col（例如 B）' }
            $count = [Math]::Max(1, [int](Get-Prop $op 'count' 1))
            $index = $sheet.Range($col + '1').Column
            $block = $sheet.Range($sheet.Cells.Item(1, $index), $sheet.Cells.Item(1, $index + $count - 1))
            $block.EntireColumn.Insert() | Out-Null
            return "inserted $count column(s) at $col"
        }
        'delete_columns' {
            $col = [string](Get-Prop $op 'col' '')
            if ($col -eq '') { throw 'delete_columns 需要 col（例如 B）' }
            $count = [Math]::Max(1, [int](Get-Prop $op 'count' 1))
            $index = $sheet.Range($col + '1').Column
            $block = $sheet.Range($sheet.Cells.Item(1, $index), $sheet.Cells.Item(1, $index + $count - 1))
            $block.EntireColumn.Delete() | Out-Null
            return "deleted $count column(s) at $col"
        }
        'clear_range' {
            if (-not (Has-Prop $op 'range')) { throw 'clear_range 需要 range' }
            $range = $sheet.Range([string]$op.range)
            if ([bool](Get-Prop $op 'formats' $false)) {
                $range.Clear() | Out-Null
                return 'cleared contents and formats of ' + [string]$range.Address()
            }
            $range.ClearContents() | Out-Null
            return 'cleared contents of ' + [string]$range.Address()
        }
        'sort' {
            $address = [string](Get-Prop $op 'range' '')
            if ($address -eq '') { throw 'sort 需要 range' }
            $range = $sheet.Range($address)
            $key = [string](Get-Prop $op 'key' '')
            $keyIndex = 1
            if ($key -ne '') {
                # Key is a column letter inside the sort range (or the whole sheet,
                # in which case its offset inside the range still decides the column).
                $keyIndex = $sheet.Range($key + '1').Column - $range.Column + 1
            }
            if ($keyIndex -lt 1 -or $keyIndex -gt $range.Columns.Count) { throw "sort 的 key「$key」不在排序区域内" }
            $order = if (([string](Get-Prop $op 'order' 'asc')).ToLowerInvariant() -eq 'desc') { 2 } else { 1 }
            $hasHeader = if ([bool](Get-Prop $op 'has_header' $true)) { 1 } else { 2 }
            $sheet.Sort.SortFields.Clear()
            [void]$sheet.Sort.SortFields.Add($range.Columns.Item($keyIndex), 0, $order, $null)
            $sheet.Sort.SetRange($range)
            $sheet.Sort.Header = $hasHeader
            $sheet.Sort.Orientation = 1
            $sheet.Sort.Apply()
            return 'sorted ' + [string]$range.Address() + " by column $keyIndex"
        }
        'autofilter' {
            if ([bool](Get-Prop $op 'clear' $false)) {
                try { $sheet.AutoFilterMode = $false } catch {}
                return 'cleared the autofilter'
            }
            $address = [string](Get-Prop $op 'range' '')
            # A worksheet range is enumerable, so `$target = if (...) { $range }`
            # would collapse into an Object[] of cells; keep plain assignments.
            $target = $null
            if ($address -eq '') { $target = $sheet.UsedRange } else { $target = $sheet.Range($address) }
            if ([int]$target.Rows.Count -lt 2) { throw 'autofilter 区域至少需要表头与一行数据' }
            $target.AutoFilter() | Out-Null
            return 'autofilter on ' + [string]$target.Address()
        }
        'conditional_format' {
            $address = [string](Get-Prop $op 'range' '')
            if ($address -eq '') { throw 'conditional_format 需要 range' }
            $range = $sheet.Range($address)
            $rule = ([string](Get-Prop $op 'rule' '')).ToLowerInvariant()
            switch ($rule) {
                'greater' { $condition = $range.FormatConditions.Add(1, 5, [double](Get-Prop $op 'value' 0)) }
                'less' { $condition = $range.FormatConditions.Add(1, 6, [double](Get-Prop $op 'value' 0)) }
                'equal' { $condition = $range.FormatConditions.Add(1, 3, (Get-Prop $op 'value' 0)) }
                'between' {
                    $condition = $range.FormatConditions.Add(1, 1, [double](Get-Prop $op 'value' 0), [double](Get-Prop $op 'value2' 0))
                }
                'contains' {
                    $text = [string](Get-Prop $op 'value' '')
                    if ($text -eq '') { throw 'contains 需要 value（要包含的文字）' }
                    # xlTextString = 9: the text goes in the String argument, so
                    # Formula1/Formula2 must be passed as Missing, not omitted.
                    $condition = $range.FormatConditions.Add(9, 0, [System.Reflection.Missing]::Value, [System.Reflection.Missing]::Value, $text, 0)
                }
                'colorscale' {
                    $scale = $range.FormatConditions.AddColorScale(3)
                    if (Has-Prop $op 'min_color') { $scale.ColorScaleCriteria.Item(1).FormatColor.Color = Color-Int ([string]$op.min_color) }
                    if (Has-Prop $op 'mid_color') { $scale.ColorScaleCriteria.Item(2).FormatColor.Color = Color-Int ([string]$op.mid_color) }
                    if (Has-Prop $op 'max_color') { $scale.ColorScaleCriteria.Item(3).FormatColor.Color = Color-Int ([string]$op.max_color) }
                    return 'added a color scale to ' + [string]$range.Address()
                }
                default { throw 'conditional_format 的 rule 支持 greater/less/equal/between/contains/colorscale' }
            }
            if (Has-Prop $op 'fill') { $condition.Interior.Color = Color-Int ([string]$op.fill) }
            if (Has-Prop $op 'color') { $condition.Font.Color = Color-Int ([string]$op.color) }
            if ([bool](Get-Prop $op 'bold' $false)) { $condition.Font.Bold = $true }
            return "added a $rule rule to " + [string]$range.Address()
        }
        'validate_list' {
            $address = [string](Get-Prop $op 'range' '')
            $values = @(Get-Prop $op 'values' @())
            if ($address -eq '' -or $values.Count -eq 0) { throw 'validate_list 需要 range 与 values' }
            $parts = @()
            foreach ($value in $values) {
                $text = [string]$value
                if ($text -match ',') { throw 'validate_list 的选项不能包含逗号' }
                $parts += $text
            }
            $formula = $parts -join ','
            if ($formula.Length -gt 250) { throw 'validate_list 的选项合计不能超过 250 字符' }
            $range = $sheet.Range($address)
            $range.Validation.Delete()
            # xlValidateList = 3, xlValidAlertStop = 1, xlBetween = 1
            $range.Validation.Add(3, 1, 1, $formula) | Out-Null
            $range.Validation.InCellDropdown = $true
            if (Has-Prop $op 'prompt') { $range.Validation.InputMessage = [string]$op.prompt }
            return "added a $($parts.Count)-item list validation to " + [string]$range.Address()
        }
        'add_comment' {
            $cell = [string](Get-Prop $op 'cell' '')
            $text = [string](Get-Prop $op 'text' '')
            if ($cell -eq '' -or $text -eq '') { throw 'add_comment 需要 cell 与 text' }
            $target = $sheet.Range($cell)
            if ($null -ne $target.Comment) { $target.Comment.Delete() }
            [void]$target.AddComment($text)
            return "commented $cell"
        }
        'add_hyperlink' {
            $cell = [string](Get-Prop $op 'cell' '')
            $url = [string](Get-Prop $op 'url' '')
            if ($cell -eq '' -or $url -eq '') { throw 'add_hyperlink 需要 cell 与 url' }
            $text = [string](Get-Prop $op 'text' $url)
            [void]$sheet.Hyperlinks.Add($sheet.Range($cell), $url, '', $text, $text)
            return "linked $cell to $url"
        }
        'define_name' {
            $name = [string](Get-Prop $op 'name' '')
            $address = [string](Get-Prop $op 'range' '')
            if ($name -eq '' -or $address -eq '') { throw 'define_name 需要 name 与 range' }
            $existing = $null
            foreach ($item in $book.Names) {
                if ([string]$item.Name -eq $name) { $existing = $item }
            }
            if ($null -ne $existing) { $existing.Delete() }
            [void]$book.Names.Add($name, $sheet.Range($address))
            return "defined $name = $($sheet.Name)!$address"
        }
        'tab_color' {
            $color = [string](Get-Prop $op 'color' '')
            if ($color -eq '') { throw 'tab_color 需要 color（RRGGBB）' }
            if ($color.ToLowerInvariant() -eq 'none') {
                $sheet.Tab.ColorIndex = -4142
                return "removed the tab color of $($sheet.Name)"
            }
            $sheet.Tab.Color = Color-Int $color
            return "set the tab color of $($sheet.Name)"
        }
        'protect_sheet' {
            $password = [string](Get-Prop $op 'password' '')
            if ([bool](Get-Prop $op 'unprotect' $false)) {
                if ($password -eq '') { $sheet.Unprotect() } else { $sheet.Unprotect($password) }
                return "unprotected $($sheet.Name)"
            }
            if ($password -eq '') { $sheet.Protect() } else { $sheet.Protect($password) }
            return "protected $($sheet.Name)"
        }
        default { throw "Excel 不支持的操作：$name（先调用 office_help app='excel'）" }
    }
}
# --- PowerPoint -------------------------------------------------------------
# The PowerPoint object model uses msoTrue/msoFalse (-1/0) for every boolean,
# so the helpers below convert $true/$false with Mso-Bool instead of passing
# PowerShell booleans straight to COM.

function Slide-Layouts {
    return @{
        title = 1; title_only = 1; title_content = 2; text = 2; title_and_content = 2
        two_content = 3; two_column_text = 3; blank = 12
    }
}

function Mso-Bool([bool]$value) {
    if ($value) { return -1 }
    return 0
}

function Shape-Text($shape) {
    $text = ''
    try {
        if ([int]$shape.HasTextFrame -eq -1) {
            if ([int]$shape.TextFrame.HasText -eq -1) {
                $text = Trim-Cell ([string]$shape.TextFrame.TextRange.Text)
            }
        }
    } catch {}
    return $text
}

function Get-Slide($pres, $op) {
    $index = [int](Get-Prop $op 'slide' 0)
    $total = [int]$pres.Slides.Count
    if ($index -lt 1 -or $index -gt $total) { throw "slide 编号超出范围：$index（共 $total 页）" }
    return $pres.Slides.Item($index)
}

function Get-Slide-Shape($slide, $op) {
    if (Has-Prop $op 'shape') {
        $index = [int]$op.shape
        if ($index -lt 1 -or $index -gt [int]$slide.Shapes.Count) {
            throw "shape 编号超出范围：$index（本页共 $($slide.Shapes.Count) 个）"
        }
        return $slide.Shapes.Item($index)
    }
    $placeholders = @()
    foreach ($shape in $slide.Shapes.Placeholders) { $placeholders += $shape }
    if ($placeholders.Count -eq 0) { throw '这一页没有占位符，请用 shape 指定（先 office_read 看 shapes）' }
    $wanted = 'body'
    if (Has-Prop $op 'placeholder') { $wanted = ([string]$op.placeholder).ToLowerInvariant() }
    if ($wanted -match '^\d+$') {
        $index = [int]$wanted
        if ($index -lt 1 -or $index -gt $placeholders.Count) { throw "占位符编号超出范围：$index" }
        return $placeholders[$index - 1]
    }
    foreach ($shape in $placeholders) {
        $type = 0
        try { $type = [int]$shape.PlaceholderFormat.Type } catch {}
        # 1/3 = title, 2/6 = body, 4 = subtitle (ppPlaceholder*).
        if ($wanted -eq 'title' -and ($type -eq 1 -or $type -eq 3)) { return $shape }
        if ($wanted -eq 'body' -and ($type -eq 2 -or $type -eq 6)) { return $shape }
        if ($wanted -eq 'subtitle' -and $type -eq 4) { return $shape }
        if ($wanted -eq 'any') { return $shape }
    }
    throw "这一页没有「$wanted」占位符；可用 title / body / subtitle / 占位符编号"
}

function Apply-PowerPoint-Format($shape, $op) {
    $range = $shape.TextFrame.TextRange
    if (Has-Prop $op 'bold') { $range.Font.Bold = Mso-Bool ([bool]$op.bold) }
    if (Has-Prop $op 'italic') { $range.Font.Italic = Mso-Bool ([bool]$op.italic) }
    if (Has-Prop $op 'font_size') {
        $size = [double]$op.font_size
        if ($size -lt 4 -or $size -gt 400) { throw 'font_size 需在 4..400 之间' }
        $range.Font.Size = $size
    }
    if (Has-Prop $op 'font') { $range.Font.Name = [string]$op.font }
    if (Has-Prop $op 'color') { $range.Font.Color.RGB = Color-Int ([string]$op.color) }
    if (Has-Prop $op 'alignment') {
        $align = ([string]$op.alignment).ToLowerInvariant()
        # ppAlignLeft = 1, ppAlignCenter = 2, ppAlignRight = 3, ppAlignJustify = 4
        $map = @{ left = 1; center = 2; right = 3; justify = 4 }
        if (-not $map.ContainsKey($align)) { throw 'alignment 只能是 left/center/right/justify' }
        $range.ParagraphFormat.Alignment = $map[$align]
    }
    if (Has-Prop $op 'bullet') { $range.ParagraphFormat.Bullet.Visible = Mso-Bool ([bool]$op.bullet) }
}

function Get-Notes-Shape($slide) {
    # On a notes page every shape reports Type 14 (ppPlaceholderHeader-ish), so the
    # notes body is picked by PlaceholderFormat.Type: 2 = ppPlaceholderBody, 13 =
    # slide number. The slide thumbnail has no text frame at all.
    $fallback = $null
    try {
        foreach ($shape in $slide.NotesPage.Shapes) {
            $hasFrame = $false
            try { $hasFrame = ([int]$shape.HasTextFrame -eq -1) } catch {}
            if (-not $hasFrame) { continue }
            $type = 0
            try { $type = [int]$shape.PlaceholderFormat.Type } catch {}
            if ($type -eq 2) { return $shape }
            if ($type -ne 13 -and $null -eq $fallback) { $fallback = $shape }
        }
    } catch {}
    return $fallback
}

function Notes-Text($slide) {
    $shape = Get-Notes-Shape $slide
    if ($null -eq $shape) { return '' }
    return Shape-Text $shape
}

function Read-PowerPoint($request, $target) {
    $pres = $target.document
    $maxSlides = 60
    if (Has-Prop $request 'max_slides') { $maxSlides = [Math]::Min(200, [Math]::Max(1, [int]$request.max_slides)) }
    $maxShapes = 30
    if (Has-Prop $request 'max_shapes') { $maxShapes = [Math]::Min(60, [Math]::Max(1, [int]$request.max_shapes)) }
    $full = $true
    if (Has-Prop $request 'full') { $full = [bool]$request.full }
    $withNotes = $true
    if (Has-Prop $request 'include_notes') { $withNotes = [bool]$request.include_notes }
    $textLimit = if ($full) { 2000 } else { 200 }
    $total = [int]$pres.Slides.Count
    $count = [Math]::Min($total, $maxSlides)
    $slides = @()
    $cutShapes = $false
    for ($i = 1; $i -le $count; $i++) {
        $slide = $pres.Slides.Item($i)
        $layout = ''
        try { $layout = [string]$slide.CustomLayout.Name } catch {}
        $shapes = @()
        $shapeTotal = [int]$slide.Shapes.Count
        $shapeCount = [Math]::Min($shapeTotal, $maxShapes)
        if ($shapeTotal -gt $shapeCount) { $cutShapes = $true }
        for ($s = 1; $s -le $shapeCount; $s++) {
            $shape = $slide.Shapes.Item($s)
            $text = Shape-Text $shape
            if ($text.Length -gt $textLimit) { $text = $text.Substring(0, $textLimit) + '...' }
            $entry = [ordered]@{ index = $s; name = [string]$shape.Name; type = 0; placeholder = ''; text = $text }
            try { $entry.type = [int]$shape.Type } catch {}
            try {
                $type = [int]$shape.PlaceholderFormat.Type
                if ($type -eq 1 -or $type -eq 3) { $entry.placeholder = 'title' }
                elseif ($type -eq 2 -or $type -eq 6) { $entry.placeholder = 'body' }
                elseif ($type -eq 4) { $entry.placeholder = 'subtitle' }
                elseif ($type -gt 0) { $entry.placeholder = 'other' }
            } catch {}
            if ($entry.type -eq 19) {   # msoTable
                try {
                    $table = $shape.Table
                    $rowCount = [Math]::Min([int]$table.Rows.Count, 20)
                    $colCount = [Math]::Min([int]$table.Columns.Count, 10)
                    $rows = @()
                    for ($r = 1; $r -le $rowCount; $r++) {
                        $cells = @()
                        for ($c = 1; $c -le $colCount; $c++) {
                            $cells += Trim-Cell ([string]$table.Cell($r, $c).Shape.TextFrame.TextRange.Text)
                        }
                        $rows += ,$cells
                    }
                    $entry.table = [ordered]@{ rows = $rowCount; columns = $colCount; data = $rows }
                } catch {}
            }
            try {
                $entry.left = [int]$shape.Left
                $entry.top = [int]$shape.Top
                $entry.width = [int]$shape.Width
                $entry.height = [int]$shape.Height
            } catch {}
            $shapes += [pscustomobject]$entry
        }
        $notes = ''
        if ($withNotes) { $notes = Notes-Text $slide }
        $slides += [pscustomobject]@{
            index = $i
            layout = $layout
            shapes = $shapes
            shapes_total = $shapeTotal
            notes = $notes
        }
    }
    $saved = $false
    try { $saved = [bool]$pres.Saved } catch {}
    $width = 0
    $height = 0
    try {
        $width = [int]$pres.PageSetup.SlideWidth
        $height = [int]$pres.PageSetup.SlideHeight
    } catch {}
    return @{
        ok = $true
        action = 'read'
        app = 'powerpoint'
        path = $target.path
        name = [string]$pres.Name
        saved = $saved
        slide_count = $total
        slides = $slides
        truncated = (($total -gt $count) -or $cutShapes)
        slide_width = $width
        slide_height = $height
        properties = [pscustomobject]@{
            title = Get-Doc-Property $pres 'Title'
            subject = Get-Doc-Property $pres 'Subject'
            author = Get-Doc-Property $pres 'Author'
            keywords = Get-Doc-Property $pres 'Keywords'
        }
    }
}

function Run-PowerPoint-Op($pres, $app, $op) {
    $name = [string]$op.op
    switch ($name) {
        'add_slide' {
            $layoutName = ([string](Get-Prop $op 'layout' 'title_content')).ToLowerInvariant()
            $layouts = Slide-Layouts
            $layout = 0
            if ($layoutName -match '^\d+$') {
                $layout = [int]$layoutName
            } elseif ($layouts.ContainsKey($layoutName)) {
                $layout = $layouts[$layoutName]
            } else {
                throw 'layout 支持 title / title_content / two_content / blank（或 1/2/3/12）'
            }
            $index = [int]$pres.Slides.Count + 1
            if (Has-Prop $op 'index') {
                $index = [Math]::Max(1, [Math]::Min([int]$op.index, [int]$pres.Slides.Count + 1))
            }
            $slide = $pres.Slides.Add($index, $layout)
            return "added slide $($slide.SlideIndex) (layout $layoutName)"
        }
        'set_text' {
            $slide = Get-Slide $pres $op
            $shape = Get-Slide-Shape $slide $op
            if (-not (Has-Prop $op 'text')) { throw 'set_text 需要 text' }
            $shape.TextFrame.TextRange.Text = [string]$op.text
            Apply-PowerPoint-Format $shape $op
            return "set text of slide $($slide.SlideIndex) shape「$($shape.Name)」"
        }
        'format_text' {
            $slide = Get-Slide $pres $op
            $shape = Get-Slide-Shape $slide $op
            Apply-PowerPoint-Format $shape $op
            return "formatted slide $($slide.SlideIndex) shape「$($shape.Name)」"
        }
        'add_textbox' {
            $slide = Get-Slide $pres $op
            if (-not (Has-Prop $op 'text')) { throw 'add_textbox 需要 text' }
            $left = [double](Get-Prop $op 'left' 60)
            $top = [double](Get-Prop $op 'top' 60)
            $width = [double](Get-Prop $op 'width' 520)
            $height = [double](Get-Prop $op 'height' 120)
            # msoTextOrientationHorizontal = 1
            $shape = $slide.Shapes.AddTextbox(1, $left, $top, $width, $height)
            $shape.TextFrame.TextRange.Text = [string]$op.text
            $shape.TextFrame.WordWrap = -1
            Apply-PowerPoint-Format $shape $op
            return "added textbox on slide $($slide.SlideIndex)"
        }
        'add_bullets' {
            $slide = Get-Slide $pres $op
            $items = @(Get-Prop $op 'items' @())
            if ($items.Count -eq 0) { throw 'add_bullets 需要 items' }
            $shape = Get-Slide-Shape $slide $op
            $text = ($items | ForEach-Object { [string]$_ }) -join "`r"
            $shape.TextFrame.TextRange.Text = $text
            $range = $shape.TextFrame.TextRange
            $paragraphs = [int]$range.Paragraphs().Count
            for ($p = 1; $p -le $paragraphs; $p++) {
                $range.Paragraphs($p).ParagraphFormat.Bullet.Visible = -1
            }
            Apply-PowerPoint-Format $shape $op
            return "wrote $($items.Count) bullet(s) on slide $($slide.SlideIndex)"
        }
        'add_table' {
            $slide = Get-Slide $pres $op
            $data = @(Get-Prop $op 'data' @())
            if ($data.Count -eq 0) { throw 'add_table 需要 data（二维数组）' }
            $rows = $data.Count
            $cols = 0
            foreach ($row in $data) { $cols = [Math]::Max($cols, @($row).Count) }
            if ($rows -gt 50 -or $cols -gt 20) { throw '表格过大（最多 50 行、20 列）' }
            $left = [double](Get-Prop $op 'left' 40)
            $top = [double](Get-Prop $op 'top' 120)
            $width = [double](Get-Prop $op 'width' 640)
            $height = [double](Get-Prop $op 'height' (26 * $rows))
            $shape = $slide.Shapes.AddTable($rows, $cols, $left, $top, $width, $height)
            $table = $shape.Table
            for ($r = 1; $r -le $rows; $r++) {
                $row = @($data[$r - 1])
                for ($c = 1; $c -le $cols; $c++) {
                    $text = if ($c -le $row.Count) { [string]$row[$c - 1] } else { '' }
                    $table.Cell($r, $c).Shape.TextFrame.TextRange.Text = $text
                }
            }
            if ([bool](Get-Prop $op 'header' $true)) {
                for ($c = 1; $c -le $cols; $c++) {
                    $table.Cell(1, $c).Shape.TextFrame.TextRange.Font.Bold = -1
                }
            }
            return "added table ${rows}x${cols} on slide $($slide.SlideIndex)"
        }
        'add_image' {
            $slide = Get-Slide $pres $op
            $path = [string](Get-Prop $op 'path' '')
            if ($path -eq '' -or -not (Test-Path -LiteralPath $path)) { throw "图片不存在：$path" }
            $left = [double](Get-Prop $op 'left' 40)
            $top = [double](Get-Prop $op 'top' 40)
            # -1 keeps the picture's own size (msoTrue for the scale arguments).
            $width = -1
            if (Has-Prop $op 'width') { $width = [double]$op.width }
            $height = -1
            if (Has-Prop $op 'height') { $height = [double]$op.height }
            $shape = $slide.Shapes.AddPicture((Full-Path $path), 0, -1, $left, $top, $width, $height)
            return "added image $([System.IO.Path]::GetFileName($path)) on slide $($slide.SlideIndex)"
        }
        'add_shape' {
            $slide = Get-Slide $pres $op
            $kind = ([string](Get-Prop $op 'shape' 'rectangle')).ToLowerInvariant()
            $left = [double](Get-Prop $op 'left' 60)
            $top = [double](Get-Prop $op 'top' 60)
            $width = [double](Get-Prop $op 'width' 240)
            $height = [double](Get-Prop $op 'height' 120)
            $fill = [string](Get-Prop $op 'fill' '')
            $line = [string](Get-Prop $op 'line' '')
            if ($kind -eq 'line') {
                $shape = $slide.Shapes.AddLine($left, $top, $left + $width, $top + $height)
                if ($line -ne '') { $shape.Line.ForeColor.RGB = Color-Int $line }
                return "added line on slide $($slide.SlideIndex)"
            }
            # msoShapeRectangle = 1, Diamond = 4, RoundedRectangle = 5, Oval = 9,
            # RightArrow = 33, Chevron = 52
            $types = @{ rectangle = 1; rounded_rectangle = 5; diamond = 4; oval = 9; arrow = 33; right_arrow = 33; chevron = 52 }
            if (-not $types.ContainsKey($kind)) {
                throw "add_shape 支持 rectangle/rounded_rectangle/diamond/oval/arrow/chevron/line，收到 $kind"
            }
            $shape = $slide.Shapes.AddShape($types[$kind], $left, $top, $width, $height)
            if ($fill -ne '') {
                $shape.Fill.ForeColor.RGB = Color-Int $fill
                $shape.Fill.Solid()
            }
            if ($line -ne '') { $shape.Line.ForeColor.RGB = Color-Int $line }
            if ((Has-Prop $op 'text') -and [string]$op.text -ne '') {
                $shape.TextFrame.TextRange.Text = [string]$op.text
                Apply-PowerPoint-Format $shape $op
            }
            return "added $kind on slide $($slide.SlideIndex)"
        }
        'move_slide' {
            $slide = Get-Slide $pres $op
            $from = [int]$slide.SlideIndex
            $total = [int]$pres.Slides.Count
            $to = [Math]::Max(1, [Math]::Min([int](Get-Prop $op 'to' $total), $total))
            $slide.MoveTo($to)
            return "moved slide $from to $to"
        }
        'duplicate_slide' {
            $slide = Get-Slide $pres $op
            $copy = $slide.Duplicate()
            return "duplicated slide $($slide.SlideIndex) as $($copy.SlideIndex)"
        }
        'delete_slide' {
            $slide = Get-Slide $pres $op
            $expect = [string](Get-Prop $op 'expect' '')
            if ($expect -eq '') { throw 'delete_slide 需要 expect（该页上应有的一段文字）以防误删' }
            $texts = @()
            foreach ($shape in $slide.Shapes) { $texts += (Shape-Text $shape) }
            $all = ($texts -join "`n")
            if ($all.IndexOf($expect, [StringComparison]::Ordinal) -lt 0) {
                throw "第 $($slide.SlideIndex) 页没有「$expect」；请先 office_read"
            }
            if ([int]$pres.Slides.Count -le 1) { throw '不能删除唯一的幻灯片' }
            $index = [int]$slide.SlideIndex
            $slide.Delete()
            return "deleted slide $index"
        }
        'set_notes' {
            $slide = Get-Slide $pres $op
            $text = [string](Get-Prop $op 'text' '')
            if ($text -eq '') { throw 'set_notes 需要 text' }
            $shape = Get-Notes-Shape $slide
            if ($null -eq $shape) { throw '这一页的备注页没有可写文本框' }
            $shape.TextFrame.TextRange.Text = $text
            return "set notes of slide $($slide.SlideIndex)"
        }
        'set_background' {
            $slide = Get-Slide $pres $op
            $color = [string](Get-Prop $op 'color' '')
            if ($color -eq '') { throw 'set_background 需要 color（RRGGBB）' }
            $slide.FollowMasterBackground = 0
            $slide.Background.Fill.ForeColor.RGB = Color-Int $color
            $slide.Background.Fill.Solid()
            return "set background of slide $($slide.SlideIndex)"
        }
        'set_slide_size' {
            $unit = [string](Get-Prop $op 'unit' 'cm')
            $width = Length-Points ([double](Get-Prop $op 'width' 33.87)) $unit
            $height = Length-Points ([double](Get-Prop $op 'height' 19.05)) $unit
            $pres.PageSetup.SlideWidth = $width
            $pres.PageSetup.SlideHeight = $height
            return "slide size is now $([int]$width)x$([int]$height) pt"
        }
        'set_layout' {
            $slide = Get-Slide $pres $op
            $layoutName = ([string](Get-Prop $op 'layout' '')).ToLowerInvariant()
            $layouts = Slide-Layouts
            if (-not $layouts.ContainsKey($layoutName)) { throw 'set_layout 的 layout 支持 title/title_content/two_content/blank' }
            $slide.Layout = $layouts[$layoutName]
            return "applied layout $layoutName to slide $($slide.SlideIndex)"
        }
        default { throw "PowerPoint 不支持的操作：$name（先调用 office_help app='powerpoint'）" }
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
            } elseif ($target.app -eq 'powerpoint') {
                $done += Run-PowerPoint-Op $target.document $target.application $op
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
    # Resolve by state: a `path` here is the save-as destination, not the source.
    # Building the probe by hand avoids touching a missing `path` property on the
    # request (PowerShell cannot add one to a PSCustomObject).
    $probe = @{ app = $null; state_path = (Get-Prop $request 'state_path' '') }
    if (Has-Prop $request 'app') { $probe.app = [string]$request.app }
    $target = Resolve-Document $probe $state $false
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
            } elseif ($target.app -eq 'powerpoint') {
                # ppSaveAsPDF = 32
                [void]$doc.SaveAs($savePath, 32)
            } else {
                $doc.ExportAsFixedFormat(0, $savePath)
            }
            $resultPath = $savePath
        } else {
            if ($target.app -eq 'word') {
                [void]$doc.SaveAs2($savePath)
            } elseif ($target.app -eq 'powerpoint') {
                # ppSaveAsOpenXMLPresentation = 24, so .pptx stays the native format.
                [void]$doc.SaveAs($savePath, 24)
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
    if ($target.app -eq 'word') { $doc.Close(0) } elseif ($target.app -eq 'powerpoint') { $doc.Close() } else { $doc.Close($false) }
    Forget-Owned $state $target.app $target.path
    if ($null -ne $state.last -and ([string]$state.last.path) -eq $target.path) { $state.last = $null }
    Write-State $request.state_path $state
    return @{ ok = $true; action = 'close'; app = $target.app; path = $target.path; saved = $doSave }
}
function Get-Status($state) {
    $result = @()
    foreach ($kind in @('word', 'excel', 'powerpoint')) {
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
            } elseif ($kind -eq 'powerpoint') {
                for ($i = 1; $i -le $app.Presentations.Count; $i++) {
                    $pres = $app.Presentations.Item($i)
                    $full = ''
                    try { $full = [string]$pres.FullName } catch {}
                    $saved = $false
                    try { $saved = [bool]$pres.Saved } catch {}
                    $documents += [pscustomobject]@{
                        name = [string]$pres.Name
                        path = $full
                        saved = $saved
                        slides = $(try { [int]$pres.Slides.Count } catch { 0 })
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

function Matches-Query([string]$text, [string]$query, [bool]$regex, [bool]$matchCase) {
    if ($regex) {
        $options = if ($matchCase) { [System.Text.RegularExpressions.RegexOptions]::None } else { [System.Text.RegularExpressions.RegexOptions]::IgnoreCase }
        try { return [System.Text.RegularExpressions.Regex]::IsMatch($text, $query, $options) }
        catch { throw "正则无效：$($_.Exception.Message)" }
    }
    if ($matchCase) { return $text.Contains($query) }
    return ($text.IndexOf($query, [StringComparison]::OrdinalIgnoreCase) -ge 0)
}

function Cell-Text($value) {
    if ($null -eq $value) { return '' }
    if ($value -is [double] -or $value -is [int] -or $value -is [long] -or $value -is [decimal]) {
        return [Convert]::ToString($value, [System.Globalization.CultureInfo]::InvariantCulture)
    }
    return [string]$value
}

function Find-Word($request, $target) {
    $query = [string]$request.query
    if ($query -eq '') { throw 'office_find 需要 query' }
    $doc = $target.document
    $limit = [Math]::Min(200, [Math]::Max(1, [int](Get-Prop $request 'max_hits' 50)))
    $regex = [bool](Get-Prop $request 'regex' $false)
    $matchCase = [bool](Get-Prop $request 'match_case' $false)
    $hits = @()
    $scanned = 0
    $total = [int]$doc.Paragraphs.Count
    for ($i = 1; $i -le $total; $i++) {
        $para = $doc.Paragraphs.Item($i)
        $text = Trim-Cell ([string]$para.Range.Text)
        $scanned++
        if (-not (Matches-Query $text $query $regex $matchCase)) { continue }
        $style = ''
        try { $style = [string]$para.Style.NameLocal } catch {}
        $hits += [pscustomobject]@{ index = $i; text = (Snippet $text $query); style = $style }
        if ($hits.Count -ge $limit) { break }
    }
    return @{
        ok = $true
        action = 'find'
        app = 'word'
        path = $target.path
        scanned = $scanned
        paragraphs = $total
        hits = $hits
        truncated = (($scanned -lt $total) -or ($hits.Count -ge $limit))
    }
}

function Find-Excel($request, $target) {
    $query = [string]$request.query
    if ($query -eq '') { throw 'office_find 需要 query' }
    $book = $target.document
    $date1904 = $false
    try { $date1904 = [bool]$book.Date1904 } catch {}
    $limit = [Math]::Min(200, [Math]::Max(1, [int](Get-Prop $request 'max_hits' 50)))
    $rowCap = [Math]::Max(1, [Math]::Min(2000, [int](Get-Prop $request 'max_rows' 200)))
    $colCap = [Math]::Max(1, [Math]::Min(100, [int](Get-Prop $request 'max_cols' 40)))
    $regex = [bool](Get-Prop $request 'regex' $false)
    $matchCase = [bool](Get-Prop $request 'match_case' $false)
    $wanted = [string](Get-Prop $request 'sheet' '')
    $names = @()
    if ($wanted -ne '') {
        $names = @($wanted)
    } else {
        foreach ($ws in $book.Worksheets) { $names += [string]$ws.Name }
    }
    if ($names.Count -gt 30) { $names = $names[0..29] }
    $hits = @()
    $scanned = 0
    $limited = ($names.Count -lt [int]$book.Worksheets.Count)
    foreach ($name in $names) {
        if ($hits.Count -ge $limit) { $limited = $true; break }
        $sheet = $book.Worksheets.Item($name)
        $used = $sheet.UsedRange
        $rowStart = [int]$used.Row
        $colStart = [int]$used.Column
        $rowsHere = [Math]::Min([int]$used.Rows.Count, $rowCap)
        $colsHere = [Math]::Min([int]$used.Columns.Count, $colCap)
        if ($rowsHere -lt [int]$used.Rows.Count -or $colsHere -lt [int]$used.Columns.Count) { $limited = $true }
        if ($rowsHere -le 0 -or $colsHere -le 0) { continue }
        $block = $sheet.Range($sheet.Cells.Item($rowStart, $colStart), $sheet.Cells.Item($rowStart + $rowsHere - 1, $colStart + $colsHere - 1))
        # One COM call brings the whole block back; per-cell reads on a large
        # sheet cost seconds, this costs milliseconds. Date cells are matched by the
        # text a person sees, so a numeric cell asks for its own format (the scan is
        # bounded by max_rows/max_cols).
        $data = $block.Value2
        for ($r = 1; $r -le $rowsHere; $r++) {
            for ($c = 1; $c -le $colsHere; $c++) {
                $value = if (($rowsHere -eq 1) -and ($colsHere -eq 1)) { $data } else { $data[$r, $c] }
                if ($null -eq $value) { continue }
                $scanned++
                $text = Cell-Text $value
                if (($value -is [double]) -and ([double]$value -gt 0) -and ([double]$value -le 2958465)) {
                    $format = ''
                    try { $format = [string]$sheet.Cells.Item($rowStart + $r - 1, $colStart + $c - 1).NumberFormat } catch {}
                    if (Test-Date-Format $format) {
                        $dated = Format-Serial-Date ([double]$value) $date1904
                        if ($null -ne $dated) { $text = $dated }
                    }
                }
                if (-not (Matches-Query $text $query $regex $matchCase)) { continue }
                $address = [string]$sheet.Cells.Item($rowStart + $r - 1, $colStart + $c - 1).Address(0, 0)
                $hits += [pscustomobject]@{ sheet = $name; cell = $address; text = (Snippet $text $query) }
                if ($hits.Count -ge $limit) { $limited = $true; break }
            }
            if ($hits.Count -ge $limit) { break }
        }
    }
    return @{
        ok = $true
        action = 'find'
        app = 'excel'
        path = $target.path
        sheets_searched = $names.Count
        scanned = $scanned
        hits = $hits
        truncated = $limited
    }
}

function Find-PowerPoint($request, $target) {
    $query = [string]$request.query
    if ($query -eq '') { throw 'office_find 需要 query' }
    $pres = $target.document
    $limit = [Math]::Min(200, [Math]::Max(1, [int](Get-Prop $request 'max_hits' 50)))
    $maxSlides = [Math]::Min(200, [Math]::Max(1, [int](Get-Prop $request 'max_slides' 100)))
    $regex = [bool](Get-Prop $request 'regex' $false)
    $matchCase = [bool](Get-Prop $request 'match_case' $false)
    $total = [int]$pres.Slides.Count
    $count = [Math]::Min($total, $maxSlides)
    $hits = @()
    $scanned = 0
    $limited = ($count -lt $total)
    for ($i = 1; $i -le $count; $i++) {
        if ($hits.Count -ge $limit) { $limited = $true; break }
        $slide = $pres.Slides.Item($i)
        foreach ($shape in $slide.Shapes) {
            $text = Shape-Text $shape
            if ($text -eq '') { continue }
            $scanned++
            if (Matches-Query $text $query $regex $matchCase) {
                $hits += [pscustomobject]@{ slide = $i; shape = [string]$shape.Name; text = (Snippet $text $query) }
                if ($hits.Count -ge $limit) { $limited = $true; break }
            }
        }
        if ($hits.Count -ge $limit) { break }
        $notes = Notes-Text $slide
        if ($notes -ne '') {
            $scanned++
            if (Matches-Query $notes $query $regex $matchCase) {
                $hits += [pscustomobject]@{ slide = $i; shape = 'notes'; text = (Snippet $notes $query) }
                if ($hits.Count -ge $limit) { $limited = $true }
            }
        }
    }
    return @{
        ok = $true
        action = 'find'
        app = 'powerpoint'
        path = $target.path
        slides_searched = $count
        scanned = $scanned
        hits = $hits
        truncated = $limited
    }
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
            if ($target.app -eq 'word') {
                $result = Read-Word $request $target
            } elseif ($target.app -eq 'powerpoint') {
                $result = Read-PowerPoint $request $target
            } else {
                $result = Read-Excel $request $target
            }
        }
        'find' {
            $target = Resolve-Document $request $state $true
            if ($target.app -eq 'word') {
                $result = Find-Word $request $target
            } elseif ($target.app -eq 'powerpoint') {
                $result = Find-PowerPoint $request $target
            } else {
                $result = Find-Excel $request $target
            }
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
