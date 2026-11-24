/**
 * Template interpolation: `{{ path | filter }}` inside manifest strings.
 *
 * Templates only interpolate; conditions and collection expressions use the
 * separate expression language in `expr.ts`. A string that consists of exactly
 * one unfiltered template keeps the value's original type, so `"{{ steps.x.json.count }}"`
 * stays a number.
 */

import { evaluate, resolvePath, stringify, truthy } from './expr.ts'
import type { Scope } from './types.ts'

/** One filter name understood by the renderer. */
type FilterName = 'trim' | 'upper' | 'lower' | 'json' | 'len' | 'first' | 'last' | 'lines'

const FILTERS = new Set<string>(['trim', 'upper', 'lower', 'json', 'len', 'first', 'last', 'lines'])

/**
 * Report whether text contains at least one `{{ ... }}` template.
 * @param text - candidate string.
 * @returns true when interpolation would change the text.
 */
export function hasTemplate(text: string): boolean {
  return text.includes('{{') && text.includes('}}')
}

/**
 * Render every template in a string.
 * @param text - the source string.
 * @param scope - names visible to the templates.
 * @returns the interpolated string.
 */
export function renderString(text: string, scope: Scope): string {
  if (!hasTemplate(text)) return text
  let out = ''
  let cursor = 0
  for (;;) {
    const open = text.indexOf('{{', cursor)
    if (open < 0) {
      out += text.slice(cursor)
      return out
    }
    const close = text.indexOf('}}', open + 2)
    if (close < 0) throw new Error(`template: unclosed "{{" in ${JSON.stringify(text)}`)
    out += text.slice(cursor, open)
    const inner = text.slice(open + 2, close).trim()
    out += stringify(applyFilters(inner, scope))
    cursor = close + 2
  }
}

/**
 * Check template syntax without a scope: balanced braces and known filters.
 * @param text - candidate string.
 * @returns an error message, or null when the templates are well formed.
 */
export function checkTemplateSyntax(text: string): string | null {
  let cursor = 0
  for (;;) {
    const open = text.indexOf('{{', cursor)
    if (open < 0) return null
    const close = text.indexOf('}}', open + 2)
    if (close < 0) return `unclosed "{{" in ${JSON.stringify(text)}`
    const inner = text.slice(open + 2, close).trim()
    for (const filter of splitFilters(inner).filters) {
      const name = filter.split('(')[0]!.trim()
      if (!FILTERS.has(name)) return `unknown filter "${name}" (have: ${[...FILTERS].join(', ')})`
    }
    cursor = close + 2
  }
}

/**
 * Render a value: strings are interpolated, objects and arrays are walked.
 * A bare single-template string keeps its resolved type.
 * @param value - the value to render.
 * @param scope - names visible to the templates.
 * @returns the rendered value.
 */
export function renderDeep(value: unknown, scope: Scope): unknown {
  if (typeof value === 'string') {
    const whole = matchWholeTemplate(value)
    if (whole !== null) {
      const parts = splitFilters(whole)
      if (parts.filters.length === 0) return evaluateOrResolve(parts.expression, scope)
      return applyFilters(whole, scope)
    }
    return renderString(value, scope)
  }
  if (Array.isArray(value)) return value.map(entry => renderDeep(entry, scope))
  if (value && typeof value === 'object') {
    const out: Record<string, unknown> = {}
    for (const [key, entry] of Object.entries(value)) out[key] = renderDeep(entry, scope)
    return out
  }
  return value
}

/**
 * Evaluate a `when` expression and interpret the result as a condition.
 * @param source - the expression text.
 * @param scope - names visible to the expression.
 * @returns whether the step should run.
 */
export function evaluateCondition(source: string, scope: Scope): boolean {
  return truthy(evaluate(source, scope))
}

/**
 * Return the inner text when the string is exactly one template.
 * @param text - candidate string.
 * @returns the template body, or null.
 */
function matchWholeTemplate(text: string): string | null {
  const trimmed = text.trim()
  if (!trimmed.startsWith('{{') || !trimmed.endsWith('}}')) return null
  const inner = trimmed.slice(2, -2)
  if (inner.includes('{{') || inner.includes('}}')) return null
  return inner.trim()
}

/**
 * Split a template body into its expression and filter list.
 * `||` is a logical operator, not a filter separator.
 * @param inner - template body text.
 * @returns the expression and the ordered filters.
 */
function splitFilters(inner: string): { expression: string, filters: string[] } {
  const filters: string[] = []
  let expression = ''
  let current = ''
  let quote: string | null = null
  let depth = 0
  for (let i = 0; i < inner.length; i += 1) {
    const ch = inner[i]!
    if (quote) {
      current += ch
      if (ch === quote) quote = null
      continue
    }
    if (ch === '"' || ch === "'") {
      quote = ch
      current += ch
      continue
    }
    if (ch === '(') depth += 1
    if (ch === ')') depth -= 1
    if (ch === '|' && depth === 0 && inner[i + 1] !== '|' && inner[i - 1] !== '|') {
      if (expression === '') expression = current
      else filters.push(current.trim())
      current = ''
      continue
    }
    current += ch
  }
  if (expression === '') expression = current
  else filters.push(current.trim())
  return { expression: expression.trim(), filters: filters.filter(filter => filter !== '') }
}

/**
 * Evaluate the expression part of a template body.
 * @param expression - the expression text.
 * @param scope - names visible to the expression.
 * @returns the resolved value.
 */
function evaluateOrResolve(expression: string, scope: Scope): unknown {
  if (expression === '') return ''
  const barePath = /^[A-Za-z_$][A-Za-z0-9_$.]*$/.test(expression)
  if (barePath) return resolvePath(expression, scope)
  return evaluate(expression, scope)
}

/**
 * Evaluate a template body and apply its filters.
 * @param inner - template body text.
 * @param scope - names visible to the template.
 * @returns the filtered value.
 */
function applyFilters(inner: string, scope: Scope): unknown {
  const { expression, filters } = splitFilters(inner)
  let value = evaluateOrResolve(expression, scope)
  for (const filter of filters) {
    const name = filter.split('(')[0]!.trim() as FilterName
    if (!FILTERS.has(name)) throw new Error(`template: unknown filter "${name}" (have: ${[...FILTERS].join(', ')})`)
    value = applyFilter(name, value)
  }
  return value
}

/**
 * Apply one named filter.
 * @param name - filter name.
 * @param value - the input value.
 * @returns the filtered value.
 */
function applyFilter(name: FilterName, value: unknown): unknown {
  switch (name) {
    case 'trim':
      return stringify(value).trim()
    case 'upper':
      return stringify(value).toUpperCase()
    case 'lower':
      return stringify(value).toLowerCase()
    case 'json':
      return JSON.stringify(value, null, 2)
    case 'len':
      return Array.isArray(value) ? value.length : stringify(value).length
    case 'first':
      return Array.isArray(value) ? value[0] : stringify(value).split('\n')[0]
    case 'last':
      return Array.isArray(value) ? value[value.length - 1] : stringify(value).trimEnd().split('\n').at(-1)
    case 'lines':
      return stringify(value).split(/\r?\n/).filter(line => line.trim() !== '')
    default:
      return value
  }
}
