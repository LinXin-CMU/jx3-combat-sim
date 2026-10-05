"""Target diagnosis, verified repairs and redesigned workspace on isolated :3037."""
from urllib.parse import urlsplit
from playwright.sync_api import sync_playwright, expect


def main():
    with sync_playwright() as p:
        browser = p.chromium.launch(channel='msedge', headless=True)
        page = browser.new_page(viewport={'width': 1800, 'height': 1100})
        page.set_default_timeout(30000)
        errors, requests = [], []
        page.on('pageerror', lambda error: errors.append(str(error)))

        def route_api(route):
            path = urlsplit(route.request.url).path
            if path == '/api/settings':
                route.fulfill(json={})
            elif route.request.method == 'GET' or path in {'/api/simulate', '/api/macro/diagnose', '/api/macro/prune_candidates', '/api/macro/assist', '/api/macro/assist/program', '/api/calculate', '/api/skill_damage'}:
                if path == '/api/macro/diagnose':
                    requests.append(route.request.post_data_json)
                route.continue_()
            else:
                route.fulfill(json={'ok': True})

        page.route('**/api/**', route_api)
        for version in ['AnYingQianJi', 'CangShengZhuShiTest']:
            assert page.request.post('http://127.0.0.1:3037/api/mounts/switch', data={'version': version, 'mount': 'FenShanJin', 'persist': False}).ok
            page.goto('http://127.0.0.1:3037', wait_until='networkidle')
            page.wait_for_function('!!window.Jx3MacroRepairPanel && !!window.Jx3MacroEditor')
            page.evaluate("""async () => {
                await currentMountReady; Jx3Nav.switchPage('page-sim');
                document.getElementById('sim_sequence').replaceChildren();
                ['盾压','盾刀','盾刀','盾刀','盾刀','盾刀'].forEach(skill=>addSeqItem(skill));
                await runSimulate();
            }""")
            page.locator('#macro_assist_toggle').click()
            page.locator('#macro_draft_shield').fill('/cast 盾刀\n/cast 盾压')
            page.locator('#macro_draft_blade').fill('')
            before = page.evaluate('JSON.stringify({sequence:readSequence(),template:lastSimResult,macro:buildMacroText()})')
            page.evaluate('async () => await Jx3MacroEditor.run()')
            panel = page.locator('.ma-diagnostic')
            expect(panel).to_contain_text('模板期待：盾压')
            expect(panel).to_contain_text('Step 2 顺序阻断')
            expect(panel.locator('.ma-diagnostic-conclusion')).to_be_visible()
            expect(page.locator('#macro_review')).to_have_attribute('data-tab', 'diagnosis')
            geometry = page.evaluate("""() => {
                const box=id=>{const r=document.getElementById(id).getBoundingClientRect();return {x:r.x,y:r.y,right:r.right,width:r.width};};
                return {editor:box('macro_editor_panel'),review:box('macro_review'),diagnostic:box('macro_compare_detail')};
            }""")
            assert geometry['editor']['right'] <= geometry['review']['x'], geometry
            assert geometry['diagnostic']['width'] > 550, geometry
            panel.get_by_role('button', name='生成并验证改法').click()
            expect(panel.get_by_role('button', name='重新验证改法')).to_be_enabled()
            cards = panel.locator('.ma-repair-card')
            expect(cards.first).to_contain_text('技能差异减少')
            expect(cards.first).to_contain_text('目标位置已匹配')
            expect(cards.first).to_contain_text('未新增技能分歧')
            assert page.locator('#macro_draft_shield').input_value() == '/cast 盾刀\n/cast 盾压'
            assert page.evaluate('JSON.stringify({sequence:readSequence(),template:lastSimResult,macro:buildMacroText()})') == before
            verification = [request for request in requests if request.get('include_result')]
            assert verification and all(request['simulation']['macro_duration'] > 0 for request in verification)
            for theme in ['', 'pink-theme', 'light-theme', 'indigo-theme']:
                page.evaluate("""theme=>{document.body.classList.remove('pink-theme','light-theme','indigo-theme');if(theme)document.body.classList.add(theme);}""", theme)
                colors = panel.evaluate("el=>['.ma-diagnostic-pass','.ma-diagnostic-fail'].map(s=>getComputedStyle(el.querySelector(s)).backgroundColor)")
                assert colors[0] != colors[1]
            page.evaluate("document.body.classList.remove('pink-theme','light-theme','indigo-theme')")
            page.mouse.move(10, 10)
            page.screenshot(path='backend/target/macro-repair-review.png')
            cards.first.get_by_role('button', name='应用到草稿并重跑').click()
            page.wait_for_function("Jx3MacroEditor.getResult()?.text.startsWith('#page shield\\n/cast 盾压') || Jx3MacroEditor.getResult()?.text.startsWith('/cast 盾压')")
            assert page.locator('#macro_draft_shield').input_value().startswith('/cast 盾压')
            page.locator('#macro_draft_undo').click()
            expect(page.locator('#macro_draft_shield')).to_have_value('/cast 盾刀\n/cast 盾压')
            expect(panel).to_contain_text('草稿或环境已变化')
            # Genuine Step 1 failure must expose a value and still provide a validated edit.
            page.locator('#macro_draft_shield').fill('/cast [rage>100] 盾压\n/cast 盾刀')
            page.evaluate('async () => await Jx3MacroEditor.run()')
            expect(panel).to_contain_text('Step 1 阻断')
            expect(panel).to_contain_text('怒气 0')
            panel.get_by_role('button', name='生成并验证改法').click()
            expect(panel.get_by_role('button', name='重新验证改法')).to_be_enabled()
            assert panel.locator('.ma-repair-card').count() > 0
            expect(panel).to_contain_text('缺失')
            page.locator('#macro_review_conditions').click()
            expect(page.locator('#macro_assist_panel')).to_be_visible()
            expect(page.locator('#macro_compare_detail')).to_be_visible()
            page.locator('#macro_review_draft').click()
            expect(panel).to_be_visible()
            for width in [1800, 1000]:
                page.set_viewport_size({'width': width, 'height': 1100})
                for handle in ['macro_split_top_width', 'macro_split_bottom_width', 'macro_split_editor_width', 'macro_split_height']:
                    separator = page.locator('#'+handle)
                    expect(separator).to_be_visible()
                    initial = separator.get_attribute('aria-valuenow')
                    separator.focus()
                    separator.press('ArrowUp' if handle == 'macro_split_height' else 'ArrowLeft')
                    if separator.get_attribute('aria-valuenow') == initial:
                        separator.press('ArrowDown' if handle == 'macro_split_height' else 'ArrowRight')
                    assert separator.get_attribute('aria-valuenow') != initial, (width, handle, initial)
                assert page.locator('#macro_compare_detail').evaluate('el=>el.scrollWidth<=el.clientWidth+1')
            page.set_viewport_size({'width': 1800, 'height': 1100})
            # A late verified candidate must not reappear after editing the draft.
            page.locator('#macro_draft_shield').fill('/cast 盾刀\n/cast 盾压')
            page.evaluate('async () => await Jx3MacroEditor.run()')
            expect(panel).to_contain_text('Step 2 顺序阻断')
            held = []

            def hold_verification(route):
                if route.request.post_data_json.get('include_result'):
                    held.append(route)
                else:
                    route.fallback()

            page.route('**/api/macro/diagnose', hold_verification)
            with page.expect_request(lambda request: request.url.endswith('/api/macro/diagnose') and request.post_data_json.get('include_result')):
                panel.get_by_role('button', name='生成并验证改法').click()
            page.locator('#macro_draft_shield').fill('/cast 盾刀\n/cast [rage>99] 盾压')
            expect(panel).to_contain_text('草稿或环境已变化')
            assert held
            held[0].fulfill(response=held[0].fetch())
            page.unroute('**/api/macro/diagnose', hold_verification)
            expect(panel.locator('.ma-repair-card')).to_have_count(0)
        assert not errors, errors
        browser.close()
        print('PASS: target blockers, verified repair/apply/undo, negative-window metrics, original data unchanged, two versions, four themes, tabs and adjustable layout')


if __name__ == '__main__':
    main()
