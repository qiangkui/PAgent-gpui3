import assert from 'node:assert/strict'
import { test } from 'node:test'
import { streamCompletion } from '../src/stream.ts'

/**
 * A response whose body is an event stream built from the given pieces.
 * @param pieces - raw SSE text, split so chunks can end mid-line.
 * @returns the response.
 */
function sse(pieces: string[]): Response {
  const encoder = new TextEncoder()
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      for (const piece of pieces) controller.enqueue(encoder.encode(piece))
      controller.close()
    }
  })
  return new Response(body, { status: 200, headers: { 'content-type': 'text/event-stream' } })
}

/**
 * A response carrying one complete JSON answer.
 * @param body - the JSON value.
 * @param status - HTTP status.
 * @returns the response.
 */
function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' }
  })
}

/**
 * A fetch stub that answers with one prepared response.
 * @param response - what to answer.
 * @returns the stub plus the recorded call.
 */
function stubFetch(response: Response): {
  fetchImpl: typeof fetch
  calls: Array<{ url: string; init: RequestInit }>
} {
  const calls: Array<{ url: string; init: RequestInit }> = []
  const fetchImpl = (async (url: string, init: RequestInit) => {
    calls.push({ url, init })
    return response
  }) as unknown as typeof fetch
  return { fetchImpl, calls }
}

test('reports text fragments as they arrive', async () => {
  const stub = stubFetch(
    sse([
      'data: {"choices":[{"delta":{"content":"你"}}]}\n\n',
      'data: {"choices":[{"delta":{"content":"好"}}]}\n\n',
      'data: [DONE]\n\n'
    ])
  )
  const seen: string[] = []
  const result = await streamCompletion({
    url: 'https://example.test/chat/completions',
    apiKey: 'sk-test',
    label: 'deepseek-chat',
    body: { model: 'deepseek-chat', stream: true },
    fetchImpl: stub.fetchImpl,
    onDelta: (text) => seen.push(text)
  })

  assert.equal(result.content, '你好')
  assert.deepEqual(result.toolCalls, [])
  assert.deepEqual(seen, ['你', '好'])
  const body = JSON.parse(String(stub.calls[0].init.body)) as { stream: boolean }
  assert.equal(body.stream, true)
})

test('puts a tool call back together from its pieces', async () => {
  const stub = stubFetch(
    sse([
      'data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"read","arguments":"{\\"pa',
      'th\\":\\"a.txt\\"}"}}]}}]}\n\n',
      'data: {"choices":[{"delta":{"tool_calls":[{"index":1,"id":"call_2","function":{"name":"glob","arguments":"{}"}}]}}]}\n\n',
      'data: [DONE]\n\n'
    ])
  )
  const result = await streamCompletion({
    url: 'https://example.test/chat/completions',
    apiKey: 'sk-test',
    label: 'deepseek-chat',
    body: {},
    fetchImpl: stub.fetchImpl
  })

  assert.equal(result.content, '')
  assert.equal(result.toolCalls.length, 2)
  assert.deepEqual(result.toolCalls[0], {
    id: 'call_1',
    type: 'function',
    function: { name: 'read', arguments: '{"path":"a.txt"}' }
  })
  assert.equal(result.toolCalls[1].function.name, 'glob')
})

test('does not append a tool name that a gateway repeats in every fragment', async () => {
  const stub = stubFetch(
    sse([
      'data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"read","arguments":"{\\"path\\":"}}]}}]}\n\n',
      'data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"read","arguments":"\\"a.txt\\"}"}}]}}]}\n\n',
      'data: [DONE]\n\n'
    ])
  )
  const result = await streamCompletion({
    url: 'https://example.test/chat/completions',
    apiKey: 'sk-test',
    label: 'deepseek-chat',
    body: {},
    fetchImpl: stub.fetchImpl
  })
  assert.equal(result.toolCalls.length, 1)
  assert.equal(result.toolCalls[0].function.name, 'read')
  assert.equal(result.toolCalls[0].function.arguments, '{"path":"a.txt"}')
})

test('normalizes arguments that a gateway already parsed', async () => {
  const stub = stubFetch(
    json({
      choices: [
        {
          message: {
            tool_calls: [
              { index: 0, id: 'c1', function: { name: 'read', arguments: { path: 'a.txt' } } }
            ]
          }
        }
      ]
    })
  )
  const result = await streamCompletion({
    url: 'https://example.test/chat/completions',
    apiKey: 'sk-test',
    label: 'deepseek-chat',
    body: {},
    fetchImpl: stub.fetchImpl
  })
  assert.equal(result.toolCalls[0].function.name, 'read')
  assert.equal(result.toolCalls[0].function.arguments, '{"path":"a.txt"}')
})

test('reads an event stream that a proxy labels text/plain', async () => {
  const stub = stubFetch(
    new Response('data: {"choices":[{"delta":{"content":"代理"}}]}\n\n', {
      status: 200,
      headers: { 'content-type': 'text/plain' }
    })
  )
  const result = await streamCompletion({
    url: 'https://example.test/chat/completions',
    apiKey: 'sk-test',
    label: 'deepseek-chat',
    body: {},
    fetchImpl: stub.fetchImpl
  })
  assert.equal(result.content, '代理')
})

test('accepts one whole JSON answer that is labelled as a stream', async () => {
  const stub = stubFetch(
    sse([
      `${JSON.stringify({
        choices: [{ message: { content: '整段' } }],
        usage: { total_tokens: 3 }
      })}\n`
    ])
  )
  const result = await streamCompletion({
    url: 'https://example.test/chat/completions',
    apiKey: 'sk-test',
    label: 'deepseek-chat',
    body: {},
    fetchImpl: stub.fetchImpl
  })
  assert.equal(result.content, '整段')
  assert.deepEqual(result.usage, { total_tokens: 3 })
})

test('reads a final event that arrives without a trailing newline', async () => {
  const stub = stubFetch(sse(['data: {"choices":[{"delta":{"content":"尾"}}]}']))
  const result = await streamCompletion({
    url: 'https://example.test/chat/completions',
    apiKey: 'sk-test',
    label: 'deepseek-chat',
    body: {},
    fetchImpl: stub.fetchImpl
  })
  assert.equal(result.content, '尾')
})

test('keeps usage and ignores comment lines', async () => {
  const stub = stubFetch(
    sse([
      ': keep-alive\n\n',
      'data: {"choices":[{"delta":{"content":"ok"}}],"usage":{"total_tokens":7}}\n\n',
      'data: [DONE]\n\n'
    ])
  )
  const result = await streamCompletion({
    url: 'https://example.test/chat/completions',
    apiKey: 'sk-test',
    label: 'deepseek-chat',
    body: {},
    fetchImpl: stub.fetchImpl
  })
  assert.equal(result.content, 'ok')
  assert.deepEqual(result.usage, { total_tokens: 7 })
})

test('reads a provider that answers with one JSON body', async () => {
  const stub = stubFetch(json({ choices: [{ message: { content: '整段回答' } }], usage: { total: 1 } }))
  const seen: string[] = []
  const result = await streamCompletion({
    url: 'https://example.test/chat/completions',
    apiKey: 'sk-test',
    label: 'deepseek-chat',
    body: {},
    fetchImpl: stub.fetchImpl,
    onDelta: (text) => seen.push(text)
  })

  assert.equal(result.content, '整段回答')
  assert.deepEqual(result.usage, { total: 1 })
  assert.deepEqual(seen, [])
})

test('reports a failure before the stream starts', async () => {
  const stub = stubFetch(json({ error: { message: 'invalid api key' } }, 401))
  await assert.rejects(
    streamCompletion({
      url: 'https://example.test/chat/completions',
      apiKey: 'sk-test',
      label: 'deepseek-chat',
      body: {},
      fetchImpl: stub.fetchImpl
    }),
    (error: Error) => error.message === 'deepseek-chat 调用失败：invalid api key'
  )
})

test('reports a failure inside the stream', async () => {
  const stub = stubFetch(sse(['data: {"error":{"message":"boom"}}\n\n']))
  await assert.rejects(
    streamCompletion({
      url: 'https://example.test/chat/completions',
      apiKey: 'sk-test',
      label: 'deepseek-chat',
      body: {},
      fetchImpl: stub.fetchImpl
    }),
    (error: Error) => error.message.includes('boom')
  )
})

test('asks again without the token fields when the endpoint refuses them', async () => {
  const responses = [
    json({ error: { message: 'unknown field: stream_options' } }, 400),
    json({ choices: [{ message: { content: 'ok' } }], usage: { total_tokens: 12 } })
  ]
  const calls: Array<{ url: string; init: RequestInit }> = []
  const fetchImpl = (async (url: string, init: RequestInit) => {
    calls.push({ url, init })
    return responses[Math.min(calls.length - 1, responses.length - 1)]
  }) as unknown as typeof fetch

  const result = await streamCompletion({
    url: 'https://example.test/chat/completions',
    apiKey: 'sk-test',
    label: 'deepseek-chat',
    body: { model: 'm', stream: true, stream_options: { include_usage: true } },
    fetchImpl
  })

  assert.equal(result.content, 'ok')
  assert.equal(calls.length, 2)
  const second = JSON.parse(String(calls[1].init.body)) as Record<string, unknown>
  assert.equal('stream_options' in second, false)
  assert.equal(second.stream, true)
})

test('retries a rate-limited round before answering', async () => {
  const responses = [
    json({ error: { message: 'too many requests' } }, 429),
    json({ choices: [{ message: { content: 'ok' } }] })
  ]
  const calls: Array<{ url: string; init: RequestInit }> = []
  const fetchImpl = (async (url: string, init: RequestInit) => {
    calls.push({ url, init })
    return responses[Math.min(calls.length - 1, responses.length - 1)]
  }) as unknown as typeof fetch

  const result = await streamCompletion({
    url: 'https://example.test/chat/completions',
    apiKey: 'sk-test',
    label: 'deepseek-chat',
    body: { model: 'm', stream: true },
    fetchImpl
  })

  assert.equal(result.content, 'ok')
  assert.equal(calls.length, 2)
})

test('gives up after the last retry', async () => {
  const calls: Array<{ url: string; init: RequestInit }> = []
  const fetchImpl = (async (url: string, init: RequestInit) => {
    calls.push({ url, init })
    return json({ error: { message: 'unavailable' } }, 503)
  }) as unknown as typeof fetch
  await assert.rejects(
    streamCompletion({
      url: 'https://example.test/chat/completions',
      apiKey: 'sk-test',
      label: 'deepseek-chat',
      body: { model: 'm', stream: true },
      fetchImpl
    }),
    (error: Error) => error.message.includes('unavailable')
  )
  assert.equal(calls.length, 3)
})

test('does not retry a failure unrelated to the token fields', async () => {
  const stub = stubFetch(json({ error: { message: 'invalid api key' } }, 401))
  await assert.rejects(
    streamCompletion({
      url: 'https://example.test/chat/completions',
      apiKey: 'sk-test',
      label: 'deepseek-chat',
      body: { stream: true, stream_options: { include_usage: true } },
      fetchImpl: stub.fetchImpl
    }),
    (error: Error) => error.message.includes('invalid api key')
  )
  assert.equal(stub.calls.length, 1)
})
