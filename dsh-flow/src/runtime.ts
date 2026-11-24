/**
 * Runtime resolution for manifest commands.
 *
 * A manifest names a runtime logically (`python`) and the runner resolves it to
 * an executable that exists on this machine (`python`, `python3`, or
 * `py -3`). Resolution happens before a run so a missing interpreter is a
 * validation error, not a mid-run surprise.
 */

import { existsSync, statSync } from 'node:fs'
import { delimiter, isAbsolute, join } from 'node:path'

/** A resolved executable plus leading arguments baked into the alias. */
export interface ResolvedRuntime {
  /** Executable path or bare name handed to `spawn`. */
  file: string
  /** Arguments the alias contributes before the step's own arguments. */
  prefixArgs: string[]
}

/** Logical runtime names and the executables that satisfy them, in preference order. */
const ALIASES: Record<string, Array<[string, string[]]>> = {
  python: [['python', []], ['python3', []], ['py', ['-3']]],
  node: [['node', []]],
  npm: [['npm', []]],
  git: [['git', []]],
  rg: [['rg', []]],
  bash: [['bash', []]],
  sh: [['sh', []]],
  pwsh: [['pwsh', []]],
  powershell: [['powershell', []]],
  cmd: [['cmd', []]],
}

/** Resolution results, cached per process. */
const cache = new Map<string, ResolvedRuntime | null>()

/**
 * Resolve a logical runtime name to an executable on this machine.
 * @param name - the manifest's runtime name.
 * @returns the resolution, or null when no candidate exists.
 */
export function resolveRuntime(name: string): ResolvedRuntime | null {
  const cached = cache.get(name)
  if (cached !== undefined) return cached
  const candidates = ALIASES[name]
  const resolved = candidates
    ? candidates.map(([file, prefixArgs]) => (findExecutable(file) ? { file, prefixArgs } : null)).find(entry => entry !== null) ?? null
    : findExecutable(name) ? { file: name, prefixArgs: [] } : null
  cache.set(name, resolved)
  return resolved
}

/**
 * Describe where a missing runtime was looked for.
 * @param name - the logical runtime name.
 * @returns a hint listing the accepted executable names.
 */
export function describeRuntime(name: string): string {
  const candidates = ALIASES[name]
  if (!candidates) return `"${name}" was not found on PATH`
  return `"${name}" was not found (accepted: ${candidates.map(([file, args]) => [file, ...args].join(' ')).join(', ')})`
}

/**
 * The interpreter and PATH entries a project folder itself provides.
 *
 * A `.venv` or `node_modules/.bin` next to the code wins over PATH: that is the
 * environment the person who wrote the manifest expects the snippet to run in.
 */
export interface ProjectRuntime {
  /** Interpreter to start, or null when this machine has none. */
  resolved: ResolvedRuntime | null
  /** Directories to put in front of PATH for this one command. */
  pathPrepend: string[]
  /** True when `resolved` is the project's own `.venv` interpreter. */
  local: boolean
}

/** Virtual-environment folders checked inside the run directory, in order. */
const VENV_DIRS = ['.venv', 'venv']

/**
 * Resolve `python` / `node` against the run directory before falling back to PATH.
 * @param language - the tool being run.
 * @param cwd - the resolved run directory.
 * @returns the interpreter plus the project folders to put on PATH.
 */
export function resolveProjectRuntime(
  language: 'python' | 'node',
  cwd: string
): ProjectRuntime {
  const pathPrepend: string[] = []
  const nodeBin = join(cwd, 'node_modules', '.bin')
  if (existsSync(nodeBin)) pathPrepend.push(nodeBin)
  let local = false
  if (language === 'python') {
    const binDir = process.platform === 'win32' ? 'Scripts' : 'bin'
    const exeName = process.platform === 'win32' ? 'python.exe' : 'python'
    for (const venv of VENV_DIRS) {
      const envDir = join(cwd, venv, binDir)
      if (!existsSync(envDir)) continue
      pathPrepend.push(envDir)
      const exe = join(envDir, exeName)
      if (!existsSync(exe) || !isFile(exe)) continue
      local = true
      return { resolved: { file: exe, prefixArgs: [] }, pathPrepend, local }
    }
  }
  return { resolved: resolveRuntime(language), pathPrepend, local }
}

/**
 * Locate an executable on PATH, honouring PATHEXT on Windows.
 * @param name - executable name or path.
 * @returns true when the file exists.
 */
export function findExecutable(name: string): boolean {
  if (name.includes('/') || name.includes('\\') || isAbsolute(name)) return existsSync(name) && isFile(name)
  const pathValue = process.env.PATH ?? ''
  const extensions = process.platform === 'win32'
    ? (process.env.PATHEXT ?? '.EXE;.CMD;.BAT;.COM').split(';').filter(entry => entry !== '')
    : ['']
  for (const directory of pathValue.split(delimiter)) {
    if (directory === '') continue
    for (const extension of extensions) {
      const candidate = join(directory, `${name}${extension}`)
      if (existsSync(candidate) && isFile(candidate)) return true
    }
    if (process.platform === 'win32') continue
    const bare = join(directory, name)
    if (existsSync(bare) && isFile(bare)) return true
  }
  return false
}

/**
 * Report whether a path is a regular file.
 * @param path - candidate path.
 * @returns true for regular files.
 */
function isFile(path: string): boolean {
  try {
    return statSync(path).isFile()
  } catch (error) {
    if (error instanceof Error) return false
    throw error
  }
}
