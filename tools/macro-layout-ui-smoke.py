"""Historical v1 workspace layout fixture on an isolated localhost:3037 worker.

The 2026-09-10 redesign replaces the layout assertions with
macro-repair-ui-smoke.py; this file preserves the original layout fixtures.

Usage: python -X utf8 tools/macro-layout-ui-smoke.py --base-url http://127.0.0.1:3037
Requires Playwright and Edge. All persistence writes are intercepted.
"""
import argparse
import json
import re
from pathlib import Path
from urllib.parse import urlsplit

from playwright.sync_api import expect, sync_playwright


FIXTURE = ["盾压", "盾刀", "盾刀", "盾压", "盾刀", "盾压", "盾刀", "盾刀"] * 3
SPLITS = ["macro_split_top_width", "macro_split_bottom_width", "macro_split_height", "macro_split_editor_width"]


def contrast_ratio(first, second):
    def luminance(color):
        values = [float(value) for value in re.findall(r'-?\d+(?:\.\d+)?', color)][:3]
        if not color.startswith('color('):
            values = [value / 255 for value in values]
        return sum((value / 12.92 if value <= .04045 else ((value + .055) / 1.055) ** 2.4) * weight
                   for value, weight in zip(values, [.2126, .7152, .0722]))
    a, b = luminance(first), luminance(second)
    return (max(a, b) + .05) / (min(a, b) + .05)


def run(base_url, review_only=False, draft_panes_only=False):
    parsed = urlsplit(base_url)
    assert parsed.scheme == "http" and parsed.hostname in {"127.0.0.1", "localhost"} and parsed.port == 3037, \
        "Use only the isolated localhost:3037 worker"
    with sync_playwright() as playwright:
        api = playwright.request.new_context(base_url=base_url)
        response = api.post("/api/mounts/switch", data={"version": "AnYingQianJi", "mount": "FenShanJin", "persist": False})
        assert response.ok and response.json().get("ok"), response.text()
        api.dispose()
        browser = playwright.chromium.launch(channel="msedge", headless=True)
        try:
            page = browser.new_page(viewport={"width": 1440, "height": 1000})
            page.set_default_timeout(20000)
            errors, writes, simulations, analyses = [], [], [], []
            page.on("pageerror", lambda error: errors.append(str(error)))

            def route_api(route):
                path = urlsplit(route.request.url).path
                if path == "/api/settings":
                    if route.request.method not in {"GET", "HEAD"}:
                        writes.append(path)
                    route.fulfill(json={})
                elif route.request.method in {"GET", "HEAD"} or path in {
                    "/api/simulate", "/api/macro/assist", "/api/macro/assist/program", "/api/calculate", "/api/skill_damage",
                }:
                    if path == '/api/simulate' and route.request.method == 'POST':
                        simulations.append(route.request.post_data_json)
                    elif path in {'/api/macro/assist','/api/macro/assist/program'}:
                        analyses.append(route.request.post_data_json)
                    route.continue_()
                else:
                    writes.append(path)
                    route.fulfill(json={"ok": True})

            page.route("**/api/**", route_api)

            def settle():
                page.evaluate("() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))")

            def ready():
                page.wait_for_function("!!window.Jx3MacroAssist && !!window.Jx3MacroEditor && !!window.Jx3MacroStateDiff && !!window.Jx3MacroDraftPanes")
                page.evaluate("async () => { await currentMountReady; window.Jx3Nav.switchPage('page-sim'); }")

            def geometry():
                return page.evaluate("""() => {
                    const box = selector => { const r = document.querySelector(selector).getBoundingClientRect();
                        return {x:r.x,y:r.y,width:r.width,height:r.height,right:r.right,bottom:r.bottom}; };
                    const result = {workspace:box('#macro_workspace'),top:box('.ma-top-row'),bottom:box('.ma-bottom-row'),
                        template:box('.ma-template'),reference:box('#sim_sequence'),actual:box('#macro_compare_sequence'),
                        assistant:box('#macro_assist_panel'),editor:box('#macro_editor_panel'),draft:box('#macro_draft_shield'),
                        bladeDraft:box('#macro_draft_blade'),
                        draftPane:box('.ma-draft-pane'),detail:box('#macro_compare_detail'),
                        title:box('#macro_compare_heading'),mode:box('.ma-mode-switch')};
                    result.ratios = [result.template.width/(result.top.width-8), result.assistant.width/(result.bottom.width-8),
                        result.top.height/(result.workspace.height-8)];
                    return result;
                }""")

            def assert_aligned(bounds):
                assert abs(bounds["reference"]["y"] - bounds["actual"]["y"]) <= 1, bounds
                assert abs(bounds["title"]["y"] - bounds["mode"]["y"]) <= 5, bounds
                assert bounds["title"]["bottom"] <= bounds["actual"]["y"] + 1, bounds
                assert bounds["template"]["right"] <= bounds["actual"]["x"], bounds
                assert bounds["assistant"]["right"] <= bounds["editor"]["x"], bounds
                assert bounds["top"]["bottom"] <= bounds["bottom"]["y"], bounds

            def assert_detail_columns():
                bounds = geometry()
                assert abs(bounds['draftPane']['y']-bounds['detail']['y']) <= 1, bounds
                assert bounds['draftPane']['right'] <= bounds['detail']['x'], bounds
                assert abs(bounds['draftPane']['height']-bounds['editor']['height']) <= 2, bounds
                assert abs(bounds['detail']['height']-bounds['editor']['height']) <= 2, bounds
                assert abs(bounds['draftPane']['width']-bounds['detail']['width']) <= 8, bounds

            def assert_selection_outlines():
                page.wait_for_function("document.querySelectorAll('#sim_sequence .ma-selection-outline').length>0")
                result=page.evaluate("""() => {
                    const kind=node=>['extra','focus','selected','occurrence'].find(value=>node.classList.contains(`ma-${value}`))||null;
                    const box=node=>{const r=node.getBoundingClientRect();return {x:r.x,y:r.y,right:r.right,bottom:r.bottom};};
                    return {items:[...document.querySelectorAll('#sim_sequence > .sim-seq-item')].map(node=>({...box(node),kind:kind(node),outline:getComputedStyle(node).outlineStyle})),
                        outlines:[...document.querySelectorAll('#sim_sequence > .ma-selection-outline')].map(node=>({...box(node),
                            kind:['extra','focus','selected','occurrence'].find(value=>node.classList.contains(`ma-selection-${value}`)),
                            skill:node.classList.contains('sim-seq-item')||node.hasAttribute('data-skill'),pointer:getComputedStyle(node).pointerEvents}))};
                }""")
                segments=[]
                for outline in result['outlines']:
                    assert not outline['skill'] and outline['pointer']=='none', outline
                    contained=[(index,item) for index,item in enumerate(result['items']) if item['x']>=outline['x']-1 and item['right']<=outline['right']+1
                               and item['y']>=outline['y']-1 and item['bottom']<=outline['bottom']+1]
                    assert contained and all(item['kind']==outline['kind'] for _,item in contained), (outline,contained)
                    assert max(item['y'] for _,item in contained)-min(item['y'] for _,item in contained)<=1, (outline,contained)
                    indices=[index for index,_ in contained]
                    assert indices==list(range(indices[0],indices[-1]+1)), indices
                    assert abs(outline['x']-min(item['x'] for _,item in contained))<=1, outline
                    assert abs(outline['right']-max(item['right'] for _,item in contained))<=1, outline
                    assert all(item['outline']=='none' for _,item in contained), contained
                    segments.append(indices)
                for index,item in enumerate(result['items']):
                    if item['kind']:
                        assert sum(index in segment for segment in segments)==1, (index,segments)
                return segments

            def assert_state_field_fixture():
                fixture=page.evaluate(r"""() => {
                    const reference={name:'盾刀',cast_time:10.04,state_before:{rage:80,berserk_value:100,block_value:null,time:10.04,
                        buffs:[{buff_id:1,name:'血怒',stacks:2,remaining:12.04},{buff_id:2,name:'擎盾',stacks:1,remaining:0}],
                        skill_cds:[{name:'盾压',remaining:3}],skill_states:[{skill_id:7,name:'盾飞',charges:3,max_charges:5}]}};
                    const actual=JSON.parse(JSON.stringify(reference));actual.cast_time=10.041;actual.state_before.time=10.041;
                    actual.state_before.rage=60;actual.state_before.buffs.reverse();actual.state_before.buffs[1].remaining=9.96;
                    actual.state_before.buffs.push({buff_id:3,name:'卷云',stacks:1,remaining:4});
                    actual.state_before.skill_cds[0].remaining=5;actual.state_before.skill_states[0].charges=1;
                    const summarize=grid=>[...grid.querySelectorAll('.ma-detail-row')].map(row=>[...row.children].map(cell=>({
                        text:cell.textContent,values:[...cell.querySelectorAll('.ma-detail-value')].map(value=>({text:value.textContent,
                            changed:value.matches('.ma-detail-before,.ma-detail-after')}))})));
                    const changed=Jx3MacroStateDiff.render(reference,actual);
                    const reordered=JSON.parse(JSON.stringify(reference));reordered.state_before.buffs.reverse();
                    const same=Jx3MacroStateDiff.render(reference,reordered);
                    const unknown=Jx3MacroStateDiff.render(reference,{name:'盾刀',cast_time:10.04,state_before:null});
                    return {rows:summarize(changed),reorderedHighlights:same.querySelectorAll('.ma-detail-before,.ma-detail-after').length,
                        unknownActual:[...unknown.querySelectorAll('.ma-detail-row')].map(row=>row.children[1].textContent).join('\n')};
                }""")
                assert fixture['reorderedHighlights']==0,fixture
                rage=next(row for row in fixture['rows'] if row[0]['text'].startswith('怒气'))
                assert rage[0]['values'][0]['changed'] and sum(value['changed'] for value in rage[0]['values'])==1,rage
                assert rage[1]['values'][0]['changed'] and sum(value['changed'] for value in rage[1]['values'])==1,rage
                buff=next(row for row in fixture['rows'] if row[0]['text'].startswith('血怒'))
                assert [value['changed'] for value in buff[0]['values']]==[False,False,True],buff
                charge=next(row for row in fixture['rows'] if row[0]['text'].startswith('盾飞'))
                assert [value['changed'] for value in charge[0]['values']]==[False,True,False],charge
                stamp=next(row for row in fixture['rows'] if row[0]['text'].startswith('释放时间'))
                assert all(not value['changed'] for cell in stamp for value in cell['values']),stamp
                missing_buff=next(row for row in fixture['rows'] if row[1]['text'].startswith('卷云'))
                assert missing_buff[0]['text']=='无' and all(value['changed'] for cell in missing_buff for value in cell['values']),missing_buff
                assert '未记录' in fixture['unknownActual'] and not re.search(r'怒气\s*0|暴怒\s*0|格挡\s*0',fixture['unknownActual']),fixture
                return {'reordered_list_highlights':fixture['reorderedHighlights'],'field_groups':len(fixture['rows'])}

            def drag(identifier, dx, dy):
                handle = page.locator('#' + identifier)
                box = handle.bounding_box()
                x, y = box["x"] + box["width"] / 2, box["y"] + box["height"] / 2
                page.mouse.move(x, y)
                page.mouse.down()
                page.mouse.move(x + dx, y + dy, steps=12)
                page.mouse.up()
                settle()

            def state_unchanged():
                assert page.evaluate("""() => {
                    const before = window.__macroLayoutSmoke;
                    return lastSimResult === before.reference && Jx3MacroEditor.getResult() === before.result
                        && JSON.stringify(readSequence()) === before.sequence && JSON.stringify(macroPages) === before.macros
                        && document.getElementById('macro_draft_text').value === before.draft
                        && Jx3MacroAssist.contextKey() === before.key;
                }"""), "A layout change modified template, draft, current macro, or comparison result"

            def assert_bands():
                page.wait_for_function("document.querySelectorAll('#macro_compare_sequence .ma-band-start').length > 0")
                bands = page.evaluate("""() => ['sim_sequence','macro_compare_sequence'].map(id => {
                    const nodes=[...document.querySelectorAll(`#${id} > .sim-seq-item`)];
                    return nodes.map(item => {
                        const r=item.getBoundingClientRect(),style=getComputedStyle(item);
                        const marked=item.matches('.ma-reference-missing,.ma-reference-changed,.ma-redline-extra,.ma-redline-changed');
                        return {marked,x:r.x,y:r.y,right:r.right,start:item.classList.contains('ma-band-start'),end:item.classList.contains('ma-band-end'),
                            left:parseFloat(style.getPropertyValue('--ma-band-left'))||0,rightExtension:parseFloat(style.getPropertyValue('--ma-band-right'))||0,
                            breakBefore:item.previousElementSibling?.classList.contains('seq-break-wrap')||false};
                    });
                })""")
                joins = row_breaks = normal_breaks = 0
                for nodes in bands:
                    for index, item in enumerate(nodes):
                        if not item['marked']:
                            continue
                        previous = nodes[index-1] if index else None
                        following = nodes[index+1] if index+1 < len(nodes) else None
                        joins_previous = previous and previous['marked'] and abs(previous['y']-item['y']) < 4 and not item['breakBefore']
                        joins_following = following and following['marked'] and abs(following['y']-item['y']) < 4 and not following['breakBefore']
                        assert item['start'] == (not bool(joins_previous)), (index,item,previous)
                        assert item['end'] == (not bool(joins_following)), (index,item,following)
                        if joins_previous:
                            joins += 1
                            assert item['left'] > 0 and previous['rightExtension'] > 0, (previous,item)
                            assert item['left'] + previous['rightExtension'] >= item['x'] - previous['right'] - 2, (previous,item)
                        elif previous:
                            row_breaks += int(previous['marked'] and abs(previous['y']-item['y']) >= 4)
                            normal_breaks += int(not previous['marked'])
                        if item['start']:
                            assert item['left'] == 0, item
                        if item['end']:
                            assert item['rightExtension'] == 0, item
                assert joins > 0 and row_breaks > 0, {"joins":joins,"row_breaks":row_breaks}
                return {"joins":joins,"row_breaks":row_breaks,"normal_breaks":normal_breaks}

            page.goto(base_url, wait_until="networkidle")
            ready()
            if draft_panes_only:
                page.evaluate("""async skills => {
                    document.getElementById('sim_sequence').replaceChildren();
                    skills.forEach(skill => addSeqItem(skill)); await runSimulate();
                }""", FIXTURE)
                page.locator('#macro_assist_toggle').click()
                page.evaluate("""() => Jx3MacroAssist.selectItems([...document.querySelectorAll('#sim_sequence .sim-seq-item:not(.seq-auto):not(.seq-pre-release)')].slice(0,2))""")
                expect(page.locator('.ma-program-code').first).to_be_visible()
                shield=page.locator('#macro_draft_shield')
                blade=page.locator('#macro_draft_blade')
                canonical=page.locator('#macro_draft_text')
                expect(shield).to_be_visible()
                expect(blade).to_be_visible()
                expect(canonical).to_be_hidden()

                # Hidden headers are anchored to line boundaries: deleting the
                # preceding comment's newline must not put a header inside it.
                anchored_source='//comment\n#page shield\n/cast 盾刀'
                page.evaluate("""text => {
                    window.__draftOriginalBuildMacroText=buildMacroText;buildMacroText=()=>text;
                }""",anchored_source)
                try:
                    page.locator('#macro_draft_import').click()
                finally:
                    page.evaluate("buildMacroText=window.__draftOriginalBuildMacroText;delete window.__draftOriginalBuildMacroText")
                expect(canonical).to_have_value(anchored_source)
                expect(shield).to_have_value('//comment\n/cast 盾刀')
                shield.fill('//comment/cast 盾刀')
                expect(canonical).to_have_value('#page shield\n//comment/cast 盾刀')
                expect(shield).to_have_value('//comment/cast 盾刀')
                page.locator('#macro_draft_undo').click()
                expect(canonical).to_have_value(anchored_source)
                expect(shield).to_have_value('//comment\n/cast 盾刀')

                # The import preserves comments, empty lines, headers and order.
                source=page.evaluate(r"""() => {
                    macroMode='stance';
                    macroPages.shield='// 盾侧注释\n\n/cast [rage>40] 盾飞\n/cast 盾刀';
                    macroPages.blade='// 刀侧注释\n\n/cast 盾回';
                    return buildMacroText();
                }""")
                page.locator('#macro_draft_import').click()
                expect(canonical).to_have_value(source)
                boundary=source.index('#page blade')
                assert shield.input_value()==source[:boundary].removeprefix('#page shield\n')
                assert blade.input_value()==source[boundary:].removeprefix('#page blade\n')
                assert '#page' not in shield.input_value()+blade.input_value()
                page.evaluate("""() => Jx3MacroAssist.selectItems([...document.querySelectorAll('#sim_sequence .sim-seq-item:not(.seq-auto):not(.seq-pre-release)')].slice(0,2))""")
                page.wait_for_function('!!Jx3MacroAssist.getReference()')
                expect(page.locator('.ma-program-code').first).to_be_visible()
                original_shield,original_blade=shield.input_value(),blade.input_value()
                shield.fill(original_shield+'\n// 左栏编辑')
                expect(canonical).to_have_value('#page shield\n'+original_shield+'\n// 左栏编辑\n'+source[boundary:])
                expect(blade).to_have_value(original_blade)
                page.locator('#macro_draft_undo').click()
                expect(canonical).to_have_value(source)
                blade.fill(original_blade+'\n// 右栏编辑')
                expect(canonical).to_have_value(source+'\n// 右栏编辑')
                expect(shield).to_have_value(original_shield)
                blade.press('Control+z')
                expect(canonical).to_have_value(source)

                # Shared Copy exports the complete draft, including both pages.
                page.evaluate("""() => {
                    window.__draftCopied=null;
                    Object.defineProperty(navigator,'clipboard',{configurable:true,value:{writeText:async text=>{window.__draftCopied=text;}}});
                }""")
                page.locator('#macro_draft_copy').click()
                assert page.evaluate('window.__draftCopied')==source

                # Candidate buttons keep the most recently focused pane/selection.
                for pane,other in [(shield,blade),(blade,shield)]:
                    pane.focus()
                    pane.evaluate("node => node.setSelectionRange(node.value.length,node.value.length)")
                    other_before=other.input_value()
                    before=canonical.input_value()
                    candidate=page.locator('.ma-program-code').first.inner_text()
                    page.locator('.ma-program').first.get_by_role('button',name='插入宏',exact=True).click()
                    assert pane.input_value().endswith(candidate), {'pane':pane.get_attribute('id'),'value':pane.input_value(),'candidate':candidate,'canonical':canonical.input_value(),'status':page.locator('#macro_assist_status').inner_text()}
                    expect(other).to_have_value(other_before)
                    page.locator('#macro_draft_undo').click()
                    expect(canonical).to_have_value(before)

                # A new blade pane supplies its own page only in the full text.
                blade.fill('/cast 盾回')
                assert canonical.input_value()==source[:boundary]+'#page blade\n/cast 盾回'
                expect(shield).to_have_value(original_shield)
                page.locator('#macro_draft_undo').click()
                expect(canonical).to_have_value(source)

                page.evaluate("""() => window.__draftPanesBefore={reference:lastSimResult,
                    sequence:JSON.stringify(readSequence()),macros:JSON.stringify(macroPages),key:Jx3MacroAssist.contextKey()}""")
                calls_before=len(simulations)
                blade.press('Control+Enter')
                page.wait_for_function("!!Jx3MacroEditor.getResult()?.actual?.timeline?.length")
                assert len(simulations)>calls_before
                assert simulations[-1]['macro_text']==source
                assert page.evaluate("""() => lastSimResult===__draftPanesBefore.reference
                    && JSON.stringify(readSequence())===__draftPanesBefore.sequence
                    && JSON.stringify(macroPages)===__draftPanesBefore.macros
                    && Jx3MacroAssist.contextKey()===__draftPanesBefore.key""")
                page.locator('#macro_compare_layer').select_option('states')
                located=[]
                for page_number,pane in [(1,shield),(2,blade)]:
                    index=page.evaluate("""number => {
                        const result=Jx3MacroEditor.getResult();
                        return result.alignment.rows.findIndex(row=>row.actualIndex!=null&&result.actual.timeline[row.actualIndex].macro_page===number);
                    }""",page_number)
                    assert index>=0, {'missing_macro_page':page_number}
                    page.locator(f'#macro_compare_sequence [data-compare-row="{index}"]').click()
                    expected_line=page.evaluate("""index=>{
                        const result=Jx3MacroEditor.getResult(),row=result.alignment.rows[index],event=result.actual.timeline[row.actualIndex];
                        return result.pages[event.macro_page-1][event.macro_line-1];
                    }""",index)
                    locate=page.locator('#macro_compare_detail').get_by_role('button',name=re.compile('定位.*第.*行'))
                    label=locate.inner_text()
                    line_number=int(re.search(r'\d+',label).group())
                    locate.click()
                    expect(pane).to_be_focused()
                    selected=pane.evaluate('node=>node.value.slice(node.selectionStart,node.selectionEnd)')
                    assert selected==expected_line['text']==source.split('\n')[expected_line['number']-1], {'label':label,'selected':selected}
                    assert selected==pane.input_value().split('\n')[line_number-1],{'label':label,'selected':selected}
                    located.append({'page':page_number,'physical_line':expected_line['number'],'visible_line':line_number,'selected':selected})

                # Starting from two blank panes creates two active stance pages;
                # an unrestricted first page must not mask the new blade page.
                page.evaluate("macroMode='general';macroPages.general=''")
                page.locator('#macro_draft_import').click()
                expect(canonical).to_have_value('')
                expect(shield).to_have_value('')
                expect(blade).to_have_value('')
                plain_shield='/cast [rage>40] 盾飞\n/cast 盾刀'
                plain_blade='/cast 盾回'
                shield.fill(plain_shield)
                expect(canonical).to_have_value(plain_shield)
                blade.fill(plain_blade)
                fresh_source='#page shield\n'+plain_shield+'\n#page blade\n'+plain_blade
                expect(canonical).to_have_value(fresh_source)
                expect(shield).to_have_value(plain_shield)
                expect(blade).to_have_value(plain_blade)
                blade.press('Control+z')
                expect(canonical).to_have_value(plain_shield)
                expect(shield).to_have_value(plain_shield)
                expect(blade).to_have_value('')
                blade.fill(plain_blade)
                page.evaluate("""() => Jx3MacroAssist.selectItems([...document.querySelectorAll('#sim_sequence .sim-seq-item:not(.seq-auto):not(.seq-pre-release)')].slice(0,2))""")
                page.wait_for_function('!!Jx3MacroAssist.getReference()')
                page.evaluate("""() => window.__draftPanesBefore={reference:lastSimResult,
                    sequence:JSON.stringify(readSequence()),macros:JSON.stringify(macroPages),key:Jx3MacroAssist.contextKey()}""")
                page.locator('#macro_compare_run').click()
                page.wait_for_function('text=>Jx3MacroEditor.getResult()?.text===text',arg=fresh_source)
                assert simulations[-1]['macro_text']==fresh_source
                pages_seen=page.evaluate("""() => [...new Set(Jx3MacroEditor.getResult().actual.timeline
                    .filter(event=>!event.triggered&&event.macro_page>0).map(event=>event.macro_page))]""")
                assert 1 in pages_seen and 2 in pages_seen,pages_seen
                assert page.evaluate("""() => lastSimResult===__draftPanesBefore.reference
                    && JSON.stringify(readSequence())===__draftPanesBefore.sequence
                    && JSON.stringify(macroPages)===__draftPanesBefore.macros
                    && Jx3MacroAssist.contextKey()===__draftPanesBefore.key""")
                fresh_blade_row=page.evaluate("""() => {
                    const result=Jx3MacroEditor.getResult();return result.alignment.rows.findIndex(row=>
                        row.actualIndex!=null&&result.actual.timeline[row.actualIndex].macro_page===2);
                }""")
                page.locator(f'#macro_compare_sequence [data-compare-row="{fresh_blade_row}"]').click()
                fresh_locate=page.locator('#macro_compare_detail').get_by_role('button',name=re.compile('定位.*第.*行'))
                fresh_line=int(re.search(r'\d+',fresh_locate.inner_text()).group())
                fresh_locate.click()
                expect(blade).to_be_focused()
                assert blade.evaluate('node=>node.value.slice(node.selectionStart,node.selectionEnd)')==plain_blade==blade.input_value().split('\n')[fresh_line-1]
                located.append({'page':2,'visible_line':fresh_line,'new_draft':True})
                page.locator('#macro_compare_sequence [data-compare-row]').first.click()

                # Two visible editors stay horizontally separated in every theme
                # and both viewport sizes, including when the detail is open.
                for width in [1440,900]:
                    page.set_viewport_size({'width':width,'height':1100})
                    settle()
                    bounds=geometry()
                    left,right=bounds['draft'],bounds['bladeDraft']
                    assert abs(left['y']-right['y'])<=1 and left['right']<=right['x'],bounds
                    assert left['width']>=40 and right['width']>=40,bounds
                    assert right['right']<=bounds['draftPane']['right']+1,bounds
                    for theme in ['', 'pink-theme','light-theme','indigo-theme']:
                        colors=page.evaluate("""theme => {
                            document.body.classList.remove('pink-theme','light-theme','indigo-theme');if(theme)document.body.classList.add(theme);
                            return ['macro_draft_shield','macro_draft_blade'].map(id=>{const style=getComputedStyle(document.getElementById(id));
                                return {color:style.color,background:style.backgroundColor,border:style.borderTopColor,width:parseFloat(style.borderTopWidth)};});
                        }""",theme)
                        assert colors[0]==colors[1] and colors[0]['width']>=1,colors
                        assert all(contrast_ratio(item['color'],item['background'])>=4.5 for item in colors),colors

                page.set_viewport_size({'width':1800,'height':1100})
                page.evaluate("document.body.classList.remove('pink-theme','indigo-theme');document.body.classList.add('light-theme')")
                drag('macro_split_bottom_width',-150,0)
                drag('macro_split_editor_width',90,0)
                page.locator('#macro_split_height').press('Home')
                page.evaluate("""() => Jx3MacroAssist.selectItems([...document.querySelectorAll('#sim_sequence .sim-seq-item:not(.seq-auto):not(.seq-pre-release)')].slice(0,2))""")
                expect(page.locator('.ma-program-code').first).to_be_visible()
                page.wait_for_timeout(220)
                page.evaluate("""() => {
                    const make=(rage,cd)=>({name:'盾刀',cast_time:1.4,state_before:{rage,buffs:[],target_buffs:[],
                        skill_cds:[{name:'盾压',remaining:cd}],skill_states:[]}});
                    document.querySelector('#macro_compare_detail .ma-detail-grid').replaceWith(Jx3MacroStateDiff.render(make(20,3),make(10,1)));
                    document.getElementById('macro_compare_detail').scrollTop=0;
                    document.getElementById('sim_sequence').scrollTop=0;
                    document.getElementById('macro_compare_sequence').scrollTop=0;
                }""")
                assert_selection_outlines()
                page.mouse.move(40,40)
                page.wait_for_timeout(250)
                screenshot=Path(__file__).resolve().parents[1]/'backend'/'target'/'macro-draft-panes-review-1800.png'
                page.screenshot(path=str(screenshot),full_page=True)
                assert not errors,errors
                print(json.dumps({'ok':True,'draft_panes':True,'located':located,'screenshot':str(screenshot),
                    'checks':['hidden header stays on line boundary when comment newline is removed','lossless import','both pane edits and undo','full draft copy','focused pane candidate insertion',
                              'automatic blade page','real two-page run preserves template','physical line navigation',
                              'four themes and 1440/900 side-by-side panes'],
                    'write_requests_intercepted':len(writes)},ensure_ascii=False))
                return
            # Both sides genuinely cast the same channel skill once; the manual
            # one-tick override creates only a state/channel difference.
            page.evaluate("""async () => {
                document.getElementById('sim_sequence').replaceChildren();
                addSeqItem('盾舞'); channelOverrides[0]=1; await runSimulate();
            }""")
            page.locator('#macro_assist_toggle').click()
            expect(page.locator('#macro_compare_layer')).to_have_value('skills')
            expect(page.locator('#macro_compare_sync')).to_be_checked()
            page.locator('#macro_draft_shield').fill('/cast 盾舞')
            page.evaluate("async () => { await Jx3MacroEditor.run(); }")
            pure_state = page.evaluate("Jx3MacroEditor.getResult()?.alignment.summary")
            assert pure_state and pure_state['missing'] == 0 and pure_state['extra'] == 0 and pure_state['changed'] > 0, pure_state
            expect(page.locator('.ma-reference-changed,.ma-redline-changed')).to_have_count(0)
            expect(page.locator('#macro_compare_first')).to_be_disabled()
            calls_before_layer = len(simulations)
            page.locator('#macro_compare_layer').select_option('states')
            expect(page.locator('#macro_compare_sequence .ma-redline-changed')).to_have_count(1)
            expect(page.locator('#macro_compare_first')).to_be_enabled()
            page.locator('#macro_compare_first').click()
            expect(page.locator('#macro_compare_detail')).to_contain_text('时间或状态不同')
            page.locator('#macro_compare_layer').select_option('skills')
            expect(page.locator('.ma-reference-changed,.ma-redline-changed')).to_have_count(0)
            expect(page.locator('#macro_compare_first')).to_be_disabled()
            assert len(simulations) == calls_before_layer, "Switching difference layers re-ran simulation"
            page.locator('#macro_assist_edit').click()
            page.evaluate("""async names => {
                document.getElementById('sim_sequence').replaceChildren();
                Object.keys(channelOverrides).forEach(key => delete channelOverrides[key]);
                names.forEach(name => addSeqItem(name));
                await runSimulate();
            }""", FIXTURE)
            page.locator('#macro_assist_toggle').click()
            page.evaluate("""() => Jx3MacroAssist.selectItems([...document.querySelectorAll('#sim_sequence .sim-seq-item:not(.seq-auto):not(.seq-pre-release)')].slice(0,2))""")
            expect(page.locator('.ma-program-code').first).to_be_visible()
            page.locator('#macro_draft_blade').fill('')
            page.locator('#macro_draft_shield').fill('/cast 盾刀')
            page.evaluate("async () => { await Jx3MacroEditor.run(); }")
            page.wait_for_function("!!Jx3MacroEditor.getResult()?.actual?.timeline?.length")
            expected_actual = page.evaluate("Jx3MacroEditor.getResult().alignment.rows.filter(row => row.actualIndex != null).length")
            expect(page.locator('#macro_compare_sequence .sim-seq-item:not(.seq-pre-release)')).to_have_count(expected_actual)
            expect(page.locator('#macro_compare_sequence .ma-redline-missing, #sim_sequence .ma-compare-gap')).to_have_count(0)
            expect(page.locator('.ma-diff-badge')).to_have_count(0)
            assert page.locator('#sim_sequence .seq-rage-chip').count()>0
            assert page.locator('#macro_compare_sequence .seq-rage-chip').count()>0
            expect(page.locator('#macro_compare_detail')).to_be_hidden()
            expect(page.locator('#macro_split_editor_width')).to_be_hidden()
            without_detail=geometry()
            assert without_detail['draftPane']['width']>=without_detail['editor']['width']-8,without_detail
            assert page.locator('#sim_sequence .ma-reference-missing').count() > 0
            expect(page.locator('.ma-reference-changed,.ma-redline-changed')).to_have_count(0)
            page.locator('#macro_compare_first').click()
            expect(page.locator('#macro_compare_detail')).to_contain_text('模板技能缺失')
            page.locator('#sim_sequence .ma-reference-missing').first.click()
            expect(page.locator('#macro_compare_detail')).to_contain_text('此侧没有对应释放')
            initial_bands = assert_bands()
            assert initial_bands['normal_breaks'] > 0, initial_bands
            before_layer = len(simulations)
            page.locator('#macro_compare_layer').select_option('states')
            page.locator('#macro_compare_sequence .ma-redline-changed').first.click()
            expect(page.locator('#macro_compare_detail')).to_contain_text('实际')
            expect(page.locator('#macro_compare_detail')).to_contain_text('怒气')
            expect(page.locator('#macro_compare_detail .ma-detail-before').first).to_be_visible()
            expect(page.locator('#macro_compare_detail .ma-detail-after').first).to_be_visible()
            state_field_fixture=assert_state_field_fixture()
            assert_detail_columns()
            footer_spacing=page.evaluate("""() => {
                const box=selector=>{const r=document.querySelector(selector).getBoundingClientRect();return {top:r.top,bottom:r.bottom};};
                return {fields:box('#macro_compare_detail .ma-detail-grid'),footer:box('#macro_compare_detail .ma-detail-footer'),
                    macro:box('#macro_compare_detail .ma-detail-macro'),button:box('#macro_compare_detail .ma-detail-footer button'),
                    explanation:box('#macro_compare_detail .ma-detail-footer .ma-explanation')};
            }""")
            assert footer_spacing['footer']['top']-footer_spacing['fields']['bottom']>=13,footer_spacing
            assert footer_spacing['macro']['top']-footer_spacing['footer']['top']>=10,footer_spacing
            assert footer_spacing['button']['top']-footer_spacing['macro']['bottom']>=7,footer_spacing
            assert footer_spacing['explanation']['top']-footer_spacing['button']['bottom']>=7,footer_spacing
            assert len(simulations) == before_layer
            # Clicking a left difference may select a new target for suggestions;
            # restore the pair for the layout checks below.
            page.evaluate("""() => Jx3MacroAssist.selectItems([...document.querySelectorAll('#sim_sequence .sim-seq-item:not(.seq-auto):not(.seq-pre-release)')].slice(0,2))""")
            expect(page.locator('.ma-program-code').first).to_be_visible()
            page.evaluate("""() => window.__macroLayoutSmoke = {
                reference:lastSimResult,result:Jx3MacroEditor.getResult(),sequence:JSON.stringify(readSequence()),
                macros:JSON.stringify(macroPages),draft:document.getElementById('macro_draft_text').value,key:Jx3MacroAssist.contextKey(),
            }""")

            if review_only:
                # The real API comparison above has already been checked. This
                # short display fixture keeps the value highlights and footer
                # visible together for a compact visual review.
                page.set_viewport_size({'width':1440,'height':1100})
                page.evaluate("document.body.classList.remove('pink-theme','indigo-theme');document.body.classList.add('light-theme')")
                page.locator('#macro_split_height').focus()
                page.keyboard.press('Home')
                page.wait_for_timeout(180)
                page.evaluate("""() => {
                    const make=(rage,cd)=>({name:'盾刀',cast_time:1.4,state_before:{rage,buffs:[],target_buffs:[],
                        skill_cds:[{name:'盾压',remaining:cd}],skill_states:[]}});
                    const detail=document.getElementById('macro_compare_detail');
                    detail.querySelector('.ma-detail-grid').replaceWith(Jx3MacroStateDiff.render(make(20,3),make(10,1)));
                    detail.scrollTop=0;document.getElementById('sim_sequence').scrollTop=0;
                    document.getElementById('macro_compare_sequence').scrollTop=0;
                }""")
                settle()
                assert_selection_outlines()
                expect(page.locator('#sim_sequence .ma-selection-selected')).to_have_count(1)
                assert_detail_columns()
                bounds=page.evaluate("""() => ({footer:document.querySelector('.ma-detail-footer').getBoundingClientRect().bottom,
                    detail:document.getElementById('macro_compare_detail').getBoundingClientRect().bottom})""")
                assert bounds['footer']<=bounds['detail']+1,bounds
                page.mouse.move(40,40)
                page.wait_for_timeout(250)
                screenshot=Path(__file__).resolve().parents[1]/'backend'/'target'/'macro-polish-review-1440.png'
                page.screenshot(path=str(screenshot),full_page=True)
                assert not errors,errors
                print(json.dumps({'ok':True,'visual_fixture':'resource and cooldown values via actual detail renderer',
                    'screenshot':str(screenshot),'state_field_fixture':state_field_fixture,'write_requests_intercepted':len(writes)},ensure_ascii=False))
                return

            # One rectangle covers each contiguous selected/matched group. The
            # rectangles are display-only and never become sequence inputs.
            outline_segments=assert_selection_outlines()
            assert any(len(segment)>1 for segment in outline_segments),outline_segments
            expect(page.locator('#sim_sequence .ma-selection-selected')).to_have_count(1)
            before_outlines=(len(simulations),len(analyses))
            page.evaluate("document.getElementById('sim_sequence').style.width='70px'")
            expect(page.locator('#sim_sequence .ma-selection-selected')).to_have_count(2)
            assert_selection_outlines()
            page.evaluate("document.getElementById('sim_sequence').style.width=''")
            expect(page.locator('#sim_sequence .ma-selection-selected')).to_have_count(1)
            page.evaluate("""() => {
                const divider=document.createElement('span');divider.className='seq-break-wrap';divider.dataset.smokeBreak='1';
                divider.style.cssText='display:block;flex-basis:100%;width:100%;height:0';
                document.querySelector('#sim_sequence > .sim-seq-item').after(divider);
            }""")
            expect(page.locator('#sim_sequence .ma-selection-selected')).to_have_count(2)
            assert_selection_outlines()
            page.evaluate("document.querySelector('#sim_sequence [data-smoke-break]').remove()")
            expect(page.locator('#sim_sequence .ma-selection-selected')).to_have_count(1)
            page.wait_for_timeout(180)
            assert (len(simulations),len(analyses))==before_outlines,'Display outline redraw triggered combat/analysis requests'
            state_unchanged()

            # Make both scroll ranges nonzero and deliberately unequal. Syncing
            # progress must still let the much longer actual side reach its end.
            page.locator('#macro_split_height').focus()
            page.keyboard.press('Home')
            page.wait_for_function("""() => ['sim_sequence','macro_compare_sequence'].every(id => {
                const node=document.getElementById(id);return node.scrollHeight-node.clientHeight > 20;
            })""")
            page.wait_for_timeout(220)  # Let first-difference focus suppression expire.
            page.evaluate("""() => { const node=document.getElementById('sim_sequence');node.scrollTop=(node.scrollHeight-node.clientHeight)*.6; }""")
            page.wait_for_function("""() => { const node=document.getElementById('macro_compare_sequence');
                return Math.abs(node.scrollTop/(node.scrollHeight-node.clientHeight)-.6)<.025; }""")
            page.evaluate("""() => { const node=document.getElementById('macro_compare_sequence');node.scrollTop=node.scrollHeight-node.clientHeight; }""")
            page.wait_for_function("""() => ['sim_sequence','macro_compare_sequence'].every(id => {
                const node=document.getElementById(id);return node.scrollTop/(node.scrollHeight-node.clientHeight)>.98;
            })""")
            page.locator('#macro_compare_sync').uncheck()
            actual_end = page.locator('#macro_compare_sequence').evaluate('node => node.scrollTop')
            page.evaluate("""() => { const node=document.getElementById('sim_sequence');node.scrollTop=(node.scrollHeight-node.clientHeight)*.25; }""")
            page.wait_for_timeout(80)
            assert abs(page.locator('#macro_compare_sequence').evaluate('node=>node.scrollTop')-actual_end)<1
            reference_position = page.locator('#sim_sequence').evaluate('node=>node.scrollTop')
            page.evaluate("""() => { const node=document.getElementById('macro_compare_sequence');node.scrollTop=(node.scrollHeight-node.clientHeight)*.5; }""")
            page.wait_for_timeout(80)
            assert abs(page.locator('#sim_sequence').evaluate('node=>node.scrollTop')-reference_position)<1
            page.evaluate("document.getElementById('sim_sequence').scrollTop=0;document.getElementById('macro_compare_sequence').scrollTop=0")
            page.locator('#macro_compare_sync').check()
            page.locator('#macro_split_height').dblclick()
            state_unchanged()

            measurements = []
            for width in [1440, 900]:
                page.set_viewport_size({"width": width, "height": 1000})
                settle()
                for identifier in SPLITS:
                    handle = page.locator('#' + identifier)
                    expect(handle).to_be_visible()
                    expect(handle).to_have_attribute('role', 'separator')
                    expect(handle).to_have_attribute('tabindex', '0')
                    expect(handle).to_have_attribute('aria-orientation', 'horizontal' if identifier.endswith('height') else 'vertical')
                    handle.dblclick()
                baseline = geometry()
                assert_aligned(baseline)
                assert_detail_columns()
                assert_selection_outlines()
                page.evaluate("document.getElementById('macro_assist_content').scrollTop = 0")
                first_code = page.locator('.ma-program-code').first.bounding_box()
                offset = first_code['y'] - baseline['assistant']['y']
                assert offset <= (120 if width == 1440 else 150), {"width": width, "candidate_offset": offset}
                measurements.append({"width": width, "candidate_offset": round(offset, 1)})

                drag('macro_split_top_width', 60 if width == 1440 else 25, 0)
                changed = geometry()
                assert changed['template']['width'] > baseline['template']['width'] + 10, (baseline, changed)
                assert abs(changed['assistant']['width'] - baseline['assistant']['width']) <= 1, (baseline, changed)
                assert_aligned(changed)
                page.locator('#macro_split_top_width').focus()
                previous = changed['template']['width']
                page.keyboard.press('ArrowLeft')
                assert geometry()['template']['width'] < previous - 1
                page.locator('#macro_split_top_width').dblclick()
                assert abs(geometry()['template']['width'] - baseline['template']['width']) <= 1

                drag('macro_split_bottom_width', -55 if width == 1440 else -25, 0)
                changed = geometry()
                assert changed['assistant']['width'] < baseline['assistant']['width'] - 10, (baseline, changed)
                assert abs(changed['template']['width'] - baseline['template']['width']) <= 1, (baseline, changed)
                page.locator('#macro_split_bottom_width').focus()
                previous = changed['assistant']['width']
                page.keyboard.press('ArrowRight')
                assert geometry()['assistant']['width'] > previous + 1
                page.locator('#macro_split_bottom_width').dblclick()
                assert abs(geometry()['assistant']['width'] - baseline['assistant']['width']) <= 1

                drag('macro_split_height', 0, 35)
                changed = geometry()
                assert changed['top']['height'] > baseline['top']['height'] + 10, (baseline, changed)
                assert changed['bottom']['height'] < baseline['bottom']['height'] - 10, (baseline, changed)
                page.locator('#macro_split_height').focus()
                previous = changed['top']['height']
                page.keyboard.press('ArrowUp')
                assert geometry()['top']['height'] < previous - 1
                page.locator('#macro_split_height').dblclick()
                assert abs(geometry()['top']['height'] - baseline['top']['height']) <= 1

                drag('macro_split_editor_width',30 if width==1440 else 15,0)
                changed=geometry()
                assert changed['draftPane']['width']>baseline['draftPane']['width']+5,(baseline,changed)
                assert changed['detail']['width']<baseline['detail']['width']-5,(baseline,changed)
                assert abs(changed['editor']['height']-baseline['editor']['height'])<=1,(baseline,changed)
                page.locator('#macro_split_editor_width').focus()
                previous=changed['draftPane']['width']
                page.keyboard.press('ArrowLeft')
                assert geometry()['draftPane']['width']<previous-1
                page.locator('#macro_split_editor_width').dblclick()
                assert abs(geometry()['draftPane']['width']-baseline['draftPane']['width'])<=1
                state_unchanged()

                for theme in ['', 'pink-theme', 'light-theme', 'indigo-theme']:
                    page.evaluate("""theme => { document.body.classList.remove('pink-theme','light-theme','indigo-theme');
                        if (theme) document.body.classList.add(theme); }""", theme)
                    settle()
                    assert_aligned(geometry())
                    handles = page.evaluate("""ids => ids.map(id => {
                        const element=document.getElementById(id), style=getComputedStyle(element,'::before'), box=element.getBoundingClientRect();
                        const color=getComputedStyle(element).getPropertyValue('--text-muted').trim();
                        const probe=document.createElement('span'); probe.style.color=color; element.append(probe);
                        const resolved=getComputedStyle(probe).color; probe.remove();
                        return {id,width:box.width,height:box.height,markers:style.backgroundImage,color:resolved};
                    })""", SPLITS)
                    assert all(item['width'] >= 4 and item['height'] >= 4 and item['color'] in item['markers'] for item in handles), handles
                    colors = page.evaluate("""() => ['reference','actual'].map(side => {
                        const item = document.querySelector(side === 'reference' ? '#sim_sequence .ma-reference-missing' : '#macro_compare_sequence .ma-redline-extra');
                        const probe=document.createElement('span'); item.parentElement.append(probe);
                        const variable=side === 'reference' ? '--red' : '--green';
                        probe.style.backgroundColor=`color-mix(in srgb,var(${variable}) 45%,var(--surface))`;
                        probe.style.color=`var(${variable})`;
                        const style=getComputedStyle(item),band=getComputedStyle(item,'::after'),expected=getComputedStyle(probe);
                        const result={side,background:band.backgroundColor,expectedBackground:expected.backgroundColor,opacity:style.opacity,
                            text:getComputedStyle(item.querySelector('.seq-label')).color};
                        probe.remove();return result;
                    })""")
                    assert all(item['background'] == item['expectedBackground'] and item['opacity'] == '1' for item in colors), colors
                    assert all(contrast_ratio(item['text'],item['background'])>=4.5 for item in colors),colors
                    assert_bands()
                    assert_selection_outlines()
                    if theme == 'light-theme':
                        page.mouse.move(40, 40)
                        screenshot = Path(__file__).resolve().parents[1] / 'backend' / 'target' / f'macro-layout-{width}.png'
                        page.screenshot(path=str(screenshot), full_page=True)
                state_unchanged()

            # Distinct ratios survive mode exit and reload. Only layout persists;
            # the comparison object remains unchanged while toggling mode in-page.
            page.set_viewport_size({"width":1440,"height":1000})
            drag('macro_split_top_width', 60, 0)
            drag('macro_split_bottom_width', -45, 0)
            drag('macro_split_height', 0, 25)
            drag('macro_split_editor_width',25,0)
            expected_ratios = geometry()['ratios']
            expected_editor_ratio=page.evaluate("parseFloat(getComputedStyle(document.getElementById('panel_manual')).getPropertyValue('--ma-editor-left'))")
            page.locator('#macro_assist_edit').click()
            for identifier in SPLITS:
                expect(page.locator('#' + identifier)).to_be_hidden()
            expect(page.locator('#sim_sequence .ma-selection-outline')).to_have_count(0)
            state_unchanged()
            page.locator('#macro_assist_toggle').click()
            settle()
            assert all(abs(a-b) < .004 for a,b in zip(geometry()['ratios'],expected_ratios))
            state_unchanged()

            page.evaluate("""() => Jx3MacroAssist.selectItems([document.querySelector('#sim_sequence .sim-seq-item:not(.seq-auto):not(.seq-pre-release)')])""")
            expect(page.locator('#macro_assist_candidates .ma-table code').first).to_be_visible()
            page.evaluate("document.getElementById('macro_assist_content').scrollTop=0")
            single_offset = page.locator('#macro_assist_candidates .ma-table code').first.bounding_box()['y'] - geometry()['assistant']['y']
            assert single_offset <= 160, {"single_candidate_offset":single_offset}
            measurements.append({"width":1440,"single_candidate_offset":round(single_offset,1)})
            for selector in ['.ma-candidate-help','#macro_assist_contrast','.ma-occurrences']:
                expect(page.locator(selector)).not_to_have_attribute('open','')
            single_sections = page.evaluate("""() => ['#macro_assist_panel .ma-panel-heading','#macro_assist_selection',
                '#macro_assist_panel .ma-filters','#macro_assist_panel .ma-result-count','#macro_assist_candidates .ma-table thead',
                '#macro_assist_candidates .ma-table tbody tr'].map(selector => {
                    const item=document.querySelector(selector);if (!item) return {selector,missing:true};
                    const rect=item.getBoundingClientRect();return {selector,top:rect.top,height:rect.height};
                })""")
            page.evaluate("document.body.classList.remove('pink-theme','indigo-theme');document.body.classList.add('light-theme')")
            settle()
            page.mouse.move(40,40)
            page.screenshot(path=str(Path(__file__).resolve().parents[1] / 'backend' / 'target' / 'macro-layout-single-1440.png'),full_page=True)

            page.reload(wait_until='networkidle')
            ready()
            page.locator('#macro_assist_toggle').click()
            settle()
            actual_ratios = geometry()['ratios']
            assert all(abs(a-b) < .004 for a,b in zip(actual_ratios,expected_ratios)), (actual_ratios, expected_ratios)
            assert abs(page.evaluate("parseFloat(getComputedStyle(document.getElementById('panel_manual')).getPropertyValue('--ma-editor-left'))")-expected_editor_ratio)<.001
            expect(page.locator('#macro_split_editor_width')).to_be_hidden()
            assert geometry()['draftPane']['width']>=geometry()['editor']['width']-8
            expect(page.locator('#macro_draft_text')).to_have_value('/cast 盾刀')
            assert not errors, errors
            print(json.dumps({"ok":True,"measurements":measurements,"single_sections":single_sections,"persisted_ratios":actual_ratios,
                "pure_state_summary":pure_state,"default_layer_bands":initial_bands,
                "state_field_fixture":state_field_fixture,
                "write_requests_intercepted":len(writes),"checks":["header title aligned","sequence container top edges aligned",
                    "candidate code appears early","three independent pointer splitters","keyboard adjustments","double-click reset",
                    "1440/900 and four themes","mode exit preserves template and macro state","ratios persist across reload",
                    "skills-only default hides pure state differences","layer switch uses cached result and current-layer first difference",
                    "connected bands stop at normal skills and wrapped rows","relative scroll sync reaches both ends and can be disabled",
                    "one outline per contiguous selection or match without mutations","selection outlines stop at natural and manual wraps",
                    "draft and detail share horizontal space at full height","diff badges removed while resource labels remain",
                    "actual label contrast is readable on 45 percent bands in four themes","only changed state values are highlighted",
                    "draft/detail splitter supports drag keyboard reset and persistence","macro footer is spaced below state fields"]},ensure_ascii=False))
        finally:
            browser.close()


if __name__ == '__main__':
    parser=argparse.ArgumentParser()
    parser.add_argument('--base-url',required=True)
    parser.add_argument('--review-only',action='store_true',help='Capture the compact detail visual fixture after the real comparison checks')
    parser.add_argument('--draft-panes-only',action='store_true',help='Check the two visible draft panes and their real shared macro run')
    options=parser.parse_args()
    run(options.base_url.rstrip('/'),options.review_only,options.draft_panes_only)
