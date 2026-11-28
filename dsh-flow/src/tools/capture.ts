/**
 * Run one command and keep a bounded amount of its output.
 *
 * Shared by the tools that read a program's report rather than execute it:
 * `run_tests` and `diagnostics`. Both keep the head and the tail, because a
 * failure is usually near the end while the build's own summary is at the start,
 * and a runaway log must not sit in memory until the tool returns.
 *
 * The two streams are captured apart. Merging them into one buffer looks tidy
 * until a program writes progress to stderr and its report to stdout — cargo
 * does exactly that — and a chunk of progress lands in the middle of a JSON
 * line, corrupting the very message the tool came for.
 */

import { killTree } from './process.ts'
import { resolveRuntime } from '../runtime.ts'
import { decodeOutput, stripAnsi } from './text.ts'
import { spawnTool } from './spawn.ts'

/** How much of each stream is kept before the tail is lost. */
const MAX_CAPTURE = 400_000

/** How many characters of each half survive that cap. */
const MAX_HALF = MAX_CAPTURE / 2

/** One command to start, as a plan built it. */
export interface CapturePlan {
  /** Executable to start. */
  file: string
  /** Arguments after it. */
  args: string[]
  /** The whole command line, used as the label when `shell` is set. */
  label: string
  /** True when the command needs the platform shell (`npm`, `tsc.cmd`). */
  shell: boolean
}

/** What one command produced. */
export interface CaptureOutcome {
  /** Exit code, or -1 when it never started. */
  code: number
  /** stdout, head then tail; the middle may be omitted. */
  stdout: string
  /** stderr, head then tail; the middle may be omitted. */
  stderr: string
  /** stdout followed by stderr, never interleaved — read this when order matters. */
  text: string
  /** True when the command was killed at its timeout. */
  timedOut: boolean
}

/** How often live output is forwarded, so a chatty build cannot flood the UI. */
const LIVE_OUTPUT_INTERVAL_MS = 200

/** One stream's kept output, plus the bytes of a line that is still arriving. */
interface StreamState {
  /** Text kept from the start, up to {@link MAX_HALF}. */
  head: string
  /** Text kept from the end, up to {@link MAX_HALF}. */
  tail: string
  /** Bytes since the last newline: a character may be split across chunks. */
  pending: Buffer
}

/**
 * Start one plan and collect its output.
 *
 * The child is started through `spawnTool`, so a project's build or test command
 * runs inside the same Windows sandbox as `shell` does. Output is decoded line by
 * line (`decodeOutput`), because a Chinese Windows tool answers in the OEM code
 * page while another writes UTF-8 into the same pipe.
 * @param plan - what to run.
 * @param cwd - the directory to run it in.
 * @param timeoutMs - when to give up and kill the tree.
 * @param onOutput - receives the newest output line while the command runs.
 * @returns the exit code, both streams bounded, and a non-interleaved merge.
 */
export function captureCommand(
  plan: CapturePlan,
  cwd: string,
  timeoutMs: number,
  onOutput?: (text: string) => void
): Promise<CaptureOutcome> {
  return new Promise((resolve) => {
    const runtime = plan.shell ? null : resolveRuntime(plan.file)
    if (!plan.shell && runtime === null) {
      const message = `找不到 ${plan.file}：PATH 上没有可用的可执行文件`
      resolve({ code: -1, stdout: '', stderr: message, text: message, timedOut: false })
      return
    }
    const spec = plan.shell
      ? { file: plan.label, args: [], shell: true }
      : { file: runtime!.file, args: [...runtime!.prefixArgs, ...plan.args], shell: false }
    const child = spawnTool(spec, cwd, { capture: true })
    const out: StreamState = { head: '', tail: '', pending: Buffer.alloc(0) }
    const err: StreamState = { head: '', tail: '', pending: Buffer.alloc(0) }
    let timedOut = false
    // Only the newest line of each tick is forwarded: the live row shows one
    // line, and a compiler can print thousands per second.
    let liveAt = 0
    // The newest line that arrived inside the throttle window: it still has to
    // be forwarded once the command ends, or the last thing it printed is lost.
    let liveLatest = ''
    const emit = (line: string): void => {
      const text = stripAnsi(line).trim()
      if (text === '' || onOutput === undefined) return
      liveLatest = text
      if (Date.now() - liveAt < LIVE_OUTPUT_INTERVAL_MS) return
      liveAt = Date.now()
      liveLatest = ''
      try {
        onOutput(text)
      } catch {
        // The window may have gone away; the command still has to finish.
      }
    }
    const flushLive = (): void => {
      if (liveLatest === '' || onOutput === undefined) return
      const text = liveLatest
      liveLatest = ''
      try {
        onOutput(text)
      } catch {
        // Same as above.
      }
    }
    const append = (into: StreamState, chunk: Buffer | string): void => {
      const bytes = Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk)
      const joined = into.pending.length === 0 ? bytes : Buffer.concat([into.pending, bytes])
      let start = 0
      for (let index = 0; index < joined.length; index += 1) {
        if (joined[index] !== 0x0a) continue
        // `decodeOutput` rebuilds the breaks *between* lines, not the trailing
        // one, so the newline is put back here: `addText` keeps whole lines.
        const line = `${decodeOutput(joined.subarray(start, index))}\n`
        start = index + 1
        addText(into, line)
        emit(line)
      }
      into.pending = joined.subarray(start)
    }
    /** Decode the tail of a stream that never ended with a newline. */
    const flush = (into: StreamState): void => {
      if (into.pending.length === 0) return
      const line = decodeOutput(into.pending)
      into.pending = Buffer.alloc(0)
      addText(into, line)
      emit(line)
    }
    const timer = setTimeout(() => {
      timedOut = true
      killTree(child)
    }, timeoutMs)
    child.stdout?.on('data', (chunk: Buffer) => append(out, chunk))
    child.stderr?.on('data', (chunk: Buffer) => append(err, chunk))
    const settle = (code: number): void => {
      clearTimeout(timer)
      flush(out)
      flush(err)
      flushLive()
      const stdout = join(out)
      const stderr = join(err)
      resolve({ code, stdout, stderr, text: merge(stdout, stderr), timedOut })
    }
    child.on('error', (error) => {
      append(err, error.message)
      settle(-1)
    })
    child.on('close', (code) => settle(code ?? -1))
  })
}

/** Keep one decoded line: fill the head, then keep the newest tail. */
function addText(state: StreamState, text: string): void {
  if (state.head.length < MAX_HALF) {
    const room = MAX_HALF - state.head.length
    if (text.length <= room) {
      state.head += text
      return
    }
    state.head += text.slice(0, room)
    state.tail = text.slice(room)
    return
  }
  state.tail = (state.tail + text).slice(-MAX_HALF)
}

/** Head then tail, with a marker where the middle was dropped. */
function join(stream: StreamState): string {
  return stream.tail === '' ? stream.head : `${stream.head}\n…（中间省略）…\n${stream.tail}`
}

/** stdout first, then stderr: the order a reader expects, with no interleaving. */
function merge(stdout: string, stderr: string): string {
  if (stderr.trim() === '') return stdout
  if (stdout.trim() === '') return stderr
  return `${stdout.trimEnd()}\n${stderr}`
}