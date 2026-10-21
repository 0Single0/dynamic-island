/**
 * Render smoke test for the panel body.
 *
 * It runs the real component tree against a tiny hook runtime that keeps hook
 * state per component instance, so an interaction (hovering the trend chart)
 * survives a re-render and its tooltip can be asserted. Output is a plain
 * element tree the assertions can read as text or as SVG nodes.
 */
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, join } from 'node:path'

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

const here = dirname(fileURLToPath(import.meta.url))
const source = readFileSync(join(here, '..', 'client.js'), 'utf8')

let registration = null
globalThis.window = { __ModuleLoader__: { load: (entry) => { registration = entry } } }

// ── hook storage, keyed by component identity ───────────────────────────────
/** `${componentName}#${ordinal}` → hook values. Persists across render passes. */
const hookStore = new Map()
let ordinals = new Map()
let currentHooks = null
let cursor = 0
let dirty = false
let pendingEffects = []

const resolveInitial = (value) => (typeof value === 'function' ? value() : value)

const React = {
  createElement(type, props, ...children) {
    return { type, props: props === null || props === undefined ? {} : props, children: children.flat(Infinity) }
  },
  useState(initial) {
    const index = cursor
    cursor += 1
    const hooks = currentHooks
    if (hooks.length <= index) hooks.push(resolveInitial(initial))
    const set = (next) => {
      const value = typeof next === 'function' ? next(hooks[index]) : next
      if (Object.is(value, hooks[index])) return
      hooks[index] = value
      dirty = true
    }
    return [hooks[index], set]
  },
  useEffect(effect) {
    pendingEffects.push(effect)
    return undefined
  },
  useMemo(factory) {
    return factory()
  },
  useCallback(fn) {
    return fn
  },
  useRef(value) {
    const index = cursor
    cursor += 1
    const hooks = currentHooks
    if (hooks.length <= index) hooks.push({ current: value })
    return hooks[index]
  },
}

/** Render one element into a tree of plain elements, running function components. */
function render(node) {
  if (node === null || node === undefined || typeof node === 'boolean') return null
  if (typeof node !== 'object') return node
  if (Array.isArray(node)) return node.map(render)
  if (typeof node.type !== 'function') {
    return { ...node, children: (node.children ?? []).map(render).filter((child) => child !== null) }
  }
  const name = node.type.name === '' ? 'anonymous' : node.type.name
  const ordinal = ordinals.get(name) ?? 0
  ordinals.set(name, ordinal + 1)
  const key = `${name}#${ordinal}`
  if (!hookStore.has(key)) hookStore.set(key, [])
  const previousHooks = currentHooks
  const previousCursor = cursor
  currentHooks = hookStore.get(key)
  cursor = 0
  let output
  try {
    output = node.type({ ...node.props, children: node.children })
  } finally {
    currentHooks = previousHooks
    cursor = previousCursor
  }
  return render(output)
}

/** Render until no state setter fires, flushing effects in between. */
async function renderSettled(element) {
  let tree = null
  for (let pass = 0; pass < 12; pass += 1) {
    ordinals = new Map()
    dirty = false
    pendingEffects = []
    tree = render(element)
    for (const effect of pendingEffects) effect()
    for (let spin = 0; spin < 3; spin += 1) {
      await Promise.resolve()
      await new Promise((resolve) => setTimeout(resolve, 0))
    }
    if (!dirty && pass >= 2) break
  }
  return tree
}

/** Re-render the existing tree after an interaction, keeping hook state. */
async function rerender(element) {
  ordinals = new Map()
  dirty = false
  pendingEffects = []
  const tree = render(element)
  for (const effect of pendingEffects) effect()
  await Promise.resolve()
  return tree
}

/** Collect every string/number in a rendered tree. */
function textOf(node, out = []) {
  if (node === null || node === undefined || typeof node === 'boolean') return out
  if (typeof node === 'string' || typeof node === 'number') {
    out.push(String(node))
    return out
  }
  if (Array.isArray(node)) {
    for (const child of node) textOf(child, out)
    return out
  }
  if (typeof node !== 'object') return out
  if (node.children !== undefined) for (const child of node.children) textOf(child, out)
  return out
}

/** Collect every element of one tag from a rendered tree. */
function leaves(node, type, out = []) {
  if (node === null || node === undefined || typeof node !== 'object') return out
  if (Array.isArray(node)) {
    for (const child of node) leaves(child, type, out)
    return out
  }
  if (node.type === type) out.push(node)
  if (node.children !== undefined) for (const child of node.children) leaves(child, type, out)
  return out
}

/** Find the first element satisfying a predicate. */
function find(node, predicate) {
  if (node === null || node === undefined || typeof node !== 'object') return undefined
  if (Array.isArray(node)) {
    for (const child of node) {
      const hit = find(child, predicate)
      if (hit !== undefined) return hit
    }
    return undefined
  }
  if (predicate(node)) return node
  for (const child of node.children ?? []) {
    const hit = find(child, predicate)
    if (hit !== undefined) return hit
  }
  return undefined
}

new Function('window', source)(globalThis.window)
const plugin = registration.factory((specifier) => {
  if (specifier === 'react') return React
  throw new Error(`unexpected module request ${specifier}`)
})

// ── slot context ─────────────────────────────────────────────────────────────
const SERIES_COLORS = ['#10B981', '#3B82F6', '#6366F1', '#EC4899']

const dayRows = Array.from({ length: 30 }, (_, index) => {
  const day = 21 + index
  const key = day <= 30
    ? `2026-09-${String(day).padStart(2, '0')}`
    : `2026-10-${String(day - 30).padStart(2, '0')}`
  return {
    key,
    uncachedInput: index * 1000,
    cacheRead: index * 9000,
    cacheWrite: index * 100,
    output: index * 2000,
    prompt: index * 10100,
    total: index * 12100,
    calls: index,
    hitRate: 0.89,
  }
})

const dayPayload = {
  generatedAt: Date.UTC(2026, 9, 20, 4, 30, 0),
  range: { days: 30, bucket: 'day', from: '2026-09-21', to: '2026-10-20', tzOffsetMinutes: -480 },
  totals: {
    uncachedInput: 1_240_000, cacheRead: 9_800_000, cacheWrite: 90_000, output: 360_000,
    prompt: 11_130_000, total: 11_490_000, calls: 412, hitRate: 9_800_000 / 11_130_000,
  },
  providers: [
    { provider: 'fwq', uncachedInput: 1_000_000, cacheRead: 8_000_000, cacheWrite: 80_000, output: 300_000, prompt: 9_080_000, total: 9_380_000, calls: 300, hitRate: 8_000_000 / 9_080_000 },
    { provider: 'wb', uncachedInput: 240_000, cacheRead: 1_800_000, cacheWrite: 10_000, output: 60_000, prompt: 2_050_000, total: 2_110_000, calls: 112, hitRate: 1_800_000 / 2_050_000 },
  ],
  models: [
    { provider: 'fwq', model: 'deepseek-v4.1-flash', uncachedInput: 1_000_000, cacheRead: 8_000_000, cacheWrite: 80_000, output: 300_000, prompt: 9_080_000, total: 9_380_000, calls: 300, hitRate: 8_000_000 / 9_080_000 },
    { provider: 'wb', model: 'glm-5.3-flash', uncachedInput: 240_000, cacheRead: 1_800_000, cacheWrite: 10_000, output: 60_000, prompt: 2_050_000, total: 2_110_000, calls: 112, hitRate: 1_800_000 / 2_050_000 },
  ],
  series: dayRows,
  sessions: 27,
}

const registered = []
const ctx = {
  effect(fn) {
    return fn()
  },
  locale: {
    register: () => () => {},
    bind: () => (key, params) => (params === undefined ? key : `${key}:${JSON.stringify(params)}`),
  },
  slots: {
    inject: (name, callback) => {
      callback()
      return () => {}
    },
    register: (options, component) => {
      registered.push({ options, component })
      return () => {}
    },
  },
  connection: {
    rpc: {
      async call() {
        return { ok: true, value: dayPayload }
      },
    },
  },
}

plugin.apply(ctx)
const page = registered.find((entry) => entry.options.name === 'main')
check('page component is a function', typeof page.component, 'function')

const t = (key, params) => (params === undefined ? key : `${key}:${JSON.stringify(params)}`)
const element = { type: page.component, props: { t, ...page.options.inject() }, children: [] }

// ── settled render ──────────────────────────────────────────────────────────
const tree = await renderSettled(element)
const text = textOf(tree).join(' ')
const strings = textOf(tree)

check('renders the title', strings.includes('title'), true)
check('renders every KPI tile', ['metric.total', 'metric.input', 'metric.output', 'metric.cacheRead', 'metric.cacheWrite', 'metric.hitRate'].every((key) => strings.includes(key)), true)
check('formats the total compactly', strings.includes('11.5M'), true)
check('formats the hit rate as a percent', strings.includes('88.1%'), true)
check('renders every chart heading', ['chart.channel', 'chart.trend', 'chart.models'].every((key) => strings.includes(key)), true)
check('renders channel labels', ['fwq', 'wb'].every((name) => strings.includes(name)), true)
check('renders model labels', ['deepseek-v4.1-flash', 'glm-5.3-flash'].every((name) => strings.includes(name)), true)
check('renders a donut arc per channel', leaves(tree, 'circle').length, 3)
check('renders a header row and one row per model', leaves(tree, 'div').filter((node) => typeof node.props.style?.gridTemplateColumns === 'string' && node.props.style.gridTemplateColumns.includes('minmax(140px')).length, 3)
check('no NaN in the rendered text', /NaN|undefined|\[object Object\]/.test(text), false)

// ── the trend chart ─────────────────────────────────────────────────────────
const seriesPaths = leaves(tree, 'path').filter((node) => SERIES_COLORS.includes(node.props.stroke))
const areaPaths = leaves(tree, 'path').filter((node) => typeof node.props.fill === 'string' && node.props.fill.startsWith('url('))

check('one line per series', seriesPaths.length, 4)
check('one gradient area per series', areaPaths.length, 4)
check('every line is a cubic curve', seriesPaths.every((node) => node.props.d.startsWith('M ') && node.props.d.includes('C ')), true)
check('every area closes its shape', areaPaths.every((node) => node.props.d.trimEnd().endsWith('Z')), true)
check('line strokes use the series colours', seriesPaths.map((node) => node.props.stroke).sort(), [...SERIES_COLORS].sort())
check('gradients are declared', leaves(tree, 'linearGradient').length, 4)
check('the total line is painted last (on top)', leaves(tree, 'path').filter((node) => SERIES_COLORS.includes(node.props.stroke)).at(-1).props.stroke, '#10B981')

const yTicks = leaves(tree, 'text').filter((node) => node.props.textAnchor === 'end').map((node) => node.children[0])
check('y-axis ticks are round numbers from zero', yTicks, ['0', '100K', '200K', '300K', '400K'])

const xTicks = leaves(tree, 'text').filter((node) => node.props.textAnchor === 'middle').map((node) => node.children[0])
check('x-axis is bucketed and thinned to ~6 labels', xTicks.length, 7)
check('x-axis labels are month/day', xTicks.every((value) => /^\d{2}\/\d{2}$/.test(value)), true)
check('legend names every series', ['trend.total', 'trend.cacheRead', 'trend.uncachedInput', 'trend.output'].every((key) => strings.includes(key)), true)
check('no crosshair before hovering', leaves(tree, 'line').filter((node) => node.props.strokeDasharray === '4 4').length, 0)

// Hovering the plot must reveal the crosshair, markers, and tooltip.
const plot = find(tree, (node) => node.type === 'svg' && typeof node.props.onPointerMove === 'function')
check('plot exposes a pointer surface', plot !== undefined, true)
const event = { clientX: 360, currentTarget: { getBoundingClientRect: () => ({ left: 0, width: 720 }) } }
plot.props.onPointerMove(event)

const hovered = await rerender(element)
const hoverStrings = textOf(hovered)
check('hovering draws a crosshair', leaves(hovered, 'line').filter((node) => node.props.strokeDasharray === '4 4').length, 1)
check('hovering marks one point per series', leaves(hovered, 'circle').filter((node) => node.props.r === 3.5).length, 4)
check('hovering opens a tooltip with the date heading', hoverStrings.some((value) => value.startsWith('trend.dateHeading')), true)
check('hovering lists every series in the tooltip', ['trend.total', 'trend.cacheRead', 'trend.uncachedInput', 'trend.output'].every((key) => hoverStrings.includes(key)), true)
check('tooltip values carry a unit', hoverStrings.filter((value) => value.endsWith('trend.unit')).length >= 4, true)
check('tooltip has no NaN', /NaN/.test(hoverStrings.join(' ')), false)

// Leaving the plot clears the tooltip again.
plot.props.onPointerLeave()
const left = await rerender(element)
check('leaving clears the crosshair', leaves(left, 'line').filter((node) => node.props.strokeDasharray === '4 4').length, 0)

// ── hourly granularity ──────────────────────────────────────────────────────
const hourlyPayload = {
  generatedAt: Date.UTC(2026, 9, 20, 4, 30, 0),
  range: { days: 1, bucket: 'hour', from: '2026-10-19T13', to: '2026-10-20T12', tzOffsetMinutes: -480 },
  totals: { uncachedInput: 1000, cacheRead: 5000, cacheWrite: 100, output: 400, prompt: 6100, total: 6500, calls: 4, hitRate: 5000 / 6100 },
  providers: [{ provider: 'fwq', uncachedInput: 1000, cacheRead: 5000, cacheWrite: 100, output: 400, prompt: 6100, total: 6500, calls: 4, hitRate: 5000 / 6100 }],
  models: [{ provider: 'fwq', model: 'deepseek-v4.1-flash', uncachedInput: 1000, cacheRead: 5000, cacheWrite: 100, output: 400, prompt: 6100, total: 6500, calls: 4, hitRate: 5000 / 6100 }],
  series: Array.from({ length: 24 }, (_, index) => ({
    key: `2026-10-19T${String(index).padStart(2, '0')}`,
    uncachedInput: index * 10, cacheRead: index * 50, cacheWrite: 1, output: index * 4,
    prompt: index * 61, total: index * 65, calls: index, hitRate: 0.82,
  })),
  sessions: 1,
}

hookStore.clear()
const hourlyCtx = { ...ctx, connection: { rpc: { async call() { return { ok: true, value: hourlyPayload } } } } }
registered.length = 0
plugin.apply(hourlyCtx)
const hourlyPage = registered.find((entry) => entry.options.name === 'main')
const hourlyElement = { type: hourlyPage.component, props: { t, ...hourlyPage.options.inject() }, children: [] }
const hourlyTree = await renderSettled(hourlyElement)
const hourlyText = textOf(hourlyTree)
check('hourly x-axis shows clock hours', hourlyText.includes('00:00') && hourlyText.includes('23:00'), true)
check('hourly chart draws one line per series', leaves(hourlyTree, 'path').filter((node) => SERIES_COLORS.includes(node.props.stroke)).length, 4)
check('hourly chart has no NaN', /NaN/.test(hourlyText.join(' ')), false)

// ── empty payload ───────────────────────────────────────────────────────────
hookStore.clear()
const emptyPayload = {
  generatedAt: 0,
  range: { days: 30, bucket: 'day', from: null, to: null, tzOffsetMinutes: 0 },
  totals: { uncachedInput: 0, cacheRead: 0, cacheWrite: 0, output: 0, prompt: 0, total: 0, calls: 0, hitRate: null },
  providers: [], models: [], series: [], sessions: 0,
}
const emptyCtx = { ...ctx, connection: { rpc: { async call() { return { ok: true, value: emptyPayload } } } } }
registered.length = 0
plugin.apply(emptyCtx)
const emptyPage = registered.find((entry) => entry.options.name === 'main')
const emptyElement = { type: emptyPage.component, props: { t, ...emptyPage.options.inject() }, children: [] }
const emptyTree = await renderSettled(emptyElement)
const emptyText = textOf(emptyTree)
check('empty state renders both lines', emptyText.includes('empty') && emptyText.includes('emptyHint'), true)
check('empty state shows a dash for the hit rate', emptyText.includes('—'), true)
check('empty state draws no series', leaves(emptyTree, 'path').filter((node) => SERIES_COLORS.includes(node.props.stroke)).length, 0)
check('empty state draws no donut', leaves(emptyTree, 'circle').length, 0)
check('empty state has no NaN', /NaN/.test(emptyText.join(' ')), false)

// ── a failed request ────────────────────────────────────────────────────────
hookStore.clear()
const failedCtx = { ...ctx, connection: { rpc: { async call() { return { ok: false, error: { code: 'usage-stats/failed', message: 'disk is busy' } } } } } }
registered.length = 0
plugin.apply(failedCtx)
const failedPage = registered.find((entry) => entry.options.name === 'main')
const failedTree = await renderSettled({ type: failedPage.component, props: { t, ...failedPage.options.inject() }, children: [] })
const failedText = textOf(failedTree)
check('a failure renders the error state', failedText.includes('error') && failedText.includes('disk is busy'), true)
check('a failure offers a retry', failedText.includes('retry'), true)
check('a failure has no NaN', /NaN/.test(failedText.join(' ')), false)

console.log(failures === 0 ? 'ALL PASS' : `${failures} FAILURE(S)`)
process.exit(failures === 0 ? 0 : 1)
