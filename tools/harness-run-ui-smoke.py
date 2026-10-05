"""Real browser/HTTP Harness regression against an explicitly isolated local worker.

No API mocking and no Agent sessions are created. The offline provider invokes the
real experiment operators. Run records go to the worker's disposable userdata.
"""
import argparse
import json
from pathlib import Path
from urllib.parse import urlsplit
from playwright.sync_api import sync_playwright


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--backend', default='http://127.0.0.1:3319')
    parser.add_argument('--isolated-worker', action='store_true', required=True,
                        help='Confirm this server uses disposable userdata, not real user data.')
    parser.add_argument('--screenshots', type=Path)
    args = parser.parse_args()
    assert urlsplit(args.backend).hostname in ('localhost', '127.0.0.1'), 'Local isolated worker only'
    checks = []
    with sync_playwright() as pw:
        browser = pw.chromium.launch(channel='msedge', headless=True)
        page = browser.new_page(viewport={'width': 1480, 'height': 1000})
        page.emulate_media(reduced_motion='reduce')
        errors = []
        page.on('pageerror', lambda error: errors.append(str(error)))
        try:
            page.goto(args.backend, wait_until='networkidle')
            page.wait_for_function('window.Jx3Assistant && window.Jx3HarnessWorkspace && document.getElementById("hr_goal")', timeout=30000)
            page.evaluate('''async () => {
                await Promise.all([currentMountReady, attributesReady]);
                Jx3Nav.switchPage('page-sim');
                const cfg = buildLoopConfig();
                cfg.sequence = [{type:'skill', skill:'盾击', count:2}];
                cfg.macro = {mode:'general',general:'',shield:'',blade:''};
                cfg.macro_duration = 0;
                applyLoopConfig(cfg,{skipSimulate:true});
                await runSimulate();
            }''')
            baseline = page.evaluate('async () => await Jx3HarnessWorkspace.capture()')
            assert baseline['simulation']['sequence'] == ['盾击', '盾击'], baseline['simulation']['sequence']
            checks.append('fresh manual scene capture')
            page.evaluate("Jx3Assistant.open('harness'); document.querySelector('[data-assistant-reset]').click(); Jx3Assistant.close()")
            ball = page.locator('#assistant_ball')
            assert ball.count() == 1 and not page.locator('#sim_ai_fab').is_visible()
            box = ball.bounding_box()
            page.mouse.move(box['x']+28, box['y']+28)
            page.mouse.down()
            page.mouse.move(box['x']-120+28, box['y']-85+28, steps=8)
            page.mouse.up()
            assert not page.locator('#assistant_shell').is_visible()
            moved = ball.bounding_box()
            assert moved['x'] < box['x']-80 and moved['y'] < box['y']-50
            ball.click()
            page.locator('#assistant_shell').wait_for(state='visible')
            panel = page.locator('#assistant_shell')
            before = panel.bounding_box()
            handle = page.locator('.assistant-handle').bounding_box()
            page.mouse.move(handle['x']+80, handle['y']+20)
            page.mouse.down()
            page.mouse.move(handle['x']-120+80, handle['y']-35+20, steps=8)
            page.mouse.up()
            after = panel.bounding_box()
            assert after['x'] < before['x']-80 and after['y'] < before['y']-20, (before, after)
            edge = page.locator('.assistant-resize-left').bounding_box()
            page.mouse.move(edge['x']+5, edge['y']+60)
            page.mouse.down()
            page.mouse.move(edge['x']-100+5, edge['y']+60, steps=8)
            page.mouse.up()
            assert panel.bounding_box()['width'] > after['width']+60
            checks.append('single draggable ball, movable/resizable shell')
            page.locator('#assistant_tab_analysis').click()
            assert page.locator('#assistant_analysis_panel #sim_ai_dock').is_visible()
            assert page.locator('#sim_ai_question').is_visible()
            page.locator('#sim_ai_question').fill('保留未发送的分析草稿')
            page.locator('#assistant_tab_harness').click()
            page.locator('#assistant_tab_analysis').click()
            assert page.locator('#sim_ai_question').input_value() == '保留未发送的分析草稿'
            page.locator('#assistant_tab_harness').click()
            page.locator('#hr_settings').click()
            page.wait_for_function('!!document.querySelector("dialog[open], #agent_provider_overlay")')
            # Close the real reused provider settings without saving credentials.
            page.keyboard.press('Escape')
            if not page.locator('#hr_goal').is_visible():
                page.evaluate("Jx3Assistant.open('harness')")
            checks.append('old AI draft survives mode switching; provider settings reused')
            page.wait_for_function('Array.from(document.querySelector("#hr_provider").options).some(o => o.value === "offline" && !o.disabled)')
            page.locator('#hr_provider').select_option('offline')
            page.locator('#hr_goal').fill('把当前技能轴写成宏，验证动作顺序、时机和资源。')
            page.locator('#hr_start').click()
            page.wait_for_function('document.querySelector("#hr_run").hidden === false', timeout=30000)
            page.wait_for_function('document.querySelector("#hr_status").dataset.running === "false"', timeout=120000)
            assert page.locator('#hr_events li').count() >= 2
            assert page.locator('#hr_delivery').is_visible()
            run_id = page.locator('#hr_recent').input_value()
            if not run_id:
                run_id = page.evaluate('async () => (await (await fetch("/api/harness/runs")).json()).runs[0].run_id')
            snapshot = page.evaluate('async id => await (await fetch(`/api/harness/runs/${id}`)).json()', run_id)
            assert snapshot['status'] == 'completed', json.dumps(snapshot, ensure_ascii=False)
            assert snapshot['usage']['model_calls'] >= 3
            assert snapshot['artifacts'], snapshot
            checks.append('real offline model loop → experiment → evidence → delivery')
            page.locator('#hr_preview').click()
            page.locator('.hr-dialog').wait_for(state='visible')
            page.locator('.hr-dialog button', has_text='确认应用方案').click()
            page.wait_for_function('document.querySelector("#hr_undo").disabled === false || !!document.querySelector(".hr-dialog .hr-feedback[data-error=true]")', timeout=30000)
            error = page.locator('.hr-dialog .hr-feedback[data-error=true]')
            assert error.count() == 0, error.all_text_contents()
            applied = page.evaluate('async () => await Jx3HarnessWorkspace.capture()')
            assert applied['simulation'].get('macro_text'), applied
            assert applied['simulation']['sequence'] != baseline['simulation']['sequence']
            # A normal main-editor recompute must keep the exact installed duration and attributes.
            replay = page.evaluate('''async () => {const r=await runSimulate(); return {body:window._lastSimBody,dps:r?.dps};}''')
            assert replay['body'].get('macro_duration') == applied['simulation'].get('macro_duration')
            assert replay['body']['attributes'] == applied['simulation']['attributes']
            checks.append('server transaction → real editor application → normal replay preserves scene')
            page.locator('#hr_undo').click()
            page.wait_for_function('document.querySelector("#hr_feedback").textContent.includes("已恢复应用前")', timeout=30000)
            restored = page.evaluate('async () => await Jx3HarnessWorkspace.capture()')
            assert restored['sourceKey'] == baseline['sourceKey'], 'Undo did not restore original environment/axis/equipment'
            checks.append('server undo transaction and actual workspace restoration')
            # Candidate from a frozen run must never overwrite a subsequent edit.
            page.evaluate("document.getElementById('sim_delay').value=123")
            page.locator('#hr_preview').click()
            page.wait_for_function('document.querySelector("#hr_feedback").dataset.error === "true"')
            assert '改变' in page.locator('#hr_feedback').inner_text()
            assert not page.locator('.hr-dialog').is_visible()
            checks.append('stale candidate cannot overwrite later edits')
            page.evaluate("document.getElementById('sim_delay').value='"+str(baseline['simulation']['network_delay'])+"'")
            with page.expect_download() as download:
                page.locator('#hr_export').click()
            assert download.value.suggested_filename == run_id+'.json'
            checks.append('downloadable full experiment artifact')
            # Exercise equipment writeback with a real catalog item, not invented stats.
            equipment_baseline = page.evaluate('''async () => {
                const response = await fetch('/api/equip/search',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({sub_type:0,min_level:0,max_level:0,schools:['苍云'],kinds:['外功'],attrs:[],battle_types:[],keyword:'',categories:[]})});
                if (!response.ok) throw new Error('Cannot load real equipment catalog');
                const items = await response.json();
                const weapon = items.filter(i=>i.id>0).sort((a,b)=>a.level-b.level || a.id-b.id)[0];
                if (!weapon) throw new Error('No real FenShan weapon fixture in current catalog');
                const equipment = Jx3HarnessWorkspace.equipmentSnapshot({slots:{},stoneId:0});
                equipment.slots.PRIMARY_WEAPON.equip_id=weapon.id;
                const calculated = await Jx3Equip.applyConfig({slots:equipment.slots,stoneId:0,stoneName:''});
                if (!calculated?.raw) throw new Error('Cannot apply real equipment fixture');
                const cfg = buildLoopConfig();
                cfg.sequence=[{type:'macro',count:64}];
                cfg.macro={mode:'general',general:'/cast 盾击',shield:'',blade:''};cfg.macro_duration=10;
                applyLoopConfig(cfg,{skipSimulate:true});await runSimulate();
                return await Jx3HarnessWorkspace.capture();
            }''')
            page.locator('.hr-constraints > summary').click()
            page.locator('#hr_duration').fill('10')
            page.locator('#hr_sims').fill('32')
            page.evaluate("document.querySelectorAll('#hr_locks input').forEach(el=>el.checked=el.value!=='PRIMARY_WEAPON')")
            page.locator('.hr-constraints > summary').click()
            page.locator('#hr_goal').fill('配装实验：保持技能宏，搜索主武器候选并验证真实面板与输出。')
            page.locator('#hr_start').click()
            page.wait_for_function('document.querySelector("#hr_status").dataset.running === "true" || document.querySelector("#hr_conclusion").textContent.includes("离线")',timeout=30000)
            page.wait_for_function('document.querySelector("#hr_start").disabled === false && document.querySelector("#hr_status").dataset.running === "false"',timeout=120000)
            equipment_run = page.evaluate('async () => (await (await fetch("/api/harness/runs")).json()).runs[0]')
            assert equipment_run['run_id'] != run_id
            full_equipment_run = page.evaluate('async id => await (await fetch(`/api/harness/runs/${id}/artifacts`)).json()',equipment_run['run_id'])
            equipment_artifacts = [a for a in full_equipment_run['artifacts'] if a['kind']=='optimize_equipment']
            assert equipment_artifacts and equipment_artifacts[-1]['result']['best']['verified'], full_equipment_run
            page.locator('#hr_artifacts').select_option(equipment_artifacts[-1]['id'])
            page.locator('#hr_preview').click()
            page.locator('.hr-dialog').wait_for(state='visible')
            page.locator('.hr-dialog button',has_text='确认应用方案').click()
            page.wait_for_function('document.querySelector("#hr_undo").disabled === false || !!document.querySelector(".hr-dialog .hr-feedback[data-error=true]")',timeout=30000)
            assert page.locator('.hr-dialog .hr-feedback[data-error=true]').count()==0, page.locator('.hr-dialog .hr-feedback').all_text_contents()
            equipment_applied = page.evaluate('async () => await Jx3HarnessWorkspace.capture()')
            candidate = equipment_artifacts[-1]['result']['best']
            assert equipment_applied['equipment']['slots']==candidate['equipment']['slots'], {position:{'actual':equipment_applied['equipment']['slots'][position],'expected':candidate['equipment']['slots'].get(position)} for position in equipment_applied['equipment']['slots'] if equipment_applied['equipment']['slots'][position]!=candidate['equipment']['slots'].get(position)}
            assert equipment_applied['simulation']['attributes']==candidate['simulation']['attributes']
            equipment_replay = page.evaluate('''async () => {const r=await runSimulate();return {body:window._lastSimBody,dps:r?.dps};}''')
            assert equipment_replay['body']['attributes']==candidate['simulation']['attributes']
            page.locator('#hr_undo').click()
            page.wait_for_function('document.querySelector("#hr_feedback").textContent.includes("已恢复应用前")',timeout=30000)
            equipment_restored = page.evaluate('async () => await Jx3HarnessWorkspace.capture()')
            assert equipment_restored['sourceKey']==equipment_baseline['sourceKey']
            checks.append('real catalog equipment experiment, complete-attribute apply/replay/undo')
            if args.screenshots:
                args.screenshots.mkdir(parents=True, exist_ok=True)
                page.evaluate("document.getElementById('assistant_harness_panel').scrollTop=0")
                page.screenshot(path=str(args.screenshots/'harness-desktop.png'))
                page.locator('#hr_delivery').scroll_into_view_if_needed()
                page.screenshot(path=str(args.screenshots/'harness-evidence.png'))
                page.locator('#assistant_tab_analysis').click()
                page.screenshot(path=str(args.screenshots/'analysis-desktop.png'))
                page.locator('#assistant_tab_harness').click()
                page.evaluate("document.getElementById('assistant_harness_panel').scrollTop=0")
            page.set_viewport_size({'width':390,'height':844})
            bounds = panel.bounding_box()
            assert bounds['x'] >= 0 and bounds['x']+bounds['width'] <= 390
            assert page.locator('#assistant_harness_panel').evaluate('el => el.scrollWidth <= el.clientWidth + 1')
            if args.screenshots:
                page.screenshot(path=str(args.screenshots/'harness-mobile.png'))
            checks.append('mobile shell remains within viewport without horizontal overflow')
            assert not errors, errors
        except Exception:
            print(json.dumps({'browser_errors':errors,'feedback':page.locator('#hr_feedback').all_text_contents(),'status':page.locator('#hr_status').all_text_contents()},ensure_ascii=False))
            if args.screenshots:
                args.screenshots.mkdir(parents=True,exist_ok=True)
                page.screenshot(path=str(args.screenshots/'failure.png'))
            raise
        finally:
            browser.close()
    print(json.dumps({'status':'passed','checks':checks,'count':len(checks)},ensure_ascii=False,indent=2))


if __name__ == '__main__':
    main()
