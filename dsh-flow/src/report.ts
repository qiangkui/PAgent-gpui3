/**
 * Self-contained HTML run report.
 *
 * One file, no server, no assets: open it after a failure to see which step
 * broke, what it printed, and what the run had in scope at that point.
 */

import type { RunSummary } from './engine.ts'
import type { LoadedWorkflow } from './load.ts'
import type { StepResult } from './types.ts'

/**
 * Render a run summary as a standalone HTML document.
 * @param loaded - the loaded manifest.
 * @param summary - the settled run.
 * @returns HTML text.
 */
export function renderReport(loaded: LoadedWorkflow, summary: RunSummary): string {
  const { workflow } = loaded
  const rows = Object.values(summary.steps).map(result => stepSection(result)).join('\n')
  const status = summary.status === 'ok' ? 'ok' : 'failed'
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${escapeHtml(workflow.name)} — ${escapeHtml(status)}</title>
<style>
:root { color-scheme: light dark; --ok:#1a7f37; --fail:#c62828; --skip:#8a8a8a; --line:#d8d8d8; --muted:#666; }
* { box-sizing: border-box; }
body { margin:0; padding:24px; font:14px/1.5 -apple-system,"Segoe UI",system-ui,sans-serif; }
h1 { font-size:20px; margin:0 0 4px; }
.sub { color:var(--muted); margin:0 0 20px; }
.badge { display:inline-block; padding:2px 8px; border-radius:10px; font-size:12px; font-weight:600; color:#fff; }
.badge.ok { background:var(--ok); } .badge.failed { background:var(--fail); }
table { border-collapse:collapse; width:100%; margin-bottom:24px; }
th,td { text-align:left; padding:6px 10px; border-bottom:1px solid var(--line); vertical-align:top; }
th { font-size:12px; text-transform:uppercase; letter-spacing:.04em; color:var(--muted); }
tr.failed td { background:rgba(198,40,40,.08); }
code,pre { font-family:ui-monospace,SFMono-Regular,Consolas,monospace; font-size:12px; }
pre { margin:0; padding:8px; overflow:auto; max-height:360px; background:rgba(127,127,127,.10); border-radius:6px; }
details { margin:4px 0; } summary { cursor:pointer; color:var(--muted); }
.result { font-size:13px; margin:2px 0; }
.artifacts { margin:4px 0 0 18px; padding:0; font-size:12px; }
.artifacts .path { color:var(--muted); font-size:11px; }
.status-ok { color:var(--ok); } .status-failed { color:var(--fail); } .status-skipped { color:var(--skip); }
</style>
</head>
<body>
<h1>${escapeHtml(workflow.name)} <span class="badge ${summary.status === 'ok' ? 'ok' : 'failed'}">${escapeHtml(status)}</span></h1>
<p class="sub">${escapeHtml(workflow.description)}<br>
run <code>${escapeHtml(summary.runId)}</code> · ${summary.durationMs} ms · ${Object.keys(summary.steps).length} steps${
  summary.failedStep === undefined ? '' : ` · failed at <code>${escapeHtml(summary.failedStep)}</code>`
}</p>
${summary.error === undefined ? '' : `<p class="status-failed">${escapeHtml(summary.error)}</p>`}
<table>
<thead><tr><th>Step</th><th>Status</th><th>Duration</th><th>Output</th></tr></thead>
<tbody>
${rows}
</tbody>
</table>
<h2 style="font-size:16px;margin:0 0 8px">Vars</h2>
<pre>${escapeHtml(JSON.stringify(summary.vars, null, 2))}</pre>
</body>
</html>
`
}

/**
 * Render one step row plus its expandable detail section.
 * @param result - the settled step result.
 * @returns HTML table rows.
 */
function stepSection(result: StepResult): string {
  const details: string[] = []
  if (result.summary !== undefined) {
    details.push(`<div class="result">${escapeHtml(result.summary)}</div>`)
  }
  if (result.errorInfo !== undefined) {
    const info = result.errorInfo
    const parts = [`<strong>${escapeHtml(info.kind)}</strong>`, escapeHtml(info.message)]
    if (info.hint !== undefined) parts.push(`<em>${escapeHtml(info.hint)}</em>`)
    if (info.where !== undefined) parts.push(`<code>${escapeHtml(info.where)}</code>`)
    if (info.expected !== undefined || info.gotten !== undefined) {
      parts.push(
        `期望 <code>${escapeHtml(info.expected ?? '—')}</code> · 实际 <code>${escapeHtml(info.gotten ?? '—')}</code>`,
      )
    }
    details.push(`<div class="status-failed">${parts.join(' · ')}</div>`)
  }
  if (result.artifacts !== undefined && result.artifacts.length > 0) {
    const items = result.artifacts
      .map((artifact) => `<li><code>${escapeHtml(artifact.name)}</code> ${escapeHtml(artifact.kind)} · ${artifact.bytes} B<br><span class="path">${escapeHtml(artifact.path)}</span></li>`)
      .join('')
    details.push(`<details open><summary>产物 (${result.artifacts.length})</summary><ul class="artifacts">${items}</ul></details>`)
  }
  if (result.command !== undefined) details.push(`<details><summary>command</summary><pre>${escapeHtml(result.command)}</pre></details>`)
  if (result.stdout !== undefined && result.stdout.trim() !== '') {
    details.push(`<details><summary>stdout</summary><pre>${escapeHtml(clip(result.stdout))}</pre></details>`)
  }
  if (result.stderr !== undefined && result.stderr.trim() !== '') {
    details.push(`<details><summary>stderr</summary><pre>${escapeHtml(clip(result.stderr))}</pre></details>`)
  }
  if (result.json !== undefined) {
    details.push(`<details><summary>json</summary><pre>${escapeHtml(clip(JSON.stringify(result.json, null, 2)))}</pre></details>`)
  }
  if (result.error !== undefined) details.push(`<div class="status-failed">${escapeHtml(result.error)}</div>`)
  const meta = [
    result.kind,
    result.model === undefined ? '' : `model ${result.model}`,
    result.cached === true ? 'reused' : '',
  ].filter(entry => entry !== '').join(' · ')
  const rowClass = result.status === 'failed' ? ' class="failed"' : ''
  return `<tr${rowClass}><td><code>${escapeHtml(result.id)}</code><br><span class="status-${result.status}">${escapeHtml(meta)}</span></td>
<td class="status-${result.status}">${escapeHtml(result.status)}</td>
<td>${result.durationMs} ms</td>
<td>${details.join('')}</td></tr>`
}

/**
 * Keep very long output from bloating the report.
 * @param text - the captured text.
 * @returns text capped at 20000 characters.
 */
function clip(text: string): string {
  return text.length > 20000 ? `${text.slice(0, 20000)}\n… truncated (${text.length} chars)` : text
}

/**
 * Escape text for HTML text and attribute contexts.
 * @param text - the text to escape.
 * @returns escaped text.
 */
function escapeHtml(text: string): string {
  return text
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
}
