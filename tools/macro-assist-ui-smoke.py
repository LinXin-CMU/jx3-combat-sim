"""Browser regression against an isolated localhost backend, without user writes.

Usage: python tools/macro-assist-ui-smoke.py --base-url http://127.0.0.1:3037
Requires Playwright and Edge. Browser persistence requests are intercepted.
Delayed-response checks hold real backend responses entirely inside this browser.
"""
import argparse
import json
import re
from pathlib import Path
from urllib.parse import urlsplit

from playwright.sync_api import expect, sync_playwright


TIMEOUT = 20000
FIXTURE = ["盾压", "盾刀", "盾刀", "盾压", "盾刀", "盾压", "盾刀", "盾刀"]
NUMERIC_BUFF = re.compile(r"\b[a-z]*buff(?:time)?:\s*\d+(?=\s*(?:[<>=!&|)\]]|$))")
BUFF_TIME = re.compile(r"\b(?:t?bufftime):[^&|\]\r\n]*?(?:[<>!=]=?)(-?\d+(?:\.\d+)?)")
DRAFT_PREFIX = "jx3_macro_draft_v1:"
REMOTE_DRAFT_KEY = DRAFT_PREFIX + '["user:remote-fixture","AnYingQianJi","FenShanJin"]'
OTHER_DRAFT_KEY = DRAFT_PREFIX + '["user:other-fixture","AnYingQianJi","FenShanJin"]'


def assert_macro_syntax(text: str) -> None:
    assert not NUMERIC_BUFF.search(text), f"Numeric buff ID in copyable condition: {text}"
    for threshold in BUFF_TIME.findall(text):
        assert "." not in threshold or len(threshold.split(".", 1)[1]) <= 1, text


def contrast_ratio(foreground: str, background: str) -> float:
    def luminance(color):
        channels = [int(value) / 255 for value in re.findall(r"\d+", color)[:3]]
        linear = [value / 12.92 if value <= .04045 else ((value + .055) / 1.055) ** 2.4 for value in channels]
        return sum(value * weight for value, weight in zip(linear, [.2126, .7152, .0722]))
    a, b = luminance(foreground), luminance(background)
    return (max(a, b) + .05) / (min(a, b) + .05)


def run(base_url: str, version: str, storage_only: bool = False) -> None:
    parsed = urlsplit(base_url)
    assert parsed.scheme == "http" and parsed.hostname in {"127.0.0.1", "localhost"} and parsed.port == 3037, \
        "Use the isolated localhost:3037 backend; never target the development/user worker"
    with sync_playwright() as p:
        context = p.request.new_context(base_url=base_url)
        switched = context.post('/api/mounts/switch', data={"version": version, "mount": "FenShanJin", "persist": False})
        assert switched.ok and switched.json().get("ok"), switched.text()
        context.dispose()
        browser = p.chromium.launch(channel="msedge", headless=True)
        try:
            page = browser.new_page(viewport={"width": 1440, "height": 1000})
            page.set_default_timeout(TIMEOUT)
            failures, calls, simulations, writes, settings_posts = [], [], [], [], []
            page.on("pageerror", lambda error: failures.append(str(error)))

            def route_api(route):
                path = urlsplit(route.request.url).path
                if path == "/api/settings":
                    if route.request.method not in {"GET", "HEAD"}:
                        writes.append(path)
                        settings_posts.append(route.request.post_data_json)
                        route.fulfill(json={})
                    else:
                        route.fulfill(json={REMOTE_DRAFT_KEY: "/cast 服务端旧草稿", "macro_smoke_settings_fixture": "server-fixture"})
                elif route.request.method in {"GET", "HEAD"}:
                    route.continue_()
                elif path in {"/api/simulate", "/api/macro/assist", "/api/macro/assist/program", "/api/calculate", "/api/skill_damage"}:
                    if path in {"/api/macro/assist", "/api/macro/assist/program"}:
                        calls.append(route.request.post_data_json)
                    elif path == "/api/simulate":
                        simulations.append(route.request.post_data_json)
                    route.continue_()
                else:
                    writes.append(path)
                    route.fulfill(json={"ok": True})

            page.route("**/api/**", route_api)
            page.goto(base_url, wait_until="networkidle")
            page.wait_for_function("!!window.Jx3MacroAssist && !!window.Jx3MacroEditor && !!window.Jx3MacroAlignment")

            # Historical remote settings must restore ordinary settings without
            # importing any account's local macro draft into this browser.
            assert page.evaluate("localStorage.getItem('macro_smoke_settings_fixture')") == "server-fixture"
            assert page.evaluate("key => localStorage.getItem(key)", REMOTE_DRAFT_KEY) is None
            page.evaluate("""({key}) => {
                localStorage.setItem(key, '/cast 另一账号的本地草稿');
                window.Jx3MacroEditor.insert('/cast 盾刀');
            }""", {"key": OTHER_DRAFT_KEY})
            assert page.evaluate("key => localStorage.getItem(key)", OTHER_DRAFT_KEY) == "/cast 另一账号的本地草稿"
            assert page.evaluate("""prefix => Object.keys(localStorage).filter(key => key.startsWith(prefix)).length >= 2""",
                                 DRAFT_PREFIX), "Both the current and other account drafts must be present locally"
            before_settings = len(settings_posts)
            page.evaluate("setSeqDisplayMode('text')")
            page.wait_for_timeout(800)  # Existing settings sync uses a 600 ms debounce.
            assert len(settings_posts) > before_settings, "An ordinary display setting did not trigger settings synchronization"
            assert any(snapshot.get("seq_display_mode") == "text" for snapshot in settings_posts[before_settings:])
            assert all(not any(key.startswith(DRAFT_PREFIX) for key in snapshot) for snapshot in settings_posts), \
                "Local macro drafts leaked into the account settings payload"
            assert page.evaluate("key => localStorage.getItem(key)", OTHER_DRAFT_KEY) == "/cast 另一账号的本地草稿"
            if storage_only:
                assert not failures, failures
                print(json.dumps({"ok": True, "version": version, "checks": ["settings restore ignores remote draft keys",
                    "ordinary settings still restore", "ordinary setting triggers POST", "settings POST excludes all local account draft keys",
                    "other account draft remains local"], "settings_posts_intercepted": len(settings_posts)}, ensure_ascii=False))
                return
            page.evaluate("setSeqDisplayMode('icon')")
            page.evaluate("""async (names) => {
                await currentMountReady;
                window.Jx3Nav.switchPage('page-sim');
                document.getElementById('sim_sequence').replaceChildren();
                for (const name of names) addSeqItem(name);
                await runSimulate();
                const originalFetch = window.fetch.bind(window);
                const smoke = window.__macroAssistSmoke = {
                    holdNext: false, held: [], responses: [], nextId: 0, copied: [],
                };
                window.fetch = async (...args) => {
                    const input = args[0];
                    const path = new URL(typeof input === 'string' ? input : input.url, location.href).pathname;
                    if (!['/api/macro/assist', '/api/macro/assist/program'].includes(path)) return originalFetch(...args);
                    const held = smoke.holdNext;
                    smoke.holdNext = false;
                    const request = JSON.parse(args[1].body);
                    // Intentionally detach cancellation for this one read-only
                    // request so the revision guard faces a deliverable old body.
                    const raw = await originalFetch(input, held ? {...args[1], signal: undefined} : args[1]);
                    // Drain before holding: aborting a superseded request must not hide
                    // a missing revision check by making its eventual json() fail.
                    const body = await raw.text();
                    const entry = {id: ++smoke.nextId, path, request, data: JSON.parse(body), delivered: false};
                    smoke.responses.push(entry);
                    if (held) await new Promise(resolve => smoke.held.push({id: entry.id, resolve}));
                    entry.delivered = true;
                    return new Response(body, {status: raw.status, statusText: raw.statusText, headers: raw.headers});
                };
                Object.defineProperty(navigator, 'clipboard', {configurable: true, value: {
                    writeText: async text => { smoke.copied.push(String(text)); },
                }});
            }""", FIXTURE)

            items = page.locator("#sim_sequence .sim-seq-item:not(.seq-auto):not(.seq-pre-release)")
            preview = page.locator("#macro_assist_selection")
            status = page.locator("#macro_assist_status")
            panel = page.locator("#macro_assist_panel")
            edit = page.locator("#macro_assist_edit")
            write = page.locator("#macro_assist_toggle")

            def settle_ui():
                page.evaluate("() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))")

            def selected_indices():
                return page.locator("#sim_sequence .ma-selected").evaluate_all(
                    "nodes => nodes.map(node => Number(node.dataset.macroAssistIndex)).sort((a,b) => a-b)")

            def assert_preview(names, indices):
                expect(preview).to_be_visible()
                expect(preview).to_contain_text("已选：" + " → ".join(names))
                assert selected_indices() == indices

            def hold_next():
                page.evaluate("window.__macroAssistSmoke.holdNext = true")

            def wait_held():
                try:
                    page.wait_for_function("window.__macroAssistSmoke.held.length === 1")
                except Exception as error:
                    diagnostic = page.evaluate("""() => ({status:document.getElementById('macro_assist_status').textContent,
                        holdNext:window.__macroAssistSmoke.holdNext, held:window.__macroAssistSmoke.held.length,
                        responses:window.__macroAssistSmoke.responses.slice(-5).map(entry => ({id:entry.id,path:entry.path,
                            delivered:entry.delivered,selection:[entry.request.selection_start,entry.request.selection_end]}))})""")
                    raise AssertionError({"held_response_timeout": diagnostic, "page_errors": failures}) from error
                return page.evaluate("window.__macroAssistSmoke.held[0].id")

            def release_held():
                page.evaluate("window.__macroAssistSmoke.held.splice(0).forEach(entry => entry.resolve())")

            def wait_result(start, end, step, after=0, open_steps=False):
                page.wait_for_function("""({start,end,step,after}) => window.__macroAssistSmoke.responses.some(entry => {
                    const data = entry.data.steps?.[step] || entry.data;
                    return entry.id > after && entry.delivered && data.selection?.start_active_index === start &&
                        data.selection?.end_active_index === end && data.selection?.selected_step === step;
                })
                """, arg={"start": start, "end": end, "step": step, "after": after})
                settle_ui()
                # Let the existing 120 ms environment debounce adopt its result
                # before interacting with newly rendered disclosure controls.
                page.wait_for_timeout(180)
                assert calls[-1]["selection_start"] == start and calls[-1]["selection_end"] == end, calls[-1]
                if start == end:
                    expect(page.locator("#macro_assist_candidates .ma-table tbody tr").first).to_be_visible()
                else:
                    expect(page.locator(".ma-program").first).to_be_visible()
                    if open_steps:
                        details = page.locator(".ma-program-steps")
                        if details.get_attribute("open") is None:
                            details.locator("summary").first.click()
                        expect(page.locator("#macro_assist_candidates .ma-table tbody tr").first).to_be_visible()
                return page.evaluate("""({start,end,step}) => window.__macroAssistSmoke.responses.filter(entry =>
                    entry.delivered && (entry.data.steps?.[step] || entry.data).selection?.start_active_index === start &&
                    (entry.data.steps?.[step] || entry.data).selection?.end_active_index === end &&
                    (entry.data.steps?.[step] || entry.data).selection?.selected_step === step)
                    .map(entry => entry.data.steps?.[step] || entry.data).at(-1)
                """, {"start": start, "end": end, "step": step})

            def last_response_id():
                return page.evaluate("window.__macroAssistSmoke.nextId")

            def drag_box(first, last):
                page.mouse.move(first["x"] - 2, first["y"] - 2)
                page.mouse.down()
                page.mouse.move(last["x"] + last["width"] + 2, last["y"] + last["height"] + 2, steps=12)
                page.mouse.up()

            expect(items).to_have_count(len(FIXTURE))
            assert page.evaluate("lastSimResult.timeline.filter(e => !e.triggered).length") == len(FIXTURE)
            original_sequence = page.evaluate("readSequence()")
            expect(edit).to_have_text("编辑序列")
            expect(write).to_have_text("写宏")
            expect(page.locator("#macro_assist_refresh, #macro_assist_close")).to_have_count(0)
            write.click()
            expect(write).to_have_attribute("aria-pressed", "true")
            expect(edit).to_have_attribute("aria-pressed", "false")
            expect(panel).to_be_visible()

            # Single selection is visible before the actual response is delivered.
            hold_next()
            items.first.click()
            assert_preview(["盾压"], [0])
            wait_held()
            assert_preview(["盾压"], [0])
            expect(page.locator(".ma-table")).to_have_count(0)
            assert not page.evaluate("window.__macroAssistSmoke.responses.find(entry => entry.id === window.__macroAssistSmoke.held[0].id).delivered")
            release_held()
            single = wait_result(0, 0, 0)
            expect(status).to_contain_text("找到 3 处")
            assert len(single["occurrences"]) == 3
            assert page.evaluate("readSequence()") == original_sequence

            # Center-to-center range selection crosses a wrapped row.
            page.evaluate("document.getElementById('sim_sequence').style.width = '170px'")
            boxes = [items.nth(index).bounding_box() for index in range(len(FIXTURE))]
            wrap = next(index for index in range(len(FIXTURE)-1) if abs(boxes[index]["y"]-boxes[index+1]["y"]) > 5)
            start, end = boxes[wrap], boxes[wrap+1]
            before = last_response_id()
            hold_next()
            page.mouse.move(start["x"]+start["width"]/2, start["y"]+start["height"]/2)
            page.mouse.down()
            page.mouse.move(end["x"]+end["width"]/2, end["y"]+end["height"]/2, steps=10)
            page.mouse.up()
            assert_preview(FIXTURE[wrap:wrap+2], [wrap, wrap+1])
            wait_held()
            assert_preview(FIXTURE[wrap:wrap+2], [wrap, wrap+1])
            expect(page.locator(".ma-table")).to_have_count(0)
            release_held()
            wait_result(wrap, wrap+1, 0, before)
            page.evaluate("document.getElementById('sim_sequence').style.width = ''")

            # Blank-space rectangle never invokes editor move/copy actions.
            before = last_response_id()
            drag_box(items.nth(0).bounding_box(), items.nth(1).bounding_box())
            assert_preview(["盾压", "盾刀"], [0, 1])
            wait_result(0, 1, 0, before)
            expect(page.locator(".seq-select-popup")).to_have_count(0)
            assert page.evaluate("readSequence()") == original_sequence

            # Derived passive display blocks neither invalidate nor enter selection.
            page.evaluate("""() => {
                const item = document.createElement('div');
                item.className = 'sim-seq-item seq-auto'; item.dataset.skill = '盾回'; item.textContent = '自动';
                document.querySelector('#sim_sequence .sim-seq-item').after(item);
            }""")
            drag_box(items.nth(0).bounding_box(), items.nth(1).bounding_box())
            assert_preview(["盾压", "盾刀"], [0, 1])
            expect(page.locator("#sim_sequence .seq-auto.ma-selected")).to_have_count(0)
            wait_result(0, 1, 0)
            page.evaluate("document.querySelectorAll('#sim_sequence .seq-auto').forEach(node => node.remove())")

            # A combination now defaults to a full multi-line program; individual
            # steps are a local view of the same response, not additional requests.
            expect(page.locator(".ma-program-steps")).not_to_have_attribute("open", "")
            program_text = page.locator(".ma-program-code").first.inner_text()
            assert len(program_text.splitlines()) >= 2, program_text
            program_call_count = len(calls)
            page.locator(".ma-program-steps > summary").click()
            page.get_by_role("button", name="2. 盾刀", exact=True).click()
            second_step = wait_result(0, 1, 1)
            assert len(calls) == program_call_count
            assert second_step["same_skill_outside_combo_indices"] == [2, 7], second_step
            assert set(second_step["positives"]) == {1, 4, 6}, second_step
            assert "same_skill_other_step_indices" in second_step
            assert "other_skill_negative_indices" in second_step
            expect(page.locator("#macro_assist_contrast")).to_be_visible()
            expect(page.locator("#macro_assist_contrast")).to_contain_text("组合内本步骤")
            expect(page.locator("#macro_assist_contrast")).to_contain_text("组合外同技能：2 次")
            assert page.locator(".ma-table code").first.inner_text().endswith(" 盾刀")
            page.locator("#macro_assist_candidates").get_by_role("button", name="复制", exact=True).first.click()
            page.wait_for_function("window.__macroAssistSmoke.copied.length > 0")
            copied = page.evaluate("window.__macroAssistSmoke.copied.at(-1)")
            assert copied.endswith(" 盾刀"), copied
            assert_macro_syntax(copied)
            alternatives = page.locator(".ma-equivalents").first
            expect(alternatives).to_be_attached()
            alternatives.locator("summary").click()
            equivalent = alternatives.locator(".ma-equivalent-line").first
            expression = equivalent.locator("code").inner_text()
            copy_count = page.evaluate("window.__macroAssistSmoke.copied.length")
            equivalent.get_by_role("button", name="复制写法", exact=True).click()
            page.wait_for_function("count => window.__macroAssistSmoke.copied.length > count", arg=copy_count)
            copied = page.evaluate("window.__macroAssistSmoke.copied.at(-1)")
            assert copied == (f"/cast [{expression}] 盾刀" if expression != "无条件" else "/cast 盾刀"), copied
            assert_macro_syntax(copied)
            page.get_by_role("button", name="查看", exact=True).first.click()
            expect(page.locator("#macro_assist_evidence")).to_be_visible()
            occurrences = page.locator(".ma-occurrences")
            if occurrences.get_attribute("open") is None:
                occurrences.locator("summary").first.click()
            page.locator(".ma-hit summary").first.click()
            expect(page.locator(".ma-hit .ma-state").first).to_be_visible()
            assert "怒气" in page.locator(".ma-hit .ma-state").first.inner_text()
            assert "充能" in page.locator(".ma-hit .ma-state").first.inner_text()

            # Both fixed mode buttons retain their labels. Reentry reuses the
            # selected pattern, current step and completed result without requests.
            request_counts = (len(calls), len(simulations))
            old_macros = page.locator("#macro_assist_candidates .ma-table code").all_text_contents()
            edit.click()
            expect(panel).to_be_hidden()
            expect(edit).to_have_attribute("aria-pressed", "true")
            expect(write).to_have_text("写宏")
            write.click()
            expect(panel).to_be_visible()
            assert_preview(["盾压", "盾刀"], [0, 1])
            if page.locator(".ma-program-steps").get_attribute("open") is None:
                page.locator(".ma-program-steps > summary").click()
            expect(page.get_by_role("button", name="2. 盾刀", exact=True)).to_have_attribute("aria-pressed", "true")
            expect(page.locator("#macro_assist_candidates .ma-table code")).to_have_text(old_macros)
            # A debounce must not turn an unchanged mode switch into recomputation.
            page.wait_for_timeout(400)
            assert (len(calls), len(simulations)) == request_counts

            # Environment changes recompute automatically while keeping the
            # pattern/step/preview. No refresh button or reselect action is used.
            before = last_response_id()
            simulate_count = len(simulations)
            hold_next()
            page.evaluate("""() => {
                const delay = document.getElementById('sim_delay');
                delay.value = '123';
                delay.dispatchEvent(new Event('input', {bubbles: true}));
                delay.dispatchEvent(new Event('change', {bubbles: true}));
            }""")
            wait_held()
            assert_preview(["盾压", "盾刀"], [0, 1])
            expect(page.locator(".ma-table")).to_have_count(0)
            assert len(simulations) > simulate_count
            assert simulations[-1]["network_delay"] == 123, simulations[-1]
            release_held()
            wait_result(0, 1, 1, before, open_steps=True)
            expect(page.get_by_role("button", name="2. 盾刀", exact=True)).to_have_attribute("aria-pressed", "true")
            expect(page.locator("#macro_assist_content")).not_to_contain_text("重新选择")
            expect(status).not_to_contain_text("过期")

            # A real old response arriving after a new selection cannot replace it.
            hold_next()
            items.nth(0).click()
            assert_preview(["盾压"], [0])
            old_id = wait_held()
            items.nth(2).click()
            assert_preview(["盾刀"], [2])
            wait_result(2, 2, 0, old_id)
            expect(status).to_contain_text("找到 5 处")
            release_held()
            page.wait_for_function("id => window.__macroAssistSmoke.responses.find(entry => entry.id === id)?.delivered", arg=old_id)
            settle_ui()
            assert_preview(["盾刀"], [2])
            expect(status).to_contain_text("找到 5 处")
            assert page.locator(".ma-table code").first.inner_text().endswith(" 盾刀")

            # Escape must also invalidate a held response, not just its highlights.
            hold_next()
            items.first.click()
            canceled_id = wait_held()
            page.keyboard.press("Escape")
            expect(page.locator("#sim_sequence .ma-selected")).to_have_count(0)
            release_held()
            page.wait_for_function("id => window.__macroAssistSmoke.responses.find(entry => entry.id === id)?.delivered", arg=canceled_id)
            settle_ui()
            expect(page.locator("#sim_sequence .ma-selected")).to_have_count(0)
            expect(page.locator(".ma-table")).to_have_count(0)

            # Rejected selections clear both the synchronous preview and marks.
            rejected_call_count = len(calls)
            page.evaluate("""() => {
                const item = document.createElement('div');
                item.className = 'sim-seq-item seq-auto'; item.dataset.skill = '盾回'; item.textContent = '自动';
                document.querySelector('#sim_sequence .sim-seq-item').after(item);
            }""")
            page.locator("#sim_sequence .seq-auto").click()
            expect(status).to_contain_text("请选择成功释放的主动技能")
            expect(preview).to_have_text("尚未选择技能")
            expect(page.locator("#sim_sequence .ma-selected, #sim_sequence .seq-selected")).to_have_count(0)
            page.evaluate("document.querySelectorAll('#sim_sequence .seq-auto').forEach(node => node.remove())")
            page.evaluate("""() => {
                const nodes = document.querySelectorAll('#sim_sequence [data-macro-assist-index]');
                window.Jx3MacroAssist.selectItems([nodes[0], nodes[2]]);
            }""")
            expect(status).to_contain_text("框选结果中间有未选技能")
            expect(preview).to_have_text("尚未选择技能")
            expect(page.locator("#sim_sequence .ma-selected, #sim_sequence .seq-selected")).to_have_count(0)
            assert len(calls) == rejected_call_count

            # Restore a pair for layout and editor compatibility checks.
            before = last_response_id()
            drag_box(items.nth(0).bounding_box(), items.nth(1).bounding_box())
            wait_result(0, 1, 0, before)

            # Candidate insertion edits an independent draft, including undo and
            # importing the current macro. No simulation is needed for these edits.
            draft = page.locator("#macro_draft_shield")
            blade_draft = page.locator("#macro_draft_blade")
            canonical_draft = page.locator("#macro_draft_text")
            expect(draft).to_be_visible()
            expect(blade_draft).to_be_visible()
            expect(canonical_draft).to_be_hidden()
            existing_macros = page.evaluate("JSON.stringify(macroPages)")
            current_macro = page.evaluate("buildMacroText()")
            draft.fill("/cast 盾刀")
            draft.evaluate("node => node.setSelectionRange(node.value.length, node.value.length)")
            program_text = page.locator(".ma-program-code").first.inner_text()
            page.locator(".ma-program").first.get_by_role("button", name="插入宏", exact=True).click()
            expect(draft).to_have_value("/cast 盾刀\n" + program_text)
            page.locator("#macro_draft_undo").click()
            expect(draft).to_have_value("/cast 盾刀")
            page.locator("#macro_draft_import").click()
            expect(canonical_draft).to_have_value(current_macro)
            assert page.evaluate("JSON.stringify(macroPages)") == existing_macros

            # Re-run the template before freezing the identity and exact payload:
            # the right-hand real simulation must not replace any left-side state.
            page.evaluate("""async () => {
                await runSimulate();
                Object.assign(window.__macroAssistSmoke, {
                    templateRef: lastSimResult, templateJSON: JSON.stringify(lastSimResult),
                    sequenceJSON: JSON.stringify(readSequence()), macrosJSON: JSON.stringify(macroPages),
                    contextKey: window.Jx3MacroAssist.contextKey(),
                });
            }""")
            blade_draft.fill("")
            draft.fill("/cast 盾刀")
            before_run = len(simulations)
            page.locator("#macro_compare_run").click()
            page.wait_for_function("""() => {
                const result = window.Jx3MacroEditor.getResult();
                return result?.text === '/cast 盾刀' && result.actual?.timeline?.length > 0;
            }""")
            expect(page.locator("#macro_compare_run")).to_be_enabled()
            assert len(simulations) > before_run
            assert simulations[-1]["macro_text"] == "/cast 盾刀", simulations[-1]
            assert page.evaluate("""() => {
                const smoke = window.__macroAssistSmoke;
                return lastSimResult === smoke.templateRef && JSON.stringify(lastSimResult) === smoke.templateJSON
                    && JSON.stringify(readSequence()) === smoke.sequenceJSON && JSON.stringify(macroPages) === smoke.macrosJSON
                    && window.Jx3MacroAssist.contextKey() === smoke.contextKey;
            }"""), "Running the draft changed the template sequence/result/current macro"
            expect(page.locator('#sim_sequence .ma-compare-gap, #macro_compare_sequence .ma-redline-missing')).to_have_count(0)
            comparison = page.locator("#macro_compare_sequence")
            expect(comparison.locator(".sim-seq-item").first).to_be_visible()
            assert comparison.locator(".sim-seq-item .seq-icon-wrap").count() > 0
            assert comparison.locator(".sim-seq-item .seq-label").count() > 0
            expect(page.locator('#macro_compare_layer')).to_have_value('skills')
            expect(comparison.locator('.ma-redline-changed')).to_have_count(0)
            layer_simulation_count = len(simulations)
            page.locator('#macro_compare_layer').select_option('states')
            expect(comparison.locator('.ma-redline-changed').first).to_be_visible()
            assert len(simulations) == layer_simulation_count
            assert comparison.locator(".ma-redline-changed").count() > 0

            page.locator("#macro_compare_first").click()
            expect(page.locator("#macro_compare_detail")).to_be_visible()
            comparison.locator(".ma-redline-changed").first.click()
            detail = page.locator("#macro_compare_detail")
            expect(detail).to_contain_text("怒气")
            expect(detail).to_contain_text("模板")
            expect(detail).to_contain_text("实际")
            detail.get_by_role("button", name=re.compile("定位.*第.*行")).click()
            assert draft.evaluate("node => node.value.slice(node.selectionStart, node.selectionEnd)").strip() == "/cast 盾刀"

            # All four existing themes must resolve the same semantic variables.
            # Check readable editor text and visible panel/redline boundaries.
            themes = []
            for theme in ["", "pink-theme", "light-theme", "indigo-theme"]:
                theme_result = page.evaluate("""theme => {
                    document.body.classList.remove('pink-theme', 'light-theme', 'indigo-theme');
                    if (theme) document.body.classList.add(theme);
                    const text = document.getElementById('macro_draft_shield');
                    const panel = document.getElementById('macro_compare_panel');
                    const changed = document.querySelector('#macro_compare_sequence .ma-redline-changed');
                    const sample = document.createElement('span'); panel.append(sample);
                    const resolve = name => { sample.style.color = `var(${name})`; return getComputedStyle(sample).color; };
                    const style = getComputedStyle(text), panelStyle = getComputedStyle(panel);
                    const result = {theme: theme || 'default', color:style.color, background:style.backgroundColor,
                        border:style.borderTopColor, panelBorder:panelStyle.borderLeftColor,
                        borderWidth:parseFloat(style.borderTopWidth), panelBorderWidth:parseFloat(panelStyle.borderLeftWidth),
                        expectedColor:resolve('--text'), expectedBorder:resolve('--border'), expectedBackground:resolve('--code-bg'),
                        changedBackground:getComputedStyle(changed,'::after').backgroundColor, changedOpacity:getComputedStyle(changed).opacity};
                    sample.style.backgroundColor = 'color-mix(in srgb,var(--green) 45%,var(--surface))';
                    result.expectedChangedBackground = getComputedStyle(sample).backgroundColor;
                    sample.remove(); return result;
                }""", theme)
                assert theme_result["color"] == theme_result["expectedColor"], theme_result
                assert theme_result["background"] == theme_result["expectedBackground"], theme_result
                assert theme_result["border"] == theme_result["expectedBorder"], theme_result
                assert theme_result["panelBorder"] == theme_result["expectedBorder"], theme_result
                assert theme_result["borderWidth"] >= 1 and theme_result["panelBorderWidth"] >= 1, theme_result
                assert theme_result["changedBackground"] == theme_result["expectedChangedBackground"] and theme_result["changedOpacity"] == '1', theme_result
                assert contrast_ratio(theme_result["color"], theme_result["background"]) >= 4.5, theme_result
                themes.append(theme_result)
                if theme == "light-theme":
                    page.evaluate("document.getElementById('macro_assist_content').scrollTop = 0; document.getElementById('macro_compare_detail').scrollTop = 0")
                    settle_ui()
                    screenshot = Path(__file__).resolve().parents[1] / "backend" / "target" / "macro-editor-review.png"
                    page.screenshot(path=str(screenshot), full_page=True)
            assert len({result["background"] for result in themes}) == 4, themes
            page.evaluate("document.body.classList.remove('pink-theme', 'light-theme', 'indigo-theme')")

            # The original display controls apply to both sequence containers.
            # Compare computed style, rather than merely checking duplicated classes.
            for mode, label, helpers in [("text", "name", True), ("icon", "name", True),
                                          ("icon", "time", False), ("icon", "none", False)]:
                page.evaluate("""({mode,label,helpers}) => {
                    setSeqDisplayMode(mode); setSeqIconLabel(label); setSeqHelpersEnabled(helpers);
                }""", {"mode": mode, "label": label, "helpers": helpers})
                settle_ui()
                styles = page.evaluate("""() => {
                    const inspect = id => {
                        const node = document.querySelector(`#${id} .sim-seq-item:not(.seq-auto):not(.ma-redline-missing)`);
                        const style = selector => { const el = node.querySelector(selector); if (!el) return null;
                            const s = getComputedStyle(el); return {display:s.display,visibility:s.visibility,fontSize:s.fontSize}; };
                        return {icon:style('.seq-icon-wrap'), label:style('.seq-label'), time:style('.seq-time')};
                    };
                    return [inspect('sim_sequence'), inspect('macro_compare_sequence')];
                }""")
                assert styles[0] == styles[1], {"mode": mode, "label": label, "styles": styles}
            page.evaluate("setSeqDisplayMode('icon'); setSeqIconLabel('name'); setSeqHelpersEnabled(true)")
            page.locator("#seq_toolbar_toggle").click()
            page.locator("#seq_zoom_in").click()
            page.wait_for_function("""() => getComputedStyle(document.getElementById('sim_sequence')).getPropertyValue('--seq-scale') ===
                getComputedStyle(document.getElementById('macro_compare_sequence')).getPropertyValue('--seq-scale')""")
            assert page.evaluate("parseFloat(getComputedStyle(document.getElementById('macro_compare_sequence')).getPropertyValue('--seq-scale'))") > 1
            page.locator("#seq_zoom_value").click()
            page.keyboard.press("Escape")

            previous_result = page.evaluate("JSON.stringify(window.Jx3MacroEditor.getResult().actual)")
            previous_blocks = comparison.locator(".sim-seq-item").count()
            draft.fill("/cast 盾压\n/cast 盾刀")
            expect(page.locator("#macro_compare_status")).to_contain_text(re.compile("未重跑|过期|已修改|变化"))
            assert comparison.locator(".sim-seq-item").count() == previous_blocks
            assert page.evaluate("JSON.stringify(window.Jx3MacroEditor.getResult().actual)") == previous_result

            for width in [1440, 900]:
                page.set_viewport_size({"width": width, "height": 1000})
                bounds = page.evaluate("""() => {
                    const box = id => { const r = document.getElementById(id).getBoundingClientRect(); return {left:r.left,right:r.right,top:r.top,bottom:r.bottom,height:r.height,width:r.width}; };
                    return {sequence:box('sim_sequence'), inspector:box('macro_assist_panel'), edit:box('macro_assist_edit'),
                        entry:box('macro_assist_toggle'), toolbar:box('seq_float_toolbar'), comparison:box('macro_compare_panel'),
                        editor:box('macro_editor_panel'), draft:box('macro_draft_shield')};
                }""")
                assert bounds["entry"]["bottom"] <= bounds["sequence"]["top"] + 1, bounds
                assert bounds["edit"]["bottom"] <= bounds["sequence"]["top"] + 1, bounds
                assert bounds["sequence"]["bottom"] <= bounds["toolbar"]["top"] + 1, bounds
                assert bounds["toolbar"]["bottom"] <= bounds["inspector"]["top"] + 1, bounds
                assert bounds["inspector"]["height"] >= 150, bounds
                assert bounds["sequence"]["right"] <= bounds["comparison"]["left"] + 1, bounds
                assert bounds["inspector"]["right"] <= bounds["editor"]["left"] + 1, bounds
                assert bounds["comparison"]["bottom"] <= bounds["editor"]["top"] + 1, bounds
                assert bounds["draft"]["width"] >= 40 and bounds["draft"]["height"] >= 75, bounds
            page.set_viewport_size({"width": 1440, "height": 1000})

            # Escape used to close the display toolbar intentionally also clears
            # the assistant pick, so restore it before testing placement modes.
            before = last_response_id()
            drag_box(items.nth(0).bounding_box(), items.nth(1).bounding_box())
            wait_result(0, 1, 0, before)

            # Returning to edit restores the original rectangle popup.
            edit.click()
            expect(panel).to_be_hidden()
            expect(page.locator("#sim_sequence .ma-compare-gap")).to_have_count(0)
            drag_box(items.nth(0).bounding_box(), items.nth(1).bounding_box())
            expect(page.locator(".seq-select-popup")).to_be_visible()
            page.locator(".seq-select-popup").get_by_role("button", name="取消", exact=True).click()
            assert page.evaluate("readSequence()") == original_sequence

            # Switching mode from either placement action cannot mutate sequence.
            for action in ["复制", "移动"]:
                drag_box(items.nth(0).bounding_box(), items.nth(1).bounding_box())
                page.locator(".seq-select-popup").get_by_role("button", name=action, exact=True).click()
                expect(page.locator(".seq-placement-preview")).to_be_visible()
                write.click()
                expect(write).to_have_attribute("aria-pressed", "true")
                assert page.evaluate("readSequence()") == original_sequence
                expect(page.locator(".seq-placement-preview")).to_have_count(0)
                assert_preview(["盾压", "盾刀"], [0, 1])
                edit.click()

                # Clicking the already-active edit button cancels placement too.
                drag_box(items.nth(0).bounding_box(), items.nth(1).bounding_box())
                page.locator(".seq-select-popup").get_by_role("button", name=action, exact=True).click()
                expect(page.locator(".seq-placement-preview")).to_be_visible()
                edit.click()
                expect(edit).to_have_attribute("aria-pressed", "true")
                expect(panel).to_be_hidden()
                expect(page.locator(".seq-placement-preview, #sim_sequence .seq-selected")).to_have_count(0)
                assert page.evaluate("readSequence()") == original_sequence
                write.click()
                assert_preview(["盾压", "盾刀"], [0, 1])
                edit.click()

            responses = page.evaluate("window.__macroAssistSmoke.responses.map(entry => entry.data)")
            expressions = []
            for response in responses:
                for step_response in response.get("steps", [response]):
                    for candidate in step_response.get("candidates", []):
                        expressions.extend([candidate["expression"], candidate["macro_text"]])
                        expressions.extend(candidate.get("equivalent_expressions", []))
                expressions.extend(program["macro_text"] for program in response.get("programs", []))
            assert expressions, "No copyable candidate conditions were checked"
            for expression in expressions:
                assert_macro_syntax(expression)
            assert not failures, failures
            assert all(not any(key.startswith(DRAFT_PREFIX) for key in snapshot) for snapshot in settings_posts)
            print(json.dumps({"ok": True, "version": version, "analysis_requests": len(calls), "write_requests_intercepted": len(writes),
                "copyable_expressions_checked": len(expressions),
                "checks": ["single immediate preview", "rectangle immediate preview", "wrapped center drag", "ignore passive display",
                    "all occurrences", "per step", "same skill outside combo contrast", "state details", "evidence", "copy and equivalent copy",
                    "fixed mode buttons", "preserve selection step and results", "reuse unchanged result", "automatic environment update",
                    "late response cannot overwrite new selection", "Escape cancels late response", "layout 1440/900", "edit restored",
                    "invalid and noncontiguous selection clears feedback", "cancel copy and move placement on mode switch",
                    "active edit button cancels placement", "named buff references", "one decimal bufftime including equivalents",
                    "multi-line programs default and locally expanded steps", "insert undo and import draft", "real macro run preserves template and saved macro",
                    "original sequence styles reused", "resource differences and macro-line navigation", "old result retained and stale after draft edits",
                    "four themes resolve semantic colors and readable text", "shared icon text helpers and zoom settings",
                    "settings restore and upload exclude all account draft keys"]},
                ensure_ascii=False))
        finally:
            browser.close()


if __name__ == "__main__":
    args = argparse.ArgumentParser()
    args.add_argument("--base-url", required=True)
    args.add_argument("--version", choices=["AnYingQianJi", "CangShengZhuShiTest"], default="AnYingQianJi")
    args.add_argument("--storage-only", action="store_true", help="Run only the local draft/settings synchronization boundary checks")
    options = args.parse_args()
    run(options.base_url.rstrip("/"), options.version, options.storage_only)
