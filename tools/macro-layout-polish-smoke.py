"""Read-only browser checks for left-side conditions and theme contrast on :3005."""
from urllib.parse import urlsplit
from playwright.sync_api import sync_playwright, expect


with sync_playwright() as p:
    browser = p.chromium.launch(channel='msedge', headless=True)
    page = browser.new_page(viewport={'width': 1600, 'height': 1050})
    errors = []
    page.on('pageerror', lambda error: errors.append(str(error)))

    def route_api(route):
        path = urlsplit(route.request.url).path
        if path == '/api/settings':
            route.fulfill(json={})
        elif route.request.method == 'GET' or path in {'/api/simulate', '/api/macro/diagnose', '/api/macro/prune_candidates', '/api/macro/assist', '/api/macro/assist/program', '/api/calculate', '/api/skill_damage'}:
            route.continue_()
        else:
            route.fulfill(json={'ok': True})

    page.route('**/api/**', route_api)
    page.goto('http://127.0.0.1:3005', wait_until='networkidle')
    page.wait_for_function('!!window.Jx3MacroEditor && !!window.Jx3MacroLayout')
    page.evaluate("""async () => {
        await currentMountReady; Jx3Nav.switchPage('page-sim');
        document.getElementById('sim_sequence').replaceChildren();
        ['盾压','盾刀','盾刀','盾刀'].forEach(skill=>addSeqItem(skill));await runSimulate();
    }""")
    page.locator('#macro_assist_toggle').click()
    page.locator('#macro_draft_shield').fill('/cast 盾刀\n/cast 盾压')
    page.locator('#macro_draft_blade').fill('')
    page.evaluate('async () => await Jx3MacroEditor.run()')
    expect(page.locator('.ma-diagnostic-conclusion')).to_be_visible()
    page.evaluate("Jx3MacroAssist.selectItems([...document.querySelectorAll('#sim_sequence .sim-seq-item:not(.seq-auto):not(.seq-pre-release)')].slice(0,2))")
    expect(page.locator('#macro_assist_panel')).to_be_visible()
    expect(page.locator('#macro_compare_detail')).to_be_visible()
    expect(page.locator('#macro_draft_pane')).to_be_hidden()
    bounds = page.evaluate("""() => {
        const box=id=>{const r=document.getElementById(id).getBoundingClientRect();return {x:r.x,right:r.right};};
        return {conditions:box('macro_assist_panel'),editor:box('macro_editor_panel'),review:box('macro_review')};
    }""")
    assert bounds['conditions']['right'] <= bounds['review']['x']
    assert abs(bounds['conditions']['x'] - bounds['editor']['x']) < 2
    for theme in ['', 'pink-theme', 'light-theme', 'indigo-theme']:
        page.evaluate("""theme=>{document.body.classList.remove('pink-theme','light-theme','indigo-theme');if(theme)document.body.classList.add(theme);}""", theme)
        ratios = page.evaluate("""() => {
            const ctx=document.createElement('canvas').getContext('2d');
            const lum=color=>{ctx.fillStyle=color;ctx.fillRect(0,0,1,1);const rgb=[...ctx.getImageData(0,0,1,1).data].slice(0,3).map(v=>v/255).map(v=>v<=.04045?v/12.92:((v+.055)/1.055)**2.4);return rgb.reduce((v,x,i)=>v+x*[.2126,.7152,.0722][i],0);};
            return ['.ma-diagnostic-conclusion','#macro_review_conditions'].map(s=>{const css=getComputedStyle(document.querySelector(s));const a=lum(css.color),b=lum(css.backgroundColor);return (Math.max(a,b)+.05)/(Math.min(a,b)+.05);});
        }""")
        assert min(ratios) >= 4.5, (theme, ratios)
        expect(page.locator('#macro_assist_panel')).to_be_visible()
    page.evaluate("document.body.classList.remove('pink-theme','indigo-theme');document.body.classList.add('light-theme')")
    page.mouse.move(10, 10)
    page.screenshot(path='backend/target/macro-layout-polish.png')
    page.evaluate("Jx3MacroEditor.insert('/cast 盾刀')")
    expect(page.locator('#macro_draft_pane')).to_be_visible()
    expect(page.locator('#macro_assist_panel')).to_be_hidden()
    expect(page.locator('#macro_compare_detail')).to_be_visible()
    page.locator('#macro_review_conditions').click()
    expect(page.locator('#macro_assist_selection')).to_contain_text('盾压')
    assert not errors, errors
    browser.close()
    print('PASS: conditions beneath template, diagnosis stays visible, selection retained, insertion returns to draft, all theme contrast >= 4.5')
