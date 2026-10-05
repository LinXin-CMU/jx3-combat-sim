"""Focused pane/editor regression on an isolated worker; no existing userdata is touched."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
import urllib.request
from playwright.sync_api import sync_playwright, expect

parser = argparse.ArgumentParser()
parser.add_argument('--exe', required=True)
parser.add_argument('--port', type=int, default=3097)
parser.add_argument('--storage', action='store_true')
args = parser.parse_args()
root = Path(__file__).resolve().parents[2]
temp = root / '.tmp' / f'loop-tabs-ui-{os.getpid()}'
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
            api('/api/auth/me')
            break
        except Exception:
            time.sleep(.25)
    with sync_playwright() as pw:
        browser = pw.chromium.launch(channel='msedge', headless=True)
        page = browser.new_page(viewport={'width': 1550, 'height': 1020})
        errors = []
        page.on('pageerror', lambda e: errors.append(str(e)))
        page.goto(base, wait_until='domcontentloaded')
        page.wait_for_function('window.Jx3LoopTabs?.ownsAutosave()', timeout=30000)
        page.evaluate("Jx3Nav.switchPage('page-sim')")
        page.evaluate("""async () => {
          applyLoopConfig({version:1, sequence:[{type:'skill',skill:'盾刀',count:3}], initial_rage:0}, {skipSimulate:true});
          await runSimulate();
        }""")
        expect(page.locator('#sim_sequence .sim-seq-item')).to_have_count(3)
        original = page.evaluate('lastSimResult.rage')
        def open_tools():
            page.mouse.move(0, 0)
            page.locator('.ltab-pane.is-active .ltab-title').hover()
            expect(page.locator('#loop_tools')).to_be_visible()
        assert page.locator('.ltab-tools-trigger, .ltab-pages, .ltab-badge').count() == 0
        # Only the common title row occupies layout; tools float above the sequence.
        before_tools = page.locator('#sim_sequence').bounding_box()
        open_tools()
        assert page.locator('#sim_sequence').bounding_box() == before_tools
        page.mouse.move(0, 0)
        expect(page.locator('#loop_tools')).to_be_hidden()
        open_tools()
        page.get_by_role('button', name='复制当前循环到新页', exact=True).click()
        page.wait_for_function("Jx3LoopTabs.activeId !== 'main' && lastSimResult !== null")
        copy_id = page.evaluate('Jx3LoopTabs.activeId')
        page.evaluate("async () => { addSeqItem('盾刀'); await runSimulate(); }")
        expect(page.locator('#sim_sequence .sim-seq-item')).to_have_count(4)
        page.locator('.ltab-pane.is-active .ltab-menu summary').click()
        page.locator('.ltab-pane.is-active input[aria-label="循环页名称"]').fill('对照 B')
        page.locator('.ltab-pane.is-active input[aria-label="循环页名称"]').press('Enter')
        page.locator('.ltab-pane.is-active .ltab-menu summary').click()
        page.locator('.ltab-pane.is-active select[aria-label="选择对比循环"]').select_option('main')
        expect(page.locator('#sim_sequence [data-loop-diff="added"]')).to_have_count(1)
        left_y = page.locator('[data-loop-tab="main"] .sim-sequence').bounding_box()['y']
        right_y = page.locator('#sim_sequence').bounding_box()['y']
        assert abs(left_y - right_y) <= 1, (left_y, right_y)
        assert page.locator('.ltab-title .ltab-diff-summary').count() == 2
        assert page.locator('.ltab-diffbar').count() == 0
        page.locator('.ltab-pane.is-active .ltab-menu summary').click()
        # Resize boundary 2/3: page 1 and the outer edges must remain stationary.
        open_tools()
        page.get_by_role('button', name='新建空白循环页', exact=True).click()
        expect(page.locator('.ltab-pane')).to_have_count(3)
        third_id = page.evaluate('Jx3LoopTabs.activeId')
        main_pane = page.locator('[data-loop-tab="main"]')
        copy_pane = page.locator(f'[data-loop-tab="{copy_id}"]')
        third_pane = page.locator(f'[data-loop-tab="{third_id}"]')
        before = [p.bounding_box() for p in [main_pane, copy_pane, third_pane]]
        boundary = copy_pane.locator('.ltab-resizer').bounding_box()
        bx, by = boundary['x'] + boundary['width'] / 2, boundary['y'] + 120
        page.mouse.move(bx, by); page.mouse.down(); page.mouse.move(bx + 40, by, steps=8); page.mouse.up()
        after = [p.bounding_box() for p in [main_pane, copy_pane, third_pane]]
        for field in ['x', 'width']:
            assert abs(before[0][field] - after[0][field]) < .1, (before, after)
        assert abs(after[1]['width'] - before[1]['width'] - 40) < .1, (before, after)
        assert abs(after[2]['width'] - before[2]['width'] + 40) < .1, (before, after)
        assert abs(after[2]['x'] + after[2]['width'] - before[2]['x'] - before[2]['width']) < .1
        expect(third_pane.locator('.ltab-resizer')).to_be_hidden()
        copy_pane.locator('.ltab-resizer').dblclick()
        assert abs(copy_pane.bounding_box()['width'] - third_pane.bounding_box()['width']) < .1
        assert abs(main_pane.bounding_box()['width'] - before[0]['width']) < .1
        # Exchange visible pages through the ellipsis, preserving the original editable nodes.
        third_pane.locator('.ltab-menu summary').click()
        third_pane.get_by_role('combobox', name='交换页面', exact=True).select_option(copy_id)
        assert copy_pane.bounding_box()['x'] > third_pane.bounding_box()['x']
        expect(page.locator('#sim_sequence .sim-seq-item')).to_have_count(4)
        page.wait_for_timeout(700)
        page.reload(wait_until='domcontentloaded')
        page.wait_for_function('window.Jx3LoopTabs?.ownsAutosave()')
        page.evaluate("Jx3Nav.switchPage('page-sim')")
        assert copy_pane.bounding_box()['x'] > third_pane.bounding_box()['x']
        expect(page.locator('#sim_sequence .sim-seq-item')).to_have_count(4)
        third_pane.locator('.ltab-menu summary').click()
        page.once('dialog', lambda dialog: dialog.accept())
        third_pane.get_by_role('button', name='删除', exact=True).click()
        expect(page.locator('.ltab-pane')).to_have_count(2)
        copy_pane.locator('.ltab-title').click()
        # Removing a page releases its space to the final pane, with no empty work area.
        last_box = copy_pane.bounding_box()
        strip_box = page.locator('.ltab-strip').bounding_box()
        assert abs(last_box['x'] + last_box['width'] - strip_box['x'] - strip_box['width']) <= 2
        # Start a rectangle directly on the background of an inactive pane.
        page.evaluate('window._originalEditorForTest = document.getElementById("sim_sequence")')
        main_box = page.locator('[data-loop-tab="main"] .sim-sequence').bounding_box()
        last_box = page.locator('[data-loop-tab="main"] .sim-seq-item').nth(2).bounding_box()
        page.mouse.move(main_box['x'] + 2, main_box['y'] + 2)
        page.mouse.down()
        page.mouse.move(last_box['x'] + last_box['width'] + 3, last_box['y'] + last_box['height'] + 3, steps=10)
        page.mouse.up()
        assert page.evaluate('Jx3LoopTabs.activeId') == 'main'
        expect(page.locator('#sim_sequence .seq-selected')).to_have_count(3)
        assert page.evaluate('lastSimResult.rage') == original
        # Existing copy-placement still runs in the original container.
        box = page.locator('#sim_sequence').bounding_box()
        first = page.locator('#sim_sequence .sim-seq-item').first.bounding_box()
        third = page.locator('#sim_sequence .sim-seq-item').nth(2).bounding_box()
        page.locator('.seq-select-popup').get_by_text('复制', exact=True).click()
        page.mouse.move(third['x'] + third['width'] + 12, third['y'] + 15)
        page.mouse.click(third['x'] + third['width'] + 12, third['y'] + 15)
        expect(page.locator('#sim_sequence .sim-seq-item')).to_have_count(6)
        # Hide/show, width, and named persistence.
        page.evaluate('(id) => Jx3LoopTabs.activate(id)', copy_id)
        handle = page.locator('[data-loop-tab="main"] .ltab-resizer')
        handle.focus(); handle.press('ArrowRight')
        page.evaluate('(id) => Jx3LoopTabs.hide(id)', copy_id)
        expect(page.locator(f'[data-loop-tab="{copy_id}"]')).to_be_hidden()
        assert page.evaluate('document.getElementById("sim_sequence") === _originalEditorForTest')
        page.wait_for_timeout(1000)
        page.evaluate('Jx3LoopTabs.save()')
        if args.storage:
            page.wait_for_function("document.querySelector('.ltab-save').textContent === '已保存'")
        page.reload(wait_until='domcontentloaded')
        page.wait_for_function('window.Jx3LoopTabs?.ownsAutosave()', timeout=30000)
        page.evaluate("Jx3Nav.switchPage('page-sim')")
        expect(page.locator(f'[data-loop-tab="{copy_id}"]')).to_be_hidden()
        expect(page.locator('#sim_sequence .sim-seq-item')).to_have_count(6)
        page.locator('.ltab-pane.is-active .ltab-menu summary').click()
        page.locator('.ltab-pane.is-active').get_by_role('combobox', name='交换页面', exact=True).select_option(copy_id)
        expect(page.locator('#sim_sequence .sim-seq-item')).to_have_count(4)
        # A late response from a previous foreground may not repaint the newly active one.
        pending = []
        page.route('**/api/simulate', lambda route: pending.append(route))
        page.evaluate('void runSimulate()')
        for _ in range(20):
            if pending: break
            page.wait_for_timeout(50)
        assert pending
        page.evaluate("Jx3LoopTabs.activate('main')")
        foreground_rage = page.evaluate('lastSimResult?.rage')
        late = api('/api/simulate', pending[0].request.post_data_json)
        late['rage'] = 99
        pending[0].fulfill(json=late)
        page.wait_for_timeout(100)
        assert page.evaluate('lastSimResult?.rage') == foreground_rage
        for remaining in pending[1:]: remaining.continue_()
        page.unroute('**/api/simulate')
        page.evaluate('(id) => Jx3LoopTabs.activate(id)', copy_id)
        # Immediate refresh must recover the edit even before the debounced server save.
        page.evaluate("addSeqItem('盾刀')")
        page.reload(wait_until='domcontentloaded')
        page.wait_for_function('window.Jx3LoopTabs?.ownsAutosave()', timeout=30000)
        page.evaluate("Jx3Nav.switchPage('page-sim')")
        expect(page.locator('#sim_sequence .sim-seq-item')).to_have_count(5)
        page.evaluate('window._dragTestItem = document.querySelector("#sim_sequence .sim-seq-item")')
        page.locator('#sim_sequence .sim-seq-item').first.drag_to(page.locator('#sim_sequence .sim-seq-item').nth(3), target_position={'x': 38, 'y': 15})
        assert page.evaluate('[...document.querySelectorAll("#sim_sequence .sim-seq-item")].indexOf(_dragTestItem)') > 0
        page.locator('.ltab-pane.is-active .ltab-menu summary').click()
        page.locator('.ltab-pane.is-active select[aria-label="循环对比差异层级"]').select_option('states')
        page.wait_for_function("document.querySelector('.ltab-pane.is-active .ltab-diff-summary').textContent.includes('· 状态 ')")
        page.locator('.ltab-pane.is-active .ltab-menu summary').click()
        # Writing-macro mode retains its existing reference/draft layout.
        open_tools()
        page.locator('#macro_assist_toggle').click()
        expect(page.locator('#macro_compare_sequence')).to_be_visible()
        expect(page.locator('#sim_sequence')).to_be_visible()
        assert abs(page.locator('#sim_sequence').bounding_box()['y'] - page.locator('#macro_compare_sequence').bounding_box()['y']) <= 1
        open_tools()
        page.locator('#macro_assist_edit').click()
        expect(page.locator('[data-loop-tab="main"]')).to_be_visible()
        page.evaluate("document.body.classList.add('light-theme')")
        output = root / '.tmp' / 'loop-tabs-preview.png'
        page.keyboard.press('Escape')
        page.mouse.move(0, 0)
        page.locator('#panel_manual').screenshot(path=str(output))
        open_tools()
        page.locator('#panel_manual').screenshot(path=str(root / '.tmp' / 'loop-tabs-tools-preview.png'))
        # A narrow pane keeps the same title height without pushing the sequence down.
        page.evaluate("() => { const p = document.querySelector('.ltab-pane.is-active'); p.style.flex = '0 0 260px'; }")
        assert page.locator('.ltab-pane.is-active .ltab-heading').bounding_box()['height'] == 32
        assert abs(page.locator('#sim_sequence').bounding_box()['y'] - page.locator('[data-loop-tab="main"] .sim-sequence').bounding_box()['y']) <= 1
        assert not errors, errors
        print('PASS: hover tools, three-pane resize isolation, exchange/reload, native selection/copy/drag, redline, rename, hide/show, macro layout.')
        if args.storage:
            scope = page.evaluate('({version:currentMount.version,mount:currentMount.mount})')
            query = urllib.parse.urlencode(scope)
            stored = api('/api/loop-tabs?' + query)
            assert len(stored['tabs']) == 2
            assert api('/api/loop-tabs?version=OtherVersion&mount=FenShanJin') is None
            other_env = dict(env, JX3_USERDATA_DIR=str(temp / 'other-user'), JX3_PORT=str(args.port + 1))
            other = subprocess.Popen([str(Path(args.exe).resolve())], cwd=root / 'backend', env=other_env,
                                     stdout=log, stderr=log, creationflags=subprocess.CREATE_NO_WINDOW if os.name == 'nt' else 0)
            try:
                other_base = f'http://127.0.0.1:{args.port + 1}'
                for _ in range(120):
                    try:
                        with urllib.request.urlopen(other_base + '/api/auth/me', timeout=1): break
                    except Exception: time.sleep(.1)
                second = browser.new_page()
                second.route('**/api/auth/me', lambda r: r.fulfill(json={'enabled':True,'authed':True,'username':'fixture-b'}))
                second.add_init_script('localStorage.setItem("jx3_autosave_loop_v1",' + json.dumps(json.dumps({'version':1,'sequence':[{'type':'skill','skill':'盾刀','count':88}]})) + ')')
                second.goto(other_base, wait_until='domcontentloaded')
                second.wait_for_function('window.Jx3LoopTabs?.ownsAutosave()', timeout=30000)
                assert second.evaluate('Jx3LoopTabs.snapshot().tabs[0].sequence') == []
                second.wait_for_function("document.querySelector('.ltab-save').textContent === '已保存'")
                with urllib.request.urlopen(other_base + '/api/loop-tabs?' + query) as r:
                    assert json.load(r)['tabs'][0]['sequence'] == []
                assert len(api('/api/loop-tabs?' + query)['tabs']) == 2
                second.close()
                print('PASS: independent worker/user storage, version scope, no adoption of previous account local data.')
            finally:
                other.terminate()
                try: other.wait(timeout=5)
                except subprocess.TimeoutExpired: other.kill(); other.wait()
        browser.close()
finally:
    process.terminate()
    try: process.wait(timeout=5)
    except subprocess.TimeoutExpired: process.kill(); process.wait()
    log.close()
    # temp is created above under the repository; never touch another userdata tree.
    assert temp.resolve().parent == (root / '.tmp').resolve()
    shutil.rmtree(temp)
