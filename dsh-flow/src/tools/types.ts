/** The shapes the tool loop speaks: a call, a result and a preview. */

/** A tool the model can call: name, prose description and JSON schema. */
export interface ToolDefinition {
  type: 'function'
  function: {
    name: string
    description: string
    parameters: Record<string, unknown>
  }
}

/** A pending change that a person should see before it happens. */
export interface ToolPreview {
  /** A unified-ish line diff of the file before and after the call. */
  diff: string
  /** Text before the change, when the pair is small enough to keep for undo. */
  before?: string
  /** Text after the change, for the same reason. */
  after?: string
  /** False when the call would create the file. */
  existed: boolean
  /** True when `before` / `after` are included and undo is possible. */
  undoable: boolean
}

/** One call the model asked for. */
export interface ToolCall {
  /** Provider-assigned id, echoed back on the tool message. */
  id: string
  /** Tool name, one of the definitions below. */
  name: string
  /** Parsed arguments. */
  args: Record<string, unknown>
}

/** What running a tool produced. */
export interface ToolResult {
  /** False when the call was rejected or threw. */
  ok: boolean
  /** Text handed back to the model. */
  text: string
}

/** Where relative paths start, and the folder writes may not leave. */
export type ToolScope = string | null
