/**
 * Run every suite in this directory: `node tests/all.mjs`.
 *
 * Each suite is a standalone Node script that exits non-zero on failure, so this
 * runner only aggregates their outcomes.
 */
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { readdirSync } from 'node:fs'
import { dirname, join } from 'node:path'

const here = dirname(fileURLToPath(import.meta.url))
const suites = readdirSync(here)
  .filter((name) => name.endsWith('.test.mjs'))
  .sort()

let failed = 0
for (const suite of suites) {
  const result = spawnSync(process.execPath, [join(here, suite)], { stdio: 'inherit' })
  if (result.status !== 0) {
    failed += 1
    console.log(`FAILED ${suite}`)
  }
}

console.log(failed === 0 ? `\nALL SUITES PASS (${suites.length})` : `\n${failed} SUITE(S) FAILED`)
process.exit(failed === 0 ? 0 : 1)
