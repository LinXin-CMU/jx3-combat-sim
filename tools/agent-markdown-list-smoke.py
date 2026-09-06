"""Exercise real chat rendering, using intercepted history without model calls."""
from playwright.sync_api import sync_playwright, expect

SESSION = "markdown-list-smoke"
TEXT = """### 做得好的地方
1. **全程零空转。** 第一项

1. 绝刀释放
   这一行仍属于第二项

1. 嗜血覆盖
1. 怒气管理
1. 体态切换

这里是列表外的段落。

3. 从三开始
4. 第四项
   - 子项甲
   - 子项乙
5. 第五项

### 无序列表
- 项目甲
- 项目乙

### 括号序号
1) 第一步
2) 第二步
"""
RESULT = {"run_id": "list-smoke", "status": "completed", "accounting": {},
          "report": {"provider_profile": "offline", "model": "fixture", "content": {
              "summary": TEXT, "findings": [], "recommendations": [], "limitations": []}}}

with sync_playwright() as p:
    browser = p.chromium.launch(channel="msedge", headless=True)
    page = browser.new_page(viewport={"width": 1280, "height": 900})
    page.route("**/api/agent/sessions", lambda r: r.fulfill(json={"sessions": [
        {"session_id": SESSION, "title": "列表检查", "status": "completed"}]}))
    page.route(f"**/api/agent/sessions/{SESSION}", lambda r: r.fulfill(json={
        "summary": {"title": "列表检查", "status": "completed"},
        "events": [{"kind": "run_result", "result": RESULT}]}))
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
        lists = page.locator(f'{surface} .agent-prose ol')
        expect(lists).to_have_count(3)
        expect(lists.nth(0).locator(':scope > li')).to_have_count(5)
        expect(lists.nth(0).locator('li').nth(1)).to_contain_text('仍属于第二项')
        assert lists.nth(0).evaluate('el => el.start') == 1
        assert lists.nth(1).evaluate('el => el.start') == 3
        expect(lists.nth(1).locator(':scope > li')).to_have_count(3)
        expect(lists.nth(1).locator('li > ul > li')).to_have_count(2)
        expect(lists.nth(2).locator(':scope > li')).to_have_count(2)
        expect(page.locator(f'{surface} .agent-prose > ul > li')).to_have_count(2)
        for theme in ['', 'light-theme', 'pink-theme']:
            page.evaluate("t => { document.body.classList.remove('light-theme','pink-theme'); if(t) document.body.classList.add(t); }", theme)
            assert lists.first.locator('li').first.evaluate('e => getComputedStyle(e).listStyleType') == 'decimal'
    browser.close()
print('PASS: continuous/loose/continued/nested/non-one/parenthesized lists in both chat surfaces and all themes.')
