"""Replay design-analysis questions against an isolated worker; retain reviewable reports.

The input is a ScenarioSnapshotV1 JSON supplied by the operator. No credentials,
raw provider transcripts or real user storage are copied into the output.
"""
import argparse
import json
import time
from pathlib import Path
from urllib.request import Request, urlopen
from urllib.parse import urlsplit

parser = argparse.ArgumentParser()
parser.add_argument('--backend', default='http://127.0.0.1:3038')
parser.add_argument('--scenario', required=True)
parser.add_argument('--output', required=True)
parser.add_argument('--provider-profile', default='deepseek-v4-flash')
parser.add_argument('--turns', type=int, choices=(1,2,3), default=3)
parser.add_argument('--review-only', action='store_true', help='Check saved results without calling a model.')
args = parser.parse_args()
assert urlsplit(args.backend).hostname in ('localhost', '127.0.0.1')

def check_rows(rows):
    # Necessary checks, not a substitute for reading the planner's argument.
    assert len(rows) == args.turns
    assert all(row['status'] in ('completed','partially_verified') for row in rows)
    assert all(row['clarification'] is None for row in rows)
    assert all(len(row['report'].get('body_markdown','')) >= 500 for row in rows)
    assert any(call['tool_name'] == 'lookup_skill_definitions' for row in rows for call in row['calls'])
    for row in rows:
        for call in row['calls']:
            if call['tool_name'] != 'compare_scenarios':
                continue
            # A bounded capability probe or legal A baseline is legitimate.
            # Neither a macro edit nor an illegal talent stack represents B.
            assert all(set(candidate['patch']).issubset({'talents'})
                       for candidate in call['arguments']['candidates'])
            if call.get('ok') is False:
                assert call.get('code') in ('unavailable_candidate_action', 'conflicting_candidate_talents')
                assert not call.get('evidence_ids')
            else:
                assert call.get('ok') is True and call.get('evidence_ids')
                original_talents = json.loads(Path(args.scenario).read_text(encoding='utf-8-sig'))['simulation']['talents']
                for candidate in call['arguments']['candidates']:
                    selected = candidate['patch'].get('talents', original_talents)
                    assert 30769 not in selected
                    assert len(selected) <= len(original_talents)
    assert all('暴怒' in row['report']['body_markdown'] and '格挡' in row['report']['body_markdown'] for row in rows)
    assert all('这项数值尚待核验' not in row['report']['body_markdown'] for row in rows)
    assert all(not row['report']['body_markdown'].lstrip().startswith('{"schema_version"') for row in rows)
    print(f'PASS: {len(rows)} continuous design answers, no clarification or substitute macro A/B; review prose in output.')

if args.review_only:
    check_rows(json.loads(Path(args.output).read_text(encoding='utf-8')))
    raise SystemExit(0)

def api(path, body=None):
    request = Request(args.backend + path, data=None if body is None else json.dumps(body).encode(),
                      headers={'Content-Type': 'application/json'})
    with urlopen(request, timeout=25) as response:
        return json.load(response)

source = json.loads(Path(args.scenario).read_text(encoding='utf-8-sig'))
versions = {'2026_10_cangsheng_zhushi_test':'CangShengZhuShiTest'}
mounts = {'tieguyi':'TieGuYi', 'fenshanjin':'FenShanJin'}
original = api('/api/mounts/current')
rows = []
session = None
questions = [
    '如果把断马换成阵云，有什么好处？写一个商业化策划案分析',
    '就是假设把第一重的断马换成分山测试服版本的阵云，对输出循环有什么影响',
    '你自己看模拟器数据库',
]
try:
    api('/api/mounts/switch', {'version':versions[source['game_version']], 'mount':mounts[source['mount']], 'persist':False})
    for question in questions[:args.turns]:
        body = {'question':question, 'provider_profile':args.provider_profile, 'simulation':source['simulation']}
        if session:
            body['session_id'] = session
        created = api('/api/agent/runs', body)
        session = created['session_id']
        deadline = time.monotonic() + 660
        while True:
            status = api(created['status_url'])
            if not status['running']:
                break
            if time.monotonic() > deadline:
                raise TimeoutError(created['run_id'])
            time.sleep(.5)
        result = status['result']
        calls = result.get('debug', {}).get('tool_calls', [])
        content = (result.get('report') or {}).get('content') or {}
        row = {'question':question, 'session_id':session, 'run_id':created['run_id'],
               'provider_profile':args.provider_profile, 'model':result.get('model'),
               'status':result['status'], 'accounting':result['accounting'],
               'prompt':result['prompt_version'], 'calls':calls, 'report':content,
               'clarification':result.get('clarification'), 'error':result.get('error')}
        rows.append(row)
        Path(args.output).write_text(json.dumps(rows, ensure_ascii=False, indent=2), encoding='utf-8')
        print(json.dumps({'turn':len(rows), 'status':row['status'],
            'tools':[call['tool_name'] for call in calls],
            'seconds':round(row['accounting']['duration_ms']/1000,1),
            'tokens':row['accounting']['total_tokens'],
            'body_chars':len(content.get('body_markdown',''))}, ensure_ascii=False), flush=True)
finally:
    api('/api/mounts/switch', {'version':original['version'], 'mount':original['mount'], 'persist':False})

check_rows(rows)
