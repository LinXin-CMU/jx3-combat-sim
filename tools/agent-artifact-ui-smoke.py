"""Candidate delivery in both chat surfaces; intercepted history, no model calls."""
import json
from pathlib import Path
from playwright.sync_api import sync_playwright, expect

SESSION = "artifact-ui-smoke"
CODE = "#page shield\n/cast [nobuff:血怒·惊涌] 血怒\n/cast 盾击\n#page blade\n/cast 斩刀"
RESULT = {"run_id": "artifact-smoke", "status": "partially_verified", "accounting": {},
          "report": {"provider_profile": "offline", "model": "fixture", "content": {
              "summary": "候选宏如下，复刻效果尚待核对。", "findings": [], "recommendations": [],
              "artifacts": [{"title": "双页宏", "language": "jx3_macro", "content": CODE, "syntax": "parsed"}],
              "limitations": []}}}

with sync_playwright() as p:
    browser = p.chromium.launch(channel="msedge", headless=True)
    context = browser.new_context(permissions=["clipboard-read", "clipboard-write"], viewport={"width":1280, "height":900})
    page = context.new_page()
    page.route("**/api/agent/sessions", lambda r: r.fulfill(json={"sessions":[{"session_id":SESSION,"title":"候选交付检查","status":"partially_verified"}]}))
    page.route(f"**/api/agent/sessions/{SESSION}", lambda r: r.fulfill(json={"summary":{"title":"候选交付检查","status":"partially_verified"},"events":[{"kind":"run_result","result":RESULT}]}))
    page.goto("http://127.0.0.1:3005", wait_until="domcontentloaded")
    page.wait_for_function("!!window.Jx3Nav")
    page.evaluate("window.Jx3Nav.switchPage('page-agent')")
    page.locator(f'#agent_session_list [data-session-id="{SESSION}"]').click()
    for surface in ["#agent_transcript", "#sim_ai_chat"]:
        if surface == "#sim_ai_chat":
            page.evaluate("window.Jx3Nav.switchPage('page-sim')")
            page.locator('#sim_ai_fab').click()
            page.locator('#sim_ai_history').click()
            page.locator(f'#sim_ai_history_list [data-session-id="{SESSION}"]').click()
        card = page.locator(f"{surface} .agent-draft-artifact")
        card.wait_for()
        expect(card.locator('.agent-macro-page')).to_have_count(2)
        bodies = ["/cast [nobuff:血怒·惊涌] 血怒\n/cast 盾击", "/cast 斩刀"]
        for index, title in enumerate(['盾宏', '刀宏']):
            section = card.locator('.agent-macro-page').nth(index)
            assert section.locator('pre code').text_content() == bodies[index]
            assert section.locator('.agent-macro-page-count').text_content() == f'{len(bodies[index])} / 128 字'
            section.get_by_role('button', name=f'复制{title}', exact=True).click()
            expect(section.get_by_role('button', name='已复制', exact=True)).to_be_visible()
            assert page.evaluate('navigator.clipboard.readText()').replace('\r\n', '\n') == bodies[index]
            assert section.evaluate('e => getComputedStyle(e).borderTopWidth') == '1px'
            assert section.locator('pre').evaluate('e => getComputedStyle(e).whiteSpace') == 'pre'
        card.get_by_role("button", name="复制完整宏").click()
        expect(card.locator('.agent-draft-head').get_by_role("button", name="已复制", exact=True)).to_be_visible()
        assert page.evaluate("navigator.clipboard.readText()").replace("\r\n", "\n") == CODE
        assert card.evaluate("e => e.scrollWidth <= e.clientWidth + 1")
        # The per-report feedback copy must also include the actual deliverable.
        page.locator(surface).get_by_role("button", name="复制结论", exact=True).click()
        expect(page.locator(surface).get_by_role("button", name="已复制", exact=True).last).to_be_visible()
        assert CODE in page.evaluate("navigator.clipboard.readText()").replace("\r\n", "\n")
    colors = []
    for theme in ['', 'light-theme', 'pink-theme']:
        page.evaluate("theme => { document.body.classList.remove('light-theme', 'pink-theme'); if (theme) document.body.classList.add(theme); }", theme)
        colors.append(card.locator('.agent-macro-page').first.evaluate('e => getComputedStyle(e).backgroundColor'))
        assert card.locator('.agent-macro-page-head').first.evaluate('e => getComputedStyle(e).borderBottomWidth') == '1px'
    assert len(set(colors)) == 3
    page.evaluate("document.body.classList.remove('pink-theme'); document.body.classList.add('light-theme')")
    card.screenshot(path=str(Path(__file__).resolve().parents[1] / 'backend/target/agent-macro-pages-light.png'))
    long_code = '#page shield\n/cast [' + 'buff:嗜血&' * 18 + 'rage>60] 盾飞\n/cast 盾击\n#page blade\n/cast 斩刀'
    RESULT['report']['content']['artifacts'] = [
        {'title':'长行检查','language':'jx3_macro','content':long_code,'syntax':'parsed'},
        {'title':'通用宏','language':'jx3_macro','content':'/cast 盾击\r\n/cast 斩刀\r\n'},
        {'title':'中文分页','language':'jx3_macro','content':'#page 擎盾\r\n/cast 盾击\r\n#page 擎刀\r\n/cast 斩刀'},
        {'title':'未知分页','language':'jx3_macro','content':'#page unknown\n/cast 盾击','syntax':'invalid'},
        {'title':'其它代码','language':'text','content':'<img src=x onerror=alert(1)>'},
    ]
    page.reload(wait_until='domcontentloaded')
    page.wait_for_function('!!window.Jx3Nav')
    page.evaluate("window.Jx3Nav.switchPage('page-agent')")
    page.locator(f'#agent_session_list [data-session-id="{SESSION}"]').click()
    cards = page.locator('#agent_transcript .agent-draft-artifact')
    expect(cards).to_have_count(5)
    assert cards.nth(0).locator('.is-over-limit').count() == 1
    assert cards.nth(0).locator('pre').first.evaluate('e => e.scrollWidth > e.clientWidth')
    assert cards.nth(0).evaluate('e => e.scrollWidth <= e.clientWidth + 1')
    assert cards.nth(1).locator('pre code').text_content() == '/cast 盾击\n/cast 斩刀'
    expect(cards.nth(2).locator('.agent-macro-page')).to_have_count(2)
    assert cards.nth(3).locator('pre code').text_content() == '#page unknown\n/cast 盾击'
    assert cards.nth(4).locator('pre code').text_content() == '<img src=x onerror=alert(1)>'
    assert cards.nth(4).locator('img').count() == 0
    print(json.dumps({"ok":True,"surfaces":["full","dock"],"page_copy":True,"full_copy":True,"report_copy":True,"long_line_contained":True,"fallbacks":True,"model_calls":0}))
    browser.close()
