/**
 * Curve-geometry regression test for the trend chart.
 *
 * The plugin's bundle is a page module (`window.__ModuleLoader__.load`), so this
 * harness evaluates it with a stub loader and pulls `smoothPath` out of the
 * rendered SVG output instead of importing it — asserting on the real path data
 * the browser would draw.
 *
 * The bug this locks down: a Catmull-Rom spline overshoots at a local extremum,
 * so a lone spike in a run of zeros drew *below* the zero baseline. The chart
 * now uses monotone cubic interpolation, whose tangents are zero at extrema.
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

// ── pull the pure path builder out of the bundle ────────────────────────────
// The factory closes over its helpers, so the cheapest faithful route is to
// expose `smoothPath` by evaluating the module with a recording loader and then
// re-entering the same file with a one-line export shim appended.
let registration = null
globalThis.window = { __ModuleLoader__: { load: (entry) => { registration = entry } } }
new Function('window', source)(globalThis.window)

const shimmed = `${source.replace(
  'return {\n      inject:',
  'return {\n      __smoothPath: smoothPath,\n      __monotoneTangent: monotoneTangent,\n      inject:',
)}`
let shimRegistration = null
globalThis.window = { __ModuleLoader__: { load: (entry) => { shimRegistration = entry } } }
new Function('window', shimmed)(globalThis.window)
const shimPlugin = shimRegistration.factory(() => ({ useState: () => [null, () => {}] }))
check('bundle exposes the curve builders for testing', typeof shimPlugin.__smoothPath, 'function')
check('bundle exposes the tangent rule for testing', typeof shimPlugin.__monotoneTangent, 'function')

const smoothPath = shimPlugin.__smoothPath

/**
 * Sample a path of cubic Béziers at many t values, returning the drawn y range.
 * Cartesian y grows downward, so a curve that "dips below the baseline" has a
 * larger y than the baseline.
 * @param {string} path - `M`/`C` path data.
 * @returns {{ min: number, max: number, samples: number }} drawn extent.
 */
function extentOf(path) {
  const tokens = path.match(/[MC]|-?\d+(?:\.\d+)?/g) ?? []
  let index = 0
  let current = null
  let min = Infinity
  let max = -Infinity
  let samples = 0
  while (index < tokens.length) {
    const command = tokens[index]
    index += 1
    if (command === 'M') {
      current = { x: Number(tokens[index]), y: Number(tokens[index + 1]) }
      index += 2
      min = Math.min(min, current.y)
      max = Math.max(max, current.y)
      samples += 1
      continue
    }
    const c1 = { x: Number(tokens[index]), y: Number(tokens[index + 1]) }
    const c2 = { x: Number(tokens[index + 2]), y: Number(tokens[index + 3]) }
    const to = { x: Number(tokens[index + 4]), y: Number(tokens[index + 5]) }
    index += 6
    for (let step = 1; step <= 64; step += 1) {
      const t = step / 64
      const u = 1 - t
      const y = u * u * u * current.y + 3 * u * u * t * c1.y + 3 * u * t * t * c2.y + t * t * t * to.y
      min = Math.min(min, y)
      max = Math.max(max, y)
      samples += 1
    }
    current = to
  }
  return { min, max, samples }
}

// ── the reported bug: a spike inside a run of zeros ─────────────────────────
// Baseline sits at y = 200; the spike rises to y = 20. Nothing may exceed 200.
const spike = [
  { x: 0, y: 200 }, { x: 10, y: 200 }, { x: 20, y: 200 },
  { x: 30, y: 20 },
  { x: 40, y: 200 }, { x: 50, y: 200 }, { x: 60, y: 200 },
]
const spikeExtent = extentOf(smoothPath(spike))
check('a spike never dips below its zero baseline', spikeExtent.max <= 200.0001, true)
check('a spike still reaches its peak', spikeExtent.min <= 20.0001, true)

// Two spikes, matching the screenshot's double bump.
const doubleSpike = [
  { x: 0, y: 200 }, { x: 10, y: 200 },
  { x: 20, y: 40 },
  { x: 30, y: 200 },
  { x: 40, y: 200 },
  { x: 50, y: 10 },
  { x: 60, y: 200 },
]
const doubleExtent = extentOf(smoothPath(doubleSpike))
check('two spikes stay inside the baseline', doubleExtent.max <= 200.0001, true)

// A descending ramp must not overshoot past its endpoints either.
const ramp = [{ x: 0, y: 0 }, { x: 10, y: 100 }, { x: 20, y: 200 }, { x: 30, y: 300 }]
const rampExtent = extentOf(smoothPath(ramp))
check('a monotone ramp stays inside its endpoints', rampExtent.min >= -0.0001 && rampExtent.max <= 300.0001, true)

// Flat data draws a flat line.
const flatExtent = extentOf(smoothPath([{ x: 0, y: 120 }, { x: 50, y: 120 }, { x: 100, y: 120 }]))
check('flat data stays flat', [flatExtent.min, flatExtent.max], [120, 120])

// Degenerate inputs must not throw.
check('an empty series has no path', smoothPath([]), '')
check('a single point is a move command', smoothPath([{ x: 5, y: 7 }]).startsWith('M '), true)

// ── the tangent rule itself ─────────────────────────────────────────────────
// At a local maximum the slopes disagree, so the tangent must be exactly zero.
check('a local extremum gets a zero tangent', shimPlugin.__monotoneTangent(10, 10, 5, -5), 0)
// On a monotone run the tangent points the same way as the data.
check('a rising run gets a positive tangent', shimPlugin.__monotoneTangent(10, 10, 5, 5) > 0, true)
check('a falling run gets a negative tangent', shimPlugin.__monotoneTangent(10, 10, -5, -5) < 0, true)

console.log(failures === 0 ? 'ALL PASS' : `${failures} FAILURE(S)`)
process.exit(failures === 0 ? 0 : 1)
