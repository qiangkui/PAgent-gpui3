import assert from 'node:assert/strict'
import { test } from 'node:test'
import { checkValue, extractJson, firstIssue, validateSchema } from '../src/schema.ts'

const CLASSIFICATION = {
  type: 'object',
  required: ['category', 'files'],
  additionalProperties: false,
  properties: {
    category: { type: 'string', enum: ['bug', 'question'] },
    files: { type: 'array', items: { type: 'string' }, minItems: 1 },
    confidence: { type: 'number', minimum: 0, maximum: 1 },
  },
}

test('accepts a schema using only the supported subset', () => {
  assert.deepEqual(validateSchema(CLASSIFICATION), [])
})

test('rejects unknown keywords and types', () => {
  assert.match(validateSchema({ type: 'strng' }).join('\n'), /"type" 必须是/)
  assert.match(validateSchema({ oneOf: [] }).join('\n'), /不支持的 schema 关键字 "oneOf"/)
  assert.match(validateSchema({ type: 'object', required: ['a'], properties: {} }).join('\n'), /没有对应的 properties/)
  assert.match(validateSchema({ type: 'array', items: { type: 'wat' } }).join('\n'), /items: "type"/)
  assert.match(validateSchema({}).join('\n'), /至少要有一个约束/)
})

test('accepts a matching object', () => {
  assert.deepEqual(checkValue(CLASSIFICATION, { category: 'bug', files: ['a.ts'], confidence: 0.5 }), [])
  assert.equal(firstIssue(CLASSIFICATION, { category: 'bug', files: ['a.ts'] }), null)
})

test('names the path and the expectation of every mismatch', () => {
  const issues = checkValue(CLASSIFICATION, { category: 'other', files: [], extra: true })
  const text = issues.map(issue => `${issue.path}: ${issue.message}`).join('\n')
  assert.match(text, /\$\.category: 期望取值 "bug" \/ "question"/)
  assert.match(text, /\$\.files: 至少 1 项/)
  assert.match(text, /\$: 不允许额外字段 "extra"/)
  assert.match(checkValue(CLASSIFICATION, { category: 'bug', files: ['a.ts'], confidence: 3 })[0]?.message ?? '', /不能大于 1/)
})

test('reports missing required fields and wrong types', () => {
  assert.match(firstIssue(CLASSIFICATION, { files: ['a.ts'] }) ?? '', /缺少必需字段 "category"/)
  assert.match(firstIssue(CLASSIFICATION, []) ?? '', /期望 object，实际 array/)
  assert.match(firstIssue({ type: 'integer' }, 1.5) ?? '', /期望 integer，实际 number/)
  assert.match(firstIssue({ properties: { a: { type: 'string' } } }, 'x') ?? '', /期望 object，实际 string/)
  assert.match(firstIssue({ items: { type: 'string' } }, 'x') ?? '', /期望 array，实际 string/)
  assert.equal(firstIssue({ type: 'string' }, 'a'), null)
})

test('extracts JSON from bare, fenced, and chatty answers', () => {
  assert.deepEqual(extractJson('{"a":1}'), { ok: true, value: { a: 1 } })
  assert.deepEqual(extractJson('```json\n{"a":[1,2]}\n```'), { ok: true, value: { a: [1, 2] } })
  assert.deepEqual(extractJson('好的，结果是：{"a":"}"}\n希望有帮助'), { ok: true, value: { a: '}' } })
  assert.equal(extractJson('   ').ok, false)
  assert.equal(extractJson('没有 JSON 的回答').ok, false)
})
