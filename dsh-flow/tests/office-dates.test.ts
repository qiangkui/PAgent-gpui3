// Excel stores dates as serial numbers, so the bridge converts them on the way
// in and out: read/find show the text a person sees, and write_range with an
// ISO string stores a real date. Those three share one small dot-sourceable
// script, which this test loads directly — no Office needed.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { execFile } from 'node:child_process'
import { promisify } from 'node:util'
import { fileURLToPath } from 'node:url'

const run = promisify(execFile)
const script = fileURLToPath(new URL('../../plugins/office/office_dates.ps1', import.meta.url))
const skip = process.platform === 'win32' ? false : '需要 Windows PowerShell'

/** Run PowerShell with the date helpers loaded and return the JSON it printed. */
async function ask(body: string): Promise<Record<string, unknown>> {
  const quoted = `'${script.replace(/'/g, "''")}'`
  const command = `. ${quoted}; ${body} | ConvertTo-Json -Compress`
  const { stdout } = await run(
    'powershell.exe',
    ['-NoProfile', '-NonInteractive', '-Command', command],
    { windowsHide: true }
  )
  const text = stdout.trim()
  assert.ok(text !== '', `PowerShell 没有输出：${command}`)
  return JSON.parse(text) as Record<string, unknown>
}

test('date number formats are told apart from plain numbers', { skip }, async () => {
  const formats = await ask(
    [
      '$a = Test-Date-Format "yyyy-mm-dd"',
      '$b = Test-Date-Format "hh:mm:ss"',
      '$c = Test-Date-Format "[$-804]yyyy/m/d h:mm"',
      '$d = Test-Date-Format "0.00"',
      '$e = Test-Date-Format "General"',
      '$f = Test-Date-Format "#,##0.00_);[Red](#,##0.00)"',
      '$g = Test-Date-Format "0.00E+00"',
      `$h = Test-Date-Format '"2026"0.00'`,
      '[ordered]@{ a = $a; b = $b; c = $c; d = $d; e = $e; f = $f; g = $g; h = $h }'
    ].join('; ')
  )
  assert.deepEqual(formats, { a: true, b: true, c: true, d: false, e: false, f: false, g: false, h: false })
})

test('serials become the text Excel shows, including time and 1904 workbooks', { skip }, async () => {
  const dates = await ask(
    [
      '$a = Format-Serial-Date 46027 $false',
      '$b = Format-Serial-Date 46027.5 $false',
      '$c = Format-Serial-Date 0.5 $false',
      '$d = Format-Serial-Date 0 $true',
      '$e = Format-Serial-Date 0 $false',
      '$f = Format-Serial-Date 1 $false',
      '[ordered]@{ a = $a; b = $b; c = $c; d = $d; e = $e; f = $f }'
    ].join('; ')
  )
  assert.deepEqual(dates, {
    a: '2026-01-05',
    // Seconds are dropped when a cell lands exactly on the minute.
    b: '2026-01-05 12:00',
    c: '12:00:00',
    // Serial 0 is 1904-01-01 in the 1904 date system.
    d: '1904-01-01',
    e: null,
    // Excel shows serial 1 as 1900-01-01; OLE automation counts it as 1899-12-31.
    f: '1900-01-01'
  })
})

test('only ISO date strings are written as dates', { skip }, async () => {
  const cells = await ask(
    [
      '$a = ConvertTo-Date-Cell "2026-01-05"',
      '$b = ConvertTo-Date-Cell "2026-01-05 09:30"',
      '$c = ConvertTo-Date-Cell "2026-01-05 09:30:15"',
      '$d = ConvertTo-Date-Cell "hello"',
      '$e = ConvertTo-Date-Cell "2026/1/5"',
      '$f = ConvertTo-Date-Cell "1/2/2026"',
      '$g = ConvertTo-Date-Cell "=SUM(A1:A3)"',
      '[ordered]@{ a = $a.serial; b = $b.format; c = $c.format; d = $null -ne $d; e = $null -ne $e; f = $null -ne $f; g = $null -ne $g }'
    ].join('; ')
  )
  assert.deepEqual(cells, {
    a: 46027,
    b: 'yyyy-mm-dd hh:mm',
    c: 'yyyy-mm-dd hh:mm:ss',
    d: false,
    e: false,
    f: false,
    g: false
  })
})