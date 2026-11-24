import assert from 'node:assert/strict'
import { test } from 'node:test'
import { parseYaml } from '../src/yaml.ts'

test('parses nested mappings and sequences of mappings', () => {
  const doc = `
name: demo
inputs:
  repo:
    description: Path
    required: true
steps:
  - id: first
    run: [python, steps/a.py]
  - id: second
    desc: second step
    set:
      value: 2
`
  assert.deepEqual(parseYaml(doc), {
    name: 'demo',
    inputs: { repo: { description: 'Path', required: true } },
    steps: [
      { id: 'first', run: ['python', 'steps/a.py'] },
      { id: 'second', desc: 'second step', set: { value: 2 } },
    ],
  })
})

test('keeps block scalar content verbatim, including comment markers', () => {
  const doc = `prompt: |
  line one
  # not a comment
    indented
next: done
`
  const parsed = parseYaml(doc) as Record<string, unknown>
  assert.equal(parsed.prompt, 'line one\n# not a comment\n  indented\n')
  assert.equal(parsed.next, 'done')
})

test('strips the trailing newline for chomped block scalars', () => {
  const parsed = parseYaml('text: |-\n  a\n  b\n') as Record<string, string>
  assert.equal(parsed.text, 'a\nb')
})

test('parses scalars, inline lists, and trailing comments', () => {
  const parsed = parseYaml(`
plain: hello
quoted: "a: b"
single: 'it''s'
number: 42
float: 1.5
yes: true
none: null
list: [a, "b, c", 3]
commented: value # trailing
`) as Record<string, unknown>
  assert.equal(parsed.plain, 'hello')
  assert.equal(parsed.quoted, 'a: b')
  assert.equal(parsed.single, "it's")
  assert.equal(parsed.number, 42)
  assert.equal(parsed.float, 1.5)
  assert.equal(parsed.yes, true)
  assert.equal(parsed.none, null)
  assert.deepEqual(parsed.list, ['a', 'b, c', 3])
  assert.equal(parsed.commented, 'value')
})

test('rejects tab indentation', () => {
  assert.throws(() => parseYaml('a:\n\tb: 1\n'), /tabs are not allowed/)
})
