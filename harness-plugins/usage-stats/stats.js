/**
 * Pure usage folds shared by the plugin's Host half and its tests.
 *
 * The only durable source of truth is the Session log: every billed model call
 * settles as `assistant/message` (or `assistant/attempt` before a retry) with a
 * provider-reported `TokenUsage` and the model identity that produced it. This
 * module turns those settlements into per-session contribution records and then
 * into the page's aggregate view. It never touches a Context, the clock, or the
 * network, so it is deterministic and directly testable.
 */

/** One day in milliseconds. */
export const DAY_MS = 86_400_000

/** A phone-book cap on days accepted from a client. */
const MAX_DAYS = 3660

/** A cap on rendered days for the unbounded range. */
const MAX_DAILY_BUCKETS = 120

/** A cap on rendered hours, so an hourly request cannot serialise a huge series. */
const MAX_HOURLY_BUCKETS = 24 * 31

/** Hour in milliseconds. */
const HOUR_MS = 3_600_000

const emptyCounts = () => ({
  uncachedInput: 0,
  cacheRead: 0,
  cacheWrite: 0,
  output: 0,
  calls: 0,
})

/**
 * Normalize a provider counter to a finite non-negative integer.
 * @param value - any provider-supplied number.
 * @returns the count, or 0 for absent/negative/NaN values.
 */
function count(value) {
  const number = typeof value === 'number' ? value : Number(value)
  return Number.isFinite(number) && number > 0 ? number : 0
}

/**
 * Read the usage sample one Assistant settlement reports.
 *
 * Mirrors the token-meter contract: an explicit `data.usage` wins, otherwise the
 * last `usage` stream chunk carried by the settlement is used (an in-step retry
 * keeps its earlier sample in the stream).
 * @param event - one durable session event.
 * @returns the provider usage object, or undefined when none was reported.
 */
export function sampleUsage(event) {
  const data = event === null || event === undefined ? undefined : event.data
  if (data === null || typeof data !== 'object') return undefined
  if (data.usage !== null && typeof data.usage === 'object') return data.usage
  const stream = data.stream
  if (!Array.isArray(stream)) return undefined
  for (let index = stream.length - 1; index >= 0; index -= 1) {
    const record = stream[index]
    if (record === null || typeof record !== 'object') continue
    if (record.type !== 'chunk') continue
    const chunk = record.chunk
    if (chunk === null || typeof chunk !== 'object' || chunk.type !== 'usage') continue
    if (chunk.usage !== null && typeof chunk.usage === 'object') return chunk.usage
  }
  return undefined
}

/**
 * Split one provider usage sample into the four buckets the page reports.
 *
 * `inputTokens` is uncached input (the provider contract in this Harness), so
 * the prompt side is `uncachedInput + cacheRead + cacheWrite`.
 * @param usage - provider usage sample.
 * @returns the four counter buckets.
 */
export function bucketsOf(usage) {
  return {
    uncachedInput: count(usage.inputTokens),
    cacheRead: count(usage.cacheReadTokens),
    cacheWrite: count(usage.cacheWriteTokens),
    output: count(usage.outputTokens),
  }
}

/**
 * Fold one Session's own events into billed usage contributions.
 *
 * A repeated `turn:step` replaces its earlier sample in place, exactly like the
 * token-meter fold, so a streaming settlement followed by its final message is
 * counted once. `llm/retry-started` ends that replacement scope: the retry is a
 * separately billed attempt and therefore a separate contribution.
 * @param events - the session's own events (an inherited fork prefix excluded).
 * @returns contributions in log order.
 */
export function foldUsage(events) {
  const records = []
  const slots = new Map()
  let provider = 'unknown'
  let model = 'unknown'
  if (!Array.isArray(events)) return records
  for (const event of events) {
    if (event === null || typeof event !== 'object') continue
    if (event.type === 'request/header') {
      const config = event.data === undefined ? undefined : event.data.header === undefined ? undefined : event.data.header.config
      if (config !== null && typeof config === 'object') {
        if (typeof config.provider === 'string' && config.provider !== '') provider = config.provider
        if (typeof config.model === 'string' && config.model !== '') model = config.model
      }
      continue
    }
    if (event.type === 'llm/retry-started') {
      const data = event.data === undefined ? {} : event.data
      slots.delete(`${String(data.turn)}:${String(data.step)}`)
      continue
    }
    if (event.type !== 'assistant/message' && event.type !== 'assistant/attempt') continue
    const usage = sampleUsage(event)
    if (usage === undefined) continue
    const data = event.data === undefined ? {} : event.data
    const source = data.message === undefined ? undefined : data.message.source
    const slotKey = `${String(data.turn)}:${String(data.step)}`
    const nextProvider = source !== undefined && typeof source.provider === 'string' && source.provider !== '' ? source.provider : provider
    const nextModel = source !== undefined && typeof source.model === 'string' && source.model !== '' ? source.model : model
    const buckets = bucketsOf(usage)
    const existing = slots.get(slotKey)
    if (existing !== undefined) {
      existing.time = count(event.time)
      existing.provider = nextProvider
      existing.model = nextModel
      existing.buckets = buckets
      continue
    }
    const record = {
      time: count(event.time),
      provider: nextProvider,
      model: nextModel,
      buckets,
    }
    records.push(record)
    slots.set(slotKey, record)
  }
  return records
}

/**
 * Local calendar day key for one instant.
 * @param time - epoch milliseconds.
 * @param shift - `getTimezoneOffset() * 60_000` of the viewing browser.
 * @returns `YYYY-MM-DD` in that browser's zone.
 */
function dateKey(time, shift) {
  const shifted = new Date(time - shift)
  if (Number.isNaN(shifted.getTime())) return '1970-01-01'
  return shifted.toISOString().slice(0, 10)
}

/**
 * Local hour key for one instant: the `date` plus a zero-padded hour.
 * @param time - epoch milliseconds.
 * @param shift - `getTimezoneOffset() * 60_000` of the viewing browser.
 * @returns `{ date, hour, key }` where `key` is `YYYY-MM-DDTHH`.
 */
function hourKey(time, shift) {
  const shifted = new Date(time - shift)
  if (Number.isNaN(shifted.getTime())) return { date: '1970-01-01', hour: 0, key: '1970-01-01T00' }
  const iso = shifted.toISOString()
  const date = iso.slice(0, 10)
  const hour = Number(iso.slice(11, 13))
  return { date, hour, key: `${date}T${iso.slice(11, 13)}` }
}

/**
 * Turn raw counters into the reported figures.
 * @param counts - the mutable accumulator.
 * @returns prompt/total/hit-rate alongside the counters.
 */
function summarize(counts) {
  const prompt = counts.uncachedInput + counts.cacheRead + counts.cacheWrite
  const total = prompt + counts.output
  return {
    uncachedInput: counts.uncachedInput,
    cacheRead: counts.cacheRead,
    cacheWrite: counts.cacheWrite,
    output: counts.output,
    prompt,
    total,
    calls: counts.calls,
    hitRate: prompt > 0 ? counts.cacheRead / prompt : null,
  }
}

/**
 * Add one contribution to an accumulator.
 * @param target - accumulator to mutate.
 * @param buckets - contribution buckets.
 */
function accumulate(target, buckets) {
  target.uncachedInput += buckets.uncachedInput
  target.cacheRead += buckets.cacheRead
  target.cacheWrite += buckets.cacheWrite
  target.output += buckets.output
  target.calls += 1
}

/**
 * Read or create one grouped accumulator.
 * @param map - grouping map.
 * @param key - group identity.
 * @param create - seed factory for a missing group.
 * @returns the group's mutable accumulator.
 */
function groupOf(map, key, create) {
  let entry = map.get(key)
  if (entry === undefined) {
    entry = { ...create(), counts: emptyCounts() }
    map.set(key, entry)
  }
  return entry
}

/**
 * Aggregate contributions into the page's read model.
 * @param records - contributions from every scanned Session.
 * @param options - `days` (0 means the complete log), the viewing browser's
 *   `tzOffsetMinutes`, a `bucket` of `day` or `hour`, and an optional `now` for
 *   tests.
 * @returns the aggregate payload consumed by the Client page.
 */
export function aggregate(records, options = {}) {
  const now = Number.isFinite(options.now) ? options.now : Date.now()
  const requestedDays = Number.isFinite(options.days) ? Math.floor(options.days) : 0
  const days = requestedDays > 0 ? Math.min(requestedDays, MAX_DAYS) : 0
  const offset = Number.isFinite(options.tzOffsetMinutes) ? options.tzOffsetMinutes : 0
  const bucket = options.bucket === 'hour' ? 'hour' : 'day'
  const shift = offset * 60_000
  const cutoff = days > 0 ? now - days * DAY_MS : 0

  const totals = emptyCounts()
  const providers = new Map()
  const models = new Map()
  const buckets = new Map()

  for (const record of Array.isArray(records) ? records : []) {
    if (record === null || typeof record !== 'object' || record.buckets === undefined) continue
    const time = Number(record.time)
    const when = Number.isFinite(time) ? time : 0
    if (days > 0 && when < cutoff) continue
    const counts = record.buckets
    const provider = typeof record.provider === 'string' && record.provider !== '' ? record.provider : 'unknown'
    const model = typeof record.model === 'string' && record.model !== '' ? record.model : 'unknown'
    const key = bucket === 'hour' ? hourKey(when, shift).key : dateKey(when, shift)

    accumulate(totals, counts)
    accumulate(groupOf(providers, provider, () => ({ provider })).counts, counts)
    accumulate(groupOf(models, `${provider}\u0000${model}`, () => ({ provider, model })).counts, counts)
    accumulate(groupOf(buckets, key, () => ({ bucket: key })).counts, counts)
  }

  let keys
  if (bucket === 'hour') {
    // The hourly axis spans exactly the window the totals cover: from the hour
    // containing the cutoff through the current hour, so a contribution sitting
    // on the cutoff is representable instead of inflating the totals only.
    const from = days > 0 ? now - days * DAY_MS : now - (MAX_HOURLY_BUCKETS - 1) * HOUR_MS
    const first = Math.floor(from / HOUR_MS) * HOUR_MS
    const last = Math.floor(now / HOUR_MS) * HOUR_MS
    const stamps = new Set()
    for (let stamp = first; stamp <= last; stamp += HOUR_MS) {
      stamps.add(hourKey(stamp, shift).key)
      if (stamps.size >= MAX_HOURLY_BUCKETS) break
    }
    // A DST shift can repeat or skip an hour key; the Set de-duplicates and the
    // sort restores chronological order.
    keys = [...stamps].sort()
  } else if (days > 0) {
    keys = []
    for (let index = days - 1; index >= 0; index -= 1) keys.push(dateKey(now - index * DAY_MS, shift))
  } else {
    keys = [...buckets.keys()].sort()
    if (keys.length > MAX_DAILY_BUCKETS) keys = keys.slice(-MAX_DAILY_BUCKETS)
  }

  return {
    generatedAt: now,
    range: {
      days,
      bucket,
      from: keys.length > 0 ? keys[0] : null,
      to: keys.length > 0 ? keys[keys.length - 1] : null,
      tzOffsetMinutes: offset,
    },
    totals: summarize(totals),
    providers: [...providers.values()]
      .map((entry) => ({ provider: entry.provider, ...summarize(entry.counts) }))
      .sort((left, right) => right.total - left.total),
    models: [...models.values()]
      .map((entry) => ({ provider: entry.provider, model: entry.model, ...summarize(entry.counts) }))
      .sort((left, right) => right.total - left.total),
    series: keys.map((key) => {
      const entry = buckets.get(key)
      const counts = entry === undefined ? emptyCounts() : entry.counts
      return { key, ...summarize(counts) }
    }),
  }
}
