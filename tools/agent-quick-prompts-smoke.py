"""Shortcut consistency and fill-only interaction; no model calls or userdata writes."""
from playwright.sync_api import sync_playwright, expect

LABELS = ['循环诊断', '攻略解读', '蒸馏成宏', '对比已存宏']
MACRO = '把当前循环蒸馏成宏，调优并实测，给我最终版本和与原循环的差异。'

with sync_playwright() as p:
    browser = p.chromium.launch(channel='msedge', headless=True)
    page = browser.new_page(viewport={'width': 1280, 'height': 900})
    calls = []
    page.route('**/api/agent/sessions', lambda r: r.fulfill(json={'sessions': []}))
    def reject_run(route):
        calls.append(route.request.url)
        route.abort()
    page.route('**/api/agent/runs', reject_run)
    page.goto('http://127.0.0.1:3005', wait_until='domcontentloaded')
    page.wait_for_function('!!window.Jx3Nav')
    page.evaluate("window.Jx3Nav.switchPage('page-agent')")
    starters = page.locator('#agent_transcript .agent-starter-grid button')
    expect(starters).to_have_text(LABELS)
    page.get_by_role('button', name='蒸馏成宏', exact=True).click()
    expect(page.locator('#agent_question')).to_have_value(MACRO)
    page.locator('#agent_new_session').click()
    expect(starters).to_have_text(LABELS)
    page.evaluate("window.Jx3Nav.switchPage('page-sim')")
    page.locator('#sim_ai_fab').click()
    shortcuts = page.locator('#sim_ai_quick button')
    expect(shortcuts).to_have_text(LABELS)
    page.locator('#sim_ai_quick').get_by_role('button', name='蒸馏成宏', exact=True).click()
    expect(page.locator('#sim_ai_question')).to_have_value(MACRO)
    page.evaluate("window.Jx3Nav.switchPage('page-equip')")
    expect(shortcuts).to_have_count(8)
    expect(shortcuts.first).to_have_text('当前配装')
    page.evaluate("window.Jx3Nav.switchPage('page-sim')")
    expect(shortcuts).to_have_text(LABELS)
    assert not calls, 'Shortcut click must fill the prompt, not start a paid model run'
    print('PASS: four shared shortcuts; welcome/reset/dock consistent; macro prompt fills only; equipment unchanged.')
    browser.close()
