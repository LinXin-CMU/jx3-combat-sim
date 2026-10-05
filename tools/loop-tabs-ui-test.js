/* Run against an isolated worker. Fixtures are intercepted, never written to real user data. */
const assert = require('node:assert/strict');
const { chromium } = require('playwright');

(async () => {
  const browser = await chromium.launch({ channel: 'msedge', headless: true });
  try {
    const page = await browser.newPage({ viewport: { width: 1600, height: 950 } });
    const errors = [], requests = [];
    page.on('pageerror', e => errors.push(e.message));
    page.on('request', r => { if (r.url().endsWith('/api/simulate')) requests.push(JSON.parse(r.postData())); });
    const macro = { mode: 'general', general: '/cast 盾压\n/cast 盾击\n/cast 盾飞\n/cast 斩刀\n/cast 绝刀\n/cast 盾回' };
    const fixture = { schema: 1, active: 'main', tabs: [
      { id: 'main', name: '主循环', sequence: [{ type: 'macro', count: 180 }], macro },
      { id: 'copy-a', name: '副页 A', compareTo: '', sequence: [{ type: 'macro', count: 181 }], macro },
      { id: 'copy-b', name: '副页 B', sequence: [{ type: 'macro', count: 179 }], macro },
    ] };
    let stored = fixture;
    await page.route('**/api/loop-tabs?*', route => {
      if (route.request().method() !== 'GET') stored = JSON.parse(route.request().postData());
      return route.fulfill({ json: route.request().method() === 'GET' ? stored : { ok: true } });
    });
    await page.goto(process.argv[2] || 'http://127.0.0.1:3005', { waitUntil: 'domcontentloaded' });
    await page.waitForFunction(() => window.Jx3LoopTabs?.ownsAutosave());
    await page.evaluate(() => Jx3Nav.switchPage('page-sim'));
    await page.waitForFunction(() => [...document.querySelectorAll('.ltab-pane')].every(p =>
      p.querySelector('[data-resolved-skill]') && !p.hasAttribute('aria-busy')));
    assert.equal(await page.evaluate(() => Jx3LoopTabs.activeId), 'main');
    for (const id of ['copy-a', 'copy-b']) {
      const pane = page.locator(`[data-loop-tab="${id}"]`);
      assert.equal(await pane.locator('select[aria-label="选择对比循环"]').inputValue(), 'main');
      assert.match(await pane.locator('.ltab-diff-summary').innerText(), /对比 主循环/);
      assert.equal(await pane.locator('.seq-macro:not([data-resolved-skill])').count(), 0);
    }
    assert.equal(await page.locator('.ltab-menu-content').getByText(/redline/i).count(), 0);
    const counts = () => requests.filter(r => [179, 180, 181].includes(r.sequence.length));
    assert.deepEqual(counts().map(r => r.sequence.length).sort(), [179, 180, 181]);
    const base = counts().find(r => r.sequence.length === 180);
    const environment = request => Object.fromEntries(Object.entries(request).filter(([key]) =>
      !['sequence', 'macro_text', 'macro_duration', 'channel_ticks', 'timing_offsets', 'solidified_casts', 'qijin_buffs', 'pre_releases'].includes(key)));
    for (const request of counts()) assert.deepEqual(environment(request), environment(base));
    await page.waitForTimeout(220); // Allow the deferred geometry pass to complete.

    // Sync includes all three visible panes, and disabling it applies to just that comparison.
    await page.evaluate(() => {
      const box = document.querySelector('[data-loop-tab="main"] .sim-sequence');
      box.scrollTop = box.scrollHeight;
    });
    await page.waitForFunction(() => [...document.querySelectorAll('.ltab-pane .sim-sequence')].every(box =>
      box.scrollHeight - box.clientHeight - box.scrollTop < 2));
    await page.evaluate(() => {
      const toggle = document.querySelector('[data-loop-tab="copy-b"] .ltab-sync-label input');
      toggle.checked = false; toggle.dispatchEvent(new Event('change', { bubbles: true }));
    });
    await page.waitForTimeout(220);
    await page.evaluate(() => { document.querySelector('[data-loop-tab="main"] .sim-sequence').scrollTop = 0; });
    await page.waitForFunction(() => document.querySelector('[data-loop-tab="copy-a"] .sim-sequence').scrollTop < 2);
    assert.ok(await page.locator('[data-loop-tab="copy-b"] .sim-sequence').evaluate(box => box.scrollTop > 100));
    const top = await page.locator('[data-loop-tab="copy-b"] .sim-sequence').evaluate(box => box.scrollTop);
    await page.locator('[data-loop-tab="copy-b"] .ltab-title').click();
    assert.equal(await page.evaluate(() => Jx3LoopTabs.activeId), 'copy-b');
    assert.ok(Math.abs(await page.locator('#sim_sequence').evaluate(box => box.scrollTop) - top) < 2);
    assert.equal(await page.locator('[data-loop-tab="main"] .sim-sequence').evaluate(box => box.scrollTop), 0);
    assert.equal(counts().length, 3, 'cached focus changes must not trigger more simulations');

    // The shared toolbar acts on the foreground only; menu operations do not steal focus.
    await page.locator('[data-loop-tab="copy-a"] summary').click();
    assert.equal(await page.evaluate(() => Jx3LoopTabs.activeId), 'copy-b');
    await page.locator('[data-loop-tab="copy-a"] select[aria-label="选择对比循环"]').selectOption('');
    assert.equal(await page.evaluate(() => Jx3LoopTabs.activeId), 'copy-b');
    await page.waitForTimeout(150);
    assert.equal(await page.locator('[data-loop-tab="copy-a"] .ltab-diff-summary').isVisible(), false);
    assert.equal(await page.locator('[data-loop-tab="copy-b"] .ltab-diff-summary').isVisible(), true);
    const saved = await page.evaluate(() => Jx3LoopTabs.snapshot());
    assert.equal(saved.tabs.find(t => t.id === 'copy-a').comparisonExplicit, true);
    assert.equal(saved.tabs.find(t => t.id === 'copy-b').syncScroll, false);

    // Resizing two adjacent pages must leave the third page's width unchanged.
    const panes = page.locator('.ltab-pane');
    const before = await panes.evaluateAll(nodes => nodes.map(n => n.getBoundingClientRect().width));
    const handle = page.locator('[data-loop-tab="copy-a"] .ltab-resizer');
    await handle.focus(); await handle.press('ArrowLeft');
    const after = await panes.evaluateAll(nodes => nodes.map(n => n.getBoundingClientRect().width));
    assert.ok(Math.abs(before[0] - after[0]) < 1);
    assert.ok(Math.abs(before[1] + before[2] - after[1] - after[2]) < 1);

    // Display settings refresh cached passive labels without network work.
    await page.evaluate(() => setSeqDisplayMode('text'));
    await page.waitForTimeout(150);
    assert.equal(counts().length, 3);
    await page.evaluate(async () => {
      Jx3LoopTabs.add(false);
      applyLoopConfig({ version: 1, sequence: [
        { type: 'pre_release', skill: '血怒', pre_time: 5 },
        { type: 'skill', skill: '盾舞', channel_ticks: 2 },
        { type: 'skill', skill: '盾击' }, { type: 'skill', skill: '盾压' },
      ] }, { skipSimulate: true });
      await runSimulate();
    });
    const manual = await page.evaluate(() => ({ config: buildLoopConfig(), request: buildSimulateRequest() }));
    assert.deepEqual(manual.request.channel_ticks, { 0: 2 });
    assert.equal(manual.config.sequence.find(e => e.type === 'pre_release').channel_ticks, undefined);
    assert.equal(manual.config.sequence.find(e => e.skill === '盾舞').channel_ticks, 2);
    await page.locator('#sim_sequence [data-skill="盾舞"]').dragTo(page.locator('#sim_sequence [data-skill="盾压"]'));
    await page.waitForFunction(() => readSequence()[2] === '盾舞');
    assert.deepEqual(await page.evaluate(() => buildSimulateRequest().channel_ticks), { 2: 2 });
    const copy = await page.evaluate(() => { Jx3LoopTabs.add(true); return Jx3LoopTabs.activeId; });
    assert.deepEqual(await page.evaluate(() => buildSimulateRequest().channel_ticks), { 2: 2 });
    await page.waitForFunction(() => lastSimResult && document.querySelector('#sim_sequence [data-macro-assist-index]'));
    assert.equal(await page.locator(`[data-loop-tab="${copy}"] select[aria-label="选择对比循环"]`).inputValue(), 'main');
    // The ordinary rectangle-selection handler still operates on the moved real editor.
    await page.evaluate(() => { document.getElementById('sim_sequence').scrollTop = 0; });
    const box = await page.locator('#sim_sequence').boundingBox();
    const first = await page.locator('#sim_sequence .sim-seq-item:not(.seq-pre-release):not(.seq-auto)').first().boundingBox();
    await page.mouse.move(box.x + 2, first.y - 2); await page.mouse.down();
    await page.mouse.move(first.x + first.width + 3, first.y + first.height + 3, { steps: 5 });
    await page.mouse.up();
    assert.ok(await page.locator('#sim_sequence .seq-selected').count() > 0);
    await page.keyboard.press('Escape');
    assert.equal(await page.locator('#sim_sequence .seq-selected').count(), 0);
    await page.evaluate(() => Jx3LoopTabs.save());
    await page.reload({ waitUntil: 'domcontentloaded' });
    await page.waitForFunction(id => window.Jx3LoopTabs?.ownsAutosave() && Jx3LoopTabs.activeId === id, copy);
    await page.evaluate(() => Jx3Nav.switchPage('page-sim'));
    assert.deepEqual(await page.evaluate(() => buildSimulateRequest().channel_ticks), { 2: 2 });
    assert.equal(await page.locator('[data-loop-tab="copy-a"] select[aria-label="选择对比循环"]').inputValue(), '');
    assert.equal(await page.locator('[data-loop-tab="copy-b"] .ltab-sync-label input').isChecked(), false);
    await page.locator(`[data-loop-tab="${copy}"] summary`).click();
    await page.locator(`[data-loop-tab="${copy}"] .ltab-menu-actions`).getByRole('button', { name: '隐藏', exact: true }).click();
    assert.equal(await page.evaluate(() => Jx3LoopTabs.activeId), 'main');
    assert.equal(await page.locator(`[data-loop-tab="${copy}"]`).isVisible(), false);
    await page.locator('[data-loop-tab="main"] summary').click();
    await page.locator('[data-loop-tab="main"] select[aria-label="交换页面"]').selectOption(copy);
    assert.equal(await page.evaluate(() => Jx3LoopTabs.activeId), copy);
    assert.deepEqual(await page.evaluate(() => buildSimulateRequest().channel_ticks), { 2: 2 });
    await page.locator(`[data-loop-tab="${copy}"] summary`).click();
    const name = page.locator(`[data-loop-tab="${copy}"] input[aria-label="循环页名称"]`);
    await name.fill('验证副页'); await name.press('Enter');
    assert.equal(await page.locator(`[data-loop-tab="${copy}"] .ltab-name`).innerText(), '验证副页');

    // A delayed preview from the old scene must be discarded and replaced, without repainting the foreground.
    const race = await browser.newPage({ viewport: { width: 1600, height: 950 } });
    race.on('pageerror', e => errors.push(e.message));
    await race.route('**/api/loop-tabs?*', route => route.fulfill({ json: route.request().method() === 'GET' ? fixture : { ok: true } }));
    let held = 0, releaseOld, releaseNew;
    const oldGate = new Promise(resolve => { releaseOld = resolve; });
    const newGate = new Promise(resolve => { releaseNew = resolve; });
    await race.route('**/api/simulate', async route => {
      const body = JSON.parse(route.request().postData());
      if (body.sequence.length === 181) {
        const number = ++held;
        const response = await route.fetch();
        if (number === 1) await oldGate; else if (number === 2) await newGate;
        await route.fulfill({ response });
      } else await route.continue();
    });
    await race.goto(process.argv[2] || 'http://127.0.0.1:3005', { waitUntil: 'domcontentloaded' });
    await race.waitForFunction(() => window.Jx3LoopTabs?.ownsAutosave() && lastSimResult);
    await race.evaluate(() => Jx3Nav.switchPage('page-sim'));
    await assertEventually(() => held >= 1);
    await race.evaluate(async () => { document.getElementById('sim_delay').value = '42'; await runSimulate(); });
    releaseOld();
    await assertEventually(() => held >= 2);
    assert.equal(await race.locator('[data-loop-tab="copy-a"] [data-resolved-skill]').count(), 0);
    assert.equal(await race.evaluate(() => Jx3LoopTabs.activeId), 'main');
    assert.equal(await race.evaluate(() => window._lastSimBody.network_delay), 42);
    releaseNew();
    await race.waitForFunction(() => document.querySelector('[data-loop-tab="copy-a"] [data-resolved-skill]'));
    await race.locator('[data-loop-tab="copy-a"] .ltab-title').click();
    assert.equal(await race.evaluate(() => window._lastSimBody.network_delay), 42);
    await race.close();
    assert.deepEqual(errors, []);
    console.log('Passed: first-load material, default comparisons, request parity, three-pane scroll, focus/cache, menu isolation, preferences/reload, pair resize, display refresh, channel indexing, drag/copy, selection, hide/restore/rename and stale async-result rejection.');
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });

async function assertEventually(predicate) {
  for (let attempt = 0; attempt < 100; attempt++) {
    if (predicate()) return;
    await new Promise(resolve => setTimeout(resolve, 25));
  }
  assert.fail('Expected request was not issued');
}
