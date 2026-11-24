import assert from 'node:assert/strict'
import { mkdtemp, readFile, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { test } from 'node:test'
import {
  DROPPED_TOOL_RESULT,
  MAX_CALLS,
  MAX_TOOL_CONTEXT,
  MAX_TOOL_MESSAGE,
  runAgent,
  summarise
} from '../src/agent.ts'
import type { AgentProgress, AgentStep, ApprovalRequest } from '../src/agent.ts'
import { stubFetch } from './helpers.ts'

/**
 * A throw-away working directory.
 * @returns the absolute path.
 */
async function workspace(): Promise<string> {
  return await mkdtemp(path.join(tmpdir(), 'dsh-agent-'))
}

test('answers without calling a tool', async () => {
  const stub = stubFetch([{ choices: [{ message: { content: 'hello' } }] }])
  const steps: AgentStep[] = []
  const result = await runAgent(
    { prompt: 'hi', apiKey: 'sk-test', model: 'deepseek-chat' },
    { fetchImpl: stub.fetchImpl, env: {}, root: await workspace(), onStep: (step) => steps.push(step) }
  )
  assert.equal(result.text, 'hello')
  assert.equal(steps.length, 0)
  assert.equal(stub.calls[0].body.tools !== undefined, true, 'tools are declared')
})

test('reports the tool schemas it sends, so the window can size them', async () => {
  const stub = stubFetch([{ choices: [{ message: { content: 'ok' } }] }])
  const sent: Array<{ name: string; schema: string }> = []
  await runAgent(
    { prompt: 'hi', apiKey: 'sk-test', allowedTools: ['read', 'list'] },
    {
      fetchImpl: stub.fetchImpl,
      env: {},
      root: await workspace(),
      onTools: (report) => sent.push(...report.items)
    }
  )
  assert.deepEqual(sent.map((item) => item.name), ['read', 'list'])
  // The schema is the definition itself, not a summary of it.
  const read = JSON.parse(sent[0].schema) as { function: { name: string; description?: string } }
  assert.equal(read.function.name, 'read')
  assert.equal(typeof read.function.description, 'string')
})

test('builds plugin panels from the request it is about to send', async () => {
  // Panels are made from the same request the model sees, so their numbers
  // describe this question rather than a copy kept for display.
  const stub = stubFetch([{ choices: [{ message: { content: 'ok' } }] }])
  const seen: Array<{ id: string; turns: number; window: number; toolChars: number }> = []
  const built = await runAgent(
    {
      prompt: 'hi',
      apiKey: 'sk-test',
      model: 'deepseek-chat',
      contextWindow: 64000,
      history: [{ role: 'user', content: 'earlier' }]
    },
    {
      fetchImpl: stub.fetchImpl,
      env: {},
      root: await workspace(),
      panels: [
        {
          id: 'counter',
          title: '计数',
          plugin: 'demo',
          provide: (request) => {
            seen.push({
              id: 'counter',
              turns: request.turns.length,
              window: request.contextWindow,
              toolChars: request.toolChars
            })
            return { summary: [{ label: '窗口', value: String(request.contextWindow) }] }
          }
        }
      ],
      onTools: (report) => {
        assert.equal(report.panels.length, 1)
        assert.equal(report.panels[0].title, '计数')
        assert.deepEqual(report.panelErrors, [])
      }
    }
  )
  assert.equal(built.text, 'ok')
  // The replayed turn plus this question; no system prompt in this request.
  assert.equal(seen[0].turns, 2)
  assert.equal(seen[0].window, 64000)
  // The tool definitions the request carries are counted for the panel.
  assert.ok(seen[0].toolChars > 0)
})

test('runs a plugin tool only after the reader agrees', async () => {
  const root = await workspace()
  const stub = stubFetch([
    {
      choices: [
        {
          message: {
            content: null,
            tool_calls: [
              {
                id: 'call-1',
                type: 'function',
                function: { name: 'plugin_hello', arguments: JSON.stringify({ who: 'w' }) }
              }
            ]
          }
        }
      ]
    },
    { choices: [{ message: { content: 'ok' } }] }
  ])
  const asked: ApprovalRequest[] = []
  const plugin = {
    name: 'plugin_hello',
    description: '打招呼',
    parameters: { type: 'object', properties: { who: { type: 'string' } } },
    plugin: 'demo',
    run: (args: Record<string, unknown>) => `hello ${String(args.who)}`
  }
  await runAgent(
    { prompt: 'hi', apiKey: 'sk-test', allowedTools: ['read'] },
    {
      fetchImpl: stub.fetchImpl,
      env: {},
      root,
      plugins: [plugin],
      promptInjections: [{ id: 'demo', text: '插件提示：先看 README。' }],
      confirmActions: async (request) => {
        asked.push(request)
        return true
      }
    }
  )
  // A plugin tool is offered even though the window's allow-list only names
  // built-ins, and asking for it must reach the approval card first.
  const offered = (stub.calls[0].body.tools ?? []) as Array<{ function: { name: string } }>
  assert.ok(offered.some((tool) => tool.function.name === 'plugin_hello'))
  assert.equal(asked.length, 1)
  assert.equal(asked[0].tool, 'plugin_hello')
  assert.match(asked[0].diff, /插件 demo/)
  // The injected prompt rides along in the system message.
  const system = stub.calls[0].body.messages[0]
  assert.equal(system.role, 'system')
  assert.match(String(system.content), /先看 README/)
})

test('reports the last round as the context size, not the whole spend', async () => {
  const root = await workspace()
  const stub = stubFetch([
    {
      usage: { prompt_tokens: 1000, completion_tokens: 50, total_tokens: 1050 },
      choices: [
        {
          message: {
            content: null,
            tool_calls: [
              {
                id: 'call-1',
                type: 'function',
                function: { name: 'list', arguments: JSON.stringify({ path: '.' }) }
              }
            ]
          }
        }
      ]
    },
    {
      usage: { prompt_tokens: 1400, completion_tokens: 60, total_tokens: 1460 },
      choices: [{ message: { content: 'done' } }]
    }
  ])
  const result = await runAgent(
    { prompt: 'list', apiKey: 'sk-test' },
    { fetchImpl: stub.fetchImpl, env: {}, root }
  )
  // What the next question carries is the last request, not every round added up.
  assert.equal(result.context, 1460)
  assert.deepEqual(result.usage, {
    prompt_tokens: 2400,
    completion_tokens: 110,
    total_tokens: 2510
  })
})

test('forwards the sampled temperature and the token cap, leaving defaults alone', async () => {
  const pinned = stubFetch([{ choices: [{ message: { content: 'ok' } }] }])
  await runAgent(
    { prompt: 'hi', apiKey: 'sk-test', temperature: 0.2, maxTokens: 4096 },
    { fetchImpl: pinned.fetchImpl, env: {}, root: await workspace() }
  )
  assert.equal(pinned.calls[0].body.temperature, 0.2)
  assert.equal(pinned.calls[0].body.max_tokens, 4096)

  const plain = stubFetch([{ choices: [{ message: { content: 'ok' } }] }])
  await runAgent(
    { prompt: 'hi', apiKey: 'sk-test' },
    { fetchImpl: plain.fetchImpl, env: {}, root: await workspace() }
  )
  assert.equal(plain.calls[0].body.temperature, undefined, 'no key, no temperature')
  assert.equal(plain.calls[0].body.max_tokens, undefined, 'no cap, no max_tokens')
})

test('asks before a write and leaves the file alone when refused', async () => {
  const root = await workspace()
  const stub = stubFetch([
    {
      choices: [
        {
          message: {
            content: null,
            tool_calls: [
              {
                id: 'call-1',
                type: 'function',
                function: {
                  name: 'write',
                  arguments: JSON.stringify({ path: 'new.txt', content: 'hello\n' })
                }
              }
            ]
          }
        }
      ]
    },
    { choices: [{ message: { content: 'ok' } }] }
  ])
  const asked: ApprovalRequest[] = []
  const steps: AgentStep[] = []
  await runAgent(
    { prompt: 'write', apiKey: 'sk-test' },
    {
      fetchImpl: stub.fetchImpl,
      env: {},
      root,
      approve: async (request) => {
        asked.push(request)
        return false
      },
      onStep: (step) => steps.push(step)
    }
  )
  assert.equal(asked.length, 1)
  assert.equal(asked[0].tool, 'write')
  assert.equal(asked[0].path, 'new.txt')
  assert.match(asked[0].diff, /\+ hello/)
  assert.equal(asked[0].before, '')
  assert.equal(asked[0].after, 'hello\n')
  assert.equal(asked[0].existed, false)
  assert.equal(asked[0].undoable, true)
  await assert.rejects(readFile(path.join(root, 'new.txt')), 'the file must not exist')
  assert.equal(steps[0].ok, false)
  assert.match(steps[0].output, /拒绝/)
})

test('writes after the change is allowed', async () => {
  const root = await workspace()
  const stub = stubFetch([
    {
      choices: [
        {
          message: {
            content: null,
            tool_calls: [
              {
                id: 'call-1',
                type: 'function',
                function: {
                  name: 'write',
                  arguments: JSON.stringify({ path: 'new.txt', content: 'hello\n' })
                }
              }
            ]
          }
        }
      ]
    },
    { choices: [{ message: { content: 'done' } }] }
  ])
  await runAgent(
    { prompt: 'write', apiKey: 'sk-test' },
    { fetchImpl: stub.fetchImpl, env: {}, root, approve: async () => true }
  )
  assert.equal(await readFile(path.join(root, 'new.txt'), 'utf8'), 'hello\n')
})

test('writes without asking when the request allows direct writes', async () => {
  const root = await workspace()
  const stub = stubFetch([
    {
      choices: [
        {
          message: {
            content: null,
            tool_calls: [
              {
                id: 'call-1',
                type: 'function',
                function: {
                  name: 'write',
                  arguments: JSON.stringify({ path: 'new.txt', content: 'hello\n' })
                }
              }
            ]
          }
        }
      ]
    },
    { choices: [{ message: { content: 'done' } }] }
  ])
  const reported: ApprovalRequest[] = []
  const steps: AgentStep[] = []
  const result = await runAgent(
    { prompt: 'write', apiKey: 'sk-test' },
    {
      fetchImpl: stub.fetchImpl,
      env: {},
      root,
      onApproval: (request) => reported.push(request),
      onStep: (step) => steps.push(step)
    }
  )
  assert.equal(result.text, 'done')
  assert.equal(await readFile(path.join(root, 'new.txt'), 'utf8'), 'hello\n')
  assert.equal(reported.length, 1)
  assert.equal(reported[0].call, 'call-1')
  assert.equal(reported[0].after, 'hello\n')
  assert.equal(steps[0].ok, true)
})

test('writes outside the conversation folder in full scope', async () => {
  const root = await workspace()
  const target = path.join(await workspace(), 'elsewhere.txt')
  const stub = stubFetch([
    {
      choices: [
        {
          message: {
            content: null,
            tool_calls: [
              {
                id: 'call-1',
                type: 'function',
                function: {
                  name: 'write',
                  arguments: JSON.stringify({ path: target, content: 'wide\n' })
                }
              }
            ]
          }
        }
      ]
    },
    { choices: [{ message: { content: 'done' } }] }
  ])
  await runAgent(
    { prompt: 'write', apiKey: 'sk-test' },
    { fetchImpl: stub.fetchImpl, env: {}, root, scopeRoot: null, approve: async () => true }
  )
  assert.equal(await readFile(target, 'utf8'), 'wide\n')
})

test('offers only the tools the request allows', async () => {
  const stub = stubFetch([{ choices: [{ message: { content: 'ok' } }] }])
  await runAgent(
    { prompt: 'hi', apiKey: 'sk-test', allowedTools: ['read', 'glob'] },
    { fetchImpl: stub.fetchImpl, env: {}, root: await workspace() }
  )
  const tools = stub.calls[0].body.tools as Array<{ function: { name: string } }>
  assert.deepEqual(
    tools.map((tool) => tool.function.name),
    ['read', 'glob']
  )
})

test('keeps attached images on the wire through every tool round', async () => {
  const root = await workspace()
  await writeFile(path.join(root, 'a.txt'), 'x\n')
  const stub = stubFetch([
    {
      choices: [
        {
          message: {
            content: null,
            tool_calls: [
              {
                id: 'call-1',
                type: 'function',
                function: { name: 'read', arguments: JSON.stringify({ path: 'a.txt' }) }
              }
            ]
          }
        }
      ]
    },
    { choices: [{ message: { content: 'ok' } }] }
  ])
  await runAgent(
    {
      prompt: '看看这张图',
      apiKey: 'sk-test',
      attachments: [{ kind: 'image', name: 'a.png', mimeType: 'image/png', data: 'aGk=' }]
    },
    { fetchImpl: stub.fetchImpl, env: {}, root }
  )
  const first = stub.calls[0]!.body.messages[0]!.content
  const second = stub.calls[1]!.body.messages[0]!.content
  assert.deepEqual(second, first)
  assert.deepEqual(first, [
    { type: 'text', text: '看看这张图' },
    { type: 'image_url', image_url: { url: 'data:image/png;base64,aGk=' } }
  ])
})

test('refuses a tool the request did not allow', async () => {
  const root = await workspace()
  const stub = stubFetch([
    {
      choices: [
        {
          message: {
            content: null,
            tool_calls: [
              {
                id: 'call-1',
                type: 'function',
                function: { name: 'write', arguments: JSON.stringify({ path: 'x.txt', content: 'no' }) }
              }
            ]
          }
        }
      ]
    },
    { choices: [{ message: { content: 'nope' } }] }
  ])
  const steps: AgentStep[] = []
  await runAgent(
    { prompt: 'write', apiKey: 'sk-test', allowedTools: ['read'] },
    { fetchImpl: stub.fetchImpl, env: {}, root, onStep: (step) => steps.push(step) }
  )
  assert.equal(steps.length, 1)
  assert.equal(steps[0].tool, 'write')
  assert.equal(steps[0].ok, false)
  assert.match(steps[0].detail, /工具未启用/)
  assert.match(steps[0].output, /工具未启用/)
})

test('asks before running code and runs it when allowed', async () => {
  const root = await workspace()
  const stub = stubFetch([
    {
      choices: [
        {
          message: {
            content: null,
            tool_calls: [
              {
                id: 'call-1',
                type: 'function',
                function: {
                  name: 'node',
                  arguments: JSON.stringify({
                    code: "require('node:fs').writeFileSync('made.txt', 'made')"
                  })
                }
              }
            ]
          }
        }
      ]
    },
    { choices: [{ message: { content: 'ok' } }] }
  ])
  const asked: ApprovalRequest[] = []
  await runAgent(
    { prompt: '写个文件', apiKey: 'sk-test' },
    {
      fetchImpl: stub.fetchImpl,
      env: {},
      root,
      confirmActions: async (question) => {
        asked.push(question)
        return true
      }
    }
  )
  assert.equal(asked.length, 1)
  assert.equal(asked[0]!.tool, 'node')
  assert.equal(asked[0]!.command, '')
  assert.match(asked[0]!.diff, /node -e/)
  assert.equal(await readFile(path.join(root, 'made.txt'), 'utf8'), 'made')
})

test('asks before a request and refuses it without a decision bridge', async () => {
  const root = await workspace()
  const call = {
    choices: [
      {
        message: {
          content: null,
          tool_calls: [
            {
              id: 'call-1',
              type: 'function',
              function: {
                name: 'http',
                arguments: JSON.stringify({ url: 'http://127.0.0.1:1/none' })
              }
            }
          ]
        }
      }
    ]
  }
  const asked: ApprovalRequest[] = []
  const stub = stubFetch([call, { choices: [{ message: { content: 'denied' } }] }])
  await runAgent(
    { prompt: '看个接口', apiKey: 'sk-test' },
    {
      fetchImpl: stub.fetchImpl,
      env: {},
      root,
      confirmActions: async (question) => {
        asked.push(question)
        return false
      }
    }
  )
  assert.equal(asked.length, 1)
  assert.equal(asked[0]!.tool, 'http')
  assert.equal(asked[0]!.path, '')
  assert.match(asked[0]!.diff, /^\$ http GET http:\/\/127\.0\.0\.1:1\/none/)

  const steps: AgentStep[] = []
  const silent = stubFetch([call, { choices: [{ message: { content: 'skipped' } }] }])
  await runAgent(
    { prompt: '看个接口', apiKey: 'sk-test' },
    { fetchImpl: silent.fetchImpl, env: {}, root, onStep: (step) => steps.push(step) }
  )
  assert.equal(steps[0]?.ok, false)
  assert.match(steps[0]?.output ?? '', /没有确认界面，请求没有发出/)
})

test('refuses a code snippet when nobody can confirm it', async () => {
  const root = await workspace()
  const stub = stubFetch([
    {
      choices: [
        {
          message: {
            content: null,
            tool_calls: [
              {
                id: 'call-1',
                type: 'function',
                function: {
                  name: 'node',
                  arguments: JSON.stringify({
                    code: "require('node:fs').writeFileSync('made.txt', 'made')"
                  })
                }
              }
            ]
          }
        }
      ]
    },
    { choices: [{ message: { content: 'ok' } }] }
  ])
  const steps: AgentStep[] = []
  await runAgent(
    { prompt: '写个文件', apiKey: 'sk-test' },
    { fetchImpl: stub.fetchImpl, env: {}, root, onStep: (step) => steps.push(step) }
  )
  assert.equal(steps.length, 1)
  assert.equal(steps[0]!.tool, 'node')
  assert.equal(steps[0]!.ok, false)
  assert.match(steps[0]!.detail, /没有确认界面/)
  await assert.rejects(readFile(path.join(root, 'made.txt'), 'utf8'))
})

test('refuses a command when the caller cannot ask', async () => {
  const root = await workspace()
  const stub = stubFetch([
    {
      choices: [
        {
          message: {
            content: null,
            tool_calls: [
              {
                id: 'call-1',
                type: 'function',
                function: {
                  name: 'shell',
                  arguments: JSON.stringify({ command: `node -e "process.stdout.write('ran')"` })
                }
              }
            ]
          }
        }
      ]
    },
    { choices: [{ message: { content: 'no' } }] }
  ])
  const steps: AgentStep[] = []
  await runAgent(
    { prompt: 'run', apiKey: 'sk-test' },
    { fetchImpl: stub.fetchImpl, env: {}, root, onStep: (step) => steps.push(step) }
  )
  assert.equal(steps[0].tool, 'shell')
  assert.equal(steps[0].ok, false)
  assert.match(steps[0].output, /没有确认界面/)
  assert.doesNotMatch(steps[0].output, /ran/)
})

test('asks before a command and runs it when allowed', async () => {
  const root = await workspace()
  const stub = stubFetch([
    {
      choices: [
        {
          message: {
            content: null,
            tool_calls: [
              {
                id: 'call-1',
                type: 'function',
                function: {
                  name: 'shell',
                  arguments: JSON.stringify({ command: `node -e "process.stdout.write('ran')"` })
                }
              }
            ]
          }
        }
      ]
    },
    { choices: [{ message: { content: 'done' } }] }
  ])
  const asked: ApprovalRequest[] = []
  const steps: AgentStep[] = []
  const result = await runAgent(
    { prompt: 'run', apiKey: 'sk-test' },
    {
      fetchImpl: stub.fetchImpl,
      env: {},
      root,
      confirmActions: async (request) => {
        asked.push(request)
        return true
      },
      onStep: (step) => steps.push(step)
    }
  )
  assert.equal(result.text, 'done')
  assert.equal(asked.length, 1)
  assert.equal(asked[0].tool, 'shell')
  assert.match(asked[0].target, /node -e/)
  assert.equal(asked[0].path, '')
  assert.equal(asked[0].undoable, false)
  assert.match(asked[0].diff, /^\$ node -e/)
  assert.equal(asked[0].command, `node -e "process.stdout.write('ran')"`)
  assert.equal(steps[0].ok, true, steps[0].output)
  assert.match(steps[0].output, /ran/)
})

test('leaves a command alone when it is refused', async () => {
  const root = await workspace()
  const stub = stubFetch([
    {
      choices: [
        {
          message: {
            content: null,
            tool_calls: [
              {
                id: 'call-1',
                type: 'function',
                function: {
                  name: 'shell',
                  arguments: JSON.stringify({ command: `node -e "process.stdout.write('ran')"` })
                }
              }
            ]
          }
        }
      ]
    },
    { choices: [{ message: { content: 'ok' } }] }
  ])
  const steps: AgentStep[] = []
  await runAgent(
    { prompt: 'run', apiKey: 'sk-test' },
    {
      fetchImpl: stub.fetchImpl,
      env: {},
      root,
      confirmActions: async () => false,
      onStep: (step) => steps.push(step)
    }
  )
  assert.equal(steps[0].ok, false)
  assert.match(steps[0].output, /拒绝/)
  assert.doesNotMatch(steps[0].output, /ran/)
})

test('asks before deleting and leaves the file when refused', async () => {
  const root = await workspace()
  await writeFile(path.join(root, 'gone.txt'), 'bye\n', 'utf8')
  const stub = stubFetch([
    {
      choices: [
        {
          message: {
            content: null,
            tool_calls: [
              {
                id: 'call-1',
                type: 'function',
                function: { name: 'delete', arguments: JSON.stringify({ path: 'gone.txt' }) }
              }
            ]
          }
        }
      ]
    },
    { choices: [{ message: { content: 'ok' } }] }
  ])
  const asked: ApprovalRequest[] = []
  const steps: AgentStep[] = []
  await runAgent(
    { prompt: 'delete', apiKey: 'sk-test' },
    {
      fetchImpl: stub.fetchImpl,
      env: {},
      root,
      confirmActions: async (request) => {
        asked.push(request)
        return false
      },
      onStep: (step) => steps.push(step)
    }
  )
  assert.equal(asked.length, 1)
  assert.equal(asked[0].tool, 'delete')
  assert.equal(asked[0].target, 'gone.txt')
  assert.equal(asked[0].path, 'gone.txt')
  assert.equal(asked[0].undoable, false)
  assert.match(asked[0].diff, /^删除文件：gone\.txt$/)
  assert.equal(await readFile(path.join(root, 'gone.txt'), 'utf8'), 'bye\n')
  assert.equal(steps[0].ok, false)
  assert.match(steps[0].output, /拒绝/)
})

test('moves a file after the action is allowed', async () => {
  const root = await workspace()
  await writeFile(path.join(root, 'old.txt'), 'body\n', 'utf8')
  const stub = stubFetch([
    {
      choices: [
        {
          message: {
            content: null,
            tool_calls: [
              {
                id: 'call-1',
                type: 'function',
                function: {
                  name: 'move',
                  arguments: JSON.stringify({ from: 'old.txt', to: 'new.txt' })
                }
              }
            ]
          }
        }
      ]
    },
    { choices: [{ message: { content: 'moved' } }] }
  ])
  const asked: ApprovalRequest[] = []
  const result = await runAgent(
    { prompt: 'move', apiKey: 'sk-test' },
    {
      fetchImpl: stub.fetchImpl,
      env: {},
      root,
      confirmActions: async (request) => {
        asked.push(request)
        return true
      }
    }
  )
  assert.equal(result.text, 'moved')
  assert.equal(asked.length, 1)
  assert.equal(asked[0].tool, 'move')
  assert.equal(asked[0].target, 'old.txt → new.txt')
  assert.match(asked[0].diff, /^移动：old\.txt → new\.txt$/)
  assert.equal(await readFile(path.join(root, 'new.txt'), 'utf8'), 'body\n')
  await assert.rejects(readFile(path.join(root, 'old.txt')))
})

test('runs a tool and feeds the result back', async () => {
  const root = await workspace()
  await writeFile(path.join(root, 'notes.txt'), 'alpha\n', 'utf8')
  const stub = stubFetch([
    {
      choices: [
        {
          message: {
            content: null,
            tool_calls: [
              {
                id: 'call-1',
                type: 'function',
                function: { name: 'read', arguments: JSON.stringify({ path: 'notes.txt' }) }
              }
            ]
          }
        }
      ]
    },
    { choices: [{ message: { content: '文件里是 alpha' } }] }
  ])
  const steps: AgentStep[] = []
  const result = await runAgent(
    { prompt: 'notes.txt 里是什么？', apiKey: 'sk-test' },
    { fetchImpl: stub.fetchImpl, env: {}, root, onStep: (step) => steps.push(step) }
  )

  assert.equal(result.text, '文件里是 alpha')
  assert.equal(steps.length, 1)
  assert.equal(steps[0].tool, 'read')
  assert.equal(steps[0].target, 'notes.txt')
  assert.equal(steps[0].ok, true)
  assert.match(steps[0].input, /notes\.txt/)
  assert.match(steps[0].output, /alpha/)

  const second = stub.calls[1].body.messages
  assert.equal(second[second.length - 1].role, 'tool')
  assert.equal(second[second.length - 1].tool_call_id, 'call-1')
  assert.match(String(second[second.length - 1].content), /alpha/)
})

test('adds up the tokens of every round', async () => {
  const root = await workspace()
  await writeFile(path.join(root, 'notes.txt'), 'alpha\n', 'utf8')
  const stub = stubFetch([
    {
      choices: [
        {
          message: {
            content: null,
            tool_calls: [
              {
                id: 'call-1',
                type: 'function',
                function: { name: 'read', arguments: JSON.stringify({ path: 'notes.txt' }) }
              }
            ]
          }
        }
      ],
      usage: { prompt_tokens: 100, completion_tokens: 10, total_tokens: 110 }
    },
    {
      choices: [{ message: { content: 'alpha' } }],
      usage: { prompt_tokens: 200, completion_tokens: 20, total_tokens: 220 }
    }
  ])

  const result = await runAgent(
    { prompt: 'notes.txt 里是什么？', apiKey: 'sk-test' },
    { fetchImpl: stub.fetchImpl, env: {}, root }
  )

  const usage = result.usage as { prompt_tokens: number; completion_tokens: number; total_tokens: number }
  assert.equal(usage.prompt_tokens, 300)
  assert.equal(usage.completion_tokens, 30)
  assert.equal(usage.total_tokens, 330)
})

test('reports a failed tool back to the model', async () => {
  const root = await workspace()
  const stub = stubFetch([
    {
      choices: [
        {
          message: {
            content: null,
            tool_calls: [
              {
                id: 'call-1',
                type: 'function',
                function: { name: 'read', arguments: JSON.stringify({ path: '../secret' }) }
              }
            ]
          }
        }
      ]
    },
    { choices: [{ message: { content: '读不到' } }] }
  ])
  const steps: AgentStep[] = []
  await runAgent(
    { prompt: 'x', apiKey: 'sk-test' },
    { fetchImpl: stub.fetchImpl, env: {}, root, onStep: (step) => steps.push(step) }
  )
  assert.equal(steps[0].ok, false)
})

test('names the source file in a move step', async () => {
  const root = await workspace()
  await writeFile(path.join(root, 'from.txt'), 'body\n', 'utf8')
  const stub = stubFetch([
    {
      choices: [
        {
          message: {
            content: null,
            tool_calls: [
              {
                id: 'call-1',
                type: 'function',
                function: {
                  name: 'move',
                  arguments: JSON.stringify({ from: 'from.txt', to: 'to.txt' })
                }
              }
            ]
          }
        }
      ]
    },
    { choices: [{ message: { content: 'done' } }] }
  ])
  const steps: AgentStep[] = []
  await runAgent(
    { prompt: 'move', apiKey: 'sk-test' },
    {
      fetchImpl: stub.fetchImpl,
      env: {},
      root,
      confirmActions: async () => true,
      onStep: (step) => steps.push(step)
    }
  )
  assert.equal(steps[0].target, 'from.txt')
})

test('reports broken tool arguments instead of running the tool', async () => {
  const root = await workspace()
  const stub = stubFetch([
    {
      choices: [
        {
          message: {
            content: null,
            tool_calls: [
              {
                id: 'call-1',
                type: 'function',
                function: { name: 'write', arguments: '{oops' }
              }
            ]
          }
        }
      ]
    },
    { choices: [{ message: { content: 'retry' } }] }
  ])
  const steps: AgentStep[] = []
  const asked: ApprovalRequest[] = []
  const result = await runAgent(
    { prompt: 'write', apiKey: 'sk-test' },
    {
      fetchImpl: stub.fetchImpl,
      env: {},
      root,
      approve: async (request) => {
        asked.push(request)
        return true
      },
      onStep: (step) => steps.push(step)
    }
  )
  assert.equal(result.text, 'retry')
  assert.equal(asked.length, 0, 'a call that never runs needs no approval')
  assert.equal(steps[0].ok, false)
  assert.match(steps[0].output, /JSON/)
  const second = stub.calls[1].body.messages
  const tool = second[second.length - 1]
  assert.equal(tool.role, 'tool')
  assert.match(String(tool.content), /JSON/)
})

test('caps a huge tool result before sending it back', async () => {
  const root = await workspace()
  await writeFile(path.join(root, 'big.txt'), 'x'.repeat(MAX_TOOL_MESSAGE + 500), 'utf8')
  const stub = stubFetch([
    {
      choices: [
        {
          message: {
            content: null,
            tool_calls: [
              {
                id: 'call-1',
                type: 'function',
                function: { name: 'read', arguments: JSON.stringify({ path: 'big.txt' }) }
              }
            ]
          }
        }
      ]
    },
    { choices: [{ message: { content: 'done' } }] }
  ])
  const steps: AgentStep[] = []
  await runAgent(
    { prompt: 'read', apiKey: 'sk-test' },
    { fetchImpl: stub.fetchImpl, env: {}, root, onStep: (step) => steps.push(step) }
  )
  const second = stub.calls[1].body.messages
  const content = String(second[second.length - 1].content)
  assert.ok(content.length < MAX_TOOL_MESSAGE + 200, `sent ${content.length} characters`)
  assert.match(content, /结果过长/)
  assert.equal(steps[0].ok, true)
})

test('keeps the tail of a long result so the line count survives', async () => {
  const root = await workspace()
  const lines = Array.from({ length: 2000 }, (_, index) => `line ${index} ${'x'.repeat(80)}`)
  await writeFile(path.join(root, 'many.txt'), lines.join('\n'), 'utf8')
  const stub = stubFetch([
    {
      choices: [
        {
          message: {
            content: null,
            tool_calls: [
              {
                id: 'call-1',
                type: 'function',
                function: { name: 'read', arguments: JSON.stringify({ path: 'many.txt' }) }
              }
            ]
          }
        }
      ]
    },
    { choices: [{ message: { content: 'done' } }] }
  ])
  await runAgent(
    { prompt: 'read', apiKey: 'sk-test' },
    { fetchImpl: stub.fetchImpl, env: {}, root }
  )
  const second = stub.calls[1].body.messages
  const tool = second.find((message) => message.role === 'tool')
  assert.ok(tool)
  const content = String(tool.content)
  assert.match(content, /中间已省略/)
  assert.match(content, /共 2000 行/)
  assert.ok(content.length <= MAX_TOOL_MESSAGE + 100, `${content.length} characters`)
})

test('announces a shell command before its first output', async () => {
  const root = await workspace()
  const starts: string[] = []
  const stub = stubFetch([
    {
      choices: [
        {
          message: {
            content: null,
            tool_calls: [
              {
                id: 'call-1',
                type: 'function',
                function: {
                  name: 'shell',
                  arguments: JSON.stringify({
                    command: `node -e "process.stdout.write('hi')"`
                  })
                }
              }
            ]
          }
        }
      ]
    },
    { choices: [{ message: { content: 'done' } }] }
  ])
  const result = await runAgent(
    { prompt: 'run', apiKey: 'sk-test' },
    {
      fetchImpl: stub.fetchImpl,
      env: {},
      root,
      confirmActions: async () => true,
      onToolStart: (call, tool, target) => starts.push(`${call}|${tool}|${target}`)
    }
  )
  assert.equal(result.text, 'done')
  assert.equal(starts.length, 1)
  assert.match(starts[0], /^call-1\|shell\|node -e/)
})

test('reports a broken approval callback instead of dropping the answer', async () => {
  const root = await workspace()
  const stub = stubFetch([
    {
      choices: [
        {
          message: {
            content: null,
            tool_calls: [
              {
                id: 'call-1',
                type: 'function',
                function: {
                  name: 'write',
                  arguments: JSON.stringify({ path: 'x.txt', content: 'hi' })
                }
              }
            ]
          }
        }
      ]
    },
    { choices: [{ message: { content: 'ok' } }] }
  ])
  const steps: AgentStep[] = []
  const result = await runAgent(
    { prompt: 'write', apiKey: 'sk-test' },
    {
      fetchImpl: stub.fetchImpl,
      env: {},
      root,
      approve: async () => {
        throw new Error('界面已关闭')
      },
      onStep: (step) => steps.push(step)
    }
  )
  assert.equal(result.text, 'ok')
  assert.equal(steps[0].ok, false)
  assert.match(steps[0].output, /界面已关闭/)
  await assert.rejects(readFile(path.join(root, 'x.txt')), 'the write must not happen')
})

test('shrinks older tool results so the transcript stays inside its budget', async () => {
  const root = await workspace()
  await writeFile(path.join(root, 'big.txt'), 'x'.repeat(MAX_TOOL_MESSAGE + 500), 'utf8')
  const calls = Array.from({ length: 5 }, (_, index) => ({
    id: `call-${index + 1}`,
    type: 'function',
    function: { name: 'read', arguments: JSON.stringify({ path: 'big.txt' }) }
  }))
  const stub = stubFetch([
    { choices: [{ message: { content: null, tool_calls: calls } }] },
    { choices: [{ message: { content: 'done' } }] }
  ])
  await runAgent(
    { prompt: 'read', apiKey: 'sk-test' },
    { fetchImpl: stub.fetchImpl, env: {}, root }
  )
  const second = stub.calls[1].body.messages
  const tools = second.filter((message) => message.role === 'tool')
  assert.equal(tools.length, 5, 'every call keeps its answer')
  assert.deepEqual(
    tools.map((message) => message.tool_call_id),
    ['call-1', 'call-2', 'call-3', 'call-4', 'call-5']
  )
  const full = tools.filter((message) => String(message.content).startsWith('1\t'))
  assert.equal(full.length, 2, 'only the newest results survive in full')
  const dropped = tools.filter((message) => message.content === DROPPED_TOOL_RESULT)
  assert.equal(dropped.length, 3)
  const total = tools.reduce((sum, message) => sum + String(message.content).length, 0)
  assert.ok(total <= MAX_TOOL_CONTEXT, `${total} characters`)
})

test('refuses a request without a key', async () => {
  const stub = stubFetch([{ choices: [] }])
  await assert.rejects(
    runAgent(
      { prompt: 'hi' },
      { fetchImpl: stub.fetchImpl, env: {}, root: await workspace() }
    ),
    (error: Error) => error.message.includes('API Key')
  )
})

test('asks for a summary when the round budget runs out', async () => {
  const root = await workspace()
  const call = {
    choices: [
      {
        message: {
          content: null,
          tool_calls: [
            {
              id: 'call-1',
              type: 'function',
              function: { name: 'glob', arguments: JSON.stringify({ pattern: '*' }) }
            }
          ]
        }
      }
    ]
  }
  const stub = stubFetch([call])
  const result = await runAgent(
    { prompt: 'x', apiKey: 'sk-test', maxRounds: 2 },
    { fetchImpl: stub.fetchImpl, env: {}, root }
  )
  assert.match(result.text, /2 轮上限/)
  assert.equal(stub.calls.length, 3, 'two tool rounds plus the summary round')
  assert.equal(stub.calls[2].body.tools, undefined, 'the summary declares no tools')
})

test('skips calls past the tool budget and still answers', async () => {
  const root = await workspace()
  await writeFile(path.join(root, 'a.txt'), 'alpha\n', 'utf8')
  const read = (id: string) => ({
    id,
    type: 'function',
    function: { name: 'read', arguments: JSON.stringify({ path: 'a.txt' }) }
  })
  const stub = stubFetch([
    {
      choices: [
        {
          message: {
            content: null,
            tool_calls: [read('call-1'), read('call-2')]
          }
        }
      ]
    },
    { choices: [{ message: { content: 'done' } }] }
  ])
  const steps: AgentStep[] = []
  const result = await runAgent(
    { prompt: 'x', apiKey: 'sk-test', maxCalls: 1 },
    {
      fetchImpl: stub.fetchImpl,
      env: {},
      root,
      onStep: (step) => steps.push(step)
    }
  )
  assert.equal(steps.length, 2)
  assert.equal(steps[0].ok, true)
  assert.equal(steps[1].ok, false)
  assert.match(steps[1].detail, /上限/)
  assert.match(result.text, /1 次上限/)
  assert.match(result.text, /done/)
  const second = stub.calls[1].body.messages
  const first = second[second.length - 2]
  const skipped = second[second.length - 1]
  assert.equal(first.role, 'tool')
  assert.equal(skipped.role, 'tool')
  assert.match(String(skipped.content), /上限/)
})

test('reports the round and the finished calls before each request', async () => {
  const root = await workspace()
  await writeFile(path.join(root, 'a.txt'), 'alpha\n', 'utf8')
  const read = (id: string) => ({
    id,
    type: 'function',
    function: { name: 'read', arguments: JSON.stringify({ path: 'a.txt' }) }
  })
  const stub = stubFetch([
    {
      usage: { prompt_tokens: 1000, completion_tokens: 50, total_tokens: 1050 },
      choices: [{ message: { content: null, tool_calls: [read('call-1')] } }]
    },
    // Second round: two calls, so the strip has to count both of them.
    {
      usage: { prompt_tokens: 1200, completion_tokens: 60, total_tokens: 1260 },
      choices: [
        {
          message: { content: null, tool_calls: [read('call-2'), read('call-3')] }
        }
      ]
    },
    {
      usage: { prompt_tokens: 1400, completion_tokens: 70, total_tokens: 1470 },
      choices: [{ message: { content: 'done' } }]
    }
  ])
  const seen: AgentProgress[] = []
  const result = await runAgent(
    { prompt: 'x', apiKey: 'sk-test' },
    { fetchImpl: stub.fetchImpl, env: {}, root, onProgress: (progress) => seen.push(progress) }
  )
  // One event before each of the three round trips, with the calls finished by
  // then: the first request has none behind it, the second one, the last three.
  assert.deepEqual(
    seen.map((progress) => [progress.round, progress.calls]),
    [
      [1, 0],
      [2, 1],
      [3, 3]
    ]
  )
  // The final answer agrees with the loop: three round trips, and the per-round
  // call counts sum to the same three.
  assert.equal(result.rounds, 3)
  assert.deepEqual(
    result.roundTokens.map((round) => round[2]),
    [1, 2, 0]
  )
})

test('asks before ending a background task but not before reading it', async () => {
  const root = await workspace()
  const call = (id: string, name: string, args: Record<string, unknown>) => ({
    id,
    type: 'function',
    function: { name, arguments: JSON.stringify(args) }
  })
  const stub = stubFetch([
    {
      choices: [
        {
          message: {
            content: null,
            tool_calls: [
              call('call-1', 'task', { action: 'list' }),
              call('call-2', 'task', { action: 'stop', id: 'task-1' })
            ]
          }
        }
      ]
    },
    { choices: [{ message: { content: 'ok' } }] }
  ])
  const asked: ApprovalRequest[] = []
  const steps: AgentStep[] = []
  await runAgent(
    { prompt: '看任务', apiKey: 'sk-test' },
    {
      fetchImpl: stub.fetchImpl,
      env: {},
      root,
      confirmActions: async (question) => {
        asked.push(question)
        return false
      },
      onStep: (step) => steps.push(step)
    }
  )
  assert.equal(asked.length, 1, 'only the stop asks')
  assert.equal(asked[0]!.tool, 'task')
  assert.match(asked[0]!.diff, /结束后台任务：task-1/)
  assert.equal(asked[0]!.call, 'call-2', 'the reply is matched to the right call')
  assert.equal(steps[0]!.ok, true, 'list needs no confirmation')
  assert.match(String(steps[0]!.output), /还没有后台任务/)
  assert.match(steps[1]!.detail, /拒绝/)
})

test('refuses a command that leaves the sandbox before it runs', async () => {
  const root = await workspace()
  const stub = stubFetch([
    {
      choices: [
        {
          message: {
            content: null,
            tool_calls: [
              {
                id: 'call-1',
                type: 'function',
                function: {
                  name: 'shell',
                  arguments: JSON.stringify({ command: 'node -e "console.log(1)"', cwd: '..' })
                }
              }
            ]
          }
        }
      ]
    },
    { choices: [{ message: { content: 'ok' } }] }
  ])
  const steps: AgentStep[] = []
  let asked = 0
  await runAgent(
    { prompt: '跑一下', apiKey: 'sk-test', fileScope: 'workspace', scopeRoot: root },
    {
      fetchImpl: stub.fetchImpl,
      env: {},
      root,
      scopeRoot: root,
      sandbox: { roots: [root] },
      confirmActions: async () => {
        asked += 1
        return true
      },
      onStep: (step) => steps.push(step)
    }
  )
  assert.equal(asked, 0, 'the card is never shown for a refused command')
  assert.equal(steps[0]!.ok, false)
  assert.match(String(steps[0]!.output), /工作区/)
})

test('caps the number of tool calls', () => {
  assert.ok(MAX_CALLS > 0)
})

test('summarises tool output for the transcript', () => {
  assert.equal(summarise('read', true, '1\talpha\n2\tbeta'), '已读取 2 行')
  assert.equal(summarise('read', true, '1\thuge\n…（共 900 行，已显示到 1 行）'), '已读取 1 行')
  assert.equal(summarise('list', true, 'src/\na.ts\nb.ts'), '3 项')
  assert.equal(summarise('list', true, '文件夹是空的'), '文件夹是空的')
  assert.equal(summarise('glob', true, 'a.ts\nb.ts'), '2 个文件')
  assert.equal(summarise('glob', true, '没有匹配 *.rs 的文件'), '没有匹配 *.rs 的文件')
  assert.equal(summarise('grep', true, 'a.ts:3: let x = 1\nb.ts:9: let y = 2'), '2 处匹配')
  assert.equal(summarise('grep', true, 'a.ts:3: let x = 1\n…（已截断）'), '1 处匹配')
  assert.equal(summarise('write', true, '已写入 a.ts（3 字符）'), '已写入 a.ts（3 字符）')
  assert.equal(summarise('read', false, '路径超出工作目录：../x'), '路径超出工作目录：../x')
})

test('replays the conversation inside the tool loop', async () => {
  const root = await mkdtemp(path.join(tmpdir(), 'dsh-flow-agent-history-'))
  await writeFile(path.join(root, 'a.ts'), 'let a = 1\n', 'utf8')
  const answer = { choices: [{ message: { content: '看过了' } }] }
  const stub = stubFetch([answer])
  const result = await runAgent(
    {
      prompt: '那 a.ts 呢？',
      apiKey: 'sk-test',
      history: [
        { role: 'user', content: '这里有什么文件？' },
        { role: 'assistant', content: '有 a.ts。' }
      ]
    },
    { fetchImpl: stub.fetchImpl, env: {}, root }
  )
  assert.equal(result.text, '看过了')
  assert.deepEqual(
    stub.calls[0].body.messages.map((message) => message.role),
    ['user', 'assistant', 'user']
  )
  assert.equal(stub.calls[0].body.messages[0].content, '这里有什么文件？')
})
