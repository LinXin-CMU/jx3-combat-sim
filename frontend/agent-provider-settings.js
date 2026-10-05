// User-owned provider settings. Keys only exist in the password field and the
// dedicated request; never persist them in browser storage or Agent payloads.
(function () {
  'use strict';
  const endpoint = '/api/agent/providers/custom';
  const dialog = document.createElement('dialog');
  dialog.className = 'agent-provider-dialog';
  dialog.setAttribute('aria-labelledby', 'custom_provider_title');
  dialog.innerHTML = `
    <form id="custom_provider_form" autocomplete="off">
      <header><div><span class="agent-eyebrow">LLM API</span><h2 id="custom_provider_title">自定义接口</h2></div>
        <button type="button" class="sim-btn" data-close aria-label="关闭接口设置">×</button></header>
      <label for="custom_provider_label">显示名称</label>
      <input class="sim-input" id="custom_provider_label" maxlength="40" placeholder="我的模型">
      <label for="custom_provider_protocol">接口协议</label>
      <select class="sim-input" id="custom_provider_protocol">
        <option value="chat_completions">OpenAI 兼容 · Chat Completions</option>
        <option value="responses">OpenAI · Responses</option>
        <option value="deepseek">DeepSeek 兼容</option>
      </select>
      <label for="custom_provider_url">API 地址</label>
      <input class="sim-input" id="custom_provider_url" type="url" required maxlength="2048" placeholder="https://api.example.com/v1" spellcheck="false" autocapitalize="off" aria-describedby="custom_provider_url_hint">
      <small id="custom_provider_url_hint">填写服务商提供的接口地址，例如以 /v1 结尾的地址。</small>
      <label for="custom_provider_model">模型名称</label>
      <input class="sim-input" id="custom_provider_model" required maxlength="128" placeholder="填写服务商的模型 ID" spellcheck="false" autocapitalize="off">
      <label for="custom_provider_key">API Key</label>
      <input class="sim-input" id="custom_provider_key" type="password" maxlength="4096" autocomplete="new-password" spellcheck="false" autocapitalize="off" aria-describedby="custom_provider_key_hint">
      <small id="custom_provider_key_hint">Key 在当前服务中保留，服务重启后需重新填写。</small>
      <p id="custom_provider_status" role="status" aria-live="polite"></p>
      <footer><button type="button" class="sim-btn" data-remove>移除</button><div>
        <button type="button" class="sim-btn" data-test>测试连接</button>
        <button type="submit" class="sim-btn sim-btn-primary" data-save>保存并使用</button>
      </div></footer>
    </form>`;
  document.body.appendChild(dialog);
  const form = dialog.querySelector('form');
  const fields = Object.fromEntries(['label', 'protocol', 'url', 'model', 'key'].map(name => [name, dialog.querySelector(`#custom_provider_${name}`)]));
  const status = dialog.querySelector('#custom_provider_status');
  const remove = dialog.querySelector('[data-remove]');
  let busy = false;
  let config = null;
  let controller = null;
  let generation = 0;
  const securePage = location.protocol === 'https:' || ['localhost', '127.0.0.1', '[::1]'].includes(location.hostname);

  function message(text, isError = false) {
    status.textContent = text;
    status.classList.toggle('is-error', isError);
  }
  function setBusy(value) {
    busy = value;
    form.querySelectorAll('input, select, button:not([data-close])').forEach(el => { el.disabled = value || !securePage; });
    remove.disabled = value || !config || !securePage;
    form.setAttribute('aria-busy', String(value));
  }
  function resetKey() { fields.key.value = ''; }
  function changed(selectedId) {
    window.dispatchEvent(new CustomEvent('jx3-agent-providers-changed', { detail: { selectedId } }));
  }
  function fill(body) {
    config = body.config || null;
    fields.label.value = config?.label || '';
    fields.protocol.value = config?.protocol || 'chat_completions';
    fields.url.value = config?.base_url || '';
    fields.model.value = config?.model || '';
    fields.key.placeholder = body.has_key ? '已设置，留空保留' : '输入 API Key';
    resetKey();
    setBusy(false);
  }
  const errors = {
    provider_http_401: 'API Key 无效，请检查后重试。',
    provider_http_403: '接口拒绝访问，请检查 Key 权限和模型权限。',
    provider_http_404: '接口或模型不存在，请检查地址、协议和模型名。',
    provider_balance_insufficient: '接口余额不足。',
    provider_http_429: '请求过于频繁或额度不足，请稍后重试。',
    provider_timeout: '连接测试超时，请检查接口地址与模型。',
    provider_network_error: '连接失败，请检查 API 地址及服务端网络。',
    provider_http_400: '接口不接受测试请求，请检查模型和所选协议。',
    provider_http_5xx: '服务商暂时不可用，请稍后重试。',
    provider_response_invalid: '接口响应格式不匹配，请检查所选协议。'
  };
  async function request(method, suffix = '', body) {
    controller = new AbortController();
    const response = await fetch(endpoint + suffix, {
      method, cache: 'no-store', signal: controller.signal,
      headers: { 'Content-Type': 'application/json', 'X-JX3-Provider-Settings': '1' },
      ...(body ? { body: JSON.stringify(body) } : {})
    });
    const result = await response.json().catch(() => null);
    if (!response.ok) {
      const e = result?.error;
      throw new Error(errors[e?.code] || (e?.code?.startsWith('custom_') ? e.message : '')
        || (response.status === 401 ? '登录已过期，请重新登录。' : '接口请求失败，请重试。'));
    }
    if (!result) throw new Error('接口返回格式不正确。');
    return result;
  }

  async function open() {
    if (dialog.open) return;
    const current = ++generation;
    form.reset(); config = null; resetKey();
    dialog.showModal();
    if (!securePage) {
      setBusy(false);
      message('请通过 HTTPS 或本机地址打开模拟器，再配置 API Key。', true);
      return;
    }
    setBusy(true); message('正在读取接口配置…');
    try {
      const body = await request('GET');
      if (!dialog.open || current !== generation) return;
      fill(body);
      message(body.config && !body.has_key ? '填写 API Key 即可继续使用这个接口。' : '');
      (body.config ? fields.key : fields.url).focus();
    } catch (e) {
      if (current === generation && e.name !== 'AbortError') message(e.message, true);
    } finally { if (current === generation) setBusy(false); }
  }

  async function submit(testOnly) {
    if (busy || !securePage || !form.reportValidity()) return;
    const current = generation;
    const body = { label: fields.label.value, protocol: fields.protocol.value,
      base_url: fields.url.value, model: fields.model.value, api_key: fields.key.value };
    setBusy(true); message(testOnly ? '正在测试连接与工具调用…' : '正在保存…');
    try {
      const result = await request(testOnly ? 'POST' : 'PUT', testOnly ? '/test' : '', body);
      if (!dialog.open || current !== generation) return;
      if (testOnly) {
        message(result.tool_calling
          ? `连接和工具调用测试通过，用时 ${(result.latency_ms / 1000).toFixed(1)} 秒。`
          : '连接成功，但模型未完成工具调用测试；Agent 分析需要支持工具调用的模型。');
      } else {
        fill(result); changed(result.profile_id); dialog.close();
      }
    } catch (e) {
      if (current === generation && e.name !== 'AbortError') message(e.message, true);
    } finally {
      body.api_key = '';
      if (current === generation) setBusy(false);
    }
  }

  document.querySelectorAll('[data-agent-provider-settings]').forEach(button => button.addEventListener('click', open));
  form.addEventListener('submit', event => { event.preventDefault(); submit(false); });
  dialog.querySelector('[data-test]').addEventListener('click', () => submit(true));
  dialog.querySelector('[data-close]').addEventListener('click', () => dialog.close());
  dialog.addEventListener('close', () => { generation++; resetKey(); controller?.abort(); });
  remove.addEventListener('click', async () => {
    if (busy || !config) return;
    const current = generation;
    setBusy(true); message('正在移除…');
    try {
      await request('DELETE');
      changed();
      if (current !== generation) return;
      fill({}); message('自定义接口已移除。');
    } catch (e) { if (current === generation && e.name !== 'AbortError') message(e.message, true); }
    finally { if (current === generation) setBusy(false); }
  });
})();
