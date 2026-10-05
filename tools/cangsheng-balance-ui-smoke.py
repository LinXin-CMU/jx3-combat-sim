"""Balance/settings integration smoke against an isolated worker (never writes settings)."""
import argparse
import json
import urllib.request
from urllib.parse import urlsplit
from playwright.sync_api import sync_playwright, expect

parser = argparse.ArgumentParser()
parser.add_argument('--backend', default='http://127.0.0.1:3037')
args = parser.parse_args()


def api(path, body=None):
    request = urllib.request.Request(args.backend + path,
        data=None if body is None else json.dumps(body).encode(),
        headers={'Content-Type': 'application/json'})
    with urllib.request.urlopen(request, timeout=30) as response:
        return json.load(response)


original = api('/api/mounts/current')
try:
    api('/api/mounts/switch', {'version': 'CangShengZhuShiTest', 'mount': 'FenShanJin', 'persist': False})
    with sync_playwright() as p:
        browser = p.chromium.launch(channel='msedge', headless=True)
        page = browser.new_page(viewport={'width': 1600, 'height': 1050})
        page.set_default_timeout(30000)
        errors, calls = [], []
        page.on('pageerror', lambda error: errors.append(str(error)))

        def route_api(route):
            path = urlsplit(route.request.url).path
            if path == '/api/settings':
                route.fulfill(json={})
            elif route.request.method == 'GET' or path in {
                '/api/simulate', '/api/skill_damage', '/api/calculate',
                '/api/macro/assist', '/api/macro/assist/program', '/api/macro/diagnose',
            }:
                if route.request.method == 'POST':
                    calls.append((path, route.request.post_data_json))
                route.continue_()
            else:
                route.fulfill(json={'ok': True})

        page.route('**/api/**', route_api)
        page.goto(args.backend, wait_until='networkidle')
        page.wait_for_function('!!window.Jx3MacroEditor && !!window.Jx3MacroAssist')
        expect(page.locator('.tb-brand-tag')).to_have_text('v2.0.10-20260911')
        page.evaluate('''async () => {
            await currentMountReady;
            if (!isCangShengFenShan()) throw new Error('Wrong test scope');
            if (testTalentCoreRequirement(91002) !== null) throw new Error('Shenwei still requires core');
            localStorage.setItem('expectation_enabled', '1');
            localStorage.setItem('dunya_reset_seed', '17');
            Jx3Nav.switchPage('page-sim');
            document.getElementById('sim_sequence').replaceChildren();
            ['盾压', ...Array(35).fill('盾刀'), '盾压'].forEach(addSeqItem);
            await runSimulate();
        }''')
        page.locator('#btn_settings').click()
        expect(page.locator('#toggle_expectation')).to_have_attribute('aria-checked', 'true')
        expect(page.locator('#settings_overlay')).to_contain_text('累计概率')
        expect(page.locator('#dunya_reset_settings')).to_be_hidden()
        assert page.locator('#dunya_reset_mode').count() == 0, 'Use the existing toggle'
        for theme in ['', 'pink-theme', 'indigo-theme', 'light-theme']:
            page.evaluate('''theme => {
                document.body.classList.remove('pink-theme','indigo-theme','light-theme');
                if (theme) document.body.classList.add(theme);
            }''', theme)
            expect(page.locator('#toggle_expectation')).to_be_visible()
        with page.expect_response(lambda r: '/api/simulate' in r.url
            and r.request.post_data_json.get('hanjia_expectation') is False):
            page.locator('#toggle_expectation').click()
        expect(page.locator('#dunya_reset_settings')).to_be_visible()
        page.locator('#dunya_reset_seed').fill('1234')
        page.locator('#dunya_reset_seed').press('Tab')
        page.wait_for_function("localStorage.getItem('dunya_reset_seed') === '1234'")
        page.locator('#settings_overlay .talent-close').click()
        page.evaluate('async () => await runSimulate()')
        page.locator('#macro_assist_toggle').click()
        page.locator('#macro_draft_shield').fill('/cast 盾压\n/cast 盾刀')
        page.locator('#macro_draft_blade').fill('')
        result = page.evaluate('''async () => {
            const result = await Jx3MacroEditor.run();
            const live = Jx3MacroEditor.getResult();
            return {ok: !!live, body: lastSimResult._macroAssistBody};
        }''')
        assert result['ok'] and result['body']['dunya_reset_seed'] == 1234
        assert result['body']['hanjia_expectation'] is False
        macro_calls = [body for path, body in calls if path == '/api/simulate' and body.get('macro_duration')]
        assert any(body.get('dunya_reset_seed') == 1234 and body.get('hanjia_expectation') is False for body in macro_calls)
        page.locator('#btn_settings').click()
        expect(page.locator('#toggle_expectation')).to_have_attribute('aria-checked', 'false')
        expect(page.locator('#dunya_reset_seed')).to_have_value('1234')
        assert not errors, errors
        browser.close()
        print('PASS: release label, Shenwei dependency, existing expectation toggle, seed persistence, four themes, template/macro environment')
finally:
    api('/api/mounts/switch', {'version': original['version'], 'mount': original['mount'], 'persist': False})
