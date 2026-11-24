/**
 * Value helpers and the expression language used by `when` and `map.over`.
 *
 * The language is deliberately tiny — literals, dotted paths, comparisons,
 * `&&`/`||`/`!`, parentheses, and a handful of functions. It never evaluates
 * JavaScript, so a manifest generated from a session log cannot smuggle code
 * into the runner.
 */

import type { Scope } from './types.ts'

/**
 * Convert any value into the string form used by templates and messages.
 * @param value - the value to convert.
 * @returns template-ready text; objects become compact JSON.
 */
export function stringify(value: unknown): string {
  if (value === null || value === undefined) return ''
  if (typeof value === 'string') return value
  if (typeof value === 'number' || typeof value === 'boolean' || typeof value === 'bigint') return String(value)
  if (typeof value === 'object') {
    try {
      return JSON.stringify(value)
    } catch (error) {
      const reason = error instanceof Error ? error.message : String(error)
      return `[unserializable: ${reason}]`
    }
  }
  return String(value as symbol)
}

/**
 * JavaScript truthiness with an explicit empty-collection rule.
 * @param value - the value to test.
 * @returns whether the value counts as present.
 */
export function truthy(value: unknown): boolean {
  if (Array.isArray(value)) return value.length > 0
  if (value && typeof value === 'object') return Object.keys(value).length > 0
  return Boolean(value)
}

/**
 * Resolve a dotted path against a scope.
 * @param path - e.g. `steps.scan.json.files`.
 * @param scope - names visible to the expression.
 * @returns the resolved value, or undefined when a segment is absent.
 */
export function resolvePath(path: string, scope: Scope): unknown {
  const segments = path.split('.').map(segment => segment.trim()).filter(segment => segment !== '')
  if (segments.length === 0) return undefined
  const root = segments[0]!
  const bound = ROOTS.has(root)
    ? undefined
    : scope.bindings !== undefined && Object.hasOwn(scope.bindings, root) ? scope.bindings : undefined
  let current: unknown = bound ?? scope as unknown as Record<string, unknown>
  for (const segment of segments) {
    if (current === null || current === undefined) return undefined
    if (Array.isArray(current)) {
      const index = Number(segment)
      if (!Number.isInteger(index)) return undefined
      current = current[index]
      continue
    }
    if (typeof current !== 'object') return undefined
    current = (current as Record<string, unknown>)[segment]
  }
  return current
}

/** Token categories produced by the expression tokenizer. */
type TokenType = 'number' | 'string' | 'path' | 'operator' | 'paren' | 'comma'

/** One lexical token. */
interface Token {
  /** Token category. */
  type: TokenType
  /** Token text. */
  value: string
  /** Offset in the source expression. */
  at: number
}

const COMPARISONS = new Set(['==', '!=', '>=', '<=', '>', '<'])

/** Top-level scope names; any other bare path is looked up in `bindings`. */
const ROOTS = new Set(['inputs', 'vars', 'steps', 'env', 'item', 'index', 'bindings'])

/**
 * Evaluate an expression against a scope.
 * @param source - the expression text.
 * @param scope - names visible to the expression.
 * @returns the evaluated value.
 */
export function evaluate(source: string, scope: Scope): unknown {
  const parser = new ExpressionParser(source, tokenize(source), scope)
  const value = parser.parseExpression(0)
  parser.expectEnd()
  return value
}

/** Pratt parser over the token list. */
class ExpressionParser {
  private readonly source: string
  private readonly tokens: Token[]
  private readonly scope: Scope
  private position = 0

  /**
   * @param source - the original expression, used in error messages.
   * @param tokens - tokenized expression.
   * @param scope - names visible to the expression.
   */
  constructor(source: string, tokens: Token[], scope: Scope) {
    this.source = source
    this.tokens = tokens
    this.scope = scope
  }

  /**
   * Parse an expression whose operators bind no looser than `minPrecedence`.
   * @param minPrecedence - lowest precedence to accept.
   * @returns the evaluated value.
   */
  parseExpression(minPrecedence: number): unknown {
    let left = this.parseUnary()
    for (;;) {
      const token = this.tokens[this.position]
      if (!token || token.type !== 'operator') break
      const precedence = precedenceOf(token.value)
      if (precedence < minPrecedence) break
      this.position += 1
      const right = this.parseExpression(precedence + 1)
      left = applyOperator(token.value, left, right)
    }
    return left
  }

  /**
   * Parse a prefix operator or a primary value.
   * @returns the evaluated value.
   */
  private parseUnary(): unknown {
    const token = this.tokens[this.position]
    if (token && token.type === 'operator' && token.value === '!') {
      this.position += 1
      return !truthy(this.parseUnary())
    }
    if (token && token.type === 'operator' && token.value === '-') {
      this.position += 1
      const value = this.parseUnary()
      if (typeof value !== 'number') this.fail(token, 'unary - needs a number')
      return -value
    }
    return this.parsePrimary()
  }

  /**
   * Parse a literal, a path, a function call, or a parenthesized expression.
   * @returns the evaluated value.
   */
  private parsePrimary(): unknown {
    const token = this.tokens[this.position]
    if (!token) this.fail(null, 'unexpected end of expression')
    if (token.type === 'paren' && token.value === '(') {
      this.position += 1
      const value = this.parseExpression(0)
      this.expect('paren', ')')
      return value
    }
    if (token.type === 'number') {
      this.position += 1
      return Number(token.value)
    }
    if (token.type === 'string') {
      this.position += 1
      return token.value
    }
    if (token.type === 'path') {
      this.position += 1
      const next = this.tokens[this.position]
      if (next && next.type === 'paren' && next.value === '(') return this.callFunction(token.value)
      return this.resolvePathToken(token.value, token)
    }
    this.fail(token, `unexpected token "${token.value}"`)
  }

  /**
   * Evaluate a function call.
   * @param name - the function name.
   * @returns the call result.
   */
  private callFunction(name: string): unknown {
    this.expect('paren', '(')
    const args: unknown[] = []
    if (this.tokens[this.position]?.type === 'paren' && this.tokens[this.position]?.value === ')') {
      this.position += 1
    } else {
      for (;;) {
        args.push(this.parseExpression(0))
        const token = this.tokens[this.position]
        if (token && token.type === 'comma') {
          this.position += 1
          continue
        }
        this.expect('paren', ')')
        break
      }
    }
    return callFunction(name, args, this.source)
  }

  /**
   * Resolve a bare path, honouring `true`, `false`, and `null` keywords.
   * @param path - the dotted path text.
   * @param token - the source token, for error reporting.
   * @returns the resolved value.
   */
  private resolvePathToken(path: string, token: Token): unknown {
    if (path === 'true') return true
    if (path === 'false') return false
    if (path === 'null' || path === 'nil') return null
    if (path === 'undefined') return undefined
    const value = resolvePath(path, this.scope)
    if (value !== undefined) return value
    if (!path.includes('.')) this.fail(token, `unknown name "${path}" (use inputs., vars., steps., or env.)`)
    return undefined
  }

  /**
   * Consume a token of an expected type and value.
   * @param type - expected token type.
   * @param value - expected token text.
   */
  private expect(type: TokenType, value: string): void {
    const token = this.tokens[this.position]
    if (!token || token.type !== type || token.value !== value) {
      this.fail(token ?? null, `expected "${value}"`)
    }
    this.position += 1
  }

  /**
   * Fail when tokens remain after the expression.
   */
  expectEnd(): void {
    const token = this.tokens[this.position]
    if (token) this.fail(token, `unexpected trailing "${token.value}"`)
  }

  /**
   * Throw a positioned parse error.
   * @param token - the offending token, or null at end of input.
   * @param message - what went wrong.
   */
  private fail(token: Token | null, message: string): never {
    const position = token ? ` at ${token.at}` : ' at end'
    throw new Error(`expression ${JSON.stringify(this.source)}${position}: ${message}`)
  }
}

/**
 * Precedence of a binary operator.
 * @param operator - the operator text.
 * @returns a larger number for tighter binding.
 */
function precedenceOf(operator: string): number {
  if (operator === '||') return 1
  if (operator === '&&') return 2
  if (COMPARISONS.has(operator)) return 3
  return 0
}

/**
 * Apply one binary operator.
 * @param operator - the operator text.
 * @param left - left operand.
 * @param right - right operand.
 * @returns the result.
 */
function applyOperator(operator: string, left: unknown, right: unknown): unknown {
  if (operator === '&&') return truthy(left) ? right : left
  if (operator === '||') return truthy(left) ? left : right
  if (operator === '==' || operator === '!=') {
    const equal = looseEqual(left, right)
    return operator === '==' ? equal : !equal
  }
  const a = left as number
  const b = right as number
  if (operator === '>') return a > b
  if (operator === '>=') return a >= b
  if (operator === '<') return a < b
  return a <= b
}

/**
 * Compare two operands, treating numeric strings as numbers.
 * @param left - left operand.
 * @param right - right operand.
 * @returns whether they are equal.
 */
function looseEqual(left: unknown, right: unknown): boolean {
  if (left === right) return true
  if (left === null || left === undefined || right === null || right === undefined) {
    return (left ?? null) === (right ?? null)
  }
  if (typeof left === 'object' || typeof right === 'object') return stringify(left) === stringify(right)
  const asNumberLeft = typeof left === 'string' && left.trim() !== '' ? Number(left) : Number.NaN
  const asNumberRight = typeof right === 'string' && right.trim() !== '' ? Number(right) : Number.NaN
  if (!Number.isNaN(asNumberLeft) && !Number.isNaN(asNumberRight)) return asNumberLeft === asNumberRight
  return String(left) === String(right)
}

/**
 * Evaluate a built-in function call.
 * @param name - function name.
 * @param args - evaluated arguments.
 * @param source - the source expression, for error messages.
 * @returns the call result.
 */
function callFunction(name: string, args: unknown[], source: string): unknown {
  const need = (count: number): void => {
    if (args.length !== count) throw new Error(`expression ${JSON.stringify(source)}: ${name}() takes ${count} argument(s)`)
  }
  switch (name) {
    case 'contains':
      need(2)
      if (Array.isArray(args[0])) return args[0].some(entry => looseEqual(entry, args[1]))
      return stringify(args[0]).includes(stringify(args[1]))
    case 'startswith':
      need(2)
      return stringify(args[0]).startsWith(stringify(args[1]))
    case 'endswith':
      need(2)
      return stringify(args[0]).endsWith(stringify(args[1]))
    case 'matches':
      need(2)
      return new RegExp(stringify(args[1])).test(stringify(args[0]))
    case 'len':
      need(1)
      return Array.isArray(args[0]) ? args[0].length : stringify(args[0]).length
    case 'empty':
      need(1)
      return !truthy(args[0])
    case 'num':
      need(1)
      return Number(stringify(args[0]))
    case 'str':
      need(1)
      return stringify(args[0])
    case 'lower':
      need(1)
      return stringify(args[0]).toLowerCase()
    case 'upper':
      need(1)
      return stringify(args[0]).toUpperCase()
    default:
      throw new Error(`expression ${JSON.stringify(source)}: unknown function "${name}"`)
  }
}

/**
 * Split an expression into tokens.
 * @param source - expression text.
 * @returns the tokens.
 */
function tokenize(source: string): Token[] {
  const tokens: Token[] = []
  let i = 0
  while (i < source.length) {
    const ch = source[i]!
    if (ch === ' ' || ch === '\t' || ch === '\n') {
      i += 1
      continue
    }
    if (ch === '"' || ch === "'") {
      const quote = ch
      let value = ''
      let j = i + 1
      for (; j < source.length; j += 1) {
        const current = source[j]!
        if (current === '\\' && quote === '"' && j + 1 < source.length) {
          value += source[j + 1]!
          j += 1
          continue
        }
        if (current === quote) break
        value += current
      }
      if (j >= source.length) throw new Error(`expression ${JSON.stringify(source)}: unterminated string at ${i}`)
      tokens.push({ type: 'string', value, at: i })
      i = j + 1
      continue
    }
    if (ch === '(' || ch === ')') {
      tokens.push({ type: 'paren', value: ch, at: i })
      i += 1
      continue
    }
    if (ch === ',') {
      tokens.push({ type: 'comma', value: ch, at: i })
      i += 1
      continue
    }
    const two = source.slice(i, i + 2)
    if (two === '==' || two === '!=' || two === '>=' || two === '<=' || two === '&&' || two === '||') {
      tokens.push({ type: 'operator', value: two, at: i })
      i += 2
      continue
    }
    if (ch === '!' || ch === '>' || ch === '<' || ch === '-') {
      tokens.push({ type: 'operator', value: ch, at: i })
      i += 1
      continue
    }
    if (/[0-9]/.test(ch)) {
      let j = i
      while (j < source.length && /[0-9.]/.test(source[j]!)) j += 1
      tokens.push({ type: 'number', value: source.slice(i, j), at: i })
      i = j
      continue
    }
    if (/[A-Za-z_$]/.test(ch)) {
      let j = i
      while (j < source.length && /[A-Za-z0-9_$.]/.test(source[j]!)) j += 1
      tokens.push({ type: 'path', value: source.slice(i, j), at: i })
      i = j
      continue
    }
    throw new Error(`expression ${JSON.stringify(source)}: unexpected character "${ch}" at ${i}`)
  }
  return tokens
}
