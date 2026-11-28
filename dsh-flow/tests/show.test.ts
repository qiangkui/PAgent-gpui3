// `show`: the tool that puts a finished file in front of the person. The window
// draws what it returns, so the result has to carry real paths and sizes, and a
// path outside the workspace must be refused like every other reader.
import assert from 'node:assert/strict'
import { mkdir, mkdtemp, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { test } from 'node:test'
import { TOOLS, runTool, runToolScoped } from '../src/tools.ts'
import { MAX_SHOWN_FILES } from '../src/tools/show.ts'
import { runAgent } from '../src/agent.ts'
import type { AgentStep } from '../src/agent.ts'
import { stubFetch } from './helpers.ts'

/**
 * A throw-away workspace, holding the files a call may show.
 * @param files - relative name to content.
 * @returns the absolute directory.
 */
async function workspace(files: Record<string, string> = {}): Promise<string> {
  const root = await mkdtemp(path.join(tmpdir(), 'dsh-show-'))
  for (const [name, text] of Object.entries(files)) {
    await mkdir(path.dirname(path.join(root, name)), { recursive: true })
    await writeFile(path.join(root, name), text, 'utf8')
  }
  return root
}

test('the tool list offers show with a description the model can act on', () => {
  const definition = TOOLS.find((tool) => tool.function.name === 'show')
  assert.ok(definition, 'show 必须在工具清单里')
  assert.match(definition.function.description, /结果文件/)
  const properties = definition.function.parameters.properties as Record<string, unknown>
  assert.deepEqual(Object.keys(properties), ['paths', 'note'])
})

test('show hands over the files it was given, with their sizes', async () => {
  const root = await workspace({ 'report.md': '# 报告', 'data.csv': 'a,b\n1,2\n' })
  const result = await runTool(root, {
    id: 'c1',
    name: 'show',
    args: { paths: ['report.md', 'data.csv'] }
  })
  assert.equal(result.ok, true, result.text)
  assert.equal(result.files?.length, 2)
  const report = result.files?.[0]
  assert.equal(report?.name, 'report.md')
  assert.equal(report?.path, path.join(root, 'report.md'))
  assert.ok((report?.bytes ?? 0) > 0, '尺寸要报给窗口')
  assert.match(result.text, /已在对话里展示 2 个文件/)
  assert.match(result.text, /report\.md/)
  assert.match(result.text, /data\.csv/)
})

test('a missing file is named instead of swallowing the ones that exist', async () => {
  const root = await workspace({ 'ok.txt': 'fine' })
  const result = await runTool(root, {
    id: 'c2',
    name: 'show',
    args: { paths: ['ok.txt', 'gone.txt'] }
  })
  assert.equal(result.ok, true, result.text)
  assert.equal(result.files?.length, 1)
  assert.equal(result.files?.[0]?.name, 'ok.txt')
  assert.match(result.text, /没有展示：gone\.txt：文件不存在/)
})

test('showing nothing at all fails with the reason on the row', async () => {
  const root = await workspace()
  const result = await runTool(root, { id: 'c3', name: 'show', args: { paths: ['nope.txt'] } })
  assert.equal(result.ok, false)
  assert.match(result.text, /没有可展示的文件：nope\.txt：文件不存在/)
})

test('a path outside the workspace is refused like any other read', async () => {
  const root = await workspace({ 'inside.txt': 'x' })
  const result = await runToolScoped(root, root, {
    id: 'c4',
    name: 'show',
    args: { paths: ['../escape.txt'] }
  })
  assert.equal(result.ok, false)
  assert.match(result.text, /超出工作区/)
})

test('a folder is not a file to show', async () => {
  const root = await workspace({ 'notes/keep.txt': 'x' })
  const result = await runTool(root, { id: 'c5', name: 'show', args: { paths: ['notes'] } })
  assert.equal(result.ok, false)
  assert.match(result.text, /这是文件夹/)
})

test('one call cannot become a folder dump', async () => {
  const root = await workspace()
  const paths: string[] = []
  for (let index = 0; index <= MAX_SHOWN_FILES; index += 1) {
    const name = `f${index}.txt`
    await writeFile(path.join(root, name), 'x', 'utf8')
    paths.push(name)
  }
  const result = await runTool(root, { id: 'c6', name: 'show', args: { paths } })
  assert.equal(result.ok, false)
  assert.match(result.text, new RegExp(`一次最多展示 ${MAX_SHOWN_FILES} 个文件`))
})

test('the step a show call reports carries the files the window draws', async () => {
  const root = await workspace({ 'report.md': '# 报告' })
  const stub = stubFetch([
    {
      choices: [
        {
          message: {
            content: '',
            tool_calls: [
              {
                id: 'c7',
                type: 'function',
                function: { name: 'show', arguments: '{"paths":["report.md"]}' }
              }
            ]
          }
        }
      ]
    },
    { choices: [{ message: { content: '报告已经放在对话里了。' } }] }
  ])
  const steps: AgentStep[] = []
  const result = await runAgent(
    { prompt: '出一份报告', apiKey: 'sk-test', model: 'deepseek-chat', allowedTools: ['show'] },
    {
      fetchImpl: stub.fetchImpl,
      env: {},
      root,
      onStep: (step) => steps.push(step)
    }
  )
  assert.equal(result.text, '报告已经放在对话里了。')
  assert.equal(steps.length, 1)
  assert.equal(steps[0]?.tool, 'show')
  assert.equal(steps[0]?.target, 'report.md', '工具行按文件名命名')
  assert.equal(steps[0]?.files?.length, 1)
  assert.equal(steps[0]?.files?.[0]?.path, path.join(root, 'report.md'))
  const tool = stub.calls[1]?.body.messages.at(-1)
  assert.equal(tool?.role, 'tool', '模型读到的仍是文字')
  assert.match(String(tool?.content), /已在对话里展示 1 个文件/)
})