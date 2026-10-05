"""Verify coefficient metadata, preview/event parity and real skill tooltips on an isolated worker."""
import argparse
import json
import urllib.request
from urllib.parse import urlsplit
from playwright.sync_api import sync_playwright, expect

parser = argparse.ArgumentParser()
parser.add_argument('--backend', default='http://127.0.0.1:3037')
parser.add_argument('--screenshot')
args = parser.parse_args()
address = urlsplit(args.backend)
assert address.hostname in ('localhost', '127.0.0.1') and address.port != 3005, 'Use an isolated worker'

def api(path, body=None):
    req = urllib.request.Request(args.backend + path,
        data=None if body is None else json.dumps(body).encode(), headers={'Content-Type': 'application/json'})
    with urllib.request.urlopen(req, timeout=30) as response:
        return json.load(response)

original = api('/api/mounts/current')
try:
    api('/api/mounts/switch', {'version': 'CangShengZhuShiTest', 'mount': 'FenShanJin', 'persist': False})
    specs = api('/api/skills')
    shield = next(s for s in specs if s['name'] == '盾刀·一段')
    assert shield['base_damage_range'] == [13, 14]
    assert shield['base_damage'] == 13.5
    array_end = next(s for s in specs if s['skill_id'] == 30856)
    assert array_end['high_berserk_damage']['base_damage_range'] == [13.8, 14.7]
    attr = dict.fromkeys(['vitality', 'li_dao', 'gen_gu', 'yuan_qi', 'shen_fa', 'crit_level',
        'crit_effect_level', 'overcome_level', 'strain_level', 'haste_level'], 0)
    attr.update(base_attack=10000, weapon_damage=100)
    env = {'attributes': attr, 'target': {'level': 50, 'defense_bonus': 0}, 'talents': [30769],
        'recipes': [], 'equipment': {}, 'team_buffs': [], 'formation': None, 'tiegu_mode': 0}
    preview = {s['name']: s for s in api('/api/skill_damage', env)['skills']}
    for sequence in [['盾刀'], ['阵云结晦', '月照连营', '雁门迢递']]:
        body = dict(env, sequence=sequence, haste_level=0, initial_rage=50, experimental=False,
            hanjia_expectation=False, dunya_reset_seed=0, boss_attack_interval=None, pauses=[], lite=False)
        full = api('/api/simulate', body)
        repeated = api('/api/simulate', body)
        lite = api('/api/simulate', dict(body, lite=True))
        assert full['total_damage'] == repeated['total_damage'] == lite['total_damage']
        for event in full['timeline']:
            if event['skill_id'] in (13044, 30769, 30855, 30856):
                assert event['damage_normal'] == preview[event['name']]['normal_damage'], event['name']

    with sync_playwright() as p:
        browser = p.chromium.launch(channel='msedge', headless=True)
        page = browser.new_page(viewport={'width': 1600, 'height': 1050})
        errors = []
        page.on('pageerror', lambda e: errors.append(str(e)))
        def route_api(route):
            path = urlsplit(route.request.url).path
            if path == '/api/settings': route.fulfill(json={})
            elif route.request.method == 'GET' or path in {'/api/simulate', '/api/skill_damage', '/api/calculate'}:
                route.continue_()
            else: route.fulfill(json={'ok': True})
        page.route('**/api/**', route_api)
        page.goto(args.backend, wait_until='networkidle')
        page.wait_for_function("typeof skillInfoMap !== 'undefined' && !!skillInfoMap['盾刀']")
        expect(page.locator('.tb-brand-tag')).to_have_text('v2.0.13-20260914')
        page.evaluate("Jx3Nav.switchPage('page-sim')")
        page.locator('.sim-skill-btn[data-skill="盾刀"]').hover()
        tooltip = page.locator('.g-tooltip.visible')
        expect(tooltip).to_contain_text('13–14')
        expect(tooltip).to_contain_text('0.156×最终外功攻击')
        expect(tooltip).to_contain_text('回复10点怒气，对目标造成100%的武器伤害外加')
        expect(tooltip).to_contain_text('盾刀·二段')
        expect(tooltip).to_contain_text('盾刀·三段')
        expect(tooltip).to_contain_text('1×武器伤害')
        for theme in ['', 'pink-theme', 'indigo-theme', 'light-theme']:
            page.evaluate('''theme => {
                document.body.classList.remove('pink-theme', 'indigo-theme', 'light-theme');
                if (theme) document.body.classList.add(theme);
            }''', theme)
            expect(tooltip).to_be_visible()
            box = tooltip.bounding_box()
            assert box and box['x'] >= 0 and box['x'] + box['width'] <= 1600
            assert box['y'] >= 0 and box['y'] + box['height'] <= 1050
        if args.screenshot: page.screenshot(path=args.screenshot)
        descriptions = page.evaluate("Object.fromEntries(['雁门迢递','斩刀','血怒'].map(n=>[n,skillTooltipHtml(skillInfoMap[n])]))")
        assert '13.8–14.7' in descriptions['雁门迢递']
        assert '首段消耗100暴怒' in descriptions['雁门迢递']
        assert '0.014×最终外功攻击' in descriptions['斩刀']
        assert '点伤害' not in descriptions['血怒']
        checks = page.evaluate('''() => {
            talentSelection[4] = 91002;
            talentSelection[6] = 91003;
            const shield = skillTooltipHtml(skillInfoMap['盾刀']);
            const array = skillTooltipHtml(skillInfoMap['阵云结晦']);
            talentSelection[4] = null;
            const removed = skillTooltipHtml(skillInfoMap['盾刀']);
            return {shield, array, removed};
        }''')
        assert '【神威】' in checks['shield'] and '【威压】' in checks['shield']
        assert '可叠加8层' in checks['shield'] and '伤害提高30%' in checks['shield']
        assert '【神威】' not in checks['array']
        assert '【神威】' not in checks['removed'] and '【威压】' in checks['removed']
        assert '{{damage:' not in checks['shield'] and '<TALENT' not in checks['shield']
        assert 'skill-damage-note' not in checks['shield']
        assert not errors, errors
        browser.close()
    print('PASS: API range metadata, preview/event damage parity, deterministic Lite/Full, multi-rank tooltip, high tier, DOT, non-damage skill, four themes')
finally:
    api('/api/mounts/switch', {'version': original['version'], 'mount': original['mount'], 'persist': False})
