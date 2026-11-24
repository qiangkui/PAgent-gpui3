import assert from 'node:assert/strict'
import { test } from 'node:test'
import { evaluate, resolvePath, truthy } from '../src/expr.ts'
import type { Scope } from '../src/types.ts'

const scope: Scope = {
  inputs: { repo: '.', count: 3 },
  vars: { flagged: true, list: ['a', 'b'] },
  steps: { check: { status: 'ok', exitCode: 0, stdout: 'PASS\n', json: { total: 7, files: ['x.ts'] }, durationMs: 1, id: 'check', kind: 'run' } },
  env: { HOME: '/home/me' },
}

test('resolves dotted paths and array indexes', () => {
  assert.equal(resolvePath('steps.check.json.total', scope), 7)
  assert.equal(resolvePath('vars.list.1', scope), 'b')
  assert.equal(resolvePath('missing.deep.value', scope), undefined)
})

test('compares numbers, strings, and booleans', () => {
  assert.equal(evaluate('steps.check.json.total == 7', scope), true)
  assert.equal(evaluate('steps.check.exitCode == 0', scope), true)
  assert.equal(evaluate('inputs.repo != "main"', scope), true)
  assert.equal(evaluate('steps.check.json.total > 10', scope), false)
})

test('honours && before ||', () => {
  assert.equal(evaluate('false && true || true', scope), true)
  assert.equal(evaluate('true || false && false', scope), true)
  assert.equal(evaluate('(true || false) && false', scope), false)
})

test('evaluates string and collection helpers', () => {
  assert.equal(evaluate('contains(steps.check.stdout, "PASS")', scope), true)
  assert.equal(evaluate('contains(steps.check.json.files, "x.ts")', scope), true)
  assert.equal(evaluate('len(vars.list) == 2', scope), true)
  assert.equal(evaluate('empty(vars.missing)', scope), true)
  assert.equal(evaluate('startswith(inputs.repo, ".")', scope), true)
})

test('rejects unknown bare names so typos fail loudly', () => {
  assert.throws(() => evaluate('mystery == 1', scope), /unknown name "mystery"/)
})

test('treats empty collections as absent', () => {
  assert.equal(truthy([]), false)
  assert.equal(truthy({}), false)
  assert.equal(truthy([0]), true)
  assert.equal(truthy('0'), true)
  assert.equal(truthy(''), false)
})
