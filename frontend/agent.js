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
    simPage: document.getElementById('page-sim'),
    dock: document.getElementById('sim_ai_dock'),
    dockFab: document.getElementById('sim_ai_fab'),
    dockClose: document.getElementById('sim_ai_close'),
    dockExpand: document.getElementById('sim_ai_expand'),
    dockNew: document.getElementById('sim_ai_new'),
    dockChat: document.getElementById('sim_ai_chat'),
    dockQuestion: document.getElementById('sim_ai_question'),
    dockProvider: document.getElementById('sim_ai_provider'),
    dockProviderModel: document.getElementById('sim_ai_provider_model'),
    dockSend: document.getElementById('sim_ai_send'),
    dockStop: document.getElementById('sim_ai_stop'),
    dockStatus: document.getElementById('sim_ai_status'),
    dockContext: document.getElementById('sim_ai_context'),
    dockContextText: document.getElementById('sim_ai_context_text'),
    dockSession: document.getElementById('sim_ai_session_label'),
    dockCopy: document.getElementById('sim_ai_copy'),
  };

  let initialized = false;
  let currentSessionId = null;
  let activeRun = null;
  let activeSource = null;
  let activeTrace = null;
  let latestResult = null;
  let activeSurface = 'full';
  let dockTrace = null;
  let dockLatestResult = null;
  let activeThinking = null;
  let dockThinking = null;

  const traceLabels = {
    planning: '拆解问题',
    tool_started: '调用工具',
    tool_finished: '取得证据',
    validating: '校验证据',
    report_repair_requested: '修复报告',
    report_citations_normalized: '补全证据引用',
    report_claims_sanitized: '保留可信结论',
    completed: '分析完成',
    partially_verified: '部分通过',
    refused: '安全拒绝',
    cancelled: '任务取消',
    evidence_insufficient: '证据校验未通过',
    budget_exhausted: '预算耗尽',
    provider_failed: 'Provider 故障',
    protocol_failed: '协议故障',
    timed_out: '任务超时',
    cancel_requested: '请求取消',
  };

  const statusLabels = {
    created: '已创建', running: '运行中', completed: '已完成', partially_verified: '部分通过', refused: '已拒绝',
    cancelled: '已取消', interrupted: '已中断', evidence_insufficient: '未形成可靠结论',
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
    if (els.dockStatus && activeSurface === 'dock') {
      els.dockStatus.textContent = text;
      els.dockStatus.classList.toggle('sim-ai-error', !!isError);
    }
  }

  function setBusy(busy) {
    els.run.disabled = busy;
    els.run.hidden = busy;
    els.cancel.hidden = !busy;
    els.provider.disabled = busy;
    if (els.dockSend) {
      els.dock.classList.toggle('is-loading', busy && activeSurface === 'dock');
      els.dock.setAttribute('aria-busy', String(busy && activeSurface === 'dock'));
      els.dockSend.disabled = busy;
      els.dockSend.hidden = busy;
      els.dockStop.hidden = !busy;
      els.dockProvider.disabled = busy;
      els.dockQuestion.disabled = busy;
      els.dockExpand.disabled = busy;
      els.dockNew.disabled = busy;
    }
    if (!busy) clearThinking();
  }

  function createThinking(container) {
    const wrap = element('div', 'ai-thinking');
    wrap.setAttribute('role', 'status');
    wrap.setAttribute('aria-live', 'polite');
    const mark = element('span', 'ai-thinking-mark', '✦');
    mark.setAttribute('aria-hidden', 'true');
    const copy = element('div', 'ai-thinking-copy');
    copy.appendChild(element('b', '', 'AI 正在思考'));
    const message = element('span', '', '正在理解问题与当前循环…');
    copy.appendChild(message);
    const dots = element('span', 'ai-thinking-dots');
    dots.setAttribute('aria-hidden', 'true');
    for (let index = 0; index < 3; index += 1) dots.appendChild(element('i'));
    wrap.append(mark, copy, dots);
    wrap._message = message;
    container.appendChild(wrap);
    return wrap;
  }

  function thinkingText(event) {
    const kind = event?.trace_kind || event?.kind;
    if (kind === 'planning') return '正在拆解问题并选择验证路径…';
    if (kind === 'tool_started') return `正在调用 ${event.tool_name || '模拟器'}…`;
    if (kind === 'tool_finished') return '已取得工具证据，正在继续分析…';
    if (kind === 'validating') return '正在校验数值、单位与证据引用…';
    if (kind === 'report_repair_requested') return '报告引用未通过，正在尝试修复…';
    if (kind === 'report_claims_sanitized') return '正在隐藏未验证内容并保留可信结论…';
    if (kind === 'report_citations_normalized') return '已补全可验证引用，正在完成校验…';
    if (kind === 'cancel_requested') return '正在安全停止当前任务…';
    return '正在分析当前循环…';
  }

  function updateThinking(node, event) {
    if (node?._message) node._message.textContent = thinkingText(event);
  }

  function clearThinking() {
    activeThinking?.remove();
    dockThinking?.remove();
    activeThinking = null;
    dockThinking = null;
  }

  function setTerminalStatus(result, persistenceError, recovered) {
    if (persistenceError) {
      setStatus('分析结束，但会话保存失败', true);
      return;
    }
    const status = result?.status;
    if (status === 'completed') {
      setStatus(recovered ? '分析完成 · 已恢复验证结论' : '分析完成 · 结论已绑定证据并保存');
    } else if (status === 'partially_verified') {
      setStatus(recovered ? '分析完成 · 已恢复部分验证结论' : '分析完成 · 已保留通过逐项校验的结论');
    } else if (status === 'evidence_insufficient') {
      setStatus('分析结束 · 未验证内容已被证据校验器拦截');
    } else if (status === 'refused') {
      setStatus('分析结束 · 请求超出只读分析边界');
    } else {
      const label = statusLabels[status] || status || '未知状态';
      setStatus(`分析结束 · ${label}`, ['provider_failed', 'protocol_failed', 'timed_out'].includes(status));
    }
  }

  function updateScenarioState() {
    const scenario = window._lastSimBody;
    if (!scenario) {
      els.scenario.classList.remove('ready');
      els.scenario.lastChild.textContent = ' 尚未捕获循环场景';
      if (els.dockContext) {
        els.dockContext.classList.remove('ready');
        els.dockContextText.textContent = '等待当前循环完成一次模拟';
      }
      return;
    }
    const mode = scenario.macro_text ? '宏循环' : `${(scenario.sequence || []).length} 个技能`;
    const delay = Number(scenario.network_delay || 0);
    els.scenario.classList.add('ready');
    els.scenario.lastChild.textContent = ` 已捕获当前场景 · ${mode} · ${delay}ms 延迟`;
    if (els.dockContext) {
      els.dockContext.classList.add('ready');
      els.dockContextText.textContent = `当前场景已就绪 · ${mode} · ${delay}ms 延迟`;
    }
  }

  async function safeJson(response) {
    try { return await response.json(); } catch (_) { return null; }
  }

  async function loadProviders() {
    try {
      const response = await fetch('/api/agent/providers', { cache: 'no-store' });
      if (!response.ok) throw new Error('Provider 列表不可用');
      const body = await response.json();
      const profiles = Array.isArray(body.profiles) ? body.profiles : [];
      let preferred = '';
      try { preferred = localStorage.getItem('agent_provider_profile') || ''; } catch (_) {}
      const available = profiles.find(profile => profile.id === preferred && profile.available)
        || profiles.find(profile => profile.available && profile.id !== 'offline')
        || profiles.find(profile => profile.available);
      [els.provider, els.dockProvider].filter(Boolean).forEach(select => {
        clear(select);
        profiles.forEach(profile => {
          const option = document.createElement('option');
          option.value = profile.id;
          option.textContent = `${profile.label}${profile.available ? '' : '（不可用）'}`;
          option.disabled = !profile.available;
          option.dataset.model = profile.model;
          select.appendChild(option);
        });
        if (available) select.value = available.id;
      });
      syncProviderModel();
    } catch (error) {
      setStatus(error.message || 'Provider 列表加载失败', true);
    }
  }

  function syncProviderModel() {
    const option = els.provider.selectedOptions[0];
    els.providerModel.textContent = option?.dataset.model || '—';
    if (els.dockProvider) {
      const dockOption = els.dockProvider.selectedOptions[0];
      els.dockProviderModel.textContent = dockOption?.dataset.model || '—';
    }
  }

  function selectProvider(source) {
    const value = source.value;
    [els.provider, els.dockProvider].filter(select => select && select !== source).forEach(select => {
      if ([...select.options].some(option => option.value === value && !option.disabled)) select.value = value;
    });
    try { localStorage.setItem('agent_provider_profile', value); } catch (_) {}
    syncProviderModel();
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
    const evidenceInsufficient = result.status === 'evidence_insufficient';
    const partiallyVerified = result.status === 'partially_verified';
    card.classList.toggle('is-limited', evidenceInsufficient || partiallyVerified);
    const head = element('div', 'agent-report-head');
    head.appendChild(element('b', '', evidenceInsufficient
      ? '本轮未形成可靠结论'
      : partiallyVerified ? '部分验证分析报告' : '已验证分析报告'));
    head.appendChild(element('span', '', `${report.provider_profile} / ${report.model} · ${result.accounting?.duration_ms || 0}ms`));
    card.appendChild(head);
    if (evidenceInsufficient) {
      const notice = element('div', 'agent-result-notice');
      notice.appendChild(element('b', '', '这不是系统故障'));
      notice.appendChild(element('span', '', '模型输出中的数值或引用未通过模拟器证据校验，未验证内容已被拦截。'));
      if (result.error?.code) notice.title = `校验码：${result.error.code}`;
      card.appendChild(notice);
    } else if (partiallyVerified) {
      const notice = element('div', 'agent-result-notice');
      notice.appendChild(element('b', '', '逐条校验后保留'));
      notice.appendChild(element('span', '', '个别模型表述或指标未通过证据校验，已单独隐藏；下方内容仍可继续追问。'));
      if (result.error?.code) notice.title = `首个校验码：${result.error.code}`;
      card.appendChild(notice);
    }
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

  function clearDockChat() {
    clear(els.dockChat);
    const welcome = element('div', 'sim-ai-welcome');
    welcome.appendChild(element('span', '', 'AI 只读当前循环并调用确定性模拟器。'));
    welcome.appendChild(element('small', '', '不会修改技能、宏、配装或其他游戏数据。'));
    els.dockChat.appendChild(welcome);
    dockTrace = null;
    dockLatestResult = null;
    els.dockCopy.disabled = true;
  }

  function prepareDockChat() {
    const welcome = els.dockChat.querySelector('.sim-ai-welcome');
    if (welcome) welcome.remove();
  }

  function scrollDock() {
    requestAnimationFrame(() => { els.dockChat.scrollTop = els.dockChat.scrollHeight; });
  }

  function appendDockBubble(role, text, isError) {
    prepareDockChat();
    const wrap = element('article', `sim-ai-bubble ${role}${isError ? ' sim-ai-error' : ''}`);
    wrap.appendChild(element('div', 'sim-ai-bubble-label', role === 'user' ? '你' : 'AI 分析'));
    wrap.appendChild(element('div', 'sim-ai-bubble-body', text));
    els.dockChat.appendChild(wrap);
    scrollDock();
  }

  function createDockTrace(runId) {
    prepareDockChat();
    const wrap = element('div', 'sim-ai-progress');
    const head = element('div', 'sim-ai-progress-head');
    head.appendChild(element('span', '', '可验证分析流程'));
    head.appendChild(element('span', '', runId || '准备中'));
    const steps = element('div', 'sim-ai-progress-steps');
    wrap.append(head, steps);
    els.dockChat.appendChild(wrap);
    scrollDock();
    return { wrap, steps };
  }

  function appendDockTraceStep(trace, event) {
    if (!trace) return;
    const kind = event.trace_kind || event.kind;
    const label = traceLabels[kind] || kind;
    const suffix = event.tool_name ? ` · ${event.tool_name}` : '';
    trace.steps.appendChild(element('span', 'sim-ai-progress-step', `${label}${suffix}`));
    scrollDock();
  }

  function renderDockReport(result) {
    if (!result) return;
    prepareDockChat();
    dockLatestResult = result;
    latestResult = result;
    els.dockCopy.disabled = false;
    const report = result.report;
    const card = element('article', 'sim-ai-result');
    const evidenceInsufficient = result.status === 'evidence_insufficient';
    const partiallyVerified = result.status === 'partially_verified';
    card.classList.toggle('is-limited', evidenceInsufficient || partiallyVerified);
    const head = element('div', 'sim-ai-result-head');
    head.appendChild(element('b', '', statusLabels[result.status] || result.status || '分析结果'));
    head.appendChild(element('span', '', `${result.provider_profile || '—'} · ${result.accounting?.duration_ms || 0}ms`));
    card.appendChild(head);

    if (evidenceInsufficient) {
      const notice = element('div', 'sim-ai-result-notice');
      notice.appendChild(element('b', '', '这不是系统故障'));
      notice.appendChild(element('span', '', '模型输出中的数值或引用未通过证据校验，因此没有发布为结论。'));
      if (result.error?.code) notice.title = `校验码：${result.error.code}`;
      card.appendChild(notice);
    } else if (partiallyVerified) {
      const notice = element('div', 'sim-ai-result-notice');
      notice.appendChild(element('b', '', '部分通过'));
      notice.appendChild(element('span', '', '未验证的单项内容已隐藏，其余证据结论仍然有效并可继续追问。'));
      if (result.error?.code) notice.title = `首个校验码：${result.error.code}`;
      card.appendChild(notice);
    }

    const summary = report?.content?.summary || result.error?.message || '本次任务没有生成可展示的结论。';
    card.appendChild(element('div', 'sim-ai-result-summary', summary));
    (report?.content?.findings || []).slice(0, 4).forEach(finding => {
      const block = element('div', 'sim-ai-result-finding');
      block.appendChild(element('b', '', finding.title));
      block.appendChild(element('p', '', finding.explanation));
      card.appendChild(block);
    });

    const metrics = (report?.content?.findings || [])
      .flatMap(finding => finding.metrics || [])
      .slice(0, 6);
    if (metrics.length) {
      const grid = element('div', 'sim-ai-result-metrics');
      metrics.forEach(metric => {
        const item = element('div', 'sim-ai-result-metric');
        item.appendChild(element('b', '', metricValue(metric)));
        item.appendChild(element('span', '', `${metric.label} · ${metric.unit}`));
        item.title = `${metric.evidence_id}${metric.json_pointer}`;
        grid.appendChild(item);
      });
      card.appendChild(grid);
    }
    const limitations = report?.content?.limitations || [];
    if (limitations.length) {
      const block = element('div', 'sim-ai-result-finding');
      block.appendChild(element('b', '', '边界与限制'));
      block.appendChild(element('p', '', limitations.slice(0, 3).join('；')));
      card.appendChild(block);
    }
    els.dockChat.appendChild(card);
    scrollDock();
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
      const button = element('button', 'sim-btn', label);
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
      if (els.dockSession) els.dockSession.textContent = `续接 · ${sessionId.slice(0, 18)}…`;
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

  function setDockOpen(open, focusInput) {
    if (!els.dock) return;
    els.dock.classList.toggle('open', open);
    els.dock.setAttribute('aria-hidden', String(!open));
    els.dockFab.setAttribute('aria-expanded', String(open));
    try { localStorage.setItem('sim_ai_dock_open', open ? '1' : '0'); } catch (_) {}
    if (open) {
      updateScenarioState();
      if (focusInput) setTimeout(() => els.dockQuestion.focus(), 80);
    } else {
      els.dockFab.focus({ preventScroll: true });
    }
  }

  async function startDockRun() {
    if (activeRun) return;
    const question = els.dockQuestion.value.trim();
    if (!question) {
      activeSurface = 'dock';
      setStatus('请先输入一个策划问题', true);
      els.dockQuestion.focus();
      return;
    }
    activeSurface = 'dock';
    setBusy(true);
    setStatus('正在冻结当前循环…');
    try {
      const simulation = await captureScenario();
      appendDockBubble('user', question);
      dockTrace = createDockTrace('准备创建 run');
      dockThinking = createThinking(els.dockChat);
      const payload = {
        question,
        provider_profile: els.dockProvider.value,
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
      els.dockSession.textContent = `会话 · ${body.session_id.slice(0, 18)}…`;
      dockTrace.wrap.querySelector('.sim-ai-progress-head span:last-child').textContent = body.run_id;
      els.dockQuestion.value = '';
      setStatus(`运行中 · 场景 ${body.scenario_hash.slice(0, 12)}…`);
      connectDockStream(body.stream_url);
      await loadSessions();
    } catch (error) {
      setBusy(false);
      activeRun = null;
      appendDockBubble('agent', error.message || 'Agent 运行失败', true);
      setStatus(error.message || 'Agent 运行失败', true);
    }
  }

  function connectDockStream(url) {
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
          clearThinking();
          renderDockReport(event.result);
          const persistenceError = !!event.persistence_error;
          setTerminalStatus(event.result, persistenceError, false);
          activeRun = null;
          dockTrace = null;
          setBusy(false);
          loadSessions();
        } else {
          appendDockTraceStep(dockTrace, event);
          updateThinking(dockThinking, event);
          const label = traceLabels[kind] || kind;
          setStatus(`${label}${event.tool_name ? ` · ${event.tool_name}` : ''}`);
        }
      });
    });
    source.onerror = () => {
      if (!activeRun) return;
      source.close();
      activeSource = null;
      recoverDockRunStatus();
    };
  }

  async function recoverDockRunStatus() {
    if (!activeRun) return;
    try {
      const response = await fetch(activeRun.status_url, { cache: 'no-store' });
      const body = await safeJson(response);
      if (response.ok && !body.running && body.result) {
        clearThinking();
        renderDockReport(body.result);
        setTerminalStatus(body.result, !!body.persistence_error, true);
        activeRun = null;
        dockTrace = null;
        setBusy(false);
        await loadSessions();
        return;
      }
      if (response.ok && body.running) {
        setStatus('连接中断，任务仍在运行；正在恢复…');
        updateThinking(dockThinking, { kind: 'planning' });
        setTimeout(() => activeRun && connectDockStream(activeRun.stream_url), 600);
        return;
      }
      throw new Error(body?.error?.message || '无法恢复任务状态');
    } catch (error) {
      appendDockBubble('agent', error.message || '任务状态恢复失败', true);
      setStatus(error.message || '任务状态恢复失败', true);
      activeRun = null;
      dockTrace = null;
      setBusy(false);
    }
  }

  async function startRun() {
    if (activeRun) return;
    activeSurface = 'full';
    const question = els.question.value.trim();
    if (!question) { setStatus('请先输入一个策划问题', true); els.question.focus(); return; }
    setBusy(true);
    setStatus('正在冻结当前场景…');
    try {
      const simulation = await captureScenario();
      appendMessage('user', question);
      activeTrace = createTrace('准备创建 run');
      activeThinking = createThinking(els.transcript);
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
          clearThinking();
          renderReport(event.result);
          const persistenceError = !!event.persistence_error;
          setTerminalStatus(event.result, persistenceError, false);
          activeRun = null;
          activeTrace = null;
          setBusy(false);
          loadSessions();
        } else {
          appendTraceStep(activeTrace, event);
          updateThinking(activeThinking, event);
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
        clearThinking();
        renderReport(body.result);
        setTerminalStatus(body.result, !!body.persistence_error, true);
        activeRun = null;
        activeTrace = null;
        setBusy(false);
        await loadSessions();
        return;
      }
      if (response.ok && body.running) {
        setStatus('流已断开，任务仍在运行；正在重新连接…');
        updateThinking(activeThinking, { kind: 'planning' });
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
    if (els.dockStop) els.dockStop.disabled = true;
    try {
      const response = await fetch(activeRun.cancel_url, { method: 'POST' });
      const body = await safeJson(response);
      if (!response.ok) throw new Error(body?.error?.message || '取消失败');
      setStatus(body.already_terminal ? '任务已经结束' : '已请求取消，等待安全终止…');
    } catch (error) {
      setStatus(error.message || '取消失败', true);
    } finally {
      els.cancel.disabled = false;
      if (els.dockStop) els.dockStop.disabled = false;
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
    dockLatestResult = null;
    els.copy.disabled = true;
    if (els.dockCopy) els.dockCopy.disabled = true;
    if (els.dockSession) els.dockSession.textContent = '新对话';
    renderWelcome();
    if (els.dockChat) clearDockChat();
    setStatus('新会话 · 下一次运行时创建持久记录');
    loadSessions();
  }

  async function initialize() {
    if (initialized) return;
    initialized = true;
    updateScenarioState();
    await Promise.all([loadProviders(), loadSessions()]);
  }

  els.provider.addEventListener('change', () => selectProvider(els.provider));
  els.dockProvider?.addEventListener('change', () => selectProvider(els.dockProvider));
  els.run.addEventListener('click', startRun);
  els.cancel.addEventListener('click', cancelRun);
  els.copy.addEventListener('click', copySummary);
  els.newSession.addEventListener('click', newSession);
  els.goSim.addEventListener('click', () => window.Jx3Nav?.switchPage('page-sim'));
  els.dockFab?.addEventListener('click', () => setDockOpen(true, true));
  els.dockClose?.addEventListener('click', () => setDockOpen(false));
  els.dockSend?.addEventListener('click', startDockRun);
  els.dockStop?.addEventListener('click', cancelRun);
  els.dockNew?.addEventListener('click', () => {
    activeSurface = 'dock';
    newSession();
    els.dockQuestion.focus();
  });
  els.dockExpand?.addEventListener('click', () => {
    const sessionToOpen = currentSessionId;
    setDockOpen(false);
    window.Jx3Nav?.switchPage('page-agent');
    initialize().then(() => {
      if (sessionToOpen && !activeRun) openSession(sessionToOpen);
    });
  });
  els.dockCopy?.addEventListener('click', () => {
    if (!dockLatestResult) return;
    activeSurface = 'dock';
    latestResult = dockLatestResult;
    copySummary();
  });
  els.dockQuestion?.addEventListener('keydown', event => {
    if (event.key === 'Enter' && !event.shiftKey && !event.isComposing) {
      event.preventDefault();
      startDockRun();
    }
  });
  document.querySelectorAll('[data-sim-ai-question]').forEach(button => {
    button.addEventListener('click', () => {
      setDockOpen(true);
      els.dockQuestion.value = button.dataset.simAiQuestion;
      els.dockQuestion.focus();
    });
  });
  document.addEventListener('keydown', event => {
    if (event.key === 'Escape' && els.dock?.classList.contains('open') && !activeRun) {
      setDockOpen(false);
    }
  });
  els.question.addEventListener('keydown', event => {
    if (event.key === 'Enter' && !event.shiftKey && !event.isComposing) {
      event.preventDefault();
      startRun();
    }
  });
  document.querySelectorAll('[data-agent-question]').forEach(button => {
    button.addEventListener('click', () => { els.question.value = button.dataset.agentQuestion; els.question.focus(); });
  });
  window.addEventListener('jx3-sim-complete', updateScenarioState);
  function syncAgentPageLayout() {
    document.body.classList.toggle('agent-fullheight', page.classList.contains('active'));
  }
  new MutationObserver(() => {
    syncAgentPageLayout();
    if (page.classList.contains('active')) {
      initialize();
      updateScenarioState();
      loadSessions();
    }
  }).observe(page, { attributes: true, attributeFilter: ['class'] });
  syncAgentPageLayout();
  initialize();
  try {
    if (localStorage.getItem('sim_ai_dock_open') === '1' && els.simPage?.classList.contains('active')) {
      setDockOpen(true);
    }
  } catch (_) {}
  if (window.location.hash === '#page-agent') {
    setTimeout(() => window.Jx3Nav?.switchPage('page-agent'), 0);
  } else if (window.location.hash === '#page-sim' || window.location.hash === '#page-sim-ai') {
    setTimeout(() => {
      window.Jx3Nav?.switchPage('page-sim');
      if (window.location.hash === '#page-sim-ai') setDockOpen(true);
    }, 0);
  }
})();
