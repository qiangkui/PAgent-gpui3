import assert from 'node:assert/strict'
import { test } from 'node:test'
import { checkTemplateSyntax, renderDeep, renderString } from '../src/template.ts'
import type { Scope } from '../src/types.ts'

const scope: Scope = {
  inputs: { repo: '.', count: 3 },
  vars: { name: 'world', list: ['a', 'b'], empty: '' },
  steps: { scan: { id: 'scan', kind: 'run', status: 'ok', durationMs: 1, json: { files: 5 }, stdout: 'raw\n' } },
  env: { HOME: '/home/me' },
}

test('interpolates plain paths inside text', () => {
  assert.equal(renderString('repo={{ inputs.repo }}!', scope), 'repo=.!')
})

test('renders objects as JSON inside text', () => {
  assert.equal(renderString('data={{ steps.scan.json }}', scope), 'data={"files":5}')
})

test('keeps the original type for a single unfiltered template', () => {
  assert.equal(renderDeep('{{ inputs.count }}', scope), 3)
  assert.deepEqual(renderDeep('{{ steps.scan.json }}', scope), { files: 5 })
  assert.equal(renderDeep('{{ vars.missing }}', scope), undefined)
})

test('applies filters', () => {
  assert.equal(renderString('{{ steps.scan.stdout | trim }}', scope), 'raw')
  assert.equal(renderString('{{ vars.name | upper }}', scope), 'WORLD')
  assert.equal(renderString('{{ vars.list | len }}', scope), '2')
  assert.equal(renderString('{{ vars.list | json }}', scope), '[\n  "a",\n  "b"\n]')
})

test('treats || as a logical operator, not a filter separator', () => {
  assert.equal(renderString('{{ vars.empty || vars.name }}', scope), 'world')
})

test('walks nested objects and arrays', () => {
  assert.deepEqual(
    renderDeep({ a: '{{ vars.name }}', b: ['{{ inputs.count }}'], c: 1 }, scope),
    { a: 'world', b: [3], c: 1 },
  )
})

test('reports unknown filters and unclosed templates', () => {
  assert.match(checkTemplateSyntax('{{ vars.name | nope }}') ?? '', /unknown filter "nope"/)
  assert.match(checkTemplateSyntax('{{ vars.name') ?? '', /unclosed/)
  assert.equal(checkTemplateSyntax('{{ vars.name | trim }}'), null)
})
