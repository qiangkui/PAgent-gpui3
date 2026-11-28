/**
 * `env`: what this machine and this project actually provide.
 *
 * The assistant otherwise learns its environment by trial and error — one round
 * for `python --version`, another for `npm`, another for whether the folder is
 * a Rust or a Node project. This answers all of it at once, and it only reads:
 * every probe is a fixed `--version` command and a folder listing.
 */

import { existsSync, promises as fs } from 'node:fs'
import { spawnSync } from 'node:child_process'
import path from 'node:path'
import type { ToolResult, ToolScope } from './types.ts'
import { findExecutable, resolveRuntime } from '../runtime.ts'
import { resolveScoped } from './paths.ts'

/** How long one `--version` probe may take. */
const PROBE_TIMEOUT_MS = 3_000

/** Files and folders that say what kind of project this is, in report order. */
const MARKERS: Array<{ name: string; kind: 'file' | 'dir'; note: string }> = [
  { name: 'package.json', kind: 'file', note: 'Node 项目' },
  { name: 'package-lock.json', kind: 'file', note: 'npm 锁文件' },
  { name: 'pnpm-lock.yaml', kind: 'file', note: 'pnpm 锁文件' },
  { name: 'yarn.lock', kind: 'file', note: 'yarn 锁文件' },
  { name: 'tsconfig.json', kind: 'file', note: 'TypeScript' },
  { name: 'node_modules', kind: 'dir', note: '依赖已安装' },
  { name: 'Cargo.toml', kind: 'file', note: 'Rust 项目' },
  { name: 'rust-toolchain.toml', kind: 'file', note: '锁定 Rust 版本' },
  { name: 'pyproject.toml', kind: 'file', note: 'Python 项目' },
  { name: 'requirements.txt', kind: 'file', note: 'Python 依赖清单' },
  { name: 'setup.py', kind: 'file', note: 'Python 打包脚本' },
  { name: '.venv', kind: 'dir', note: 'Python 虚拟环境' },
  { name: 'venv', kind: 'dir', note: 'Python 虚拟环境' },
  { name: 'go.mod', kind: 'file', note: 'Go 项目' },
  { name: 'pom.xml', kind: 'file', note: 'Maven 项目' },
  { name: 'build.gradle', kind: 'file', note: 'Gradle 项目' },
  { name: 'build.gradle.kts', kind: 'file', note: 'Gradle 项目' },
  { name: 'gradlew', kind: 'file', note: 'Gradle wrapper' },
  { name: 'eslint.config.js', kind: 'file', note: 'eslint 配置' },
  { name: 'Dockerfile', kind: 'file', note: '容器' },
  { name: '.github/workflows', kind: 'dir', note: 'CI 工作流' },
  { name: '.git', kind: 'dir', note: 'git 仓库' }
]

/** Longest list one section prints, so a wide repo cannot flood the context. */
const MAX_LIST = 20

/** What one probed executable answered. */
interface Probe {
  /** Logical name shown in the report. */
  name: string
  /** Version text, or an empty string when the probe failed. */
  version: string
  /** How the probe was started, when it is not simply the name. */
  command: string
}

/** Probe results, cached per process: versions do not change mid-question. */
const cache = new Map<string, Probe | null>()

/**
 * Report the environment of one folder.
 * @param workDir - relative paths resolve from here.
 * @param scopeRoot - writable root; `null` means the work directory is the root.
 * @param args - optional `path` of the folder to inspect, default the workspace.
 * @returns a plain-text report: platform, runtimes, project markers.
 */
export async function runEnv(
  workDir: string,
  scopeRoot: ToolScope,
  args: Record<string, unknown>
): Promise<ToolResult> {
  const requested = typeof args.path === 'string' && args.path.trim() ? args.path.trim() : '.'
  // `.` means the workspace, like `tree`: the report is about the project, not
  // about the (usually empty) folder this conversation writes into.
  const base =
    requested === '.' && scopeRoot !== null
      ? path.resolve(scopeRoot)
      : await resolveScoped(workDir, scopeRoot, requested)
  const info = await fs.stat(base).catch(() => null)
  if (!info?.isDirectory()) throw new Error(`找不到文件夹：${requested}`)

  const lines: string[] = [
    `平台：${process.platform} ${process.arch} · node ${process.version} · 目录：${base}`
  ]
  const probes = probesFor()
  const found = probes.filter((probe) => probe.version !== '')
  const missing = probes.filter((probe) => probe.version === '').map((probe) => probe.name)
  lines.push('运行时：')
  if (found.length === 0) {
    lines.push('- （一个都没探到）')
  } else {
    for (const probe of found) {
      const via = probe.command === probe.name ? '' : `（${probe.command}）`
      lines.push(`- ${probe.name} ${probe.version}${via}`)
    }
  }
  if (missing.length > 0) {
    lines.push(`未找到：${missing.join('、')}`)
  }
  lines.push(`项目标记（${requested === '.' ? '工作区' : requested}）：`)
  const markers: string[] = []
  for (const marker of MARKERS) {
    if (!existsSync(path.join(base, marker.name))) continue
    markers.push(`- ${marker.name}${marker.kind === 'dir' ? '/' : ''}：${marker.note}`)
  }
  lines.push(...(markers.length === 0 ? ['- （没有识别到常见项目文件）'] : markers))
  const scripts = await packageScripts(base)
  if (scripts.length > 0) {
    lines.push(`package.json 脚本：${scripts.slice(0, MAX_LIST).join('、')}`)
  }
  return { ok: true, text: lines.join('\n') }
}

/** The names package.json declares, so the assistant knows what it can run. */
async function packageScripts(base: string): Promise<string[]> {
  const file = path.join(base, 'package.json')
  if (!existsSync(file)) return []
  const text = await fs.readFile(file, 'utf8').catch(() => '')
  try {
    const parsed: unknown = JSON.parse(text)
    if (!parsed || typeof parsed !== 'object') return []
    const scripts = (parsed as { scripts?: unknown }).scripts
    if (!scripts || typeof scripts !== 'object') return []
    return Object.keys(scripts as Record<string, unknown>).filter((name) => name.trim() !== '')
  } catch {
    return []
  }
}

/** The executables worth probing, in report order. */
const PROBES: string[] = [
  'npm',
  'pnpm',
  'yarn',
  'python',
  'git',
  'cargo',
  'rustc',
  'go',
  'dotnet',
  'mvn',
  'rg',
  'bash',
  'pwsh'
]

/**
 * Probe every known runtime, in order.
 * @returns one entry per runtime; a missing one has an empty `version`.
 */
function probesFor(): Probe[] {
  return PROBES.map((name) => {
    const known = cache.get(name)
    if (known !== undefined) return known ?? { name, version: '', command: name }
    const probe = probeRuntime(name)
    cache.set(name, probe)
    return probe ?? { name, version: '', command: name }
  })
}

/**
 * Ask one executable for its version.
 *
 * `resolveRuntime` is used for `python` so a Windows `py -3` alias counts, the
 * same resolution a workflow step gets.
 * @param name - the executable's logical name.
 * @returns the probe, or null when it is not installed.
 */
function probeRuntime(name: string): Probe | null {
  const resolved =
    name === 'python'
      ? resolveRuntime('python')
      : findExecutable(name)
        ? { file: name, prefixArgs: [] as string[] }
        : null
  if (!resolved) return null
  const result = spawnSync(resolved.file, [...resolved.prefixArgs, '--version'], {
    encoding: 'utf8',
    timeout: PROBE_TIMEOUT_MS,
    windowsHide: true,
    // A `.cmd` shim (npm, yarn) only starts through the shell on Windows. Every
    // argument here is a fixed `--version`, so nothing is interpolated.
    shell: process.platform === 'win32'
  })
  const text = `${result.stdout ?? ''}\n${result.stderr ?? ''}`.trim()
  const first = text.split('\n').map((line) => line.trim()).find((line) => line !== '') ?? ''
  if (first === '') return null
  return {
    name,
    version: versionOf(first).slice(0, 80),
    command: [resolved.file, ...resolved.prefixArgs].join(' ')
  }
}

/**
 * The version number inside a `--version` line.
 * @param line - for example `git version 2.45.1.windows.1`.
 * @returns the numeric version, or the line itself when it has none.
 */
function versionOf(line: string): string {
  const match = /\d+(?:\.\d+)+(?:[-+][0-9A-Za-z.]+)?/.exec(line)
  return match ? match[0] : line
}