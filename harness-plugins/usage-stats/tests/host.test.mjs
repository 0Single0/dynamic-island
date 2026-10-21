/**
 * Host-half integration test: drive `apply` with a stub Cordis context so the
 * route registration, wire envelopes, admission, the per-revision cache, and
 * the response payload are exercised without a live Harness.
 */
import { apply, CHANNEL, inject } from '../index.js'

let failures = 0
const check = (name, actual, expected) => {
  const a = JSON.stringify(actual)
  const e = JSON.stringify(expected)
  if (a !== e) {
    failures += 1
    console.log(`FAIL ${name}\n  actual   ${a}\n  expected ${e}`)
  } else {
    console.log(`ok   ${name}`)
  }
}

const now = Date.now()
const day = 86_400_000

/** Build one fake stored Session snapshot. */
function session(id, revision) {
  return { header: { id, version: 4, createdAt: now - day, isSeeded: false }, revision }
}

const logs = new Map([
  ['session-a', [
    { type: 'request/header', seq: 0, time: now - day * 2, data: { header: { config: { provider: 'fwq', model: 'deepseek-v4.1-flash' } }, reason: 'initial' } },
    { type: 'assistant/message', seq: 1, time: now - day * 2, data: { turn: 1, step: 1, usage: { inputTokens: 1000, outputTokens: 200, cacheReadTokens: 8000, cacheWriteTokens: 100 } } },
  ]],
  ['session-b', [
    { type: 'request/header', seq: 0, time: now - day, data: { header: { config: { provider: 'wb', model: 'glm-5.3-flash' } }, reason: 'initial' } },
    { type: 'assistant/message', seq: 1, time: now - day, data: { turn: 1, step: 1, usage: { inputTokens: 500, outputTokens: 100, cacheReadTokens: 1000 } } },
  ]],
])

let openCount = 0
const snapshots = [
  session('session-a', 'rev-1'),
  session('session-b', 'rev-1'),
  { header: { id: 'session-old', version: 4, createdAt: 0, isSeeded: false }, revision: 'rev-1', formatStatus: 'migration-required' },
]

const persistence = {
  async list() {
    return snapshots
  },
  async open(id) {
    openCount += 1
    return {
      inheritedEventCount: 0,
      async read() {
        const events = logs.get(String(id))
        if (events === undefined) throw new Error('missing log')
        return { eventState: 'detached', events }
      },
      async close() {},
    }
  },
}

const routes = []
let rejectWith
const ctx = {
  sessionPersistence: persistence,
  connection: {
    requestRejection: () => rejectWith,
  },
  webServer: {
    register(route) {
      routes.push(route)
      return () => {}
    },
  },
  effect(fn) {
    return fn()
  },
}

apply(ctx)

check('declares every service it reads', inject, ['sessionPersistence', 'connection', 'webServer'])
check('one prefix route', routes.map((route) => [route.kind, route.path]), [['prefix', CHANNEL]])

const route = routes[0]
const handler = route.handler

/** Build a minimal node request object the route handler can consume. */
async function call(options = {}) {
  const body = options.body === undefined
    ? JSON.stringify({ type: 'client-request', rpcId: 'rpc-1', method: options.method ?? 'summary', payload: options.payload ?? {} })
    : options.body
  const chunks = [Buffer.from(body)]
  const req = {
    method: options.httpMethod ?? 'POST',
    url: options.url ?? `${CHANNEL}/${options.method ?? 'summary'}`,
    headers: { 'content-type': options.contentType ?? 'application/json' },
    async *[Symbol.asyncIterator]() {
      for (const chunk of chunks) yield chunk
    },
  }
  const captured = { status: 200, headers: {}, body: '' }
  const res = {
    writeHead(status, headers) {
      captured.status = status
      captured.headers = headers ?? {}
      return res
    },
    end(text) {
      captured.body = text === undefined ? '' : String(text)
    },
  }
  await handler(req, res)
  let json
  try {
    json = JSON.parse(captured.body)
  } catch {
    json = undefined
  }
  return { ...captured, json }
}

const first = await call({ payload: { days: 30, tzOffsetMinutes: 0 } })
check('HTTP 200', first.status, 200)
check('success envelope', [first.json.type, first.json.rpcId, first.json.result.ok], ['server-response', 'rpc-1', true])
check('scanned sessions', first.json.result.value.sessions, 2)
check('totals', first.json.result.value.totals, {
  uncachedInput: 1500, cacheRead: 9000, cacheWrite: 100, output: 300,
  prompt: 10600, total: 10900, calls: 2, hitRate: 9000 / 10600,
})
check('provider order', first.json.result.value.providers.map((row) => row.provider), ['fwq', 'wb'])
check('model rows', first.json.result.value.models.map((row) => row.model), ['deepseek-v4.1-flash', 'glm-5.3-flash'])
check('migration-required skipped', first.json.result.value.providers.some((row) => row.provider === 'unknown'), false)
check('json content type', first.headers['content-type'], 'application/json; charset=utf-8')
check('defaults to daily buckets', first.json.result.value.range.bucket, 'day')
check('daily series is dense', first.json.result.value.series.length, 30)

// A day-long window switches the default granularity to hourly.
const hourly = await call({ payload: { days: 1, tzOffsetMinutes: 0 } })
check('a one-day window is hourly', hourly.json.result.value.range.bucket, 'hour')
check('an explicit bucket wins', (await call({ payload: { days: 30, bucket: 'hour' } })).json.result.value.range.bucket, 'hour')
check('rejects an unknown bucket', (await call({ payload: { bucket: 'week' } })).json.result.error.code, 'usage-stats/bad-request')

const opensAfterFirst = openCount
const second = await call({ payload: { days: 30, tzOffsetMinutes: 0 } })
check('cache reuse (no re-read)', openCount - opensAfterFirst, 0)
check('cache reuse totals', second.json.result.value.totals.total, 10900)

// A revision bump re-reads only that Session.
logs.set('session-b', [
  ...logs.get('session-b'),
  { type: 'assistant/message', seq: 2, time: now, data: { turn: 2, step: 1, usage: { inputTokens: 10, outputTokens: 5 } } },
])
snapshots[1] = session('session-b', 'rev-2')
const before = openCount
const third = await call({ payload: { days: 30, tzOffsetMinutes: 0 } })
check('revision bump re-reads one', openCount - before, 1)
check('revision bump totals', third.json.result.value.totals.total, 10915)

check('bad days rejected', (await call({ payload: { days: -1 } })).json.result.error.code, 'usage-stats/bad-request')
check('bad offset rejected', (await call({ payload: { tzOffsetMinutes: 9999 } })).json.result.error.code, 'usage-stats/bad-request')
check('unknown endpoint', (await call({ method: 'nope', url: `${CHANNEL}/nope` })).json.result.error.code, 'usage-stats/unknown-endpoint')
check('method mismatch', (await call({ method: 'summary', url: `${CHANNEL}/other` })).json.result.error.code, 'gateway/bad-request')

// Transport-level refusals.
rejectWith = 401
check('unauthenticated', (await call({ payload: {} })).status, 401)
rejectWith = 403
check('untrusted origin', (await call({ payload: {} })).status, 403)
rejectWith = undefined
check('GET refused', (await call({ httpMethod: 'GET' })).status, 405)
check('wrong content type', (await call({ contentType: 'text/plain' })).status, 415)
check('invalid JSON', (await call({ body: '{' })).status, 400)
check('invalid envelope', (await call({ body: JSON.stringify({ type: 'x' }) })).status, 400)

// A single unreadable log must not fail the page.
snapshots.length = 0
snapshots.push(session('broken', 'rev-1'))
const resilient = await call({ payload: { days: 7, tzOffsetMinutes: 0 } })
check('unreadable log tolerated', resilient.json.result.ok, true)
check('unreadable log empty', resilient.json.result.value.totals.total, 0)

// ── the same route over a real HTTP server ─────────────────────────────────
// Proves the wire contract end to end: real request parsing, chunked bodies,
// the JSON envelope, and the response the browser's `rpc.call` parses.
const { createServer } = await import('node:http')
const server = createServer(handler)
await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve))
const { port } = server.address()
const base = `http://127.0.0.1:${port}`

snapshots.length = 0
snapshots.push(session('session-a', 'rev-1'))
const live = await fetch(`${base}${CHANNEL}/summary`, {
  method: 'POST',
  headers: { 'content-type': 'application/json' },
  body: JSON.stringify({ type: 'client-request', rpcId: 'rpc-live', method: 'summary', payload: { days: 30, tzOffsetMinutes: 0 } }),
})
const liveBody = await live.json()
check('http status', live.status, 200)
check('http content type', live.headers.get('content-type'), 'application/json; charset=utf-8')
check('http envelope', [liveBody.type, liveBody.rpcId, liveBody.result.ok], ['server-response', 'rpc-live', true])
check('http totals', liveBody.result.value.totals.total, 9300)
check('http provider', liveBody.result.value.providers[0].provider, 'fwq')

const liveGet = await fetch(`${base}${CHANNEL}/summary`)
check('http GET refused', liveGet.status, 405)

await new Promise((resolve) => server.close(resolve))

console.log(failures === 0 ? 'ALL PASS' : `${failures} FAILURE(S)`)
process.exit(failures === 0 ? 0 : 1)
