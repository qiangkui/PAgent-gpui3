import assert from 'node:assert/strict'
import { test } from 'node:test'
import { listModels } from '../src/ai.ts'

/** One captured request. */
interface Captured {
  url: string
  init: RequestInit
}

/**
 * Build a fetch stub that records the call and answers with `body`.
 * @param body - JSON body to answer with.
 * @param status - HTTP status to answer with.
 * @returns the stub plus the calls it recorded.
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

test('lists the model ids the endpoint reports', async () => {
  const stub = stubFetch({
    object: 'list',
    data: [{ id: 'deepseek-reasoner' }, { id: 'deepseek-chat' }, { id: 'deepseek-chat' }]
  })
  const result = await listModels(
    { apiKey: 'sk-test', baseUrl: 'https://example.test/v1/' },
    { fetchImpl: stub.fetchImpl, env: {} }
  )

  assert.deepEqual(result.models, ['deepseek-chat', 'deepseek-reasoner'])
  const call = stub.calls[0]
  assert.equal(call.url, 'https://example.test/v1/models')
  assert.equal(call.init.method, 'GET')
  const auth = (call.init.headers as Record<string, string>).authorization
  assert.ok(auth.includes('sk-test'), auth)
})

test('accepts the plain string form of a model list', async () => {
  const stub = stubFetch({ models: ['deepseek-v4', { id: 'deepseek-v4.1' }] })
  const result = await listModels({ apiKey: 'sk-test' }, { fetchImpl: stub.fetchImpl, env: {} })
  assert.deepEqual(result.models, ['deepseek-v4', 'deepseek-v4.1'])
})

test('reports a provider that answers with an error', async () => {
  const stub = stubFetch({ error: { message: 'invalid api key' } }, 401)
  await assert.rejects(
    listModels({ apiKey: 'bad' }, { fetchImpl: stub.fetchImpl, env: {} }),
    (error: Error) => error.message.includes('invalid api key')
  )
})

test('reports an empty model list', async () => {
  const stub = stubFetch({ data: [] })
  await assert.rejects(
    listModels({ apiKey: 'sk-test' }, { fetchImpl: stub.fetchImpl, env: {} }),
    (error: Error) => error.message.length > 0
  )
})

test('refuses to list models without a key', async () => {
  const stub = stubFetch({ data: [{ id: 'deepseek-chat' }] })
  await assert.rejects(
    listModels({}, { fetchImpl: stub.fetchImpl, env: {} }),
    (error: Error) => error.message.includes('API Key')
  )
  assert.equal(stub.calls.length, 0)
})
