/**
 * Host half of the usage-stats bundle.
 *
 * Billing truth lives in the durable Session log: every settled model call is an
 * `assistant/message` (or a retried `assistant/attempt`) carrying a
 * provider-reported `TokenUsage`. This plugin folds those settlements once per
 * Session revision, caches the result, and serves one aggregate to the browser
 * over a private Connection RPC channel.
 */
import { aggregate, foldUsage } from './stats.js'
import { createRouteHandler } from './rpc-route.js'

/** Private Connection channel owned by this plugin. */
export const CHANNEL = '/usage-stats'

/** Parallel Session reads; the log store serializes per handle anyway. */
const READ_CONCURRENCY = 4

/** Services required before the Host half can answer. */
export const inject = ['sessionPersistence', 'connection', 'webServer']

/**
 * Read every event of one stored Session, excluding an inherited fork prefix.
 * @param persistence - session persistence service.
 * @param sessionId - stored Session id.
 * @returns the Session's own events.
 */
async function readOwnEvents(persistence, sessionId) {
  const handle = await persistence.open(sessionId, 'read')
  try {
    const result = await handle.read()
    const inherited = Number(handle.inheritedEventCount) || 0
    const events = Array.isArray(result.events) ? result.events : []
    return inherited > 0 ? events.slice(inherited) : events
  } finally {
    await handle.close()
  }
}

/**
 * Normalize one client request payload.
 * @param payload - raw RPC payload.
 * @returns validated options, or a rejection reason.
 */
function readOptions(payload) {
  const body = payload === null || typeof payload !== 'object' ? {} : payload
  const days = body.days === undefined ? 30 : body.days
  if (typeof days !== 'number' || !Number.isFinite(days) || days < 0 || days > 3660) {
    return { error: 'days must be a number between 0 and 3660' }
  }
  const offset = body.tzOffsetMinutes === undefined ? 0 : body.tzOffsetMinutes
  if (typeof offset !== 'number' || !Number.isFinite(offset) || Math.abs(offset) > 24 * 60) {
    return { error: 'tzOffsetMinutes must be the browser timezone offset in minutes' }
  }
  // A day-long window is only useful hourly; every longer window reads daily.
  const requested = body.bucket === undefined ? (days > 0 && days <= 2 ? 'hour' : 'day') : body.bucket
  if (requested !== 'hour' && requested !== 'day') {
    return { error: 'bucket must be "hour" or "day"' }
  }
  return { options: { days: Math.floor(days), tzOffsetMinutes: offset, bucket: requested } }
}

/**
 * Host plugin body: register the RPC channel and its per-Session fold cache.
 * @param ctx - Host plugin context.
 */
export function apply(ctx) {
  const persistence = ctx.sessionPersistence
  /** sessionId -> { revision, records } */
  const cache = new Map()
  let inFlight = null

  /**
   * Fold every stored Session, reusing unchanged revisions.
   * @returns contribution records per session plus the scanned session count.
   */
  async function scan() {
    const snapshots = await persistence.list()
    const records = []
    let sessions = 0
    const pending = []
    for (const snapshot of snapshots) {
      if (snapshot.formatStatus === 'migration-required') continue
      const id = String(snapshot.header.id)
      sessions += 1
      const revision = String(snapshot.revision)
      const cached = cache.get(id)
      if (cached !== undefined && cached.revision === revision) {
        records.push(...cached.records)
        continue
      }
      pending.push({ id, revision, snapshot })
    }
    let cursor = 0
    const worker = async () => {
      for (;;) {
        const index = cursor
        cursor += 1
        if (index >= pending.length) return
        const item = pending[index]
        try {
          const events = await readOwnEvents(persistence, item.snapshot.header.id)
          const folded = foldUsage(events)
          cache.set(item.id, { revision: item.revision, records: folded })
          records.push(...folded)
        } catch {
          // A single unreadable log must not blank the whole page.
          cache.set(item.id, { revision: item.revision, records: [] })
        }
      }
    }
    await Promise.all(Array.from({ length: Math.min(READ_CONCURRENCY, Math.max(1, pending.length)) }, worker))
    const live = new Set(snapshots.map((snapshot) => String(snapshot.header.id)))
    for (const id of [...cache.keys()]) if (!live.has(id)) cache.delete(id)
    return { records, sessions }
  }

  // The browser reaches this endpoint through `ctx.connection.rpc.call`, which
  // posts a `client-request` envelope to `<channel>/<endpoint>`. Connection's own
  // channel registry cannot be extended by another plugin — its `register()`
  // reads `webServer` from the Connection fiber — so this route speaks the same
  // wire format and reuses Connection's admission for the cookie and origin
  // fence. Registering on `webServer` directly keeps ownership on this fiber.
  ctx.effect(() => ctx.webServer.register({
    kind: 'prefix',
    path: CHANNEL,
    handler: createRouteHandler({
      channel: CHANNEL,
      requestRejection: (req) => ctx.connection.requestRejection(req),
      handle: async (endpoint, payload) => {
        if (endpoint !== 'summary') {
          return { ok: false, error: { code: 'usage-stats/unknown-endpoint', message: `unknown endpoint ${endpoint}`, details: {} } }
        }
        const parsed = readOptions(payload)
        if (parsed.error !== undefined) {
          return { ok: false, error: { code: 'usage-stats/bad-request', message: parsed.error, details: {} } }
        }
        if (inFlight === null) {
          inFlight = scan().finally(() => {
            inFlight = null
          })
        }
        const scanned = await inFlight
        return {
          ok: true,
          value: {
            ...aggregate(scanned.records, parsed.options),
            sessions: scanned.sessions,
          },
        }
      },
    }),
  }), 'usage-stats: RPC route')
}
