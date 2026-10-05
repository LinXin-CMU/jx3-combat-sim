/* Use an isolated worker; private scene arguments are never copied into the repository. */
const fs = require('node:fs');
const assert = require('node:assert/strict');
const { chromium } = require('playwright');
const scene = JSON.parse(fs.readFileSync(process.argv[3], 'utf8'));
(async () => {
  const browser = await chromium.launch({ channel: 'msedge', headless: true });
  try {
    const page = await browser.newPage({ viewport: { width: 1500, height: 950 } });
    const errors = [], requests = [];
    page.on('pageerror', e => errors.push(e.message));
    page.on('request', r => { if (r.url().endsWith('/api/simulate')) requests.push(r); });
    await page.goto(process.argv[2] || 'http://127.0.0.1:3005', { waitUntil: 'domcontentloaded' });
    await page.waitForFunction(() => window.Jx3LoopTabs?.ownsAutosave());
    await page.evaluate(() => Jx3Nav.switchPage('page-sim'));
    await page.evaluate(b => {
      applyLoopConfig({ version: 1, sequence: b.sequence.map((skill, index) => skill === '__macro__'
        ? { type: 'macro' } : { type: 'skill', skill, ...(b.solidified_casts?.[index] ? { solidified_cast: b.solidified_casts[index] } : {}) }).filter(e => e.type === 'macro' || e.skill), macro: { mode: 'general', general: b.macro_text },
        initial_rage: b.initial_rage, network_delay: b.network_delay, macro_duration: b.macro_duration }, { skipSimulate: true });
      for (const [key, value] of Object.entries(b.attributes || {})) {
        const el = document.getElementById(key); if (el) el.value = value;
      }
      getSimAttrs = () => b.attributes || getAttrs(); getSimHasteLevel = () => b.haste_level;
      getSelectedTalents = () => b.talents || []; getSelectedRecipes = () => b.recipes || [];
      getEquipmentMap = () => b.equipment || {}; getTeamBuffs = () => b.team_buffs || [];
      getCurrentFormation = () => b.formation || null; isExperimental = () => !!b.experimental;
      isHanjiaExpectationEnabled = () => !!b.hanjia_expectation; getTieguMode = () => b.tiegu_mode || 0;
      getBossAttackInterval = () => b.boss_attack_interval || null; if (b.target) getTarget = () => b.target;
      window._solidifyBefore = buildLoopConfig();
    }, scene);
    if (Object.keys(scene.solidified_casts || {}).length) {
      const capture = await page.evaluate(async () => {
        const original = await runSimulate();
        const source = await Jx3HarnessWorkspace.capture();
        const response = await fetch('/api/simulate', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(source.simulation) });
        const replay = await response.json();
        return { preserved: Object.keys(source.simulation.solidified_casts).length,
          installed: source.workspace.loop.sequence.filter(e => e.solidified_cast).length,
          successful: replay.timeline.filter(e => !e.triggered).length,
          skipped: replay.skipped, difference: Jx3MacroSolidify.compare(original, replay) };
      });
      assert.equal(capture.preserved, scene.sequence.length);
      assert.equal(capture.installed, scene.sequence.length);
      assert.equal(capture.successful, scene.sequence.length);
      assert.deepEqual(capture.skipped, []);
      assert.equal(capture.difference, null);
      assert.deepEqual(errors, []);
      console.log(JSON.stringify(capture));
      return;
    }
    const start = requests.length;
    const result = await page.evaluate(() => _runExpandMacro({ askConfirm: false, silent: true }));
    assert.equal(result.ok, true, JSON.stringify(result));
    assert.equal(requests.length - start, 2);
    assert.equal(await page.locator('#sim_sequence [data-solidified-cast]').count(), result.expanded,
      JSON.stringify(await page.evaluate(() => [...document.querySelectorAll('#sim_sequence .sim-seq-item')]
        .filter(e => !e.dataset.solidifiedCast).map(e => ({ name: e.dataset.skill, index: e.dataset.sequenceIndex })))));
    const saved = await page.evaluate(() => buildLoopConfig());
    assert.equal(saved.sequence.reduce((n, e) => n + (e.solidified_cast ? e.count || 1 : 0), 0), result.expanded);
    const difference = await page.evaluate(async cfg => {
      const source = lastSimResult; applyLoopConfig(cfg, { skipSimulate: true });
      return Jx3MacroSolidify.compare(source, await runSimulate());
    }, saved);
    assert.equal(difference, null);
    await page.evaluate(async () => {
      applyLoopConfig(window._solidifyBefore, { skipSimulate: true });
      await runSimulate();
    });
    const warmStart = requests.length;
    const warm = await page.evaluate(() => _runExpandMacro({ askConfirm: false, silent: true }));
    assert.equal(warm.ok, true, JSON.stringify(warm));
    assert.equal(requests.length - warmStart, 1);
    await page.evaluate(() => applyLoopConfig(window._solidifyBefore, { skipSimulate: true }));
    const original = await page.evaluate(() => JSON.stringify(buildLoopConfig().sequence));
    let count = 0;
    await page.route('**/api/simulate', async route => {
      if (++count !== 2) return route.continue();
      const response = await route.fetch(); const json = await response.json();
      json.timeline.find(e => !e.triggered).cast_time += 1;
      await route.fulfill({ response, json });
    });
    const failure = await page.evaluate(() => _runExpandMacro({ askConfirm: false, silent: true }));
    assert.equal(failure.ok, false);
    assert.equal(await page.evaluate(() => JSON.stringify(buildLoopConfig().sequence)), original);
    assert.deepEqual(errors, []);
    console.log(JSON.stringify({ expanded: result.expanded, cold_requests: 2, fresh_result_requests: 1,
      export_import: 'passed', failure_preserves_original: 'passed' }));
  } finally { await browser.close(); }
})().catch(e => { console.error(e); process.exitCode = 1; });
