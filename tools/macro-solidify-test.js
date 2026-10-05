const assert = require('node:assert/strict');
const { prepare, compare, read } = require('../frontend/macro-solidify.js');
const action = { sequence_index: 2, name: '盾刀·三段', skill_id: 13044, cast_time: 4.25,
  triggered: false, rage_after: 30, solidify: { skill_id: 13044, waits: [.5, 1.25], fcast: false },
  state_after: { rage: 30, skill_cds: [{ name: '盾压', remaining: 1 }, { name: '盾飞', remaining: 2 }] } };
const source = { timeline: [action], fight_time: 5, total_damage: 10, rage: 30, stance: 'shield' };
const body = { sequence: ['__切体态延迟中__', '盾挡', '__macro__', '__macro__'], timing_offsets: { 2: 33 }, channel_ticks: {} };
const original = JSON.stringify(body);
const prepared = prepare(source, body);
assert.equal(JSON.stringify(body), original);
assert.equal(prepared.body.sequence[2], '盾刀');
assert.equal(prepared.body.sequence[3], '__macro__');
assert.equal(prepared.unready, 1);
assert.equal(prepared.body.timing_offsets[2], undefined);
assert.deepEqual(prepared.body.solidified_casts[2], action.solidify);
const reordered = structuredClone(source);
reordered.timeline[0].state_after.skill_cds.reverse();
assert.equal(compare(source, reordered), null);
for (const change of [s => s.timeline.pop(), s => s.timeline[0].cast_time += .001,
  s => s.timeline[0].name = '盾击', s => s.timeline[0].rage_after++, s => s.fight_time++]) {
  const result = structuredClone(source); change(result); assert.ok(compare(source, result));
}
assert.throws(() => prepare({ timeline: [{ ...action, solidify: null }] }, body));
const item = (dataset, pre = false) => ({ dataset, classList: { contains: () => pre } });
assert.deepEqual(read([item({}, true), item({}), item({ solidifiedCast: JSON.stringify(action.solidify) })]), { 1: action.solidify });
assert.deepEqual(read([item({ solidifiedCast: JSON.stringify(action.solidify), timingOffset: '.3' })]), {});
console.log('macro solidification preparation, exact verification and editable waits passed');
const workspace = require('../frontend/harness-workspace.js');
const loop = workspace.toLoop({}, { sequence: ['盾刀'], solidified_casts: { 0: action.solidify } });
assert.deepEqual(loop.sequence[0].solidified_cast, action.solidify);
const exact = require('../frontend/macro-exact.js');
assert.equal(exact.sceneParameters({ solidified_casts: { 0: action.solidify }, haste_level: 1 }).solidified_casts, undefined);
assert.deepEqual(exact.previewRequest({ simulation: { solidified_casts: { 0: action.solidify } }, horizon: 20 }, '/cast 盾刀').solidified_casts, {});
