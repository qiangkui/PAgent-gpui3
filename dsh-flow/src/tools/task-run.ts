/**
 * Runs one background command and keeps its output in a log file.
 *
 * Started by {@link startBackground} as `node task-run.ts <log> <cwd> <command>`.
 * The wrapping is what makes the log survive: a detached process cannot inherit
 * a file handle on Windows, but this one is alive for the whole run, so it can
 * open the log and hand the handle to the command it starts.
 */

import { spawn } from 'node:child_process'
import { closeSync, openSync, writeSync } from 'node:fs'

const [log, cwd, command] = process.argv.slice(2)
if (!log || !cwd || command === undefined) {
  process.exit(2)
}

const fd = openSync(log, 'a')
let done = false
/**
 * Note how the command ended, close the log and leave.
 * @param note - one line such as `[退出码 0]`.
 */
function finish(note: string): void {
  if (done) return
  done = true
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
  stdio: ['ignore', fd, fd]
})
child.on('error', (error) => finish(`\n[无法启动：${error.message}]\n`))
child.on('close', (code, signal) => {
  const note = signal ? `\n[被信号 ${signal} 结束]\n` : `\n[退出码 ${code ?? '?'}]\n`
  finish(note)
})
