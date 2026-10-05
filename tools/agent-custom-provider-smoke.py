"""Custom-provider UI and HTTP integration against an isolated local worker + mock LLM.

Run only on a disposable worker (default port 3039); creates one fixture Agent session.
No real credentials or external provider requests are used.
"""
import argparse
import json
import re
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import urlsplit
from urllib.request import Request, urlopen
from playwright.sync_api import sync_playwright, expect

parser = argparse.ArgumentParser()
parser.add_argument('--backend', default='http://127.0.0.1:3039')
parser.add_argument('--userdata', type=Path, required=True)
parser.add_argument('--screenshots', type=Path)
args = parser.parse_args()
assert urlsplit(args.backend).hostname in ('localhost', '127.0.0.1')
assert urlsplit(args.backend).port not in (3005, 3006), 'Use a disposable worker.'
KEY = 'fixture-only-custom-secret'
HEADERS = {'Content-Type': 'application/json', 'X-JX3-Provider-Settings': '1'}
calls = []
agent_calls = []

def api(path, body=None, method='GET'):
    request = Request(args.backend + path, data=json.dumps(body).encode() if body is not None else None,
                      headers=HEADERS, method=method)
    with urlopen(request, timeout=50) as response:
        raw = response.read().decode()
        assert KEY not in raw
        return json.loads(raw)

class MockLlm(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        assert self.headers['Authorization'] == 'Bearer ' + KEY
        assert KEY not in json.dumps(body)
        calls.append((self.path, body))
        tools = [t.get('function', t)['name'] for t in body.get('tools', [])]
        is_responses = self.path.endswith('/responses')
        if body['model'] == 'fixture-unauthorized':
            self.send_response(401)
            self.end_headers()
            self.wfile.write(json.dumps({'error': {'message': KEY}}).encode())
            return
        if 'connection_test' in tools:
            name, arguments = 'connection_test', {'ok': True}
        else:
            agent_calls.append(body)
            step = len(agent_calls)
            name = 'get_current_scenario' if step == 1 else 'simulate_scenario' if step == 2 else None
            arguments = {}
        transcript = '\n'.join(m.get('content') or '' for m in body.get('messages', []) if isinstance(m.get('content'), str))
        evidence_ids = list(dict.fromkeys(re.findall(r'"evidence_id"\s*:\s*"([0-9a-f]{64})"', transcript)))
        findings = [{'title': '基线模拟', 'explanation': '已通过工具执行当前场景。', 'evidence_ids': evidence_ids, 'metrics': []}] if evidence_ids else []
        report = json.dumps({'schema_version': 'agent-report-content/v1', 'summary': '自定义接口回归完成。',
                             'findings': findings, 'recommendations': [], 'limitations': ['仅用于接口集成测试。'],
                             'refusal_reason': None}, ensure_ascii=False)
        if is_responses:
            output = [{'type': 'function_call', 'call_id': 'fixture-call-' + str(len(calls)), 'name': name, 'arguments': json.dumps(arguments)}] if name else [
                {'type': 'message', 'role': 'assistant', 'content': [{'type': 'output_text', 'text': report}]}]
            result = {'status': 'completed', 'output': output}
        else:
            message = {'role': 'assistant', 'content': None if name else report}
            if name:
                message['tool_calls'] = [{'id': 'fixture-call-' + str(len(calls)), 'type': 'function',
                                         'function': {'name': name, 'arguments': json.dumps(arguments)}}]
            result = {'choices': [{'finish_reason': 'tool_calls' if name else 'stop', 'message': message}]}
        encoded = json.dumps(result).encode()
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)

server = ThreadingHTTPServer(('127.0.0.1', 0), MockLlm)
threading.Thread(target=server.serve_forever, daemon=True).start()
base = f'http://127.0.0.1:{server.server_port}/v1'
initial = api('/api/agent/providers/custom')
assert initial['config'] is None, 'Refusing to overwrite an existing custom profile.'

try:
    with sync_playwright() as pw:
        browser = pw.chromium.launch(channel='msedge', headless=True)
        page = browser.new_page(viewport={'width': 1440, 'height': 1000})
        errors = []
        page.on('pageerror', lambda e: errors.append(str(e)))

        def route_api(route):
            path = urlsplit(route.request.url).path
            if path.startswith('/api/agent/providers') or route.request.method == 'GET' or path in ('/api/calculate', '/api/skill_damage'):
                route.continue_()
            else:
                route.fulfill(json={'ok': True})
        page.route('**/api/**', route_api)
        page.goto(args.backend, wait_until='networkidle')
        page.wait_for_selector('.agent-provider-dialog', state='attached')
        page.evaluate("Jx3Nav.switchPage('page-agent')")
        page.locator('#page-agent [data-agent-provider-settings]').click()
        dialog = page.locator('.agent-provider-dialog')
        expect(dialog).to_be_visible()
        expect(page.locator('#custom_provider_url')).to_be_enabled()
        page.locator('#custom_provider_label').fill('我的测试接口')
        page.locator('#custom_provider_url').fill(base + '/chat/completions')
        page.locator('#custom_provider_model').fill('fixture-model')
        page.locator('#custom_provider_key').fill(KEY)
        page.locator('[data-test]').click()
        expect(page.locator('#custom_provider_status')).to_contain_text('连接和工具调用测试通过')
        assert api('/api/agent/providers/custom')['config'] is None
        if args.screenshots:
            args.screenshots.mkdir(parents=True, exist_ok=True)
            for theme in ('', 'light-theme', 'ink-theme'):
                page.evaluate("theme => {document.body.classList.remove('light-theme', 'ink-theme'); if(theme) document.body.classList.add(theme)}", theme)
                dialog.screenshot(path=str(args.screenshots / f'{theme or "dark"}.png'))
        page.locator('[data-save]').click()
        expect(dialog).not_to_be_visible()
        expect(page.locator('#agent_provider')).to_have_value('user-custom')
        expect(page.locator('#sim_ai_provider')).to_have_value('user-custom')
        assert page.locator('#custom_provider_key').input_value() == ''
        assert api('/api/agent/providers/custom')['has_key']
        disk = (args.userdata / 'agent_custom_provider.json').read_text(encoding='utf-8')
        assert KEY not in disk and 'api_key' not in disk
        page.reload(wait_until='networkidle')
        page.evaluate("Jx3Nav.switchPage('page-agent')")
        expect(page.locator('#agent_provider')).to_have_value('user-custom')
        page.locator('#page-agent [data-agent-provider-settings]').click()
        expect(page.locator('#custom_provider_key')).to_have_attribute('placeholder', '已设置，留空保留')
        # Editing the model/protocol at the same endpoint retains the server key.
        page.locator('#custom_provider_protocol').select_option('responses')
        page.locator('[data-test]').click()
        expect(page.locator('#custom_provider_status')).to_contain_text('连接和工具调用测试通过')
        assert calls[-1][0] == '/v1/responses'
        page.locator('#custom_provider_protocol').select_option('deepseek')
        page.locator('[data-test]').click()
        expect(page.locator('#custom_provider_status')).to_contain_text('连接和工具调用测试通过')
        assert calls[-1][0] == '/v1/chat/completions'
        page.locator('#custom_provider_model').fill('fixture-unauthorized')
        page.locator('[data-test]').click()
        expect(page.locator('#custom_provider_status')).to_contain_text('API Key 无效')
        assert KEY not in dialog.inner_text()
        page.locator('#custom_provider_model').fill('fixture-model')
        page.locator('#custom_provider_url').fill(base + '/other')
        before = len(calls)
        page.locator('[data-test]').click()
        expect(page.locator('#custom_provider_status')).to_contain_text('请为这个 API 地址填写 API Key')
        assert len(calls) == before
        page.locator('#custom_provider_url').fill(base)
        page.locator('#custom_provider_protocol').select_option('chat_completions')
        page.locator('[data-save]').click()
        expect(dialog).not_to_be_visible()
        # Real Agent HTTP orchestration: custom profile -> mock model -> Rust tools.
        simulation = {'sequence': ['盾击', '盾压'], 'haste_level': 42087, 'network_delay': 0,
                      'attributes': {'base_attack': 38466, 'weapon_damage': 10986, 'crit_level': 54841,
                                     'crit_effect_level': 0, 'overcome_level': 29480, 'strain_level': 66031, 'haste_level': 42087},
                      'target': {'level': 134, 'defense_bonus': 0, 'damage_cof': 0}, 'initial_rage': 50, 'tiegu_mode': 2}
        created = api('/api/agent/runs', {'provider_profile': 'user-custom', 'question': '运行当前循环基线。', 'simulation': simulation}, 'POST')
        for _ in range(150):
            result = api(created['status_url'])
            if result.get('result'):
                break
            time.sleep(.1)
        assert result.get('result'), 'Custom Agent run did not complete.'
        result = result['result']
        assert result['status'] == 'completed', (result['status'], result.get('error'))
        assert result['provider_profile'] == 'user-custom'
        assert result['model'] == 'fixture-model'
        assert len(agent_calls) >= 3
        assert any(t.get('tool_name') == 'simulate_scenario' for t in result['trace'])
        assert KEY not in json.dumps(api(created['session_url']))
        # Open from dock, check responsive layout and Escape leaves dock open.
        page.evaluate("Jx3Nav.switchPage('page-sim')")
        page.locator('#sim_ai_fab').click()
        page.locator('#sim_ai_dock [data-agent-provider-settings]').click()
        expect(dialog).to_be_visible()
        page.set_viewport_size({'width': 390, 'height': 844})
        expect(page.locator('#custom_provider_url')).to_be_enabled()
        assert dialog.evaluate('(el) => el.scrollWidth <= el.clientWidth + 1')
        if args.screenshots:
            dialog.screenshot(path=str(args.screenshots / 'mobile.png'))
        page.keyboard.press('Escape')
        expect(dialog).not_to_be_visible()
        expect(page.locator('#sim_ai_dock')).to_have_class(re.compile(r'.*\bopen\b.*'))
        page.locator('#sim_ai_dock [data-agent-provider-settings]').click()
        expect(page.locator('[data-remove]')).to_be_enabled()
        page.locator('[data-remove]').click()
        expect(page.locator('#custom_provider_status')).to_contain_text('自定义接口已移除')
        assert api('/api/agent/providers/custom')['config'] is None
        expect(page.locator('#agent_provider option[value="user-custom"]')).to_have_count(0)
        storage = page.evaluate('JSON.stringify({local: {...localStorage}, session: {...sessionStorage}})')
        assert KEY not in storage
        assert not errors, errors
        browser.close()
    for path in args.userdata.rglob('*'):
        if path.is_file():
            assert KEY.encode() not in path.read_bytes(), f'Fixture credential persisted in {path.name}'
    print('PASS: custom API save/reload/edit/remove; three protocols; safe errors; Agent tool loop; dock/mobile; no persisted key.')
finally:
    api('/api/agent/providers/custom', method='DELETE')
    server.shutdown()
    server.server_close()
