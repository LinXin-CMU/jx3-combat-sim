// Grounded combat-analysis Agent panel. No credentials or provider payloads enter browser state.
(function initAgentPanel() {
  'use strict';

  const page = document.getElementById('page-agent');
  if (!page) return;

  const els = {
    sessions: document.getElementById('agent_session_list'),
    transcript: document.getElementById('agent_transcript'),
    question: document.getElementById('agent_question'),
    provider: document.getElementById('agent_provider'),
    providerModel: document.getElementById('agent_provider_model'),
    run: document.getElementById('agent_run'),
    cancel: document.getElementById('agent_cancel'),
    status: document.getElementById('agent_composer_status'),
    scenario: document.getElementById('agent_scenario_state'),
    copy: document.getElementById('agent_copy_summary'),
    newSession: document.getElementById('agent_new_session'),
    goSim: document.getElementById('agent_go_sim'),
  };

  let initialized = false;
  let currentSessionId = null;
  let activeRun = null;
  let activeSource = null;
  let activeTrace = null;
  let latestResult = null;

  const traceLabels = {
    planning: '拆解问题',
    tool_started: '调用工具',
    tool_finished: '取得证据',
    validating: '校验证据',
    report_repair_requested: '修复报告',
    completed: '分析完成',
    refused: '安全拒绝',
    cancelled: '任务取消',
    evidence_insufficient: '证据不足',
    budget_exhausted: '预算耗尽',
    provider_failed: 'Provider 故障',
    protocol_failed: '协议故障',
    timed_out: '任务超时',
    cancel_requested: '请求取消',
  };

  const statusLabels = {
    created: '已创建', running: '运行中', completed: '已完成', refused: '已拒绝',
    cancelled: '已取消', interrupted: '已中断', evidence_insufficient: '证据不足',
    budget_exhausted: '预算耗尽', provider_failed: 'Provider 故障',
    protocol_failed: '协议故障', timed_out: '超时', finished: '已结束',
  };

  function clear(node) {
    while (node.firstChild) node.removeChild(node.firstChild);
  }

  function element(tag, className, text) {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text != null) node.textContent = text;
    return node;
  }

  function setStatus(text, isError) {
    els.status.textContent = text;
    els.status.classList.toggle('agent-error', !!isError);
  }

  function setBusy(busy) {
    els.run.disabled = busy;
    els.run.hidden = busy;
    els.cancel.hidden = !busy;
    els.provider.disabled = busy;
  }

  function updateScenarioState() {
    const scenario = window._lastSimBody;
    if (!scenario) {
      els.scenario.classList.remove('ready');
      els.scenario.lastChild.textContent = ' 尚未捕获循环场景';
      return;
    }
    const mode = scenario.macro_text ? '宏循环' : `${(scenario.sequence || []).length} 个技能`;
    const delay = Number(scenario.network_delay || 0);
    els.scenario.classList.add('ready');
    els.scenario.lastChild.textContent = ` 已捕获当前场景 · ${mode} · ${delay}ms 延迟`;
  }

  async function safeJson(response) {
    try { return await response.json(); } catch (_) { return null; }
  }

  async function loadProviders() {
    try {
      const response = await fetch('/api/agent/providers', { cache: 'no-store' });
      if (!response.ok) throw new Error('Provider 列表不可用');
      const body = await response.json();
      clear(els.provider);
      const profiles = Array.isArray(body.profiles) ? body.profiles : [];
      profiles.forEach(profile => {
        const option = document.createElement('option');
        option.value = profile.id;
        option.textContent = `${profile.label} · ${profile.model}${profile.available ? '' : '（不可用）'}`;
        option.disabled = !profile.available;
        option.dataset.model = profile.model;
        els.provider.appendChild(option);
      });
      const available = profiles.find(profile => profile.available);
      if (available) els.provider.value = available.id;
      syncProviderModel();
    } catch (error) {
      setStatus(error.message || 'Provider 列表加载失败', true);
    }
  }

  function syncProviderModel() {
    const option = els.provider.selectedOptions[0];
    els.providerModel.textContent = option?.dataset.model || '—';
  }

  function formatTime(ms) {
    if (!ms) return '—';
    return new Date(ms).toLocaleString('zh-CN', { month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' });
  }

  async function loadSessions() {
    try {
      const response = await fetch('/api/agent/sessions', { cache: 'no-store' });
      if (!response.ok) throw new Error('会话列表不可用');
      const body = await response.json();
      renderSessionList(Array.isArray(body.sessions) ? body.sessions : []);
    } catch (error) {
      clear(els.sessions);
      els.sessions.appendChild(element('div', 'agent-empty agent-error', error.message || '会话读取失败'));
    }
  }

  function renderSessionList(sessions) {
    clear(els.sessions);
    if (!sessions.length) {
      els.sessions.appendChild(element('div', 'agent-empty', '还没有分析会话。首次运行后会在当前账号目录中增量保存。'));
      return;
    }
    sessions.forEach(session => {
      const button = element('button', 'agent-session-item');
      button.type = 'button';
      button.dataset.sessionId = session.session_id;
      if (session.session_id === currentSessionId) button.classList.add('active');
      button.appendChild(element('span', 'agent-session-title', session.title));
      const meta = element('span', 'agent-session-meta');
      meta.appendChild(element('span', 'agent-session-status', statusLabels[session.status] || session.status));
      meta.appendChild(element('span', '', formatTime(session.updated_at_ms)));
      button.appendChild(meta);
      if (session.corrupted_event_count) button.title = `检测到 ${session.corrupted_event_count} 个损坏事件；原文件未被覆盖`;
      button.addEventListener('click', () => openSession(session.session_id));
      els.sessions.appendChild(button);
    });
  }

  function appendMessage(role, text) {
    const wrap = element('article', `agent-message ${role}`);
    wrap.appendChild(element('div', 'agent-message-label', role === 'user' ? '策划问题' : 'Agent 结论'));
    wrap.appendChild(element('div', 'agent-message-body', text || '—'));
    els.transcript.appendChild(wrap);
    scrollTranscript();
    return wrap;
  }

  function createTrace(runId) {
    const wrap = element('section', 'agent-trace');
    const title = element('div', 'agent-trace-title');
    title.appendChild(element('span', '', '规范化执行轨迹'));
    title.appendChild(element('span', '', runId || '—'));
    const steps = element('div', 'agent-trace-steps');
    wrap.appendChild(title);
    wrap.appendChild(steps);
    els.transcript.appendChild(wrap);
    return { wrap, steps, seen: new Set() };
  }

  function appendTraceStep(trace, event) {
    if (!trace || !event) return;
    const key = `${event.sequence || ''}:${event.kind || event.trace_kind || ''}`;
    if (trace.seen.has(key)) return;
    trace.seen.add(key);
    const kind = event.trace_kind || event.kind;
    const suffix = event.tool_name ? ` · ${event.tool_name}` : '';
    const step = element('span', 'agent-trace-step', `${traceLabels[kind] || kind}${suffix}`);
    if (event.code) step.title = event.code;
    trace.steps.appendChild(step);
    scrollTranscript();
  }

  function metricValue(metric) {
    const value = Number(metric?.value);
    return Number.isFinite(value) ? value.toLocaleString('zh-CN', { maximumFractionDigits: 2 }) : '—';
  }

  function renderReport(result) {
    latestResult = result || null;
    els.copy.disabled = !latestResult;
    const report = result?.report;
    if (!report) {
      const reason = result?.error?.message || `任务状态：${statusLabels[result?.status] || result?.status || '未知'}`;
      appendMessage('agent', reason);
      return;
    }
    const card = element('article', 'agent-report');
    const head = element('div', 'agent-report-head');
    head.appendChild(element('b', '', '已验证分析报告'));
    head.appendChild(element('span', '', `${report.provider_profile} / ${report.model} · ${result.accounting?.duration_ms || 0}ms`));
    card.appendChild(head);
    card.appendChild(element('div', 'agent-report-summary', report.content?.summary || '—'));

    (report.content?.findings || []).forEach(finding => {
      const block = element('section', 'agent-finding');
      block.appendChild(element('h4', '', finding.title));
      block.appendChild(element('p', '', finding.explanation));
      if (finding.metrics?.length) {
        const metrics = element('div', 'agent-metrics');
        finding.metrics.forEach(metric => {
          const metricNode = element('div', 'agent-metric');
          metricNode.appendChild(element('b', '', metricValue(metric)));
          metricNode.appendChild(element('span', '', `${metric.label} · ${metric.unit}`));
          metricNode.title = `${metric.evidence_id}${metric.json_pointer}`;
          metrics.appendChild(metricNode);
        });
        block.appendChild(metrics);
      }
      card.appendChild(block);
    });

    const recommendations = report.content?.recommendations || [];
    if (recommendations.length) {
      const block = element('section', 'agent-finding');
      block.appendChild(element('h4', '', '建议的下一步实验'));
      recommendations.forEach(item => block.appendChild(element('p', '', `${item.title}：${item.rationale}`)));
      card.appendChild(block);
    }
    const limitations = report.content?.limitations || [];
    if (limitations.length) {
      const block = element('section', 'agent-finding');
      block.appendChild(element('h4', '', '边界与限制'));
      limitations.forEach(item => block.appendChild(element('p', '', `• ${item}`)));
      card.appendChild(block);
    }
    card.appendChild(element('div', 'agent-evidence', `scenario ${result.scenario_hash} · prompt ${result.prompt_version} / ${result.prompt_sha256} · evidence ${(report.evidence_ids || []).join(', ') || 'none'}`));
    els.transcript.appendChild(card);
    scrollTranscript();
  }

  function scrollTranscript() {
    requestAnimationFrame(() => { els.transcript.scrollTop = els.transcript.scrollHeight; });
  }

  function renderWelcome() {
    clear(els.transcript);
    const welcome = element('div', 'agent-welcome');
    welcome.appendChild(element('div', 'agent-welcome-mark', '✦'));
    welcome.appendChild(element('h3', '', '从一个可验证的问题开始'));
    welcome.appendChild(element('p', '', '先在“循环模拟”准备场景，再问 Agent 当前基线、候选改动或时间轴异常。离线 profile 可完整演示工具与证据闭环，不产生模型费用。'));
    const starters = element('div', 'agent-starter-grid');
    [
      ['分析当前基线', '分析当前循环的输出基线，并说明证据边界。'],
      ['诊断时间轴', '找出当前循环中值得进一步验证的时间轴问题。'],
      ['检查证据边界', '说明这个场景还缺少哪些证据，避免给出未经验证的结论。'],
    ].forEach(([label, question]) => {
      const button = element('button', '', label);
      button.type = 'button';
      button.addEventListener('click', () => { els.question.value = question; els.question.focus(); });
      starters.appendChild(button);
    });
    welcome.appendChild(starters);
    els.transcript.appendChild(welcome);
  }

  async function openSession(sessionId) {
    if (activeRun) return;
    try {
      const response = await fetch(`/api/agent/sessions/${encodeURIComponent(sessionId)}`, { cache: 'no-store' });
      const body = await safeJson(response);
      if (!response.ok) throw new Error(body?.error?.message || '会话读取失败');
      currentSessionId = sessionId;
      clear(els.transcript);
      let trace = null;
      let traceRunId = null;
      body.events.forEach(event => {
        if (event.kind === 'user_message') {
          appendMessage('user', event.question);
        } else if (event.kind === 'run_started') {
          trace = createTrace(event.run_id);
          traceRunId = event.run_id;
          appendTraceStep(trace, { sequence: event.sequence, kind: 'planning' });
        } else if (event.kind === 'run_trace') {
          if (!trace || traceRunId !== event.run_id) {
            trace = createTrace(event.run_id);
            traceRunId = event.run_id;
          }
          appendTraceStep(trace, event);
        } else if (event.kind === 'cancel_requested' || event.kind === 'run_interrupted') {
          if (!trace || traceRunId !== event.run_id) trace = createTrace(event.run_id);
          appendTraceStep(trace, { ...event, trace_kind: event.kind });
        } else if (event.kind === 'run_result') {
          renderReport(event.result || { status: 'finished', scenario_hash: event.scenario_hash });
        }
      });
      if (!body.events.length) renderWelcome();
      if (body.summary.corrupted_event_count) {
        appendMessage('agent', `检测到 ${body.summary.corrupted_event_count} 个损坏事件。系统已保留原文件，未覆盖或自动修复；请新建会话继续分析。`);
      }
      setStatus(`已恢复会话 · ${statusLabels[body.summary.status] || body.summary.status}`);
      await loadSessions();
    } catch (error) {
      setStatus(error.message || '会话读取失败', true);
    }
  }

  async function captureScenario() {
    if (!window._lastSimBody && typeof window.runSimulate === 'function') {
      await window.runSimulate();
    } else if (!window._lastSimBody && typeof runSimulate === 'function') {
      await runSimulate();
    }
    if (!window._lastSimBody) {
      throw new Error('当前没有可分析的循环。请先在循环模拟中添加技能或宏并运行一次。');
    }
    const scenario = JSON.parse(JSON.stringify(window._lastSimBody));
    delete scenario.lite;
    delete scenario.lite_keep_timeline;
    updateScenarioState();
    return scenario;
  }

  async function startRun() {
    if (activeRun) return;
    const question = els.question.value.trim();
    if (!question) { setStatus('请先输入一个策划问题', true); els.question.focus(); return; }
    setBusy(true);
    setStatus('正在冻结当前场景…');
    try {
      const simulation = await captureScenario();
      appendMessage('user', question);
      activeTrace = createTrace('准备创建 run');
      const payload = {
        question,
        provider_profile: els.provider.value,
        simulation,
      };
      if (currentSessionId) payload.session_id = currentSessionId;
      const response = await fetch('/api/agent/runs', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(payload),
      });
      const body = await safeJson(response);
      if (!response.ok) throw new Error(body?.error?.message || `创建任务失败 (${response.status})`);
      activeRun = body;
      currentSessionId = body.session_id;
      activeTrace.wrap.querySelector('.agent-trace-title span:last-child').textContent = body.run_id;
      els.question.value = '';
      setStatus(`运行中 · ${body.run_id} · 场景 ${body.scenario_hash.slice(0, 12)}…`);
      connectStream(body.stream_url);
      await loadSessions();
    } catch (error) {
      setBusy(false);
      activeRun = null;
      setStatus(error.message || 'Agent 运行失败', true);
    }
  }

  function connectStream(url) {
    if (activeSource) activeSource.close();
    const source = new EventSource(url);
    activeSource = source;
    const eventKinds = Object.keys(traceLabels).concat(['run_result']);
    eventKinds.forEach(kind => {
      source.addEventListener(kind, raw => {
        let event;
        try { event = JSON.parse(raw.data); } catch (_) { return; }
        if (kind === 'run_result') {
          source.close();
          activeSource = null;
          renderReport(event.result);
          const persistenceError = !!event.persistence_error;
          setStatus(persistenceError ? '分析完成，但会话落盘失败' : '分析完成 · 结论已绑定证据并保存', persistenceError);
          activeRun = null;
          activeTrace = null;
          setBusy(false);
          loadSessions();
        } else {
          appendTraceStep(activeTrace, event);
          const label = traceLabels[kind] || kind;
          setStatus(`${label}${event.tool_name ? ` · ${event.tool_name}` : ''}`);
        }
      });
    });
    source.onerror = () => {
      if (!activeRun) return;
      source.close();
      activeSource = null;
      recoverRunStatus();
    };
  }

  async function recoverRunStatus() {
    if (!activeRun) return;
    try {
      const response = await fetch(activeRun.status_url, { cache: 'no-store' });
      const body = await safeJson(response);
      if (response.ok && !body.running && body.result) {
        renderReport(body.result);
        setStatus(body.persistence_error ? '任务结束，但会话落盘失败' : '任务结束 · 已从状态接口恢复结果', body.persistence_error);
        activeRun = null;
        activeTrace = null;
        setBusy(false);
        await loadSessions();
        return;
      }
      if (response.ok && body.running) {
        setStatus('流已断开，任务仍在运行；正在重新连接…');
        setTimeout(() => activeRun && connectStream(activeRun.stream_url), 600);
        return;
      }
      throw new Error(body?.error?.message || '无法恢复任务状态');
    } catch (error) {
      setStatus(error.message || '任务状态恢复失败', true);
      activeRun = null;
      activeTrace = null;
      setBusy(false);
    }
  }

  async function cancelRun() {
    if (!activeRun) return;
    els.cancel.disabled = true;
    try {
      const response = await fetch(activeRun.cancel_url, { method: 'POST' });
      const body = await safeJson(response);
      if (!response.ok) throw new Error(body?.error?.message || '取消失败');
      setStatus(body.already_terminal ? '任务已经结束' : '已请求取消，等待安全终止…');
    } catch (error) {
      setStatus(error.message || '取消失败', true);
    } finally {
      els.cancel.disabled = false;
    }
  }

  async function copySummary() {
    if (!latestResult) return;
    const report = latestResult.report;
    const lines = [
      '# 苍云战斗分析 Agent 实验摘要',
      `- status: ${latestResult.status}`,
      `- provider/model: ${latestResult.provider_profile} / ${latestResult.model}`,
      `- scenario: ${latestResult.scenario_hash}`,
      `- prompt: ${latestResult.prompt_version} / ${latestResult.prompt_sha256}`,
      `- tools/simulations: ${latestResult.accounting?.tool_calls || 0} / ${latestResult.accounting?.simulations || 0}`,
      `- evidence: ${(report?.evidence_ids || []).join(', ') || 'none'}`,
      '',
      report?.content?.summary || latestResult.error?.message || 'No report',
    ];
    try {
      await navigator.clipboard.writeText(lines.join('\n'));
      setStatus('已复制可复现实验摘要');
    } catch (_) {
      setStatus('浏览器未允许写入剪贴板', true);
    }
  }

  function newSession() {
    if (activeRun) return;
    currentSessionId = null;
    latestResult = null;
    els.copy.disabled = true;
    renderWelcome();
    setStatus('新会话 · 下一次运行时创建持久记录');
    loadSessions();
  }

  async function initialize() {
    if (initialized) return;
    initialized = true;
    updateScenarioState();
    await Promise.all([loadProviders(), loadSessions()]);
  }

  els.provider.addEventListener('change', syncProviderModel);
  els.run.addEventListener('click', startRun);
  els.cancel.addEventListener('click', cancelRun);
  els.copy.addEventListener('click', copySummary);
  els.newSession.addEventListener('click', newSession);
  els.goSim.addEventListener('click', () => window.Jx3Nav?.switchPage('page-sim'));
  els.question.addEventListener('keydown', event => {
    if ((event.ctrlKey || event.metaKey) && event.key === 'Enter') {
      event.preventDefault();
      startRun();
    }
  });
  document.querySelectorAll('[data-agent-question]').forEach(button => {
    button.addEventListener('click', () => { els.question.value = button.dataset.agentQuestion; els.question.focus(); });
  });
  window.addEventListener('jx3-sim-complete', updateScenarioState);
  new MutationObserver(() => {
    if (page.classList.contains('active')) {
      initialize();
      updateScenarioState();
      loadSessions();
    }
  }).observe(page, { attributes: true, attributeFilter: ['class'] });
  if (page.classList.contains('active')) initialize();
  if (window.location.hash === '#page-agent') {
    setTimeout(() => window.Jx3Nav?.switchPage('page-agent'), 0);
  }
})();
