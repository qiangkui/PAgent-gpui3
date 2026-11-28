/**
 * Credential files the tools refuse to read.
 *
 * The write side already refused to touch these; reading them was still open,
 * which meant one `read .env` put a live key into the transcript and into the
 * provider's logs. Refusing here is deliberate and loud: the message says why and
 * offers the safe alternative, so the model asks instead of guessing.
 */

import path from 'node:path'

/** Example files that look like `.env` but hold no secrets. */
const ENV_EXAMPLES = new Set(['.env.example', '.env.sample', '.env.template', '.env.dist'])

/** Dotfiles that hold credentials under a familiar name. */
const CREDENTIAL_DOTFILES = new Set([
  '.npmrc',
  '.pypirc',
  '.netrc',
  '.htpasswd',
  '.git-credentials'
])

/** Extensions that are keys or certificate stores. */
const SECRET_EXTENSIONS = /\.(pem|key|pfx|p12|jks|kdbx)$/i

/** Private keys written by ssh-keygen; `.pub` is public and allowed. */
const PRIVATE_KEY = /^id_(rsa|dsa|ecdsa|ed25519)$/i

/**
 * Whether a path names credentials, a key, or a place that stores them.
 * @param candidate - a path, absolute or relative.
 * @returns true when reading it would put a secret in the transcript.
 */
export function isSensitivePath(candidate: string): boolean {
  const slashed = candidate.replace(/\\/g, '/').toLowerCase()
  if (slashed.includes('/.ssh/') || slashed.startsWith('.ssh/') || slashed.endsWith('/.ssh')) {
    return true
  }
  const name = path.basename(candidate).toLowerCase()
  if (name === '.env' || (name.startsWith('.env.') && !ENV_EXAMPLES.has(name))) return true
  if (SECRET_EXTENSIONS.test(name)) return true
  if (PRIVATE_KEY.test(name)) return true
  if (CREDENTIAL_DOTFILES.has(name)) return true
  return name.includes('credential') || name.includes('secret')
}

/**
 * Whether one name is a credential file, judged narrowly.
 *
 * Narrower than {@link isSensitivePath}: a command line is prose as much as it is
 * code, and a commit message that merely says "secret" must still run. Only a
 * familiar file name, a key extension or an `.env`-style dotfile counts.
 * @param candidate - a token, a path or a quoted string.
 * @returns true for a name that must not be read by a command.
 */
function isCredentialName(candidate: string): boolean {
  const name = path.basename(candidate.replace(/\\/g, '/')).toLowerCase()
  if (name === '' || ENV_EXAMPLES.has(name)) return false
  if (name === '.env' || name.startsWith('.env.')) return true
  if (CREDENTIAL_DOTFILES.has(name)) return true
  if (SECRET_EXTENSIONS.test(name)) return true
  if (PRIVATE_KEY.test(name)) return true
  return /^(credentials?|secrets?)\.(json|ya?ml|toml|ini|conf)$/.test(name)
}

/**
 * Credential files a command looks like it would read.
 *
 * `read` and `grep` refuse these files; a shell command could print the same
 * bytes, so the tools that run commands check the text before running it. The scan
 * looks at whole tokens, `--flag=value` parts and quoted strings inside code, which
 * is enough for `type .env`, `--env-file=.env` and `open('.env')` without turning a
 * commit message into a false alarm.
 * @param text - the command line or the code about to run.
 * @returns the names found, in order and without duplicates.
 */
export function credentialMentions(text: string): string[] {
  const found = new Set<string>()
  const consider = (candidate: string): void => {
    const clean = candidate
      .trim()
      .replace(/^[('"`[]+/, '')
      .replace(/[)'"`\],;:]+$/, '')
    if (clean === '' || clean.length > 300) return
    if (!isCredentialName(clean)) return
    // `.ssh/id_rsa` reads as `id_rsa`: the name is what the message should say.
    found.add(path.basename(clean.replace(/\\/g, '/')))
  }
  for (const token of text.split(/[\s;|&<>(){}\[\],=]+/)) consider(token)
  for (const quoted of text.matchAll(/['"`]([^'"`\n]{1,300})['"`]/g)) consider(quoted[1] ?? '')
  return [...found].slice(0, 5)
}

/**
 * What a refused command says.
 *
 * The model needs both facts: that this is deliberate, and what to do instead.
 * @param names - the credential names the command mentioned.
 * @returns the refusal text.
 */
export function credentialRefusal(names: readonly string[]): string {
  return [
    `这条命令提到凭据文件（${names.join('、')}），没有执行。`,
    '把整份密钥读进对话会同时进入提供方日志；需要其中某一项时，请让用户单独贴出那个值，',
    '或只用 list / glob 看文件名。'
  ].join('')
}

/**
 * What a refused read says.
 *
 * The model needs two things: that this is not a permissions accident, and what
 * to do instead. Asking for the one value it needs is safe; pasting the file is
 * not.
 * @param requested - the path the call named, as the model wrote it.
 * @returns the refusal text.
 */
export function sensitiveRefusal(requested: string): string {
  return `${requested} 看起来是凭据或密钥文件，这里不读它，也不把它发进对话。\n如果确实需要其中的某个值，请让用户把那一项单独贴出来（不要贴整份文件）；只需要路径或键名时用 list / glob。`
}