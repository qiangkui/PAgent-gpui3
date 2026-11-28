/**
 * Runs one background command, keeps its output in a log file, and forwards
 * whatever is appended to its control file into the command's stdin.
 *
 * Started by {@link startBackground} as
 * `node task-run.ts <log> <cwd> <command> <stdin-file> <close-marker>`.
 * The wrapping is what makes the log readable: this process stays alive for the
 * whole run, so it can decode the command's output (which may be in the OEM code
 * page) and append UTF-8 to the log — a detached child could not be rewritten
 * after the fact. The control file is the same trick for input: `task input`
 * appends to it, and this wrapper passes the bytes on, so a dev server that asks
 * a question can be answered after the answer that started it has already ended.
 */

import { spawn } from 'node:child_process'
import { closeSync, existsSync, openSync, readSync, statSync, writeSync } from 'node:fs'
import { decodeOutput } from './text.ts'

const [log, cwd, command, control, closeMarker] = process.argv.slice(2)
if (!log || !cwd || command === undefined) {
  process.exit(2)
}

/** How often appended input is looked for. */
const PUMP_MS = 120

const fd = openSync(log, 'a')
const controlFd = control === undefined ? -1 : openControl(control)
let consumed = 0
let stdinLost = false
let stdinClosed = false
let done = false

/** Bytes of each stream that have not formed a whole line yet. */
const pendingOut: Buffer[] = []
const pendingErr: Buffer[] = []

/**
 * Longest unterminated run kept in memory before it is written as it is.
 *
 * A program that redraws a progress bar with `\r` may never print a break; holding
 * that run until the command ends would grow the wrapper's memory with the whole
 * download, so a long one is flushed unbuffered.
 */
const MAX_PENDING = 64 * 1024

/**
 * Append text to the log, refusing to hold the wrapper alive on a full disk.
 * @param text - the text to append.
 */
function writeLog(text: string): void {
  if (text === '') return
  try {
    writeSync(fd, text)
  } catch {
    // The log is unwritable; nothing else to do about it.
  }
}

/**
 * Decode the complete lines in `chunk` and append them to the log.
 *
 * The log is always UTF-8, whatever the command printed: the window reads it as
 * text, and a GBK line would show there as replacement characters. `decodeOutput`
 * decides per line, so a mixed stream (a `cmd` builtin in the OEM code page beside
 * a UTF-8 script) comes out right. A character or a line may be split across two
 * chunks, which is what the pending buffer is for.
 * @param pending - bytes of this stream that are not a whole line yet.
 * @param chunk - what just arrived.
 */
function appendDecoded(pending: Buffer[], chunk: Buffer): void {
  const joined = pending.length === 0 ? chunk : Buffer.concat([...pending, chunk])
  let start = 0
  let text = ''
  for (let index = 0; index < joined.length; index += 1) {
    if (joined[index] !== 0x0a) continue
    text += `${decodeOutput(joined.subarray(start, index))}\n`
    start = index + 1
  }
  pending.length = 0
  if (start < joined.length) {
    const rest = joined.subarray(start)
    // A run this long is a progress stream, not a line: write it rather than let
    // it grow.
    if (rest.length > MAX_PENDING) writeLog(decodeOutput(rest))
    else pending.push(rest)
  }
  writeLog(text)
}

/**
 * Append a stream's last line, which ended without a break.
 * @param pending - the bytes still waiting.
 */
function flushPending(pending: Buffer[]): void {
  if (pending.length === 0) return
  const rest = Buffer.concat(pending)
  pending.length = 0
  if (rest.length > 0) writeLog(decodeOutput(rest))
}

/**
 * Open the control file, or report that input cannot be forwarded.
 * @param file - the control file's path.
 * @returns the read handle, or -1 when it cannot be opened.
 */
function openControl(file: string): number {
  try {
    return openSync(file, 'r')
  } catch {
    return -1
  }
}

/**
 * Note how the command ended, close the log and leave.
 * @param note - one line such as `[退出码 0]`.
 */
function finish(note: string): void {
  if (done) return
  done = true
  if (controlFd >= 0) closeSync(controlFd)
  try {
    writeSync(fd, note)
  } catch {
    // A full disk must not keep the wrapper alive.
  }
  closeSync(fd)
  process.exit(0)
}

const child = spawn(command, {
  cwd,
  shell: true,
  windowsHide: true,
  // Both output streams are pipes so their bytes can be decoded before they reach
  // the log; stdin stays a pipe for `task input`.
  stdio: ['pipe', 'pipe', 'pipe']
})
child.stdout?.on('data', (chunk: Buffer) => appendDecoded(pendingOut, chunk))
child.stderr?.on('data', (chunk: Buffer) => appendDecoded(pendingErr, chunk))
child.stdin?.on('error', () => {
  if (stdinLost) return
  stdinLost = true
  writeLog('\n[命令已经关闭 stdin，之后的输入没有送达]\n')
})
child.on('error', (error) => {
  flushPending(pendingOut)
  flushPending(pendingErr)
  finish(`\n[无法启动：${error.message}]\n`)
})
child.on('close', (code, signal) => {
  // The last line of a stream may have no break; it is still output.
  flushPending(pendingOut)
  flushPending(pendingErr)
  const note = signal ? `\n[被信号 ${signal} 结束]\n` : `\n[退出码 ${code ?? '?'}]\n`
  finish(note)
})

/**
 * Copy whatever was appended to the control file into the command's stdin.
 *
 * Reading from a byte position means an input line is delivered once: the file
 * is the queue, so a write that lands between two polls is not lost.
 */
function pump(): void {
  // The window can say "no more input": it drops a marker file, and the command
  // sees the end of stdin instead of waiting for it forever.
  if (!stdinClosed && closeMarker !== undefined && closeMarker !== '') {
    if (existsSync(closeMarker)) {
      stdinClosed = true
      try {
        child.stdin?.end()
      } catch {
        // The command already closed stdin; nothing left to end.
      }
    }
  }
  if (stdinClosed) return
  if (controlFd < 0 || stdinLost || child.stdin === null || child.stdin.destroyed) return
  let size = 0
  try {
    size = statSync(control ?? '').size
  } catch {
    return
  }
  if (size <= consumed) return
  const buffer = Buffer.alloc(size - consumed)
  let read = 0
  while (read < buffer.length) {
    let got = 0
    try {
      got = readSync(controlFd, buffer, read, buffer.length - read, consumed + read)
    } catch {
      return
    }
    if (got <= 0) break
    read += got
  }
  consumed += read
  if (read === 0) return
  try {
    child.stdin.write(buffer.subarray(0, read))
  } catch {
    stdinLost = true
  }
}

const timer = setInterval(pump, PUMP_MS)
child.on('close', () => clearInterval(timer))
