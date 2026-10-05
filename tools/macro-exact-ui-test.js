const test = require('node:test');
const assert = require('node:assert/strict');
const ui = require('../frontend/macro-exact.js');

test('automatic compression retains its stage across replay, pause, and completion', () => {
  const compression = { initial_chars: 400, best_chars: 220, trial_count: 13, status: 'running' };
  const job = { done: false, stage: 'compression', phase: 'replaying', compression };
  assert.match(ui.phaseText(job), /第二阶段 · 压缩/);
  assert.match(ui.phaseText({ ...job, phase: 'paused' }), /已暂停/);
  assert.match(ui.compressionText(job), /400 → 220 · 减少 180 · 试算 13 次/);
  assert.match(ui.phaseText({ ...job, done: true, result: { report: { compression_stop: 'cancelled' } } }), /保留已验证宏/);
  assert.match(ui.phaseText({ ...job, done: true, compression: { ...compression, status: 'scope_exhausted' } }), /本轮压缩完成/);
  assert.match(ui.phaseText({ done: false, phase: 'solving' }), /第一阶段/);
});

test('progress-only snapshots preserve macro text, clear stale comparisons, and isolate jobs', () => {
  const before = { id: 'a', revision: 1, best: { macro: '/cast A', macro_revision: 'aaa' },
    candidate: { macro: '/cast B', macro_revision: 'bbb', comparison: { reproduced: true } } };
  const after = ui.mergeSnapshot(before, { id: 'a', revision: 2, candidate: { macro_revision: 'bbb', comparison: null } });
  assert.equal(after.candidate.macro, '/cast B');
  assert.equal(after.candidate.comparison, null);
  assert.equal(after.best.macro, '/cast A');
  const query = new URLSearchParams(ui.snapshotQuery(after));
  assert.equal(query.get('best_macro'), 'aaa');
  assert.equal(query.get('candidate_macro'), 'bbb');
  assert.equal(query.get('job_id'), 'a');
  assert.equal(new URLSearchParams(ui.snapshotQuery(after, false)).has('revision'), false);
  assert.equal(ui.mergeSnapshot(after, { id: 'b', candidate: { macro: '/cast C' } }).best, undefined);
  assert.equal(new URLSearchParams(ui.snapshotQuery({ id: 'a', best: { macro_revision: 'aaa' } })).has('best_macro'), false);
  assert.equal(ui.mergeSnapshot(after, { id: 'a', candidate: null }).candidate, null);
});

test('a new untested macro is shown immediately without borrowing the best candidate certificate', () => {
  const best = { macro: '/cast A', comparison: { reproduced: true } };
  const candidate = { macro: '/cast B', iteration: 2, comparison: null };
  assert.equal(ui.currentCandidate({ best, candidate }), candidate);
  assert.equal(ui.currentCandidate({ best, candidate, phase: 'paused' }), candidate);
  assert.equal(ui.currentCandidate({ best, candidate }).comparison, null);
  assert.equal(ui.currentCandidate({ best }), best);
  assert.equal(ui.displayCandidate({ best, candidate }), best);
  assert.equal(ui.displayCandidate({ best, candidate }, 'current'), candidate);
  assert.equal(ui.displayCandidate({ best, candidate, done: true }), best);
});

test('preparation failure identifies the real divergence instead of suggesting an active solve', () => {
  const report = { status: 'semantic_mismatch', preparation_details: { probe_failure: { index: 6, kind: 'state_mismatch', expected: { name: '血怒', time: 3.875 } } } };
  assert.match(ui.preparationText(report), /第 7 次「血怒」@ 3.9 秒/);
  assert.match(ui.preparationText(report), /尚未进入求解/);
  assert.match(ui.preparationText(report), /状态不同/);
});

test('sub-decimal errors must not look like exact matches when rounded for display', () => {
  const comparison = { order_prefix: 23, target_count: 23, actual_count: 23, exact_prefix: 5, state_prefix: 5, reproduced: false, max_time_error_on_order_prefix: 0.0125, first_difference: { index: 5, expected: { name: '盾飞', time: 5 }, actual: { name: '盾飞', time: 5.0125 } } };
  assert.match(ui.comparisonText(comparison), /12.5ms/);
  assert.match(ui.comparisonText(comparison), /时序与状态 5\/23/);
  assert.match(ui.differenceText(comparison), /第 6 次/);
  assert.doesNotMatch(ui.differenceText(comparison), /均通过/);
});

test('matching the entire prefix with one extra cast remains a divergence', () => {
  const c = { reproduced: false, first_difference: { index: 23, expected: null, actual: { name: '盾回', time: 24 } } };
  assert.match(ui.differenceText(c), /目标：无施放/);
  assert.match(ui.differenceText(c), /候选：盾回 @ 24.0 秒/);
});

test('active-cast acceptance separates state diagnostics from timing progress', () => {
  const c = { acceptance:'skills_and_time', order_prefix:325, target_count:325, actual_count:325,
    exact_prefix:325, state_prefix:46, reproduced:true, state_reproduced:false,
    max_time_error_on_order_prefix:0.0065, time_tolerance_seconds:0.125 };
  assert.match(ui.comparisonText(c), /时序 325\/325/);
  assert.match(ui.comparisonText(c), /状态快照诊断 46\/325/);
  assert.equal(ui.differenceText(c), '');
});

test('runtime freezes through pause and resume, including legacy wall-clock snapshots', () => {
  let now = 1000;
  const clock = ui.runClock(() => now);
  const job = { id: 'a', elapsed_ms: 4000, phase: 'solving', done: false, elapsed_excludes_pauses: true };
  clock.update(job); now += 200;
  assert.equal(clock.value(), 4200);
  clock.freeze(); clock.update({ ...job, elapsed_ms: 4200, pause_requested: true, phase: 'paused' });
  now += 10000; assert.equal(clock.value(), 4200);
  clock.update({ ...job, elapsed_ms: 4200 }); now += 300; assert.equal(clock.value(), 4500);
  clock.update({ ...job, elapsed_ms: 4500, done: true }); now += 10000; assert.equal(clock.value(), 4500);
  const legacy = ui.runClock(() => now);
  legacy.update({ id: 'old', elapsed_ms: 3000 }); now += 100; legacy.freeze();
  legacy.update({ id: 'old', elapsed_ms: 9000, pause_requested: true });
  now += 10000; assert.equal(legacy.value(), 3100);
  legacy.update({ id: 'old', elapsed_ms: 19000, phase: 'resuming' }); now += 100;
  assert.equal(legacy.value(), 3200);
});

test('macro diff aligns insertions and highlights only the modified condition', () => {
  const diff = ui.macroDiff('/cast [rage>30] 盾刀\n/cast 盾飞\n/cast 斩刀', '/cast [rage>40] 盾刀\n/cast 血怒\n/cast 盾飞\n/cast 斩刀');
  assert.equal(diff.changed, 1); assert.equal(diff.added, 1); assert.equal(diff.removed, 0);
  assert.equal(diff.lines[0].text.slice(diff.lines[0].start, diff.lines[0].end), '4');
  assert.equal(diff.lines[2].changed, false); assert.equal(diff.lines[3].changed, false);
  assert.equal(ui.macroDiff('A\nB\nC', 'A\nC').removed, 1);
  assert.equal(ui.macroDiff('A\nB\nA', 'A\nA').removed, 1);
  const long = Array.from({ length: 2000 }, (_, i) => `/cast 技能${i}`).join('\n');
  assert.equal(ui.macroDiff(long, long + '\n/cast 新增').added, 1);
});
