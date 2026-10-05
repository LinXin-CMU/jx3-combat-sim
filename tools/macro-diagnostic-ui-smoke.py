"""Real browser diagnostic checks; requires an isolated worker on localhost:3037."""
from urllib.parse import urlsplit
from playwright.sync_api import sync_playwright, expect


def main():
    with sync_playwright() as p:
        browser = p.chromium.launch(channel="msedge", headless=True)
        page = browser.new_page(viewport={"width": 1440, "height": 1000})
        page.set_default_timeout(30000)
        errors, calls = [], []
        page.on("pageerror", lambda error: errors.append(str(error)))

        def route_api(route):
            path = urlsplit(route.request.url).path
            if path == '/api/settings':
                route.fulfill(json={})
            elif route.request.method == 'GET' or path in {'/api/simulate', '/api/macro/diagnose', '/api/macro/prune_candidates', '/api/macro/assist', '/api/macro/assist/program', '/api/calculate', '/api/skill_damage'}:
                if path == '/api/macro/diagnose':
                    calls.append(route.request.post_data_json)
                route.continue_()
            else:
                route.fulfill(json={"ok": True})

        page.route('**/api/**', route_api)
        for version in ['AnYingQianJi', 'CangShengZhuShiTest']:
            response = page.request.post('http://127.0.0.1:3037/api/mounts/switch', data={"version": version, "mount": "FenShanJin", "persist": False})
            assert response.ok
            page.goto('http://127.0.0.1:3037', wait_until='networkidle')
            page.wait_for_function('!!window.Jx3MacroDiagnostic && !!window.Jx3MacroEditor')
            page.evaluate("""async () => {
                await currentMountReady; Jx3Nav.switchPage('page-sim');
                document.getElementById('sim_sequence').replaceChildren();
                ['盾刀','盾刀','盾压','盾刀'].forEach(skill=>addSeqItem(skill));
                await runSimulate();
            }""")
            page.locator('#macro_assist_toggle').click()
            page.locator('#macro_draft_shield').fill('/cast [rage>100] 血怒\n/cast 不存在\n/cast 盾刀\n/cast 盾压')
            page.locator('#macro_draft_blade').fill('')
            page.evaluate('async () => await Jx3MacroEditor.run()')
            page.locator('#macro_compare_sequence [data-compare-row]').first.click()
            panel = page.locator('.ma-diagnostic')
            expect(panel).to_contain_text('Step 1')
            expect(panel).to_contain_text('Step 2')
            expect(panel).to_contain_text('不成立，不进入技能池')
            expect(panel).to_contain_text('本行未检查可释放性')
            expect(panel).to_contain_text('最终释放：盾刀')
            panel.locator('summary').filter(has_text='展开完整宏判定日志').click()
            panel.get_by_role('button', name='第 3 行 · 盾刀').first.click()
            selected = page.locator('#macro_draft_shield').evaluate('(el)=>el.value.slice(el.selectionStart,el.selectionEnd)')
            assert selected == '/cast 盾刀', selected
            count = len(calls)
            page.locator('#macro_compare_sequence [data-compare-row]').first.click()
            expect(panel).to_contain_text('Step 2')
            assert len(calls) == count, 'Cached detail must not replay again'
            for theme in ['', 'pink-theme', 'light-theme', 'indigo-theme']:
                page.evaluate("""theme => { document.body.classList.remove('pink-theme','light-theme','indigo-theme'); if(theme)document.body.classList.add(theme); }""", theme)
                colors = panel.evaluate("el=>['.ma-diagnostic-pass','.ma-diagnostic-fail'].map(s=>getComputedStyle(el.querySelector(s)).backgroundColor)")
                assert colors[0] != colors[1], colors
            body = calls[-1]
            invalid = dict(body, end=body['start']+11)
            assert page.request.post('http://127.0.0.1:3037/api/macro/diagnose', data=invalid).status == 400
            mismatch = dict(body, mount='TieGuYi')
            assert page.request.post('http://127.0.0.1:3037/api/macro/diagnose', data=mismatch).status == 409
            page.locator('#macro_draft_shield').fill('/cast [rage>100] 血怒')
            page.evaluate('async () => await Jx3MacroEditor.run()')
            page.locator('#macro_compare_first').click()
            expect(panel).to_contain_text('技能池为空')
            expect(panel).to_contain_text('本轮未选中技能')
            rounds = panel.locator('select option').count()
            if rounds > 1:
                panel.get_by_role('button', name='下一轮', exact=True).click()
                expect(panel.locator('select')).to_have_value('1')
            # A changed draft must prevent old decisions from appearing again.
            page.locator('#macro_draft_shield').fill('/cast 盾压')
            page.locator('#macro_compare_first').click()
            expect(panel).to_contain_text('草稿或环境已变化')
            expect(panel.locator('select')).to_have_count(0)
        assert not errors, errors
        page.screenshot(path='backend/target/macro-diagnostic-review.png')
        browser.close()
        print('PASS: formal/test versions, real Step 1/2, line navigation, cache, four themes, empty pool, request limits, version isolation')


if __name__ == '__main__':
    main()
