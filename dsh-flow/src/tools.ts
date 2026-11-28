/**
 * The tools the assistant may call inside one conversation.
 *
 * The implementations live under `tools/` by concern: file access, search,
 * execution and git. This module keeps the catalogue and the entry points, and
 * re-exports every helper callers used to import from `tools.ts`.
 */

import type { ToolCall, ToolResult, ToolScope } from './tools/types.ts'
import {
  copyFile, deleteFile, editFile, listDir, makeDir, moveFile, readFile, replaceFiles, treeDir,
  writeFile
} from './tools/files.ts'
import { globFiles, grepFiles } from './tools/search.ts'
import { indexSymbols } from './tools/symbols.ts'
import { runGit } from './tools/git.ts'
import { runHttp } from './tools/http.ts'
import { runEnv } from './tools/env.ts'
import { readImage, screenshot } from './tools/image.ts'
import { runGitWrite } from './tools/git-write.ts'
import { waitFor } from './tools/waitfor.ts'
import { runBrowser } from './tools/browser.ts'
import { showFiles } from './tools/show.ts'
import { runTests } from './tools/run-tests.ts'
import { runDiagnostics } from './tools/diagnostics.ts'
import { runCode, runShell } from './tools/exec.ts'
import { runTask } from './tools/tasks.ts'
import type { Sandbox } from './tools/sandbox.ts'

export type { ToolCall, ToolDefinition, ToolFile, ToolImage, ToolPreview, ToolResult, ToolScope } from './tools/types.ts'
export type { Sandbox } from './tools/sandbox.ts'
export type { DecodedText } from './tools/text.ts'
export { TOOLS } from './tools/specs.ts'
export { previewTool, previewToolScoped } from './tools/files.ts'
export { ignoreRules, wildcard } from './tools/walk.ts'
export { decodeText, decodeOutput, decodeTextInfo, safeEnd, safeStart } from './tools/text.ts'
export {
  MAX_READ_LINES, MAX_READ_CHARS, MAX_WRITE_CHARS, MAX_EDIT_BYTES, MAX_WALK_FILES,
  MAX_RESULTS, MAX_GREP_BYTES, MAX_GREP_CHARS, MAX_SHELL_CHARS, SHELL_TIMEOUT_MS,
  MAX_SHELL_TIMEOUT_MS, MAX_SHELL_COMMAND, MAX_CODE_CHARS, MAX_STDIN_CHARS, MAX_READ_BYTES,
  MAX_TREE_ENTRIES, MAX_TREE_DEPTH, GIT_TIMEOUT_MS, MAX_GIT_CHARS, MAX_GREP_CONTEXT,
  MAX_TASK_LINES, MAX_TASK_OUTPUT, MAX_HTTP_BODY, MAX_HTTP_BYTES, MAX_HTTP_CHARS,
  MAX_HTTP_TIMEOUT_MS, MAX_HTTP_REDIRECTS, MAX_MEMORY_ENTRY, HTTP_TIMEOUT_MS,
  MAX_IMAGE_BYTES, MAX_IMAGES_PER_QUESTION, MAX_IMAGE_BASE64, MAX_HTTP_SAVE_BYTES
} from './tools/limits.ts'

/**
 * Run one tool call against a conversation directory.
 * @param root - the conversation's working directory.
 * @param call - name plus parsed arguments.
 * @returns whether it worked, and the text handed back to the model.
 */
export async function runTool(
  root: string,
  call: ToolCall,
  readOnlyRoots: string[] = []
): Promise<ToolResult> {
  return await runToolScoped(root, root, call, readOnlyRoots)
}

/**
 * Run one tool with a separate work directory and writable scope.
 * @param workDir - relative paths resolve from here.
 * @param scopeRoot - writable root; null means the whole computer.
 * @param call - name plus parsed arguments.
 * @param readOnlyRoots - extra folders that may be read.
 * @param onOutput - receives throttled output of a running command.
 * @param sandbox - folders a command may reach, or null to skip the check.
 * @param skipDirs - top-level workspace folders the search tools leave out.
 * @returns whether it worked, and the text handed back to the model.
 */
export async function runToolScoped(
  workDir: string,
  scopeRoot: ToolScope,
  call: ToolCall,
  readOnlyRoots: string[] = [],
  onOutput?: (text: string) => void,
  sandbox: Sandbox | null = null,
  skipDirs: readonly string[] = []
): Promise<ToolResult> {
  try {
    switch (call.name) {
      case 'read':
        return { ok: true, text: await readFile(workDir, scopeRoot, call.args, readOnlyRoots) }
      case 'write':
        return { ok: true, text: await writeFile(workDir, scopeRoot, call.args) }
      case 'edit':
        return { ok: true, text: await editFile(workDir, scopeRoot, call.args) }
      case 'replace':
        return { ok: true, text: await replaceFiles(workDir, scopeRoot, call.args, skipDirs) }
      case 'move':
        return { ok: true, text: await moveFile(workDir, scopeRoot, call.args) }
      case 'copy':
        return { ok: true, text: await copyFile(workDir, scopeRoot, call.args) }
      case 'mkdir':
        return { ok: true, text: await makeDir(workDir, scopeRoot, call.args) }
      case 'delete':
        return { ok: true, text: await deleteFile(workDir, scopeRoot, call.args) }
      case 'list':
        return { ok: true, text: await listDir(workDir, scopeRoot, call.args, readOnlyRoots) }
      case 'glob':
        return { ok: true, text: await globFiles(workDir, scopeRoot, call.args, readOnlyRoots, skipDirs) }
      case 'grep':
        return { ok: true, text: await grepFiles(workDir, scopeRoot, call.args, readOnlyRoots, skipDirs) }
      case 'symbols':
        return { ok: true, text: await indexSymbols(workDir, scopeRoot, call.args, skipDirs) }
      case 'tree':
        return { ok: true, text: await treeDir(workDir, scopeRoot, call.args, readOnlyRoots, skipDirs) }
      case 'git':
        return await runGit(workDir, scopeRoot, call.args)
      case 'git_write':
        return await runGitWrite(workDir, scopeRoot, call.args)
      case 'wait_for':
        return await waitFor(workDir, call.args)
      case 'run_tests':
        return await runTests(workDir, scopeRoot, call.args, sandbox, onOutput)
      case 'diagnostics':
        return await runDiagnostics(workDir, scopeRoot, call.args, sandbox, onOutput)
      case 'task':
        return await runTask(workDir, call.args)
      case 'shell':
        return await runShell(workDir, scopeRoot, call.args, onOutput, sandbox)
      case 'python':
        return await runCode(workDir, scopeRoot, call.args, 'python', onOutput, sandbox)
      case 'node':
        return await runCode(workDir, scopeRoot, call.args, 'node', onOutput, sandbox)
      case 'http':
        return await runHttp(workDir, scopeRoot, call.args)
      case 'env':
        return await runEnv(workDir, scopeRoot, call.args)
      case 'read_image':
        return await readImage(workDir, scopeRoot, call.args, readOnlyRoots)
      case 'screenshot':
        return await screenshot(workDir, scopeRoot, call.args)
      case 'browser':
        return await runBrowser(workDir, call.args)
      case 'show':
        return await showFiles(workDir, scopeRoot, call.args, readOnlyRoots)
      default:
        return { ok: false, text: `未知工具：${call.name}` }
    }
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    return { ok: false, text: message }
  }
}
