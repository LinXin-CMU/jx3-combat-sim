/* Run with: node tools/harness-alignment-test.js. Shared Rust/JS conformance cases. */
'use strict';
const assert = require('node:assert/strict');
const cases = require('../backend/tests/harness/alignment-cases.json');
const { align, DEFAULT_TIME_TOLERANCE } = require('../frontend/macro-alignment.js');

for (const fixture of cases) {
  const windowed = events => events.map(event =>
    Number.isFinite(event.cast_time) && event.cast_time >= 0 && event.cast_time < fixture.window
      ? { skill_id: 1, triggered: false, ...event } : null);
  const result = align(windowed(fixture.reference), windowed(fixture.actual), {
    timeTolerance: fixture.time_tolerance ?? DEFAULT_TIME_TOLERANCE,
  });
  const observed = {
    pairs: result.rows.filter(row => row.referenceIndex !== null && row.actualIndex !== null)
      .map(row => [row.referenceIndex, row.actualIndex]),
    missing: result.summary.missing,
    extra: result.summary.extra,
    changed: result.summary.changed,
    first_difference: result.summary.firstDifference,
    time_error: result.rows.reduce((sum, row) => sum + Math.abs(row.timeDelta ?? 0), 0),
  };
  if (Object.hasOwn(fixture.expected, 'resource_fields')) {
    observed.resource_fields = result.rows.flatMap(row => row.resourceDiffs.map(diff => diff.field));
  }
  assert.deepEqual(observed, fixture.expected, fixture.name);
  process.stdout.write(`PASS ${fixture.name}\n`);
}
process.stdout.write(`${cases.length} shared harness alignment fixtures passed.\n`);
