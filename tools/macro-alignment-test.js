/* Run with: node tools/macro-alignment-test.js */
'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const path = require('node:path');
const { performance } = require('node:perf_hooks');
const { align, MAX_ACTIVE_EVENTS } = require('../frontend/macro-alignment.js');

let passed = 0;
function test(name, fn) {
  fn(); passed++;
  process.stdout.write(`PASS ${name}\n`);
}
function event(name, cast_time, state_before, extra = {}) {
  return { name, cast_time, triggered: false, state_before, ...extra };
}
function sequence(names) { return names.map((name, index) => event(name, index)); }
function pairs(result) {
  return result.rows.filter(row => row.referenceIndex !== null && row.actualIndex !== null)
    .map(row => [row.referenceIndex, row.actualIndex]);
}

test('browser global and CommonJS expose the same standalone API', () => {
  const sandbox = { window: {} };
  vm.runInNewContext(fs.readFileSync(path.join(__dirname, '../frontend/macro-alignment.js'), 'utf8'), sandbox);
  assert.equal(typeof sandbox.window.Jx3MacroAlignment.align, 'function');
  assert.equal(sandbox.window.Jx3MacroAlignment.align([], []).rows.length, 0);
});

test('inserted skill realigns the remaining repeated cycles', () => {
  const reference = sequence(['盾刀', '盾压', '盾刀', '盾压', '盾刀', '盾压']);
  const actual = [...reference]; actual.splice(2, 0, event('血怒', 1.5));
  const result = align(reference, actual);
  assert.deepEqual(pairs(result), [[0, 0], [1, 1], [2, 3], [3, 4], [4, 5], [5, 6]]);
  assert.deepEqual(result.summary, { missing: 0, extra: 1, changed: 0, firstDifference: 2 });
});

test('deleted skill leaves one gap without making subsequent skills red', () => {
  const reference = sequence(['盾刀', '盾压', '血怒', '盾刀', '盾压']);
  const actual = reference.filter((_, index) => index !== 2);
  const result = align(reference, actual);
  assert.equal(result.rows[2].kind, 'missing');
  assert.deepEqual(result.summary, { missing: 1, extra: 0, changed: 0, firstDifference: 2 });
  assert.deepEqual(pairs(result), [[0, 0], [1, 1], [3, 2], [4, 3]]);
});

test('leading insertion and deletion keep the whole suffix paired', () => {
  const reference = sequence(['盾刀', '盾压', '盾刀', '盾压']);
  const actual = [event('血怒', -1), ...reference];
  assert.equal(align(reference, actual).rows[0].kind, 'extra');
  assert.equal(align(actual, reference).rows[0].kind, 'missing');
  assert.deepEqual(pairs(align(reference, actual)), [[0, 1], [1, 2], [2, 3], [3, 4]]);
});

test('repeated full cycle uses time only to choose between equal-length alignments', () => {
  const reference = sequence(['盾刀', '盾压', '盾刀', '盾压', '盾刀', '盾压']);
  const result = align(reference, reference.slice(2));
  assert.deepEqual(pairs(result), [[2, 0], [3, 1], [4, 2], [5, 3]]);
  const farAway = reference.map(entry => ({ ...entry, cast_time: entry.cast_time + 1e7 }));
  assert.equal(pairs(align(reference, farAway)).length, reference.length);
});

test('same-time off-GCD skills stay in order; triggered and virtual events retain original indices', () => {
  const reference = [event('破·盾刀', 0, null, { triggered: true }), event('盾刀', 0),
    event('血怒', 0, null, { gcd: 0 }), event('移除气劲', 0, null, { skill_id: 90001 }),
    event('__clearCD__:盾压', 0), event('盾压', 1)];
  const actual = [event('失败释放', 0, null, { cast_success: false }), reference[1],
    event('虚拟名', 0, null, { skill_id: 90001 }), reference[2], reference[5]];
  const result = align(reference, actual);
  assert.deepEqual(pairs(result), [[1, 1], [2, 3], [5, 4]]);
  assert.deepEqual(result.summary, { missing: 0, extra: 0, changed: 0, firstDifference: null });
});

test('unknown resources and timestamps are not fabricated or treated as zero', () => {
  const result = align([event('盾刀', undefined, { rage: 0, block_value: null, berserk_value: 20 })],
    [event('盾刀', 2, { block_value: 0, berserk_value: NaN })]);
  assert.equal(result.rows[0].timeDelta, null);
  assert.equal(result.rows[0].kind, 'same');
  assert.deepEqual(result.rows[0].resourceDiffs, []);
});

test('resource and rage-tier changes pair the same base skill and preserve actual values', () => {
  const result = align([event('绝刀·50怒', 1, { rage: 85, berserk_value: 120, block_value: 10 })],
    [event('绝刀·30怒', 1.5, { rage: 35, berserk_value: 90, block_value: 10 })]);
  assert.deepEqual(pairs(result), [[0, 0]]);
  assert.equal(result.rows[0].kind, 'changed');
  assert.equal(result.rows[0].timeDelta, .5);
  assert.deepEqual(result.rows[0].resourceDiffs, [
    { field: 'rage', reference: 85, actual: 35 },
    { field: 'berserk_value', reference: 120, actual: 90 },
  ]);
  assert.equal(align([event('盾刀·1级', 0)], [event('盾刀·2级', 0)]).rows[0].kind, 'changed');
});

test('雾海 and the three combo skills keep separate identities', () => {
  for (const [name, id] of [['阵云结晦', 90010], ['月照连营', 90011], ['雁门迢递', 90012]]) {
    const normal = event(name, 0), variant = event(`${name}·雾海`, 0, null, { skill_id: id });
    assert.equal(pairs(align([normal], [variant])).length, 0);
    assert.equal(pairs(align([normal], [{ ...variant, skill_id: undefined }])).length, 0);
  }
  const result = align(sequence(['阵云结晦', '月照连营', '雁门迢递']), sequence(['雁门迢递', '月照连营', '阵云结晦']));
  assert.equal(pairs(result).length, 1);
  for (const row of result.rows) {
    if (row.referenceIndex !== null && row.actualIndex !== null) assert.equal(row.referenceIndex, 2 - row.actualIndex);
  }
});

test('one-frame tolerance preserves raw delta and supports an explicit threshold', () => {
  const before = [event('盾刀', 10)];
  const after = [event('盾刀', 10 + 1 / 16)];
  assert.equal(align(before, after).rows[0].kind, 'same');
  assert.equal(align(before, after).rows[0].timeDelta, 1 / 16);
  assert.equal(align(before, after, { timeTolerance: 0 }).rows[0].kind, 'changed');
  assert.equal(align(before, [event('盾刀', 9)]).rows[0].timeDelta, -1);
  assert.throws(() => align(before, after, { timeTolerance: -1 }), RangeError);
});

test('empty timelines, unrelated skills and missing timestamps remain deterministic', () => {
  assert.deepEqual(align([], []).summary, { missing: 0, extra: 0, changed: 0, firstDifference: null });
  assert.equal(align(sequence(['盾刀']), []).rows[0].kind, 'missing');
  assert.equal(align([], sequence(['盾刀'])).rows[0].kind, 'extra');
  assert.deepEqual(align(sequence(['盾刀']), sequence(['盾压'])).rows.map(row => row.kind), ['missing', 'extra']);
  const reference = [event('盾刀'), event('盾刀'), event('盾刀')];
  assert.deepEqual(pairs(align(reference, reference.slice(1))), [[0, 0], [1, 1]]);
});

test('maximum-size repeated alignment fits the computation budget and rejects overflow', () => {
  const names = ['盾刀', '盾刀', '盾压', '盾飞', '血怒', '斩刀', '绝刀', '盾回'];
  const reference = Array.from({ length: MAX_ACTIVE_EVENTS }, (_, index) => event(names[index % names.length], index / 16));
  const actual = reference.slice(); actual.splice(1024, 1);
  const started = performance.now();
  const result = align(reference, actual);
  const elapsed = performance.now() - started;
  assert.equal(result.summary.missing, 1);
  assert.equal(result.summary.extra, 0);
  assert.equal(result.summary.changed, 0);
  assert.equal(result.rows.length, MAX_ACTIVE_EVENTS);
  assert.ok(elapsed < 5000, `maximum alignment took ${elapsed.toFixed(1)} ms`);
  process.stdout.write(`  ${MAX_ACTIVE_EVENTS} × ${actual.length}: ${elapsed.toFixed(1)} ms\n`);
  assert.throws(() => align([...reference, event('盾刀', 999)], actual), error => error.code === 'MACRO_ALIGNMENT_LIMIT');
  assert.throws(() => align(actual, [...reference, event('盾刀', 999)]), error => error.code === 'MACRO_ALIGNMENT_LIMIT');
  // 限制主动技能数量，不因正常的被动伤害事件误报超限。
  assert.equal(align([...reference, ...reference.map(entry => ({ ...entry, triggered: true }))], reference).rows.length, MAX_ACTIVE_EVENTS);
});

process.stdout.write(`${passed} alignment checks passed.\n`);
