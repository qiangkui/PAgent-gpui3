// The browser tool: its URL rule, its approval rule, the labels a card shows,
// and the browser it finds on this machine. The protocol itself is exercised by
// the opt-in test at the bottom, which needs a real Edge or Chrome.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { execFile } from 'node:child_process'
import { promisify } from 'node:util'
import { existsSync, mkdtempSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { createServer } from 'node:http'
import {
  browserActive,
  browserIdleMs,
  browserNeedsCard,
  browserSummary,
  browserUrl,
  clip,
  downloadsDir,
  findBrowser,
  killByProfile,
  listVersions,
  readSavedSession,
  runBrowser,
  shortUrl,
  writeSavedSession
} from '../src/tools/browser.ts'

const execFileAsync = promisify(execFile)

test('browser only opens http and https pages', () => {
  assert.equal(browserUrl('https://example.com/a?b=1'), 'https://example.com/a?b=1')
  assert.equal(browserUrl('http://127.0.0.1:8080/'), 'http://127.0.0.1:8080/')
  assert.equal(browserUrl('about:blank'), 'about:blank')
  for (const refused of ['file:///C:/Windows/win.ini', 'javascript:alert(1)', 'data:text/html,x']) {
    assert.throws(() => browserUrl(refused), /只打开 http \/ https/, refused)
  }
  assert.throws(() => browserUrl('example.com'), /不是完整 URL/)
  assert.throws(() => browserUrl(''), /缺少 url/)
})

test('only navigation and page-changing browser actions ask for a card', () => {
  for (const action of ['open', 'goto', 'click', 'type', 'press', 'select', 'eval']) {
    assert.equal(browserNeedsCard(action), true, action)
  }
  for (const action of [
    'snapshot',
    'text',
    'console',
    'network',
    'screenshot',
    'wait_for',
    'downloads',
    'close'
  ]) {
    assert.equal(browserNeedsCard(action), false, action)
  }
})

test('a browser card names what the call will do', () => {
  assert.equal(
    browserSummary({ action: 'open', url: 'https://example.com' }),
    '打开页面：https://example.com'
  )
  assert.equal(
    browserSummary({ action: 'goto', url: 'https://example.com/x' }),
    '跳转到：https://example.com/x'
  )
  assert.equal(browserSummary({ action: 'click', ref: 3 }), '点击 编号 [3]')
  assert.equal(browserSummary({ action: 'click', selector: '#ok' }), '点击 #ok')
  assert.equal(browserSummary({ action: 'click' }), '点击 （缺少 ref / selector）')
  assert.match(
    browserSummary({ action: 'type', ref: 2, text: '张三', submit: true }),
    /在 编号 \[2\] 输入：张三（并回车提交）/
  )
  assert.match(browserSummary({ action: 'press', key: 'enter' }), /按键：enter/)
  assert.match(browserSummary({ action: 'select', ref: 4, values: ['b'] }), /选择：b/)
  assert.match(browserSummary({ action: 'eval', expression: 'document.title' }), /执行 JavaScript：document\.title/)
})

test('long lines are cut without breaking characters', () => {
  assert.equal(clip('  一行  两个空格  ', 20), '一行 两个空格')
  assert.equal(clip('abcdefghij', 5), 'abcd…')
  assert.equal(shortUrl('https://example.com/aaa', 80), 'https://example.com/aaa')
  assert.equal(shortUrl('x'.repeat(100), 20).length, 20)
})

test('the newest browser version folder is tried first', () => {
  const root = mkdtempSync(path.join(tmpdir(), 'browser-versions-'))
  for (const name of ['9.9.9', '158.0.4370.33', '17.0.1', '158.0.1.deleting', 'notes.txt']) {
    mkdirSync(path.join(root, name), { recursive: true })
  }
  assert.deepEqual(listVersions(root), ['158.0.4370.33', '17.0.1', '9.9.9'])
  assert.deepEqual(listVersions(path.join(root, 'missing')), [])
})

test('DSH_BROWSER_PATH decides which browser is started', () => {
  const dir = mkdtempSync(path.join(tmpdir(), 'browser-path-'))
  const fake = path.join(dir, 'chrome.exe')
  writeFileSync(fake, 'not really a browser')
  const before = process.env.DSH_BROWSER_PATH
  try {
    process.env.DSH_BROWSER_PATH = fake
    assert.equal(findBrowser()?.file, fake)
    assert.equal(findBrowser()?.label, 'chrome')
    process.env.DSH_BROWSER_PATH = path.join(dir, 'gone.exe')
    // A missing override falls back to what the machine actually has; on a
    // machine without Edge or Chrome that is null, so only the path is checked.
    const fallback = findBrowser()
    assert.ok(fallback === null || fallback.file !== fake)
  } finally {
    if (before === undefined) delete process.env.DSH_BROWSER_PATH
    else process.env.DSH_BROWSER_PATH = before
  }
})

const e2eSkip = process.env.DSH_BROWSER_TEST !== '1' && '设置 DSH_BROWSER_TEST=1 才运行（需要本机 Edge / Chrome）'

test('browser drives a real page when DSH_BROWSER_TEST is set', { skip: e2eSkip }, async () => {
  const server = createServer((request, response) => {
    response.writeHead(200, { 'content-type': 'text/html; charset=utf-8' })
    response.end(
      '<html><head><title>测试页</title></head><body>' +
        '<p id="out">未点击</p>' +
        // A tall spacer puts the button below the fold: the element screenshot
        // must use page coordinates (scroll offset included), not viewport ones.
        '<div style="height:1200px">占位</div>' +
        "<button id=\"go\" onclick=\"document.getElementById('out').textContent='已点击'\">确认</button>" +
        '<input id="name" /><select id="pick"><option value="a">A</option><option value="b">B</option></select>' +
        '<script>console.log("脚本运行")</script></body></html>'
    )
  })
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', () => resolve()))
  const port = (server.address() as { port: number }).port
  const root = mkdtempSync(path.join(tmpdir(), 'browser-e2e-'))
  const call = (args: Record<string, unknown>) => runBrowser(root, args)
  try {
    const opened = await call({ action: 'open', url: `http://127.0.0.1:${port}/` })
    assert.equal(opened.ok, true, opened.text)
    assert.match(opened.text, /测试页/)

    const listing = await call({ action: 'snapshot' })
    assert.match(listing.text, /textbox/)
    const ref = Number(/^\[(\d+)\] button/m.exec(listing.text)?.[1] ?? 0)
    assert.ok(ref > 0, listing.text)

    assert.equal((await call({ action: 'click', ref })).ok, true)
    assert.match((await call({ action: 'text' })).text, /已点击/)

    const inputRef = Number(/^\[(\d+)\] textbox/m.exec(listing.text)?.[1] ?? 0)
    assert.equal((await call({ action: 'type', ref: inputRef, text: '张三' })).ok, true)
    assert.equal(
      (await call({ action: 'eval', expression: "document.getElementById('name').value" })).text,
      '张三'
    )

    const selectRef = Number(/^\[(\d+)\] combobox/m.exec(listing.text)?.[1] ?? 0)
    assert.equal((await call({ action: 'select', ref: selectRef, values: ['b'] })).ok, true)
    assert.equal(
      (await call({ action: 'eval', expression: "document.getElementById('pick').value" })).text,
      'b'
    )

    assert.equal((await call({ action: 'wait_for', text: '已点击', timeoutMs: 3000 })).ok, true)
    const shot = await call({ action: 'screenshot' })
    assert.equal(shot.ok, true, shot.text)
    assert.equal(shot.image?.mime, 'image/jpeg')
    assert.ok(shot.image?.path !== undefined)

    // A captcha is a picture of one element: `selector` clips the shot to it,
    // which is what a workflow step needs to send the model.
    const elementShot = await call({ action: 'screenshot', selector: '#go', format: 'png', maxEdge: 400 })
    assert.equal(elementShot.ok, true, elementShot.text)
    assert.equal(elementShot.image?.mime, 'image/png')
    assert.match(elementShot.text, /#go/)
    // A missing selector is an error rather than a picture of the viewport: a
    // workflow step must not send the model a screenshot of the wrong thing.
    await assert.rejects(call({ action: 'screenshot', selector: '#nope' }), /#nope/)

    assert.match((await call({ action: 'console' })).text, /脚本运行/)
    assert.match((await call({ action: 'network' })).text, /200 GET Document/)
    assert.equal((await call({ action: 'close' })).ok, true)
  } finally {
    await call({ action: 'close' }).catch(() => undefined)
    server.close()
  }
})

test('the browser and its page survive into the next request', { skip: e2eSkip }, async () => {
  // The engine runs one process per question; this test runs one process per
  // action, which is the same thing as far as the browser session is concerned.
  const server = createServer((request, response) => {
    response.writeHead(200, { 'content-type': 'text/html; charset=utf-8' })
    response.end(
      '<html><head><title>连续测试</title></head><body><h1>跨请求的页面</h1><p>第二问应该还能看到这段文字。</p></body></html>'
    )
  })
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', () => resolve()))
  const port = (server.address() as { port: number }).port
  const root = mkdtempSync(path.join(tmpdir(), 'browser-continuity-'))
  const runner = fileURLToPath(new URL('./helpers/browser-runner.ts', import.meta.url))
  const run = async (args: Record<string, unknown>) => {
    const { stdout } = await execFileAsync(process.execPath, [runner, root, JSON.stringify(args)], {
      timeout: 60_000
    })
    return JSON.parse(stdout.trim().split('\n').filter(Boolean).at(-1) ?? '{}') as {
      ok: boolean
      text: string
    }
  }
  const sessionFile = path.join(root, '.dsh-flow', 'browser', 'session.json')
  try {
    const opened = await run({ action: 'open', url: `http://127.0.0.1:${port}/` })
    assert.equal(opened.ok, true, opened.text)
    assert.match(opened.text, /跨请求的页面/)
    assert.equal(existsSync(sessionFile), true, '第一问把浏览器写进了 session.json')
    const before = JSON.parse(readFileSync(sessionFile, 'utf8')) as {
      endpoint: string
      pageTarget: string
    }

    // Something only this page knows: it survives only if the page itself did.
    const marked = await run({
      action: 'eval',
      expression: 'window.__carried = "kept"; document.title'
    })
    assert.match(marked.text, /连续测试$/)

    // A second process, standing in for the next question.
    const carried = await run({ action: 'eval', expression: 'window.__carried' })
    assert.match(carried.text, /kept$/, carried.text)
    const text = await run({ action: 'text' })
    assert.match(text.text, /第二问应该还能看到这段文字/)
    assert.match(text.text, /已接上上一问打开的浏览器/)

    const after = JSON.parse(readFileSync(sessionFile, 'utf8')) as { endpoint: string }
    assert.equal(after.endpoint, before.endpoint, '同一个浏览器，不是新起的')

    const closed = await run({ action: 'close' })
    assert.equal(closed.ok, true)
    assert.equal(existsSync(sessionFile), false, 'close 之后不留指针')
    assert.equal(existsSync(path.join(root, '.dsh-flow', 'browser')), false, 'close 之后不留 profile')
  } finally {
    await run({ action: 'close' }).catch(() => undefined)
    server.close()
  }
})

test('downloads land in the conversation, and the list reads the folder', async () => {
  const root = mkdtempSync(path.join(tmpdir(), 'browser-downloads-'))
  assert.equal(downloadsDir(root), path.join(root, '.dsh-flow', 'downloads'))
  // No browser has run yet: the report is a fact about the folder, not a session.
  const empty = await runBrowser(root, { action: 'downloads' })
  assert.equal(empty.ok, true)
  assert.match(empty.text, /还没有文件/)
  assert.match(empty.text, /\.dsh-flow\\downloads|\.dsh-flow\/downloads/)

  const dir = downloadsDir(root)
  mkdirSync(dir, { recursive: true })
  writeFileSync(path.join(dir, 'old.csv'), 'a,b\n', 'utf8')
  writeFileSync(path.join(dir, 'invoice.pdf'), 'x'.repeat(2048), 'utf8')
  // A file Chrome is still writing must not be listed as a finished download.
  writeFileSync(path.join(dir, 'half.zip.crdownload'), 'x', 'utf8')
  const listed = await runBrowser(root, { action: 'downloads' })
  assert.match(listed.text, /共 2 个文件/)
  assert.match(listed.text, /invoice\.pdf/)
  assert.match(listed.text, /old\.csv/)
  assert.doesNotMatch(listed.text, /crdownload/)
  // The newest file is listed first, and every line carries the absolute path the
  // model can hand to `show`.
  const lines = listed.text.split('\n').filter((line) => line.startsWith('- '))
  assert.ok(lines[0]?.includes('invoice.pdf'), listed.text)
  assert.ok(lines[0]?.includes(dir), lines[0])
})

test('wait_download reports the finished file as data, not prose', async () => {
  const root = mkdtempSync(path.join(tmpdir(), 'browser-wait-'))
  const dir = downloadsDir(root)
  mkdirSync(dir, { recursive: true })
  const target = path.join(dir, '使用单位导出.xls')
  writeFileSync(target, 'x')
  const result = await runBrowser(root, {
    action: 'wait_download',
    name: '使用单位导出.xls',
    timeoutMs: 2_000,
  })
  assert.equal(result.ok, true)
  assert.equal(result.files?.length, 1)
  assert.equal(result.files?.[0]?.name, '使用单位导出.xls')
  assert.equal(result.files?.[0]?.bytes, 1)
  assert.equal(result.files?.[0]?.path, target)
})

test('wait_download ignores a file left over from an earlier run', async () => {
  // A second export often lands as `name (1).xls`; `since` is what keeps the
  // previous run's file from being reported as the new result.
  const root = mkdtempSync(path.join(tmpdir(), 'browser-wait-old-'))
  const dir = downloadsDir(root)
  mkdirSync(dir, { recursive: true })
  writeFileSync(path.join(dir, '使用单位导出.xls'), 'old')
  const result = await runBrowser(root, {
    action: 'wait_download',
    name: '使用单位导出.xls',
    since: Date.now() + 60_000,
    timeoutMs: 500,
  })
  assert.equal(result.ok, false)
  assert.match(result.text, /500 ms/)
})

test('wait_download says what it was waiting for when nothing arrives', async () => {
  const root = mkdtempSync(path.join(tmpdir(), 'browser-wait-none-'))
  const result = await runBrowser(root, { action: 'wait_download', pattern: '*.xls', timeoutMs: 500 })
  assert.equal(result.ok, false)
  assert.match(result.text, /\*\.xls/)
})

test('a page that hands over a file writes it into that folder', { skip: e2eSkip }, async () => {
  const server = createServer((request, response) => {
    if (request.url?.startsWith('/file')) {
      response.writeHead(200, {
        'content-type': 'text/plain; charset=utf-8',
        'content-disposition': 'attachment; filename="report.txt"'
      })
      response.end('下载内容')
      return
    }
    response.writeHead(200, { 'content-type': 'text/html; charset=utf-8' })
    response.end(
      '<html><head><title>下载页</title></head><body>' +
        '<a id="dl" href="/file" download="report.txt">下载报告</a></body></html>'
    )
  })
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', () => resolve()))
  const port = (server.address() as { port: number }).port
  const root = mkdtempSync(path.join(tmpdir(), 'browser-download-'))
  const call = (args: Record<string, unknown>) => runBrowser(root, args)
  try {
    const opened = await call({ action: 'open', url: `http://127.0.0.1:${port}/` })
    assert.equal(opened.ok, true, opened.text)
    // Clicking the link is what starts a download a person would get as a file.
    const clicked = await call({ action: 'eval', expression: "document.getElementById('dl').click()" })
    assert.equal(clicked.ok, true, clicked.text)
    assert.match(clicked.text, /刚下载了 report\.txt/, clicked.text)

    const dir = downloadsDir(root)
    const saved = path.join(dir, 'report.txt')
    assert.equal(existsSync(saved), true, readdirSync(dir).join(', ') + ' ' + clicked.text)
    assert.equal(readFileSync(saved, 'utf8'), '下载内容')

    // The result names a path the model can hand to `show`, and the folder list
    // agrees with it.
    assert.ok(clicked.text.includes(saved), clicked.text)
    const listed = await call({ action: 'downloads' })
    assert.match(listed.text, /report\.txt/)
    // A download is not a reason to keep the browser: `close` still resets it.
    assert.equal((await call({ action: 'close' })).ok, true)
    assert.equal(existsSync(saved), true, 'close 只收掉浏览器，下载的文件还在对话里')
  } finally {
    await call({ action: 'close' }).catch(() => undefined)
    server.close()
  }
})

test('the session pointer is written, read, and skipped when broken', () => {
  const dir = mkdtempSync(path.join(tmpdir(), 'browser-state-'))
  assert.equal(readSavedSession(dir), null)
  writeSavedSession(dir, {
    endpoint: 'ws://127.0.0.1:9/devtools/browser/x',
    profile: path.join(dir, 'profile'),
    pageTarget: 't1',
    lastUsedAt: 1
  })
  assert.deepEqual(readSavedSession(dir), {
    endpoint: 'ws://127.0.0.1:9/devtools/browser/x',
    profile: path.join(dir, 'profile'),
    pageTarget: 't1',
    lastUsedAt: 1
  })
  writeFileSync(path.join(dir, 'session.json'), '{not json', 'utf8')
  assert.equal(readSavedSession(dir), null)
  writeFileSync(path.join(dir, 'session.json'), '{"endpoint":"ws://x"}', 'utf8')
  assert.equal(readSavedSession(dir), null, '缺少 profile / lastUsedAt 的指针不算数')
})

test('the idle timeout is thirty minutes unless the environment says otherwise', () => {
  const before = process.env.DSH_BROWSER_IDLE_MIN
  try {
    delete process.env.DSH_BROWSER_IDLE_MIN
    assert.equal(browserIdleMs(), 30 * 60_000)
    process.env.DSH_BROWSER_IDLE_MIN = '5'
    assert.equal(browserIdleMs(), 5 * 60_000)
    process.env.DSH_BROWSER_IDLE_MIN = 'nonsense'
    assert.equal(browserIdleMs(), 30 * 60_000)
  } finally {
    if (before === undefined) delete process.env.DSH_BROWSER_IDLE_MIN
    else process.env.DSH_BROWSER_IDLE_MIN = before
  }
})

test('a browser killed mid-question is restarted and says the page is blank', { skip: e2eSkip }, async () => {
  const server = createServer((request, response) => {
    response.writeHead(200, { 'content-type': 'text/html; charset=utf-8' })
    response.end('<html><head><title>重启测试</title></head><body><h1>页面内容</h1></body></html>')
  })
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', () => resolve()))
  const port = (server.address() as { port: number }).port
  const root = mkdtempSync(path.join(tmpdir(), 'browser-restart-'))
  const call = (args: Record<string, unknown>) => runBrowser(root, args)
  const profiles = (): string[] => {
    const dir = path.join(root, '.dsh-flow', 'browser')
    try {
      // The folder also holds `session.json`; only profile folders count here.
      return readdirSync(dir, { withFileTypes: true })
        .filter((entry) => entry.isDirectory())
        .map((entry) => entry.name)
    } catch {
      return []
    }
  }
  try {
    const opened = await call({ action: 'open', url: `http://127.0.0.1:${port}/` })
    assert.equal(opened.ok, true, opened.text)
    assert.equal(profiles().length, 1, profiles().join(', '))

    // Kill the browser behind the tool's back: exactly what a crash or the Edge
    // updater does, and what chat-1 hit.
    killByProfile(path.join(root, '.dsh-flow', 'browser', profiles()[0]))
    await new Promise((resolve) => setTimeout(resolve, 1_500))

    const after = await call({ action: 'text' })
    assert.equal(after.ok, true, after.text)
    assert.match(after.text, /上一次的浏览器已经断开/, after.text)
    assert.match(after.text, /about:blank/)

    // The action itself still works once the page is opened again, and the dead
    // session's profile was swept instead of piling up.
    const reopened = await call({ action: 'open', url: `http://127.0.0.1:${port}/` })
    assert.equal(reopened.ok, true, reopened.text)
    assert.match(reopened.text, /页面内容/)
    assert.equal(profiles().length, 1, profiles().join(', '))
    assert.equal((await call({ action: 'close' })).ok, true)
    assert.deepEqual(profiles(), [])
  } finally {
    await call({ action: 'close' }).catch(() => undefined)
    server.close()
  }
})