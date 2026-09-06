"""Small browser check with synthetic history and intercepted model requests.

Requires Python Playwright and local Edge. Writes no simulator userdata and
never calls a model. Run against the existing local UI on port 3005.
"""
import json
from playwright.sync_api import sync_playwright

BASE = "http://127.0.0.1:3005"
SESSION = "ui-clarification-smoke"
submitted = []


def history(structured=False):
    clarification = {
        "schema_version": "agent-clarification/v1",
        "question": "要继续核实这次绝刀的位置吗？",
        "reason": "请确认下一步关注的内容。",
        "answer_hint": "继续核实 / 不必，直接给结论",
        "analysis_text": "已有分析正文应保留，等待用户澄清目标。",
    }
    if structured:
        clarification["options"] = [
            {"label": "继续核实", "description": "定位具体位置"},
            {"label": "直接给结论", "description": "根据已有资料回答"},
        ]
    result = {
        "run_id": "run-ui-clarification", "status": "needs_user_input",
        "provider_profile": "offline", "model": "ui-fixture",
        "accounting": {"duration_ms": 12}, "clarification": clarification,
    }
    return {
        "summary": {"status": "needs_user_input", "title": "选项交互检查", "corrupted_event_count": 0},
        "events": [{"kind": "run_result", "result": result}],
    }


with sync_playwright() as p:
    browser = p.chromium.launch(channel="msedge", headless=True)
    page = browser.new_page(viewport={"width": 1280, "height": 900})
    page.route("**/api/agent/sessions", lambda route: route.fulfill(json={"sessions": [
        {"session_id": SESSION, "title": "选项交互检查", "status": "needs_user_input"}
    ]}))
    page.route(f"**/api/agent/sessions/{SESSION}", lambda route: route.fulfill(json=history()))

    def intercept_run(route):
        submitted.append(route.request.post_data_json)
        route.fulfill(status=409, json={"error": {"message": "离线交互检查：未创建任务"}})

    page.route("**/api/agent/runs", intercept_run)
    page.goto(BASE, wait_until="domcontentloaded")
    page.wait_for_function("!!window.Jx3Nav")
    page.evaluate("window.Jx3Nav.switchPage('page-agent')")
    page.locator(f'#agent_session_list [data-session-id="{SESSION}"]').click()
    form = page.locator('#agent_transcript .agent-answer-form')
    form.wait_for()
    assert form.locator('input[type=radio]').count() == 3
    assert form.locator('button[type=submit]').is_disabled()
    form.get_by_text('1. 继续核实', exact=True).click()
    assert form.locator('button[type=submit]').is_enabled()
    assert not submitted  # Selection alone does not create a run.
    page.evaluate("window._lastSimBody = {sequence:['盾击'], haste_level:0}")
    page.locator('#agent_question').fill('保留未发送的草稿')
    form.locator('button[type=submit]').click()
    page.wait_for_function("!document.querySelector('#agent_run').disabled")
    assert len(submitted) == 1
    assert submitted[-1]['session_id'] == SESSION
    assert submitted[-1]['question'].endswith('我的回答：继续核实')
    assert page.locator('#agent_question').input_value() == '保留未发送的草稿'
    form.get_by_text('3. 自行填写', exact=True).click()
    form.locator('textarea').fill('先看12秒附近')
    form.locator('button[type=submit]').click()
    page.wait_for_function("!document.querySelector('#agent_run').disabled")
    assert submitted[-1]['question'].endswith('我的回答：先看12秒附近')

    # Structured options use the same component in the simulation dock.
    page.unroute(f"**/api/agent/sessions/{SESSION}")
    page.route(f"**/api/agent/sessions/{SESSION}", lambda route: route.fulfill(json=history(True)))
    page.evaluate("window.Jx3Nav.switchPage('page-sim')")
    page.locator('#sim_ai_fab').click()
    page.locator('#sim_ai_history').click()
    page.locator(f'#sim_ai_history_list [data-session-id="{SESSION}"]').click()
    dock_form = page.locator('#sim_ai_chat .agent-answer-form')
    dock_form.wait_for()
    analysis = page.locator('#sim_ai_chat .agent-clarification-analysis')
    assert analysis.evaluate('e => getComputedStyle(e).paddingLeft') == '14px'
    assert analysis.locator('.agent-prose').evaluate('e => getComputedStyle(e).fontSize') == '12px'
    assert page.locator('#sim_ai_chat .agent-clarification-analysis').get_by_text(
        '已有分析正文应保留，等待用户澄清目标。', exact=True).is_visible()
    assert dock_form.get_by_text('定位具体位置', exact=True).is_visible()
    dock_form.get_by_text('2. 直接给结论', exact=True).click()
    dock_form.locator('button[type=submit]').click()
    page.wait_for_function("!document.querySelector('#sim_ai_send').disabled")
    assert submitted[-1]['question'].endswith('我的回答：直接给结论')
    assert submitted[-1]['session_id'] == SESSION
    print(json.dumps({"ok": True, "intercepted_submissions": len(submitted),
                      "surfaces": ["full", "dock"], "model_calls": 0}))
    browser.close()
