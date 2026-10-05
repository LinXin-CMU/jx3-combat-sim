"""Level-specific attribute storage and equipment initialization on an isolated worker."""
import argparse
import json
import urllib.request
from urllib.parse import urlsplit
from playwright.sync_api import sync_playwright

parser = argparse.ArgumentParser()
parser.add_argument('--backend', default='http://127.0.0.1:3038')
args = parser.parse_args()
address = urlsplit(args.backend)
assert address.hostname in ('localhost', '127.0.0.1') and address.port not in (3005, 3006)

def api(path, body=None, as_text=False):
    req = urllib.request.Request(args.backend + path, data=None if body is None else json.dumps(body).encode(), headers={'Content-Type':'application/json'})
    with urllib.request.urlopen(req, timeout=30) as response:
        text = response.read().decode()
        return text if as_text else json.loads(text)

original = api('/api/mounts/current')
old_attributes = dict(base_attack=4907, vitality=328941, li_dao=44, shen_fa=780,
    weapon_damage=12330, surplus_value=71084, crit_level=6441, crit_effect_level=0,
    overcome_level=5752, strain_level=138791, haste_level=48907, parry_level=65628, parry_value=755202)
equipment = {'slots': {'PRIMARY_WEAPON': {'equip_id':45320, 'strength':0, 'embedding':[], 'enhance_id':0, 'enchant_id':0}}, 'stoneId':0}

def switch(version):
    api('/api/mounts/switch', dict(version=version, mount='TieGuYi', persist=False))

try:
    switch('AnYingQianJi')
    api('/api/attrs/save', old_attributes, as_text=True)
    api('/api/attrs/save_profile', {'name':'level-regression','data':old_attributes}, as_text=True)
    assert api('/api/attrs/load') == old_attributes
    formal_item = api('/api/equip/detail', {'id':45320,'sub_type':0})['detail']
    switch('CangShengZhuShiTest')
    assert api('/api/attrs/load') is None
    assert 'level-regression' not in api('/api/attrs/profiles')
    test_item = api('/api/equip/detail', {'id':45320,'sub_type':0})['detail']
    assert formal_item['level'] == 42500 and test_item['level'] == 719
    panel = api('/api/equip/calculate', {'slots':equipment['slots'],'stone_id':0,'talents':[]})
    with sync_playwright() as pw:
        browser = pw.chromium.launch(channel='msedge', headless=True)
        page = browser.new_page(viewport={'width':1600,'height':1050})
        errors = []
        page.on('pageerror', lambda err:errors.append(str(err)))
        settings = {'eq_config_v1':json.dumps(equipment),
            'mount_choice':json.dumps({'version':'CangShengZhuShiTest','mount':'TieGuYi'}),
            'jx3_shared_inputs':json.dumps({'attrs':old_attributes,'target':{'level':134,'defense_bonus':0}})}
        def route_api(route):
            path = urlsplit(route.request.url).path
            if path == '/api/settings': route.fulfill(json=settings)
            elif route.request.method == 'GET' or path in {'/api/calculate','/api/skill_damage','/api/simulate','/api/equip/calculate','/api/equip/detail'}: route.continue_()
            else: route.fulfill(json={'ok':True})
        page.route('**/api/**', route_api)
        page.goto(args.backend, wait_until='networkidle')
        page.evaluate('() => attributesReady')
        attrs = page.evaluate('getAttrs()')
        for field in ('vitality','base_attack','strain_level','parry_level','haste_level'):
            assert attrs[field] == panel['raw'][field], (field,attrs,panel['raw'])
        assert page.evaluate('getTarget().level') == 54
        assert 'v2.0.13-20260914' in page.locator('.tb-brand-tag').inner_text()
        page.reload(wait_until='networkidle')
        page.evaluate('() => attributesReady')
        assert page.evaluate('getAttrs().vitality') == panel['raw']['vitality']
        assert not errors, errors
        browser.close()
    # A manually saved level50 profile takes priority over equipment defaults.
    api('/api/attrs/save', attrs, as_text=True)
    assert api('/api/attrs/load')['vitality'] == attrs['vitality']
    switch('AnYingQianJi')
    assert api('/api/attrs/load') == old_attributes
    assert api('/api/attrs/load_profile?name=level-regression') == old_attributes
    print('PASS: separate level50/130 storage and equipment; old shared cache cannot restore old attributes; reload preserves level50 inputs')
finally:
    api('/api/mounts/switch', dict(version=original['version'],mount=original['mount'],persist=False))
