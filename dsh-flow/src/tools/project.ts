/**
 * What a folder's own files say about how it is built and tested.
 *
 * `run_tests` and `diagnostics` ask the same questions of the same folder —
 * "is there a solution file?", "does this project configure eslint?" — so the
 * answers live here instead of being written twice and drifting apart.
 */

import { existsSync, readdirSync, readFileSync } from 'node:fs'
import path from 'node:path'

/** The build file naming a .NET project: a solution first, then a project. */
export function dotnetProject(cwd: string): string | null {
  try {
    const files = readdirSync(cwd, { withFileTypes: true })
      .filter((entry) => entry.isFile())
      .map((entry) => entry.name)
      .sort()
    return (
      files.find((name) => name.toLowerCase().endsWith('.sln')) ??
      files.find((name) => name.toLowerCase().endsWith('.csproj')) ??
      null
    )
  } catch {
    return null
  }
}

/** True when the folder holds a Maven build. */
export function hasMaven(cwd: string): boolean {
  return existsSync(path.join(cwd, 'pom.xml'))
}

/** True when the folder holds a Gradle build. */
export function hasGradle(cwd: string): boolean {
  return (
    existsSync(path.join(cwd, 'build.gradle')) || existsSync(path.join(cwd, 'build.gradle.kts'))
  )
}

/**
 * The Gradle launcher this folder should use: its own wrapper when it has one.
 *
 * The wrapper pins the version the project was written for, and it is the form
 * every guide tells people to run. `.bat` needs the shell on Windows.
 * @param cwd - folder to inspect.
 * @returns the program to start.
 */
export function gradleCommand(cwd: string): string {
  const wrapper = process.platform === 'win32' ? 'gradlew.bat' : 'gradlew'
  const local = path.join(cwd, wrapper)
  return existsSync(local) ? local : 'gradle'
}

/** The eslint configuration files a project may use. */
const ESLINT_CONFIGS = [
  'eslint.config.js',
  'eslint.config.mjs',
  'eslint.config.cjs',
  'eslint.config.ts',
  'eslint.config.mts',
  '.eslintrc',
  '.eslintrc.js',
  '.eslintrc.cjs',
  '.eslintrc.json',
  '.eslintrc.yml',
  '.eslintrc.yaml'
]

/**
 * True when the folder configures eslint.
 *
 * A config file is the usual answer; a `eslint` entry in `package.json` alone
 * also counts, because that is what a project scaffolded from a template has
 * before anyone writes the file.
 * @param cwd - folder to inspect.
 * @returns whether eslint would find rules to apply here.
 */
export function hasEslint(cwd: string): boolean {
  if (ESLINT_CONFIGS.some((name) => existsSync(path.join(cwd, name)))) return true
  const file = path.join(cwd, 'package.json')
  if (!existsSync(file)) return false
  try {
    const parsed = JSON.parse(readFileSync(file, 'utf8')) as {
      devDependencies?: Record<string, unknown>
      dependencies?: Record<string, unknown>
    }
    return (
      'eslint' in (parsed.devDependencies ?? {}) ||
      'eslint' in (parsed.dependencies ?? {}) ||
      '@eslint/js' in (parsed.devDependencies ?? {})
    )
  } catch {
    return false
  }
}

/**
 * The eslint command this folder should use: its own install when it has one.
 *
 * A project-local eslint is the version its config was written for; the one on
 * PATH may be older and reject a flat config. Both `.cmd` shims need the shell.
 * @param cwd - folder to inspect.
 * @returns the program to start.
 */
export function eslintCommand(cwd: string): string {
  const local = process.platform === 'win32'
    ? path.join(cwd, 'node_modules', '.bin', 'eslint.cmd')
    : path.join(cwd, 'node_modules', '.bin', 'eslint')
  return existsSync(local) ? local : 'npx'
}