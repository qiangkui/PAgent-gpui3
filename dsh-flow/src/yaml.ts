/**
 * Minimal YAML reader for workflow manifests.
 *
 * Supports the subset the format needs: block mappings and sequences, plain and
 * quoted scalars, inline `[a, b]` sequences, and block scalars (`|`, `>` with an
 * optional `-` chomping marker). Anchors, aliases, tags, and multiple documents
 * are rejected rather than guessed at.
 */

/** One physical line, with its indentation and structural classification. */
interface Line {
  /** Leading spaces; -1 for blank and comment lines. */
  indent: number
  /** Content after the indentation, with any trailing comment still present. */
  text: string
  /** Raw content after the indentation, preserved for block scalars. */
  rawContent: string
  /** The line carries no structure. */
  blank: boolean
  /** Whole-line comment. */
  comment: boolean
}

/**
 * Parse YAML text into plain JavaScript values.
 * @param source - YAML document text.
 * @returns the parsed root value.
 */
export function parseYaml(source: string): unknown {
  return new YamlParser(source).parseDocument()
}

/** Indentation-driven recursive-descent reader over classified lines. */
class YamlParser {
  private readonly lines: Line[]
  private index = 0

  /**
   * @param source - YAML document text.
   */
  constructor(source: string) {
    this.lines = classify(source)
  }

  /**
   * Parse the document root.
   * @returns the root value.
   */
  parseDocument(): unknown {
    const first = this.peek()
    if (!first) return null
    const value = this.parseNode(first.indent)
    const rest = this.peek()
    if (rest) throw new Error(`yaml: unexpected content "${rest.text}" (check indentation)`)
    return value
  }

  /**
   * Return the next structural line without consuming it.
   * @returns the line, or null at end of input.
   */
  private peek(): Line | null {
    for (let j = this.index; j < this.lines.length; j += 1) {
      const line = this.lines[j]!
      if (line.blank || line.comment) continue
      this.index = j
      return line
    }
    this.index = this.lines.length
    return null
  }

  /**
   * Parse a block whose entries start at `indent`.
   * @param indent - the block's indentation.
   * @returns the parsed block.
   */
  private parseNode(indent: number): unknown {
    const line = this.peek()
    if (!line) return null
    if (isSequenceItem(line.text)) return this.parseSequence(indent)
    if (findKeySeparator(line.text) >= 0) return this.parseMapping(indent)
    this.index += 1
    return parseScalar(line.text)
  }

  /**
   * Parse a block mapping.
   * @param indent - key indentation.
   * @returns the mapping.
   */
  private parseMapping(indent: number): Record<string, unknown> {
    const out: Record<string, unknown> = {}
    for (;;) {
      const line = this.peek()
      if (!line) return out
      if (line.indent !== indent) {
        if (line.indent > indent) throw new Error(`yaml: unexpected indentation at "${line.text}"`)
        return out
      }
      if (isSequenceItem(line.text)) return out
      if (findKeySeparator(line.text) < 0) throw new Error(`yaml: expected "key: value" at "${line.text}"`)
      this.index += 1
      this.assignPair(out, line.text, indent)
    }
  }

  /**
   * Parse a block sequence.
   * @param indent - dash indentation.
   * @returns the array.
   */
  private parseSequence(indent: number): unknown[] {
    const out: unknown[] = []
    for (;;) {
      const line = this.peek()
      if (!line || line.indent !== indent || !isSequenceItem(line.text)) return out
      this.index += 1
      const content = line.text === '-' ? '' : line.text.slice(2).trim()
      if (content === '') {
        const next = this.peek()
        out.push(next && next.indent > indent ? this.parseNode(next.indent) : null)
        continue
      }
      if (findKeySeparator(content) < 0) {
        out.push(parseScalar(content))
        continue
      }
      const itemIndent = indent + 2
      const entry: Record<string, unknown> = {}
      this.assignPair(entry, content, itemIndent)
      for (;;) {
        const following = this.peek()
        if (!following || following.indent !== itemIndent) break
        if (isSequenceItem(following.text)) break
        if (findKeySeparator(following.text) < 0) {
          throw new Error(`yaml: expected "key: value" at "${following.text}"`)
        }
        this.index += 1
        this.assignPair(entry, following.text, itemIndent)
      }
      out.push(entry)
    }
  }

  /**
   * Store one `key: value` pair, resolving nested blocks and block scalars.
   * @param target - the mapping receiving the pair.
   * @param text - the pair's source text.
   * @param indent - the key's indentation.
   */
  private assignPair(target: Record<string, unknown>, text: string, indent: number): void {
    const separator = findKeySeparator(text)
    if (separator < 0) throw new Error(`yaml: expected "key: value" at "${text}"`)
    const key = unquoteKey(text.slice(0, separator).trim())
    const rest = text.slice(separator + 1).trim()
    if (rest.startsWith('|') || rest.startsWith('>')) {
      target[key] = this.readBlockScalar(indent, rest)
      return
    }
    if (rest !== '') {
      target[key] = parseScalar(rest)
      return
    }
    const next = this.peek()
    target[key] = next && next.indent > indent ? this.parseNode(next.indent) : null
  }

  /**
   * Read a block scalar body starting at the current raw line.
   * @param keyIndent - indentation of the owning key.
   * @param marker - the scalar header, e.g. `|`, `|-`, `>`.
   * @returns the scalar text.
   */
  private readBlockScalar(keyIndent: number, marker: string): string {
    const collected: Line[] = []
    let j = this.index
    for (; j < this.lines.length; j += 1) {
      const line = this.lines[j]!
      if (!line.blank && line.indent <= keyIndent) break
      collected.push(line)
    }
    this.index = j
    while (collected.length > 0 && collected[collected.length - 1]!.blank) collected.pop()
    const indents = collected.filter(line => !line.blank).map(line => line.indent)
    const base = indents.length > 0 ? Math.min(...indents) : 0
    const body = collected.map(line => (line.blank ? '' : ' '.repeat(line.indent - base) + line.rawContent)).join('\n')
    if (marker.includes('-')) return body
    return body === '' ? '' : `${body}\n`
  }
}

/**
 * Classify every physical line.
 * @param source - document text.
 * @returns the classified lines.
 */
function classify(source: string): Line[] {
  const out: Line[] = []
  for (const raw of source.split(/\r?\n/)) {
    if (raw.trim() === '') {
      out.push({ indent: -1, text: '', rawContent: '', blank: true, comment: false })
      continue
    }
    const content = raw.replace(/^[ \t]+/, '')
    const leading = raw.slice(0, raw.length - content.length)
    if (leading.includes('\t')) throw new Error('yaml: tabs are not allowed for indentation')
    const indent = leading.length
    if (content === '---' || content === '...') {
      out.push({ indent: -1, text: '', rawContent: '', blank: true, comment: false })
      continue
    }
    if (content.startsWith('#')) {
      out.push({ indent, text: content, rawContent: content, blank: false, comment: true })
      continue
    }
    out.push({ indent, text: content, rawContent: content, blank: false, comment: false })
  }
  return out
}

/**
 * Report whether text opens a sequence entry.
 * @param text - trimmed line text.
 * @returns true for `-` and `- ...`.
 */
function isSequenceItem(text: string): boolean {
  return text === '-' || text.startsWith('- ')
}

/**
 * Locate the colon that separates a key from its value.
 * @param text - trimmed line text.
 * @returns the colon index, or -1 when the line is not a mapping entry.
 */
function findKeySeparator(text: string): number {
  let quote: string | null = null
  for (let i = 0; i < text.length; i += 1) {
    const ch = text[i]!
    if (quote) {
      if (ch === quote) quote = null
      continue
    }
    if (ch === '"' || ch === "'") {
      quote = ch
      continue
    }
    if (ch === ':' && (i + 1 === text.length || text[i + 1] === ' ')) return i
  }
  return -1
}

/**
 * Remove surrounding quotes from a mapping key.
 * @param raw - key text.
 * @returns the unquoted key.
 */
function unquoteKey(raw: string): string {
  return raw.length >= 2 && raw.startsWith('"') && raw.endsWith('"') ? raw.slice(1, -1) : raw
}

/**
 * Parse one scalar value.
 * @param raw - value text.
 * @returns the parsed value.
 */
function parseScalar(raw: string): unknown {
  const text = stripTrailingComment(raw).trim()
  if (text === '') return null
  if (text.startsWith('[')) {
    if (!text.endsWith(']')) throw new Error(`yaml: unterminated inline sequence ${raw}`)
    const inner = text.slice(1, -1).trim()
    return inner === '' ? [] : splitTopLevel(inner).map(parseScalar)
  }
  if (text.startsWith('"')) {
    if (!text.endsWith('"') || text.length < 2) throw new Error(`yaml: unterminated string ${raw}`)
    return JSON.parse(text)
  }
  if (text.startsWith("'")) {
    if (!text.endsWith("'") || text.length < 2) throw new Error(`yaml: unterminated string ${raw}`)
    return text.slice(1, -1).replaceAll("''", "'")
  }
  if (text === 'true') return true
  if (text === 'false') return false
  if (text === 'null' || text === '~') return null
  if (/^-?\d+$/.test(text)) return Number.parseInt(text, 10)
  if (/^-?\d+\.\d+$/.test(text)) return Number.parseFloat(text)
  return text
}

/**
 * Drop a trailing `# comment` from an unquoted scalar.
 * @param raw - value text.
 * @returns the value without the comment.
 */
function stripTrailingComment(raw: string): string {
  if (raw.startsWith('"') || raw.startsWith("'")) return raw
  const at = raw.indexOf(' #')
  return at < 0 ? raw : raw.slice(0, at)
}

/**
 * Split a comma-separated inline list, ignoring commas inside quotes.
 * @param text - inline list body.
 * @returns the element texts.
 */
function splitTopLevel(text: string): string[] {
  const out: string[] = []
  let current = ''
  let quote: string | null = null
  for (const ch of text) {
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
    if (ch === ',') {
      out.push(current.trim())
      current = ''
      continue
    }
    current += ch
  }
  if (current.trim() !== '') out.push(current.trim())
  return out
}
