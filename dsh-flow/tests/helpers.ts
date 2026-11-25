/**
 * Shared test helpers: build throwaway workflow directories.
 */

import { mkdirSync, mkdtempSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'

/** One captured chat-completions request, for the fetch stub. */
export interface CapturedRequest {
  url: string
  body: {
    model: string
    messages: Array<{
      role: string
      content?: string | Array<Record<string, unknown>> | null
      tool_calls?: unknown[]
      tool_call_id?: string
    }>
    tools?: unknown[]
    temperature?: number
    max_tokens?: number
    /** Thinking effort the engine sent, when the window chose one. */
    reasoning_effort?: string
    /** The thinking switch, when the window turned thinking off. */
    thinking?: { type: string }
  }
}

/**
 * Build a fetch stub that answers with the given bodies in order.
 * @param bodies - response bodies, one per round-trip.
 * @returns the stub plus the calls it recorded.
 */
export function stubFetch(
  bodies: unknown[]
): { fetchImpl: typeof fetch; calls: CapturedRequest[] } {
  const calls: CapturedRequest[] = []
  let index = 0
  const fetchImpl = (async (url: string, init: RequestInit) => {
    calls.push({ url, body: JSON.parse(String(init.body)) as CapturedRequest['body'] })
    const body = bodies[Math.min(index, bodies.length - 1)]
    index += 1
    return new Response(JSON.stringify(body), {
      status: 200,
      headers: { 'content-type': 'application/json' }
    })
  }) as unknown as typeof fetch
  return { fetchImpl, calls }
}

/**
 * Write a set of files into a fresh temporary directory.
 * @param files - relative path to file content.
 * @returns the absolute directory path.
 */
export function makeProject(files: Record<string, string>): string {
  const dir = mkdtempSync(join(tmpdir(), 'dsh-flow-test-'))
  for (const [name, content] of Object.entries(files)) {
    const path = join(dir, name)
    mkdirSync(dirname(path), { recursive: true })
    writeFileSync(path, content, 'utf8')
  }
  return dir
}

/**
 * A Node one-liner that echoes a JSON reply, quoted for YAML flow sequences.
 * @param body - JavaScript producing the reply object.
 * @returns the command fragment.
 */
export function nodeEcho(body: string): string {
  return `[node, -e, 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>{const c=JSON.parse(s);process.stdout.write(JSON.stringify(${body}))})']`
}
