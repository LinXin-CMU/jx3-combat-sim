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
    newSession: document.getElementById('agent_new_session'),
    goSim: document.getElementById('agent_go_sim'),
    simPage: document.getElementById('page-sim'),
    equipPage: document.getElementById('page-equip'),
    dock: document.getElementById('sim_ai_dock'),
    dockFab: document.getElementById('sim_ai_fab'),
    dockClose: document.getElementById('sim_ai_close'),
    dockHistory: document.getElementById('sim_ai_history'),
    dockHistoryPanel: document.getElementById('sim_ai_history_panel'),
    dockHistoryList: document.getElementById('sim_ai_history_list'),
    dockHistoryCount: document.getElementById('sim_ai_history_count'),
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
    dockTitle: document.getElementById('sim_ai_title'),
    dockQuick: document.getElementById('sim_ai_quick'),
    dockFabContext: document.getElementById('sim_ai_fab_context'),
  };

  // The same assistant serves both workspaces. Detach it from page-sim so a hidden
  // page cannot hide the fixed dock when the equipment configurator is active.
  if (els.dock) document.body.appendChild(els.dock);
  if (els.dockFab) document.body.appendChild(els.dockFab);

  let initialized = false;
  let currentSessionId = null;
  let activeRun = null;
  let activeSource = null;
  let activeTrace = null;
  let activeSurface = 'full';
  let dockTrace = null;
  let activeThinking = null;
  let dockThinking = null;
  let activeFeedbackTurn = null;
  let dockFeedbackTurn = null;
  let sessionSummaries = [];
  let dockMode = 'simulation';
  let composerBusy = false;
  let clarificationSerial = 0;
  const latestRunBySession = new Map();

  const traceLabels = {
    planning: '拆解问题',
    analysis_context_prepared: '准备分析上下文',
    analysis_plan_selected: '准备分析上下文',
    evidence_coverage_checked: '检查证据覆盖',
    reasoning_state_updated: '更新问题推导状态',
    reasoning_critique_started: '执行发布前批判检查',
    reasoning_critique_failed: '批判检查要求修订',
    reasoning_critique_passed: '批判检查通过',
    report_validation_started: '校验报告证据',
    report_validation_passed: '证据校验通过',
    model_context_compacted: '压缩模型上下文',
    model_context_handoff: '重建紧凑上下文',
    model_context_limit_evidence_preserved: '上下文已安全截停',
    provider_failure_evidence_preserved: '保留失败前证据',
    model_started: '模型处理中',
    model_finished: '模型响应完成',
    decision_checkpoint: '记录决策依据',
    evidence_gap_requires_tool: '补齐必要证据',
    tool_started: '调用工具',
    tool_finished: '取得证据',
    validating: '校验证据',
    report_repair_requested: '修复报告',
    report_citations_normalized: '补全证据引用',
    report_claims_sanitized: '保留可信结论',
    provider_empty_retry: '重试生成报告',
    provider_empty_evidence_preserved: '保留已有证据',
    knowledge_searches_coalesced: '合并冗余检索',
    knowledge_only_client_scope: '限定为知识问答',
    completed: '分析完成',
    partially_verified: '部分通过',
    needs_user_input: '等待你的回答',
    refused: '安全拒绝',
    cancelled: '任务取消',
    evidence_insufficient: '证据校验未通过',
    budget_exhausted: '预算耗尽',
    budget_limit_reached: '预算边界已收束',
    provider_failed: 'Provider 故障',
    protocol_failed: '协议故障',
    timed_out: '任务超时',
    cancel_requested: '请求取消',
  };

  const statusLabels = {
    created: '已创建', running: '运行中', completed: '已完成', partially_verified: '部分通过', needs_user_input: '等待回答', refused: '已拒绝',
    cancelled: '已取消', interrupted: '已中断', evidence_insufficient: '未形成可靠结论',
    budget_exhausted: '预算耗尽', provider_failed: 'Provider 故障',
    protocol_failed: '协议故障', timed_out: '超时', finished: '已结束',
  };

  const toolLabels = {
    distill_macro: '蒸馏宏 · 生成规则初稿',
    get_current_scenario: '读取当前场景',
    ask_user_question: '向你确认关键信息',
    search_knowledge_base: '检索版本知识库',
    simulate_scenario: '运行基线模拟',
    compare_scenarios: '对比候选方案',
    analyze_timeline: '分析战斗时间轴',
    inspect_timeline_events: '定位实际技能事件',
    list_saved_artifacts: '查找已保存资料',
    read_saved_artifact: '读取已保存资料',
    compare_saved_macros: '对比已保存宏',
    compare_saved_scenarios: '对比已保存方案',
    inspect_equipment_workspace: '读取当前配装',
    compare_focused_equipment: '实测换装方案',
    search_equipment_catalog: '检索装备库',
    compare_equipment_strategies: '实测四件套与四切糕',
  };

  const versionLabels = {
    current_exact: '当前版本',
    test_server_exact: '当前体服',
    historical_explicit: '指定历史版本',
    cross_version: '跨版本资料',
    reference_only: '人物 / 来源资料',
  };

  const providerErrorLabels = {
    provider_balance_insufficient: '模型供应商账户余额不足；充值后重试，或切换“离线测试”继续演示。',
    provider_http_401: '模型供应商鉴权失败，请检查服务端 API Key。',
    provider_http_403: '模型供应商拒绝访问，请检查账户权限。',
    provider_http_429: '模型供应商请求繁忙，请稍后重试。',
    provider_http_5xx: '模型供应商服务暂时异常，请稍后重试。',
    provider_http_error: '模型供应商返回了非成功状态；请展开失败调试信息查看阶段和诊断码。',
    provider_timeout: '模型供应商响应超时，请稍后重试。',
    provider_network_error: '无法连接模型供应商，请检查网络或代理设置。',
  };

  function providerErrorText(error) {
    return providerErrorLabels[error?.code] || error?.message || '模型供应商调用失败。';
  }

  function toolLabel(name) {
    return toolLabels[name] || name || '只读工具';
  }

  function clear(node) {
    while (node.firstChild) node.removeChild(node.firstChild);
  }

  function element(tag, className, text) {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text != null) node.textContent = text;
    return node;
  }

  function setPrompt(input, question) {
    input.value = question || '';
    input.focus();
  }

  function setStatus(text, isError) {
    els.status.textContent = text;
    els.status.classList.toggle('agent-error', !!isError);
    if (els.dockStatus && activeSurface === 'dock') {
      els.dockStatus.textContent = text;
      els.dockStatus.classList.toggle('sim-ai-error', !!isError);
    }
  }

  async function copyCardText(button, text) {
    if (!text) return;
    const original = button.textContent;
    try {
      await navigator.clipboard.writeText(text);
      button.textContent = '已复制';
      button.classList.add('is-copied');
    } catch (_) {
      button.textContent = '复制失败';
      button.classList.add('is-error');
    }
    window.setTimeout(() => {
      button.textContent = original;
      button.classList.remove('is-copied', 'is-error');
    }, 1200);
  }

  function cardCopyButton(label, getText, compact) {
    const button = element('button', `agent-card-copy${compact ? ' is-compact' : ''}`, label);
    button.type = 'button';
    button.title = label;
    button.addEventListener('click', event => {
      event.stopPropagation();
      copyCardText(button, getText());
    });
    return button;
  }

  function createFeedbackTurn(question, compact) {
    return {
      question: String(question || '').trim(),
      compact: !!compact,
      traceWrap: null,
      result: null,
      button: null,
    };
  }

  function feedbackCopyButton(turn) {
    const button = cardCopyButton('复制反馈', () => buildFeedbackBundle(turn), turn.compact);
    button.classList.add('agent-feedback-copy');
    button.disabled = true;
    button.title = '任务完成后复制问题、阶段概述、回复与脱敏调试信息';
    turn.button = button;
    return button;
  }

  function attachFeedbackTrace(turn, trace) {
    if (turn) turn.traceWrap = trace?.wrap || null;
  }

  function completeFeedbackTurn(turn, result) {
    if (!turn) return;
    turn.result = result || null;
    if (!turn.button) return;
    turn.button.disabled = !turn.result;
    turn.button.title = turn.result
      ? '复制本轮问题、阶段概述、完整回复与脱敏调试信息'
      : '本轮尚未形成可复制结果';
  }

  function traceCardText(wrap, compact) {
    const runId = wrap.querySelector(compact ? '.sim-ai-progress-run-id' : '.agent-trace-run-id')?.textContent || '—';
    const stepSelector = compact ? '.sim-ai-progress-step' : '.agent-trace-step';
    const overviewSelector = compact ? '.sim-ai-progress-overview' : '.agent-trace-overview';
    const lines = ['# 可验证分析流程 · 阶段概述', `- run: ${runId}`];
    wrap.querySelectorAll(stepSelector).forEach((step, index) => {
      const label = step.querySelector('b')?.textContent?.trim() || `阶段 ${index + 1}`;
      const overview = step.querySelector(overviewSelector)?.textContent?.trim();
      const meta = step.querySelector('small')?.textContent?.trim();
      lines.push('', `${index + 1}. ${label}`);
      if (overview) lines.push(`   ${overview}`);
      if (meta) lines.push(`   ${meta}`);
    });
    return lines.join('\n');
  }

  function setBusy(busy) {
    composerBusy = busy;
    updateClarificationControls();
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
      if (els.dockHistory) els.dockHistory.disabled = busy;
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
    if (event?.overview && ['planning', 'analysis_context_prepared', 'analysis_plan_selected', 'evidence_coverage_checked',
      'reasoning_state_updated', 'reasoning_critique_started',
      'tool_started', 'model_started', 'decision_checkpoint'].includes(kind)) return event.overview;
    if (kind === 'planning') return '正在拆解问题并选择验证路径…';
    if (kind === 'model_started' && event?.code === 'report_repair') return '模型正在依据校验反馈修复报告…';
    if (kind === 'model_started' && event?.code === 'final_report') return '模型正在依据已有证据生成结论…';
    if (kind === 'model_started') return '模型正在规划下一项可验证动作…';
    if (kind === 'model_finished') return '模型响应已返回，正在解析下一阶段…';
    if (kind === 'decision_checkpoint') return event?.overview || '正在记录本轮可审计的决策依据…';
    if (kind === 'evidence_gap_requires_tool') {
      return event?.tool_name === 'analyze_timeline'
        ? '当前证据还不足以判断循环优缺点，正在补做基线诊断。'
        : '当前证据还不足以发布修改方案，正在补做同场景候选对照。';
    }
    if (kind === 'reasoning_state_updated') return '正在更新本题的证据检查点与下一步动作…';
    if (kind === 'reasoning_critique_started') return '正在检查结论是否真正回答问题并满足证据边界…';
    if (kind === 'reasoning_critique_failed') return '结论未通过任务完成度检查，正在依据已有证据修订…';
    if (kind === 'reasoning_critique_passed') return '语义与证据检查通过，正在发布结论…';
    if (kind === 'model_context_compacted') return '正在压缩旧轮次与工具输出，完整证据仍保留在后台…';
    if (kind === 'model_context_handoff') return '正在用最新证据状态重建紧凑上下文…';
    if (kind === 'tool_started') return `正在${toolLabel(event.tool_name)}…`;
    if (kind === 'tool_finished') return '已取得工具证据，正在继续分析…';
    if (kind === 'validating') return '正在校验数值、单位与证据引用…';
    if (kind === 'report_repair_requested') return '报告引用未通过，正在尝试修复…';
    if (kind === 'report_claims_sanitized') return '正在隐藏未验证内容并保留可信结论…';
    if (kind === 'report_citations_normalized') return '已补全可验证引用，正在完成校验…';
    if (kind === 'knowledge_searches_coalesced') return '已合并重复检索，正在依据现有结果收束结论…';
    if (kind === 'budget_limit_reached') return '实验预算已到边界，正在用已有证据生成受限结论…';
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
    } else if (status === 'needs_user_input') {
      setStatus('分析已暂停 · 回答上方问题后可在当前会话继续');
    } else if (status === 'evidence_insufficient') {
      setStatus('分析结束 · 未验证内容已被证据校验器拦截');
    } else if (status === 'refused') {
      setStatus('分析结束 · 请求超出只读分析边界');
    } else if (status === 'provider_failed') {
      setStatus(`分析结束 · ${providerErrorText(result?.error)}`, true);
    } else {
      const label = statusLabels[status] || status || '未知状态';
      setStatus(`分析结束 · ${label}`, ['provider_failed', 'protocol_failed', 'timed_out'].includes(status));
    }
  }

  function updateScenarioState() {
    if (dockMode === 'equipment') {
      const config = window.Jx3Equip?.getCurrentConfig?.();
      const count = Object.keys(config?.slots || {}).length;
      if (els.dockContext) {
        els.dockContext.classList.toggle('ready', count > 0);
        els.dockContextText.textContent = count > 0
          ? `当前配装已就绪 · ${count} 个部位 · 只读分析`
          : '请先在配装器选择装备与 DPS 来源';
      }
      return;
    }
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
      sessionSummaries = Array.isArray(body.sessions) ? body.sessions : [];
      renderSessionList(sessionSummaries);
      renderDockSessionList(sessionSummaries);
    } catch (error) {
      clear(els.sessions);
      els.sessions.appendChild(element('div', 'agent-empty agent-error', error.message || '会话读取失败'));
      if (els.dockHistoryList) {
        clear(els.dockHistoryList);
        els.dockHistoryList.appendChild(element('div', 'sim-ai-history-empty sim-ai-error', error.message || '会话读取失败'));
      }
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

  function renderDockSessionList(sessions) {
    if (!els.dockHistoryList) return;
    clear(els.dockHistoryList);
    if (els.dockHistoryCount) els.dockHistoryCount.textContent = `${sessions.length} 条`;
    if (!sessions.length) {
      els.dockHistoryList.appendChild(element('div', 'sim-ai-history-empty', '还没有分析会话。发送第一条问题后会自动保存。'));
      return;
    }
    sessions.forEach(session => {
      const button = element('button', 'sim-ai-session-item');
      button.type = 'button';
      button.dataset.sessionId = session.session_id;
      if (session.session_id === currentSessionId) button.classList.add('active');
      const copy = element('span', 'sim-ai-session-copy');
      copy.appendChild(element('span', 'sim-ai-session-title', session.title || '未命名会话'));
      copy.appendChild(element('span', 'sim-ai-session-meta', `${statusLabels[session.status] || session.status} · ${formatTime(session.updated_at_ms)}`));
      button.appendChild(copy);
      button.appendChild(element('span', 'sim-ai-session-open', '›'));
      if (session.corrupted_event_count) button.title = `检测到 ${session.corrupted_event_count} 个损坏事件；原文件未被覆盖`;
      button.addEventListener('click', () => openDockSession(session.session_id));
      els.dockHistoryList.appendChild(button);
    });
    const activeSummary = sessions.find(session => session.session_id === currentSessionId);
    if (activeSummary && els.dockSession) {
      els.dockSession.textContent = `续接 · ${activeSummary.title || activeSummary.session_id}`;
    }
  }

  function appendMessage(role, text, actions) {
    const wrap = element('article', `agent-message ${role}`);
    const head = element('div', 'agent-message-head');
    head.appendChild(element('div', 'agent-message-label', role === 'user' ? '策划问题' : 'Agent 结论'));
    (actions || []).forEach(action => head.appendChild(action));
    wrap.appendChild(head);
    wrap.appendChild(element('div', 'agent-message-body', text || '—'));
    els.transcript.appendChild(wrap);
    scrollTranscript();
    return wrap;
  }

  function createTrace(runId) {
    const wrap = element('section', 'agent-trace');
    const title = element('div', 'agent-trace-title');
    title.appendChild(element('span', '', '可验证执行轨迹 · 阶段概述'));
    const actions = element('span', 'agent-card-actions');
    actions.appendChild(element('span', 'agent-trace-run-id', runId || '—'));
    actions.appendChild(cardCopyButton('复制思维链', () => traceCardText(wrap, false), false));
    title.appendChild(actions);
    const note = element('p', 'agent-trace-note', '展示阶段目标、工具动作、证据产出与校验结果；不展示模型隐藏推理。');
    const steps = element('div', 'agent-trace-steps');
    wrap.appendChild(title);
    wrap.appendChild(note);
    wrap.appendChild(steps);
    els.transcript.appendChild(wrap);
    return { wrap, steps, seen: new Set(), openTools: new Map(), openModel: null, activeStep: null };
  }

  function traceStepMeta(event) {
    const kind = event?.trace_kind || event?.kind;
    const evidenceCount = Array.isArray(event?.evidence_ids) ? event.evidence_ids.length : 0;
    if (kind === 'analysis_context_prepared' || kind === 'analysis_plan_selected') return '由模型决定接下来的分析步骤';
    if (kind === 'evidence_coverage_checked') {
      return `证据覆盖 · ${{ sufficient: '充分', partial: '部分', insufficient: '不足' }[event?.code] || '检查完成'}`;
    }
    if (kind === 'reasoning_state_updated') return event?.code ? `当前检查点 · ${event.code}` : '检查点已更新';
    if (kind === 'reasoning_critique_passed') return '语义契约已满足';
    if (kind === 'model_started') {
      return {
        tool_selection: '规划下一步',
        final_report: '生成结论',
        report_repair: '修复报告',
      }[event?.code] || '';
    }
    if (event?.code && isTraceWarning(event)) return `诊断码 · ${event.code}`;
    if (evidenceCount) return `${evidenceCount} 份证据`;
    if (event?.code) return `状态码 · ${event.code}`;
    return '';
  }

  function isTraceWarning(event) {
    const kind = event?.trace_kind || event?.kind;
    return kind === 'tool_finished' && !!event?.code
      || ['report_repair_requested', 'report_claims_sanitized', 'provider_empty_retry',
        'reasoning_critique_failed',
        'model_context_limit_evidence_preserved',
        'provider_failure_evidence_preserved',
        'provider_empty_evidence_preserved', 'evidence_insufficient', 'budget_exhausted',
        'budget_limit_reached',
        'provider_failed', 'protocol_failed', 'timed_out', 'cancel_requested', 'cancelled']
        .includes(kind);
  }

  function finishActiveTraceStep(trace) {
    if (!trace?.activeStep) return;
    trace.activeStep.classList.remove('is-running');
    trace.activeStep.classList.add('is-done');
    trace.activeStep.removeAttribute('aria-current');
    trace.activeStep = null;
  }

  function traceStageOverview(event) {
    if (event?.overview) return event.overview;
    const kind = event?.trace_kind || event?.kind;
    const evidenceCount = Array.isArray(event?.evidence_ids) ? event.evidence_ids.length : 0;
    const toolStarted = {
      get_current_scenario: '读取服务端冻结的场景快照，锁定版本、心法、配置与输入口径。',
      search_knowledge_base: '按问题与版本约束检索本地资料，优先返回可溯源且版本匹配的内容。',
      simulate_scenario: '在冻结场景上运行确定性基线，战斗数值只由模拟器产生。',
      compare_scenarios: '仅改变声明过的候选参数，在同一场景口径下对比结果。',
      analyze_timeline: '聚合技能、资源、冷却与增益事件，定位可观察的时间轴现象。',
      inspect_timeline_events: '按技能、时间或异常信号读取准确释放位置及前后状态。',
      list_saved_artifacts: '只在当前账号的宏、循环、配装、属性与广场方案目录中按名称查找。',
      read_saved_artifact: '用目录返回的不透明标识精确读取一份资料，不接触路径或敏感设置。',
      compare_saved_macros: '固定当前战斗环境，只替换两份已保存宏并分别运行真实模拟。',
      compare_saved_scenarios: '分别还原两份完整存档，校验版本与心法后运行真实模拟对比。',
    };
    if (kind === 'tool_started') return toolStarted[event.tool_name] || '调用一个只读工具，为下一阶段取得可验证证据。';
    if (kind === 'model_started') {
      if (event?.code === 'report_repair') return '正在依据校验反馈修复结构化报告，不调用工具或新增事实。';
      if (event?.code === 'final_report') return '正在依据已登记证据生成结构化结论，不再扩展工具范围。';
      return '正在理解问题与已有证据，选择下一项最小、只读、可验证动作。';
    }
    if (kind === 'model_finished') return '模型响应已经返回；下一阶段只解析工具请求或校验结构化报告。';
    if (kind === 'tool_finished') {
      if (event?.code) return `工具已结束，但返回诊断码 ${event.code}；后续不会把受限结果包装成可靠结论。`;
      return evidenceCount
        ? `工具完成并登记 ${evidenceCount} 份证据；后续结论必须绑定这些证据。`
        : '工具完成但未登记新证据；后续阶段不会据此扩展事实。';
    }
    const overviews = {
      planning: '识别问题类型、可用工具与本轮预算，选择最小可验证路径。',
      knowledge_only_client_scope: '该问题限定为纯知识检索，不调用尚未实现的无界端战斗模拟。',
      validating: '逐项核对报告结构、指标值、单位、证据 ID 与数据路径。',
      report_repair_requested: '报告结构未通过校验；执行一次有界修复，不新增事实或证据。',
      report_citations_normalized: '补全可确定的指标引用关系，保持模拟器原值不变。',
      report_claims_sanitized: '移除未通过数值或引用校验的表述，只发布可验证部分。',
      provider_empty_retry: '供应商返回空正文；保留已有工具证据，并进行一次无工具重试。',
      provider_empty_evidence_preserved: '模型未形成报告，但工具证据仍可复用；系统发布受限结论而非丢弃整轮。',
      knowledge_searches_coalesced: '检测到重复检索意图；复用已有结果并停止无效查询循环。',
      no_new_evidence_finish: '本轮只复用了已有确定性结果；证据已经收敛，直接进入回答。',
      decision_checkpoint: '记录当前观察、证据缺口、工具选择理由与下一步判定条件。',
      reasoning_state_updated: '逐项显示哪些判断已经有证据、哪些可以开始分析、哪些仍需补证。',
      reasoning_critique_started: '从任务完成度、证据归属、因果强度、范围和干预必要性检查报告。',
      reasoning_critique_failed: '报告通过了格式校验，但没有完成本题推导契约；只基于已有证据修订。',
      reasoning_critique_passed: '报告已经回答当前任务，并通过语义与证据边界检查。',
      model_context_compacted: '仅压缩发送给模型的副本；后台完整证据、复现记录和校验路径不变。',
      model_context_handoff: '旧对话被替换为当前问题、分析计划、紧凑证据和最新检查点，避免上下文无限累积。',
      model_context_limit_evidence_preserved: '请求在本地硬上限前停止，未把超长内容发送给供应商。',
      provider_failure_evidence_preserved: '供应商后续请求失败，但失败前完成的本地工具证据和可校验报告仍然保留。',
      evidence_gap_requires_tool: event?.tool_name === 'analyze_timeline'
        ? '必须先取得基线时间轴诊断，才能判断循环哪里做得好、哪里存在风险。'
        : '修改方案还缺少同场景候选对照，暂不发布为已验证结论。',
      completed: '结构、数值与引用均通过校验，发布可溯源结论。',
      partially_verified: '部分内容未通过校验；仅发布已验证结论并保留限制说明。',
      evidence_insufficient: '现有输出无法满足证据规则；不发布未经验证的结论。',
      budget_exhausted: '本轮已达到预设预算；保留现有证据与诊断信息后停止。',
      budget_limit_reached: '本次新实验未执行；保留已有证据并转入受限报告，不中断整段对话。',
      provider_failed: '模型供应商调用失败；工具证据和脱敏诊断仍被保留。',
      protocol_failed: '执行协议未满足预期结构；停止运行并保留可定位的诊断码。',
      timed_out: '任务超过运行时限；终止本轮并保留已完成阶段。',
      refused: '请求触发安全边界；不继续执行或生成结论。',
      cancel_requested: '已收到停止请求，正在安全结束当前运行。',
      cancelled: '运行已取消，已完成的阶段和证据继续保留。',
    };
    return overviews[kind] || '记录本阶段状态，供会话恢复、验收和失败定位。';
  }

  function createTraceStep(className, label, overview, meta, compact) {
    const step = element('div', className);
    const marker = element('span', compact ? 'sim-ai-progress-marker' : 'agent-trace-marker', '');
    marker.setAttribute('aria-hidden', 'true');
    const copy = element('span', compact ? 'sim-ai-progress-copy' : 'agent-trace-copy');
    const body = element('span', compact ? 'sim-ai-progress-body' : 'agent-trace-body');
    body.appendChild(element('b', '', label));
    body.appendChild(element('span', compact ? 'sim-ai-progress-overview' : 'agent-trace-overview', overview));
    copy.appendChild(body);
    if (meta) copy.appendChild(element('small', '', meta));
    step.append(marker, copy);
    return step;
  }

  function appendTraceStep(trace, event) {
    if (!trace || !event) return;
    const kind = event.trace_kind || event.kind;
    const key = `${event.sequence || ''}:${kind}:${event.tool_name || ''}`;
    if (trace.seen.has(key)) return;
    trace.seen.add(key);

    if (kind === 'tool_finished') {
      const queue = trace.openTools.get(event.tool_name) || [];
      const step = queue.shift();
      if (step) {
        step.classList.remove('is-running');
        step.removeAttribute('aria-current');
        step.classList.add(isTraceWarning(event) ? 'is-warning' : 'is-done');
        const label = step.querySelector('b');
        const overview = step.querySelector('.agent-trace-overview');
        const meta = step.querySelector('small');
        if (label) label.textContent = event.label || `${event.code ? '工具返回受限' : '取得证据'} · ${toolLabel(event.tool_name)}`;
        if (overview) overview.textContent = traceStageOverview(event);
        const nextMeta = traceStepMeta(event);
        if (nextMeta && meta) meta.textContent = nextMeta;
        else if (nextMeta) step.querySelector('.agent-trace-copy')?.appendChild(element('small', '', nextMeta));
        if (trace.activeStep === step) trace.activeStep = null;
        scrollTranscript();
        return;
      }
    }

    if (kind === 'model_finished' && trace.openModel) {
      const step = trace.openModel;
      step.classList.remove('is-running');
      step.removeAttribute('aria-current');
      step.classList.add('is-done');
      const label = step.querySelector('b');
      const overview = step.querySelector('.agent-trace-overview');
      const meta = step.querySelector('small');
      if (label) label.textContent = event.label || traceLabels.model_finished;
      if (overview) overview.textContent = traceStageOverview(event);
      if (meta) meta.remove();
      trace.openModel = null;
      if (trace.activeStep === step) trace.activeStep = null;
      scrollTranscript();
      return;
    }

    finishActiveTraceStep(trace);
    const suffix = event.tool_name ? ` · ${toolLabel(event.tool_name)}` : '';
    const label = event.label || (kind === 'tool_started' ? `调用工具${suffix}` : `${traceLabels[kind] || kind}${suffix}`);
    const step = createTraceStep('agent-trace-step', label, traceStageOverview(event), traceStepMeta(event), false);
    const terminal = ['completed', 'partially_verified', 'needs_user_input', 'refused', 'cancelled', 'evidence_insufficient',
      'budget_exhausted', 'provider_failed', 'protocol_failed', 'timed_out'].includes(kind);
    const active = !terminal && !['tool_finished', 'model_finished'].includes(kind);
    step.classList.add(terminal ? (isTraceWarning(event) ? 'is-warning' : 'is-done') : active ? 'is-running' : 'is-done');
    if (active) step.setAttribute('aria-current', 'step');
    if (kind === 'tool_started') {
      const queue = trace.openTools.get(event.tool_name) || [];
      queue.push(step);
      trace.openTools.set(event.tool_name, queue);
      trace.activeStep = step;
    }
    if (kind === 'model_started') {
      trace.openModel = step;
      trace.activeStep = step;
    } else if (active && kind !== 'tool_started') {
      trace.activeStep = step;
    }
    if (event.code) step.title = `诊断码：${event.code}`;
    trace.steps.appendChild(step);
    scrollTranscript();
  }

  function diagnosticHint(result) {
    const code = result?.error?.code || '';
    const hints = {
      invalid_report_json: '模型报告不完整或带有无法识别的外层格式；系统已尝试一次紧凑修复。',
      provider_response_empty: '供应商返回了空正文；系统最多重试一次，仍失败时保留已有证据。',
      numeric_prose_claim: '正文出现无法绑定到指标卡的数字，相关表述已被隐藏。',
      uncited_metric: '模型给出了未绑定证据的指标，相关指标已被隐藏。',
      metric_value_mismatch: '模型指标与模拟器原值不一致，相关指标已被隐藏。',
      provider_http_429: '供应商限流；稍后重试或切换模型档位。',
      provider_http_error: '供应商返回非成功 HTTP 状态；服务端已隐藏响应正文，可结合模型档位和 Run ID 排查。',
      provider_timeout: '供应商未在任务时限内返回；工具证据不会丢失。',
      knowledge_search_budget: '知识检索达到上限；应依据已有结果作答，而不是继续改写查询。',
      simulation_budget: '模拟预算不足以完成请求中的实验数量。',
      tool_call_budget: '工具调用达到单任务上限。',
    };
    return hints[code] || (result?.error?.message ? '服务端已返回固定脱敏错误；可结合阶段和诊断码定位。' : '本轮正常结束，没有记录失败诊断。');
  }

  function diagnosticStage(result) {
    const trace = Array.isArray(result?.trace) ? result.trace : [];
    const terminalStages = {
      provider_failed: '模型供应商调用',
      evidence_insufficient: '报告解析与证据校验',
      protocol_failed: 'Agent 执行协议',
      timed_out: '模型响应等待',
      cancelled: '任务取消',
      budget_exhausted: result?.error?.code === 'knowledge_search_budget' ? '版本知识检索' : '工具预算控制',
    };
    if (terminalStages[result?.status]) return terminalStages[result.status];
    const latest = [...trace].reverse().find(event => event?.kind);
    const kind = latest?.kind || result?.status || 'unknown';
    if (kind === 'tool_started' || kind === 'tool_finished') return `工具执行 · ${toolLabel(latest?.tool_name)}`;
    if (kind === 'validating' || kind.startsWith('report_')) return '报告解析与证据校验';
    if (kind.startsWith('provider_')) return '模型响应处理';
    if (kind === 'planning') return '问题规划';
    return traceLabels[kind] || statusLabels[kind] || kind;
  }

  function appendDiagnostics(parent, result, compact) {
    if (!parent || !result) return;
    const trace = Array.isArray(result.trace) ? result.trace : [];
    const lastTool = [...trace].reverse().find(event => event.kind === 'tool_finished'
      && !event.code && Array.isArray(event.evidence_ids) && event.evidence_ids.length)?.tool_name;
    const evidenceCount = new Set(trace.flatMap(event => event.evidence_ids || [])).size;
    const repairs = trace.filter(event => event.kind === 'report_repair_requested').length;
    const emptyRetries = trace.filter(event => event.kind === 'provider_empty_retry').length;
    const hardFailure = ['evidence_insufficient', 'provider_failed', 'protocol_failed', 'budget_exhausted', 'timed_out'].includes(result.status);
    const details = element('details', compact ? 'sim-ai-debug' : 'agent-debug');
    details.open = hardFailure;
    const summary = element('summary', '');
    summary.appendChild(element('span', '', hardFailure ? '失败调试信息' : '运行诊断'));
    summary.appendChild(element('small', '', result.error?.code || '无错误码'));
    details.appendChild(summary);
    const grid = element('div', compact ? 'sim-ai-debug-grid' : 'agent-debug-grid');
    const rows = [
      ['终止阶段', diagnosticStage(result)],
      ['最后成功工具', lastTool ? toolLabel(lastTool) : '无'],
      ['模型 / 工具轮次', `${result.accounting?.model_turns || 0} / ${result.accounting?.tool_calls || 0}`],
      ['模拟 / 检索', `${result.accounting?.simulations || 0} / ${result.accounting?.knowledge_searches || 0}`],
      ['证据 / 修复 / 空包重试', `${evidenceCount} / ${repairs} / ${emptyRetries}`],
      ['Token', `${result.accounting?.input_tokens || 0} 入 · ${result.accounting?.output_tokens || 0} 出`],
      ['耗时', `${result.accounting?.duration_ms || 0} ms`],
      ['Prompt', result.prompt_version || '—'],
      ['Run', result.run_id || '—'],
    ];
    rows.forEach(([label, value]) => {
      const row = element('div', '');
      row.appendChild(element('span', '', label));
      row.appendChild(element('b', '', value));
      grid.appendChild(row);
    });
    details.appendChild(grid);
    details.appendChild(element('p', compact ? 'sim-ai-debug-hint' : 'agent-debug-hint', diagnosticHint(result)));
    details.appendChild(element('p', compact ? 'sim-ai-debug-safe' : 'agent-debug-safe', '仅显示规范化轨迹与脱敏诊断；不记录隐藏推理、API Key 或供应商原始响应。'));
    parent.appendChild(details);
  }

  function localizedNumber(value, maximumFractionDigits) {
    return value.toLocaleString('zh-CN', { maximumFractionDigits });
  }

  function metricValue(metric) {
    const value = Number(metric?.value);
    if (!Number.isFinite(value)) return '—';
    const unit = String(metric?.unit || '').toLowerCase();
    // Ratios are stored as 0..1 fractions; percent values are already stored
    // in percentage points (for example buff coverage 97.38 means 97.38%).
    if (['fraction', 'ratio'].includes(unit)) {
      return `${localizedNumber(value * 100, 2)}%`;
    }
    if (['percent', 'percentage'].includes(unit)) {
      return `${localizedNumber(value, 2)}%`;
    }
    if (['second', 'seconds', 'sec', 's'].includes(unit)) {
      return `${localizedNumber(value, 2)} 秒`;
    }
    if (['millisecond', 'milliseconds', 'ms'].includes(unit)) {
      return `${localizedNumber(value, 2)} 毫秒`;
    }
    if (['count', 'times', 'event_count'].includes(unit)) {
      return `${localizedNumber(value, 0)} 次`;
    }
    return localizedNumber(value, 2);
  }

  function metricLabel(metric) {
    const label = String(metric?.label || '').trim();
    if (/^dps$/i.test(label)) return '平均 DPS';
    return label || '已验证指标';
  }

  function readableProse(value, metrics) {
    let text = String(value || '—');
    const hidden = '［未验证数值］';
    (metrics || []).forEach(metric => {
      const label = String(metric?.label || '');
      for (const match of label.matchAll(/[+-]?\d[\d,]*(?:\.\d+)?/g)) {
        const number = match[0];
        const start = match.index || 0;
        const before = label.slice(0, start).slice(-1);
        const after = label.slice(start + number.length, start + number.length + 1);
        if (!before || !after) continue;
        text = text.split(`${before}${hidden}${after}`).join(`${before}${number}${after}`);
      }
    });
    return text
      .replaceAll(`${hidden}ms网络延迟`, '网络延迟参数未核验')
      .replaceAll(`目标${hidden}级`, '目标等级未核验')
      .replaceAll(hidden, '未核验');
  }

  function parseTimelineReferences(spec, kind) {
    const ranges = [];
    String(spec || '').replaceAll('，', ',').replaceAll('、', ',').split(/[,/;]/).forEach(part => {
      const numbers = part.match(/\d+/g)?.map(Number).filter(value => value > 0) || [];
      if (!numbers.length) return;
      const start = numbers[0] - 1;
      const end = (numbers[1] || numbers[0]) - 1;
      ranges.push({
        kind: kind === 'ev' ? 'event' : 'operation',
        start: Math.min(start, end),
        end: Math.max(start, end),
      });
    });
    return ranges;
  }

  let rotationRefPopover = null;
  let rotationRefCloseTimer = null;

  function closeRotationReferencePopover() {
    if (rotationRefCloseTimer) window.clearTimeout(rotationRefCloseTimer);
    rotationRefCloseTimer = window.setTimeout(() => {
      rotationRefPopover?.remove();
      rotationRefPopover = null;
    }, 180);
  }

  function keepRotationReferencePopover() {
    if (rotationRefCloseTimer) window.clearTimeout(rotationRefCloseTimer);
    rotationRefCloseTimer = null;
  }

  function positionRotationReferencePopover(popover, anchor) {
    const rect = anchor.getBoundingClientRect();
    const margin = 10;
    const width = Math.min(300, window.innerWidth - margin * 2);
    popover.style.width = `${width}px`;
    popover.style.left = `${Math.max(margin, Math.min(window.innerWidth - width - margin, rect.left))}px`;
    const spaceBelow = window.innerHeight - rect.bottom - margin - 8;
    const spaceAbove = rect.top - margin - 8;
    const height = popover.offsetHeight;
    const below = rect.bottom + 8;
    popover.style.top = `${spaceBelow >= height || spaceBelow >= spaceAbove
      ? below
      : Math.max(margin, rect.top - height - 8)}px`;
  }

  function showRotationReferencePopover(anchor, label, ranges) {
    keepRotationReferencePopover();
    rotationRefPopover?.remove();
    const popover = element('aside', 'agent-rotation-popover');
    popover.setAttribute('role', 'dialog');
    popover.setAttribute('aria-label', `${label}的技能轴位置`);
    const head = element('div', 'agent-rotation-popover-head');
    head.appendChild(element('b', '', label));
    head.appendChild(element('span', '', ranges.length > 1 ? `${ranges.length} 个时间点` : '技能轴摘要'));
    popover.appendChild(head);
    const descriptions = window.Jx3TimelineBridge?.describe?.(ranges) || [];
    let activeOccurrence = 0;
    const occurrenceRows = [];
    if (descriptions.length > 1) {
      const pager = element('span', 'agent-occurrence-pager');
      const count = element('span', '', `1 / ${descriptions.length}`);
      const previous = element('button', '', '‹');
      const next = element('button', '', '›');
      previous.type = next.type = 'button';
      previous.setAttribute('aria-label', '上一处'); next.setAttribute('aria-label', '下一处');
      const select = delta => {
        activeOccurrence = (activeOccurrence + delta + descriptions.length) % descriptions.length;
        occurrenceRows.forEach((row, index) => { row.hidden = index !== activeOccurrence; });
        count.textContent = `${activeOccurrence + 1} / ${descriptions.length}`;
        positionRotationReferencePopover(popover, anchor);
      };
      previous.addEventListener('click', () => select(-1)); next.addEventListener('click', () => select(1));
      pager.append(previous, count, next); head.appendChild(pager);
    }
    if (!descriptions.length) {
      popover.appendChild(element('p', 'agent-rotation-popover-empty', '当前页面没有可对应的模拟技能轴。'));
    } else {
      descriptions.forEach((description, index) => {
        const row = element('div', 'agent-rotation-occurrence');
        row.hidden = index > 0;
        occurrenceRows.push(row);
        const meta = element('div', 'agent-rotation-occurrence-meta');
        meta.appendChild(element('b', '', descriptions.length > 1 ? `位置 ${index + 1} · ${description.timeLabel}` : description.timeLabel));
        meta.appendChild(element('span', '', '当前模拟时间'));
        row.appendChild(meta);
        const skills = element('div', 'agent-rotation-skill-strip');
        const details = element('div', 'agent-rotation-state');
        const showState = (skill, phase) => {
          details.replaceChildren();
          const snapshot = phase === 'before' ? skill.before : skill.after;
          details.appendChild(element('b', '', `${skill.name} · ${Number.isFinite(skill.time) ? skill.time.toFixed(2) + 's · ' : ''}释放${phase === 'before' ? '前' : '后'}`));
          if (!snapshot) {
            details.appendChild(element('p', '', '当前结果没有该状态快照，请运行完整模拟后查看。'));
            return;
          }
          details.appendChild(element('p', '', `怒气 ${snapshot.rage}${skill.rageCost != null ? ` · 本次消耗 ${skill.rageCost}` : ''}${snapshot.block_value != null ? ` · 盾值 ${snapshot.block_value}` : ''}`));
          const damage = [['命中', skill.damageNormal], ['会心', skill.damageCrit], ['期望', skill.damage]]
            .filter(([, value]) => value != null).map(([label, value]) => `${label} ${typeof formatDamage === 'function' ? formatDamage(Number(value)) : Number(value).toLocaleString('zh-CN', {maximumFractionDigits: 0})}`);
          if (damage.length) details.appendChild(element('p', '', `伤害：${damage.join(' / ')}`));
          const buffGroups = element('div', 'agent-state-groups');
          [['自身 Buff', snapshot.buffs], ['目标 Buff', snapshot.target_buffs]].forEach(([title, buffs]) => {
            const group = element('div');
            group.appendChild(element('div', 'agent-state-label', title));
            const list = element('div', 'agent-state-buffs');
            (buffs || []).forEach(buff => {
              const item = element('span', 'agent-state-buff');
              item.title = `${buff.name} · ${buff.stacks}层 · ${buff.remaining > 0 ? buff.remaining.toFixed(1) + '秒' : '持续生效'}`;
              if (buff.iconUrl) { const img = element('img'); img.src = buff.iconUrl; img.alt = buff.name; item.appendChild(img); }
              else item.appendChild(element('span', '', buff.name || '?'));
              if (buff.stacks > 1) item.appendChild(element('b', '', String(buff.stacks)));
              item.appendChild(element('small', '', buff.remaining > 0 ? buff.remaining.toFixed(1) + 's' : '∞'));
              list.appendChild(item);
            });
            if (!buffs?.length) list.appendChild(element('span', '', '无'));
            group.appendChild(list);
            buffGroups.appendChild(group);
          });
          details.appendChild(buffGroups);
        };
        description.skills.forEach(skill => {
          const gap = element('button', 'agent-skill-gap', '·');
          gap.type = 'button';
          gap.setAttribute('aria-label', `${skill.name}释放前`);
          gap.addEventListener('mouseenter', () => showState(skill, 'before'));
          gap.addEventListener('focus', () => showState(skill, 'before'));
          skills.appendChild(gap);
          const chip = element('button', `agent-skill-chip${skill.selected ? ' is-target' : ''}`);
          chip.type = 'button';
          chip.setAttribute('aria-label', `${skill.name}释放后`);
          if (skill.iconUrl) { const img = element('img'); img.src = skill.iconUrl; img.alt = ''; chip.appendChild(img); }
          chip.appendChild(element('span', '', skill.name));
          chip.addEventListener('mouseenter', () => showState(skill, 'after'));
          chip.addEventListener('focus', () => showState(skill, 'after'));
          skills.appendChild(chip);
        });
        row.appendChild(skills);
        row.appendChild(element('small', 'agent-state-label', '悬停技能看释放后，悬停间隔看下一技能释放前'));
        row.appendChild(details);
        const selected = description.skills.find(skill => skill.selected) || description.skills[0];
        if (selected) showState(selected, 'after');
        const button = element('button', 'agent-rotation-locate', '在技能轴中标记');
        button.type = 'button';
        button.disabled = !description.valid;
        button.addEventListener('click', event => {
          event.stopPropagation();
          window.Jx3TimelineBridge?.focus?.(description);
          rotationRefPopover?.remove();
          rotationRefPopover = null;
        });
        row.appendChild(button);
        popover.appendChild(row);
      });
    }
    popover.addEventListener('mouseenter', keepRotationReferencePopover);
    popover.addEventListener('mouseleave', closeRotationReferencePopover);
    document.body.appendChild(popover);
    rotationRefPopover = popover;
    positionRotationReferencePopover(popover, anchor);
  }

  /**
   * 渲染模型给出的稳定操作引用。
   * [[显示文字|op:22,23,24]] 绑定输入操作，[[显示文字|ev:42,97]]
   * 绑定实际释放事件；两者都只向用户显示时间。旧会话中的“行20”
   * 兼容成手动输入的单点引用。
   */
  function normalizeTimelineProse(value) {
    // Keep machine anchors in the link target, including references in old sessions.
    const grouped = String(value || '').replace(
      /\[\[([^|\]]+)\|(ev|op):([^\]]+)\]\]((?:\s*\/\s*(?:ev\s*[:：]?\s*)?\d+)+)/gi,
      (_, label, kind, spec, rest) => `[[${label}|${kind}:${spec}${rest.replace(/ev\s*[:：]?\s*/gi, '')}]]`);
    return grouped.split(/(\[\[[^\]]+\]\])/g).map(part => {
      if (part.startsWith('[[')) return part;
      return part.replace(/(\d+(?:\.\d+)?\s*(?:s|秒))\s*[（(]([^()（）\n]*?)[,，、\s]*\bev\s*[:：]?\s*(\d+)[)）]|\bev\s*[:：]?\s*(\d+(?:\s*\/\s*(?:ev\s*[:：]?\s*)?\d+)*)\b/gi,
        (_, time, detail, id, bareId) => time
          ? `[[${time}${detail.replace(/[,，、\s]+$/, '') ? `（${detail.replace(/[,，、\s]+$/, '')}）` : ''}|ev:${id}]]`
          : `[[对应技能|ev:${bareId.replace(/ev\s*[:：]?\s*/gi, '')}]]`);
    }).join('');
  }

  function timelineReferenceLabel(label) {
    return String(label || '').replace(/\bev\s*[:：]?\s*\d+\b/gi, '').trim() || '对应技能';
  }

  function appendRichProse(parent, tagName, className, value, metrics) {
    const text = normalizeTimelineProse(readableProse(value, metrics));
    const root = element(tagName === 'p' ? 'div' : tagName, `${className || ''} agent-prose`);
    parent.appendChild(root);
    const lines = text.split(/\r?\n/);
    // Protect anchor pipes before splitting Markdown table cells.
    const cells = line => {
      const anchors = [];
      const masked = line.replace(/\[\[[^\]]+\]\]/g, match => `\u0001${anchors.push(match) - 1}\u0001`);
      return masked.trim().replace(/^\|/, '').replace(/\|$/, '').split('|')
        .map(cell => cell.trim().replace(/\u0001(\d+)\u0001/g, (_, index) => anchors[index]));
    };
    const lists = [];
    for (let i = 0; i < lines.length; i++) {
      const line = lines[i].trim();
      if (!line) continue;
      const marker = /^(\s*)(?:(\d{1,9})([.)])|([-*+]))\s+(.*)$/.exec(lines[i]);
      if (marker) {
        const indent = marker[1].replace(/\t/g, '    ').length;
        const kind = marker[2] ? 'ol' : 'ul';
        const delimiter = marker[3] || marker[4];
        while (lists.length && lists.at(-1).indent > indent) lists.pop();
        if (lists.length && lists.at(-1).indent === indent
            && (lists.at(-1).kind !== kind || lists.at(-1).delimiter !== delimiter)) lists.pop();
        let current = lists.at(-1);
        if (!current || current.indent !== indent) {
          const list = element(kind);
          if (kind === 'ol' && Number(marker[2]) !== 1) list.start = Number(marker[2]);
          (current?.item || root).appendChild(list);
          current = { list, indent, kind, delimiter, item: null };
          lists.push(current);
        }
        appendInlineProse(current.list, 'li', '', marker[5]);
        current.item = current.list.lastElementChild;
        continue;
      }
      const indent = /^\s*/.exec(lines[i])[0].replace(/\t/g, '    ').length;
      while (lists.length && indent <= lists.at(-1).indent) lists.pop();
      if (lists.length && !/^(?:```|#{1,6}\s)/.test(line)) {
        appendInlineProse(lists.at(-1).item, 'span', 'agent-prose-line', line);
        continue;
      }
      lists.length = 0;
      if (/^```/.test(line)) {
        const code = [];
        while (++i < lines.length && !/^```/.test(lines[i])) code.push(lines[i]);
        root.appendChild(element('pre', '', code.join('\n')));
      } else if (i + 1 < lines.length && lines[i + 1].includes('|') && cells(lines[i + 1]).every(cell => /^:?-{3,}:?$/.test(cell))) {
        const wrap = element('div', 'agent-prose-table');
        const table = element('table');
        const header = element('tr');
        cells(line).forEach(cell => appendInlineProse(header, 'th', '', cell));
        table.appendChild(header);
        i++;
        while (i + 1 < lines.length && lines[i + 1].includes('|') && lines[i + 1].trim()) {
          const row = element('tr');
          cells(lines[++i]).forEach(cell => appendInlineProse(row, 'td', '', cell));
          table.appendChild(row);
        }
        wrap.appendChild(table); root.appendChild(wrap);
      } else if (/^#{1,6}\s+/.test(line)) {
        appendInlineProse(root, 'h4', '', line.replace(/^#{1,6}\s+/, ''));
      } else {
        appendInlineProse(root, 'span', 'agent-prose-line', line);
      }
    }
    return root;
  }

  function appendFormattedText(parent, text) {
    const pattern = /\*\*([^*]+)\*\*|`([^`]+)`/g;
    let cursor = 0;
    for (const match of text.matchAll(pattern)) {
      parent.appendChild(document.createTextNode(text.slice(cursor, match.index)));
      parent.appendChild(element(match[1] ? 'strong' : 'code', '', match[1] || match[2]));
      cursor = match.index + match[0].length;
    }
    parent.appendChild(document.createTextNode(text.slice(cursor)));
  }

  function appendInlineProse(parent, tagName, className, value, metrics) {
    const text = value == null || value === '' ? '' : normalizeTimelineProse(readableProse(value, metrics));
    const node = element(tagName, className);
    const emphasis = /\*\*([\s\S]+?)\*\*/.exec(text);
    if (emphasis) {
      appendInlineProse(node, 'span', '', text.slice(0, emphasis.index));
      appendInlineProse(node, 'strong', '', emphasis[1]);
      appendInlineProse(node, 'span', '', text.slice(emphasis.index + emphasis[0].length));
      parent.appendChild(node);
      return node;
    }
    const pattern = /\[\[([^|\]]+)\|(op|ev):([^\]]+)\]\]|行(\d+)/g;
    let cursor = 0;
    let match;
    while ((match = pattern.exec(text)) !== null) {
      if (match.index > cursor) appendFormattedText(node, text.slice(cursor, match.index));
      const label = timelineReferenceLabel(match[1] || '对应位置');
      const ranges = parseTimelineReferences(match[3] || match[4], match[2] || 'op');
      const legacyMacroLine = !!match[4] && !!window._lastSimBody?.macro_text;
      if (!ranges.length || legacyMacroLine) {
        node.appendChild(document.createTextNode(label));
      } else {
        const reference = element('button', 'agent-rotation-reference', label);
        reference.type = 'button';
        reference.setAttribute('aria-haspopup', 'dialog');
        reference.title = ranges.length > 1 ? `查看 ${ranges.length} 个具体技能轴位置` : '查看对应技能轴位置';
        const timeLabels = window.Jx3TimelineBridge?.describe?.(ranges)
          ?.map(item => item.timeLabel)
          .filter(value => value && !value.includes('未找到')) || [];
        if (timeLabels.length && !/\d+(?:\.\d+)?\s*(?:s|秒)/i.test(label)) {
          const visibleTimes = timeLabels.length <= 3
            ? timeLabels.join(' · ')
            : `${timeLabels.slice(0, 2).join(' · ')} · 共${timeLabels.length}处`;
          reference.appendChild(element('span', 'agent-rotation-reference-time', visibleTimes));
        }
        reference.addEventListener('mouseenter', () => showRotationReferencePopover(reference, label, ranges));
        reference.addEventListener('mouseleave', closeRotationReferencePopover);
        reference.addEventListener('focus', () => showRotationReferencePopover(reference, label, ranges));
        reference.addEventListener('blur', closeRotationReferencePopover);
        reference.addEventListener('click', event => {
          event.stopPropagation();
          showRotationReferencePopover(reference, label, ranges);
        });
        node.appendChild(reference);
      }
      cursor = pattern.lastIndex;
    }
    if (cursor < text.length) appendFormattedText(node, text.slice(cursor));
    parent.appendChild(node);
    return node;
  }

  function plainAgentText(value) {
    return normalizeTimelineProse(value).replace(/\[\[([^|\]]+)\|(op|ev):([^\]]+)\]\]/g,
      (_, label) => timelineReferenceLabel(label));
  }

  function appendMetricGrid(parent, metrics, className) {
    if (!metrics?.length) return;
    const grid = element('div', className);
    metrics.forEach(metric => {
      const itemClass = className === 'agent-metrics' ? 'agent-metric' : 'sim-ai-result-metric';
      const item = element('div', itemClass);
      item.appendChild(element('b', '', metricValue(metric)));
      item.appendChild(element('span', '', metricLabel(metric)));
      item.title = `${metric.evidence_id}${metric.json_pointer}`;
      grid.appendChild(item);
    });
    parent.appendChild(grid);
  }

  function safeExternalUrl(value) {
    try {
      const url = new URL(String(value || ''));
      return ['https:', 'http:'].includes(url.protocol) ? url.href : null;
    } catch (_) {
      return null;
    }
  }

  function sourceAnchor(label, value) {
    const href = safeExternalUrl(value);
    if (!href) return null;
    const link = element('a', 'agent-source-link', label);
    link.href = href;
    link.target = '_blank';
    link.rel = 'noopener noreferrer';
    return link;
  }

  function appendKnowledgeSources(parent, sources, compact) {
    if (!sources?.length) return;
    const block = element('details', compact ? 'sim-ai-result-sources' : 'agent-sources');
    block.open = !compact && sources.length <= 3;
    block.appendChild(element('summary', '', `参考资料（${sources.length}）`));
    const list = element('div', 'agent-source-list');
    sources.slice(0, compact ? 4 : 10).forEach(source => {
      const item = element('article', 'agent-source-item');
      const titleLink = sourceAnchor(source.title || '未命名资料', source.source_url || source.yuque_url);
      item.appendChild(titleLink || element('span', 'agent-source-title', source.title || '未命名资料'));
      const meta = element('div', 'agent-source-meta');
      [source.season, source.category, versionLabels[source.version_match] || source.version_match]
        .filter(Boolean)
        .forEach(value => meta.appendChild(element('span', '', value)));
      meta.appendChild(element(
        'span',
        source.fact_eligible ? 'is-grounded' : 'is-limited',
        source.fact_eligible ? '正文可核验' : '仅来源入口',
      ));
      if (source.version_warning) {
        const warning = element('span', 'is-warning', '版本信息冲突');
        warning.title = source.version_warning;
        meta.appendChild(warning);
      }
      item.appendChild(meta);
      const actions = element('div', 'agent-source-actions');
      const original = sourceAnchor('查看原始来源 ↗', source.source_url);
      const yuque = source.yuque_url !== source.source_url
        ? sourceAnchor('语雀目录 ↗', source.yuque_url)
        : null;
      if (original) actions.appendChild(original);
      if (yuque) actions.appendChild(yuque);
      if (actions.childNodes.length) item.appendChild(actions);
      list.appendChild(item);
    });
    block.appendChild(list);
    parent.appendChild(block);
  }

  function clarificationOptions(clarification) {
    if (Array.isArray(clarification.options) && clarification.options.length) {
      return clarification.options.filter(option => typeof option?.label === 'string' && option.label.trim())
        .slice(0, 4).map(option => ({ label: option.label.trim(), description: String(option.description || '') }));
    }
    // Older runs stored a short slash-separated answer hint. Only split an
    // explicit spaced separator, so units and ordinary prose stay intact.
    const parts = String(clarification.answer_hint || '').split(/\s+[\/／|｜]\s+/).map(value => value.trim()).filter(Boolean);
    return parts.length >= 2 && parts.length <= 4 && parts.every(value => value.length <= 120)
      ? [...new Set(parts)].map(label => ({ label, description: '' })) : [];
  }

  function updateClarificationControls() {
    document.querySelectorAll('.agent-answer-form').forEach(form => {
      const current = form.dataset.sessionId === currentSessionId
        && latestRunBySession.get(currentSessionId) === form.dataset.runId;
      form.querySelector('fieldset').disabled = composerBusy || !current;
      form.querySelector('.agent-answer-state').textContent = current
        ? (composerBusy ? '正在处理，请稍候…' : '选择后确认，或自行填写答案')
        : '历史追问 · 请在最新对话中继续';
    });
  }

  function noteRenderedRun(result) {
    if (currentSessionId && result?.run_id) latestRunBySession.set(currentSessionId, result.run_id);
    queueMicrotask(updateClarificationControls);
  }

  function appendClarificationChoices(parent, result, compact) {
    const clarification = result.clarification;
    const options = clarificationOptions(clarification);
    const form = element('form', 'agent-answer-form');
    form.dataset.sessionId = currentSessionId || '';
    form.dataset.runId = result.run_id || '';
    const group = element('fieldset', 'agent-answer-group');
    group.appendChild(element('legend', 'agent-answer-legend', '请选择你的回答'));
    const radioName = `agent-answer-${++clarificationSerial}`;
    const custom = element('textarea', 'agent-answer-custom');
    custom.placeholder = '输入你的答案或补充条件…';
    custom.setAttribute('aria-label', '自行填写回答');
    custom.rows = 2;
    custom.maxLength = 2000;
    custom.hidden = options.length > 0;
    let selection = options.length ? null : 'custom';
    const submit = element('button', 'sim-btn sim-btn-primary agent-answer-submit', '确认并继续');
    submit.type = 'submit';
    submit.disabled = true;
    const sync = () => { submit.disabled = selection === null || (selection === 'custom' && !custom.value.trim()); };
    [...options, { label: '自行填写', description: '补充条件，或给出其他答案', custom: true }].forEach((option, index) => {
      const label = element('label', 'agent-answer-option');
      const radio = document.createElement('input');
      radio.type = 'radio';
      radio.name = radioName;
      radio.value = option.custom ? 'custom' : String(index);
      radio.checked = !options.length && option.custom;
      const text = element('span', 'agent-answer-option-text');
      text.appendChild(element('b', '', `${index + 1}. ${option.label}`));
      if (option.description) text.appendChild(element('small', '', option.description));
      label.append(radio, text);
      radio.addEventListener('change', () => {
        selection = option.custom ? 'custom' : index;
        custom.hidden = !option.custom;
        sync();
        if (option.custom) custom.focus();
      });
      group.appendChild(label);
    });
    if (!options.length && clarification.answer_hint) {
      group.appendChild(element('p', 'agent-answer-hint', clarification.answer_hint));
    }
    custom.addEventListener('input', sync);
    group.append(custom, submit);
    form.append(group, element('small', 'agent-answer-state', '选择后确认，或自行填写答案'));
    form.addEventListener('submit', async event => {
      event.preventDefault();
      if (composerBusy || form.dataset.sessionId !== currentSessionId
        || latestRunBySession.get(currentSessionId) !== result.run_id) return;
      const answer = selection === 'custom' ? custom.value.trim() : options[selection]?.label;
      if (!answer) return;
      const question = `针对你的问题：${clarification.question}\n我的回答：${answer}`;
      if (compact) await startDockRun(question);
      else await startRun(question);
      updateClarificationControls();
    });
    parent.appendChild(form);
    queueMicrotask(updateClarificationControls);
  }

  function renderReport(result) {
    noteRenderedRun(result);
    const report = result?.report;
    if (!report) {
      if (result?.clarification) {
        const actions = [cardCopyButton('复制问题', () => result.clarification.question, false)];
        const message = appendMessage('agent', result.clarification.question, actions);
        message.classList.add('agent-clarification');
        if (result.clarification.analysis_text) {
          const progress = element('div', 'agent-clarification-analysis');
          progress.appendChild(element('small', '', '当前分析 · 尚未完成报告校验'));
          appendRichProse(progress, 'p', '', result.clarification.analysis_text);
          message.prepend(progress);
        }
        if (result.clarification.reason) {
          message.appendChild(element('p', 'agent-clarification-reason', result.clarification.reason));
        }
        appendClarificationChoices(message, result, false);
        setTimeout(() => {
          els.question.placeholder = '回答上面的问题，继续当前会话…';
          els.question.focus();
        }, 0);
        return;
      }
      const reason = result?.status === 'provider_failed'
        ? providerErrorText(result?.error)
        : result?.error?.message || `任务状态：${statusLabels[result?.status] || result?.status || '未知'}`;
      const message = appendMessage('agent', reason, [
        cardCopyButton('复制调试信息', () => buildDebugBundle(result), false),
      ]);
      appendDiagnostics(message, result, false);
      return;
    }
    const card = element('article', 'agent-report');
    const allMetrics = (report.content?.findings || []).flatMap(finding => finding.metrics || []);
    const evidenceInsufficient = result.status === 'evidence_insufficient';
    const partiallyVerified = result.status === 'partially_verified';
    card.classList.toggle('is-limited', evidenceInsufficient || partiallyVerified);
    const head = element('div', 'agent-report-head');
    head.appendChild(element('b', '', evidenceInsufficient
      ? '本轮未形成可靠结论'
      : partiallyVerified ? '部分验证分析报告' : '已验证分析报告'));
    const actions = element('span', 'agent-card-actions');
    actions.appendChild(element('span', '', `${report.provider_profile} / ${report.model} · ${result.accounting?.duration_ms || 0}ms`));
    actions.appendChild(cardCopyButton('复制结论', () => buildSummary(result), false));
    actions.appendChild(cardCopyButton('复制调试信息', () => buildDebugBundle(result), false));
    head.appendChild(actions);
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
    appendRichProse(card, 'div', 'agent-report-summary', report.content?.summary, allMetrics);
    appendEquipmentComparisons(card, report.equipment_comparisons || [], false);

    (report.content?.findings || []).forEach(finding => {
      const block = element('section', 'agent-finding');
      appendRichProse(block, 'h4', '', finding.title, finding.metrics);
      appendRichProse(block, 'p', '', finding.explanation, finding.metrics);
      appendMetricGrid(block, finding.metrics, 'agent-metrics');
      card.appendChild(block);
    });

    appendRotationChanges(card, report.content?.rotation_changes || [], allMetrics, false);
    appendDraftArtifacts(card, report.content?.artifacts || [], false);

    const recommendations = report.content?.recommendations || [];
    if (recommendations.length) {
      const block = element('section', 'agent-finding');
      block.appendChild(element('h4', '', '建议的下一步实验'));
      recommendations.forEach(item => appendRichProse(
        block,
        'p',
        '',
        `${readableProse(item.title, allMetrics)}：${readableProse(item.rationale, allMetrics)}`,
        allMetrics,
      ));
      card.appendChild(block);
    }
    appendKnowledgeSources(card, report.sources || [], false);
    const limitations = report.content?.limitations || [];
    if (limitations.length) {
      const block = element('details', 'agent-boundaries');
      block.appendChild(element('summary', '', `分析边界（${limitations.length}）`));
      limitations.forEach(item => block.appendChild(element('p', '', readableProse(item, allMetrics)
        .replaceAll('simulate_scenario', '基线模拟')
        .replaceAll('get_current_scenario', '当前场景读取')
        .replaceAll('engine_commit_unavailable', '引擎提交信息缺失')
        .replaceAll('engine_string', '引擎版本'))));
      card.appendChild(block);
    }
    appendDiagnostics(card, result, false);
    card.appendChild(element('div', 'agent-evidence', `scenario ${result.scenario_hash} · prompt ${result.prompt_version} / ${result.prompt_sha256} · evidence ${(report.evidence_ids || []).join(', ') || 'none'}`));
    els.transcript.appendChild(card);
    scrollTranscript();
  }

  function macroDisplayPages(artifact) {
    if (artifact.language !== 'jx3_macro') return null;
    const names = { shield: '盾宏', '擎盾': '盾宏', blade: '刀宏', '擎刀': '刀宏', wall: '盾墙宏', '盾墙': '盾墙宏', '': '通用宏' };
    const pages = [];
    let title = names[''];
    let lines = [];
    const flush = () => {
      const content = lines.join('\n').trimEnd();
      if (content.trim()) pages.push({ title, content });
      lines = [];
    };
    for (const line of (artifact.content || '').replace(/\r\n?/g, '\n').split('\n')) {
      const marker = line.trim().match(/^#page(?:\s+(.*))?$/);
      if (marker) {
        const stance = (marker[1] || '').trim();
        // Unknown page syntax stays visible verbatim for correction.
        if (!Object.prototype.hasOwnProperty.call(names, stance)) return null;
        flush();
        title = names[stance];
      } else lines.push(line);
    }
    flush();
    return pages.length ? pages : null;
  }

  function appendDraftArtifacts(parent, artifacts, compact) {
    artifacts.forEach(artifact => {
      const pages = macroDisplayPages(artifact);
      const block = element('section', compact ? 'sim-ai-result-finding agent-draft-artifact' : 'agent-finding agent-draft-artifact');
      const head = element('div', 'agent-draft-head');
      head.appendChild(element('b', '', artifact.title || '候选草稿'));
      head.appendChild(cardCopyButton(pages ? '复制完整宏' : '复制代码', () => artifact.content || '', compact));
      block.appendChild(head);
      const status = artifact.syntax === 'invalid' ? '语法待修正'
        : artifact.syntax === 'parsed' ? '语法已解析' : '候选代码';
      block.appendChild(element('small', 'agent-draft-status', pages ? `${pages.length} 页 · ${status}` : status));
      if (pages) {
        const group = element('div', 'agent-macro-pages');
        pages.forEach((page, index) => {
          const section = element('section', 'agent-macro-page');
          const pageHead = element('div', 'agent-macro-page-head');
          const label = element('div', 'agent-macro-page-label');
          label.appendChild(element('b', '', page.title));
          const count = element('span', `agent-macro-page-count${page.content.length > 128 ? ' is-over-limit' : ''}`, `${page.content.length} / 128 字`);
          count.title = '正文含换行；分页标记不计入字数';
          label.appendChild(count);
          pageHead.appendChild(label);
          const copy = cardCopyButton(`复制${page.title}`, () => page.content, compact);
          copy.title = '复制本页正文，不含模拟器分页标记';
          pageHead.appendChild(copy);
          section.appendChild(pageHead);
          const pre = element('pre', 'agent-draft-code agent-macro-code');
          pre.tabIndex = 0;
          pre.setAttribute('aria-label', `${page.title}代码，第 ${index + 1} 页`);
          pre.appendChild(element('code', '', page.content));
          section.appendChild(pre);
          group.appendChild(section);
        });
        block.appendChild(group);
      } else {
        const pre = element('pre', 'agent-draft-code');
        pre.appendChild(element('code', '', artifact.content || ''));
        block.appendChild(pre);
      }
      parent.appendChild(block);
    });
  }

  function appendRotationChanges(parent, changes, allMetrics, compact) {
    if (!changes.length) return;
    const block = element(compact ? 'div' : 'section', compact ? 'sim-ai-result-finding agent-rotation-changes' : 'agent-finding agent-rotation-changes');
    block.appendChild(element(compact ? 'b' : 'h4', '', '循环修改方案'));
    changes.slice(0, compact ? 3 : 8).forEach(change => {
      const item = element('div', 'agent-rotation-change');
      const mode = change.change_type === 'macro_statement' ? '宏语句' : '手动操作点';
      const operationLabels = { replace: '替换', insert_before: '在前插入', insert_after: '在后插入', adjust_timing: '调整时序' };
      const operation = operationLabels[change.edit_operation] || '修改';
      item.appendChild(element('div', 'agent-rotation-change-target', `${mode} · ${operation} · ${readableProse(change.target, allMetrics)}`));
      const current = element('div', 'agent-rotation-code');
      current.appendChild(element('span', '', '当前'));
      current.appendChild(element('code', '', change.current || '—'));
      item.appendChild(current);
      const proposed = element('div', 'agent-rotation-code is-proposed');
      proposed.appendChild(element('span', '', '修改'));
      proposed.appendChild(element('code', '', change.proposed || '—'));
      item.appendChild(proposed);
      appendRichProse(item, 'p', '', change.rationale, allMetrics);
      block.appendChild(item);
    });
    parent.appendChild(block);
  }

  function formatEquipmentValue(value, unit) {
    const number = Number(value || 0);
    if (unit === '%') return `${number.toFixed(2)}%`;
    return Math.abs(number) >= 1000
      ? Math.round(number).toLocaleString('zh-CN')
      : number.toLocaleString('zh-CN', { maximumFractionDigits: 2 });
  }

  function appendEquipmentComparisons(parent, comparisons, compact) {
    comparisons.slice(0, compact ? 1 : 3).forEach(comparison => {
      const block = element(compact ? 'div' : 'section', compact
        ? 'sim-ai-equipment-compare' : 'agent-equipment-compare');
      const head = element('div', 'agent-eq-compare-head');
      head.appendChild(element('b', '', '换装实测'));
      head.appendChild(element('span', '', comparison.source_label || '当前循环'));
      block.appendChild(head);
      const itemGrid = element('div', 'agent-eq-item-grid');
      const before = element('div', 'agent-eq-item');
      before.appendChild(element('small', '', '换前'));
      before.appendChild(element('b', '', comparison.before_item || '当前装备'));
      const after = element('div', 'agent-eq-item');
      after.appendChild(element('small', '', '换后'));
      after.appendChild(element('b', '', comparison.after_item || '候选装备'));
      itemGrid.append(before, after);
      block.appendChild(itemGrid);
      const table = element('div', 'agent-eq-diff-table');
      const tableHead = element('div', 'agent-eq-diff-row is-head');
      ['属性', '换前', '换后', '变化'].forEach(label => tableHead.appendChild(element('span', '', label)));
      table.appendChild(tableHead);
      (comparison.panel_rows || []).filter(row => Math.abs(Number(row.delta || 0)) > 0.0001).slice(0, compact ? 7 : 10).forEach(row => {
        const line = element('div', 'agent-eq-diff-row');
        const delta = Number(row.delta || 0);
        const changeClass = delta > 0 ? 'is-up' : delta < 0 ? 'is-down' : 'is-flat';
        line.appendChild(element('span', '', row.label || row.key));
        line.appendChild(element('span', '', formatEquipmentValue(row.before, row.unit)));
        line.appendChild(element('span', '', formatEquipmentValue(row.after, row.unit)));
        line.appendChild(element('span', changeClass, `${delta > 0 ? '↑ +' : delta < 0 ? '↓ ' : '→ '}${formatEquipmentValue(delta, row.unit)}`));
        table.appendChild(line);
      });
      block.appendChild(table);
      const dps = element('div', 'agent-eq-dps');
      dps.appendChild(element('span', '', `DPS ${formatEquipmentValue(comparison.before_dps)} → ${formatEquipmentValue(comparison.after_dps)}`));
      const delta = Number(comparison.dps_delta || 0);
      dps.appendChild(element('b', delta > 0 ? 'is-up' : delta < 0 ? 'is-down' : 'is-flat',
        `${delta > 0 ? '↑ +' : delta < 0 ? '↓ ' : '→ '}${formatEquipmentValue(delta)}（${Number(comparison.dps_delta_percent || 0).toFixed(2)}%）`));
      block.appendChild(dps);
      parent.appendChild(block);
    });
  }

  function scrollTranscript() {
    requestAnimationFrame(() => { els.transcript.scrollTop = els.transcript.scrollHeight; });
  }

  function clearDockChat() {
    clear(els.dockChat);
    const welcome = element('div', 'sim-ai-welcome');
    if (dockMode === 'equipment') {
      welcome.appendChild(element('span', '', 'AI 只读当前配装、装备库与确定性模拟器。'));
      welcome.appendChild(element('small', '', '换装实验会重算面板和当前循环 DPS，不会应用或覆盖配装。'));
    } else {
      welcome.appendChild(element('span', '', 'AI 只读当前循环、版本知识库与确定性模拟器。'));
      welcome.appendChild(element('small', '', '资料会标注赛季和来源，不会修改任何游戏数据。'));
    }
    els.dockChat.appendChild(welcome);
    dockTrace = null;
  }

  function prepareDockChat() {
    const welcome = els.dockChat.querySelector('.sim-ai-welcome');
    if (welcome) welcome.remove();
  }

  function scrollDock() {
    requestAnimationFrame(() => { els.dockChat.scrollTop = els.dockChat.scrollHeight; });
  }

  function appendDockBubble(role, text, isError, feedbackTurn) {
    prepareDockChat();
    const wrap = element('article', `sim-ai-bubble ${role}${isError ? ' sim-ai-error' : ''}`);
    const head = element('div', 'sim-ai-bubble-head');
    if (feedbackTurn) head.appendChild(feedbackCopyButton(feedbackTurn));
    head.appendChild(element('div', 'sim-ai-bubble-label', role === 'user' ? '你' : 'AI 分析'));
    wrap.appendChild(head);
    wrap.appendChild(element('div', 'sim-ai-bubble-body', text));
    els.dockChat.appendChild(wrap);
    scrollDock();
  }

  function createDockTrace(runId) {
    prepareDockChat();
    const wrap = element('div', 'sim-ai-progress');
    const head = element('div', 'sim-ai-progress-head');
    head.appendChild(element('span', '', '可验证分析流程 · 阶段概述'));
    const actions = element('span', 'agent-card-actions');
    actions.appendChild(element('span', 'sim-ai-progress-run-id', runId || '准备中'));
    actions.appendChild(cardCopyButton('复制思维链', () => traceCardText(wrap, true), true));
    head.appendChild(actions);
    const note = element('p', 'sim-ai-progress-note', '显示动作、证据与校验状态，不显示隐藏推理。');
    const steps = element('div', 'sim-ai-progress-steps');
    wrap.append(head, note, steps);
    els.dockChat.appendChild(wrap);
    scrollDock();
    return { wrap, steps, seen: new Set(), openTools: new Map(), openModel: null, activeStep: null };
  }

  function appendDockTraceStep(trace, event) {
    if (!trace) return;
    const kind = event.trace_kind || event.kind;
    const key = `${event.sequence ?? ''}:${kind}:${event.tool_name || ''}`;
    if (trace.seen?.has(key)) return;
    trace.seen?.add(key);
    if (kind === 'tool_finished') {
      const queue = trace.openTools.get(event.tool_name) || [];
      const step = queue.shift();
      if (step) {
        step.classList.remove('is-running');
        step.removeAttribute('aria-current');
        step.classList.add(isTraceWarning(event) ? 'is-warning' : 'is-done');
        const label = step.querySelector('b');
        const overview = step.querySelector('.sim-ai-progress-overview');
        const meta = step.querySelector('small');
        if (label) label.textContent = event.label || `${event.code ? '工具返回受限' : '取得证据'} · ${toolLabel(event.tool_name)}`;
        if (overview) overview.textContent = traceStageOverview(event);
        const nextMeta = traceStepMeta(event);
        if (nextMeta && meta) meta.textContent = nextMeta;
        else if (nextMeta) step.querySelector('.sim-ai-progress-copy')?.appendChild(element('small', '', nextMeta));
        if (trace.activeStep === step) trace.activeStep = null;
        scrollDock();
        return;
      }
    }
    if (kind === 'model_finished' && trace.openModel) {
      const step = trace.openModel;
      step.classList.remove('is-running');
      step.removeAttribute('aria-current');
      step.classList.add('is-done');
      const label = step.querySelector('b');
      const overview = step.querySelector('.sim-ai-progress-overview');
      const meta = step.querySelector('small');
      if (label) label.textContent = event.label || traceLabels.model_finished;
      if (overview) overview.textContent = traceStageOverview(event);
      if (meta) meta.remove();
      trace.openModel = null;
      if (trace.activeStep === step) trace.activeStep = null;
      scrollDock();
      return;
    }
    finishActiveTraceStep(trace);
    const suffix = event.tool_name ? ` · ${toolLabel(event.tool_name)}` : '';
    const label = event.label || (kind === 'tool_started' ? `调用工具${suffix}` : `${traceLabels[kind] || kind}${suffix}`);
    const step = createTraceStep('sim-ai-progress-step', label, traceStageOverview(event), traceStepMeta(event), true);
    const terminal = ['completed', 'partially_verified', 'needs_user_input', 'refused', 'cancelled', 'evidence_insufficient',
      'budget_exhausted', 'provider_failed', 'protocol_failed', 'timed_out'].includes(kind);
    const active = !terminal && !['tool_finished', 'model_finished'].includes(kind);
    step.classList.add(terminal ? (isTraceWarning(event) ? 'is-warning' : 'is-done') : active ? 'is-running' : 'is-done');
    if (active) step.setAttribute('aria-current', 'step');
    if (kind === 'tool_started') {
      const queue = trace.openTools.get(event.tool_name) || [];
      queue.push(step);
      trace.openTools.set(event.tool_name, queue);
      trace.activeStep = step;
    }
    if (kind === 'model_started') {
      trace.openModel = step;
      trace.activeStep = step;
    } else if (active && kind !== 'tool_started') {
      trace.activeStep = step;
    }
    if (event.code) step.title = `诊断码：${event.code}`;
    trace.steps.appendChild(step);
    scrollDock();
  }

  function renderDockReport(result) {
    if (!result) return;
    noteRenderedRun(result);
    prepareDockChat();
    const report = result.report;
    const allMetrics = (report?.content?.findings || []).flatMap(finding => finding.metrics || []);
    const card = element('article', 'sim-ai-result');
    const evidenceInsufficient = result.status === 'evidence_insufficient';
    const partiallyVerified = result.status === 'partially_verified';
    card.classList.toggle('is-limited', evidenceInsufficient || partiallyVerified);
    const head = element('div', 'sim-ai-result-head');
    head.appendChild(element('b', '', statusLabels[result.status] || result.status || '分析结果'));
    const actions = element('span', 'agent-card-actions');
    actions.appendChild(element('span', '', `${result.provider_profile || '—'} · ${result.accounting?.duration_ms || 0}ms`));
    actions.appendChild(cardCopyButton('复制结论', () => buildSummary(result), true));
    actions.appendChild(cardCopyButton('复制调试', () => buildDebugBundle(result), true));
    head.appendChild(actions);
    card.appendChild(head);

    if (result.clarification) {
      if (result.clarification.analysis_text) {
        const progress = element('div', 'agent-clarification-analysis');
        progress.appendChild(element('small', '', '当前分析 · 尚未完成报告校验'));
        appendRichProse(progress, 'p', '', result.clarification.analysis_text);
        card.appendChild(progress);
      }
      const question = element('div', 'sim-ai-result-summary', result.clarification.question);
      card.appendChild(question);
      if (result.clarification.reason) {
        card.appendChild(element('p', 'sim-ai-clarification-reason', result.clarification.reason));
      }
      appendClarificationChoices(card, result, true);
      appendDiagnostics(card, result, true);
      els.dockChat.appendChild(card);
      scrollDock();
      setTimeout(() => {
        els.dockQuestion.placeholder = '回答上面的问题，继续当前会话…';
        els.dockQuestion.focus();
      }, 0);
      return;
    }

    if (evidenceInsufficient) {
      const notice = element('div', 'sim-ai-result-notice');
      notice.appendChild(element('b', '', '这不是系统故障'));
      notice.appendChild(element('span', '', '模型输出中的数值或引用未通过证据校验，因此没有发布为结论。'));
      if (result.error?.code) notice.title = `校验码：${result.error.code}`;
      card.appendChild(notice);
    } else if (partiallyVerified) {
      const notice = element('div', 'sim-ai-result-notice');
      notice.appendChild(element('b', '', '已保留可信部分'));
      notice.appendChild(element('span', '', '个别表述未通过数值校验，不影响下方已验证结论。'));
      if (result.error?.code) notice.title = `首个校验码：${result.error.code}`;
      card.appendChild(notice);
    }

    const summary = report?.content?.summary
      ? readableProse(report.content.summary, allMetrics)
      : result?.status === 'provider_failed'
        ? providerErrorText(result?.error)
        : result.error?.message || '本次任务没有生成可展示的结论。';
    appendRichProse(card, 'div', 'sim-ai-result-summary', summary, allMetrics);
    appendEquipmentComparisons(card, report?.equipment_comparisons || [], true);
    (report?.content?.findings || []).slice(0, 4).forEach(finding => {
      const block = element('div', 'sim-ai-result-finding');
      appendRichProse(block, 'b', '', finding.title, finding.metrics);
      appendRichProse(block, 'p', '', finding.explanation, finding.metrics);
      appendMetricGrid(block, (finding.metrics || []).slice(0, 4), 'sim-ai-result-metrics');
      card.appendChild(block);
    });
    appendRotationChanges(card, report?.content?.rotation_changes || [], allMetrics, true);
    appendDraftArtifacts(card, report?.content?.artifacts || [], true);
    appendKnowledgeSources(card, report?.sources || [], true);
    const limitations = report?.content?.limitations || [];
    if (limitations.length) {
      const block = element('details', 'sim-ai-result-boundaries');
      block.appendChild(element('summary', '', `分析边界（${limitations.length}）`));
      limitations.slice(0, 3).forEach(item => block.appendChild(element('p', '', readableProse(item, allMetrics)
        .replaceAll('simulate_scenario', '基线模拟')
        .replaceAll('get_current_scenario', '当前场景读取')
        .replaceAll('engine_commit_unavailable', '引擎提交信息缺失')
        .replaceAll('engine_string', '引擎版本'))));
      card.appendChild(block);
    }
    appendDiagnostics(card, result, true);
    els.dockChat.appendChild(card);
    scrollDock();
  }

  const simulationQuestions = [
    ['循环诊断', '这套循环做得好的地方和最主要的问题是什么？'],
    ['攻略解读', '结合当前版本攻略，解释这套循环的核心思路。'],
    ['蒸馏成宏', '把当前循环蒸馏成宏，调优并实测，给我最终版本和与原循环的差异。'],
    ['对比已存宏', '我想对比已保存的宏，先列出可选方案。'],
  ];

  function appendSimulationStarters(container) {
    simulationQuestions.forEach(([label, question]) => {
      const button = element('button', 'sim-btn', label);
      button.type = 'button';
      button.addEventListener('click', () => setPrompt(els.question, question));
      container.appendChild(button);
    });
  }

  function renderWelcome() {
    clear(els.transcript);
    const welcome = element('div', 'agent-welcome');
    welcome.appendChild(element('div', 'agent-welcome-mark', '✦'));
    welcome.appendChild(element('h3', '', '从一个可验证的问题开始'));
    welcome.appendChild(element('p', '', '先在“循环模拟”准备场景，再问 Agent 当前攻略、输出基线、候选改动或时间轴异常。引用资料会标明赛季与原始来源。'));
    const starters = element('div', 'agent-starter-grid');
    appendSimulationStarters(starters);
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
      let feedbackTurn = null;
      body.events.forEach(event => {
        if (event.kind === 'user_message') {
          feedbackTurn = createFeedbackTurn(event.question, false);
          appendMessage('user', event.question, [feedbackCopyButton(feedbackTurn)]);
        } else if (event.kind === 'run_started') {
          trace = createTrace(event.run_id);
          traceRunId = event.run_id;
          attachFeedbackTrace(feedbackTurn, trace);
          appendTraceStep(trace, { sequence: event.sequence, kind: 'planning' });
        } else if (event.kind === 'run_trace') {
          if (!trace || traceRunId !== event.run_id) {
            trace = createTrace(event.run_id);
            traceRunId = event.run_id;
            attachFeedbackTrace(feedbackTurn, trace);
          }
          appendTraceStep(trace, event);
        } else if (event.kind === 'cancel_requested' || event.kind === 'run_interrupted') {
          if (!trace || traceRunId !== event.run_id) {
            trace = createTrace(event.run_id);
            attachFeedbackTrace(feedbackTurn, trace);
          }
          appendTraceStep(trace, { ...event, trace_kind: event.kind });
        } else if (event.kind === 'run_result') {
          const result = event.result || { status: 'finished', scenario_hash: event.scenario_hash };
          completeFeedbackTurn(feedbackTurn, result);
          renderReport(result);
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

  function toggleDockHistory(open) {
    if (!els.dockHistoryPanel) return;
    const next = open == null ? !els.dockHistoryPanel.classList.contains('open') : !!open;
    els.dockHistoryPanel.classList.toggle('open', next);
    els.dockHistoryPanel.setAttribute('aria-hidden', String(!next));
    els.dockHistory?.setAttribute('aria-expanded', String(next));
    if (next) loadSessions();
  }

  async function openDockSession(sessionId) {
    if (activeRun) return;
    activeSurface = 'dock';
    try {
      setStatus('正在加载历史会话…');
      const response = await fetch(`/api/agent/sessions/${encodeURIComponent(sessionId)}`, { cache: 'no-store' });
      const body = await safeJson(response);
      if (!response.ok) throw new Error(body?.error?.message || '会话读取失败');
      currentSessionId = sessionId;
      clearDockChat();
      let trace = null;
      let traceRunId = null;
      let feedbackTurn = null;
      body.events.forEach(event => {
        if (event.kind === 'user_message') {
          feedbackTurn = createFeedbackTurn(event.question, true);
          appendDockBubble('user', event.question, false, feedbackTurn);
        } else if (event.kind === 'run_started') {
          trace = createDockTrace(event.run_id);
          traceRunId = event.run_id;
          attachFeedbackTrace(feedbackTurn, trace);
          appendDockTraceStep(trace, { sequence: event.sequence, kind: 'planning' });
        } else if (event.kind === 'run_trace') {
          if (!trace || traceRunId !== event.run_id) {
            trace = createDockTrace(event.run_id);
            traceRunId = event.run_id;
            attachFeedbackTrace(feedbackTurn, trace);
          }
          appendDockTraceStep(trace, event);
        } else if (event.kind === 'cancel_requested' || event.kind === 'run_interrupted') {
          if (!trace || traceRunId !== event.run_id) {
            trace = createDockTrace(event.run_id);
            attachFeedbackTrace(feedbackTurn, trace);
          }
          appendDockTraceStep(trace, { ...event, trace_kind: event.kind });
        } else if (event.kind === 'run_result') {
          const result = event.result || { status: 'finished', scenario_hash: event.scenario_hash };
          completeFeedbackTurn(feedbackTurn, result);
          renderDockReport(result);
        }
      });
      if (body.summary.corrupted_event_count) {
        appendDockBubble('agent', `检测到 ${body.summary.corrupted_event_count} 个损坏事件。原文件已保留，请新建会话继续分析。`, true);
      }
      if (!body.events.length) clearDockChat();
      const title = body.summary.title || sessionSummaries.find(session => session.session_id === sessionId)?.title;
      els.dockSession.textContent = `续接 · ${title || `${sessionId.slice(0, 18)}…`}`;
      toggleDockHistory(false);
      setStatus(`已恢复会话 · ${statusLabels[body.summary.status] || body.summary.status}`);
      await loadSessions();
    } catch (error) {
      setStatus(error.message || '会话读取失败', true);
    }
  }

  async function captureScenario(surface = 'simulation') {
    if (surface === 'equipment') {
      const captured = await window.Jx3Equip?.captureAgentContext?.();
      if (!captured?.simulation) throw new Error('无法读取当前配装。请先完成配装并选择有效的 DPS 来源。');
      updateScenarioState();
      return captured;
    }
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
    return { simulation: scenario, equipment_workspace: null };
  }

  function setDockOpen(open, focusInput) {
    if (!els.dock) return;
    els.dock.classList.toggle('open', open);
    els.dock.setAttribute('aria-hidden', String(!open));
    els.dockFab.setAttribute('aria-expanded', String(open));
    try { localStorage.setItem('sim_ai_dock_open', open ? '1' : '0'); } catch (_) {}
    if (open) {
      updateScenarioState();
      loadSessions();
      if (focusInput) setTimeout(() => els.dockQuestion.focus(), 80);
    } else {
      toggleDockHistory(false);
      els.dockFab.focus({ preventScroll: true });
    }
  }

  async function startDockRun(answerOverride) {
    if (activeRun || composerBusy) return;
    const question = typeof answerOverride === 'string' ? answerOverride : els.dockQuestion.value.trim();
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
      const captured = await captureScenario(dockMode);
      dockFeedbackTurn = createFeedbackTurn(question, true);
      appendDockBubble('user', question, false, dockFeedbackTurn);
      dockTrace = createDockTrace('准备创建 run');
      attachFeedbackTrace(dockFeedbackTurn, dockTrace);
      dockThinking = createThinking(els.dockChat);
      const payload = {
        question,
        provider_profile: els.dockProvider.value,
        analysis_surface: dockMode,
        simulation: captured.simulation,
        equipment_workspace: captured.equipment_workspace || undefined,
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
      dockTrace.wrap.querySelector('.sim-ai-progress-run-id').textContent = body.run_id;
      latestRunBySession.set(currentSessionId, body.run_id);
      updateClarificationControls();
      if (typeof answerOverride !== 'string') els.dockQuestion.value = '';
      setStatus(`运行中 · 场景 ${body.scenario_hash.slice(0, 12)}…`);
      connectDockStream(body.stream_url);
      await loadSessions();
    } catch (error) {
      setBusy(false);
      activeRun = null;
      dockTrace = null;
      dockFeedbackTurn = null;
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
          completeFeedbackTurn(dockFeedbackTurn, event.result);
          renderDockReport(event.result);
          const persistenceError = !!event.persistence_error;
          setTerminalStatus(event.result, persistenceError, false);
          activeRun = null;
          dockTrace = null;
          dockFeedbackTurn = null;
          setBusy(false);
          loadSessions();
        } else {
          appendDockTraceStep(dockTrace, event);
          updateThinking(dockThinking, event);
          const label = traceLabels[kind] || kind;
          setStatus(`${label}${event.tool_name ? ` · ${toolLabel(event.tool_name)}` : ''}`);
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
        completeFeedbackTurn(dockFeedbackTurn, body.result);
        renderDockReport(body.result);
        setTerminalStatus(body.result, !!body.persistence_error, true);
        activeRun = null;
        dockTrace = null;
        dockFeedbackTurn = null;
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
      dockFeedbackTurn = null;
      setBusy(false);
    }
  }

  async function startRun(answerOverride) {
    if (activeRun || composerBusy) return;
    activeSurface = 'full';
    const question = typeof answerOverride === 'string' ? answerOverride : els.question.value.trim();
    if (!question) { setStatus('请先输入一个策划问题', true); els.question.focus(); return; }
    setBusy(true);
    setStatus('正在冻结当前场景…');
    try {
      const captured = await captureScenario('simulation');
      activeFeedbackTurn = createFeedbackTurn(question, false);
      appendMessage('user', question, [feedbackCopyButton(activeFeedbackTurn)]);
      activeTrace = createTrace('准备创建 run');
      attachFeedbackTrace(activeFeedbackTurn, activeTrace);
      activeThinking = createThinking(els.transcript);
      const payload = {
        question,
        provider_profile: els.provider.value,
        analysis_surface: 'agent',
        simulation: captured.simulation,
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
      activeTrace.wrap.querySelector('.agent-trace-run-id').textContent = body.run_id;
      latestRunBySession.set(currentSessionId, body.run_id);
      updateClarificationControls();
      if (typeof answerOverride !== 'string') els.question.value = '';
      setStatus(`运行中 · ${body.run_id} · 场景 ${body.scenario_hash.slice(0, 12)}…`);
      connectStream(body.stream_url);
      await loadSessions();
    } catch (error) {
      setBusy(false);
      activeRun = null;
      activeTrace = null;
      activeFeedbackTurn = null;
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
          completeFeedbackTurn(activeFeedbackTurn, event.result);
          renderReport(event.result);
          const persistenceError = !!event.persistence_error;
          setTerminalStatus(event.result, persistenceError, false);
          activeRun = null;
          activeTrace = null;
          activeFeedbackTurn = null;
          setBusy(false);
          loadSessions();
        } else {
          appendTraceStep(activeTrace, event);
          updateThinking(activeThinking, event);
          const label = traceLabels[kind] || kind;
          setStatus(`${label}${event.tool_name ? ` · ${toolLabel(event.tool_name)}` : ''}`);
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
        completeFeedbackTurn(activeFeedbackTurn, body.result);
        renderReport(body.result);
        setTerminalStatus(body.result, !!body.persistence_error, true);
        activeRun = null;
        activeTrace = null;
        activeFeedbackTurn = null;
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
      activeFeedbackTurn = null;
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

  function buildSummary(result) {
    const report = result.report;
    const lines = [
      '# 苍云战斗分析 Agent 实验摘要',
      `- status: ${result.status}`,
      `- provider/model: ${result.provider_profile} / ${result.model}`,
      `- scenario: ${result.scenario_hash}`,
      `- prompt: ${result.prompt_version} / ${result.prompt_sha256}`,
      `- tools/simulations: ${result.accounting?.tool_calls || 0} / ${result.accounting?.simulations || 0}`,
      `- knowledge searches: ${result.accounting?.knowledge_searches || 0}`,
      `- evidence: ${(report?.evidence_ids || []).join(', ') || 'none'}`,
      '',
      plainAgentText(report?.content?.summary || result.clarification?.question || result.error?.message || 'No report'),
    ];
    if (result.clarification) {
      lines.push('', '## 需要用户补充', result.clarification.reason || '');
      if (result.clarification.analysis_text) lines.push('', '## 当前分析（尚未完成报告校验）', plainAgentText(result.clarification.analysis_text));
      clarificationOptions(result.clarification).forEach((option, index) => lines.push(`${index + 1}. ${option.label}${option.description ? `：${option.description}` : ''}`));
      if (result.clarification.answer_hint) lines.push(`- 回答提示：${result.clarification.answer_hint}`);
    }
    if (report?.content?.findings?.length) {
      lines.push('', '## 分析结论');
      report.content.findings.forEach(finding => {
        lines.push('', `### ${plainAgentText(finding.title || '分析')}`, plainAgentText(finding.explanation || ''));
        (finding.metrics || []).forEach(metric => {
          lines.push(`- ${metricLabel(metric)}：${metricValue(metric)}`);
        });
      });
    }
    if (report?.content?.recommendations?.length) {
      lines.push('', '## 下一步建议');
      report.content.recommendations.forEach(recommendation => {
        lines.push(`- **${plainAgentText(recommendation.title || '建议')}**：${plainAgentText(recommendation.rationale || '')}`);
      });
    }
    if (report?.content?.rotation_changes?.length) {
      lines.push('', '## 循环修改方案');
      report.content.rotation_changes.forEach(change => {
        const operationLabels = { replace: '替换', insert_before: '在前插入', insert_after: '在后插入', adjust_timing: '调整时序' };
        lines.push('', `### ${change.change_type === 'macro_statement' ? '宏语句' : '手动操作点'} · ${operationLabels[change.edit_operation] || '修改'} · ${change.target || '当前循环'}`);
        lines.push(`- 当前：\`${change.current || '—'}\``);
        lines.push(`- 修改：\`${change.proposed || '—'}\``);
        lines.push(`- 依据：${plainAgentText(change.rationale || '')}`);
      });
    }
    if (report?.content?.artifacts?.length) {
      report.content.artifacts.forEach(artifact => {
        const fence = '`'.repeat(Math.max(3, ...Array.from((artifact.content || '').matchAll(/`+/g), match => match[0].length + 1)));
        lines.push('', `## ${artifact.title || '候选草稿'}`, '候选草稿；复刻程度与实测结果见本轮说明。', '', fence, artifact.content || '', fence);
      });
    }
    if (report?.content?.limitations?.length) {
      lines.push('', '## 证据边界');
      report.content.limitations.forEach(limitation => lines.push(`- ${limitation}`));
    }
    if (report?.sources?.length) {
      lines.push('', '## 参考资料');
      report.sources.forEach(source => {
        const href = safeExternalUrl(source.source_url || source.yuque_url);
        if (href) lines.push(`- [${source.title || '未命名资料'}](${href}) · ${source.season || '版本未标注'} · ${versionLabels[source.version_match] || source.version_match || '匹配状态未知'}`);
      });
    }
    return lines.join('\n');
  }

  function buildDebugBundle(result) {
    const debug = result?.debug || {};
    const legacyPlaybook = Array.isArray(result?.trace)
      ? result.trace.find(event => event?.playbook_id)?.playbook_id || null
      : null;
    const plan = debug.analysis_plan || (legacyPlaybook ? { playbook_id: legacyPlaybook } : null);
    const safeReport = result?.report
      ? { ...result.report, question: debug.question || result.report.question }
      : null;
    const bundle = {
      schema_version: 'agent-debug-export/v1',
      security: {
        redacted: true,
        excluded: ['api_key', 'provider_raw_response', 'hidden_reasoning', 'private_path'],
        note: '这是可分享的工程调试投影，不包含供应商密钥、原始响应、隐藏推理或服务端私有路径。',
      },
      identity: {
        session_id: result?.session_id || null,
        run_id: result?.run_id || null,
        question: debug.question || safeReport?.question || null,
        status: result?.status || null,
        scenario_hash: result?.scenario_hash || null,
        prompt_version: result?.prompt_version || null,
        prompt_sha256: result?.prompt_sha256 || null,
        provider_profile: result?.provider_profile || null,
        model: result?.model || null,
      },
      routing: plan,
      exposed_tools: debug.exposed_tools || null,
      limits: debug.limits || null,
      request_metrics: debug.request_metrics || null,
      tool_calls: debug.tool_calls || null,
      evidence_pack: debug.evidence_pack || null,
      evidence_projection: debug.evidence_projection || null,
      diagnostic_state: debug.diagnostic_state || null,
      accounting: result?.accounting || null,
      terminal_diagnostic: {
        stage: diagnosticStage(result),
        hint: diagnosticHint(result),
        error: result?.error || null,
      },
      trace: result?.trace || null,
      clarification: result?.clarification || null,
      report: safeReport,
      compatibility: result?.debug
        ? 'full'
        : 'legacy_run: this historical run predates the full debug projection',
    };
    return JSON.stringify(bundle, null, 2);
  }

  function buildFeedbackBundle(turn) {
    if (!turn?.result) return '';
    const trace = turn.traceWrap
      ? traceCardText(turn.traceWrap, turn.compact)
      : '# 可验证分析流程 · 阶段概述\n- 本轮没有可用的公开阶段记录';
    return [
      '# 苍云战斗分析 Agent · 反馈包',
      '',
      '## 用户问题',
      turn.question || turn.result?.debug?.question || '—',
      '',
      trace,
      '',
      '## Agent 回复',
      buildSummary(turn.result),
      '',
      '## 完整脱敏调试信息',
      '```json',
      buildDebugBundle(turn.result),
      '```',
    ].join('\n');
  }

  function newSession() {
    if (activeRun) return;
    toggleDockHistory(false);
    currentSessionId = null;
    if (els.dockSession) els.dockSession.textContent = '新对话';
    renderWelcome();
    if (els.dockChat) clearDockChat();
    setStatus('新会话 · 下一次运行时创建持久记录');
    loadSessions();
  }

  const dockQuestions = {
    simulation: simulationQuestions,
    equipment: [
      ['当前配装', '我当前配装怎么样？哪些属性、套装和特效最影响这套循环？', 'equipment_analysis'],
      ['单件替换', '把我标记的候选装备换上会怎样？对比面板和当前循环表现。', 'equipment_analysis'],
      ['套装 vs 切糕', '我的当前条件下，穿四件套还是四切糕更好？', 'equipment_analysis'],
      ['属性短板', '当前配装最该补哪项属性？为什么？', 'equipment_analysis'],
      ['加速档位', '当前加速档适合这套武器、循环和网络延迟吗？', 'haste_decision'],
      ['套装效果', '当前激活了哪些套装效果？它们怎样影响技能和循环？', 'equipment_analysis'],
      ['保存配装', '我保存了哪些可以和当前配装比较的方案？', 'saved_artifact_analysis'],
      ['替换优先级', '当前最值得优先更换哪个装备部位？', 'equipment_analysis'],
    ],
  };

  function syncDockMode() {
    const next = els.equipPage?.classList.contains('active') ? 'equipment'
      : els.simPage?.classList.contains('active') ? 'simulation' : null;
    if (!next) {
      if (els.dockFab) els.dockFab.hidden = true;
      if (els.dock?.classList.contains('open') && !activeRun) setDockOpen(false);
      return;
    }
    if (els.dockFab) els.dockFab.hidden = false;
    const changed = dockMode !== next;
    dockMode = next;
    if (els.dockTitle) els.dockTitle.textContent = next === 'equipment' ? '配装分析助手' : '循环分析助手';
    if (els.dockFabContext) els.dockFabContext.textContent = next === 'equipment' ? '基于当前配装' : '基于当前循环';
    if (els.dockQuestion) els.dockQuestion.placeholder = next === 'equipment'
      ? '问装备替换、套装取舍、属性和当前循环 DPS…'
      : '问当前攻略、循环基线、时间轴或候选改动…';
    if (els.dockQuick) {
      els.dockQuick.innerHTML = dockQuestions[next].map(([label, question]) =>
        `<button type="button" class="sim-btn" data-agent-dock-question="${question.replaceAll('&', '&amp;').replaceAll('"', '&quot;')}">${label}</button>`
      ).join('');
    }
    updateScenarioState();
    if (changed && !activeRun) clearDockChat();
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
  els.newSession.addEventListener('click', newSession);
  els.goSim.addEventListener('click', () => window.Jx3Nav?.switchPage('page-sim'));
  els.dockFab?.addEventListener('click', () => setDockOpen(true, true));
  els.dockClose?.addEventListener('click', () => setDockOpen(false));
  els.dockHistory?.addEventListener('click', () => {
    activeSurface = 'dock';
    toggleDockHistory();
  });
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
  els.dockQuestion?.addEventListener('keydown', event => {
    if (event.key === 'Enter' && !event.shiftKey && !event.isComposing) {
      event.preventDefault();
      startDockRun();
    }
  });
  els.dockQuick?.addEventListener('click', event => {
    const button = event.target.closest('[data-agent-dock-question], [data-sim-ai-question]');
    if (!button) return;
    setDockOpen(true);
    setPrompt(
      els.dockQuestion,
      button.dataset.agentDockQuestion || button.dataset.simAiQuestion || ''
    );
  });
  window.addEventListener('jx3-equip-ai-focus', event => {
    syncDockMode();
    const focus = event.detail || {};
    setDockOpen(true);
    setPrompt(
      els.dockQuestion,
      `把当前${focus.position || '部位'}的“${focus.current_name || '当前装备'}”换成“${focus.candidate_name || '候选装备'}”怎么样？展示换前换后面板，并用当前循环实测 DPS 和伤害构成。`
    );
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
  const initialStarters = els.transcript.querySelector('.agent-starter-grid');
  if (initialStarters) appendSimulationStarters(initialStarters);
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
  [els.simPage, els.equipPage].filter(Boolean).forEach(workspacePage => {
    new MutationObserver(syncDockMode).observe(workspacePage, { attributes: true, attributeFilter: ['class'] });
  });
  syncAgentPageLayout();
  syncDockMode();
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
