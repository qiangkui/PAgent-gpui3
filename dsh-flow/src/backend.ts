/**
 * Where an `ai` step gets its answer.
 *
 * The step itself only knows "ask one question, get one answer". Three backends
 * implement that: `http` (an OpenAI-compatible endpoint), `cli` (a program of
 * the workflow's own choosing) and `mock` (scripted, used by contract tests so
 * they never touch the network).
 */

import { basename, resolve as resolvePath } from 'node:path'
import { askModel } from './ai.ts'
import type { WireAttachment } from './ai.ts'
import { runCommand, stepEnvironment } from './exec.ts'
import { resolveRuntime } from './runtime.ts'

/** One image attached to a question. */
export interface AiImage {
  /** Absolute path the step read it from; the `cli` backend gets this. */
  path: string
  /** MIME type, for example `image/png`. */
  mime: string
  /** Base64 bytes without the `data:` prefix; the `http` backend sends these. */
  data: string
}

/** One question to a backend. */
export interface AiAsk {
  /** The user message, already rendered from the manifest template. */
  prompt: string
  /** Optional instruction prepended as a system message. */
  system?: string
  /** Model id; the backend's own default when absent. */
  model?: string
  /** Sampling temperature. */
  temperature?: number
  /** Upper bound on generated tokens. */
  maxTokens?: number
  /** The expected shape; only the `cli` backend receives it, as a hint. */
  schema?: unknown
  /** Pictures the model should look at, already read from disk. */
  images?: AiImage[]
}

/** One answer from a backend. */
export interface AiReply {
  /** The answer text, exactly as the step records it. */
  text: string
  /** Model (or backend) that produced it. */
  model: string
  /** Token accounting, when the backend reports it. */
  usage?: unknown
}

/** A source of answers. */
export interface AiBackend {
  /** Short name shown in `explain` and the run report. */
  readonly name: string
  /**
   * Ask one question.
   * @param ask - the rendered question.
   * @returns the answer.
   */
  ask(ask: AiAsk): Promise<AiReply>
}

/** How to reach the HTTP backend. */
export interface HttpBackendOptions {
  /** Stub for tests. */
  fetchImpl?: typeof fetch
  /** Environment holding `DEEPSEEK_*` fallbacks. */
  env?: Record<string, string | undefined>
  /** Extra attempts after a failure that looks retryable. Defaults to 2. */
  retries?: number
  /** Per-request timeout. Defaults to 120000. */
  timeoutMs?: number
}

/** Default attempts after the first, for flaky endpoints. */
const DEFAULT_RETRIES = 2

/** Default per-request timeout. */
const DEFAULT_TIMEOUT_MS = 120_000

/**
 * Create the HTTP backend: the normal path for a real model call.
 *
 * Retries only what looks transient (`5xx`, connection resets). A missing key,
 * a rejected key or an unknown model is reported immediately, because asking
 * again spends the user's time without changing the answer.
 * @param options - fetch stub, environment, retries and timeout.
 * @returns the backend.
 */
export function createHttpBackend(options: HttpBackendOptions = {}): AiBackend {
  const retries = Math.max(0, options.retries ?? DEFAULT_RETRIES)
  const timeoutMs = options.timeoutMs ?? DEFAULT_TIMEOUT_MS
  return {
    name: 'http',
    async ask(ask: AiAsk): Promise<AiReply> {
      let lastError: unknown
      for (let attempt = 0; attempt <= retries; attempt += 1) {
        try {
          const result = await askModel({
            prompt: ask.prompt,
            system: ask.system,
            model: ask.model,
            temperature: ask.temperature,
            maxTokens: ask.maxTokens,
            timeoutMs,
            attachments: attachmentsOf(ask.images),
          }, { fetchImpl: options.fetchImpl, env: options.env })
          return { text: result.text, model: result.model, usage: result.usage }
        } catch (error) {
          lastError = error
          if (attempt === retries || !isRetryable(error)) break
          await sleep(400 * (attempt + 1))
        }
      }
      throw lastError instanceof Error ? lastError : new Error(String(lastError))
    },
  }
}

/**
 * Turn read images into the wire attachments `askModel` expects.
 * @param images - pictures attached to the question.
 * @returns the attachments, or undefined when there are none.
 */
function attachmentsOf(images: AiImage[] | undefined): WireAttachment[] | undefined {
  if (images === undefined || images.length === 0) return undefined
  return images.map((image) => ({
    // Anything with an image MIME type travels as an `image_url` part.
    kind: 'image' as const,
    name: basename(image.path),
    mimeType: image.mime,
    data: image.data,
  }))
}

/**
 * Report whether a failed request is worth repeating.
 * @param error - the thrown value.
 * @returns true for timeouts, resets and server errors.
 */
function isRetryable(error: unknown): boolean {
  const message = error instanceof Error ? error.message : String(error)
  if (/API Key|缺少|401|403|404|模型不存在|不存在/.test(message)) return false
  return /5\d\d|timeout|timed out|ECONNRESET|ETIMEDOUT|EAI_AGAIN|fetch failed|network/i.test(message)
}

/**
 * Create a backend that runs an external program.
 *
 * The program receives one JSON document on stdin
 * (`{prompt, system, model, temperature, maxTokens, schema}`) and answers on
 * stdout with `{"text": "..."}`, `{"json": {...}}` or plain text. This is how a
 * workflow can use a local model, a stub server, or a tool it already owns
 * without teaching the runner another protocol.
 * @param command - program plus arguments, or a shell command line.
 * @param options - working directory, extra environment and timeout.
 * @returns the backend.
 */
export function createCliBackend(
  command: string | string[],
  options: { cwd?: string, env?: Record<string, string>, timeoutMs?: number } = {},
): AiBackend {
  const parts = Array.isArray(command) ? command : [command]
  if (parts.length === 0 || parts[0]!.trim() === '') throw new Error('ai backend "cli" 需要 "command"')
  const line = parts.join(' ')
  return {
    name: 'cli',
    async ask(ask: AiAsk): Promise<AiReply> {
      const resolved = resolveRuntime(parts[0]!)
      if (resolved === null) throw new Error(`ai 后端命令 "${parts[0]}" 不在 PATH 上`)
      const outcome = await runCommand({
        file: resolved.file,
        args: [...resolved.prefixArgs, ...parts.slice(1)],
        cwd: options.cwd === undefined ? process.cwd() : resolvePath(options.cwd),
        env: stepEnvironment(options.env ?? {}),
        timeoutMs: options.timeoutMs ?? DEFAULT_TIMEOUT_MS,
        stdin: `${JSON.stringify({
          prompt: ask.prompt,
          system: ask.system ?? null,
          model: ask.model ?? null,
          temperature: ask.temperature ?? null,
          maxTokens: ask.maxTokens ?? null,
          schema: ask.schema ?? null,
          // Paths, not bytes: the program can open the pictures itself, and a
          // multi-megabyte base64 string would not survive every pipe.
          images: (ask.images ?? []).map((image) => ({ path: image.path, mime: image.mime })),
        }, null, 2)}\n`,
      })
      if (outcome.spawnError !== undefined) throw new Error(`ai 后端命令 "${line}" 启动失败：${outcome.spawnError}`)
      if (outcome.exitCode !== 0) {
        const detail = firstLine(outcome.stderr) || firstLine(outcome.stdout)
        throw new Error(`ai 后端命令 "${line}" 退出码 ${outcome.exitCode}${detail === '' ? '' : `：${detail}`}`)
      }
      const text = readReply(outcome.stdout)
      if (text.trim() === '') throw new Error(`ai 后端命令 "${line}" 没有输出`)
      return { text, model: ask.model ?? 'cli' }
    },
  }
}

/**
 * Read an answer out of a CLI backend's stdout.
 * @param stdout - captured output.
 * @returns text to record.
 */
function readReply(stdout: string): string {
  const trimmed = stdout.trim()
  if (trimmed === '') return ''
  try {
    const parsed: unknown = JSON.parse(trimmed)
    if (parsed !== null && typeof parsed === 'object' && !Array.isArray(parsed)) {
      const record = parsed as Record<string, unknown>
      if (typeof record.text === 'string') return record.text
      if ('json' in record) return JSON.stringify(record.json)
    }
    return trimmed
  } catch (error) {
    if (error instanceof SyntaxError) return trimmed
    throw error
  }
}

/** A scripted backend, for tests. */
export interface MockBackend extends AiBackend {
  /** Every question it received, in order. */
  readonly calls: AiAsk[]
}

/**
 * Create a backend that answers from a script.
 *
 * Used by contract tests: the expected reply lives in
 * `fixtures/<step-id>.ai.json`, so a test is deterministic and free. A script
 * shorter than the number of calls repeats its last entry.
 * @param script - replies, or a function computing one per question.
 * @returns the backend, with the received questions for assertions.
 */
export function createMockBackend(script: Array<string | AiReply> | ((ask: AiAsk) => string | AiReply)): MockBackend {
  const calls: AiAsk[] = []
  const entries = typeof script === 'function' ? null : script
  if (entries !== null && entries.length === 0) throw new Error('mock backend 需要至少一条回复')
  return {
    name: 'mock',
    calls,
    async ask(ask: AiAsk): Promise<AiReply> {
      calls.push(ask)
      const entry = entries === null
        ? (script as (ask: AiAsk) => string | AiReply)(ask)
        : entries[Math.min(calls.length - 1, entries.length - 1)]!
      if (typeof entry === 'string') return { text: entry, model: 'mock' }
      return { text: entry.text, model: entry.model, usage: entry.usage }
    },
  }
}

/**
 * First non-empty line of text.
 * @param text - the text.
 * @returns the line, trimmed.
 */
function firstLine(text: string): string {
  return text.split(/\r?\n/).map(line => line.trim()).find(line => line !== '') ?? ''
}

/**
 * Wait.
 * @param ms - milliseconds.
 * @returns a promise resolved after the delay.
 */
function sleep(ms: number): Promise<void> {
  return new Promise(resolve => {
    setTimeout(resolve, ms)
  })
}
