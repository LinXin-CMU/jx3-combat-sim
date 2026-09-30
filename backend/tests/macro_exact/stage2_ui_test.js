/* Isolated worker only: actual two-stage lifecycle + focused panel validation. */
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { chromium } = require('playwright');
const base = process.argv[2] || 'http://127.0.0.1:3005';
const root = path.resolve(__dirname, '../../..');
const scene = JSON.parse(fs.readFileSync(path.join(root, 'backend/tests/fixtures/exact_macro_short.json'), 'utf8').replace(/^\uFEFF/, ''));
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
async function api(route, body) {
  const response = await fetch(base + route, body === undefined ? {} : { method:'POST', headers:{'Content-Type':'application/json'}, body:JSON.stringify(body) });
  const result = await response.json();
  assert.equal(response.ok, true, JSON.stringify(result));
  return result;
}

(async () => {
  await api('/api/mounts/switch', { version:scene.version, mount:scene.mount, persist:false });
  let job = await api('/api/macro/exact', scene); // Omitted compress defaults to true.
  const route = '/api/macro/exact/' + job.id;
  const deadline = Date.now() + 30000;
  while (!job.done && job.stage !== 'compression') {
    assert.ok(Date.now() < deadline, 'stage transition timed out');
    await sleep(15); job = await api(route);
  }
  assert.equal(job.stage, 'compression');
  assert.equal(job.best.comparison.reproduced, true);
  await api(route + '/pause', {});
  while (!job.done && job.phase !== 'paused') { await sleep(20); job = await api(route); }
  assert.equal(job.phase, 'paused');
  const held = job.elapsed_ms, macro = job.best.macro;
  await sleep(200);
  job = await api(route);
  assert.equal(job.elapsed_ms, held);
  assert.equal(job.best.macro, macro);
  await api(route + '/resume', {});
  while (!job.done) { assert.ok(Date.now() < deadline); await sleep(25); job = await api(route); }
  assert.equal(job.status, 'exact');
  assert.equal(job.result.report.comparison.reproduced, true);
  assert.equal(job.result.report.comparison.target_count, 23);
  assert.ok(job.compression.best_chars < job.compression.initial_chars);
  assert.equal(job.result.macro, job.best.macro);
  assert.equal(job.result.report.compression_stop, 'scope_exhausted');
  console.log(JSON.stringify({ lifecycle:'passed', compression:job.compression, comparison:job.result.report.comparison }));

  // A first-draft success also takes the same automatic transition.
  let instant = await api('/api/macro/exact', { ...scene,simulation:{...scene.simulation,sequence:['盾击']} });
  while (!instant.done) { assert.ok(Date.now() < deadline); await sleep(20); instant = await api('/api/macro/exact/'+instant.id); }
  assert.equal(instant.stage,'compression');
  assert.equal(instant.iteration,0);
  assert.equal(instant.result.report.comparison.target_count,1);

  let cancelled = await api('/api/macro/exact',scene);
  const cancelRoute = '/api/macro/exact/'+cancelled.id;
  while (!cancelled.done && cancelled.stage !== 'compression') { assert.ok(Date.now() < deadline); await sleep(15); cancelled = await api(cancelRoute); }
  await api(cancelRoute+'/cancel',{});
  while (!cancelled.done) { assert.ok(Date.now() < deadline); await sleep(20); cancelled = await api(cancelRoute); }
  assert.equal(cancelled.result.report.compression_stop,'cancelled');
  assert.equal(cancelled.result.report.compression.status,'cancelled');
  assert.equal(cancelled.result.report.comparison.reproduced,true);
  assert.equal(cancelled.result.macro,cancelled.best.macro);
  console.log('PASS first-draft transition and actual compression cancellation preserve a full certificate.');

  const browser = await chromium.launch({ channel:'msedge', headless:true });
  try {
    const page = await browser.newPage({ viewport:{ width:1500,height:1000 } });
    const errors = []; page.on('pageerror', error => errors.push(error.message));
    const comparison = job.result.report.comparison;
    let state = { ...job, done:false,status:'running',phase:'replaying',pause_requested:false,elapsed_ms:4000 };
    let startBody;
    await page.route('**/api/macro/exact**', async intercept => {
      const request = intercept.request(), url = new URL(request.url());
      if (request.method() === 'POST') {
        if (url.pathname.endsWith('/pause')) state = { ...state,phase:'paused',pause_requested:true };
        else if (url.pathname.endsWith('/resume')) state = { ...state,phase:'replaying',pause_requested:false };
        else if (url.pathname.endsWith('/cancel')) state = { ...state,done:true,phase:'finished',status:'exact',
          result:{ report:{comparison,compression:state.compression,compression_stop:'cancelled'},macro:state.best.macro } };
        else { startBody = request.postDataJSON(); state = { ...state,done:false,phase:'solving',status:'running',stage:'extraction',compression:null,result:null,best:null,candidate:null }; }
        return intercept.fulfill({json:state});
      }
      return intercept.fulfill({json:url.pathname === '/api/macro/exact' ? {available:true,job:state} : state});
    });
    await page.goto(base,{waitUntil:'domcontentloaded'});
    await page.waitForFunction(() => window.Jx3Assistant && document.getElementById('em_macro'));
    await page.evaluate(() => Jx3Assistant.open('exact'));
    await page.waitForFunction(() => document.getElementById('em_phase').textContent.includes('第二阶段'));
    assert.match(await page.locator('#em_metrics').textContent(), /字符 \d+ → \d+/);
    assert.equal(await page.locator('#em_macro').textContent(), job.best.macro);
    await page.locator('#em_view').click();
    state = { ...state,revision:state.revision+1,candidate:{macro:'/cast 盾击',iteration:999,comparison:null} };
    await page.waitForFunction(() => document.getElementById('em_macro').textContent === '/cast 盾击');
    assert.match(await page.locator('#em_verdict').textContent(), /未验证/);
    await page.locator('#em_pause').click();
    await page.waitForFunction(() => document.getElementById('em_pause').textContent === '继续');
    const clock = await page.locator('#em_runtime').textContent(); await sleep(200);
    assert.equal(await page.locator('#em_runtime').textContent(), clock);
    await page.locator('#em_view').click();
    assert.equal(await page.locator('#em_macro').textContent(), job.best.macro);
    await page.locator('#em_stop').click();
    await page.waitForFunction(() => document.getElementById('em_phase').textContent.includes('压缩已停止'));
    assert.equal(await page.locator('#em_macro').textContent(), job.best.macro);
    await page.evaluate(scene => { Jx3HarnessWorkspace.capture = async () => scene; }, scene);
    await page.locator('#em_start').click();
    await page.waitForFunction(() => document.getElementById('em_phase').textContent.includes('第一阶段'));
    assert.equal(startBody.compress,true);
    assert.deepEqual(errors,[]);
    console.log('PASS panel: stage, live trial, certified best, pause/stop retention, automatic compression request.');
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
