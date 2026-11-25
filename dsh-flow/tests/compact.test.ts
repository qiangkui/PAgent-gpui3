import assert from 'node:assert/strict'
import { test } from 'node:test'
import {
  DROPPED_RANGE_NOTE,
  PRUNE_MARKER,
  compactLimits,
  compactTranscript,
  estimateTokens,
  fitForModel,
  pruneResult,
  selectSummaryRange,
  transcriptTokens,
  wireMessageTokens
} from '../src/compact.ts'
import type { CompactMessage, SummaryTarget } from '../src/compact.ts'

/**
 * A summarizer target whose fetch always fails immediately.
 *
 * Not a transient error, so the retry loop is skipped and the call falls
 * straight through to the deterministic `drop` layer.
 * @returns the target.
 */
function offlineTarget(): SummaryTarget {
  return {
    baseUrl: 'http://127.0.0.1:9',
    apiKey: 'test',
    model: 'test-model',
    maxTokens: 256,
    fetchImpl: (() => {
      throw new Error('offline')
    }) as unknown as typeof fetch
  }
}

/** A tool call carrying `chars` characters of argument text. */
function writeCall(chars: number): unknown[] {
  return [
    {
      id: 'call_1',
      type: 'function',
      function: {
        name: 'write',
        arguments: JSON.stringify({ path: 'a.txt', content: 'x'.repeat(chars) })
      }
    }
  ]
}

test('tool-call arguments count towards the transcript size', () => {
  const call = writeCall(40_000)
  const message: CompactMessage = { role: 'assistant', content: '', tool_calls: call }
  // The arguments ride in `tool_calls`, not in `content`; a measurement that
  // only read `content` saw an empty message where the prompt held 10k tokens.
  assert.equal(wireMessageTokens(message), estimateTokens(JSON.stringify(call)))
  assert.ok(transcriptTokens([message]) >= 10_000)
  assert.equal(transcriptTokens([{ role: 'assistant', content: '' }]), 0)
})

test('a huge tool call alone pushes the transcript past the pressure line', async () => {
  const limits = compactLimits(20_000)
  const messages: CompactMessage[] = [
    { role: 'system', content: 'sys' },
    { role: 'user', content: 'q1' },
    { role: 'assistant', content: 'a1' },
    { role: 'user', content: 'q2' },
    { role: 'assistant', content: '', tool_calls: writeCall(100_000) },
    { role: 'tool', tool_call_id: 'call_1', content: '已写入 a.txt' },
    { role: 'user', content: 'q3' }
  ]
  assert.ok(transcriptTokens(messages) > limits.pressureTokens)

  const report = await compactTranscript(messages, { limits, target: offlineTarget() })
  // The summarizer is offline, so the deterministic layer replaces the oldest
  // whole turn and the question still goes out.
  assert.equal(report.changed, true)
  assert.equal(report.layer, 'drop')
  assert.ok(report.before > limits.pressureTokens)
  assert.equal(messages[1]!.content, DROPPED_RANGE_NOTE)
  assert.ok(messages.some((message) => message.content === 'q3'))
  assert.ok(messages.length < 7)
})

test('a transcript below the pressure line is left alone', async () => {
  const messages: CompactMessage[] = [
    { role: 'system', content: 'sys' },
    { role: 'user', content: 'hi' }
  ]
  const report = await compactTranscript(messages, {
    limits: compactLimits(128_000),
    target: offlineTarget()
  })
  assert.equal(report.changed, false)
  assert.equal(report.layer, 'none')
  assert.equal(messages.length, 2)
})

test('a summary range starts at a user message and keeps the newest question', () => {
  const limits = compactLimits(20_000)
  // The retained tail is 16% of the window: the turn before the newest one has
  // to be heavier than that before anything older may be summarized.
  const heavy = 'x'.repeat(limits.retainTokens * 4)
  const messages: CompactMessage[] = [
    { role: 'system', content: 'sys' },
    { role: 'user', content: 'q1' },
    { role: 'assistant', content: 'a1' },
    { role: 'tool', tool_call_id: 'c', content: 'result' },
    { role: 'user', content: heavy },
    { role: 'assistant', content: 'a2' },
    { role: 'user', content: 'newest' }
  ]
  const range = selectSummaryRange(messages, limits)
  assert.notEqual(range, null)
  assert.equal(messages[range!.start]!.role, 'user')
  // The kept span must open on a user message, so a tool call is never
  // separated from the results it produced.
  assert.equal(messages[range!.end]!.role, 'user')
  assert.ok(range!.end <= messages.length - 1)
  // A transcript with nothing older worth keeping is left alone.
  assert.equal(
    selectSummaryRange(messages.slice(4), limits),
    null
  )
})

test('a budget smaller than its own note still returns a bounded slice', () => {
  const text = 'x'.repeat(200)
  // The note alone is longer than this budget; the helper must not slice with
  // a negative length (which would keep the whole text).
  const tiny = fitForModel(text, 10)
  assert.equal(tiny, 'x'.repeat(10))
})

test('pruning keeps the head and the tail of an oversized result', () => {
  const text = `${'HEAD'.repeat(3_000)}TAIL-MARK`
  const pruned = pruneResult(text)
  assert.ok(pruned.includes(PRUNE_MARKER))
  assert.ok(pruned.startsWith('HEAD'))
  assert.ok(pruned.endsWith('TAIL-MARK'))
  assert.ok(pruned.length < text.length)
})
