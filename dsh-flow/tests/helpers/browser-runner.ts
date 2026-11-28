// Runs one browser action in its own process, the way the engine does for each
// question. Used by the cross-request test: the second run must rejoin the page
// the first one left behind.
import { runBrowser } from '../../src/tools/browser.ts'
import { detachBrowser } from '../../src/tools/browser.ts'

const [root, payload] = process.argv.slice(2)
if (!root || !payload) {
  console.error('usage: node browser-runner.ts <workDir> <json args>')
  process.exit(2)
}
const args = JSON.parse(payload) as Record<string, unknown>
const result = await runBrowser(root, args)
console.log(JSON.stringify({ ok: result.ok, text: result.text }))
// What the engine does when its answer is done: drop the socket so the process
// can end, and leave the browser running for the next request.
await detachBrowser()