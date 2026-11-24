import assert from 'node:assert/strict'
import { test } from 'node:test'
import { loadWorkflow } from '../src/load.ts'
import { makeProject } from './helpers.ts'

test('accepts a minimal manifest and normalizes the run form', () => {
  const dir = makeProject({
    'flow.yml': 'name: t\ndescription: d\nversion: 1\nsteps:\n  - id: a\n    run: [node, -e, "0"]\n',
  })
  const loaded = loadWorkflow(`${dir}/flow.yml`)
  assert.equal(loaded.workflow.steps[0]?.run?.command[1], '-e')
  assert.equal(loaded.workflow.version, 1)
})

test('treats a string command as a shell command', () => {
  const dir = makeProject({ 'flow.yml': 'name: t\ndescription: d\nsteps:\n  - id: a\n    run: echo hi\n' })
  const loaded = loadWorkflow(`${dir}/flow.yml`)
  assert.equal(loaded.workflow.steps[0]?.run?.shell, true)
})

test('rejects duplicate step ids', () => {
  const dir = makeProject({
    'flow.yml': 'name: t\ndescription: d\nsteps:\n  - id: a\n    run: [node, -e, "0"]\n  - id: a\n    run: [node, -e, "0"]\n',
  })
  assert.throws(() => loadWorkflow(`${dir}/flow.yml`), /duplicate step id "a"/)
})

test('rejects a step without exactly one executor', () => {
  const dir = makeProject({
    'flow.yml': 'name: t\ndescription: d\nsteps:\n  - id: a\n    run: [node, -e, "0"]\n    set:\n      x: 1\n',
  })
  assert.throws(() => loadWorkflow(`${dir}/flow.yml`), /may only use one of "run", "set", "map", or "ai"/)
})

test('rejects an unsupported version', () => {
  const dir = makeProject({ 'flow.yml': 'name: t\ndescription: d\nversion: 2\nsteps:\n  - id: a\n    run: [node, -e, "0"]\n' })
  assert.throws(() => loadWorkflow(`${dir}/flow.yml`), /unsupported "version" 2/)
})

test('normalizes once keys', () => {
  const dir = makeProject({
    'flow.yml': 'name: t\ndescription: d\nsteps:\n  - id: a\n    once: true\n    run: [node, -e, "0"]\n  - id: b\n    once: deploy\n    run: [node, -e, "0"]\n',
  })
  const loaded = loadWorkflow(`${dir}/flow.yml`)
  assert.equal(loaded.workflow.steps[0]?.once, true)
  assert.equal(loaded.workflow.steps[1]?.once, 'deploy')

  const bad = makeProject({
    'flow.yml': 'name: t\ndescription: d\nsteps:\n  - id: a\n    once: 3\n    run: [node, -e, "0"]\n',
  })
  assert.throws(() => loadWorkflow(`${bad}/flow.yml`), /non-boolean "once"/)
})

test('normalizes services and readiness probes', () => {
  const dir = makeProject({
    'flow.yml': `name: t
description: d
services:
  api:
    run:
      command: [node, server.mjs]
      env:
        PORT: "{{ inputs.port }}"
    ready:
      url: "http://127.0.0.1:{{ inputs.port }}/health"
      timeoutMs: 5000
  plain:
    run: [node, server.mjs]
    ready: http://127.0.0.1:9/health
steps:
  - id: a
    run: [node, -e, "0"]
`,
  })
  const loaded = loadWorkflow(`${dir}/flow.yml`)
  assert.equal(loaded.workflow.services?.api?.run.env?.PORT, '{{ inputs.port }}')
  assert.equal(loaded.workflow.services?.api?.ready?.timeoutMs, 5000)
  assert.equal(loaded.workflow.services?.plain?.ready?.url, 'http://127.0.0.1:9/health')
})

test('rejects a service without run and a non-http ready URL', () => {
  const missing = makeProject({
    'flow.yml': 'name: t\ndescription: d\nservices:\n  api:\n    ready: http://127.0.0.1:9/\nsteps:\n  - id: a\n    run: [node, -e, "0"]\n',
  })
  assert.throws(() => loadWorkflow(`${missing}/flow.yml`), /needs "run"/)

  const scheme = makeProject({
    'flow.yml': 'name: t\ndescription: d\nservices:\n  api:\n    run: [node, server.mjs]\n    ready: tcp://127.0.0.1:9\nsteps:\n  - id: a\n    run: [node, -e, "0"]\n',
  })
  assert.throws(() => loadWorkflow(`${scheme}/flow.yml`), /must start with http/)
})

test('warns about unknown keys but still loads', () => {
  const dir = makeProject({ 'flow.yml': 'name: t\ndescription: d\nsteps:\n  - id: a\n    run: [node, -e, "0"]\n    typo: 1\n' })
  const loaded = loadWorkflow(`${dir}/flow.yml`)
  assert.match(loaded.warnings.join('\n'), /unknown key "typo"/)
})

test('rejects an empty step list and a missing name', () => {
  const empty = makeProject({ 'flow.yml': 'name: t\ndescription: d\nsteps: []\n' })
  assert.throws(() => loadWorkflow(`${empty}/flow.yml`), /"steps" must be a non-empty list/)
  const nameless = makeProject({ 'flow.yml': 'description: d\nsteps:\n  - id: a\n    run: [node, -e, "0"]\n' })
  assert.throws(() => loadWorkflow(`${nameless}/flow.yml`), /"name" is required/)
})

test('loads JSON manifests', () => {
  const dir = makeProject({
    'flow.json': JSON.stringify({ name: 'j', description: 'd', steps: [{ id: 'a', run: ['node', '-e', '0'] }] }),
  })
  assert.equal(loadWorkflow(`${dir}/flow.json`).workflow.name, 'j')
})
