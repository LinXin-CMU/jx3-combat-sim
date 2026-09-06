"""UI adapter test with synthetic history/axis; no model calls or userdata writes."""
from playwright.sync_api import sync_playwright, expect

SESSION = 'timeline-link-smoke'
result = {
    'run_id': 'timeline-smoke', 'status': 'completed', 'accounting': {},
    'report': {'provider_profile': 'offline', 'model': 'fixture', 'content': {
        'summary': '131.8s(30怒,血怒外,ev137)，尤其ev137那刀。[[两次绝刀|ev:137,172]]\n## 逐刀判断\n**需要核实**\n| 时间 | 怒气 |\n| --- | --- |\n| 131.8s | 30→0 |',
        'findings': [{'title': '怒气管理', 'explanation': '全战斗的怒气消耗与获取接近。\n**具体损失需要对照验证。**', 'metrics': []}], 'recommendations': [], 'boundaries': [],
    }},
}
with sync_playwright() as p:
    browser = p.chromium.launch(channel='msedge', headless=True)
    page = browser.new_page()
    page.route('**/api/agent/sessions', lambda r: r.fulfill(json={'sessions': [
        {'session_id': SESSION, 'title': '时间超链检查', 'status': 'completed'}]}))
    page.route(f'**/api/agent/sessions/{SESSION}', lambda r: r.fulfill(json={
        'summary': {'status': 'completed', 'title': '时间超链检查'},
        'events': [{'kind': 'run_result', 'result': result}]}))
    page.goto('http://127.0.0.1:3005', wait_until='domcontentloaded')
    page.wait_for_function('!!window.Jx3Nav')
    page.evaluate('''() => {
      window.Jx3TimelineBridge = {
        describe: ranges => ranges.map(range => ({...range, valid:true,
          timeLabel:range.start === 136 ? '131.80s' : '162.90s',
          skills:[{short:'绝', name:'绝刀', time:131.8, selected:true, iconUrl:'/favicon.ico',
            before:{rage:30,buffs:[{name:'嗜血',stacks:2,remaining:5.1,iconUrl:'/favicon.ico'}]},
            after:{rage:0,buffs:[]}}]})),
        focus: value => window.testFocused = value
      };
      window.Jx3Nav.switchPage('page-agent');
    }''')
    page.locator(f'#agent_session_list [data-session-id="{SESSION}"]').click()
    report = page.locator('#agent_transcript .agent-report')
    report.wait_for()
    assert 'ev137' not in report.inner_text()
    links = report.locator('.agent-rotation-reference')
    assert links.count() == 3
    assert report.locator('table').count() == 1
    assert report.locator('strong').first.inner_text() == '需要核实'
    assert '##' not in report.inner_text()
    assert '—需要核实—' not in report.inner_text()
    assert links.nth(0).inner_text() == '131.8s（30怒,血怒外）'
    try:
        links.nth(2).hover(timeout=3000)
    except Exception:
        print(page.evaluate('''() => [...document.querySelectorAll('.agent-rotation-reference, .agent-rotation-popover')].map(e => ({cls:e.className,rect:e.getBoundingClientRect().toJSON(),style:e.getAttribute('style')}))'''))
        raise
    popup = page.locator('.agent-rotation-popover')
    assert popup.locator('.agent-rotation-occurrence').count() == 2
    bounds = popup.bounding_box()
    assert bounds['width'] <= 300 and bounds['height'] <= 360, bounds
    assert popup.evaluate('e => e.scrollHeight <= e.clientHeight + 1')
    assert popup.locator('.agent-skill-chip img').count() == 2
    popup.locator('.agent-skill-gap').first.hover()
    assert '怒气 30' in popup.locator('.agent-rotation-state').first.inner_text()
    assert popup.locator('.agent-state-buff').count() == 1
    popup.locator('.agent-skill-chip').first.hover()
    assert '怒气 0' in popup.locator('.agent-rotation-state').first.inner_text()
    popup.get_by_role('button', name='下一处', exact=True).click()
    button = popup.locator('.agent-rotation-locate').nth(1)
    button_bounds = button.bounding_box()
    popup_bounds = popup.bounding_box()
    assert button_bounds['y'] + button_bounds['height'] <= popup_bounds['y'] + popup_bounds['height']
    button.click()
    assert page.evaluate('window.testFocused.start') == 171
    links.nth(0).focus()
    assert page.locator('.agent-rotation-popover').is_visible()
    # Legacy grouped references, including an already marked first event.
    original_summary = result['report']['content']['summary']
    for case, grouped in enumerate(['ev137/172/195/252/276', '[[这些绝刀|ev:137]]/172/195/252/276', 'ev137/ev172/ev195/ev252/ev276']):
        result['report']['content']['summary'] = f'检查{case}：查看这5次上下文（' + grouped + '），怒气不足50。'
        page.mouse.move(0, 0)
        page.locator(f'#agent_session_list [data-session-id="{SESSION}"]').click()
        summary = page.locator('#agent_transcript .agent-report-summary')
        expect(summary).to_contain_text(f'检查{case}：查看这5次上下文')
        assert '/172' not in summary.inner_text()
        reference = summary.locator('.agent-rotation-reference')
        expect(reference).to_have_count(1)
        reference.hover()
        grouped_popup = page.locator('.agent-rotation-popover')
        assert grouped_popup.locator('.agent-rotation-occurrence').count() == 5
        for _ in range(4):
            grouped_popup.get_by_role('button', name='下一处', exact=True).click()
        grouped_popup.locator('.agent-rotation-locate').nth(4).click()
        assert page.evaluate('window.testFocused.start') == 275
    result['report']['content']['summary'] = original_summary
    page.mouse.move(0, 0)
    page.evaluate("window.Jx3Nav.switchPage('page-sim')")
    page.locator('#sim_ai_fab').click()
    page.locator('#sim_ai_history').click()
    page.locator(f'#sim_ai_history_list [data-session-id="{SESSION}"]').click()
    dock = page.locator('#sim_ai_chat .sim-ai-result')
    dock.wait_for()
    sizes = dock.locator('.sim-ai-result-summary, .sim-ai-result-finding > .agent-prose, th, td').evaluate_all(
        'nodes => nodes.map(node => getComputedStyle(node).fontSize)')
    assert set(sizes) == {'12px'}, sizes
    assert dock.locator('.sim-ai-result-summary').evaluate('e => getComputedStyle(e).paddingLeft') == '14px'
    dock.screenshot(path='backend/target/agent-dock-typography.png')
    print('OK: legacy references, time labels, multiple occurrences, hover, keyboard, locate callback')
    browser.close()
