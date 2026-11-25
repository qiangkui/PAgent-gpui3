import assert from 'node:assert/strict'
import { test } from 'node:test'
import { serverToolDeclarations, webEventRow, webReport, webSteps } from '../src/web.ts'

test('a search call becomes one row with its queries and sources', () => {
  const call = {
    id: 'call_1',
    type: 'deepseek_web_search',
    kind: 'search',
    action: { type: 'search', queries: ['DeepSeek V4', '发布'] },
    status: 'completed',
    sources: [
      { title: 'DeepSeek V4', url: 'https://example.com/v4' },
      { title: '公告', url: 'https://example.com/news' }
    ]
  }
  const [step] = webSteps([call], [], 'round-1')
  assert.equal(step.tool, 'web_search')
  assert.equal(step.target, 'DeepSeek V4 · 发布')
  assert.equal(step.ok, true)
  assert.equal(step.detail, '2 个来源')
  assert.match(step.output, /https:\/\/example\.com\/v4/)
  assert.match(step.output, /公告/)
  assert.equal(step.call, 'round-1-1')
  assert.deepEqual(JSON.parse(step.input), { queries: ['DeepSeek V4', '发布'] })
})

test('a page read names the page it opened', () => {
  const [step] = webSteps(
    [
      {
        type: 'deepseek_read_url',
        kind: 'read_link',
        action: { type: 'read_link', url: 'https://example.com/a' },
        status: 'completed',
        page: { url: 'https://example.com/a', title: '文章 A' }
      }
    ],
    [],
    'round-2'
  )
  assert.equal(step.tool, 'read_url')
  assert.equal(step.target, 'https://example.com/a')
  assert.equal(step.detail, '文章 A')
  assert.equal(step.ok, true)
})

test('a failed call keeps its error code on the row', () => {
  const [step] = webSteps(
    [{ kind: 'search', action: { queries: ['x'] }, status: 'failed', error: { code: 'rate_limited' } }],
    [],
    'round-3'
  )
  assert.equal(step.ok, false)
  assert.equal(step.detail, '失败：rate_limited')
  assert.equal(step.output, 'rate_limited')
})

test('citations alone still produce a row, so a search never goes unseen', () => {
  const steps = webSteps([], [{ type: 'url_citation', url: 'https://example.com/b', title: 'B' }], 'round-4')
  assert.equal(steps.length, 1)
  assert.equal(steps[0].tool, 'web_search')
  assert.equal(steps[0].detail, '1 个来源')
  assert.match(steps[0].output, /https:\/\/example\.com\/b/)
})

test('the report counts searches and fetches and lists each source once', () => {
  const report = webReport(
    [
      {
        kind: 'search',
        sources: [
          { title: 'A', url: 'https://example.com/a' },
          { title: 'A again', url: 'https://example.com/a' }
        ]
      },
      { kind: 'read_link', page: { url: 'https://example.com/b', title: 'B' } },
      { kind: 'deep_search', sources: [{ title: 'C', url: 'https://example.com/c' }] }
    ],
    [{ type: 'url_citation', url: 'https://example.com/a', title: 'A' }]
  )
  assert.equal(report.searches, 2)
  assert.equal(report.fetches, 1)
  assert.deepEqual(report.sources.map((source) => source.url), [
    'https://example.com/a',
    'https://example.com/b',
    'https://example.com/c'
  ])
})

test('a starting call is read into a running row with the id the row keeps', () => {
  const row = webEventRow(
    {
      type: 'call_started',
      step: 1,
      call: { id: 'call_9', kind: 'search', action: { type: 'search', queries: ['a', 'b'] } }
    },
    'round-2',
    1
  )
  assert.deepEqual(row, { call: 'round-2-1', tool: 'web_search', target: 'a · b' })
  // Without an id the round prefix and the position keep the ids aligned with
  // the finished calls, so the live row is replaced rather than duplicated.
  const anonymous = webEventRow(
    { type: 'call_started', call: { kind: 'read_link', action: { url: 'https://e.com' } } },
    'round-2',
    3
  )
  assert.deepEqual(anonymous, { call: 'round-2-3', tool: 'read_url', target: 'https://e.com' })
  assert.equal(webEventRow({ type: 'call_completed' }, 'round-2', 1), null)
})

test('sources are read from whichever list the gateway filled', () => {
  // The Messages format calls it `sources`, the Responses format `results`; a
  // gateway may also spell the link `link` or `href`.
  const [step] = webSteps(
    [
      {
        kind: 'search',
        action: { queries: ['x'] },
        status: 'completed',
        results: [{ title: 'R', link: 'https://example.com/r' }]
      }
    ],
    [],
    'round-7'
  )
  assert.equal(step.detail, '1 个来源')
  assert.match(step.output, /R — https:\/\/example\.com\/r/)
})

test('step-shaped calls are unwrapped, and an opened page counts as a read', () => {
  // The provider groups calls by thinking step: `{ index, reasoning_content,
  // calls: [...] }`. A deep-thinking step may also open a page, which is a read
  // even though its `kind` says `deep_search`.
  const step = {
    index: 3,
    reasoning_content: 'let me open it',
    calls: [
      {
        id: 's1',
        type: 'deepseek_web_search',
        kind: 'search',
        action: { type: 'search', queries: ['github 趋势'] },
        status: 'completed',
        sources: [{ title: 'T', url: 'https://example.com/t' }]
      },
      {
        id: 's2',
        type: 'deepseek_web_search',
        kind: 'deep_search',
        action: { type: 'open_page', url: 'https://example.com/p' },
        status: 'completed',
        page: { url: 'https://example.com/p', title: 'P' }
      }
    ]
  }
  const steps = webSteps([step], [], 'round-1')
  assert.equal(steps.length, 2)
  assert.equal(steps[0].tool, 'web_search')
  assert.equal(steps[0].target, 'github 趋势')
  assert.equal(steps[0].detail, '1 个来源')
  assert.equal(steps[1].tool, 'read_url')
  assert.equal(steps[1].target, 'https://example.com/p')
  assert.equal(steps[1].detail, 'P')
  const report = webReport([step])
  assert.equal(report.searches, 1)
  assert.equal(report.fetches, 1)
  assert.deepEqual(report.sources.map((source) => source.url), [
    'https://example.com/t',
    'https://example.com/p'
  ])
})

test('the declarations are the two DeepSeek server tools', () => {
  assert.deepEqual(serverToolDeclarations(), [
    { type: 'deepseek_web_search' },
    { type: 'deepseek_read_url' }
  ])
})