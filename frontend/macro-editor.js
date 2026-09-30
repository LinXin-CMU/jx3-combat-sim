/* 写宏模式中的独立草稿与真实循环对照。运行结果不写回手工序列或正式宏。 */
(function () {
  'use strict';
  const byId = id => document.getElementById(id);
  const draft = byId('macro_draft_text'), editor = byId('macro_editor_panel');
  const compare = byId('macro_compare_panel'), sequence = byId('macro_compare_sequence');
  const referenceSequence = byId('sim_sequence');
  if (!draft || !editor || !compare || !sequence || !referenceSequence) return;
  const count = byId('macro_draft_count'), status = byId('macro_draft_status');
  const resultStatus = byId('macro_compare_status'), summary = byId('macro_compare_summary');
  const detail = byId('macro_compare_detail'), runButton = byId('macro_compare_run');
  const firstButton = byId('macro_compare_first'), undoButton = byId('macro_draft_undo');
  const layerSelect = byId('macro_compare_layer'), syncToggle = byId('macro_compare_sync');
  const draftPanes = window.Jx3MacroDraftPanes?.create(draft, byId('macro_draft_shield'), byId('macro_draft_blade')) || null;
  const memory = new Map();
  let identity = null, scope = '', previousText = '', history = [], active = false;
  let result = null, selectedRow = null, generation = 0, controller = null, running = false, runningKey = '';
  let detailSource = null, detailRow = null, detailStale = null;
  let runError = '';
  let expectedScroll = null, suppressSyncUntil = 0;
  let authGeneration = 0, renderTimer = null, environmentTimer = null, storageFailed = false, hasConfirmedIdentity = false;
  const MAX_DURATION = 1200, MAX_ACTIVE = 2048;
  const node = (tag, text, className) => {
    const item = document.createElement(tag);
    if (text != null) item.textContent = text;
    if (className) item.className = className;
    return item;
  };
  const seconds = value => Number.isFinite(value) ? `${value.toFixed(1)}s` : '未记录';
  const button = (text, action) => {
    const item = node('button', text, 'ma-button'); item.type = 'button';
    item.addEventListener('click', action); return item;
  };
  const contextKey = () => window.Jx3MacroAssist?.contextKey() || '';
  const sameEnvironment = () => { try { return !!result && result.key === contextKey(); } catch { return false; } };
  const currentScope = () => JSON.stringify([identity || 'memory-only', currentMount.version, currentMount.mount]);
  const storageKey = key => `jx3_macro_draft_v1:${key}`;
  const isActiveEvent = event => event && !event.triggered && event.success !== false && event.cast_success !== false
    && event.failed !== true && event.name && !event.name.startsWith('__') && !event.name.startsWith('移除气劲')
    && event.name !== '清除冷却' && event.skill_id !== 90001;
  const within = (event, duration) => Number.isFinite(event?.cast_time) && event.cast_time >= 0 && event.cast_time < duration;
  const showStates = () => layerSelect?.value === 'states';
  const visibleDifference = row => row.kind === 'missing' || row.kind === 'extra' || (showStates() && row.kind === 'changed');
  const firstVisibleDifference = () => result?.alignment.rows.findIndex(visibleDifference) ?? -1;
  function setDraftText(text) { if (draftPanes) draftPanes.setText(text); else draft.value = text; }
  function focusDraft() { if (draftPanes) draftPanes.focus(); else if (!draft.hidden) draft.focus(); }
  function selectDraftRange(start, end = start, focus = true) {
    if (focus) window.Jx3MacroLayout?.tab('draft');
    if (draftPanes) draftPanes.selectRange(start, end, focus);
    else { draft.setSelectionRange(start, end); if (focus) focusDraft(); }
  }

  function saveDraft() {
    if (!scope) return;
    memory.set(scope, draft.value);
    if (!identity || storageFailed || JSON.parse(scope)[0] !== identity) return;
    try { localStorage.setItem(storageKey(scope), draft.value); }
    catch { storageFailed = true; }
  }
  function syncScope() {
    let next;
    try { next = currentScope(); } catch { return; }
    if (scope === next) return;
    saveDraft(); scope = next;
    let text = memory.get(scope);
    if (text == null && identity && !storageFailed) {
      try { text = localStorage.getItem(storageKey(scope)); } catch { storageFailed = true; }
    }
    setDraftText(text || ''); previousText = draft.value; history = [];
    invalidateRun(); runError = ''; result = null; selectedRow = null;
    clearFocus(); clearReferenceMarks(); clearBands(sequence); detail.hidden = true;
    sequence.replaceChildren(node('p', '在下方编写宏，点击“运行对照”。', 'ma-empty'));
    summary.textContent = '缺失 / 多放与时间、资源差异会标在原技能样式上。';
    firstButton.disabled = true; resultStatus.textContent = '运行宏后显示';
    updateDraftStatus();
  }
  async function refreshIdentity() {
    const token = ++authGeneration;
    const authAbort = new AbortController(), timeout = setTimeout(() => authAbort.abort(), 5000);
    let nextIdentity = null;
    try {
      const response = await fetch('/api/auth/me', { cache: 'no-store', signal: authAbort.signal });
      if (!response.ok) throw new Error('auth unavailable');
      const auth = await response.json();
      if (auth.enabled === false) nextIdentity = 'local';
      else if (auth.authed === true && typeof auth.username === 'string' && auth.username) nextIdentity = `user:${auth.username}`;
    } catch { /* 未核实身份时只保留当前页面内存，不能读取另一个账号的草稿。 */ }
    finally { clearTimeout(timeout); }
    if (token !== authGeneration) return;
    if (identity !== nextIdentity) {
      // 仅初次身份查询允许接续此页刚输入的草稿；已确认账号之间绝不迁移。
      const pendingText = !hasConfirmedIdentity && identity == null && nextIdentity ? draft.value : '';
      if (scope) memory.set(scope, draft.value);
      identity = nextIdentity;
      if (pendingText) memory.set(currentScope(), pendingText);
      if (identity) hasConfirmedIdentity = true;
      syncScope();
      if (pendingText) saveDraft();
    }
    updateDraftStatus();
  }
  function updateDraftStatus() {
    const lines = draft.value ? draft.value.split(/\r?\n/).length : 0;
    count.textContent = `${draft.value.length} 字符 · ${lines} 行`;
    undoButton.disabled = !history.length;
    if (!running) {
      const paneNote = draftPanes?.description();
      status.textContent = runError || (identity && !storageFailed ? '草稿已在当前浏览器按账号、版本与心法保留' : '草稿仅在当前页面内存保留')
        + (paneNote ? ` · ${paneNote}` : '');
    }
    if (result) {
      const stale = result.text !== draft.value || !sameEnvironment();
      resultStatus.textContent = running ? '正在运行…' : runError ? '运行未完成 · 保留上次结果' : stale ? '旧结果 · 尚未重跑' : '当前草稿 · 已运行';
      if (selectedRow != null && !detail.hidden) showDetail(selectedRow, false);
    }
  }
  function invalidateRun() {
    generation++; controller?.abort(); controller = null; running = false; runningKey = '';
    runButton.disabled = false; runButton.textContent = '运行对照';
  }
  function changed() {
    if (draft.value !== previousText) {
      history.push(previousText); if (history.length > 80) history.shift();
      previousText = draft.value; invalidateRun(); runError = ''; saveDraft();
    }
    updateDraftStatus();
  }
  function replaceDraft(text) {
    window.Jx3MacroLayout?.tab('draft');
    syncScope();
    setDraftText(text); selectDraftRange(draft.value.length, draft.value.length, false); changed();
  }
  function insert(text) {
    if (typeof text !== 'string' || !text.trim()) return false;
    window.Jx3MacroLayout?.tab('draft');
    syncScope();
    if (draftPanes) return draftPanes.insert(text);
    const start = draft.selectionStart, end = draft.selectionEnd;
    const before = draft.value.slice(0, start), after = draft.value.slice(end);
    const prefix = before && !before.endsWith('\n') ? '\n' : '';
    const suffix = after && !after.startsWith('\n') ? '\n' : '';
    draft.setRangeText(prefix + text.trim() + suffix, start, end, 'end');
    changed(); focusDraft(); return true;
  }
  function undo() {
    if (!history.length) return;
    setDraftText(history.pop()); previousText = draft.value;
    selectDraftRange(draft.value.length, draft.value.length, false);
    invalidateRun(); runError = ''; saveDraft(); updateDraftStatus(); focusDraft();
  }
  async function copy() {
    const text = draft.value;
    try {
      if (navigator.clipboard?.writeText && window.isSecureContext) await navigator.clipboard.writeText(text);
      else {
        const area = node('textarea', null, 'ma-copy-buffer'); area.value = text;
        document.body.append(area); area.select();
        const copied = document.execCommand('copy'); area.remove();
        if (!copied) throw new Error('copy unavailable');
      }
      status.textContent = '已复制草稿';
    } catch { status.textContent = '浏览器未允许复制，可在编辑区选中文本后复制。'; }
  }
  function physicalLines(text) {
    const pages = [], current = []; let offset = 0;
    for (const [index, line] of text.split('\n').entries()) {
      const trimmed = line.trim();
      if (trimmed.startsWith('#page')) {
        if (current.length) pages.push(current.splice(0));
      } else if (trimmed && !trimmed.startsWith('//')) {
        current.push({ number: index + 1, start: offset, end: offset + line.length, text: line });
      }
      offset += line.length + 1;
    }
    if (current.length) pages.push(current);
    return pages;
  }
  function clearFocus() {
    referenceSequence.querySelectorAll('.ma-diff-focus').forEach(item => item.classList.remove('ma-diff-focus'));
    sequence.querySelectorAll('.ma-diff-focus').forEach(item => item.classList.remove('ma-diff-focus'));
  }
  function clearReferenceMarks() {
    clearBands(referenceSequence);
    referenceSequence.querySelectorAll('.ma-compare-gap').forEach(item => item.remove());
    referenceSequence.querySelectorAll('.ma-reference-missing, .ma-reference-changed')
      .forEach(item => item.classList.remove('ma-reference-missing', 'ma-reference-changed'));
  }
  function clearBands(container) {
    container.querySelectorAll('.ma-band-start, .ma-band-end, [style*="--ma-band-"]').forEach(item => {
      item.classList.remove('ma-band-start', 'ma-band-end');
      item.style.removeProperty('--ma-band-left'); item.style.removeProperty('--ma-band-right');
    });
  }
  function connectBands(container) {
    clearBands(container);
    let previous = null;
    for (const item of container.children) {
      if (item.matches('.seq-break-wrap, .seq-break')) { previous = null; continue; }
      if (!item.classList.contains('sim-seq-item')) continue;
      const color = item.matches('.ma-reference-missing, .ma-reference-changed') ? 'red'
        : item.matches('.ma-redline-extra, .ma-redline-changed') ? 'green' : null;
      const bounds = item.getBoundingClientRect();
      if (!color || bounds.width <= 0 || bounds.height <= 0) { previous = null; continue; }
      item.classList.add('ma-band-start', 'ma-band-end');
      item.style.setProperty('--ma-band-left', '0px'); item.style.setProperty('--ma-band-right', '0px');
      if (previous && previous.color === color && Math.abs(bounds.top - previous.bounds.top) <= 1
        && bounds.left >= previous.bounds.right - 1) {
        const extension = Math.max(0, bounds.left - previous.bounds.right) / 2;
        previous.item.style.setProperty('--ma-band-right', `${extension}px`);
        item.style.setProperty('--ma-band-left', `${extension}px`);
        previous.item.classList.remove('ma-band-end'); item.classList.remove('ma-band-start');
      }
      previous = { item, bounds, color };
    }
  }
  function displayClone(item) {
    const clone = item.cloneNode(true);
    for (const child of [clone, ...clone.querySelectorAll('*')]) {
      child.removeAttribute('id'); child.removeAttribute('draggable');
      child.removeAttribute('data-macro-assist-index'); child.removeAttribute('data-sequence-index');
      child.classList.remove('ma-selected', 'ma-occurrence', 'ma-focus', 'ma-extra', 'seq-selected', 'ma-diff-focus');
    }
    clone.querySelectorAll('.seq-delete').forEach(child => child.remove());
    return clone;
  }
  function referenceMap(template) {
    const map = new Map(); let activeIndex = 0;
    template.timeline.forEach((event, index) => {
      if (!event.triggered) {
        const item = referenceSequence.querySelector(`[data-macro-assist-index="${activeIndex++}"]`);
        if (item) map.set(index, item);
      }
    });
    return map;
  }
  function captureTemplate(template) {
    const originals = referenceMap(template), breaks = new Map(), prefix = [];
    for (const [index, item] of originals) {
      const following = [];
      for (let next = item.nextElementSibling; next && !next.classList.contains('sim-seq-item'); next = next.nextElementSibling) {
        if (next.classList.contains('seq-break-wrap')) following.push(displayClone(next));
      }
      if (following.length) breaks.set(index, following);
    }
    for (const item of referenceSequence.children) {
      if (item.matches('.sim-seq-item:not(.seq-pre-release)')) break;
      if (item.matches('.seq-pre-release, .seq-break-wrap')) prefix.push(displayClone(item));
    }
    return { breaks, prefix };
  }
  async function run() {
    syncScope(); invalidateRun(); runError = '';
    const text = draft.value, token = generation, runScope = scope;
    if (!text.trim()) { status.textContent = '请先输入宏。'; focusDraft(); return null; }
    let key;
    try { key = contextKey(); } catch { status.textContent = '模板环境尚未准备好，请稍后重试。'; return null; }
    if (!key) { status.textContent = '写宏助手尚未准备好，请稍后重试。'; return null; }
    running = true; runningKey = key; runButton.disabled = true; runButton.textContent = '运行中…';
    resultStatus.textContent = '正在运行…'; status.textContent = '正在取得同环境模板…';
    const stillCurrent = () => token === generation && text === draft.value && scope === runScope && key === contextKey();
    let timeout, completed = false;
    try {
      let template = typeof lastSimResult === 'undefined' ? null : lastSimResult;
      if (!template?._macroAssistBody || template._macroAssistKey !== key) template = await runSimulate();
      if (!stillCurrent()) {
        if (token === generation) throw new Error('配置已变化，请重新运行对照。');
        return null;
      }
      if (!template?._macroAssistBody || template._macroAssistKey !== key || template !== lastSimResult) {
        throw new Error('未取得当前模板的完整模拟环境，请确认左侧序列后重试。');
      }
      const duration = template.fight_time;
      if (!Number.isFinite(duration) || duration <= 0) throw new Error('模板对照时长为零，请先添加可形成完整时段的技能序列。');
      if (duration > MAX_DURATION) throw new Error(`模板超过 ${MAX_DURATION} 秒，请缩短序列后重试。`);
      const templateActive = template.timeline.filter(isActiveEvent);
      if (!templateActive.length) throw new Error('模板没有成功释放的主动技能。');
      if (templateActive.length > MAX_ACTIVE) throw new Error(`模板超过 ${MAX_ACTIVE} 个主动技能，请缩短序列后重试。`);
      const body = JSON.parse(JSON.stringify(template._macroAssistBody));
      body.sequence = Array(Math.ceil(duration / 0.25) + 20).fill('__macro__');
      body.macro_text = text; body.macro_duration = duration; body.lite = false; body.lite_keep_timeline = false;
      body.channel_ticks = {}; body.timing_offsets = {}; body.qijin_buffs = {};
      const snapshots = captureTemplate(template);
      const abort = new AbortController(); controller = abort;
      timeout = setTimeout(() => abort.abort(), 60000);
      status.textContent = `正在运行 ${seconds(duration)} 的宏对照…`;
      const response = await fetch('/api/simulate', {
        method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body), signal: abort.signal,
      });
      const actual = await response.json().catch(() => null);
      if (!response.ok) throw new Error(actual?.error || `宏运行失败（${response.status}）。`);
      if (!stillCurrent()) {
        if (token === generation) throw new Error('配置已变化，请重新运行对照。');
        return null;
      }
      if (!actual || !Array.isArray(actual.timeline)) throw new Error('宏运行没有返回有效时间轴。');
      const actualActive = actual.timeline.filter(isActiveEvent);
      if (!actualActive.length && actual.skipped?.length) throw new Error(actual.skipped[0][1] || '宏运行失败。');
      if (actualActive.length > MAX_ACTIVE) throw new Error(`实际循环超过 ${MAX_ACTIVE} 个主动技能，请缩短模板后重试；未截断结果。`);
      const referenceWindow = template.timeline.map(event => within(event, duration) ? event : null);
      const actualWindow = actual.timeline.map(event => within(event, duration) ? event : null);
      const alignment = window.Jx3MacroAlignment.align(referenceWindow, actualWindow);
      result = { template, actual, text, key, request: body, version: currentMount.version, mount: currentMount.mount, window: duration, alignment, pages: physicalLines(text), ...snapshots,
        outsideReference: templateActive.filter(event => !within(event, duration)).length,
        outsideActual: actualActive.filter(event => !within(event, duration)).length };
      selectedRow = null; detail.hidden = true; completed = true; renderResult();
      const first = firstVisibleDifference();
      if (alignment.rows.length) focusRow(first >= 0 ? first : 0);
      return result;
    } catch (error) {
      if (token === generation) {
        runError = error.name === 'AbortError' ? '宏运行超时，请缩短模板后重试。' : error.message;
        status.textContent = runError;
        resultStatus.textContent = result ? '运行未完成 · 保留上次结果' : '运行未完成';
      }
      return null;
    } finally {
      clearTimeout(timeout);
      if (token === generation) {
        running = false; runningKey = ''; controller = null; runButton.disabled = false; runButton.textContent = '运行对照';
        if (completed) updateDraftStatus();
      }
    }
  }
  function renderResult() {
    if (!result || !active) return;
    const scrollPositions=[referenceSequence.scrollTop,sequence.scrollTop];
    clearReferenceMarks(); clearFocus();
    const saved = result, aligned = saved.alignment;
    sequence.replaceChildren();
    sequence.style.setProperty('--seq-scale', getComputedStyle(referenceSequence).getPropertyValue('--seq-scale').trim() || '1');
    const actualNodes = new Map();
    const wanted = new Set(aligned.rows.filter(row => row.actualIndex != null).map(row => row.actualIndex));
    const displayTimeline = saved.actual.timeline.filter((event, index) => wanted.has(index) || (event.triggered && within(event, saved.window)));
    for (const index of wanted) {
      const item = node('div', null, 'sim-seq-item seq-macro'); item.dataset.skill = '__macro__';
      item.innerHTML = _buildSeqItemInner('__macro__', { macro: true });
      item.querySelectorAll('.seq-delete').forEach(child => child.remove());
      actualNodes.set(index, item); sequence.append(item);
    }
    decorateSeqItems({ ...saved.actual, timeline: displayTimeline }, { container: sequence, readOnly: true });
    const currentMap = sameEnvironment() ? referenceMap(saved.template) : new Map();
    const fragment = document.createDocumentFragment();
    for (const item of saved.prefix) {
      const clone = displayClone(item); clone.title = '模板与宏共用的预释放配置'; fragment.append(clone);
    }
    let hasActual = false, pendingBreak = null;
    aligned.rows.forEach((row, index) => {
      const reference = currentMap.get(row.referenceIndex);
      if (reference && visibleDifference(row) && (row.kind === 'missing' || row.kind === 'changed')) reference.classList.add(`ma-reference-${row.kind}`);
      const item = row.actualIndex == null ? null : actualNodes.get(row.actualIndex);
      if (item) {
        // 只在两段实际释放之间换行；跳过模板缺失段落时不生成空行或虚构技能。
        if (hasActual && pendingBreak) fragment.append(displayClone(pendingBreak));
        pendingBreak = null;
        item.removeAttribute('data-macro-assist-index'); item.dataset.compareRow = String(index);
        item.tabIndex = 0; item.setAttribute('role', 'button');
        if (visibleDifference(row)) {
          item.classList.add(`ma-redline-${row.kind}`);
        }
        item.setAttribute('aria-label', `${({ extra: '额外释放', changed: '时间或状态不同', same: '窗口内匹配' })[row.kind]}：${saved.actual.timeline[row.actualIndex]?.name || '技能'}`);
        fragment.append(item); hasActual = true;
      }
      const lineBreak = saved.breaks.get(row.referenceIndex)?.[0];
      if (hasActual && lineBreak) pendingBreak = lineBreak;
    });
    sequence.replaceChildren(fragment);
    if (sameEnvironment()) connectBands(referenceSequence);
    connectBands(sequence);
    const stats = aligned.summary;
    summary.textContent = `共同窗口 0 ≤ t < ${seconds(saved.window)} · ${showStates() ? '含状态差异' : '技能差异'} · 缺失 ${stats.missing} · 多放 ${stats.extra}`
      + (showStates() ? ` · 时间 / 资源 / 档位差异 ${stats.changed}` : ` · 已隐藏时间 / 资源 / 档位差异 ${stats.changed} 处`)
      + ` · 窗外主动释放：模板 ${saved.outsideReference}，实际 ${saved.outsideActual}（含恰好在终点的释放）`;
    firstButton.disabled = firstVisibleDifference() < 0;
    expectedScroll=null;suppressSyncUntil=performance.now()+180;
    referenceSequence.scrollTop=scrollPositions[0];sequence.scrollTop=scrollPositions[1];
    if (selectedRow != null) focusRow(selectedRow, false, true);
    updateDraftStatus();
  }
  function showDetail(index, locate) {
    if (!result) return;
    const row = result.alignment.rows[index]; if (!row) return;
    const stale = draft.value !== result.text || !sameEnvironment();
    if (!detail.hidden && detailSource === result && detailRow === index && detailStale === stale) return;
    detailSource = result; detailRow = index; detailStale = stale;
    const reference = row.referenceIndex == null ? null : result.template.timeline[row.referenceIndex];
    const actual = row.actualIndex == null ? null : result.actual.timeline[row.actualIndex];
    detail.hidden = false; detail.replaceChildren();
    const focus = window.Jx3MacroRepair.intent(result, index);
    if (window.Jx3MacroDiagnostic) {
      const saved = result;
      window.Jx3MacroDiagnostic.mount(detail, {
        saved, focus, reference: focus.reference, actual: focus.actual,
        current: () => result === saved && draft.value === saved.text && sameEnvironment(),
        locate: (page, statement) => {
          const source = saved.pages[page - 1]?.[statement - 1];
          if (source) selectDraftRange(source.start, source.end);
        },
        first: () => { if (focus.firstSkill >= 0) focusRow(focus.firstSkill); },
        apply: async text => { if (result !== saved || draft.value !== saved.text || !sameEnvironment()) return;
          replaceDraft(text); await run(); },
      });
    }
    const stateDetails = node('details', null, 'ma-state-disclosure');
    stateDetails.append(node('summary', '释放前状态与原宏行'));
    detail.append(stateDetails);
    const names = { same: '窗口内匹配', missing: '模板技能缺失', extra: '实际额外释放', changed: '时间或状态不同' };
    stateDetails.append(node('strong', names[row.kind]));
    if (row.timeDelta != null && row.kind === 'changed') stateDetails.append(node('p', `实际相对模板：${row.timeDelta > 0 ? '延后' : row.timeDelta < 0 ? '提前' : '同一时刻'} ${seconds(Math.abs(row.timeDelta))}`));
    stateDetails.append(window.Jx3MacroStateDiff.render(reference, actual));
    const footer = node('div', null, 'ma-detail-footer');
    const line = actual ? result.pages[actual.macro_page - 1]?.[actual.macro_line - 1] : null;
    if (line) {
      footer.append(node('code', line.text, 'ma-detail-macro'));
      const visibleLine = draftPanes?.location(line.start, line.end);
      const go = button(visibleLine ? `定位${visibleLine.label}第 ${visibleLine.line} 行` : `定位宏第 ${line.number} 行`, () => {
        if (draft.value !== result.text) return;
        selectDraftRange(line.start, line.end);
      });
      go.disabled = draft.value !== result.text;
      if (go.disabled) go.title = '草稿已修改，重跑后才能定位当前宏行。';
      footer.append(go);
    }
    footer.append(node('p', '这里列出实际记录的时间、状态和执行语句；条件匹配统计不能解释全部施放原因。', 'ma-explanation'));
    stateDetails.append(footer);
    if (locate) detail.scrollIntoView({ block: 'nearest' });
  }
  function focusRow(index, scroll = true, preserveTab = false) {
    if (!result || !result.alignment.rows[index]) return;
    selectedRow = index; clearFocus();
    const row = result.alignment.rows[index];
    const actual = sequence.querySelector(`[data-compare-row="${index}"]`);
    const reference = sameEnvironment() && row.referenceIndex != null ? referenceMap(result.template).get(row.referenceIndex) : null;
    if (scroll) { expectedScroll = null; suppressSyncUntil = performance.now() + 180; }
    for (const item of [actual, reference]) {
      item?.classList.add('ma-diff-focus');
    }
    if (scroll) {
      const map = sameEnvironment() ? referenceMap(result.template) : new Map();
      const rows = result.alignment.rows;
      const nearby = field => {
        for (let i=index; i<rows.length; i++) if (rows[i][field]!=null) return {row:i,index:rows[i][field]};
        for (let i=index-1; i>=0; i--) if (rows[i][field]!=null) return {row:i,index:rows[i][field]};
        return null;
      };
      const refAnchor=nearby('referenceIndex'), actualAnchor=nearby('actualIndex');
      for (const [container,item] of [[referenceSequence,refAnchor && map.get(refAnchor.index)],
        [sequence,actualAnchor && sequence.querySelector(`[data-compare-row="${actualAnchor.row}"]`)]]) {
        if (!item) { container.scrollTop=0; continue; }
        const box=item.getBoundingClientRect(), bounds=container.getBoundingClientRect();
        container.scrollTop += box.top-bounds.top-container.clientTop-(container.clientHeight-box.height)/2;
      }
    }
    if (!preserveTab) window.Jx3MacroLayout?.tab('diagnosis');
    showDetail(index, false);
  }
  function syncScroll(from, to) {
    if (!active || !result || !sameEnvironment() || syncToggle?.checked === false
      || !document.hasFocus() || document.hidden || performance.now() < suppressSyncUntil) return;
    if (expectedScroll?.target === from && Math.abs(from.scrollTop - expectedScroll.top) <= 1) {
      expectedScroll = null; return;
    }
    const fromRange = from.scrollHeight - from.clientHeight, toRange = to.scrollHeight - to.clientHeight;
    // 短侧没有滚动空间时不驱动长侧；也不会通过它的 0 位置反向回拉。
    if (fromRange <= 0 || toRange <= 0) return;
    const target = Math.max(0, Math.min(1, from.scrollTop / fromRange)) * toRange;
    if (Math.abs(to.scrollTop - target) <= 1) return;
    to.scrollTop = target;
    expectedScroll = { target: to, top: to.scrollTop };
  }
  function scheduleRender() {
    clearTimeout(renderTimer);
    renderTimer = setTimeout(() => { syncScope(); if (active && result) renderResult(); }, 40);
  }
  function environmentChanged() {
    clearTimeout(environmentTimer);
    environmentTimer = setTimeout(() => {
      syncScope();
      if (running && runningKey !== contextKey()) { invalidateRun(); status.textContent = '配置已变化，请重新运行对照。'; }
      if (!sameEnvironment()) { clearFocus(); clearReferenceMarks(); }
      updateDraftStatus();
    }, 80);
  }
  draft.addEventListener('input', changed);
  for (const input of [draft, ...(draftPanes?.inputs || [])]) input.addEventListener('keydown', event => {
    if ((event.ctrlKey || event.metaKey) && event.key === 'Enter') { event.preventDefault(); run(); }
    else if ((event.ctrlKey || event.metaKey) && !event.shiftKey && event.key.toLowerCase() === 'z') { event.preventDefault(); undo(); }
  });
  byId('macro_draft_import').addEventListener('click', () => replaceDraft(buildMacroText()));
  undoButton.addEventListener('click', undo); byId('macro_draft_copy').addEventListener('click', copy);
  runButton.addEventListener('click', run);
  firstButton.addEventListener('click', () => { const index = firstVisibleDifference(); if (index >= 0) focusRow(index); });
  layerSelect?.addEventListener('change', () => { selectedRow = null; detail.hidden = true; renderResult(); });
  syncToggle?.addEventListener('change', () => { expectedScroll = null; if (syncToggle.checked) syncScroll(referenceSequence, sequence); });
  referenceSequence.addEventListener('scroll', () => syncScroll(referenceSequence, sequence), { passive: true });
  sequence.addEventListener('scroll', () => syncScroll(sequence, referenceSequence), { passive: true });
  sequence.addEventListener('click', event => {
    const item = event.target.closest('[data-compare-row]'); if (item) focusRow(Number(item.dataset.compareRow));
  });
  sequence.addEventListener('keydown', event => {
    if (!['Enter', ' '].includes(event.key)) return;
    const item = event.target.closest('[data-compare-row]');
    if (item) { event.preventDefault(); focusRow(Number(item.dataset.compareRow)); }
  });
  referenceSequence.addEventListener('click', event => {
    if (!active || !sameEnvironment() || document.getElementById('macro_editor_panel')?.dataset.tab === 'conditions') return;
    const item = event.target.closest('[data-macro-assist-index]'); if (!item) return;
    const map = referenceMap(result.template);
    const row = result.alignment.rows.findIndex(row => row.referenceIndex != null && map.get(row.referenceIndex) === item);
    if (row >= 0 && visibleDifference(result.alignment.rows[row])) focusRow(row);
  }, true);
  window.addEventListener('jx3-macro-assist-mode', event => {
    active = !!event.detail?.active; editor.hidden = !active; compare.hidden = !active;
    if (active) { syncScope(); refreshIdentity(); scheduleRender(); }
    else { clearFocus(); clearReferenceMarks(); clearBands(sequence); }
  });
  for (const type of ['input', 'change']) document.addEventListener(type, event => {
    if (!event.target.closest('[data-macro-draft], #macro_review, #macro_assist_panel, #macro_compare_layer, #macro_compare_sync')) environmentChanged();
  });
  window.addEventListener('jx3-sim-complete', () => { environmentChanged(); scheduleRender(); });
  window.addEventListener('jx3-macro-layout-change', scheduleRender);
  window.addEventListener('focus', refreshIdentity);
  window.addEventListener('blur', () => { expectedScroll = null; });
  new MutationObserver(scheduleRender).observe(document.body, { attributes: true, attributeFilter: ['class', 'data-theme'] });
  new MutationObserver(scheduleRender).observe(referenceSequence, { attributes: true, attributeFilter: ['style'] });
  window.Jx3MacroEditor = { insert, run, getResult: () => result };
  syncScope(); refreshIdentity();
})();
