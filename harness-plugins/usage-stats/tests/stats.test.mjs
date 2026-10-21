import { foldUsage, aggregate, sampleUsage } from '../stats.js'

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

// 1. usage from data.usage
check('usage from data.usage', sampleUsage({
  type: 'assistant/message',
  data: { usage: { inputTokens: 5, outputTokens: 7, cacheReadTokens: 3 } },
}), { inputTokens: 5, outputTokens: 7, cacheReadTokens: 3 })

// 2. usage from a stream chunk when data.usage is absent
check('usage from stream chunk', sampleUsage({
  type: 'assistant/attempt',
  data: {
    stream: [
      { type: 'chunk', chunk: { type: 'text-delta' } },
      { type: 'chunk', chunk: { type: 'usage', usage: { inputTokens: 1, outputTokens: 2 } } },
    ],
  },
}), { inputTokens: 1, outputTokens: 2 })

// 3. no usage
check('no usage', sampleUsage({ type: 'assistant/message', data: {} }), undefined)

const now = Date.UTC(2026, 9, 20, 12, 0, 0)
const day = 86_400_000

const events = [
  { type: 'request/header', seq: 0, time: now - day * 2, data: { header: { config: { provider: 'deepseek', model: 'deepseek-v4' } }, reason: 'initial' } },
  { type: 'assistant/message', seq: 1, time: now - day * 2, data: { turn: 1, step: 1, usage: { inputTokens: 100, outputTokens: 40, cacheReadTokens: 900, cacheWriteTokens: 0 } } },
  // same turn/step re-settles: replaces, not adds
  { type: 'assistant/message', seq: 2, time: now - day * 2, data: { turn: 1, step: 1, usage: { inputTokens: 100, outputTokens: 50, cacheReadTokens: 900 } } },
  { type: 'request/header', seq: 3, time: now - day, data: { header: { config: { provider: 'pi-ai', model: 'gpt-5' } }, reason: 'change' } },
  { type: 'assistant/message', seq: 4, time: now - day, data: { turn: 2, step: 1, usage: { inputTokens: 200, outputTokens: 100, cacheReadTokens: 300, cacheWriteTokens: 50 } } },
  // retry restarts the replacement scope: a second billed attempt in the same step
  { type: 'llm/retry-started', seq: 5, time: now - day, data: { turn: 2, step: 2 } },
  { type: 'assistant/message', seq: 6, time: now, data: { turn: 2, step: 2, usage: { inputTokens: 10, outputTokens: 5 } } },
  { type: 'assistant/message', seq: 7, time: now, data: { turn: 2, step: 2, usage: { inputTokens: 20, outputTokens: 15 } } },
  // no usage at all: ignored
  { type: 'assistant/message', seq: 8, time: now, data: { turn: 3, step: 1 } },
]

const folded = foldUsage(events)
check('contribution count', folded.length, 3)
check('first contribution', folded[0].buckets, { uncachedInput: 100, cacheRead: 900, cacheWrite: 0, output: 50 })
check('first provider', folded[0].provider, 'deepseek')
check('second provider/model', [folded[1].provider, folded[1].model], ['pi-ai', 'gpt-5'])
check('retry replaces within step', folded[2].buckets, { uncachedInput: 20, cacheRead: 0, cacheWrite: 0, output: 15 })

const view = aggregate(folded, { days: 30, tzOffsetMinutes: 0, now })
check('totals', view.totals, {
  uncachedInput: 320, cacheRead: 1200, cacheWrite: 50, output: 165,
  prompt: 1570, total: 1735, calls: 3, hitRate: 1200 / 1570,
})
check('provider groups', view.providers.map((row) => [row.provider, row.total]), [['deepseek', 1050], ['pi-ai', 685]])
check('model rows', view.models.map((row) => `${row.provider}/${row.model}`), ['deepseek/deepseek-v4', 'pi-ai/gpt-5'])
check('daily series length', view.series.length, 30)
check('daily series tail', view.series[29].total, 35)

// A one-day window still includes the contribution stamped exactly at the cutoff.
const narrowed = aggregate(folded, { days: 1, tzOffsetMinutes: 0, now })
check('range filter keeps boundary day', narrowed.totals.total, 685)

// Shifting "now" a day later drops everything but the newest contribution.
const later = aggregate(folded, { days: 1, tzOffsetMinutes: 0, now: now + day })
check('range filter drops older days', later.totals.total, 35)

const all = aggregate(folded, { days: 0, tzOffsetMinutes: 0, now })
check('all-time series', all.series.length, 3)

const empty = aggregate([], { days: 7, now })
check('empty hit rate', empty.totals.hitRate, null)
check('empty series', empty.series.length, 7)

// ── hourly buckets ──────────────────────────────────────────────────────────
// The hourly axis runs from the hour containing the cutoff through the current
// hour inclusive, so a 2-day window yields 48 + 1 buckets.
const hourlyRows = aggregate(folded, { days: 2, bucket: 'hour', tzOffsetMinutes: 0, now })
check('hourly bucket declared', hourlyRows.range.bucket, 'hour')
check('hourly series length', hourlyRows.series.length, 49)
check('hourly keys are ISO hour stamps', /^\d{4}-\d{2}-\d{2}T\d{2}$/.test(hourlyRows.series[48].key), true)
check('hourly totals unchanged', hourlyRows.totals.total, 1735)
// The newest contribution lands in the last hour bucket.
check('hourly tail carries the newest usage', hourlyRows.series[48].total, 35)
// The day-old contribution lands exactly 24 buckets earlier.
check('hourly places the previous day', hourlyRows.series[24].total, 650)

// A window with no usage still yields a gap-free axis.
const quiet = aggregate([], { days: 1, bucket: 'hour', now })
check('quiet hourly axis is dense', quiet.series.length, 25)
check('quiet hourly axis is empty', quiet.series.every((row) => row.total === 0), true)

console.log(failures === 0 ? 'ALL PASS' : `${failures} FAILURE(S)`)
process.exit(failures === 0 ? 0 : 1)
