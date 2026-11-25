import assert from 'node:assert/strict'
import { test } from 'node:test'
import {
  askModel,
  DEFAULT_BASE_URL,
  DEFAULT_MODEL,
  historyMessages,
  listModels,
  MAX_HISTORY,
  MAX_HISTORY_TOKENS
} from '../src/ai.ts'

/** One captured request. */
interface Captured {
  url: string
  init: RequestInit
}

/**
 * Build a fetch stub that records the call and answers with `body`.
 * @param body - JSON body to answer with.
 * @param status - HTTP status to answer with.
 * @returns the stub plus the call it recorded.
 */
function stubFetch(body: unknown, status = 200): { fetchImpl: typeof fetch; calls: Captured[] } {
  const calls: Captured[] = []
  const fetchImpl = (async (url: string, init: RequestInit) => {
    calls.push({ url, init })
    return new Response(JSON.stringify(body), {
      status,
      headers: { 'content-type': 'application/json' }
    })
  }) as unknown as typeof fetch
  return { fetchImpl, calls }
}

test('sends the thinking choice with a one-shot question', async () => {
  const stub = stubFetch({ choices: [{ message: { content: 'ok' } }] })
  await askModel(
    { prompt: 'hi', apiKey: 'sk-test', reasoning: 'off' },
    { fetchImpl: stub.fetchImpl, env: {} }
  )
  const body = JSON.parse(String(stub.calls[0].init.body)) as {
    thinking?: { type: string }
    reasoning_effort?: string
  }
  assert.deepEqual(body.thinking, { type: 'disabled' })
  assert.equal(body.reasoning_effort, undefined)
})

test('history keeps the server calls an assistant turn carried', () => {
  // Passing them back is what lets the provider restore the pages it read.
  const call = { id: 's1', kind: 'search', action: { queries: ['x'] } }
  const messages = historyMessages([
    { role: 'user', content: '问' },
    { role: 'assistant', content: '答', serverToolCalls: [call] }
  ])
  assert.equal(messages.length, 2)
  assert.equal(messages[0].server_tool_calls, undefined, 'a user turn carries none')
  assert.deepEqual(messages[1].server_tool_calls, [call])
})

test('sends the prompt to the chat completions endpoint', async () => {
  const stub = stubFetch({ choices: [{ message: { content: ' 你好 ' } }], usage: { total_tokens: 5 } })
  const result = await askModel(
    { prompt: 'hi', system: 'be brief', apiKey: 'sk-test', model: 'deepseek-reasoner' },
    { fetchImpl: stub.fetchImpl, env: {} }
  )

  assert.equal(result.text, '你好')
  assert.equal(result.model, 'deepseek-reasoner')
  assert.deepEqual(result.usage, { total_tokens: 5 })

  const call = stub.calls[0]
  assert.equal(call.url, `${DEFAULT_BASE_URL}/chat/completions`)
  assert.equal(call.init.method, 'POST')
  assert.equal((call.init.headers as Record<string, string>).authorization, 'Bearer sk-test')
  const body = JSON.parse(String(call.init.body)) as {
    model: string
    stream: boolean
    messages: Array<{ role: string; content: string }>
  }
  assert.equal(body.model, 'deepseek-reasoner')
  assert.equal(body.stream, true)
  assert.deepEqual(body.messages, [
    { role: 'system', content: 'be brief' },
    { role: 'user', content: 'hi' }
  ])
})

test('falls back to the environment and trims the base url', async () => {
  const stub = stubFetch({ choices: [{ message: { content: 'ok' } }] })
  const result = await askModel(
    { prompt: 'hi', baseUrl: 'https://example.test/v1/', model: '  ' },
    {
      fetchImpl: stub.fetchImpl,
      env: { DEEPSEEK_API_KEY: 'sk-env', DEEPSEEK_MODEL: 'deepseek-chat' }
    }
  )

  assert.equal(result.model, 'deepseek-chat')
  assert.equal(stub.calls[0].url, 'https://example.test/v1/chat/completions')
  assert.equal((stub.calls[0].init.headers as Record<string, string>).authorization, 'Bearer sk-env')
})

test('uses the documented defaults when nothing is configured', async () => {
  const stub = stubFetch({ choices: [{ message: { content: 'ok' } }] })
  const result = await askModel({ prompt: 'hi', apiKey: 'sk-test' }, { fetchImpl: stub.fetchImpl, env: {} })
  assert.equal(result.model, DEFAULT_MODEL)
  assert.equal(stub.calls[0].url, `${DEFAULT_BASE_URL}/chat/completions`)
})

test('explains a missing key instead of calling the provider', async () => {
  const stub = stubFetch({ choices: [] })
  await assert.rejects(
    askModel({ prompt: 'hi' }, { fetchImpl: stub.fetchImpl, env: {} }),
    /缺少 DeepSeek API Key/
  )
  assert.equal(stub.calls.length, 0)
})

test('surfaces the provider error message', async () => {
  const stub = stubFetch({ error: { message: 'Authentication Fails' } }, 401)
  await assert.rejects(
    askModel({ prompt: 'hi', apiKey: 'bad' }, { fetchImpl: stub.fetchImpl, env: {} }),
    // The label names the model that was actually called, so it follows the
    // default rather than a hard-coded id.
    new RegExp(`${DEFAULT_MODEL} 调用失败：Authentication Fails`)
  )
})

test('rejects an empty answer', async () => {
  const stub = stubFetch({ choices: [{ message: { content: '   ' } }] })
  await assert.rejects(
    askModel({ prompt: 'hi', apiKey: 'sk-test' }, { fetchImpl: stub.fetchImpl, env: {} }),
    /没有返回内容/
  )
})

test('replays the conversation before the new question', async () => {
  const stub = stubFetch({ choices: [{ message: { content: 'ok' } }] })
  await askModel(
    {
      prompt: '那它呢？',
      apiKey: 'sk-test',
      history: [
        { role: 'user', content: 'a.ts 是什么？' },
        { role: 'assistant', content: '一个工具函数。' },
        { role: 'tool', content: 'ignored' },
        { role: 'user', content: '   ' }
      ]
    },
    { fetchImpl: stub.fetchImpl, env: {} }
  )
  const body = JSON.parse(String(stub.calls[0].init.body)) as {
    messages: Array<{ role: string; content: string }>
  }
  assert.deepEqual(body.messages, [
    { role: 'user', content: 'a.ts 是什么？' },
    { role: 'assistant', content: '一个工具函数。' },
    { role: 'user', content: '那它呢？' }
  ])
})

test('keeps only the most recent turns', async () => {
  const stub = stubFetch({ choices: [{ message: { content: 'ok' } }] })
  const history = Array.from({ length: MAX_HISTORY + 6 }, (_, index) => ({
    role: index % 2 === 0 ? 'user' : 'assistant',
    content: `turn ${index}`
  }))
  await askModel({ prompt: 'now', apiKey: 'sk-test', history }, { fetchImpl: stub.fetchImpl, env: {} })
  const body = JSON.parse(String(stub.calls[0].init.body)) as { messages: Array<{ content: string }> }
  assert.equal(body.messages.length, MAX_HISTORY + 1)
  assert.equal(body.messages[0].content, `turn ${6}`)
  assert.equal(body.messages[body.messages.length - 1].content, 'now')
})

test('drops the oldest turns past the token budget', async () => {
  const stub = stubFetch({ choices: [{ message: { content: 'ok' } }] })
  // Chinese text is about one token per character, so this is the unit the
  // budget is expressed in.
  const big = '字'.repeat(MAX_HISTORY_TOKENS / 2 + 1)
  await askModel(
    {
      prompt: 'now',
      apiKey: 'sk-test',
      history: [
        { role: 'user', content: big },
        { role: 'assistant', content: big },
        { role: 'user', content: 'question 2' }
      ]
    },
    { fetchImpl: stub.fetchImpl, env: {} }
  )
  const body = JSON.parse(String(stub.calls[0].init.body)) as {
    messages: Array<{ content: string }>
  }
  assert.deepEqual(body.messages.map((message) => message.content), [
    big,
    'question 2',
    'now'
  ])
})

test('keeps one oversized turn instead of sending no history at all', async () => {
  const stub = stubFetch({ choices: [{ message: { content: 'ok' } }] })
  const big = '字'.repeat(MAX_HISTORY_TOKENS + 1)
  await askModel(
    { prompt: 'now', apiKey: 'sk-test', history: [{ role: 'user', content: big }] },
    { fetchImpl: stub.fetchImpl, env: {} }
  )
  const body = JSON.parse(String(stub.calls[0].init.body)) as {
    messages: Array<{ content: string }>
  }
  assert.equal(body.messages.length, 2)
  assert.equal(body.messages[0].content, big)
  assert.equal(body.messages[1].content, 'now')
})

test('sends images as image_url parts beside the text', async () => {
  const stub = stubFetch({ choices: [{ message: { content: 'ok' } }] })
  await askModel(
    {
      prompt: '这张图里是什么？',
      apiKey: 'sk-test',
      attachments: [
        { kind: 'image', name: 'shot.png', mimeType: 'image/png', data: 'aGVsbG8=' }
      ]
    },
    { fetchImpl: stub.fetchImpl, env: {} }
  )
  const body = JSON.parse(String(stub.calls[0].init.body)) as {
    messages: Array<{ role: string; content: unknown }>
  }
  assert.deepEqual(body.messages, [
    {
      role: 'user',
      content: [
        { type: 'text', text: '这张图里是什么？' },
        { type: 'image_url', image_url: { url: 'data:image/png;base64,aGVsbG8=' } }
      ]
    }
  ])
})

test('sends PDFs as file parts', async () => {
  const stub = stubFetch({ choices: [{ message: { content: 'ok' } }] })
  await askModel(
    {
      prompt: '总结这份 PDF',
      apiKey: 'sk-test',
      attachments: [
        { kind: 'pdf', name: 'spec.pdf', mimeType: 'application/pdf', data: 'JVBERi0=' }
      ]
    },
    { fetchImpl: stub.fetchImpl, env: {} }
  )
  const body = JSON.parse(String(stub.calls[0].init.body)) as {
    messages: Array<{ content: unknown }>
  }
  assert.deepEqual(body.messages[0]!.content, [
    { type: 'text', text: '总结这份 PDF' },
    { type: 'file', file: { filename: 'spec.pdf', file_data: 'data:application/pdf;base64,JVBERi0=' } }
  ])
})

test('keeps plain text a string when an attachment has no bytes', async () => {
  const stub = stubFetch({ choices: [{ message: { content: 'ok' } }] })
  await askModel(
    {
      prompt: 'hi',
      apiKey: 'sk-test',
      attachments: [{ kind: 'image', name: 'x.png', mimeType: 'image/png', data: '  ' }]
    },
    { fetchImpl: stub.fetchImpl, env: {} }
  )
  const body = JSON.parse(String(stub.calls[0].init.body)) as {
    messages: Array<{ content: unknown }>
  }
  assert.equal(body.messages[0]!.content, 'hi')
})
