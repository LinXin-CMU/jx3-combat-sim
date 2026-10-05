"""Render free prose through the actual full-page and dock renderers, with mocked reports."""
import argparse
from pathlib import Path
from urllib.parse import urlsplit
from playwright.sync_api import sync_playwright

parser = argparse.ArgumentParser()
parser.add_argument('--backend', default='http://127.0.0.1:3037')
parser.add_argument('--screenshots', type=Path)
args = parser.parse_args()
assert urlsplit(args.backend).hostname in ('localhost', '127.0.0.1')
script = Path('frontend/agent.js').read_text(encoding='utf-8')
head, tail = script.rsplit('})();', 1)
script = head + 'window.__proseTest = {renderReport, renderDockReport, buildSummary, setDockOpen};\n})();' + tail
body = '''# 苍云·铁骨衣：奇穴替换的策划分析

## 一、判断

替换会改变资源的获取与消耗，也会改变玩家安排爆发的方式。
分析需要同时考虑输出循环、团队功能和操作体验。

## 二、命题与基线

- **目标心法：** 铁骨衣。源技能提供新的资源消耗方式，目标心法仍然保留自己的战斗定位。
- **替换范围：** 主动招式与被动机制一并讨论，配套奇穴另作设计选择。

## 三、对循环的影响

1. **资源轴：** 获取与消耗需要连起来看，避免新招式缺少持续供给。
2. **施放时间：** 额外连招会占用原有操作时间，也会改变玩家保留资源的理由。
   - 连招与原有技能如何穿插。
   - 爆发与团队功能如何取舍。
3. **功能轴：** 控制与防御的变化也要纳入方案。

### 方案对照

| 选择 | 代价 |
| --- | --- |
| 保留资源 | 延后连招 |

---

这是机制推演的示例正文。

<img src=x onerror="window.proseInjected=true">'''
result = {'status':'completed','run_id':'prose-ui-fixture','provider_profile':'fixture','model':'fixture','scenario_hash':'a'*64,'prompt_version':'agent-system/v51','accounting':{'duration_ms':1,'tool_calls':0,'simulations':0},'trace':[], 'report':{
    'provider_profile':'fixture','model':'fixture','evidence_ids':[],'sources':[],
    'content':{'summary':'会话摘要不会重复出现在正文前。','body_markdown':body,'findings':[{'title':'支撑材料','explanation':'只在证据附录展示。','metrics':[]}], 'recommendations':[], 'rotation_changes':[], 'artifacts':[], 'limitations':[]}}}
with sync_playwright() as pw:
    browser = pw.chromium.launch(channel='msedge', headless=True)
    page = browser.new_page(viewport={'width':1500,'height':1000})
    page.emulate_media(reduced_motion='reduce')
    errors = []
    page.on('pageerror', lambda error: errors.append(str(error)))
    page.route('**/agent.js*', lambda route: route.fulfill(body=script, content_type='application/javascript'))
    def route_api(route):
        path = urlsplit(route.request.url).path
        if path == '/api/settings': route.fulfill(json={})
        elif route.request.method == 'GET' or path in {'/api/calculate','/api/skill_damage'}: route.continue_()
        else: route.fulfill(json={'ok':True})
    page.route('**/api/**', route_api)
    page.goto(args.backend, wait_until='networkidle')
    page.wait_for_function('window.__proseTest !== undefined')
    page.add_style_tag(content='*, *::before, *::after { animation: none !important; transition: none !important; }')
    page.evaluate("document.body.classList.add('light-theme')")
    copied = page.evaluate('''result => {
        __proseTest.renderReport(result);
        __proseTest.renderDockReport(result);
        return __proseTest.buildSummary(result);
    }''', result)
    assert copied == body
    for selector in ['#agent_transcript .agent-report', '#sim_ai_chat .sim-ai-result']:
        if selector.startswith('#agent_'):
            page.evaluate("Jx3Nav.switchPage('page-agent')")
        else:
            page.evaluate("Jx3Nav.switchPage('page-sim'); __proseTest.setDockOpen(true)")
        card = page.locator(selector).last
        article = card.locator('.agent-freeform-answer')
        assert article.locator('h1').text_content() == '苍云·铁骨衣：奇穴替换的策划分析'
        assert article.locator('h2').count() == 3
        assert article.locator('h3').count() == 1
        assert article.locator('p').first.text_content().count('分析需要') == 1
        assert article.locator('ol > li').count() == 3
        assert article.locator('ol ul > li').count() == 2
        assert article.locator('hr').count() == 1
        sizes = article.evaluate('''el => {
            const css = getComputedStyle(el);
            return {body:parseFloat(css.fontSize), padding:parseFloat(css.paddingLeft),
                title:parseFloat(getComputedStyle(el.querySelector('h1')).fontSize),
                section:parseFloat(getComputedStyle(el.querySelector('h2')).fontSize),
                itemGap:parseFloat(getComputedStyle(el.querySelector('li')).marginBottom),
                fits:el.scrollWidth <= el.clientWidth + 1};
        }''')
        assert sizes['title'] > sizes['section'] > sizes['body'] >= 14, sizes
        assert sizes['padding'] >= 16 and sizes['itemGap'] >= 7 and sizes['fits'], sizes
        assert card.locator('.agent-freeform-answer table').count() == 1
        assert card.locator('.agent-freeform-answer img').count() == 0
        assert '会话摘要' not in card.inner_text()
        evidence = card.locator('details').filter(has=page.locator('summary', has_text='依据与指标'))
        assert evidence.count() == 1 and evidence.get_attribute('open') is None
        if args.screenshots:
            args.screenshots.mkdir(parents=True, exist_ok=True)
            page.evaluate("document.querySelector('#agent_transcript').scrollTop = 0; document.querySelector('#sim_ai_chat').scrollTop = 0")
            page.screenshot(path=str(args.screenshots / ('page.png' if selector.startswith('#agent_') else 'dock.png')))
    page.set_viewport_size({'width':390,'height':844})
    article = page.locator('#sim_ai_chat .agent-freeform-answer').last
    assert article.evaluate('el => el.scrollWidth <= el.clientWidth + 1')
    if args.screenshots:
        page.screenshot(path=str(args.screenshots / 'mobile.png'))
    assert page.evaluate('window.proseInjected === undefined')
    assert not errors, errors
    browser.close()
print('PASS: free prose in both views, collapsed evidence, clean copy, safe Markdown and legacy compatibility')
