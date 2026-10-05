// Run with node --test tools/harness-ui-test.js. No browser, backend or userdata needed.
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const { captureScenario, macroPages, shouldAcceptStatus } = require('../frontend/harness.js');
const html = fs.readFileSync(path.join(__dirname, '../frontend/index.html'), 'utf8');
const source = fs.readFileSync(path.join(__dirname, '../frontend/harness.js'), 'utf8');

function fixture() {
  const body = { sequence: ['盾击', '盾压'], haste_level: 123, attributes: { base_attack: 321 }, target: { level: 54 },
    equipment: { HAT: 123, ENCHANT_HAT: 456 }, talents: [1], recipes: [2], channel_ticks: { 1: 2 },
    timing_offsets: { 0: 0.125 }, qijin_buffs: {}, network_delay: 80, initial_rage: 20,
    boss_attack_interval: 2, hanjia_expectation: true, dunya_reset_seed: 7, tiegu_mode: 1, experimental: false,
    team_buffs: [{ id: 1 }], formation: { id: 2 }, pre_releases: [{ skill: '血怒', time_before: 1 }], pauses: [[5, 1]] };
  let runs = 0;
  const result = { timeline: [{ name: '盾击', cast_time: 0 }], fight_time: 2, _macroAssistBody: body };
  const deps = { ready: async () => {}, identity: () => ({ version: 'AnYingQianJi', mount: 'FenShanJin' }),
    contextKey: () => 'fresh-context', sequence: () => body.sequence,
    simulate: async () => { runs++; return result; } };
  return { body, result, deps, runs: () => runs };
}

test('forces a fresh simulation and preserves its complete scene in an independent snapshot', async () => {
  const f = fixture();
  const capture = await captureScenario(f.deps);
  assert.equal(f.runs(), 1);
  assert.deepEqual(capture.simulation, { ...f.body, lite: false, lite_keep_timeline: false });
  f.body.equipment.HAT = 999;
  f.body.pre_releases[0].time_before = 12;
  assert.equal(capture.simulation.equipment.HAT, 123);
  assert.equal(capture.simulation.pre_releases[0].time_before, 1);
});

test('rejects empty and macro-driven axes before simulation', async () => {
  for (const sequence of [[], ['盾击', '__macro__']]) {
    const f = fixture(); f.deps.sequence = () => sequence;
    await assert.rejects(captureScenario(f.deps), /技能轴|宏块/);
    assert.equal(f.runs(), 0);
  }
});

test('rejects an edit or version switch while capture is running', async () => {
  const f = fixture(); let key = 'before'; f.deps.contextKey = () => key;
  f.deps.simulate = async () => { key = 'after'; return f.result; };
  await assert.rejects(captureScenario(f.deps), /发生变化/);
  const g = fixture(); let mount = 'FenShanJin'; g.deps.identity = () => ({ version: 'AnYingQianJi', mount });
  g.deps.simulate = async () => { mount = 'TieGuYi'; return g.result; };
  await assert.rejects(captureScenario(g.deps), /发生变化/);
});

test('does not accept stale global state when fresh simulation fails or lacks its request', async () => {
  const f = fixture(); f.deps.simulate = async () => null;
  await assert.rejects(captureScenario(f.deps), /未能完成/);
  f.deps.simulate = async () => ({ timeline: [] });
  await assert.rejects(captureScenario(f.deps), /缺少完整场景/);
});

test('rejects incomplete attributes or target and macro snapshots', async () => {
  for (const key of ['attributes', 'target']) {
    const f = fixture(); delete f.body[key];
    await assert.rejects(captureScenario(f.deps), /属性和目标/);
  }
  const f = fixture(); f.body.macro_text = '/cast 盾击';
  await assert.rejects(captureScenario(f.deps), /手动技能轴/);
});

test('splits stance pages without leaking page markers into game clipboard text', () => {
  const pages = macroPages('#page shield\r\n/cast 盾击\r\n#pageblade\r\n/cast 绝刀\r\n', [
    { stance: 'shield', chars: 8, limit: 128, within_limit: true },
    { stance: 'blade', chars: 8, limit: 128, within_limit: true },
  ]);
  assert.equal(pages.length, 2);
  assert.equal(pages[0].text, '/cast 盾击'); assert.equal(pages[1].text, '/cast 绝刀');
  assert.equal(pages[0].chars, 8);
  assert.equal(macroPages('/cast 盾击')[0].stance, 'general');
});

test('character fallback matches backend UTF-16 counting and preserves leading whitespace', () => {
  const text = '  // 🛡\n/cast 盾击';
  const page = macroPages(text)[0];
  assert.equal(page.text, text); assert.equal(page.chars, text.length);
  assert.equal(macroPages('x'.repeat(129))[0].within_limit, false);
});

test('late polling and SSE snapshots cannot overwrite a newer result or another job', () => {
  const current = { job_id: 'new', sequence: 9, status: 'completed', running: false };
  assert.equal(shouldAcceptStatus(current, { job_id: 'new', sequence: 8, status: 'running' }, 'new'), false);
  assert.equal(shouldAcceptStatus(current, { job_id: 'old', sequence: 50 }, 'new'), false);
  assert.equal(shouldAcceptStatus(current, { job_id: 'new', sequence: 9 }, 'new'), true);
  assert.equal(shouldAcceptStatus(current, { job_id: 'new', sequence: 10 }, 'new'), true);
});

test('all panel selectors, navigation entries and loader dependencies exist in the shipped page', () => {
  const ids = [...html.matchAll(/\bid="([^"]+)"/g)].map(match => match[1]);
  for (const match of source.matchAll(/\$\('([^']+)'\)/g)) {
    assert.equal(ids.filter(id => id === `harness_${match[1]}`).length, 1, match[1]);
  }
  for (const id of ['page-harness', 'page-sim', 'macro_assist_toggle', 'macro_draft_text']) assert.ok(ids.includes(id), id);
  assert.match(html, /data-quick-page="page-harness"/);
  assert.match(html, /data-page="page-harness"/);
  assert.match(html, /data-hub-goto="page-harness"/);
  assert.match(html, /data-harness-open/);
  const harnessCache = html.match(/harness\.css\?v=([^"\s]+)/)?.[1];
  assert.ok(harnessCache, 'harness stylesheet must have a cache version');
  assert.ok(html.includes(`'harness.js?v=${harnessCache}'`), 'harness script and stylesheet must share the release cache version');
  assert.ok(html.indexOf("'macro-assist.js?v=") < html.indexOf("'harness.js?v="));
  assert.match(fs.readFileSync(path.join(__dirname, '../frontend/app.js'), 'utf8'), /'page-harness': '◇ 武学助手'/);
});

// Exercise the real UI event handlers with only the DOM operations this panel uses.
// This checks data flow and interaction; it is deliberately not a visual-browser test.
class Element {
  constructor(tag = 'div') {
    this.tagName = tag; this.value = ''; this.dataset = {}; this.style = {}; this.children = [];
    this.listeners = new Map(); this.disabled = false; this.hidden = false; this._text = '';
    this.classes = new Set(); this.classList = { contains: name => this.classes.has(name) };
  }
  set textContent(value) { this._text = String(value); this.children = []; }
  get textContent() { return this._text + this.children.map(child => child.textContent).join(''); }
  get firstElementChild() { return this.children[0]; }
  append(...children) { this.children.push(...children); }
  replaceChildren(...children) { this._text = ''; this.children = children; }
  addEventListener(type, action) { const values = this.listeners.get(type) || []; values.push(action); this.listeners.set(type, values); }
  async emit(type) { for (const action of this.listeners.get(type) || []) await action({ target: this }); }
  click() { if (!this.disabled) return this.emit('click'); }
  remove() {}
  select() {}
}

function uiFixture(handler, options = {}) {
  const f = fixture(), elements = new Map([...html.matchAll(/\bid="([^"]+)"/g)].map(match => [match[1], new Element()]));
  const el = name => elements.get(`harness_${name}`), sources = [], copied = [], navigated = [], calls = [], timers = new Map(), blobs = [];
  let nextTimer = 1;
  el('budget').value = 'standard'; el('max_pages').value = '2';
  if (options.active) elements.get('page-harness').classes.add('active');
  const home = new Element('button'), body = new Element('body');
  const root = {
    document: { getElementById: id => elements.get(id) || null, createElement: tag => new Element(tag), body,
      querySelectorAll: selector => selector === '[data-harness-open]' ? [home] : [], execCommand: () => true },
    fetch: async (url, request = {}) => { calls.push({ url, request }); const result = await handler(url, request, calls); return { ok: (result?.httpStatus || 200) < 400, status: result?.httpStatus || 200, json: async () => result?.data ?? result }; },
    EventSource: class { constructor(url) { this.url = url; this.listeners = new Map(); this.closed = false; sources.push(this); } addEventListener(name, action) { this.listeners.set(name, action); } close() { this.closed = true; } emit(name, value) { this.listeners.get(name)?.({ data: JSON.stringify(value) }); } },
    Jx3Nav: { switchPage: id => navigated.push(id) }, Jx3MacroAssist: { contextKey: () => 'fresh-context', isActive: () => true },
    Jx3MacroLayout: { tab: () => {} }, navigator: { clipboard: { writeText: async text => copied.push(text) } },
    isSecureContext: true, addEventListener: () => {},
  };
  const context = { window: root, module: { exports: {} }, console, Promise, AbortSignal, Blob,
    URL: { createObjectURL: blob => { blobs.push(blob); return 'blob:test'; }, revokeObjectURL: () => {} },
    setTimeout: (callback, delay) => { const id = nextTimer++; timers.set(id, { callback, delay }); return id; }, clearTimeout: id => timers.delete(id),
    MutationObserver: class { observe() {} }, currentMountReady: Promise.resolve(), attributesReady: Promise.resolve(),
    currentMount: { version: 'AnYingQianJi', mount: 'FenShanJin', version_label: '暗影千机（2026.04）', mount_label: '分山劲' },
    readSequence: f.deps.sequence, runSimulate: f.deps.simulate,
  };
  vm.runInNewContext(source, context, { filename: 'harness.js' });
  return { ...f, el, root, home, elements, sources, copied, navigated, calls, timers, blobs, context };
}

function job(status = 'running', sequence = 1) {
  return { job_id: 'test-job', sequence, status, running: status === 'running', simulations: sequence, max_simulations: 96, elapsed_ms: 150,
    scenario_hash: 'scene-hash', experiment_hash: 'experiment-hash' };
}
function completed() {
  return { ...job('completed', 10), result: { stop_reason: 'target_reproduced', window_seconds: 2,
    baseline: { dps: 100, fight_time: 2, active_casts: 1 }, best: { verified: true, reproduced: true, full_snapshots: true,
      macro_text: '#page shield\n/cast 盾击\n#page blade\n/cast 绝刀', fingerprint: '01234567890123456789', page_constraints_passed: true,
      pages: [{ stance: 'shield', chars: 8, limit: 128, within_limit: true }, { stance: 'blade', chars: 8, limit: 128, within_limit: true }],
      alignment: { rows: [], summary: { missing: 0, extra: 0, changed: 0, first_difference: null } }, metrics: { dps: 105, fight_time: 2.01, active_casts: 1 } } } };
}
const settle = async () => { for (let i = 0; i < 5; i++) await new Promise(resolve => setImmediate(resolve)); };

test('real start handler refreshes the scene, omits an empty initial macro, and copies pages separately', async () => {
  let submitted;
  const ui = uiFixture((url, request) => {
    if (url === '/api/harness/jobs' && request.method === 'POST') { submitted = JSON.parse(request.body); return job(); }
    if (url === '/api/harness/jobs') return { jobs: [completed()] };
    return completed();
  });
  ui.body.dunya_reset_seed = 0;
  await ui.el('start').click(); await settle();
  assert.equal(ui.runs(), 1); assert.ok(submitted); assert.equal(Object.hasOwn(submitted, 'initial_macro'), false);
  assert.equal(submitted.simulation.dunya_reset_seed, 0); assert.deepEqual(submitted.simulation.pre_releases, ui.body.pre_releases);
  assert.equal(submitted.max_simulations, 96); assert.equal(submitted.max_pages, 2);
  assert.equal(ui.el('result').hidden, false); assert.match(ui.el('verdict').textContent, /动作与资源/);
  const firstCopy = ui.el('macro_pages').children[0].children[0].children[2];
  await firstCopy.click(); await ui.el('copy_all').click(); await settle();
  assert.equal(ui.copied[0], '/cast 盾击'); assert.match(ui.copied[1], /#page shield/);
  await ui.home.click(); await ui.el('go_sim').click(); await ui.el('open_editor').click();
  assert.deepEqual(ui.navigated, ['page-harness', 'page-sim', 'page-sim']);
  assert.equal(ui.el('start').disabled, false); assert.equal(ui.el('cancel').disabled, true);
});

test('real SSE and cancellation handlers preserve terminal results against late progress', async () => {
  const ui = uiFixture((url, request) => url.endsWith('/cancel') ? { accepted: true } : url === '/api/harness/jobs' && request.method !== 'POST' ? { jobs: [job()] } : job());
  await ui.el('start').click(); await settle();
  const stream = ui.sources[0]; assert.ok(stream);
  await ui.el('cancel').click();
  assert.ok(ui.calls.some(call => call.url.endsWith('/cancel') && call.request.method === 'POST'));
  stream.emit('progress', { ...job('running', 5), cancellation_requested: true });
  assert.equal(ui.el('cancel').disabled, true);
  stream.emit('completed', completed());
  stream.emit('progress', { ...job('running', 3), progress: { best: null } });
  assert.equal(ui.el('status').textContent, '验证结束'); assert.equal(ui.el('start').disabled, false);
  assert.equal(ui.el('result').hidden, false); assert.equal(stream.closed, true); assert.equal(ui.timers.size, 0);
});

test('restored jobs read their artifact identity and never cache a late running artifact as final', async () => {
  let releaseArtifact, artifactReads = 0;
  const oldRequest = { version: 'ShanHaiYuanLiu', mount: 'TieGuYi', initial_macro: '/cast 盾压', simulation: { sequence: ['盾压'], network_delay: 100 } };
  const ui = uiFixture(url => {
    if (url === '/api/harness/jobs') return { jobs: [job()] };
    if (url.endsWith('/artifacts')) {
      artifactReads++;
      if (artifactReads === 1) return new Promise(resolve => { releaseArtifact = resolve; });
      return { status: 'completed', request: oldRequest, result: completed().result };
    }
    return job();
  }, { active: true });
  await settle(); assert.equal(artifactReads, 1); assert.ok(ui.sources[0]);
  ui.sources[0].emit('completed', completed());
  releaseArtifact({ status: 'running', request: oldRequest, result: null });
  await settle();
  assert.match(ui.el('source_title').textContent, /铁骨衣.*山海源流/); assert.match(ui.el('source_detail').textContent, /与当前顶栏不同/);
  assert.equal(ui.el('before_macro').textContent, '/cast 盾压');
  await ui.el('download').click(); await settle();
  assert.equal(artifactReads, 2); assert.equal(ui.blobs.length, 1);
  const saved = JSON.parse(await ui.blobs[0].text()); assert.equal(saved.status, 'completed'); assert.ok(saved.result.best);
});

test('SSE failure falls back to polling and an expired job restores the start action', async () => {
  let gone = false;
  const ui = uiFixture((url, request) => {
    if (url === '/api/harness/jobs') return request.method === 'POST' ? job() : { jobs: [job()] };
    if (gone) return { httpStatus: 404, data: { error: { code: 'job_not_found', message: '任务不存在或已过期。' } } };
    return job();
  });
  await ui.el('start').click(); await settle();
  gone = true; ui.sources[0].onerror();
  const timer = [...ui.timers.values()][0]; assert.ok(timer); await timer.callback();
  assert.equal(ui.el('status').textContent, '任务已过期'); assert.equal(ui.el('start').disabled, false);
  assert.equal(ui.el('download').disabled, true); assert.equal(ui.timers.size, 0);
});

test('difference rendering uses original action indices, shows resource divergence and blocks overlong game pages', async () => {
  const finished = completed(), candidate = finished.result.best;
  candidate.reproduced = false; candidate.page_constraints_passed = false; candidate.macro_text = 'x'.repeat(129);
  candidate.pages = [{ stance: 'any', chars: 129, limit: 128, within_limit: false }];
  candidate.reference_actions = [{ index: 17, name: '盾压', cast_time: 2 }];
  candidate.actual_actions = [{ index: 21, name: '盾压', cast_time: 2.125 }];
  candidate.first_difference = { kind: 'changed', reference: candidate.reference_actions[0], actual: candidate.actual_actions[0] };
  candidate.alignment = { rows: [{ kind: 'changed', reference_index: 17, actual_index: 21, time_delta: .125, resource_diffs: [{ field: 'rage', reference: 30, actual: 35 }] }],
    summary: { first_difference: 0, missing: 0, extra: 0, changed: 1 } };
  const ui = uiFixture((url, request) => url === '/api/harness/jobs' && request.method === 'POST' ? job() : url === '/api/harness/jobs' ? { jobs: [finished] } : finished);
  await ui.el('start').click(); await settle();
  assert.match(ui.el('differences').textContent, /盾压.*2\.000s.*盾压.*2\.125s/);
  assert.match(ui.el('first_difference').textContent, /怒气：原轴 30 → 宏 35/);
  assert.equal(ui.el('macro_pages').children[0].children[0].children[2].disabled, true);
  await ui.el('copy_all').click(); await settle(); assert.equal(ui.copied[0], 'x'.repeat(129));
});
