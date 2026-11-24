/**
 * The JSON-schema subset used to constrain `ai` step output.
 *
 * A model answer is only usable as a step result when it has a shape the
 * workflow can rely on. The subset here is deliberately small
 * (`type` / `properties` / `required` / `items` / `enum` and a few bounds) so a
 * generated manifest cannot smuggle in keywords whose meaning the runner does
 * not implement.
 */

/** One reason a value does not match a schema. */
export interface SchemaIssue {
  /** Position inside the value, for example `$.files[0].size`. */
  path: string
  /** What is wrong, in readable Chinese. */
  message: string
}

/** Keywords the runner understands; anything else is a manifest error. */
const KEYWORDS = new Set([
  'type',
  'description',
  'properties',
  'required',
  'items',
  'enum',
  'minimum',
  'maximum',
  'minLength',
  'maxLength',
  'minItems',
  'maxItems',
  'additionalProperties',
])

/** Type names understood by {@link checkValue}. */
const TYPES = new Set(['object', 'array', 'string', 'number', 'integer', 'boolean', 'null'])

/** At least one of these must appear, or the schema constrains nothing. */
const CONSTRAINING = ['type', 'properties', 'required', 'items', 'enum', 'additionalProperties']

/**
 * Statically check a schema definition.
 *
 * Run at manifest load time so a typo (`properites`, `type: strng`) fails
 * `validate` instead of surfacing after a paid model call.
 * @param schema - the parsed `ai.schema` value.
 * @param path - position used in messages.
 * @returns one message per problem, empty when the schema is usable.
 */
export function validateSchema(schema: unknown, path = 'schema'): string[] {
  if (!isRecord(schema)) return [`${path}: 必须是映射（object）`]
  const problems: string[] = []

  for (const key of Object.keys(schema)) {
    if (!KEYWORDS.has(key)) problems.push(`${path}: 不支持的 schema 关键字 "${key}"（可用：${[...KEYWORDS].join(', ')}）`)
  }

  if (schema.type !== undefined) {
    if (typeof schema.type !== 'string' || !TYPES.has(schema.type)) {
      problems.push(`${path}: "type" 必须是 ${[...TYPES].join(' / ')} 之一`)
    }
  }

  if (schema.enum !== undefined) {
    if (!Array.isArray(schema.enum) || schema.enum.length === 0) problems.push(`${path}: "enum" 必须是非空列表`)
    else if (schema.enum.some(entry => entry !== null && typeof entry === 'object')) {
      problems.push(`${path}: "enum" 只支持字符串、数字、布尔和 null`)
    }
  }

  if (schema.properties !== undefined) {
    if (!isRecord(schema.properties)) problems.push(`${path}: "properties" 必须是映射`)
    else {
      for (const [name, child] of Object.entries(schema.properties)) {
        problems.push(...validateSchema(child, `${path}.properties.${name}`))
      }
    }
  }

  if (schema.required !== undefined) {
    if (!Array.isArray(schema.required) || schema.required.some(entry => typeof entry !== 'string')) {
      problems.push(`${path}: "required" 必须是字符串列表`)
    } else if (isRecord(schema.properties)) {
      for (const name of schema.required as string[]) {
        if (!(name in schema.properties)) problems.push(`${path}: "required" 里的 "${name}" 没有对应的 properties 定义`)
      }
    }
  }

  if (schema.items !== undefined) problems.push(...validateSchema(schema.items, `${path}.items`))

  const bounds: Array<[string, unknown]> = [
    ['minimum', schema.minimum],
    ['maximum', schema.maximum],
    ['minLength', schema.minLength],
    ['maxLength', schema.maxLength],
    ['minItems', schema.minItems],
    ['maxItems', schema.maxItems],
  ]
  for (const [name, value] of bounds) {
    if (value === undefined) continue
    if (typeof value !== 'number' || Number.isNaN(value)) problems.push(`${path}: "${name}" 必须是数字`)
  }
  if (typeof schema.minimum === 'number' && typeof schema.maximum === 'number' && schema.minimum > schema.maximum) {
    problems.push(`${path}: "minimum" 大于 "maximum"`)
  }
  if (typeof schema.minLength === 'number' && typeof schema.maxLength === 'number' && schema.minLength > schema.maxLength) {
    problems.push(`${path}: "minLength" 大于 "maxLength"`)
  }
  if (schema.additionalProperties !== undefined && typeof schema.additionalProperties !== 'boolean') {
    problems.push(`${path}: "additionalProperties" 必须是布尔值`)
  }

  if (!CONSTRAINING.some(key => key in schema)) {
    problems.push(`${path}: 至少要有一个约束（${CONSTRAINING.join(' / ')}），否则等于没有 schema`)
  }

  return problems
}

/**
 * Check one value against a schema.
 * @param schema - a schema already accepted by {@link validateSchema}.
 * @param value - the candidate value.
 * @param path - position used in messages; defaults to `$`.
 * @returns one issue per mismatch, empty when the value matches.
 */
export function checkValue(schema: unknown, value: unknown, path = '$'): SchemaIssue[] {
  if (!isRecord(schema)) return [{ path, message: 'schema 不是映射' }]

  const issues: SchemaIssue[] = []
  const expected = expectedType(schema)
  if (expected !== undefined && !matchesType(expected, value)) {
    return [{ path, message: `期望 ${expected}，实际 ${typeName(value)}` }]
  }

  if (Array.isArray(schema.enum) && !schema.enum.some(entry => sameValue(entry, value))) {
    return [{ path, message: `期望取值 ${schema.enum.map(entry => JSON.stringify(entry)).join(' / ')}，实际 ${JSON.stringify(value)}` }]
  }

  if (typeof value === 'string') {
    if (typeof schema.minLength === 'number' && value.length < schema.minLength) {
      issues.push({ path, message: `长度至少 ${schema.minLength}，实际 ${value.length}` })
    }
    if (typeof schema.maxLength === 'number' && value.length > schema.maxLength) {
      issues.push({ path, message: `长度最多 ${schema.maxLength}，实际 ${value.length}` })
    }
  }

  if (typeof value === 'number') {
    if (typeof schema.minimum === 'number' && value < schema.minimum) issues.push({ path, message: `不能小于 ${schema.minimum}，实际 ${value}` })
    if (typeof schema.maximum === 'number' && value > schema.maximum) issues.push({ path, message: `不能大于 ${schema.maximum}，实际 ${value}` })
  }

  if (Array.isArray(value)) {
    if (typeof schema.minItems === 'number' && value.length < schema.minItems) {
      issues.push({ path, message: `至少 ${schema.minItems} 项，实际 ${value.length}` })
    }
    if (typeof schema.maxItems === 'number' && value.length > schema.maxItems) {
      issues.push({ path, message: `最多 ${schema.maxItems} 项，实际 ${value.length}` })
    }
    if (schema.items !== undefined) {
      value.forEach((entry, index) => issues.push(...checkValue(schema.items, entry, `${path}[${index}]`)))
    }
  }

  if (isRecord(value)) {
    const properties = isRecord(schema.properties) ? schema.properties : {}
    for (const name of Array.isArray(schema.required) ? schema.required as string[] : []) {
      if (!(name in value)) issues.push({ path, message: `缺少必需字段 "${name}"` })
    }
    for (const [name, entry] of Object.entries(value)) {
      if (name in properties) issues.push(...checkValue(properties[name], entry, `${path}.${name}`))
      else if (schema.additionalProperties === false) issues.push({ path, message: `不允许额外字段 "${name}"` })
    }
  }

  return issues
}

/**
 * Render the first mismatch as a single message.
 * @param schema - the schema.
 * @param value - the candidate.
 * @returns a readable message, or null when the value matches.
 */
export function firstIssue(schema: unknown, value: unknown): string | null {
  const issues = checkValue(schema, value)
  if (issues.length === 0) return null
  const [first] = issues
  return `${first!.path}: ${first!.message}`
}

/**
 * Pull a JSON value out of a model reply.
 *
 * Models answer with bare JSON, with a fenced block, or with a sentence in
 * front of the object; all three are common enough that the step should not
 * fail on formatting alone.
 * @param text - the reply text.
 * @returns the parsed value, or an error message.
 */
export function extractJson(text: string): { ok: true, value: unknown } | { ok: false, error: string } {
  const trimmed = text.replace(/^\uFEFF/, '').trim()
  if (trimmed === '') return { ok: false, error: '回答为空' }

  const unfenced = stripFence(trimmed)
  const direct = tryParse(unfenced)
  if (direct.ok) return direct

  const start = firstJsonStart(unfenced)
  if (start >= 0) {
    const end = matchingEnd(unfenced, start)
    if (end > start) {
      const sliced = tryParse(unfenced.slice(start, end + 1))
      if (sliced.ok) return sliced
    }
  }
  return { ok: false, error: '回答里没有可解析的 JSON' }
}

/**
 * Try `JSON.parse`.
 * @param text - candidate text.
 * @returns the parsed value or a reason.
 */
function tryParse(text: string): { ok: true, value: unknown } | { ok: false, error: string } {
  try {
    return { ok: true, value: JSON.parse(text) }
  } catch (error) {
    return { ok: false, error: error instanceof Error ? error.message : String(error) }
  }
}

/**
 * Remove a Markdown code fence around the whole reply.
 * @param text - trimmed reply.
 * @returns the content without the fence, unchanged when there is none.
 */
function stripFence(text: string): string {
  if (!text.startsWith('```')) return text
  const firstNewline = text.indexOf('\n')
  if (firstNewline < 0) return text
  const body = text.slice(firstNewline + 1)
  const close = body.lastIndexOf('```')
  return (close < 0 ? body : body.slice(0, close)).trim()
}

/**
 * Index of the first `{` or `[` outside a fence.
 * @param text - candidate text.
 * @returns the index, or -1.
 */
function firstJsonStart(text: string): number {
  for (let index = 0; index < text.length; index += 1) {
    const ch = text[index]
    if (ch === '{' || ch === '[') return index
  }
  return -1
}

/**
 * Find where the JSON value starting at `start` ends.
 * @param text - the text.
 * @param start - index of the opening bracket.
 * @returns the index of the matching closing bracket, or -1.
 */
function matchingEnd(text: string, start: number): number {
  const open = text[start]
  const close = open === '{' ? '}' : ']'
  let depth = 0
  let inString = false
  let escaped = false
  for (let index = start; index < text.length; index += 1) {
    const ch = text[index]!
    if (inString) {
      if (escaped) escaped = false
      else if (ch === '\\') escaped = true
      else if (ch === '"') inString = false
      continue
    }
    if (ch === '"') {
      inString = true
      continue
    }
    if (ch === open) depth += 1
    else if (ch === close) {
      depth -= 1
      if (depth === 0) return index
    }
  }
  return -1
}

/**
 * The type a schema implies, even when `type` is omitted.
 * @param schema - the schema.
 * @returns the type name, or undefined when the value is unconstrained.
 */
function expectedType(schema: Record<string, unknown>): string | undefined {
  if (typeof schema.type === 'string') return schema.type
  if (isRecord(schema.properties) || Array.isArray(schema.required)) return 'object'
  if (schema.items !== undefined) return 'array'
  return undefined
}

/**
 * Report whether a value has the requested JSON type.
 * @param expected - schema type name.
 * @param value - candidate.
 * @returns true when the value matches.
 */
function matchesType(expected: string, value: unknown): boolean {
  switch (expected) {
    case 'object':
      return isRecord(value)
    case 'array':
      return Array.isArray(value)
    case 'string':
      return typeof value === 'string'
    case 'number':
      return typeof value === 'number' && !Number.isNaN(value)
    case 'integer':
      return typeof value === 'number' && Number.isInteger(value)
    case 'boolean':
      return typeof value === 'boolean'
    case 'null':
      return value === null
    default:
      return true
  }
}

/**
 * Name a value's JSON type for messages.
 * @param value - the value.
 * @returns the type name.
 */
function typeName(value: unknown): string {
  if (value === null) return 'null'
  if (Array.isArray(value)) return 'array'
  return typeof value
}

/**
 * Compare two JSON values structurally.
 * @param left - first value.
 * @param right - second value.
 * @returns true when they are the same JSON value.
 */
function sameValue(left: unknown, right: unknown): boolean {
  if (left === right) return true
  return JSON.stringify(left) === JSON.stringify(right)
}

/**
 * Report whether a value is a plain object.
 * @param value - candidate.
 * @returns true for non-array objects.
 */
function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}
