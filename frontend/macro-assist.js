/* 编辑器写宏助手。分析当前完整模拟的释放前快照，候选条件由后端确定性计算。 */
(function () {
  'use strict';
  const sequence = document.getElementById('sim_sequence');
  const panel = document.getElementById('macro_assist_panel');
  const modeButton = document.getElementById('macro_assist_toggle');
  const editButton = document.getElementById('macro_assist_edit');
  const selectionPreview = document.getElementById('macro_assist_selection');
  if (!sequence || !panel || !modeButton || !editButton || !selectionPreview) return;
  const status = document.getElementById('macro_assist_status');
  const content = document.getElementById('macro_assist_content');
  let active = false, busy = false, revision = 0, abort = null;
  let sourceResult = null, sourceKey = '', selected = null, response = null, programResponse = null;
  let step = 0, filter = 'all', search = '', shown = 30;
  let selectionPattern = [], selectionAnchors = [], pendingSelectionItems = null;
  let updateTimer = null, updatePending = false, responseKey = '', pendingAnalysisKey = '';
  const runningSimulations = new Map();
  const disclosureState = new Map();
  let outlineFrame = null;

  const el = (tag, text, className) => {
    const node = document.createElement(tag);
    if (text != null) node.textContent = text;
    if (className) node.className = className;
    return node;
  };
  const button = (text, action, title) => {
    const node = el('button', text, 'ma-button');
    node.type = 'button';
    if (title) node.title = title;
    node.addEventListener('click', action);
    return node;
  };
  const pct = value => Number.isFinite(value) ? `${(value * 100).toFixed(1).replace(/\.0$/, '')}%` : '—';
  const seconds = value => Number.isFinite(value) ? `${value.toFixed(1)}s` : '—';
  const activeEvents = () => (sourceResult?.timeline || []).filter(event => !event.triggered);
  const eventName = event => event?.skill_id >= 90010 && event?.skill_id <= 90012
    ? event.name : (event?.name || '').split('·')[0];
  const analysisKey = () => JSON.stringify([sourceKey, selected, selectionPattern.length > 1 ? 'program' : step]);
  function rememberDisclosures(parent = content) {
    parent.querySelectorAll('details[data-ma-disclosure]').forEach(node => disclosureState.set(node.dataset.maDisclosure, node.open));
  }
  function disclosure(className, name) {
    const node = el('details', null, className);
    const key = JSON.stringify([selectionPattern, name.startsWith('program-') ? null : step, name]);
    node.dataset.maDisclosure = key;
    node.open = disclosureState.get(key) === true;
    node.addEventListener('toggle', () => { if (node.isConnected) disclosureState.set(key, node.open); });
    return node;
  }

  // 与完整手动模拟请求相同的环境维度；序列附加参数直接读 DOM，避免使用上次模拟的索引缓存。
  function contextKey() {
    const inputItems = [...sequence.querySelectorAll('.sim-seq-item:not(.seq-pre-release):not(.seq-auto)')];
    return JSON.stringify({
      version: currentMount.version, mount: currentMount.mount,
      sequence: inputItems.map(item => item.dataset.skill === '__clearCD__' && item.dataset.clearcdTarget
        ? `__clearCD__:${item.dataset.clearcdTarget}` : item.dataset.skill || item.querySelector('.seq-label')?.textContent.trim()),
      macro_text: buildMacroText(),
      haste_level: getSimHasteLevel(), talents: getSelectedTalents(), recipes: getSelectedRecipes(),
      channel_ticks: channelOverrides,
      sequence_options: inputItems.map(item => [item.dataset.timingOffset || '', item.dataset.qijinBuff || '']),
      network_delay: parseInt(document.getElementById('sim_delay').value) || 0,
      attributes: getSimAttrs(), target: getTarget(), initial_rage: adminInitialRage,
      ...getDunyaResetOptions(), boss_attack_interval: getBossAttackInterval(), hanjia_expectation: isHanjiaExpectationEnabled(),
      tiegu_mode: getTieguMode(), experimental: isExperimental(), equipment: getEquipmentMap(),
      team_buffs: getTeamBuffs(), formation: getCurrentFormation(), pre_releases: getPreReleases(),
    });
  }

  function clearMarks() {
    if (outlineFrame != null) cancelAnimationFrame(outlineFrame);
    outlineFrame = null;
    clearSelectionOutlines();
    sequence.querySelectorAll('.ma-selected, .ma-occurrence, .ma-focus, .ma-extra')
      .forEach(node => node.classList.remove('ma-selected', 'ma-occurrence', 'ma-focus', 'ma-extra'));
  }
  function clearSelectionOutlines() {
    sequence.querySelectorAll('.ma-selection-outline').forEach(node => node.remove());
  }
  function redrawSelectionOutlines() {
    if (outlineFrame != null) cancelAnimationFrame(outlineFrame);
    outlineFrame = null;
    clearSelectionOutlines();
    if (!active) return;
    const container = sequence.getBoundingClientRect(), segments = [];
    let segment = null;
    for (const item of sequence.children) {
      if (item.matches('.seq-break-wrap, .seq-break')) { segment = null; continue; }
      if (!item.classList.contains('sim-seq-item')) continue;
      const kind = ['extra', 'focus', 'selected', 'occurrence'].find(value => item.classList.contains(`ma-${value}`));
      const bounds = item.getBoundingClientRect();
      if (!kind || bounds.width <= 0 || bounds.height <= 0) { segment = null; continue; }
      if (segment && segment.kind === kind && Math.abs(bounds.top - segment.rowTop) <= 1) {
        segment.left = Math.min(segment.left, bounds.left); segment.right = Math.max(segment.right, bounds.right);
        segment.top = Math.min(segment.top, bounds.top); segment.bottom = Math.max(segment.bottom, bounds.bottom);
      } else {
        segment = { kind, rowTop: bounds.top, left: bounds.left, right: bounds.right, top: bounds.top, bottom: bounds.bottom };
        segments.push(segment);
      }
    }
    const fragment = document.createDocumentFragment();
    for (const bounds of segments) {
      const outline = el('span', null, `ma-selection-outline ma-selection-${bounds.kind}`);
      outline.setAttribute('aria-hidden', 'true');
      // 仅显示装饰，不进入技能序列、框选或 flex 布局；坐标相对容器的 padding 边界。
      outline.style.position = 'absolute'; outline.style.pointerEvents = 'none'; outline.style.boxSizing = 'border-box';
      outline.style.left = `${bounds.left - container.left - sequence.clientLeft + sequence.scrollLeft}px`;
      outline.style.top = `${bounds.top - container.top - sequence.clientTop + sequence.scrollTop}px`;
      outline.style.width = `${bounds.right - bounds.left}px`; outline.style.height = `${bounds.bottom - bounds.top}px`;
      fragment.append(outline);
    }
    // 放在序列头部，保留末项/末尾换行的 sibling 语义，避免干扰既有撤销和盾回自动换行。
    sequence.prepend(fragment);
  }
  function scheduleSelectionOutlines() {
    if (!active) { clearSelectionOutlines(); return; }
    if (outlineFrame == null) outlineFrame = requestAnimationFrame(redrawSelectionOutlines);
  }
  function message(text) { status.textContent = text; }
  function cancelAnalysis() { revision++; abort?.abort(); abort = null; pendingAnalysisKey = ''; }
  function isFresh() {
    try { return sourceResult && sourceResult === lastSimResult && sourceKey === contextKey(); }
    catch { return false; }
  }
  function requireFresh() {
    if (isFresh()) return true;
    cancelAnalysis(); response = null; programResponse = null;
    showPending('配置已变化，正在自动更新所选组合…');
    scheduleUpdate();
    return false;
  }

  function showSelection(names = selectionPattern) {
    selectionPreview.textContent = names.length ? `已选：${names.join(' → ')}` : '尚未选择技能';
    if (names.length > 1) selectionPreview.textContent += ` · ${names.length} 步组合`;
  }
  function showPending(text) {
    rememberDisclosures();
    showSelection();
    message(selectionPattern.length ? `已选：${selectionPattern.join(' → ')} · ${text}` : text);
    content.replaceChildren(el('p', text, 'ma-empty'));
  }
  function showError(text) {
    content.replaceChildren(el('p', text, 'ma-empty'), button('重试', () => scheduleUpdate(), '保留所选组合，再次取得状态和分析结果。'));
    message('分析未完成，选择已保留');
  }
  function scheduleUpdate() {
    if (!active) return;
    clearTimeout(updateTimer);
    // Leave a paint opportunity after immediate selection feedback; coalesce input changes.
    updateTimer = setTimeout(update, 120);
  }

  async function update() {
    updateTimer = null;
    if (!active) return;
    if (busy) { updatePending = true; return; }
    const key = contextKey();
    if (runningSimulations.has(key)) return; // Adopt the editor's in-flight result when it completes.
    if (!pendingSelectionItems && isFresh() && response && responseKey === analysisKey()) {
      paintMarks(); return;
    }
    if (!pendingSelectionItems && isFresh() && pendingAnalysisKey === analysisKey()) return;
    busy = true; updatePending = false;
    cancelAnalysis();
    const token = revision;
    try {
      let result = lastSimResult;
      if (!result || result._macroAssistKey !== key || !result.timeline?.some(event => !event.triggered && event.state_before)) {
        showPending('正在自动取得当前配置的完整状态…');
        result = await runSimulate();
      }
      if (!active || token !== revision) return;
      if (key !== contextKey()) { updatePending = true; return; }
      if (!result || result !== lastSimResult || !result.timeline?.length) {
        throw new Error('没有取得新的模拟结果。请确认序列中有可释放技能，且后端正常。');
      }
      sourceResult = result; sourceKey = key;
      if (pendingSelectionItems) {
        const items = pendingSelectionItems; pendingSelectionItems = null;
        if (!setSelectionFromItems(items)) return;
      } else if (!restoreSelection()) {
        showSelection();
        message(selectionPattern.length ? '当前序列中没有所选组合，请重新选择' : '单击技能，或拖动框选一段连续组合');
        content.replaceChildren(el('p', selectionPattern.length ? '保留的技能组合已不在当前序列中。请选择其它组合，或在编辑模式恢复原组合。'
          : '选择后自动分析。组合包含血怒等非主 GCD 技能；被动伤害不计入。', 'ma-empty'));
        return;
      }
      showSelection(); paintMarks();
      analyze();
    } catch (error) {
      if (active && token === revision) showError(error.message);
    } finally {
      busy = false;
      if (updatePending && active) scheduleUpdate();
    }
  }

  function setMode(value) {
    if (active === value) return;
    active = value; cancelAnalysis(); clearMarks();
    clearTimeout(updateTimer); updateTimer = null;
    document.body.classList.toggle('macro-assist-active', active);
    modeButton.setAttribute('aria-pressed', String(active));
    editButton.setAttribute('aria-pressed', String(!active));
    panel.hidden = !active;
    window.dispatchEvent(new Event('jx3-seq-selection-clear'));
    window.dispatchEvent(new CustomEvent('jx3-macro-assist-mode', { detail: { active } }));
    if (active) {
      hideTooltip(); showSelection();
      if (isFresh() && response && responseKey === analysisKey()) { paintMarks(); render(); }
      else { showPending('正在自动准备状态…'); scheduleUpdate(); }
    } else message(selectionPattern.length ? '已保留所选组合，切回写宏可继续分析' : '编辑序列，或切换到写宏分析技能条件');
  }

  function selectItems(items, options = {}) {
    if (!active) return;
    cancelAnalysis(); response = null; programResponse = null; clearMarks();
    if (!items.length) { clearPick(); return; }
    window.Jx3MacroLayout?.tab('conditions');
    // Acknowledge the actual DOM selection synchronously, before starting any request.
    selectionAnchors = [...items];
    const names = items.map(item => item.dataset.resolvedSkill || item.dataset.skill || item.querySelector('.seq-label')?.textContent || '技能');
    step = 0; filter = 'all'; search = ''; shown = 30;
    selectionPattern = names;
    items.forEach(item => item.classList.add('ma-selected'));
    redrawSelectionOutlines();
    pendingSelectionItems = [...items];
    showPending('正在分析…');
    scheduleUpdate();
  }

  function setSelectionFromItems(items) {
    const indices = items.map(item => {
      const raw = item.dataset.macroAssistIndex;
      return !sequence.contains(item) || raw == null || item.classList.contains('seq-pre-release') || item.classList.contains('seq-invalid')
        ? NaN : Number(raw);
    }).sort((a, b) => a - b);
    if (indices.some(index => !Number.isInteger(index) || !activeEvents()[index])) {
      selected = null; selectionPattern = []; selectionAnchors = [];
      clearMarks(); showSelection(); window.dispatchEvent(new Event('jx3-seq-selection-clear'));
      message('请选择成功释放的主动技能；预释放、等待和跳过项没有对应的释放前状态。');
      content.replaceChildren(el('p', '当前选择没有可分析的完整主动技能状态。', 'ma-empty'));
      return false;
    }
    if (indices.some((index, i) => i && index !== indices[i - 1] + 1)) {
      selected = null; selectionPattern = []; selectionAnchors = [];
      clearMarks(); showSelection(); window.dispatchEvent(new Event('jx3-seq-selection-clear'));
      message('框选结果中间有未选技能，请选择一段连续组合。');
      content.replaceChildren(el('p', '从第一个技能拖到最后一个技能，或从空白处框选完整连续组合。', 'ma-empty'));
      return false;
    }
    selected = [indices[0], indices[indices.length - 1]];
    selectionPattern = indices.map(index => eventName(activeEvents()[index]));
    selectionAnchors = nodesAt(indices);
    return true;
  }

  function restoreSelection() {
    if (!selectionPattern.length) { selected = null; return false; }
    const events = activeEvents();
    const matches = start => selectionPattern.every((name, offset) => eventName(events[start + offset]) === name);
    const anchors = selectionAnchors.filter(node => sequence.contains(node) && node.dataset.macroAssistIndex != null)
      .map(node => Number(node.dataset.macroAssistIndex)).sort((a, b) => a - b);
    let start = anchors.length === selectionPattern.length && anchors.every((index, offset) => index === anchors[0] + offset) && matches(anchors[0])
      ? anchors[0] : null;
    if (start == null) {
      const candidates = events.map((_, index) => index).filter(matches);
      candidates.sort((a, b) => Math.abs(a - (selected?.[0] || 0)) - Math.abs(b - (selected?.[0] || 0)));
      start = candidates[0];
    }
    if (start == null) { selected = null; response = null; programResponse = null; clearMarks(); return false; }
    selected = [start, start + selectionPattern.length - 1];
    selectionAnchors = nodesAt(selectionPattern.map((_, offset) => start + offset));
    step = Math.min(step, selectionPattern.length - 1);
    return true;
  }

  function clearPick() {
    cancelAnalysis(); selected = null; response = null; programResponse = null; clearMarks();
    selectionPattern = []; selectionAnchors = []; pendingSelectionItems = null;
    clearTimeout(updateTimer); updateTimer = null; updatePending = false;
    showSelection();
    window.dispatchEvent(new Event('jx3-seq-selection-clear'));
    content.replaceChildren(el('p', '请单击技能，或框选一段连续组合。', 'ma-empty'));
    message('单击技能，或拖动框选一段连续组合');
  }

  async function analyze() {
    if (!selected || !requireFresh()) return;
    cancelAnalysis();
    const token = revision;
    const controller = new AbortController();
    abort = controller;
    pendingAnalysisKey = analysisKey();
    const timeout = setTimeout(() => controller.abort(), 15000);
    showPending('正在比较组合内外的释放状态…');
    try {
      const isProgram = selected[1] > selected[0];
      const res = await fetch(isProgram ? '/api/macro/assist/program' : '/api/macro/assist', {
        method: 'POST', headers: { 'Content-Type': 'application/json' }, signal: controller.signal,
        body: JSON.stringify({ timeline: sourceResult.timeline.map(event => ({
          name: event.name, skill_id: event.skill_id, triggered: event.triggered, cast_time: event.cast_time,
          state_before: event.state_before ? {
            time: event.state_before.time, rage: event.state_before.rage,
            block_value: event.state_before.block_value, berserk_value: event.state_before.berserk_value,
            max_berserk_value: event.state_before.max_berserk_value,
            buffs: event.state_before.buffs?.map(({ name, buff_id, remaining, stacks }) => ({ name, buff_id, remaining, stacks })),
            target_buffs: event.state_before.target_buffs?.map(({ name, buff_id, remaining, stacks }) => ({ name, buff_id, remaining, stacks })),
            skill_states: event.state_before.skill_states,
          } : null,
        })),
          selection_start: selected[0], selection_end: selected[1], step,
          options: { max_terms: 2, max_candidates: 120 } }),
      });
      if (!res.ok) {
        const data = await res.json().catch(() => null);
        throw new Error(data?.error || (res.status === 404
          ? '当前后端尚未加载写宏分析功能，请稍后刷新。' : `条件分析失败（${res.status}）。`));
      }
      const data = await res.json();
      if (token !== revision || !active || !requireFresh()) return;
      if (isProgram && (!Array.isArray(data.steps) || data.steps.length !== selectionPattern.length)) {
        throw new Error('未取得完整的逐步分析，请重试。');
      }
      programResponse = isProgram ? data : null;
      response = isProgram ? data.steps[step] : data;
      responseKey = analysisKey();
      paintMarks(); render();
    } catch (error) {
      if (token !== revision || !active) return;
      showError(error.name === 'AbortError' ? '分析超时，请缩短序列后重试。' : error.message);
    } finally {
      clearTimeout(timeout);
      if (abort === controller) { abort = null; pendingAnalysisKey = ''; }
    }
  }

  function nodesAt(indices) {
    const wanted = new Set(indices);
    return [...sequence.querySelectorAll('[data-macro-assist-index]')]
      .filter(node => wanted.has(Number(node.dataset.macroAssistIndex)));
  }
  function paintMarks() {
    clearMarks();
    const matched = new Set();
    for (const hit of response?.occurrences || []) {
      for (let i = hit.start_active_index; i <= hit.end_active_index; i++) matched.add(i);
    }
    nodesAt([...matched]).forEach(node => node.classList.add('ma-occurrence'));
    if (selected) nodesAt(Array.from({ length: selected[1] - selected[0] + 1 }, (_, i) => selected[0] + i))
      .forEach(node => node.classList.add('ma-selected'));
    redrawSelectionOutlines();
  }
  function focusIndices(indices, extra) {
    if (!requireFresh()) return;
    paintMarks();
    const nodes = nodesAt(indices);
    nodes.forEach(node => node.classList.add(extra ? 'ma-extra' : 'ma-focus'));
    redrawSelectionOutlines();
    nodes[0]?.scrollIntoView({ block: 'nearest', inline: 'nearest', behavior: 'smooth' });
  }

  function render() {
    if (!response) return;
    rememberDisclosures();
    content.replaceChildren();
    if (!programResponse) { renderStep(content); return; }
    message(`找到 ${response.occurrences.length} 处 · ${selectionPattern.length} 步组合 · ${programResponse.programs?.length || 0} 个多行方案`);
    const heading = el('div', null, 'ma-program-heading');
    heading.append(el('strong', '整段宏方案'), el('span', '模板条件检查 · 尚未运行', 'ma-program-status'));
    content.append(heading);
    const explanation = disclosure('ma-program-explanation ma-candidate-help', 'program-explanation');
    explanation.append(el('summary', '方案指标与使用说明'));
    explanation.append(el('p', '下面是覆盖所选连续组合的完整多行候选。插入右下宏编辑区后，点击“运行对照”检查实际循环。', 'ma-explanation'));
    explanation.append(el('p', '模板联合匹配：组合每一步的释放前快照中，首个已知成立宏行均指向该步技能，且前面没有状态未知的行干扰判断。'));
    explanation.append(el('p', '组合外同技能成立：组合外同一技能的释放前快照中，对应宏行条件成立的次数，同一快照只计一次；通用规则也可能正确服务这些位置，不能直接判为误放。'));
    explanation.append(el('p', '行间冲突：组合内快照中首个成立宏行指向其它技能的次数。这里只检查条件，未考虑技能当时能否施放，也未验证执行后资源、冷却或后续循环。'));
    const programs = programResponse.programs || [];
    const list = el('div', null, 'ma-program-list');
    if (!programs.length) list.append(el('p', '未找到可展示的完整方案。可展开逐步分析，先插入单行候选，再在右侧编辑和运行。', 'ma-empty'));
    programs.forEach((program, index) => {
      const card = el('article', null, 'ma-program');
      const header = el('div', null, 'ma-program-header');
      header.append(el('strong', `方案 ${index + 1}`), el('span', `${program.lines?.length || 0} 行 · ${program.chars} 字符`, 'ma-program-meta'));
      card.append(header, el('pre', program.macro_text, 'ma-program-code'));
      const metrics = el('div', null, 'ma-program-metrics');
      metrics.append(el('span', `模板联合匹配 ${program.matched_occurrences}/${program.total_occurrences} 处`));
      metrics.append(el('span', `组合外同技能成立 ${program.outside_matches} 次`));
      metrics.append(el('span', `行间冲突 ${program.priority_conflicts} 次`));
      if (program.unknowns) metrics.append(el('span', `状态不足 ${program.unknowns} 次`));
      card.append(metrics);
      const actions = el('div', null, 'ma-program-actions');
      actions.append(button('插入宏', event => insertMacro(program.macro_text, event.currentTarget), '将完整方案插入右下宏编辑区的光标位置。'));
      actions.append(button('复制', event => copyMacro(program.macro_text, event.currentTarget), '复制完整多行方案。'));
      card.append(actions); list.append(card);
    });
    content.append(list, explanation);
    const detail = disclosure('ma-program-steps', 'program-steps');
    detail.append(el('summary', `逐步分析与条件证据（${selectionPattern.length} 步）`));
    const stepContent = el('div', null, 'ma-program-step-content');
    detail.append(stepContent);
    detail.addEventListener('toggle', () => {
      if (detail.open && !stepContent.childNodes.length) renderStep(stepContent);
    });
    content.append(detail);
    if (detail.open) renderStep(stepContent);
    const limits = disclosure('ma-limits', 'program-limits');
    limits.append(el('summary', '多行候选范围与数据说明'));
    for (const text of programResponse.limitations || []) limits.append(el('p', text));
    content.append(limits);
  }

  function renderStep(parent) {
    rememberDisclosures(parent);
    parent.replaceChildren();
    const events = activeEvents();
    const names = response.selection.skill_names;
    if (!programResponse) message(`找到 ${response.occurrences.length} 处 · ${names.join(' → ')}`);
    const stepRow = el('div', null, 'ma-step-row');
    stepRow.append(el('span', '查看步骤：'));
    names.forEach((name, index) => {
      const node = button(`${index + 1}. ${name}${index === 0 ? '（入口）' : ''}`, () => {
        if (programResponse) {
          if (!requireFresh()) return;
          step = index; shown = 30; response = programResponse.steps[index];
          paintMarks(); renderStep(parent); return;
        }
        cancelAnalysis(); step = index; shown = 30; response = null;
        showPending('正在分析…'); scheduleUpdate();
      }, '组合的每一步单独分析；入口条件不会保证后续技能自动连续释放。');
      node.classList.toggle('is-active', index === step);
      node.setAttribute('aria-pressed', String(index === step));
      stepRow.append(node);
    });
    if (names.length > 1) parent.append(stepRow);
    const tabs = el('div', null, 'ma-filters');
    for (const [value, label] of [['all', '综合排序'], ['common', '覆盖全部位置'], ['exact', '无额外匹配']]) {
      const node = button(label, () => { filter = value; shown = 30; renderCandidates(); syncFilters(); }, {
        all: '优先区分同一技能在组合内与组合外的释放；再比较其它位置的额外匹配。效果相同时优先条件少、字符短的写法。',
        common: '每个目标位置都满足，且没有未知状态；仍可能匹配其他位置。',
        exact: '所有其他位置都不满足，且没有未知状态；仍可能漏掉部分目标位置。',
      }[value]);
      node.dataset.filter = value; tabs.append(node);
    }
    const input = el('input'); input.type = 'search'; input.placeholder = '筛选：sun / 血怒 / skill…';
    input.value = search; input.setAttribute('aria-label', '筛选宏条件');
    input.addEventListener('input', () => { search = input.value; shown = 30; renderCandidates(); });
    tabs.append(input); parent.append(tabs);
    const results = el('div', null, 'ma-candidates-primary'); results.id = 'macro_assist_candidates'; parent.append(results);
    const explanation = disclosure('ma-candidate-help', 'candidate-help');
    explanation.append(el('summary', '候选排序与匹配指标说明'));
    explanation.append(el('p', '优先比较同一技能在组合内、组合外的状态。覆盖表示识别了多少组合内释放；组合外误匹配表示把多少次组合外的同技能也判为成立。其它技能位置另列，不代表当时实际能够施放。', 'ma-explanation'));
    parent.append(explanation);
    renderContrast(parent);
    const occurrences = disclosure('ma-occurrences', 'occurrences');
    occurrences.append(el('summary', `全部出现位置与状态（${response.occurrences.length} 处）`));
    const hitList = el('div', null, 'ma-hit-list');
    response.occurrences.forEach((hit, index) => {
      const event = events[hit.step_active_index];
      const detail = el('details', null, 'ma-hit');
      const summary = el('summary', `第 ${index + 1} 处 · #${hit.start_active_index + 1}–#${hit.end_active_index + 1} · ${seconds(event?.cast_time)} · ${resourceSummary(event?.state_before)}`);
      detail.append(summary);
      detail.append(button('定位这处组合', () => focusIndices(
        Array.from({ length: hit.end_active_index - hit.start_active_index + 1 }, (_, i) => hit.start_active_index + i)), '在序列中高亮并滚动到这处位置。'));
      detail.addEventListener('toggle', () => {
        if (!detail.open || detail.dataset.loaded) return;
        detail.dataset.loaded = '1';
        for (let i = hit.start_active_index; i <= hit.end_active_index; i++) {
          const event = events[i];
          const block = el('div', null, 'ma-state');
          block.append(el('strong', `${i - hit.start_active_index + 1}. ${event?.name || '—'} · #${i + 1} · ${seconds(event?.cast_time)}`));
          appendState(block, event?.state_before); detail.append(block);
        }
      });
      hitList.append(detail);
    });
    occurrences.append(hitList); parent.append(occurrences);
    const limits = disclosure('ma-limits', 'candidate-limits');
    limits.append(el('summary', '候选范围与数据说明'));
    for (const text of response.limitations || []) limits.append(el('p', text));
    const stats = response.search || {};
    limits.append(el('p', '条件按当前样本中的数值边界枚举，并在有限数量内搜索组合；不穷举无限阈值和任意长度表达式。相同表现的表达式可能被合并。'));
    limits.append(el('p', `已比较 ${stats.atoms_evaluated || 0} 个单条件和 ${stats.compounds_evaluated || 0} 个组合条件，展示 ${stats.returned_candidates || 0} 条候选${stats.candidates_truncated ? '（按排名截取）' : ''}。`));
    parent.append(limits); syncFilters(); renderCandidates();
  }
  function syncFilters() {
    content.querySelectorAll('[data-filter]').forEach(node => {
      node.classList.toggle('is-active', node.dataset.filter === filter);
      node.setAttribute('aria-pressed', String(node.dataset.filter === filter));
    });
  }
  function renderCandidates() {
    const container = document.getElementById('macro_assist_candidates');
    if (!container || !response) return;
    container.replaceChildren();
    const candidates = response.candidates.filter(candidate => {
      if (filter === 'common' && (candidate.fn || candidate.unknown_positive)) return false;
      if (filter === 'exact' && (candidate.fp || candidate.unknown_negative)) return false;
      return !search || [candidate.expression, ...(candidate.equivalent_expressions || [])]
        .some(expression => expression.toLowerCase().includes(search.toLowerCase()));
    });
    const outside = response.same_skill_outside_combo_indices || [];
    container.append(el('p', `${candidates.length} 条候选 · 组合内本步骤 ${response.positives.length} 次 · 组合外同技能 ${outside.length} 次`, 'ma-result-count'));
    if (!candidates.length) {
      container.append(el('p', '没有符合筛选的条件。可切换筛选、选择另一组合，或查看状态差异。', 'ma-empty'));
      return;
    }
    const scroll = el('div', null, 'ma-table-scroll');
    const table = el('table', null, 'ma-table');
    const head = el('thead'), headRow = el('tr');
    for (const text of ['条件 / 可复制语句', '组合内覆盖', '组合外同技能', '其它位置', '漏匹配', '操作']) headRow.append(el('th', text));
    head.append(headRow); table.append(head);
    const body = el('tbody');
    candidates.slice(0, shown).forEach(candidate => {
      const row = el('tr'), expr = el('td');
      const macro = candidate.macro_text || `/cast ${candidate.expression ? `[${candidate.expression}] ` : ''}${response.selection.skill_names[step]}`;
      expr.append(el('code', macro));
      expr.append(el('small', `${candidate.terms} 项条件 · ${candidate.chars} 字符${candidate.unknown_positive || candidate.unknown_negative ? ` · ${candidate.unknown_positive + candidate.unknown_negative} 个位置状态不足` : ''}`));
      if (candidate.equivalent_expressions?.length) {
        const alternatives = el('details', null, 'ma-equivalents');
        alternatives.append(el('summary', `相同匹配结果的其他写法（${candidate.equivalent_expressions.length}）`));
        for (const expression of candidate.equivalent_expressions) {
          const line = el('div', null, 'ma-equivalent-line');
          const alternative = `/cast ${expression ? `[${expression}] ` : ''}${response.selection.step_target}`;
          line.append(el('code', expression || '无条件'), button('插入宏', event => insertMacro(alternative, event.currentTarget)),
            button('复制写法', event => copyMacro(alternative, event.currentTarget)));
          alternatives.append(line);
        }
        expr.append(alternatives);
      }
      row.append(expr, el('td', `${candidate.tp}/${response.positives.length} · ${pct(candidate.coverage)}`));
      const sameSkill = el('td');
      sameSkill.append(button(outside.length ? `${candidate.same_skill_outside_combo_fp ?? 0}/${outside.length} 误匹配` : '无对照样本',
        () => showEvidence(candidate), '查看同一技能在组合外的全部释放，检查这些位置是否也满足条件。'));
      if (candidate.same_skill_outside_combo_unknown) sameSkill.append(el('small', `${candidate.same_skill_outside_combo_unknown} 处状态未知`));
      row.append(sameSkill);
      const extra = el('td');
      extra.append(button(String(candidate.fp - (candidate.same_skill_outside_combo_fp || 0)), () => showEvidence(candidate), '查看组合内其它步骤或其它技能满足条件的位置。'));
      row.append(extra, el('td', String(candidate.fn)));
      const actions = el('td');
      actions.append(button('插入宏', event => insertMacro(macro, event.currentTarget), '插入右下宏编辑区的光标位置。'));
      actions.append(button('复制', event => copyMacro(macro, event.currentTarget), '复制当前步骤的一条宏语句，不修改现有宏。'));
      actions.append(button('查看', () => showEvidence(candidate), '查看覆盖、漏匹配与额外匹配的位置。'));
      row.append(actions);
      ['组合内覆盖', '组合外同技能', '其它位置', '漏匹配'].forEach((label, index) => {
        row.children[index + 1].dataset.label = label;
      });
      body.append(row);
    });
    table.append(body); scroll.append(table); container.append(scroll);
    if (candidates.length > shown) container.append(button('显示更多条件', () => { shown += 40; renderCandidates(); }));
  }
  function showEvidence(candidate) {
    if (!requireFresh()) return;
    document.getElementById('macro_assist_evidence')?.remove();
    const block = el('div', null, 'ma-evidence'); block.id = 'macro_assist_evidence';
    const heading = el('div', null, 'ma-evidence-heading');
    heading.append(el('strong', candidate.expression || '无条件'), button('收起', () => { block.remove(); paintMarks(); }));
    block.append(heading);
    const yes = candidate.matched_positive_indices || [], extras = candidate.matched_negative_indices || [];
    const outside = response.same_skill_outside_combo_indices || [], outsideSet = new Set(outside);
    const outsideHits = new Set(candidate.matched_same_skill_outside_combo_indices || []);
    const yesSet = new Set(yes);
    const missing = response.positives.filter(index => !yesSet.has(index));
    for (const [label, indices, extra] of [['组合内已覆盖', yes, false], ['组合内未覆盖 / 状态不足', missing, false],
      ['组合外同技能（标注条件是否成立）', outside, true], ['其它位置额外匹配', extras.filter(index => !outsideSet.has(index)), true]]) {
      const line = el('div', null, 'ma-evidence-links'); line.append(el('span', `${label} ${indices.length}：`));
      indices.slice(0, 200).forEach(index => {
        const event = activeEvents()[index];
        const verdict = label.startsWith('组合外') ? (outsideHits.has(index) ? ' · 误匹配' : candidate.unknown_negative_indices?.includes(index) ? ' · 未知' : ' · 正确排除') : '';
        line.append(button(`#${index + 1} ${event?.name || ''} ${seconds(event?.cast_time)}${verdict}`, () => {
          focusIndices([index], extra);
          block.querySelector('.ma-evidence-state')?.remove();
          const state = el('div', null, 'ma-evidence-state');
          state.append(el('strong', `#${index + 1} ${event?.name || ''} · 释放前`));
          appendState(state, event?.state_before); block.append(state);
        }));
      });
      if (indices.length > 200) line.append(el('span', '（仅列出前 200 处）'));
      block.append(line);
    }
    document.getElementById('macro_assist_candidates').append(block);
    block.scrollIntoView({ block: 'nearest' });
  }

  function renderContrast(parent) {
    const section = disclosure('ma-contrast', 'contrast'); section.id = 'macro_assist_contrast';
    const inside = response.positives || [], outside = response.same_skill_outside_combo_indices || [];
    section.append(el('summary', `同技能对比 · 组合内本步骤：${inside.length} 次 / 组合外同技能：${outside.length} 次`));
    if (!outside.length) {
      section.append(el('p', selectionPattern.length === 1
        ? '单技能查询已经包含该技能的所有释放。选择一个连续组合，才能比较同技能在组合内外的差异。'
        : '当前序列没有这个技能在组合外释放的样本，尚不能验证条件能否区分组合内外。', 'ma-explanation'));
    } else {
      const events = activeEvents();
      const a = inside.map(index => events[index]?.state_before), b = outside.map(index => events[index]?.state_before);
      const table = el('table', null, 'ma-table ma-contrast-table'), head = el('tr');
      for (const label of ['释放前状态', `组合内（${inside.length}）`, `组合外同技能（${outside.length}）`]) head.append(el('th', label));
      const thead = el('thead'); thead.append(head); table.append(thead);
      const body = el('tbody');
      const range = (states, getter, format = String) => {
        const values = states.map(getter).filter(Number.isFinite);
        if (!values.length) return '未记录';
        const min = Math.min(...values), max = Math.max(...values);
        return (min === max ? format(min) : `${format(min)}–${format(max)}`) + (values.length < states.length ? `（${states.length-values.length} 处未知）` : '');
      };
      const addRow = (label, left, right) => { const row = el('tr'); row.append(el('td', label), el('td', left), el('td', right)); body.append(row); };
      for (const [name, field] of [['怒气', 'rage'], ['暴怒', 'berserk_value'], ['格挡', 'block_value']]) {
        if ([...a, ...b].some(state => Number.isFinite(state?.[field]))) addRow(name, range(a, state => state?.[field]), range(b, state => state?.[field]));
      }
      const buffs = new Map();
      for (const state of [...a, ...b]) for (const buff of state?.buffs || []) {
        if (buff.name && !/^\d+$/.test(buff.name)) buffs.set(buff.buff_id ?? buff.name, buff.name);
      }
      const describeBuff = (states, id) => {
        const known = states.filter(state => Array.isArray(state?.buffs));
        const present = known.map(state => [...state.buffs].reverse().find(buff => (buff.buff_id ?? buff.name) === id)).filter(Boolean);
        const count = `有 ${present.length}/${states.length}`;
        if (!present.length) return count + (known.length < states.length ? '（含未知）' : '');
        const duration = present.every(buff => buff.remaining === 0) ? '永久' : range(present, buff => buff.remaining > 0 ? buff.remaining : NaN, seconds);
        return `${count} · ${range(present, buff => buff.stacks)} 层 · ${duration}` + (known.length < states.length ? '（含未知）' : '');
      };
      for (const [id, name] of buffs) addRow(name, describeBuff(a, id), describeBuff(b, id));
      const scroll = el('div', null, 'ma-table-scroll'); table.append(body); scroll.append(table); section.append(scroll);
    }
    const list = el('details', null, 'ma-outside-list');
    list.append(el('summary', `查看组合外同技能的每次释放（${outside.length}）`));
    list.addEventListener('toggle', () => {
      if (!list.open || list.dataset.loaded) return;
      list.dataset.loaded = '1';
      for (const index of outside) {
        const event = activeEvents()[index], item = el('details', null, 'ma-hit');
        item.append(el('summary', `#${index+1} ${event?.name || ''} · ${seconds(event?.cast_time)} · ${resourceSummary(event?.state_before)}`));
        item.append(button('定位释放位置', () => focusIndices([index], true)));
        const context = activeEvents().slice(Math.max(0, index-1), index+2).map((ev, offset) =>
          `${Math.max(0, index-1)+offset === index ? '【' : ''}${ev.name}${Math.max(0, index-1)+offset === index ? '】' : ''}`).join(' → ');
        item.append(el('p', context, 'ma-explanation'));
        item.addEventListener('toggle', () => {
          if (!item.open || item.dataset.loaded) return;
          item.dataset.loaded = '1'; appendState(item, event?.state_before);
        });
        list.append(item);
      }
    });
    section.append(list);
    const otherSteps = response.same_skill_other_step_indices?.length || 0;
    if (otherSteps) section.append(el('p', `另有 ${otherSteps} 次同技能属于已匹配组合的其它步骤，单独归入“其它位置”，不会当成组合外样本。`, 'ma-explanation'));
    parent.append(section);
  }
  function resourceSummary(state) {
    if (!state) return '状态缺失';
    return [['怒气', state.rage], ['暴怒', state.berserk_value], ['格挡', state.block_value]]
      .filter(([, value]) => Number.isFinite(value)).map(([name, value]) => `${name} ${value}`).join(' / ');
  }
  function appendState(parent, state) {
    if (!state) { parent.append(el('p', '此位置没有完整状态快照。')); return; }
    parent.append(el('p', `${resourceSummary(state)} · ${({ Shield: '擎盾', Blade: '擎刀', Wall: '盾墙' })[state.stance] || state.stance || '姿态未知'}`));
    for (const [label, values] of [['自身气劲', state.buffs], ['目标气劲', state.target_buffs]]) {
      parent.append(el('p', `${label}：${values ? (values.map(buff => `${buff.name} ×${buff.stacks}（${buff.remaining > 0 ? seconds(buff.remaining) : '永久'}）`).join('、') || '无') : '未记录'}`));
    }
    parent.append(el('p', `技能冷却：${state.skill_cds?.map(cd => `${cd.name} ${seconds(cd.remaining)}`).join('、') || '无冷却记录'}`));
    const charges = state.skill_states;
    parent.append(el('p', `充能：${charges ? charges.filter(skill => skill.charges != null).map(skill => `${skill.name} ${skill.charges}/${skill.max_charges ?? '—'}`).join('、') || '无充能技能' : '未记录'}`));
    if (Number.isFinite(state.time)) parent.append(el('p', `状态采样时刻：${seconds(state.time)}`));
  }
  async function copyMacro(text, target) {
    if (!requireFresh()) return;
    try {
      if (navigator.clipboard?.writeText && window.isSecureContext) await navigator.clipboard.writeText(text);
      else {
        const area = el('textarea'); area.value = text; area.className = 'ma-copy-buffer';
        document.body.append(area); area.select();
        const copied = document.execCommand('copy'); area.remove();
        if (!copied) throw new Error('copy unavailable');
      }
      const label = target.textContent;
      target.textContent = '已复制'; setTimeout(() => { target.textContent = label; }, 1500);
    } catch { message('浏览器未允许复制，可直接选中表格中的宏语句复制。'); }
  }
  function insertMacro(text, target) {
    if (!requireFresh()) return;
    if (!window.Jx3MacroEditor?.insert(text)) {
      message('宏编辑区尚未准备好，可先复制宏语句。'); return;
    }
    const label = target.textContent;
    target.textContent = '已插入'; setTimeout(() => { target.textContent = label; }, 1500);
  }

  // 写宏模式的点击只用于选择，避免触发删除、引导跳数、时间偏移或拖动编辑。
  sequence.addEventListener('click', event => {
    if (active && event.target.closest('.sim-seq-item')) { event.preventDefault(); event.stopImmediatePropagation(); }
  }, true);
  sequence.addEventListener('dragstart', event => {
    if (active) { event.preventDefault(); event.stopImmediatePropagation(); }
  }, true);
  sequence.addEventListener('dblclick', event => {
    if (active) { event.preventDefault(); event.stopImmediatePropagation(); }
  }, true);
  document.addEventListener('keydown', event => {
    if (event.key === 'Escape' && active && !event.target.closest('input, textarea')) {
      clearPick();
    }
  });
  window.addEventListener('jx3-sim-complete', () => {
    if (active && !busy && sourceResult !== lastSimResult) scheduleUpdate();
  });
  for (const type of ['input', 'change']) document.addEventListener(type, event => {
    if (active && !panel.contains(event.target) && !event.target.closest('[data-macro-draft], #macro_review')) {
      if (!isFresh()) { cancelAnalysis(); response = null; programResponse = null; showPending('配置已变化，正在自动更新…'); }
      scheduleUpdate();
    }
  });
  new MutationObserver(records => {
    const isOutline = node => node.nodeType === Node.ELEMENT_NODE && node.matches('.ma-selection-outline');
    // 新增/移除显示框不能触发模拟或分析；同时忽略显示框自身的后续 DOM 更新。
    const changed = records.some(record => {
      if (record.target.nodeType === Node.ELEMENT_NODE && record.target.closest('.ma-selection-outline')) return false;
      return record.type !== 'childList' || [...record.addedNodes, ...record.removedNodes].some(node => !isOutline(node));
    });
    if (!active || !changed) return;
    scheduleSelectionOutlines();
    if (!isFresh()) scheduleUpdate();
  }).observe(sequence, { childList: true, subtree: true, attributes: true,
    attributeFilter: ['data-skill', 'data-timing-offset', 'data-qijin-buff', 'data-pre-time', 'data-clearcd-target'] });
  new MutationObserver(scheduleSelectionOutlines).observe(sequence, { attributes: true, attributeFilter: ['style'] });
  new MutationObserver(scheduleSelectionOutlines).observe(document.body, { attributes: true, attributeFilter: ['class', 'data-theme'] });
  if (typeof ResizeObserver !== 'undefined') new ResizeObserver(scheduleSelectionOutlines).observe(sequence);
  window.addEventListener('jx3-macro-layout-change', scheduleSelectionOutlines);
  modeButton.addEventListener('click', () => setMode(true));
  editButton.addEventListener('click', () => setMode(false));
  window.Jx3MacroAssist = {
    isActive: () => active, selectItems, contextKey,
    getReference: () => isFresh() ? { result: sourceResult, key: sourceKey, selection: selected ? [...selected] : null, pattern: [...selectionPattern] } : null,
    simulationStarted(key) { if (key) runningSimulations.set(key, (runningSimulations.get(key) || 0) + 1); },
    simulationFinished(key) {
      const remaining = (runningSimulations.get(key) || 0) - 1;
      if (remaining > 0) runningSimulations.set(key, remaining); else runningSimulations.delete(key);
      if (active && !busy) scheduleUpdate();
    },
  };
  sequence.classList.add('ma-selection-overlay-ready');
})();
