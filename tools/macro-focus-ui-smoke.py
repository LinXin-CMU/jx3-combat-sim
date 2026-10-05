"""Read-only browser regression for focus, paired scrolling and state-derived repairs."""
import os, json
from pathlib import Path
from urllib.parse import urlsplit
from playwright.sync_api import sync_playwright, expect

with sync_playwright() as p:
    browser=p.chromium.launch(channel='msedge',headless=True)
    page=browser.new_page(viewport={'width':1600,'height':1050})
    page.set_default_timeout(30000)
    errors, calls=[],[]
    page.on('pageerror',lambda error:errors.append(str(error)))
    def route_api(route):
        path=urlsplit(route.request.url).path
        if path=='/api/settings':route.fulfill(json={})
        elif route.request.method=='GET' or path in {'/api/simulate','/api/macro/diagnose','/api/macro/assist','/api/macro/assist/program','/api/macro/prune_candidates','/api/calculate','/api/skill_damage'}:
            if route.request.method=='POST':calls.append((path,route.request.post_data_json))
            route.continue_()
        else:route.fulfill(json={'ok':True})
    page.route('**/api/**',route_api)
    page.goto('http://127.0.0.1:3005',wait_until='networkidle')
    page.wait_for_function('!!window.Jx3MacroRepairPanel && !!window.Jx3MacroEditor')
    page.evaluate("""async()=>{
        await currentMountReady;Jx3Nav.switchPage('page-sim');
        document.getElementById('sim_sequence').replaceChildren();
        [...Array(45).fill('盾刀'),'盾压',...Array(35).fill('盾刀'),'盾压',...Array(10).fill('盾刀')].forEach(s=>addSeqItem(s));
        await runSimulate();
    }""")
    page.locator('#macro_assist_toggle').click()
    page.locator('#macro_split_height').focus()
    page.locator('#macro_split_height').press('Home')
    page.locator('#macro_draft_shield').fill('/cast 盾刀\n/cast [rage>100] 盾压')
    page.locator('#macro_draft_blade').fill('')
    page.evaluate('async()=>await Jx3MacroEditor.run()')
    panel=page.locator('.ma-diagnostic')
    expect(panel).to_contain_text('模板期待：盾压')
    def visible_focus():
        return page.evaluate("""()=>{
            const a=document.querySelector('#sim_sequence .ma-diff-focus');const c=document.getElementById('sim_sequence');
            const r=a.getBoundingClientRect(),b=c.getBoundingClientRect();
            return {visible:r.top>=b.top-1 && r.bottom<=b.bottom+1,left:c.scrollTop,right:document.getElementById('macro_compare_sequence').scrollTop};
        }""")
    page.wait_for_function("document.getElementById('sim_sequence').scrollTop>0 && document.getElementById('macro_compare_sequence').scrollTop>0")
    bounds=visible_focus()
    assert bounds['visible'] and bounds['left']>0 and bounds['right']>0,bounds
    # A real mouse click uses the editor's mouseup selection path, not synthetic dispatch.
    late=page.locator('#sim_sequence .ma-reference-missing').last
    page.evaluate("Jx3MacroLayout.tab('draft')")
    late.scroll_into_view_if_needed()
    late.click()
    expect(page.locator('#macro_draft_pane')).to_be_visible()
    assert late.evaluate("e=>getComputedStyle(e).cursor")=='pointer'
    expected=page.evaluate("""()=>{
        const r=Jx3MacroEditor.getResult();const rows=r.alignment.rows.filter(x=>x.kind==='missing');
        return r.template.timeline[rows[rows.length-1].referenceIndex].cast_time.toFixed(3)+'s';
    }""")
    expect(panel.locator('.ma-diagnostic-goal')).to_contain_text(expected)
    expect(panel.locator('.ma-diagnostic-conclusion')).to_be_visible()
    page.locator('#macro_compare_sync').uncheck()
    page.evaluate("document.getElementById('sim_sequence').scrollTop=999999;document.getElementById('macro_compare_sequence').scrollTop=999999")
    page.locator('#macro_compare_first').click()
    bounds=visible_focus()
    assert bounds['visible'] and bounds['left']>0 and bounds['right']>0,bounds
    expect(panel.locator('.ma-repair-progress[data-complete="true"]')).to_be_visible(timeout=60000)
    assert panel.get_by_role('button',name='生成并验证改法').count()==0
    assert panel.locator('.ma-repair-card:visible').count()==1
    tabs=panel.get_by_role('tab')
    if tabs.count()>1:
        tabs.last.click()
        expect(tabs.last).to_have_attribute('aria-selected','true')
        assert panel.locator('.ma-repair-card:visible').count()==1
    assert page.evaluate("['.ma-diagnostic-flow','.ma-diagnostic-repairs'].every(s=>getComputedStyle(document.querySelector(s)).overflowY==='auto')")
    expect(panel.locator('.ma-related-check').first).to_be_visible()
    expect(panel.locator('.ma-related-check').first).to_contain_text('同一轮复查')
    expect(panel.locator('.ma-related-check').first).to_contain_text('Step 2')
    assert visible_focus()['visible']
    assert 0<panel.locator('.ma-repair-card').count()<=5
    columns=page.evaluate("""()=>{
        const a=document.querySelector('.ma-diagnostic-flow').getBoundingClientRect(),b=document.querySelector('.ma-diagnostic-repairs').getBoundingClientRect();
        return {left:a.right,right:b.left,ay:a.y,by:b.y};
    }""")
    assert columns['left']<=columns['right'] and abs(columns['ay']-columns['by'])<1,columns
    analyses=[body for path,body in calls if path=='/api/macro/assist']
    assert any(body['options']['max_terms']==1 for body in analyses)
    assert any(body['options']['max_terms']==2 for body in analyses)
    assert all('macro_text' not in body for body in analyses)
    assert any(path=='/api/macro/prune_candidates' for path,body in calls)
    assert any(path=='/api/macro/diagnose' and body.get('include_result') for path,body in calls)
    assert page.locator('#macro_draft_shield').input_value()=='/cast 盾刀\n/cast [rage>100] 盾压'
    joint=page.evaluate("""async()=>{
        const saved=Jx3MacroEditor.getResult();
        const active=saved.template.timeline.flatMap((e,i)=>e.triggered?[]:[i]);
        const at=active.findIndex(i=>saved.template.timeline[i].name.split('·')[0]==='盾压');
        const group={references:active.slice(at-1,at+2),actuals:[],start:0,end:2};
        const request=async(path,body)=>{const r=await fetch(path,{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(body)});const d=await r.json();if(!r.ok)throw Error(d.error);return d;};
        const choices=await Jx3MacroRepairGroup.generate(saved,group,request,1);
        const result=await request('/api/macro/diagnose',{simulation:{...saved.request,macro_text:choices[0].text},version:saved.version,mount:saved.mount,start:0,end:0,include_result:true});
        return {count:choices.length,multiline:choices[0].after.includes('盾压')&&choices[0].after.includes('盾刀'),timeline:Array.isArray(result.simulation.timeline)};
    }""")
    assert joint['count']>0 and joint['multiline'] and joint['timeline'],joint
    assert any(path=='/api/macro/assist/program' for path,body in calls)
    counterfactual=page.evaluate(r"""async()=>{
        const original=Jx3MacroEditor.getResult();
        const shield='#page shield\n/cast last_skill=盾飞 血怒\n/cast last_skill~=盾猛&rage>=90 盾飞\n/cast buff:嗜血 盾猛\n/cast sun>99 阵云结晦\n/cast 盾压\n/cast 盾刀\n#page blade\n/cast buff:血怒·惊涌 月照连营\n/cast buff:血怒·惊涌 雁门迢递\n/cast 斩刀\n';
        const attack='/cast last_skill~=斩刀|sun<99 绝刀\n/cast 阵云结晦\n';
        const badText=shield+attack+'/cast rage<30 盾回';
        const goodText=shield+'/cast rage<50 盾回\n'+attack;
        const request=async(path,body)=>{const r=await fetch(path,{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(body)});const d=await r.json();if(!r.ok)throw Error(d.error);return d;};
        const simulation={...original.request,sequence:Array(260).fill('__macro__'),macro_duration:60};
        const sim=text=>request('/api/simulate',{...simulation,macro_text:text});
        const bad=await sim(badText),good=await sim(goodText),only=await sim(shield+attack+'/cast rage<50 盾回');
        const back=result=>result.timeline.find(e=>!e.triggered&&e.name==='盾回');
        const expected=back(good),current=back(bad),thresholdOnly=back(only);
        if(!expected||!current||!thresholdOnly)throw Error(JSON.stringify({bad:bad.timeline.filter(e=>!e.triggered).slice(0,25).map(e=>e.name),good:good.timeline.filter(e=>!e.triggered).slice(0,25).map(e=>e.name),skipped:bad.skipped}));
        const saved={...original,text:badText,template:good,actual:bad,window:60,alignment:Jx3MacroAlignment.align(good.timeline,bad.timeline)};
        let offset=0,page=-1;saved.pages=[];
        for(const text of badText.split('\n')){if(text.startsWith('#page')){page++;saved.pages.push([]);}else if(text.startsWith('/cast'))saved.pages[page].push({text,start:offset,end:offset+text.length});offset+=text.length+1;}
        const focus={kind:'missing',reference:expected,actual:current};
        const t=Jx3MacroRepairSearch.anchor(focus);
        const diagnosed=await request('/api/macro/diagnose',{simulation:{...simulation,macro_text:badText},version:saved.version,mount:saved.mount,start:Math.max(0,t-1),end:t+1,target_skill:'盾回'});
        const step=diagnosed.trace.decisions[Jx3MacroRepairSearch.decisionIndex(diagnosed.trace.decisions,focus)];
        const choices=Jx3MacroRepairSearch.thresholds(saved,step,focus,{});
        let recovered=false;
        for(const choice of choices.filter(c=>!c.title.includes('前移'))){const result=await sim(choice.text);if(Math.abs(back(result)?.cast_time-expected.cast_time)<.001){recovered=true;break;}}
        return {expectedRage:expected.state_before.rage,expectedTime:expected.cast_time,actualTime:current.cast_time,thresholdOnlyTime:thresholdOnly.cast_time,recovered};
    }""")
    assert counterfactual['expectedTime']<counterfactual['actualTime'],counterfactual
    assert counterfactual['thresholdOnlyTime']==counterfactual['expectedTime'],counterfactual
    assert counterfactual['expectedRage']==35,counterfactual
    assert counterfactual['recovered'],counterfactual
    page.locator('#macro_review_conditions').click()
    red=page.locator('#sim_sequence .ma-reference-missing').first
    assert red.evaluate("e=>getComputedStyle(e).cursor")=='crosshair'
    red.click()
    expect(page.locator('#macro_draft_pane')).to_be_hidden()
    if os.environ.get('MACRO_PERF_FIXTURE'):
        fixture=page.evaluate("""async()=>{
            const saved=Jx3MacroEditor.getResult();
            const simulation={...saved.request,macro_duration:300};
            const r=await fetch('/api/simulate',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(simulation)});
            const data=await r.json();
            return {raw:data.timeline,compact:Jx3MacroRepair.assistTimeline(data.timeline),simulation,version:saved.version,mount:saved.mount};
        }""")
        Path(os.environ['MACRO_PERF_FIXTURE']).write_text(json.dumps(fixture,ensure_ascii=False),encoding='utf-8')
    page.mouse.move(10,10)
    page.screenshot(path='backend/target/macro-focus-review.png')
    assert not errors,errors
    browser.close()
    print('PASS: real left click updates diagnosis, auto/explicit dual scroll, two columns, automatic state-only candidate generation and simplification')
