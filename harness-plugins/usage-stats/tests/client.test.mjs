/**
 * Client-half contract test: load the bundle the way the page's module loader
 * does, then drive `apply` against stub services. It proves the factory
 * contract, the two slot registrations, the RPC call shape, and that both
 * dictionaries carry the same keys — a defect the type system cannot catch in
 * a plain-JavaScript plugin.
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
globalThis.window = {
  __ModuleLoader__: {
    load(entry) {
      registration = entry
    },
  },
}

// Minimal React surface: the module only builds elements at render time, which
// this test never does — it asserts the plugin contract instead.
const created = []
const React = {
  createElement(type, props, ...children) {
    created.push(type)
    return { type, props, children }
  },
  useState: (initial) => [initial, () => {}],
  useEffect: () => {},
  useMemo: (factory) => factory(),
  useCallback: (fn) => fn,
  useRef: (value) => ({ current: value }),
}
const moduleTable = { react: React }

// eslint-disable-next-line no-new-func
new Function('window', source)(globalThis.window)

check('bundle id', registration.id, '@local/usage-stats')

const factory = registration.factory
const plugin = factory((specifier) => {
  if (specifier in moduleTable) return moduleTable[specifier]
  throw new Error(`unexpected module request ${specifier}`)
})

check('plugin inject', plugin.inject, ['slots', 'locale', 'connection'])

const registered = []
const dictionaries = []
const effects = []
const calls = []
const ctx = {
  effect(fn, label) {
    effects.push(label)
    return fn()
  },
  locale: {
    register(ns, dicts) {
      dictionaries.push([ns, dicts])
      return () => {}
    },
    bind: () => (key, params) => (params === undefined ? key : `${key}:${JSON.stringify(params)}`),
  },
  slots: {
    inject(name, callback) {
      registered.push(name)
      callback()
      return () => {}
    },
    register(options, component) {
      registered.registered = registered.registered || []
      registered.registered.push({ options, component })
      return () => {}
    },
  },
  connection: {
    rpc: {
      async call(channel, endpoint, payload) {
        calls.push([channel, endpoint, payload])
        return {
          ok: true,
          value: {
            generatedAt: 0,
            range: { days: 30, from: null, to: null, tzOffsetMinutes: 0 },
            totals: { uncachedInput: 0, cacheRead: 0, cacheWrite: 0, output: 0, prompt: 0, total: 0, calls: 0, hitRate: null },
            providers: [],
            models: [],
            daily: [],
            sessions: 0,
          },
        }
      },
    },
  },
}

plugin.apply(ctx)

check('slots injected', registered, ['main', 'sidebar.panellist'])
const entries = registered.registered
check('main entry', [entries[0].options.name, entries[0].options.key, entries[0].options.locale], ['main', 'usage-stats', 'usageStats'])
check('panel entry', [entries[1].options.name, entries[1].options.id, entries[1].options.order], ['sidebar.panellist', 'usage-stats', 20])
check('panel label is a thunk', typeof entries[1].options.label, 'function')
check('panel label resolves', entries[1].options.label(), 'panel')

const face = entries[0].options.inject()
check('inject face exposes load', typeof face.load, 'function')

const payload = await face.load({ days: 7, tzOffsetMinutes: -480 })
check('load called the channel', calls, [['/usage-stats', 'summary', { days: 7, tzOffsetMinutes: -480 }]])
check('load unwraps value', payload.sessions, 0)

check('dictionary namespaces', dictionaries.map(([ns]) => ns), ['usageStats'])
const dicts = dictionaries[0][1]
const enKeys = Object.keys(dicts.en).sort()
const zhKeys = Object.keys(dicts.zh).sort()
check('locale key parity', enKeys, zhKeys)
check('locale covers every t() key used by the panel', enKeys.filter((key) => key.startsWith('series.')), [
  'series.cacheRead', 'series.cacheWrite', 'series.output', 'series.uncached',
])
check('effect labelled', effects, ['usage-stats: dictionaries'])

console.log(failures === 0 ? 'ALL PASS' : `${failures} FAILURE(S)`)
process.exit(failures === 0 ? 0 : 1)
