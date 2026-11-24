/**
 * Workflow manifest and runtime types.
 *
 * A workflow is an ordered list of steps. A step runs a real program in any
 * language (`run`), assigns values (`set`), fans out (`map`), or asks a model
 * one question (`ai`). Only `ai` steps cost tokens, and even those can replay a
 * recorded answer, so a settled workflow runs at zero model calls.
 */

/** Which executor owns a step. */
export type StepKind = 'run' | 'set' | 'map' | 'ai'

/** Settlement state of one step. */
export type StepStatus = 'ok' | 'failed' | 'skipped'

/** Command form of a `run` step. */
export interface RunSpec {
  /** Program plus arguments, or a command line when `shell` is set. */
  command: string | string[]
  /** Working directory, resolved against the manifest directory. */
  cwd?: string
  /** Extra environment variables. */
  env?: Record<string, string>
  /** Kill the command after this many milliseconds. */
  timeoutMs?: number
  /** Run through the platform shell instead of exec'ing the program directly. */
  shell?: boolean | string
  /** Capture stdout only as text, even when it parses as JSON. */
  textOnly?: boolean
}

/** Fan-out form of a `map` step. */
export interface MapSpec {
  /** Expression yielding the array to iterate. */
  over: string
  /** Variable name bound to the current element. Defaults to `item`. */
  as?: string
  /** Maximum simultaneous iterations. Defaults to 4. */
  concurrency?: number
  /** Steps executed once per element. */
  steps: Step[]
}

/** Where an `ai` step gets its answer. */
export type AiBackendName = 'http' | 'cli'

/** The explicit model call of an `ai` step. */
export interface AiSpec {
  /** The question, after template rendering. */
  prompt: string
  /** Optional instruction prepended as a system message. */
  system?: string
  /** JSON-schema subset the answer must satisfy. */
  schema?: Record<string, unknown>
  /** Model id; the provider default when absent. */
  model?: string
  /** Sampling temperature. */
  temperature?: number
  /** Upper bound on generated tokens. */
  maxTokens?: number
  /** `http` (default) or an external `cli` program. */
  backend?: AiBackendName
  /** Program plus arguments for `backend: cli`. */
  command?: string | string[]
  /** Per-request timeout. */
  timeoutMs?: number
}

/** How much of a run may call a model. */
export type RunLevel = 'manual' | 'assist' | 'draft'

/** One unit of work. */
export interface Step {
  /** Unique id within its containing step list; referenced as `steps.<id>`. */
  id: string
  /** Human-readable purpose. */
  desc?: string
  /** Expression gating the step; falsy marks it `skipped`. */
  when?: string
  /** Always skip, keeping the step as documentation. */
  skip?: boolean
  /** Keep the run going when the step fails. */
  allowFailure?: boolean
  /**
   * Run only until the first success, remembered in the workflow's ledger.
   * `true` keys the entry by step id; a string supplies a custom key.
   */
  once?: boolean | string
  /** Attempts after the first failure. Defaults to 0. */
  retries?: number
  /** Delay between attempts, in milliseconds. Defaults to 250. */
  retryDelayMs?: number
  /** Store the step's primary value under `vars.<save>`. */
  save?: string
  /** Why this step must ask a model; required for `ai` steps under strict mode. */
  why?: string
  /** Deterministic external program, normalized to its object form. */
  run?: RunSpec
  /** Assign rendered values into vars. */
  set?: Record<string, unknown>
  /** Fan out over a list. */
  map?: MapSpec
  /** Explicit model call, normalized to its object form. */
  ai?: AiSpec
}

/** Defaults applied to every `run` step. */
export interface RunDefaults {
  /** Working directory for commands that do not set one. */
  cwd?: string
  /** Timeout for commands that do not set one. */
  timeoutMs?: number
  /** Environment applied to every command. */
  env?: Record<string, string>
}

/** A parsed and validated manifest. */
export interface Workflow {
  /** Kebab-case identity. */
  name: string
  /** One-line purpose. */
  description: string
  /** Manifest format version; currently 1. */
  version: number
  /** Declared inputs. */
  inputs: Record<string, InputSpec>
  /** Initial variables. */
  vars: Record<string, unknown>
  /** Shared defaults. */
  defaults: RunDefaults
  /** Long-lived processes started before the steps and stopped afterwards. */
  services?: Record<string, ServiceSpec>
  /** Ordered root steps. */
  steps: Step[]
}

/** How a service proves it is ready. */
export interface ServiceReady {
  /** HTTP endpoint that must answer 2xx. */
  url: string
  /** Total time allowed; defaults to 15000. */
  timeoutMs?: number
  /** Delay between probes; defaults to 250. */
  intervalMs?: number
}

/** One run-level process. */
export interface ServiceSpec {
  /** How to start it; relative paths resolve against the manifest. */
  run: RunSpec
  /** Readiness probe; absent means "started" is good enough. */
  ready?: ServiceReady
}

/** A declared workflow input. */
export interface InputSpec {
  /** Human-readable purpose. */
  description?: string
  /** Reject the run when the input is absent. */
  required?: boolean
  /** Value used when the caller supplies none. */
  default?: unknown
}

/** One iteration of a `map` step. */
export interface MapItemResult {
  /** Zero-based position. */
  index: number
  /** The element this iteration received. */
  item: unknown
  /** Steps run for this element, keyed by step id. */
  steps: Record<string, StepResult>
  /** False when any nested step failed. */
  ok: boolean
}

/** Recorded outcome of one executed step. */
export interface StepResult {
  /** Step id. */
  id: string
  /** Which executor ran it. */
  kind: StepKind
  /** Settlement state. */
  status: StepStatus
  /** Exit code, for `run`. */
  exitCode?: number
  /** Captured standard output. */
  stdout?: string
  /** Captured standard error. */
  stderr?: string
  /** Parsed stdout, when it is JSON and the step did not opt out. */
  json?: unknown
  /** Text form of the primary output. */
  text?: string
  /** Per-iteration outcomes, for `map`. */
  items?: MapItemResult[]
  /** Model that answered, for `ai`. */
  model?: string
  /** Token accounting reported by the provider, for `ai`. */
  usage?: unknown
  /** True when the answer came from the recorded decision cache. */
  cached?: boolean
  /** Failure message, when `status` is `failed`. */
  error?: string
  /** Wall-clock duration. */
  durationMs: number
  /** True when a stream hit the capture cap. */
  truncated?: boolean
  /** Rendered command line, kept for dry runs and reports. */
  command?: string
}

/** Names visible to expressions and templates. */
export interface Scope {
  /** Daemon-free access to declared inputs. */
  inputs: Record<string, unknown>
  /** Assigned variables. */
  vars: Record<string, unknown>
  /** Recorded step results, keyed by step id. */
  steps: Record<string, StepResult>
  /** Process environment. */
  env: Record<string, string | undefined>
  /** Current fan-out element. */
  item?: unknown
  /** Current fan-out index. */
  index?: number
  /** Names bound by an enclosing `map` step, addressable as bare paths. */
  bindings?: Record<string, unknown>
}

/** One line of the run trace. */
export interface TraceRecord {
  /** ISO timestamp. */
  ts: string
  /** Event name. */
  type:
    | 'run/start'
    | 'step/start'
    | 'step/end'
    | 'run/end'
    | 'service/start'
    | 'service/ready'
    | 'service/end'
  /** Step id, for step events. */
  id?: string
  /** Step kind, for step events. */
  kind?: StepKind
  /** Settlement state, for `step/end`. */
  status?: StepStatus
  /** Duration in milliseconds, for `step/end`. */
  durationMs?: number
  /** Rendered command, for `run` steps. */
  command?: string
  /** Failure or skip reason. */
  detail?: string
}
