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

/** Total file size one `grep` call reads, so it cannot walk a whole disk. */
export const MAX_GREP_BYTES = 8 * 1024 * 1024

/** Longest `grep` text kept; matching lines can themselves be very long. */
export const MAX_GREP_CHARS = 20_000

/** Longest command output kept; the head and tail survive. */
export const MAX_SHELL_CHARS = 16_000

/** Default runtime of one command, in milliseconds. */
export const SHELL_TIMEOUT_MS = 60_000

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

/** Longest response text handed to the model. */
export const MAX_HTTP_CHARS = 8_000

/** Most response header lines listed. */
export const MAX_HTTP_HEADERS = 20
