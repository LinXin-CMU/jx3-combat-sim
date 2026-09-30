/* Parallel rotations share the scene and the original, fully interactive foreground editor. */
(function () {
  'use strict';
  const editor = document.getElementById('sim_sequence');
  const host = editor?.closest('.ma-template');
  if (!editor || !host) return;
  const clone = value => JSON.parse(JSON.stringify(value));
  const el = (tag, text, cls) => { const n = document.createElement(tag); if (text != null) n.textContent = text; if (cls) n.className = cls; return n; };
  const button = (text, action, title) => { const b = el('button', text, 'ltab-button'); b.type = 'button'; if (title) b.title = title; b.onclick = action; return b; };
  const bar = el('div', null, 'ltab-bar'), strip = el('div', null, 'ltab-strip');
  const status = el('span', '载入循环页…', 'ltab-save'); status.setAttribute('role', 'status');
  bar.id = 'loop_tools'; bar.setAttribute('role', 'group'); bar.setAttribute('aria-label', '循环功能区');
  const modes = document.querySelector('#panel_manual .ma-mode-primary');
  if (modes) bar.append(modes);
  host.prepend(strip); host.append(bar); host.classList.add('ltab-host');
  document.getElementById('panel_manual').classList.add('ltab-workspace');
  const macroHeading = document.getElementById('macro_compare_heading');
  if (macroHeading) document.getElementById('macro_compare_panel')?.prepend(macroHeading);
  const toolbar = document.getElementById('seq_float_toolbar');
  let tabs = [], activeId = 'main', epoch = 0, ready = false, moving = false, identity = null, scope = '', endpoint = '';
  let localSaved = false;
  let saveTimer = null, diffTimer = null, saving = false, dirty = false, revision = 0, updatedAt = 0, macroModeActive = false;
  let scrollLinks = [], scrollFrame = null, scrollSource = null;
  let refreshTimer = null, backgroundRunning = false;
  const syncedPositions = new WeakMap();
  const current = () => tabs.find(t => t.id === activeId);
  let toolsTimer = null, suppressToolsFocus = false;
  function showTools(open) {
    clearTimeout(toolsTimer);
    const tab = current(); if (!tab) return;
    tab.pane.classList.toggle('is-tools-open', open);
  }
  function leaveTools() {
    clearTimeout(toolsTimer);
    toolsTimer = setTimeout(() => {
      if (!bar.matches(':hover') && !bar.querySelector(':focus-visible') && !current()?.heading.matches(':hover')) showTools(false);
    }, 180);
  }
  bar.addEventListener('pointerenter', () => clearTimeout(toolsTimer));
  bar.addEventListener('pointerleave', leaveTools);
  bar.addEventListener('focusin', () => showTools(true));
  bar.addEventListener('focusout', leaveTools);
  document.addEventListener('keydown', event => {
    if (event.key === 'Escape' && current()?.pane.classList.contains('is-tools-open')) {
      suppressToolsFocus = true;
      if (bar.contains(document.activeElement)) current().title.focus();
      suppressToolsFocus = false;
      showTools(false);
    }
  });
  const container = tab => tab.id === activeId ? editor : tab.passive;
  const storageKey = () => `jx3_loop_tabs_v1:${JSON.stringify([identity, scope])}`;
  const sequenceConfig = () => {
    const cfg = buildLoopConfig();
    return { sequence: cfg.sequence, macro: cfg.macro, macro_duration: cfg.macro_duration || 0 };
  };
  function sceneKey() {
    const cfg = buildLoopConfig(); delete cfg.sequence; delete cfg.macro; delete cfg.macro_duration; delete cfg.exported_at;
    return JSON.stringify([cfg, getSimAttrs(), getEquipmentMap(), getSimHasteLevel(), isExperimental()]);
  }
  function capture() {
    const tab = current(); if (!tab || moving || !tab.initialized) return;
    tab.config = sequenceConfig(); tab.channels = getSequenceChannelTicks();
    tab.scroll = editor.scrollTop;
  }
  function documentValue() {
    capture();
    const scene = buildLoopConfig(); delete scene.sequence; delete scene.macro; delete scene.macro_duration; delete scene.exported_at;
    return { schema: 1, updatedAt, scene, active: activeId, tabs: tabs.map(t => ({ id: t.id, name: t.name, hidden: t.hidden,
      width: t.width, position: t.position, compareTo: t.compareTo, comparisonExplicit: t.comparisonExplicit,
      syncScroll: t.syncScroll, layer: t.layer, ...t.config })) };
  }
  async function getIdentity() {
    const r = await fetch('/api/auth/me', { cache: 'no-store', signal: AbortSignal.timeout(4000) });
    if (!r.ok) throw new Error('身份不可用');
    const a = await r.json();
    return a.enabled === false ? 'local' : a.authed && a.username ? `user:${a.username}` : null;
  }
  function stash() {
    if (!ready || !identity || !dirty) return;
    try { localStorage.setItem(storageKey(), JSON.stringify(documentValue())); localSaved = true; }
    catch { localSaved = false; status.textContent = '本机保存空间不足'; }
  }
  async function save() {
    if (!ready || !dirty || saving || !identity) return;
    saving = true; const ticket = revision;
    try {
      // Another tab can change the login cookie. Never send this user's panes to the new account.
      if (await getIdentity() !== identity) { dirty = false; status.textContent = '账号已切换，请刷新'; return; }
      const value = documentValue(); stash();
      const r = await fetch(endpoint, { method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(value), signal: AbortSignal.timeout(5000) });
      if (!r.ok) throw new Error('保存失败');
      if (ticket === revision) dirty = false;
      status.textContent = dirty ? '保存中…' : '已保存';
    } catch { status.textContent = localSaved ? '已保留本机 · 点击重试' : '尚未保存 · 点击重试'; }
    finally { saving = false; if (dirty && ticket !== revision) saveTimer = setTimeout(save, 600); }
  }
  function changed() {
    if (!ready || moving) return;
    capture(); dirty = true; revision++; updatedAt = Date.now(); stash(); status.textContent = identity ? '保存中…' : '仅本页保留';
    clearTimeout(saveTimer); saveTimer = setTimeout(save, 650); scheduleDiff(); scheduleRefresh();
  }
  status.title = '按账号、版本和心法保存；点击重试保存'; status.onclick = save;
  const addCopy = button('复制', () => add(true), '复制当前循环到新页'), addEmpty = button('新建', () => add(false), '新建空白循环页');
  addCopy.setAttribute('aria-label', '复制当前循环到新页'); addEmpty.setAttribute('aria-label', '新建空白循环页');
  addCopy.disabled = addEmpty.disabled = true; bar.append(addCopy, addEmpty, status);
  new MutationObserver(() => {
    status.dataset.state = status.textContent === '已保存' ? 'saved' : /失败|不足|未保存|重试|账号/.test(status.textContent) ? 'error' : 'pending';
    status.title = `${status.textContent}；按账号保存，点击重试`;
  }).observe(status, { childList: true });

  function createTab(data, index) {
    const tab = { id: index === 0 ? 'main' : data.id || `loop-${Date.now()}-${index}`, name: String(data.name || (index ? `循环 ${index + 1}` : '主循环')).slice(0, 48),
      hidden: index > 0 && !!data.hidden, width: Number.isFinite(data.width) ? Math.max(260, Math.min(2400, data.width)) : null,
      position: Number.isFinite(data.position) ? data.position : index,
      compareTo: typeof data.compareTo === 'string' && (data.compareTo || data.comparisonExplicit) ? data.compareTo : index ? 'main' : '',
      comparisonExplicit: !!data.comparisonExplicit, layer: data.layer === 'states' ? 'states' : 'skills', syncScroll: data.syncScroll !== false,
      config: { sequence: Array.isArray(data.sequence) ? data.sequence : [], macro: data.macro || { mode: 'general', general: '', shield: '', blade: '' }, macro_duration: data.macro_duration || 0 },
      channels: {}, scroll: 0, result: null, initialized: false };
    tab.pane = el('section', null, 'ltab-pane'); tab.pane.dataset.loopTab = tab.id;
    tab.heading = el('div', null, 'ltab-heading'); tab.body = el('div', null, 'ltab-body');
    tab.passive = el('div', null, 'sim-sequence ltab-passive'); tab.passive.setAttribute('aria-label', `${tab.name}循环内容`); tab.passive.style.setProperty('--seq-scale', editor.style.getPropertyValue('--seq-scale') || '1');
    tab.title = button('', () => { activate(tab.id); showTools(true); }, '设为当前循环；双击重命名'); tab.title.classList.add('ltab-title');
    tab.nameNode = el('span', tab.name, 'ltab-name'); tab.title.append(tab.nameNode);
    tab.title.ondblclick = () => { tab.menu.open = true; tab.rename.focus(); tab.rename.select(); };
    tab.menu = el('details', null, 'ltab-menu'); const summary = el('summary', '⋯'); summary.title = '命名、交换页面、对比、隐藏'; summary.setAttribute('aria-label', `${tab.name}页面操作`);
    tab.menu.addEventListener('toggle', () => { if (tab.menu.open) showTools(false); });
    const popup = el('div', null, 'ltab-menu-content');
    const nameLabel = el('label', '名称'); tab.rename = el('input'); tab.rename.value = tab.name; tab.rename.maxLength = 48;
    tab.rename.setAttribute('aria-label', '循环页名称'); nameLabel.append(tab.rename);
    tab.rename.onchange = () => { tab.name = tab.rename.value.trim() || tab.name; tab.rename.value = tab.name; renderChrome(); changed(); };
    tab.rename.onkeydown = e => { if (e.key === 'Enter') { tab.rename.blur(); tab.menu.open = false; } };
    const swapLabel = el('label', '交换页面'); tab.swap = el('select'); tab.swap.setAttribute('aria-label', '交换页面'); swapLabel.append(tab.swap);
    tab.swap.onchange = () => exchange(tab, tab.swap.value);
    const compareLabel = el('label', '差异分析'); tab.compare = el('select'); tab.compare.setAttribute('aria-label', '选择对比循环'); compareLabel.append(tab.compare);
    tab.compare.onchange = () => { tab.compareTo = tab.compare.value; tab.comparisonExplicit = true; tab.syncScroll = true; tab.alignScroll = true; scrollLinks = []; const other = tabs.find(t => t.id === tab.compareTo); if (other) other.hidden = false; renderChrome(); changed(); };
    const syncLabel = el('label', null, 'ltab-sync-label'); tab.sync = el('input'); tab.sync.type = 'checkbox'; tab.sync.checked = true;
    syncLabel.append(tab.sync, el('span', '同步滚动'));
    tab.sync.onchange = () => { tab.syncScroll = tab.sync.checked; tab.alignScroll = tab.syncScroll; scrollLinks = []; changed(); };
    const layerLabel = el('label', '差异层级'); tab.layerSelect = el('select'); tab.layerSelect.setAttribute('aria-label', '循环对比差异层级');
    for (const [value, text] of [['skills', '技能顺序'], ['states', '含时间和状态']]) { const o = el('option', text); o.value = value; tab.layerSelect.append(o); }
    tab.layerSelect.value = tab.layer; tab.layerSelect.onchange = () => { tab.layer = tab.layerSelect.value; changed(); };
    layerLabel.append(tab.layerSelect);
    const actions = el('div', null, 'ltab-menu-actions');
    actions.append(button('复制此页', () => { activate(tab.id); add(true); }), button('隐藏', () => hide(tab.id)), button('删除', () => remove(tab.id)));
    if (tab.id === 'main') { actions.children[1].disabled = true; actions.children[2].disabled = true; }
    popup.append(nameLabel, swapLabel, compareLabel, syncLabel, layerLabel, actions); tab.menu.append(summary, popup);
    tab.diff = el('span', '', 'ltab-diff-summary');
    tab.diff.hidden = true; tab.title.append(tab.diff);
    tab.first = button('首处差异', () => firstDifference(tab)); tab.first.hidden = true;
    popup.append(tab.first);
    tab.retry = button('重新回放', () => {
      tab.refreshAttempt = null; tab.result = null; tab.compareError = '';
      if (tab.id === activeId) runSimulate(); else scheduleRefresh();
    }); tab.retry.hidden = true; popup.append(tab.retry);
    tab.heading.onpointerenter = () => { if (tab.id === activeId && !dragEl && !tab.menu.open) showTools(true); };
    tab.heading.onpointerleave = leaveTools;
    tab.title.onfocus = () => { if (!suppressToolsFocus && tab.id === activeId && tab.title.matches(':focus-visible')) showTools(true); };
    tab.title.onblur = leaveTools;
    tab.heading.append(tab.title, tab.menu);
    tab.pane.append(tab.heading, tab.body); tab.body.append(tab.passive);
    tab.handle = el('div', null, 'ltab-resizer'); tab.handle.tabIndex = 0; tab.handle.setAttribute('role', 'separator'); tab.handle.setAttribute('aria-orientation', 'vertical'); tab.handle.setAttribute('aria-label', '调整循环页宽度');
    tab.handle.title = '拖动调整相邻两页宽度，双击均分这两页'; tab.pane.append(tab.handle);
    tab.handle.onpointerdown = event => {
      if (event.button !== 0) return; event.preventDefault();
      const pair = resizePair(tab); if (!pair) return;
      const start = event.clientX;
      tab.handle.setPointerCapture(event.pointerId); document.body.classList.add('ltab-resizing');
      tab.handle.onpointermove = e => resizeTo(pair, pair.leftWidth + e.clientX - start);
      let finished = false;
      const finish = () => {
        if (finished) return; finished = true;
        tab.handle.onpointermove = null; document.body.classList.remove('ltab-resizing');
        if (tab.handle.hasPointerCapture(event.pointerId)) tab.handle.releasePointerCapture(event.pointerId);
        changed();
      };
      tab.handle.onpointerup = finish; tab.handle.onpointercancel = finish; tab.handle.onlostpointercapture = finish;
    };
    tab.handle.ondblclick = () => { const pair = resizePair(tab); if (pair) { resizeTo(pair, pair.total / 2); changed(); } };
    tab.handle.onkeydown = event => {
      if (!['ArrowLeft', 'ArrowRight', 'Home'].includes(event.key)) return; event.preventDefault();
      const pair = resizePair(tab); if (!pair) return;
      resizeTo(pair, event.key === 'Home' ? pair.total / 2 : pair.leftWidth + (event.key === 'ArrowLeft' ? -24 : 24)); changed();
    };
    // pointerdown precedes the original mousedown/drag handlers. Move the real items, not clones.
    tab.heading.addEventListener('pointerdown', event => { if (tab.id !== activeId && event.target.closest('.ltab-title') && event.button === 0) activate(tab.id); }, true);
    tab.body.addEventListener('pointerdown', event => {
      const box = container(tab), rect = box.getBoundingClientRect();
      const onScrollbar = event.clientX >= rect.right - (box.offsetWidth - box.clientWidth)
        || event.clientY >= rect.bottom - (box.offsetHeight - box.clientHeight);
      if (!onScrollbar && tab.id !== activeId && !dragEl && event.button === 0) activate(tab.id);
    }, true);
    // A background hit is retargeted to this stable parent when its passive container moves.
    tab.body.addEventListener('mousedown', event => {
      if (event.target === tab.body && tab.id === activeId && event.button === 0)
        window.dispatchEvent(new CustomEvent('jx3-loop-background-down', { detail: event }));
    });
    strip.append(tab.pane); return tab;
  }
  function applyWidth(tab) {
    tab.pane.style.flex = tab.width ? `0 0 ${tab.width}px` : '1 0 0px';
    tab.handle.setAttribute('aria-valuenow', String(Math.round(tab.width || tab.pane.getBoundingClientRect().width)));
  }
  const visibleTabs = () => tabs.filter(t => !t.pane.hidden).sort((a, b) => a.position - b.position);
  function freezeWidths() {
    // Read every border-box before writing: flex redistribution must not move unrelated panes.
    const sizes = visibleTabs().map(tab => [tab, tab.pane.getBoundingClientRect().width]);
    for (const [tab, width] of sizes) { tab.width = width; applyWidth(tab); }
  }
  function resizePair(left) {
    const visible = visibleTabs(), right = visible[visible.indexOf(left) + 1];
    if (!right || right === left) return null;
    freezeWidths();
    return { left, right, leftWidth: left.width, total: left.width + right.width };
  }
  function resizeTo(pair, width) {
    pair.left.width = Math.max(Math.max(260, pair.total - 2400), Math.min(Math.min(2400, pair.total - 260), width));
    pair.right.width = pair.total - pair.left.width;
    applyWidth(pair.left); applyWidth(pair.right); scheduleDiff();
  }
  function exchange(tab, id) {
    const other = tabs.find(t => t.id === id); if (!other || tab === other) return;
    tab.menu.open = false;
    // Main cannot be hidden; choosing a hidden page from Main restores it alongside Main.
    if (other.hidden && tab.id === 'main') other.hidden = false;
    else {
      freezeWidths();
      [tab.position, other.position] = [other.position, tab.position];
      [tab.width, other.width] = [other.width, tab.width];
      [tab.hidden, other.hidden] = [other.hidden, tab.hidden];
    }
    activate(other.id); renderChrome(); changed();
  }
  function renderChrome() {
    for (const tab of tabs) {
      tab.pane.hidden = tab.hidden || (macroModeActive && tab.id !== activeId);
      tab.pane.style.order = tab.position;
      tab.pane.classList.toggle('is-active', tab.id === activeId); tab.pane.setAttribute('aria-label', tab.name);
      tab.nameNode.textContent = tab.name; tab.title.title = `${tab.name}；设为当前循环，双击重命名`; tab.title.setAttribute('aria-pressed', String(tab.id === activeId));
      tab.menu.querySelector('summary').setAttribute('aria-label', `${tab.name}页面操作`);
      tab.swap.replaceChildren(); const placeholder = el('option', '选择循环页…'); placeholder.value = ''; tab.swap.append(placeholder);
      for (const other of [...tabs].sort((a, b) => a.position - b.position)) if (other !== tab) {
        const option = el('option', other.name + (other.hidden ? '（已隐藏）' : '')); option.value = other.id; tab.swap.append(option);
      }
      tab.swap.disabled = tabs.length < 2;
      tab.compare.replaceChildren();
      const none = el('option', '不对比'); none.value = ''; tab.compare.append(none);
      for (const other of tabs) if (other.id !== tab.id) { const o = el('option', other.name + (other.hidden ? '（已隐藏）' : '')); o.value = other.id; tab.compare.append(o); }
      tab.compare.value = tab.compareTo; applyWidth(tab);
      tab.sync.checked = tab.syncScroll; tab.sync.disabled = !tab.compareTo;
    }
    const visible = visibleTabs();
    for (const tab of tabs) {
      const last = tab === visible[visible.length - 1];
      tab.handle.hidden = tab.pane.hidden || last;
      tab.pane.classList.toggle('is-last', last);
    }
    strip.classList.toggle('is-single', visible.length === 1);
    scheduleDiff();
    scheduleRefresh();
    window.dispatchEvent(new Event('jx3-loop-tabs-change'));
  }
  function clearTransient() {
    scrollLinks = []; scrollSource = null;
    if (scrollFrame != null) cancelAnimationFrame(scrollFrame);
    scrollFrame = null;
    window.dispatchEvent(new Event('jx3-loop-pane-leave'));
    hideChannelBar(); hideTimingBar(); removeQijinPanel(); hideTooltip(); _closeInsertPopover();
    _insertIndicatorEl?.remove(); _insertIndicatorEl = null; _insertIndicatorIdx = -1;
    onSimulateComplete = null;
    simRequestGeneration++; epoch++;
  }
  function loadConfig(tab) {
    Object.keys(channelOverrides).forEach(k => delete channelOverrides[k]); Object.assign(channelOverrides, tab.channels);
    Object.keys(timingOffsets).forEach(k => delete timingOffsets[k]);
    macroMode = tab.config.macro.mode || 'general'; Object.assign(macroPages, { general: '', shield: '', blade: '' }, tab.config.macro);
    macroLastDuration = tab.config.macro_duration || 0;
    if (!tab.initialized) {
      applyLoopConfig({ version: 1, sequence: tab.config.sequence, initial_rage: adminInitialRage }, { skipSimulate: true });
      tab.channels = { ...channelOverrides }; tab.initialized = true;
    }
  }
  function activate(id, options = {}) {
    const next = tabs.find(t => t.id === id); if (!next || (id === activeId && next.initialized && !options.force)) return;
    showTools(false);
    clearTransient(); capture(); const old = current();
    // Read both scroll containers before moving their children. A detached or emptied
    // container loses its scroll offset, and the passive pane may have been scrolled independently.
    const nextTop = next === old ? editor.scrollTop : next.initialized ? next.passive.scrollTop : next.scroll || 0;
    const nextLeft = next === old ? editor.scrollLeft : next.passive.scrollLeft;
    const oldTop = editor.scrollTop, oldLeft = editor.scrollLeft;
    moving = true;
    if (old) {
      old.scroll = editor.scrollTop; old.passive.replaceChildren(...editor.childNodes); old.body.append(old.passive);
    }
    activeId = id; next.hidden = false;
    editor.replaceChildren(...next.passive.childNodes); next.passive.remove(); next.body.append(editor);
    if (toolbar) next.body.append(toolbar);
    next.body.append(bar);
    loadConfig(next);
    observer.takeRecords(); moving = false;
    lastSimResult = null; window._lastSimBody = null;
    renderChrome();
    if (old && old !== next) { old.passive.scrollTop = oldTop; old.passive.scrollLeft = oldLeft; }
    editor.scrollTop = nextTop; editor.scrollLeft = nextLeft; next.scroll = nextTop;
    const fresh = next.result && next.resultScene === sceneKey() && next.resultInput === JSON.stringify(next.config);
    if (!options.silent) {
      if (fresh) {
        window._lastSimBody = next.bodyRequest || buildSimulateRequest(); lastSimResult = next.result;
        next.result._configHash = computeLoopInputsHash();
        next.result._macroAssistKey = window.Jx3MacroAssist?.contextKey();
        next.result._macroAssistBody = clone(window._lastSimBody);
        window.dispatchEvent(new CustomEvent('jx3-sim-complete', { detail: next.result }));
        presentSimResult(next.result); next.previewOnly = false;
      }
      else {
        editor.querySelectorAll('.sim-seq-item').forEach(item => { item.onclick = null; });
        set('sim_dps_value', '—'); set('sim_fight_time', '计算中…'); set('sim_total_damage', '—');
        renderBuffList([]); renderTimeline([]); renderBuffTimeline([], 0); resetSkillButtons();
        runSimulate();
      }
      changed();
      window.dispatchEvent(new CustomEvent('jx3-loop-pane-change', { detail: { id } }));
    }
    if (old && old !== next) syncedPositions.set(old.passive, old.passive.scrollTop);
    syncedPositions.set(editor, editor.scrollTop);
  }
  function add(copy) {
    if (!ready) return; capture();
    const data = copy ? clone(current().config) : { sequence: [] };
    data.name = copy ? `${current().name} 副本` : `循环 ${tabs.length + 1}`; data.id = `loop-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
    data.position = Math.max(...tabs.map(t => t.position)) + 1;
    const tab = createTab(data, tabs.length); tabs.push(tab); activate(tab.id);
  }
  function outputStamp() {
    capture();
    return JSON.stringify([epoch, sceneKey(), tabs.map(t => [t.id, t.config])]);
  }
  async function importMacro(targetId, request, result, stamp) {
    if (!ready || await getIdentity() !== identity) throw new Error('账号或循环页尚未就绪，请刷新后重试。');
    if (stamp !== outputStamp()) throw new Error('运行期间循环或场景已改变，请重新运行，避免覆盖后续编辑。');
    let target = targetId ? tabs.find(t => t.id === targetId) : null;
    if (targetId && !target) throw new Error('目标循环页已删除，请重新选择。');
    function append(data) {
      const tab = createTab({ ...data, id: `loop-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`,
        position: Math.max(...tabs.map(t => t.position)) + 1 }, tabs.length);
      tabs.push(tab); return tab;
    }
    // Retain an existing destination as a hidden, normally editable recovery page.
    if (target?.config.sequence.length) append({ ...clone(target.config), name: `${target.name}（覆盖前）`, hidden: true });
    if (!target) target = append({ name: `宏回放 ${tabs.length}`, sequence: [] });
    activate(target.id, { silent: true, force: true });
    clearTransient();
    const count = (result.timeline || []).filter(e => !e.triggered).length;
    const sequence = (request.pre_releases || []).map(p => ({ type: 'pre_release', skill: p.skill, pre_time: p.time_before }));
    sequence.push({ type: 'macro', count: Math.max(1, count) });
    applyLoopConfig({ version: 1, initial_rage: adminInitialRage, sequence,
      macro: { mode: 'general', general: request.macro_text, shield: '', blade: '' }, macro_duration: request.macro_duration }, { skipSimulate: true });
    window._lastSimBody = clone(request);
    window.dispatchEvent(new CustomEvent('jx3-sim-complete', { detail: result }));
    presentSimResult(result);
    renderChrome(); changed();
    window.dispatchEvent(new CustomEvent('jx3-loop-pane-change', { detail: { id: target.id } }));
    return target.name;
  }
  function hide(id) {
    const tab = tabs.find(t => t.id === id); if (!tab || id === 'main') return;
    if (activeId === id) activate('main'); tab.hidden = true; tab.menu.open = false; renderChrome(); changed();
  }
  function remove(id) {
    const tab = tabs.find(t => t.id === id); if (!tab || id === 'main') return;
    if (!confirm(`删除「${tab.name}」？此页的循环内容将被移除。`)) return;
    if (activeId === id) activate('main'); tabs = tabs.filter(t => t !== tab); tab.pane.remove();
    for (const t of tabs) if (t.compareTo === id) t.compareTo = t.id === 'main' ? '' : 'main';
    renderChrome(); changed();
  }
  function rendered(event) {
    if (!ready || moving) return;
    const tab = current(); capture(); tab.result = event.detail; tab.resultScene = sceneKey(); tab.resultInput = JSON.stringify(tab.config); tab.bodyRequest = window._lastSimBody;
    changed();
  }
  window.addEventListener('jx3-sim-rendered', rendered);
  const observer = new MutationObserver(records => {
    if (moving || !ready) return;
    if (records.some(r => r.type === 'childList' || r.attributeName?.startsWith('data-'))) changed();
  });
  const displayObserver = new MutationObserver(() => {
    const scale = editor.style.getPropertyValue('--seq-scale') || '1';
    tabs.forEach(tab => tab.passive.style.setProperty('--seq-scale', scale)); scheduleDiff();
  });
  displayObserver.observe(editor, { attributes: true, attributeFilter: ['style'] });
  let displayMode = '';
  new MutationObserver(() => {
    const next = ['seq-mode-text', 'seq-mode-icon', 'seq-mode-iconext', 'seq-helpers-off', 'seq-time-on']
      .filter(name => document.body.classList.contains(name)).join(' ');
    if (next === displayMode) return;
    displayMode = next;
    if (!ready) return;
    const scene = sceneKey();
    for (const tab of tabs) if (tab.id !== activeId && resultFresh(tab, scene)) {
      const box = container(tab), top = box.scrollTop, left = box.scrollLeft;
      decorateSeqItems(tab.result, { container: box, readOnly: true, automaticDisplay: true });
      box.scrollTop = top; box.scrollLeft = left; syncedPositions.set(box, box.scrollTop);
    }
    scheduleDiff();
  }).observe(document.body, { attributes: true, attributeFilter: ['class'] });
  observer.observe(editor, { childList: true, subtree: true, attributes: true, attributeFilter: ['data-skill', 'data-timing-offset', 'data-qijin-buff', 'data-pre-time', 'data-solidified-cast', 'data-channel-ticks', 'data-clearcd-target'] });

  const resultFresh = (tab, scene) => !!tab.result && tab.resultScene === scene && tab.resultInput === JSON.stringify(tab.config);
  function comparisonData(tab, scene) {
    const items = [...container(tab).querySelectorAll('.sim-seq-item:not(.seq-pre-release):not(.seq-wait-stance):not(.seq-auto)')];
    const events = tab.result?.timeline?.filter(e => !e.triggered) || [];
    const fresh = !items.length || resultFresh(tab, scene);
    return { items, fresh, timeline: items.map(item => {
      const index = item.dataset.macroAssistIndex;
      const event = fresh && !item.classList.contains('seq-invalid') && index != null ? events[Number(index)] : null;
      return event || { name: item.dataset.resolvedSkill || item.dataset.skill, cast_time: null, ...(fresh ? { success: false } : {}) };
    }) };
  }
  function scheduleRefresh() {
    if (!ready || moving) return;
    clearTimeout(refreshTimer); refreshTimer = setTimeout(refreshBackground, 160);
  }
  async function refreshBackground() {
    if (!ready || moving || backgroundRunning) return;
    capture(); const scene = sceneKey();
    const tab = tabs.find(t => t.id !== activeId && (!t.pane.hidden || tabs.some(other => !other.pane.hidden && other.compareTo === t.id)) && !resultFresh(t, scene)
      && t.refreshAttempt !== JSON.stringify([scene, t.config]));
    if (!tab) return;
    const input = JSON.stringify(tab.config), attempt = JSON.stringify([scene, tab.config]);
    tab.refreshAttempt = attempt; tab.comparing = true; backgroundRunning = true;
    tab.compareError = ''; tab.pane.setAttribute('aria-busy', 'true'); scheduleDiff();
    try {
      const macro = tab.config.macro;
      const macroText = macro.mode === 'stance'
        ? `${macro.shield ? '#page shield\n' + macro.shield + '\n' : ''}${macro.blade ? '#page blade\n' + macro.blade : ''}`.trim()
        : (macro.general || '').trim();
      const body = buildSimulateRequest({ container: container(tab),
        macroText, macroDuration: tab.config.macro_duration });
      const response = await fetch(API.simulate, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body), signal: AbortSignal.timeout(5000) });
      if (!response.ok) throw new Error();
      const result = await response.json();
      if (sceneKey() !== scene || JSON.stringify(tab.config) !== input || !tabs.includes(tab)) return;
      tab.result = result; tab.resultScene = scene; tab.resultInput = input; tab.bodyRequest = body;
      // Focus may have changed while the request was in flight; never repaint the new foreground.
      if (tab.id !== activeId) {
        const box = container(tab), top = box.scrollTop, left = box.scrollLeft;
        decorateSeqItems(result, { container: box, readOnly: true, automaticDisplay: true }); tab.previewOnly = true;
        box.scrollTop = top; box.scrollLeft = left; syncedPositions.set(box, box.scrollTop);
      }
      tab.compareError = '';
    } catch { if (tabs.includes(tab) && sceneKey() === scene && JSON.stringify(tab.config) === input) tab.compareError = '回放失败，请重新回放'; }
    finally {
      tab.comparing = false; backgroundRunning = false; tab.pane.removeAttribute('aria-busy');
      scheduleDiff(); scheduleRefresh();
    }
  }
  function clearDiff() {
    for (const tab of tabs) {
      tab.diff.textContent = ''; tab.diff.hidden = true; tab.first.hidden = true;
      tab.firstNodes = [];
      tab.retry.hidden = !tab.compareError;
      container(tab).querySelectorAll('[data-loop-diff]').forEach(item => { delete item.dataset.loopDiff; item.classList.remove('ltab-diff-focus'); item.style.removeProperty('--ltab-band-left'); item.style.removeProperty('--ltab-band-right'); });
    }
  }
  function connectBands(box) {
    const items = [...box.children].filter(n => n.classList.contains('sim-seq-item'));
    for (let i = 1; i < items.length; i++) {
      const a = items[i - 1], b = items[i]; if (!a.dataset.loopDiff || a.dataset.loopDiff !== b.dataset.loopDiff) continue;
      const ar = a.getBoundingClientRect(), br = b.getBoundingClientRect();
      if (Math.abs(ar.top - br.top) > 3 || br.left < ar.right || br.left - ar.right > 48) continue;
      const half = (br.left - ar.right) / 2 + 1; a.style.setProperty('--ltab-band-right', `${half}px`); b.style.setProperty('--ltab-band-left', `${half}px`);
    }
  }
  function renderDiff() {
    scrollLinks = [];
    clearDiff(); if (!current() || macroModeActive || !ready) return;
    capture(); const scene = sceneKey(), alignSources = [];
    // All visible comparisons survive focus changes; draw the foreground pair last.
    const comparisons = visibleTabs().filter(t => t.compareTo).sort((a, b) => Number(a.id === activeId) - Number(b.id === activeId));
    for (const tab of comparisons) {
    const other = tabs.find(t => t.id === tab.compareTo); if (!other || other === tab) continue;
    const a = comparisonData(other, scene), b = comparisonData(tab, scene); tab.diff.hidden = false;
    const pendingMacro = [a, b].some(data => !data.fresh && data.items.some(item => item.dataset.skill === '__macro__'));
    if (pendingMacro) { tab.diff.textContent = `对比 ${other.name} · ${tab.compareError || other.compareError || '正在回放…'}`; continue; }
    try {
      const aligned = window.Jx3MacroAlignment.align(a.timeline, b.timeline);
      const states = tab.layer === 'states' && a.fresh && b.fresh;
      for (const row of aligned.rows) {
        if (row.kind === 'missing' || (states && row.kind === 'changed')) a.items[row.referenceIndex].dataset.loopDiff = 'removed';
        if (row.kind === 'extra' || (states && row.kind === 'changed')) b.items[row.actualIndex].dataset.loopDiff = 'added';
        if (row.kind === 'missing' || row.kind === 'extra' || (states && row.kind === 'changed')) {
          if (row.referenceIndex != null) tab.firstNodes.push(a.items[row.referenceIndex]);
          if (row.actualIndex != null) tab.firstNodes.push(b.items[row.actualIndex]);
        }
      }
      const s = aligned.summary;
      tab.diff.textContent = `对比 ${other.name}：少 ${s.missing} · 多 ${s.extra}${states ? ` · 状态 ${s.changed}` : ''}${tab.layer === 'states' && !states ? ` · ${tab.compareError || other.compareError || '正在回放状态…'}` : ''}`;
      tab.diff.title = tab.diff.textContent;
      tab.first.hidden = !s.missing && !s.extra && !(states && s.changed);
      if (tab.syncScroll && !other.pane.hidden) {
        const left = container(other), right = container(tab), lm = Math.max(0, left.scrollHeight - left.clientHeight), rm = Math.max(0, right.scrollHeight - right.clientHeight);
        const ly = left.getBoundingClientRect().top, ry = right.getBoundingClientRect().top;
        const points = [[0, 0]];
        // Reuse redline matches, so inserted skills and different wrapping do not
        // accumulate drift. Geometry is rebuilt on edits/layout, never per scroll tick.
        for (const row of aligned.rows) {
          if (row.referenceIndex == null || row.actualIndex == null) continue;
          const x = Math.min(lm, Math.max(0, a.items[row.referenceIndex].getBoundingClientRect().top - ly + left.scrollTop));
          const y = Math.min(rm, Math.max(0, b.items[row.actualIndex].getBoundingClientRect().top - ry + right.scrollTop));
          const last = points[points.length - 1];
          if (x > last[0] && y > last[1] && x < lm && y < rm) points.push([x, y]);
        }
        points.push([lm, rm]); scrollLinks.push({ left, right, points });
        if (tab.alignScroll) { tab.alignScroll = false; alignSources.push(right); }
      }
    } catch (error) { tab.diff.textContent = error.message; }
    }
    visibleTabs().forEach(tab => connectBands(container(tab)));
    alignSources.forEach(source => syncFrom(source));
  }
  function scheduleDiff() { clearTimeout(diffTimer); diffTimer = setTimeout(renderDiff, 80); }
  function syncFrom(source, visited = new Set()) {
    if (moving || visited.has(source)) return;
    visited.add(source);
    for (const link of scrollLinks) {
    const index = source === link.left ? 0 : source === link.right ? 1 : -1;
    if (index < 0) continue;
    const target = index === 0 ? link.right : link.left, p = link.points, value = source.scrollTop;
    if (visited.has(target)) continue;
    let low = p[0], high = p[p.length - 1];
    for (let i = 1; i < p.length; i++) { if (p[i][index] >= value) { low = p[i - 1]; high = p[i]; break; } }
    const span = high[index] - low[index], fraction = span > 0 ? Math.max(0, Math.min(1, (value - low[index]) / span)) : 0;
    const position = low[1 - index] + fraction * (high[1 - index] - low[1 - index]);
    if (Math.abs(target.scrollTop - position) >= .5) {
      target.scrollTop = position; syncedPositions.set(target, target.scrollTop);
    }
    syncFrom(target, visited);
    }
  }
  strip.addEventListener('scroll', event => {
    const source = event.target;
    if (!scrollLinks.length || moving || !scrollLinks.some(link => source === link.left || source === link.right)) return;
    const expected = syncedPositions.get(source); syncedPositions.delete(source);
    if (expected != null && Math.abs(source.scrollTop - expected) < .5) return;
    scrollSource = source;
    if (scrollFrame == null) scrollFrame = requestAnimationFrame(() => { scrollFrame = null; syncFrom(scrollSource); });
  }, true);
  function firstDifference(tab) {
    const other = tabs.find(t => t.id === tab.compareTo);
    for (const t of [tab, other].filter(Boolean)) {
      const item = tab.firstNodes?.find(item => container(t).contains(item));
      if (item) { item.classList.add('ltab-diff-focus'); item.scrollIntoView({ block: 'nearest', inline: 'nearest', behavior: 'smooth' }); }
    }
  }
  window.addEventListener('jx3-macro-assist-mode', event => { macroModeActive = !!event.detail?.active; renderChrome(); });
  window.addEventListener('jx3-macro-layout-change', scheduleDiff);
  new ResizeObserver(scheduleDiff).observe(strip);
  document.addEventListener('change', event => { if (!bar.contains(event.target) && !strip.contains(event.target)) { changed(); } });
  document.addEventListener('pointerdown', event => {
    tabs.forEach(tab => { if (!tab.menu.contains(event.target)) tab.menu.open = false; });
    if (!bar.contains(event.target) && !event.target.closest('.ltab-heading')) showTools(false);
  }, true);
  window.addEventListener('pagehide', stash);
  document.addEventListener('visibilitychange', () => { if (document.hidden) { stash(); save(); } });
  window.addEventListener('focus', async () => {
    if (!ready || !identity) return;
    try { if (await getIdentity() !== identity) { dirty = false; location.reload(); } } catch { /* offline data stays in the same user namespace */ }
  });
  window.Jx3LoopTabs = { get activeId() { return activeId; }, get contextToken() { return epoch; }, ownsAutosave: () => ready,
    activate, add, hide, save, snapshot: documentValue, outputStamp, importMacro,
    list: () => ready ? [...tabs].sort((a, b) => a.position - b.position).map(t => ({ id: t.id, name: t.name, hidden: t.hidden })) : [] };

  async function initialize() {
    await Promise.all([currentMountReady, attributesReady, window.Jx3LoopAutosave?.ready]);
    scope = `${currentMount.version}/${currentMount.mount}`;
    endpoint = `/api/loop-tabs?${new URLSearchParams({ version: currentMount.version, mount: currentMount.mount })}`;
    let data = null, recoveredLocal = false;
    try {
      identity = await getIdentity();
      if (identity) {
        try { const r = await fetch(endpoint, { cache: 'no-store', signal: AbortSignal.timeout(5000) }); if (!r.ok) throw new Error(); data = await r.json(); }
        catch { /* A verified user's local copy can recover offline edits. */ }
        try {
          const local = JSON.parse(localStorage.getItem(storageKey()) || 'null');
          if (local?.schema === 1 && (!data || (local.updatedAt || 0) > (data.updatedAt || 0))) { data = local; recoveredLocal = true; }
        } catch {}
      }
    } catch { status.textContent = '仅本页保留'; }
    const saved = data?.schema === 1 && Array.isArray(data.tabs) && data.tabs.length ? data : null;
    updatedAt = Number(data?.updatedAt) || 0;
    if (saved?.scene?.version === 1) {
      applyLoopConfig({ ...saved.scene, sequence: [] }, { skipSimulate: true });
    } else if (!saved && identity) {
      // The old unscoped browser autosave could belong to a previous login. Authenticated
      // migration trusts only the settings received for this account during bootstrap.
      let legacy = null;
      try {
        const raw = identity === 'local' ? localStorage.getItem('jx3_autosave_loop_v1') : window.Jx3BootSettings?.jx3_autosave_loop_v1;
        legacy = raw ? JSON.parse(raw) : null;
      } catch {}
      if (legacy?.version === 1 && (!legacy._mount || legacy._mount === currentMount.mount)
          && (!legacy._version || legacy._version === currentMount.version)) {
        applyLoopConfig(legacy, { skipSimulate: true });
      }
    }
    const initial = sequenceConfig(); const initialResult = lastSimResult;
    const records = saved?.tabs || [{ id: 'main', name: '主循环', ...initial }];
    const seen = new Set();
    records.forEach((value, index) => {
      if (!value || typeof value !== 'object' || (index && (typeof value.id !== 'string' || seen.has(value.id)))) return;
      const tab = createTab(value, tabs.length); seen.add(tab.id); tabs.push(tab);
    });
    if (!tabs.length) tabs.push(createTab({ name: '主循环', ...initial }, 0));
    for (const tab of tabs) if (tab.compareTo === tab.id || tab.compareTo && !tabs.some(other => other.id === tab.compareTo)) {
      tab.compareTo = tab.id === 'main' ? '' : 'main';
    }
    if (!saved) {
      const tab = tabs[0]; tab.initialized = true; tab.channels = { ...channelOverrides }; tab.result = initialResult;
      tab.resultScene = sceneKey(); tab.resultInput = JSON.stringify(tab.config); tab.bodyRequest = window._lastSimBody;
      tab.passive.remove(); tab.body.append(editor, bar); if (toolbar) tab.body.append(toolbar);
    } else {
      // Build each pane with the existing item factories once; handlers and item identities survive every switch.
      moving = true; editor.replaceChildren(); moving = false;
      for (const tab of tabs) activate(tab.id, { silent: true, force: true });
      for (const tab of tabs) tab.hidden = tab.id !== 'main' && !!records.find(r => r.id === tab.id)?.hidden;
      const chosen = tabs.find(t => t.id === saved.active && !t.hidden)?.id || 'main';
      activate(chosen, { silent: true, force: true });
    }
    ready = true; observer.takeRecords(); addCopy.disabled = addEmpty.disabled = false;
    renderChrome(); status.textContent = identity ? '已载入' : '仅本页保留';
    runSimulate();
    // Migration creates only this account's new workspace; legacy snapshots remain available.
    if (!saved || recoveredLocal) changed();
  }
  initialize().catch(() => { status.textContent = '循环页未能载入，请刷新重试'; });
})();
