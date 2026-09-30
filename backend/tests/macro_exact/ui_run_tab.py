"""Focused clock/control/diff checks with an isolated worker and no real userdata."""
import argparse
import json
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import time
import urllib.request
import urllib.parse
from playwright.sync_api import sync_playwright, expect

parser = argparse.ArgumentParser()
parser.add_argument('--exe', required=True)
parser.add_argument('--port', type=int, default=3096)
args = parser.parse_args()
root = Path(__file__).resolve().parents[3]
temp = root / '.tmp' / f'exact-run-tab-{os.getpid()}'
temp.mkdir(parents=True)
env = dict(os.environ)
for key in ['JX3_ROUTER', 'JX3_AUTH_PASSWORD', 'JX3_AUTH_FILE', 'JX3_PUBLIC_DEPLOYMENT', 'JX3_DEEPSEEK_API_KEY']:
    env.pop(key, None)
env.update(JX3_USERDATA_DIR=str(temp / 'userdata'), JX3_PORT=str(args.port), JX3_BIND='127.0.0.1', JX3_NO_BROWSER='1')
log = (temp / 'worker.log').open('w', encoding='utf-8')
process = subprocess.Popen([str(Path(args.exe).resolve())], cwd=root / 'backend', env=env, stdout=log, stderr=log,
                           creationflags=subprocess.CREATE_NO_WINDOW if os.name == 'nt' else 0)
base = f'http://127.0.0.1:{args.port}'

def api(path, body=None):
    request = urllib.request.Request(base + path, data=None if body is None else json.dumps(body).encode(),
                                     headers={'Content-Type': 'application/json'})
    with urllib.request.urlopen(request, timeout=15) as response:
        return json.load(response)

try:
    for _ in range(120):
        try:
            api('/api/auth/me'); break
        except Exception:
            time.sleep(.1)
    with sync_playwright() as pw:
        browser = pw.chromium.launch(channel='msedge', headless=True)
        page = browser.new_page(viewport={'width': 1550, 'height': 1050})
        errors = []
        page.on('pageerror', lambda e: errors.append(str(e)))
        page.goto(base, wait_until='domcontentloaded')
        page.wait_for_function('window.Jx3LoopTabs?.ownsAutosave()', timeout=30000)
        page.evaluate("Jx3Nav.switchPage('page-sim')")
        page.wait_for_function('window.Jx3HarnessWorkspace && document.getElementById("em_run")')
        page.evaluate("Jx3Assistant.open('exact')")
        page.evaluate("""async () => {
          applyLoopConfig({version:1,sequence:[{type:'skill',skill:'盾刀',count:3}],initial_rage:0},{skipSimulate:true});
          await runSimulate();
        }""")
        source = page.evaluate('Jx3HarnessWorkspace.capture()')
        job = api('/api/macro/exact', {k:source[k] for k in ['version','mount','simulation']} | {'horizon':6})
        path = '/api/macro/exact/' + job['id']
        api(path + '/cancel', {})
        frozen = api(path + '/source')
        assert frozen['simulation']['sequence'] == ['盾刀'] * 3
        # Keep the real source endpoint, replace only solver progress with a fixed candidate.
        def progress(route):
            if route.request.url.split('?')[0].endswith('/source'):
                route.continue_(); return
            value = dict(job, done=True,status='cancelled',phase='finished',best={'macro':'/cast 盾刀','comparison':None})
            route.fulfill(json={'available':True,'job':value} if route.request.url.split('?')[0].endswith('/exact') else value)
        page.route('**/api/macro/exact**', progress)
        page.evaluate('Jx3LoopTabs.save()')
        page.reload(wait_until='domcontentloaded')
        page.wait_for_function('window.Jx3LoopTabs?.ownsAutosave()', timeout=30000)
        page.wait_for_function('window.Jx3HarnessWorkspace && document.getElementById("em_run")')
        page.evaluate("Jx3Nav.switchPage('page-sim'); Jx3Assistant.open('exact')")
        expect(page.locator('#em_run')).to_be_enabled()
        # Capture the full native request sent by the new button.
        sent = []
        page.on('request', lambda req: sent.append(req.post_data_json) if req.method == 'POST' and req.url.endswith('/api/simulate') else None)
        page.locator('#em_run').click()
        expect(page.locator('#em_run_feedback')).to_contain_text('已运行至', timeout=15000)
        output_id = page.evaluate('Jx3LoopTabs.activeId')
        assert output_id != 'main'
        request = next(v for v in sent if v.get('macro_text') == '/cast 盾刀')
        for field, value in frozen['simulation'].items():
            if field not in ['sequence','macro_text','macro_duration','channel_ticks','timing_offsets','qijin_buffs','lite','lite_keep_timeline']:
                assert request[field] == value, field
        assert request['macro_duration'] == 6 and len(request['sequence']) == 6000
        assert page.locator('#sim_sequence .sim-seq-item').count() > 0
        assert page.evaluate('lastSimResult.rage') > 0
        page.evaluate("Jx3LoopTabs.activate('main')")
        expect(page.locator('#sim_sequence .sim-seq-item')).to_have_count(3)
        page.evaluate('(id) => Jx3LoopTabs.hide(id)', output_id)
        page.locator('#em_output').select_option(output_id)
        page.locator('#em_run').click()
        expect(page.locator('#em_run_feedback')).to_contain_text('隐藏备份页', timeout=15000)
        assert page.evaluate('Jx3LoopTabs.activeId') == output_id
        tabs = page.evaluate('Jx3LoopTabs.snapshot().tabs')
        assert any(t['hidden'] and '覆盖前' in t['name'] for t in tabs)
        # A changed shared environment must not import a result under the wrong settings.
        count = len(tabs)
        page.evaluate("document.getElementById('sim_delay').value = '99'")
        page.locator('#em_output').select_option('')
        page.locator('#em_run').click()
        expect(page.locator('#em_run_feedback')).to_contain_text('公共场景参数与模板不同')
        assert len(page.evaluate('Jx3LoopTabs.list()')) == count
        page.locator('#assistant_shell').screenshot(path=str(root / '.tmp/exact-run-tab-preview.png'))
        page.evaluate("""async () => {
          Jx3Assistant.close(); document.getElementById('sim_delay').value = '0';
          Jx3LoopTabs.activate('main', {silent:true});
          applyLoopConfig({version:1,sequence:[{type:'skill',skill:'盾刀',count:240}],initial_rage:0},{skipSimulate:true});
          await runSimulate(); Jx3LoopTabs.add(true);
        }""")
        page.wait_for_function('lastSimResult !== null')
        second = page.evaluate('Jx3LoopTabs.activeId')
        # Compare identical long rotations. Both bottoms and viewport heights must align.
        geometry = page.evaluate("""() => {
          const a=document.querySelector('[data-loop-tab="main"] .sim-sequence'), b=document.getElementById('sim_sequence');
          a.scrollTop=a.scrollHeight; b.scrollTop=b.scrollHeight;
          return [a.clientHeight,b.clientHeight,a.lastElementChild.getBoundingClientRect().bottom,b.lastElementChild.getBoundingClientRect().bottom];
        }""")
        assert abs(geometry[0]-geometry[1]) < 1 and abs(geometry[2]-geometry[3]) < 1, geometry
        page.evaluate("""() => {
          document.querySelector('[data-loop-tab="main"] .sim-sequence').scrollTop=230;
          document.getElementById('sim_sequence').scrollTop=470;
        }""")
        page.locator('[data-loop-tab="main"] .ltab-title').click()
        assert page.evaluate('document.getElementById("sim_sequence").scrollTop') == 230
        assert page.locator(f'[data-loop-tab="{second}"] .sim-sequence').evaluate('(n) => n.scrollTop') == 470
        page.locator(f'[data-loop-tab="{second}"] .ltab-title').click()
        assert page.evaluate('document.getElementById("sim_sequence").scrollTop') == 470
        assert page.locator('[data-loop-tab="main"] .sim-sequence').evaluate('(n) => n.scrollTop') == 230
        print('PASS: equal viewport/bottom geometry; independent background scrolling survives both foreground switches.')
        page.locator('.ltab-pane.is-active .ltab-menu summary').click()
        selector = page.locator('.ltab-pane.is-active select[aria-label="选择对比循环"]')
        selector.select_option('main')
        toggle = page.locator('.ltab-pane.is-active .ltab-sync-label input')
        expect(toggle).to_be_checked()
        page.wait_for_timeout(180)
        page.evaluate('document.getElementById("sim_sequence").scrollTop=650')
        page.wait_for_function('Math.abs(document.querySelector(\'[data-loop-tab="main"] .sim-sequence\').scrollTop-650)<2')
        page.evaluate('document.querySelector(\'[data-loop-tab="main"] .sim-sequence\').scrollTop=200')
        page.wait_for_function('Math.abs(document.getElementById("sim_sequence").scrollTop-200)<2')
        toggle.uncheck()
        page.evaluate('document.getElementById("sim_sequence").scrollTop=500')
        page.wait_for_timeout(180)
        assert page.locator('[data-loop-tab="main"] .sim-sequence').evaluate('(n)=>n.scrollTop') == 200
        selector.select_option('')
        selector.select_option('main')
        expect(toggle).to_be_checked()
        page.wait_for_function('Math.abs(document.querySelector(\'[data-loop-tab="main"] .sim-sequence\').scrollTop-500)<2')
        print('PASS: comparison defaults to bidirectional sync; off preserves independent positions; re-enter resets to on.')
        assert not errors, errors
        browser.close()
        print('PASS: real frozen source after reload; native full-parameter replay; new/existing/hidden tab output; template preserved; shared-scene mismatch guarded.')
finally:
    process.terminate()
    try: process.wait(timeout=5)
    except subprocess.TimeoutExpired: process.kill(); process.wait()
    log.close()
    assert temp.resolve().parent == (root / '.tmp').resolve()
    shutil.rmtree(temp)
