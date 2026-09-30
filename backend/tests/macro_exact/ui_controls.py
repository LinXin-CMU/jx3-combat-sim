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
temp = root / '.tmp' / f'exact-controls-{os.getpid()}'
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
        page = browser.new_page(viewport={'width': 1500, 'height': 1050})
        errors, pending_pause = [], []
        page.on('pageerror', lambda error: errors.append(str(error)))
        macro = '\n'.join(f'/cast [rage>{i}.0] 盾刀' for i in range(300))
        state = {'id':'fixture', 'status':'running', 'phase':'solving', 'elapsed_ms':1200,
                 'elapsed_excludes_pauses':True, 'done':False, 'revision':1, 'iteration':2,
                 'best':{'macro':macro, 'comparison':None}, 'pause_requested':False}
        polls, metadata_only = [], []
        def route_exact(route):
            url = route.request.url
            if route.request.method == 'POST':
                if url.split('?')[0].endswith('/pause'):
                    pending_pause.append(route); return
                if url.split('?')[0].endswith('/cancel'):
                    state.update(cancel_requested=True, phase='cancelling')
                    route.fulfill(json=dict(state))
                    state.update(done=True, status='cancelled', phase='finished')
                    return
            polls.append(url)
            value = dict(state)
            value['best'] = dict(value['best'])
            value['best']['macro_revision'] = hashlib.sha256(value['best']['macro'].encode()).hexdigest()
            params = urllib.parse.parse_qs(urllib.parse.urlparse(url).query)
            if params.get('best_macro') == [value['best']['macro_revision']]:
                value['best'].pop('macro')
                metadata_only.append(len(json.dumps(value, ensure_ascii=False).encode()))
            if '/fixture?' in url:
                if f'revision={state["revision"]}' in url:
                    value.pop('best', None)
                route.fulfill(json=value)
            else:
                route.fulfill(json={'available':True, 'job':value})
        page.route('**/api/macro/exact**', route_exact)
        page.goto(base, wait_until='domcontentloaded')
        page.wait_for_function('window.Jx3Assistant && document.querySelector("#em_macro .em-code-line")')
        page.evaluate("Jx3Assistant.open('exact')")
        expect(page.locator('#em_macro .em-code-line')).to_have_count(300)
        expect(page.locator('#em_download')).to_have_count(0)
        assert page.locator('#em_macro').text_content() == macro
        assert page.locator('.em-code-line').first.bounding_box()['height'] < 25
        page.evaluate("""() => {
          window._macroMutations = 0;
          new MutationObserver(records => window._macroMutations += records.length)
            .observe(document.getElementById('em_macro'), {childList:true, subtree:true});
        }""")
        old_time = page.locator('#em_runtime').text_content()
        state['revision'] += 1  # Task progress changes, macro text does not.
        page.wait_for_timeout(350)
        assert page.locator('#em_runtime').text_content() != old_time
        for _ in range(4): page.evaluate("Jx3Assistant.open('exact')")
        page.wait_for_timeout(1800)
        assert page.evaluate('_macroMutations') == 0
        assert metadata_only and max(metadata_only) < 2000, metadata_only
        assert len(polls) < 9, polls
        page.locator('#em_macro').evaluate('(node) => node.scrollTop = 200')
        state['best'] = {'macro':macro.replace('rage>0.0', 'rage>0.1', 1) + '\n/cast 盾飞', 'comparison':None}
        state['revision'] += 1
        expect(page.locator('.em-change')).to_have_count(2)
        assert page.locator('.em-change').first.text_content() == '1'
        assert page.locator('#em_macro').evaluate('(node) => node.scrollTop') == 200
        assert page.locator('#em_macro').text_content() == state['best']['macro']
        expect(page.locator('#em_delta')).to_contain_text('改 1 行 · +1 行')
        page.evaluate("Object.defineProperty(navigator, 'clipboard', {value:{writeText:async text => window._copiedMacro = text},configurable:true})")
        page.locator('#em_copy').click()
        assert page.evaluate('_copiedMacro') == state['best']['macro']
        page.locator('#em_pause').click()
        expect(page.locator('#em_pause')).to_have_text('暂停中…')
        expect(page.locator('#em_stop')).to_be_enabled()
        frozen = page.locator('#em_runtime').text_content()
        page.wait_for_timeout(550)
        assert page.locator('#em_runtime').text_content() == frozen
        assert pending_pause
        page.locator('#em_stop').click()
        expect(page.locator('#em_status')).to_have_text('已停止')
        pending_pause[0].fulfill(json={**state, 'done':False, 'cancel_requested':False, 'pause_requested':True, 'phase':'paused', 'status':'running'})
        page.wait_for_timeout(150)
        expect(page.locator('#em_status')).to_have_text('已停止')
        assert page.locator('#em_macro').text_content() == state['best']['macro']
        page.locator('#em_macro').evaluate('(node) => node.scrollTop = 0')
        page.locator('#assistant_shell').screenshot(path=str(root / '.tmp' / 'exact-controls-preview.png'))
        assert not errors, errors
        browser.close()
        print('PASS UI: smooth clock, frozen pending pause, stop supersedes pause, single polling loop, unchanged DOM, inline diff, scroll and copy.')
        print('PASS wire: changed task revision sends metadata without unchanged macro text; cached macro stays visible.')

    # Real process control, not a mocked solver: pause/resume and cancel from preparation.
    scene = json.loads((root / 'backend/tests/fixtures/exact_macro_short.json').read_text(encoding='utf-8-sig'))
    api('/api/mounts/switch', dict(version=scene['version'], mount=scene['mount'], persist=False))
    job = api('/api/macro/exact', dict(scene, compress=False))
    path = '/api/macro/exact/' + job['id']
    api(path + '/pause', {})
    limit = time.monotonic() + 8
    while time.monotonic() < limit:
        job = api(path + '?compact=true')
        if job['phase'] == 'paused': break
        assert not job['done'], job['status']
        time.sleep(.05)
    assert job['phase'] == 'paused', job['phase']
    frozen = job['elapsed_ms']
    time.sleep(.25)
    restored = api('/api/macro/exact?compact=true')['job']
    assert restored['elapsed_ms'] == frozen
    resumed = api(path + '/resume', {})
    assert not resumed['pause_requested']
    began = time.monotonic()
    api(path + '/cancel', {})
    while time.monotonic() - began < 4:
        job = api(path + '?compact=true')
        if job['done']: break
        time.sleep(.05)
    delay = time.monotonic() - began
    assert job['done'] and job['status'] == 'cancelled', job
    assert delay < 3, delay
    print(f'PASS process: pause freezes through restore; resume/cancel finishes in {delay:.3f}s.')
finally:
    process.terminate()
    try: process.wait(timeout=5)
    except subprocess.TimeoutExpired: process.kill(); process.wait()
    log.close()
    assert temp.resolve().parent == (root / '.tmp').resolve()
    shutil.rmtree(temp)
