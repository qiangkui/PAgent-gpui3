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

/** An image a tool returns, so the model can look at it. */
export interface ToolImage {
  /** MIME type the provider accepts, for example `image/png`. */
  mime: string
  /** Base64 bytes, without the `data:` prefix. */
  data: string
  /**
   * Absolute file that holds the same picture, when there is one.
   *
   * The window keeps it so the next question can show the picture again without
   * the model having to take it a second time. Empty when the image only ever
   * existed in memory (a capture the sandbox refused to save).
   */
  path?: string
}

/** A finished file the tool hands to the person, drawn as a card in the window. */
export interface ToolFile {
  /** Name the card shows; usually the file's own name. */
  name: string
  /** Absolute path, which is what the window opens. */
  path: string
  /** Size in bytes, for the card's second line. */
  bytes: number
}

/** What running a tool produced. */
export interface ToolResult {
  /** False when the call was rejected or threw. */
  ok: boolean
  /** Text handed back to the model. */
  text: string
  /**
   * The structured value, when the action produced one.
   *
   * `eval` puts the page's own return value here: the text beside it may carry
   * human-facing notes ("rejoined the browser"), and a workflow step needs the
   * value rather than a sentence it would have to parse.
   */
  value?: unknown
  /**
   * An image to show the model, for `read_image` and `screenshot`.
   *
   * Tool messages carry text only in every provider the engine speaks to, so
   * the loop attaches this as a user message right after the result. Absent for
   * every tool that has nothing to show.
   */
  image?: ToolImage
  /**
   * Files to show the person, for `show`.
   *
   * The model decides what belongs here: a finished report it wants the reader
   * to open, not every file it happened to touch. The window draws them under
   * the tool row as cards, so the answer does not end with「完成了」and a folder to
   * search by hand.
   */
  files?: ToolFile[]
}

/** Where relative paths start, and the folder writes may not leave. */
export type ToolScope = string | null
