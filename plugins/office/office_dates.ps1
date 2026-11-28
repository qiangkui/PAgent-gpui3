# Date handling for the Excel side of the Office bridge.
#
# Excel stores dates as serial numbers (`Value2`), so a plain read turns
# 2026-01-05 into 46027 and a plain write turns "2026-01-05" into text. These
# helpers keep the three places that touch dates (read, find, write_range)
# agreeing on one conversion.
#
# Kept in its own file so `dsh-flow/tests/office-dates.test.ts` can dot-source it
# without running the bridge's request loop. The file is deliberately ASCII-only:
# Windows PowerShell 5.1 reads a BOM-less script as ANSI.

<#
.SYNOPSIS
Whether an Excel number format shows a date or a time.
.DESCRIPTION
Quoted literals, bracketed sections (colors, conditions, locales) and escaped
characters are removed first, so `0.00` or `#,##0.00;[Red](#,##0.00)` stay
numbers, while `yyyy-mm-dd`, `yyyy"/"m"/"d` and `[$-804]yyyy/m/d h:mm` do not.
.PARAMETER format
The cell's NumberFormat string.
#>
function Test-Date-Format([string]$format) {
    if ($null -eq $format -or $format.Trim() -eq '') { return $false }
    $text = [System.Text.RegularExpressions.Regex]::Replace($format, '"[^"]*"', '')
    $text = [System.Text.RegularExpressions.Regex]::Replace($text, '\[[^\]]*\]', '')
    $text = [System.Text.RegularExpressions.Regex]::Replace($text, '\\.', '')
    $text = [System.Text.RegularExpressions.Regex]::Replace($text, '[_*].', '')
    return ($text -match '(?i)[ymdhs]')
}

<#
.SYNOPSIS
Turn an Excel serial number into the text a person would see.
.DESCRIPTION
Whole serials become yyyy-MM-dd, fractional ones keep the time, and a serial
below 1 is a time-only cell. The 1904 date system shifts by 1462 days; the
1900 leap-year bug shifts serials 1..59 by one day, which is applied here so a
document made in 1900-01 reads the same way Excel shows it.
.PARAMETER serial
The cell's numeric value.
.PARAMETER date1904
Whether the workbook uses the 1904 date system.
.OUTPUTS
A string, or $null when the number is not a usable date.
#>
function Format-Serial-Date([double]$serial, [bool]$date1904) {
    $value = [Math]::Round($serial, 9)
    if ($date1904) {
        $value = $value + 1462
    } elseif ($value -ge 1 -and $value -lt 60) {
        $value = $value + 1
    }
    if ($value -le 0 -or $value -gt 2958465) { return $null }
    try { $date = [DateTime]::FromOADate($value) } catch { return $null }
    $culture = [System.Globalization.CultureInfo]::InvariantCulture
    if ($value -lt 1) { return $date.ToString('HH:mm:ss', $culture) }
    if (($value - [Math]::Floor($value)) -lt 0.0000001) { return $date.ToString('yyyy-MM-dd', $culture) }
    if ($date.Second -eq 0) { return $date.ToString('yyyy-MM-dd HH:mm', $culture) }
    return $date.ToString('yyyy-MM-dd HH:mm:ss', $culture)
}

<#
.SYNOPSIS
Recognise an ISO date string so write_range can store a real date.
.DESCRIPTION
Only plain, unambiguous shapes are converted: yyyy-MM-dd and yyyy/MM/dd, with
an optional HH:mm or HH:mm:ss. Anything else stays text, so writing "2026-Q1"
or "1/2/2026" cannot silently become a serial number.
.PARAMETER text
The cell value the model asked to write.
.OUTPUTS
A hashtable with `serial` and `format`, or $null when the text is not a date.
#>
function ConvertTo-Date-Cell([string]$text) {
    if ($null -eq $text -or $text.Trim() -eq '') { return $null }
    $value = $text.Trim()
    $shapes = [string[]]@(
        'yyyy-MM-dd', 'yyyy/MM/dd',
        'yyyy-MM-dd HH:mm', 'yyyy/MM/dd HH:mm',
        'yyyy-MM-dd HH:mm:ss', 'yyyy/MM/dd HH:mm:ss'
    )
    $culture = [System.Globalization.CultureInfo]::InvariantCulture
    $when = [DateTime]::MinValue
    if (-not [DateTime]::TryParseExact($value, $shapes, $culture, [System.Globalization.DateTimeStyles]::None, [ref]$when)) {
        return $null
    }
    $format = 'yyyy-mm-dd'
    if ($value -match '\d{1,2}:\d{2}:\d{2}') {
        $format = 'yyyy-mm-dd hh:mm:ss'
    } elseif ($value -match '\d{1,2}:\d{2}') {
        $format = 'yyyy-mm-dd hh:mm'
    }
    return @{ serial = $when.ToOADate(); format = $format }
}