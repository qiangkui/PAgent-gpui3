/** Every size and timeout cap the tools share. */

/** Upper bound for a single read, so a huge file cannot flood the context. */
export const MAX_READ_LINES = 400

/** Upper bound for one file's text, in characters. */
export const MAX_READ_CHARS = 200_000

/** Upper bound for one write, in characters. */
export const MAX_WRITE_CHARS = 400_000

/**
 * Largest file `edit` will rewrite.
 *
 * `edit` reads the whole file, diffs it and writes it back; on a huge log or
 * database dump that is both slow and a good way to lose the file, so it is
 * refused with a message that points at `shell` instead.
 */
export const MAX_EDIT_BYTES = 2 * 1024 * 1024

/** Upper bound for a directory walk, in files. */
export const MAX_WALK_FILES = 5_000

/** Upper bound for `glob` and `grep` output. */
export const MAX_RESULTS = 200

/**
 * Most matching lines one file contributes to a `grep` answer.
 *
 * The cap above is global: without a per-file limit, a log or a generated file
 * with hundreds of hits fills all 200 lines and every other file disappears from
 * the answer. Context lines count too, so one very long line cannot flood it.
 */
export const MAX_GREP_PER_FILE = 20

/** Total file size one `grep` call reads, so it cannot walk a whole disk. */
export const MAX_GREP_BYTES = 8 * 1024 * 1024

/** Longest `grep` text kept; matching lines can themselves be very long. */
export const MAX_GREP_CHARS = 20_000

/** Longest command output kept; the head and tail survive. */
export const MAX_SHELL_CHARS = 16_000

/** Default runtime of one command, in milliseconds. */
export const SHELL_TIMEOUT_MS = 60_000

/**
 * Default runtime of a whole test suite or compile check.
 *
 * A cold `cargo check` or a first `npm test` often needs minutes; the old 60 s
 * default made the model rerun with a bigger time budget, which cost a whole
 * round. This is the same value as the maximum, because the tool is confirmed
 * once and the person can still lower it with `timeoutMs`.
 */
export const LONG_RUN_TIMEOUT_MS = 300_000

/** Longest a command may run when it asks for more time. */
export const MAX_SHELL_TIMEOUT_MS = 300_000

/** Longest command line accepted, so one call cannot bloat the transcript. */
export const MAX_SHELL_COMMAND = 8_000

/** Longest code snippet the `python` / `node` tools accept. */
export const MAX_CODE_CHARS = 20_000

/** Longest text a command may be fed through stdin. */
export const MAX_STDIN_CHARS = 20_000

/** Most output lines one `task` call shows. */
export const MAX_TASK_LINES = 400

/** Longest tail of a background task's log kept for one `task` call. */
export const MAX_TASK_OUTPUT = 20_000

/** Largest file `read` loads; a huge file would be read whole before clipping. */
export const MAX_READ_BYTES = 8 * 1024 * 1024

/** Most entries a single `tree` call lists. */
export const MAX_TREE_ENTRIES = 400

/** Deepest `tree` call accepted, so a loop cannot walk the whole disk. */
export const MAX_TREE_DEPTH = 4

/** How long one read-only git command may run. */
export const GIT_TIMEOUT_MS = 15_000

/** Longest git output kept; a diff of a large change is still readable. */
export const MAX_GIT_CHARS = 16_000

/** Most context lines `grep` shows on either side of a hit. */
export const MAX_GREP_CONTEXT = 5

/** Longest body the `http` tool sends. */
export const MAX_HTTP_BODY = 20_000

/** Default wait for one `http` request; the tool-level cap is 60 s. */
export const HTTP_TIMEOUT_MS = 20_000

/** Longest wait one `http` request may ask for. */
export const MAX_HTTP_TIMEOUT_MS = 30_000

/** Largest response body kept, in bytes. */
export const MAX_HTTP_BYTES = 400_000

/**
 * Largest download `saveTo` keeps.
 *
 * A saved body is not shown to the model, so the transcript is not the reason
 * for a cap any more; this only stops a runaway download from filling the disk
 * before the person notices.
 */
export const MAX_HTTP_SAVE_BYTES = 20 * 1024 * 1024

/**
 * Longest response text handed to the model.
 *
 * A documentation page turns into a few thousand words of prose once its tags
 * are stripped, so the cap is set for a readable page rather than for one
 * screenful: 8 000 characters used to cut most real pages in half.
 */
export const MAX_HTTP_CHARS = 20_000

/** Most response header lines listed. */
export const MAX_HTTP_HEADERS = 20

/** Redirects one call may follow before it reports a loop. */
export const MAX_HTTP_REDIRECTS = 5

/**
 * Longest text one `remember` entry may carry.
 *
 * An entry rides on every later question, so a pasted document would cost more
 * than the fact it was meant to record.
 */
export const MAX_MEMORY_ENTRY = 1_000

/** Largest image a tool will read, in bytes. */
export const MAX_IMAGE_BYTES = 5 * 1024 * 1024

/**
 * Largest picture, in pixels, that `read_image` hands to a model.
 *
 * Providers bill and cap images by area, not by file size: a well-compressed
 * 5 MB PNG can be 10000×10000, which is past every vision model's limit. A file
 * that big is refused with a hint instead of being sent to be rejected.
 */
export const MAX_IMAGE_PIXELS = 20_000_000

/**
 * Longest page text one `browser` action hands back.
 *
 * A snapshot already shortens each node; this caps the whole body excerpt, so a
 * page with a million-word article costs one screenful rather than the answer.
 */
export const MAX_BROWSER_TEXT = 6_000

/** Most accessibility nodes one snapshot lists. */
export const MAX_BROWSER_NODES = 300

/** Most console lines and network requests one report lists. */
export const MAX_BROWSER_LOG = 40

/** Longest JavaScript one `eval` action may run. */
export const MAX_BROWSER_EVAL = 4_000

/** How long one DevTools command may take before it is reported as stuck. */
export const BROWSER_TIMEOUT_MS = 15_000

/** How long the browser may take to open its debugging port. */
export const BROWSER_LAUNCH_MS = 12_000

/**
 * Attempts at starting the browser binary.
 *
 * Edge updates replace its program folder while the machine runs, and during that
 * swap an existing `msedge.exe` briefly reports as missing; a second look a moment
 * later finds it again.
 */
export const BROWSER_LAUNCH_ATTEMPTS = 6

/** Most images one question may attach, so a screenshot loop cannot flood it. */
export const MAX_IMAGES_PER_QUESTION = 3

/**
 * Total base64 image text one question may carry.
 *
 * Base64 costs a third more than the file itself, and every round resends the
 * whole transcript; three small screenshots fit, three retina captures do not.
 */
export const MAX_IMAGE_BASE64 = 12_000_000

/**
 * Most pages one `read` of a PDF shows at once.
 *
 * A page range wider than this is refused rather than silently cut: the model
 * asks for the next block with `pages`, which keeps the answer inside one window.
 */
export const MAX_PDF_PAGES = 40

/** Most characters one PDF reading hands over, page markers included. */
export const MAX_PDF_CHARS = 20_000

/** How long one PDF extraction may take. */
export const PDF_TIMEOUT_MS = 60_000
