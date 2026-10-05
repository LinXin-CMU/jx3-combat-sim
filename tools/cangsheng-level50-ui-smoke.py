"""Level/version switching, attribute parity and TieGuYi combat on an isolated worker."""
import argparse
import json
import math
import urllib.request
from urllib.parse import urlsplit
from playwright.sync_api import sync_playwright

parser = argparse.ArgumentParser()
parser.add_argument('--backend', default='http://127.0.0.1:3037')
args = parser.parse_args()
address = urlsplit(args.backend)
assert address.hostname in ('localhost', '127.0.0.1') and address.port != 3005

def api(path, body=None):
    req = urllib.request.Request(args.backend + path, data=None if body is None else json.dumps(body).encode(), headers={'Content-Type': 'application/json'})
    with urllib.request.urlopen(req, timeout=30) as response:
        return json.load(response)

original = api('/api/mounts/current')
try:
    with sync_playwright() as pw:
        browser = pw.chromium.launch(channel='msedge', headless=True)
        for version, mount, level in [('CangShengZhuShiTest','TieGuYi',50), ('CangShengZhuShiTest','FenShanJin',50), ('AnYingQianJi','FenShanJin',130)]:
            api('/api/mounts/switch', dict(version=version, mount=mount, persist=False))
            info = api('/api/mounts/current')
            assert info['attribute_params']['level'] == level
            attrs = api('/api/mounts/defaults')['attributes']
            attrs.update(crit_level=1000, haste_level=100)
            stats = api('/api/calculate', attrs)
            assert math.isclose(stats['crit_rate'], 1000/info['attribute_params']['crit'])
            if level == 50:
                equip = api('/api/equip/calculate', {'slots': {}, 'talents': []})
                calc_attrs = dict(attrs, base_attack=equip['raw']['base_attack'], shen_fa=equip['raw']['agility'], vitality=equip['raw']['vitality'])
                assert api('/api/calculate', calc_attrs)['panel_attack'] == equip['panel']['physics_attack_power']
            page = browser.new_page(viewport={'width':1600, 'height':1050})
            errors = []
            page.on('pageerror', lambda error: errors.append(str(error)))
            def route_api(route):
                path = urlsplit(route.request.url).path
                if path == '/api/settings': route.fulfill(json={})
                elif route.request.method == 'GET' or path in {'/api/calculate','/api/skill_damage','/api/simulate'}: route.continue_()
                else: route.fulfill(json={'ok':True})
            page.route('**/api/**', route_api)
            page.goto(args.backend, wait_until='networkidle')
            page.wait_for_function('currentMount.attribute_params !== undefined')
            result = page.evaluate('''() => ({
                options: Array.from(document.getElementById('target_level').options, o=>Number(o.value)),
                target: getTarget().level, level:currentAttributeParams().level,
                parry: jx3BuildAttrTooltip('parry', {}, {}),
                strain: jx3BuildAttrTooltip('strain', {}, {})
            })''')
            assert result['options'] == list(range(level+1, level+5)), result
            assert result['target'] in result['options'], result
            assert str(info['attribute_params']['strain']).rstrip('0').rstrip('.') in result['strain']
            if mount == 'TieGuYi':
                assert page.evaluate('isExperimental()') is False
                page.wait_for_function("!!skillInfoMap['盾刀']")
                assert '回复10点怒气' in page.evaluate("skillTooltipHtml(skillInfoMap['盾刀'])")
                environment = dict(attributes=dict(attrs, vitality=9500), target={'level':54,'defense_bonus':0}, talents=[13133,13356,13422], recipes=[], equipment={}, team_buffs=[], formation=None, tiegu_mode=0, network_delay=0, haste_level=100, initial_rage=100, sequence=['盾刀','盾压','盾飞','斩刀','绝刀'], experimental=False, boss_attack_interval=None, pre_releases=[], pauses=[])
                full = api('/api/simulate', environment)
                lite = api('/api/simulate', dict(environment, lite=True))
                repeat = api('/api/simulate', environment)
                assert not full['skipped'], full['skipped']
                assert full['fingerprint'] == lite['fingerprint'] == repeat['fingerprint']
                assert full['total_damage'] == lite['total_damage'] == repeat['total_damage']
                assert all(not e['name'].startswith('破·') for e in full['timeline'])
            assert not errors, errors
            page.close()
        browser.close()
    print('PASS: both level50 mounts, formal level130, panel parity, target options, descriptions, TieGuYi deterministic Full/Lite')
finally:
    api('/api/mounts/switch', dict(version=original['version'], mount=original['mount'], persist=False))
