/* 武学助手：任务编排 UI。场景取自本次模拟返回的请求快照，任务只保留在 worker 内存。 */
(function (root, factory) {
  'use strict';
  const api = factory();
  if (typeof module === 'object' && module.exports) module.exports = api;
  if (root) { root.Jx3Harness = api; api.mount(root); }
})(typeof window !== 'undefined' ? window : null, function () {
  'use strict';

  const budgets = {
    quick: { max_simulations: 32, wall_time_ms: 20000, max_rounds: 3 },
    standard: { max_simulations: 96, wall_time_ms: 60000, max_rounds: 6 },
    thorough: { max_simulations: 192, wall_time_ms: 120000, max_rounds: 10 },
  };
  const terminal = new Set(['completed', 'cancelled', 'failed', 'error', 'timed_out', 'budget_exhausted']);
  const statuses = { queued: '等待运行', created: '已创建', running: '正在写宏', completed: '验证结束', cancelled: '已停止', failed: '运行失败', error: '运行失败', timed_out: '时间预算用尽', budget_exhausted: '预算已用尽', cancelling: '正在停止' };
  const stops = { target_reproduced: '窗口内动作、顺序、时机与已采集资源已还原。', cancelled: '已按要求停止，保留已验证候选。', budget_exhausted: '已用完本次预算，保留已验证候选。', time_budget: '已用完时间预算，保留已验证候选。', no_improvement: '暂未找到更好的候选。', max_rounds: '已完成本次迭代轮数。' };
  const clone = value => JSON.parse(JSON.stringify(value));

  function shouldAcceptStatus(current, incoming, jobId) {
    return !!incoming && (!incoming.job_id || incoming.job_id === jobId)
      && !(Number.isFinite(incoming.sequence) && Number.isFinite(current?.sequence) && incoming.sequence < current.sequence);
  }

  async function captureScenario(deps) {
    await deps.ready();
    const identity = clone(deps.identity());
    const before = deps.contextKey();
    const sequence = deps.sequence();
    if (!Array.isArray(sequence) || !sequence.length) throw new Error('当前技能轴为空。请先到循环编辑器添加手动技能。');
    if (sequence.some(skill => String(skill).includes('__macro__'))) throw new Error('当前技能轴包含宏块。请回循环编辑器固化宏块为手动技能轴，再开始写宏。');
    const result = await deps.simulate();
    if (!result || !Array.isArray(result.timeline)) throw new Error('未能完成技能轴模拟。请检查后端连接，并在循环编辑器重算。');
    if (deps.contextKey() !== before || JSON.stringify(deps.identity()) !== JSON.stringify(identity)) throw new Error('读取期间技能轴或战斗配置发生变化，请重新开始。');
    // _macroAssistBody 是 runSimulate 在发起本次 POST 前保存的深拷贝，属于这个 result。
    // 不能退回 _lastSimBody：并发模拟、空轴或失败都可能留下上一场景的全局缓存。
    const body = result._macroAssistBody;
    if (!body || !Array.isArray(body.sequence) || !body.sequence.length) throw new Error('本次模拟缺少完整场景快照，请刷新页面后重试。');
    if (body.macro_text || body.sequence.some(skill => String(skill).includes('__macro__'))) throw new Error('请使用手动技能轴作为目标，宏可填入“已有宏作为起点”。');
    if (!body.attributes || !body.target) throw new Error('请先在循环编辑器设置角色属性和目标，再开始写宏。');
    const simulation = clone(body);
    simulation.lite = false;
    simulation.lite_keep_timeline = false;
    return { simulation, version: identity.version, mount: identity.mount, sourceKey: before, result };
  }

  function macroPages(text, metadata = []) {
    const pages = [];
    let current = { stance: 'general', lines: [] };
    for (const line of String(text || '').replace(/\r\n?/g, '\n').split('\n')) {
      if (line.trim().startsWith('#page')) {
        if (current.lines.some(value => value.trim())) pages.push(current);
        current = { stance: line.trim().slice(5).trim() || 'general', lines: [] };
      } else current.lines.push(line);
    }
    if (current.lines.some(value => value.trim())) pages.push(current);
    return pages.map((page, index) => {
      const value = page.lines.join('\n').trimEnd();
      const meta = metadata[index] || {};
      return { stance: meta.stance || page.stance, text: value, chars: meta.chars ?? value.length, limit: meta.limit ?? 128, within_limit: meta.within_limit ?? value.length <= 128 };
    });
  }

  function mount(root) {
    const doc = root.document;
    const page = doc.getElementById('page-harness');
    if (!page) return;
    const $ = name => doc.getElementById(`harness_${name}`);
    const state = { job: null, source: null, timer: null, preparing: false, status: null, result: null, initial: '', artifact: null, renderKey: '', serial: 0, refreshing: false, seen: false, jobs: [] };
    const node = (tag, text, className) => { const el = doc.createElement(tag); if (text != null) el.textContent = text; if (className) el.className = className; return el; };
    const number = value => Number.isFinite(value) ? Math.round(value).toLocaleString('zh-CN') : '—';
    const seconds = value => Number.isFinite(value) ? `${value.toFixed(2)} 秒` : '—';
    const feedback = (text, error = false) => { $('feedback').textContent = text; $('feedback').dataset.error = String(error); };
    const path = (id, suffix = '') => `/api/harness/jobs/${encodeURIComponent(id)}${suffix}`;
    const running = status => !!status && (typeof status.running === 'boolean' ? status.running : !terminal.has(status.status));
    const active = () => state.preparing || running(state.status);
    const goSim = () => root.Jx3Nav?.switchPage('page-sim');

    async function request(url, options = {}) {
      const response = await root.fetch(url, { cache: 'no-store', ...options, signal: AbortSignal.timeout(12000) });
      let data;
      try { data = await response.json(); } catch { data = null; }
      if (!response.ok) {
        const error = new Error(data?.error?.message || data?.message || (typeof data?.error === 'string' ? data.error : '') || (response.status === 404 ? '武学助手服务暂不可用，请使用支持此功能的后端。' : `请求失败（${response.status}），请稍后重试。`));
        error.status = response.status; error.data = data; throw error;
      }
      return data;
    }

    function controls() {
      $('start').disabled = active();
      $('start').textContent = state.preparing ? '正在读取最新技能轴…' : running(state.status) ? '写宏任务运行中…' : '读取技能轴并开始写宏';
      $('cancel').disabled = !running(state.status) || state.status?.status === 'cancelling' || state.status?.cancellation_requested === true;
      $('download').disabled = !state.job;
      $('budget').disabled = state.preparing;
      $('max_pages').disabled = state.preparing;
      $('initial_macro').disabled = state.preparing;
      $('recent').disabled = state.preparing;
    }

    function sourceSummary(capture) {
      const identity = typeof currentMount !== 'undefined' ? currentMount : {};
      const body = capture.simulation;
      $('source_title').textContent = `${identity.mount_label || capture.mount} · ${identity.version_label || capture.version}`;
      $('source_detail').textContent = `${body.sequence.length} 个轴内动作 · ${seconds(capture.result.fight_time)} · 延迟 ${body.network_delay || 0} ms。已读取当前装备、属性、奇穴、秘籍、目标、团辅、阵法和预释放。`;
    }

    function artifactSource(artifact) {
      const request = artifact?.request, body = request?.simulation;
      if (!body) return;
      const versions = { ShanHaiYuanLiu: '山海源流（2025.10）', AnYingQianJi: '暗影千机（2026.04）', AnYingQianJiTest: '暗影千机·测试服（归档）', CangShengZhuShiTest: '苍生铸世测试服（2026.10）' };
      const mounts = { FenShanJin: '分山劲', TieGuYi: '铁骨衣' };
      const sameIdentity = typeof currentMount !== 'undefined' && request.version === currentMount.version && request.mount === currentMount.mount;
      $('source_title').textContent = `${mounts[request.mount] || request.mount} · ${versions[request.version] || request.version}`;
      $('source_detail').textContent = `本次任务冻结的技能轴：${body.sequence?.length || 0} 个动作 · 延迟 ${body.network_delay || 0} ms。${sameIdentity ? '再次开始将重新读取编辑器中的最新配置。' : '此结果的版本或心法与当前顶栏不同；再次开始将使用当前编辑器配置。'}`;
    }

    async function start() {
      if (active()) return;
      state.preparing = true; controls(); feedback('正在重新模拟当前技能轴，读取完整战斗配置…');
      try {
        const budget = budgets[$('budget').value] || budgets.standard;
        const initial = $('initial_macro').value.trim();
        const maxPages = Number($('max_pages').value);
        const capture = await captureScenario({
          ready: () => Promise.all([typeof currentMountReady !== 'undefined' ? currentMountReady : Promise.resolve(), typeof attributesReady !== 'undefined' ? attributesReady : Promise.resolve()]),
          identity: () => ({ version: currentMount.version, mount: currentMount.mount }),
          contextKey: () => {
            if (typeof root.Jx3MacroAssist?.contextKey !== 'function') throw new Error('循环编辑器尚未完整加载，请刷新页面后重试。');
            return root.Jx3MacroAssist.contextKey();
          },
          sequence: () => readSequence(), simulate: () => runSimulate(),
        });
        sourceSummary(capture);
        const body = { simulation: capture.simulation, version: capture.version, mount: capture.mount, ...budget, max_pages: maxPages, time_tolerance: 0.0625, ...(initial ? { initial_macro: initial } : {}) };
        const job = await request('/api/harness/jobs', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) });
        if (!job?.job_id) throw new Error('服务未返回任务标识，请刷新最近任务查看。');
        state.initial = initial; state.artifact = null; state.result = null; state.renderKey = ''; $('result').hidden = true;
        state.status = { ...job, running: true, max_simulations: budget.max_simulations, simulations: 0, elapsed_ms: 0 };
        feedback('已固定本次技能轴与战斗配置，开始生成和验证。');
        await follow(job.job_id, state.status, true);
        void refreshRecent(false);
      } catch (error) { feedback(error.message || '启动失败，请重试。', true); }
      finally { state.preparing = false; controls(); }
    }

    function stopWatching() {
      state.source?.close(); state.source = null;
      clearTimeout(state.timer); state.timer = null;
    }

    function expiredJob(error) {
      if (error.status !== 404) return false;
      stopWatching(); state.serial += 1; state.job = null;
      state.status = { status: 'failed', running: false };
      $('status').textContent = '任务已过期'; $('status').dataset.tone = 'warn';
      $('message').textContent = '任务已不在当前服务中，可重新读取技能轴开始。';
      feedback(error.message, true); controls(); return true;
    }

    function schedulePoll(delay = 1800) {
      clearTimeout(state.timer);
      if (!state.job || !running(state.status)) return;
      const id = state.job, serial = state.serial;
      state.timer = setTimeout(async () => {
        try { const status = await request(path(id)); if (serial === state.serial) update(status); }
        catch (error) { if (serial === state.serial && !expiredJob(error)) feedback('进度连接暂时中断，正在重连；任务仍可能继续运行。', true); }
        if (serial === state.serial && running(state.status)) schedulePoll();
      }, delay);
    }

    function events(id, serial) {
      if (typeof root.EventSource !== 'function') { schedulePoll(); return; }
      const source = new root.EventSource(path(id, '/events'));
      state.source = source;
      const receive = event => {
        if (serial !== state.serial) return;
        try {
          const data = JSON.parse(event.data);
          update(data.job_id ? data : data.status && typeof data.status === 'object' ? data.status : data);
          if (running(state.status)) schedulePoll(6000);
        } catch { schedulePoll(); }
      };
      source.addEventListener('progress', receive);
      source.addEventListener('completed', receive);
      source.onerror = () => { if (serial === state.serial) { source.close(); if (state.source === source) state.source = null; schedulePoll(); } };
      schedulePoll(4000);
    }

    async function follow(id, known, keepInitial = false) {
      stopWatching(); state.serial += 1;
      const serial = state.serial;
      state.job = id; state.result = null; state.artifact = null; state.renderKey = ''; $('result').hidden = true;
      state.status = { job_id: id, status: 'running', running: true };
      if (!keepInitial) { state.initial = null; $('before_macro').textContent = '正在读取起始宏…'; }
      if (known) update(known);
      try {
        const status = await request(path(id));
        if (serial !== state.serial) return;
        update(status);
        if (running(status)) events(id, serial);
        if (!keepInitial) void loadArtifact(false);
        return true;
      } catch (error) { if (serial === state.serial && !expiredJob(error)) { feedback(error.message, true); if (running(state.status)) schedulePoll(); } }
      return false;
    }

    function update(status) {
      if (!shouldAcceptStatus(state.status, status, state.job)) return;
      state.status = { ...state.status, ...status };
      const value = state.status, live = running(value);
      $('status').textContent = statuses[value.status] || (live ? '正在运行' : '验证结束');
      $('status').dataset.tone = live ? 'running' : value.status === 'completed' ? 'good' : 'warn';
      $('message').textContent = value.error?.message || (typeof value.error === 'string' ? value.error : '') || value.message || (live ? '正在生成候选并运行对照…' : '本次任务已结束。');
      const simulations = value.simulations ?? value.progress?.simulations ?? 0;
      const max = value.max_simulations || 96;
      $('simulations').textContent = `${simulations} / ${max}`;
      $('progress').max = max; $('progress').value = Math.min(simulations, max);
      $('elapsed').textContent = `${((value.elapsed_ms || 0) / 1000).toFixed(1)} 秒`;
      $('scenario_hash').textContent = value.scenario_hash || '—'; $('experiment_hash').textContent = value.experiment_hash || '—';
      const result = value.result || (value.progress?.best ? { best: value.progress.best } : null);
      if (result) { state.result = result; renderResult(result, live); }
      if (!live) { stopWatching(); if (!value.result && !state.result) feedback('任务结束，尚未生成可用的已验证宏。可查看运行消息或下载结果包。'); }
      controls();
    }

    function metrics(result) {
      const base = result.baseline, candidate = result.best?.metrics;
      const values = [
        ['原技能轴 DPS', number(base?.dps), base ? `模拟时长 ${seconds(base.fight_time)}` : '最终结果返回后显示'],
        ['候选宏 DPS', number(candidate?.dps), candidate ? `模拟时长 ${seconds(candidate.fight_time)}` : '—'],
        ['主动动作', `${number(base?.active_casts)} → ${number(candidate?.active_casts)}`, '原技能轴 → 候选宏'],
        ['宏页约束', result.best?.page_constraints_passed ? '通过' : '未通过', '每页 ≤ 128 字符'],
      ];
      $('metrics').replaceChildren(...values.map(([label, value, note]) => { const box = node('div', null, 'harness-metric'); box.append(node('span', label), node('strong', value), node('small', note)); return box; }));
    }

    async function copy(text, button) {
      try {
        if (root.navigator.clipboard?.writeText && root.isSecureContext) await root.navigator.clipboard.writeText(text);
        else {
          const textarea = node('textarea'); textarea.value = text; textarea.style.cssText = 'position:fixed;left:-9999px;top:0'; doc.body.append(textarea); textarea.select();
          const ok = doc.execCommand('copy'); textarea.remove(); if (!ok) throw new Error('copy');
        }
        if (button) { const previous = button.textContent; button.textContent = '已复制'; setTimeout(() => { button.textContent = previous; }, 1600); }
        feedback('已复制宏文本。');
      } catch { feedback('自动复制不可用，请选中宏文本后按 Ctrl+C。', true); }
    }

    function renderPages(candidate) {
      const pages = macroPages(candidate.macro_text, candidate.pages);
      $('macro_pages').replaceChildren(...pages.map((value, index) => {
        const wrapper = node('div', null, 'harness-macro-page'), head = node('div', null, 'harness-macro-page-head');
        const stance = { shield: '擎盾', blade: '擎刀', general: '通用', any: '通用', '擎盾': '擎盾', '擎刀': '擎刀' }[value.stance] || value.stance;
        const size = node('span', `${value.chars} / ${value.limit} 字符`); size.dataset.overLimit = String(!value.within_limit);
        const button = node('button', '复制本页', 'sim-btn'); button.type = 'button'; button.addEventListener('click', () => copy(value.text, button));
        button.disabled = !value.within_limit; button.title = value.within_limit ? '复制本页文本到游戏宏' : '本页超过 128 字符，请先缩短后再用于游戏宏';
        head.append(node('strong', `第 ${index + 1} 页 · ${stance}`), size, button); wrapper.append(head, node('pre', value.text)); return wrapper;
      }));
    }

    const eventText = event => {
      if (!event) return '—';
      if (typeof event === 'string') return event;
      const name = event.name || event.skill_name || event.skill || (event.skill_id != null ? `技能 ${event.skill_id}` : '—');
      const time = event.cast_time ?? event.time;
      return `${name}${Number.isFinite(time) ? ` · ${time.toFixed(3)}s` : ''}`;
    };
    const rowReference = row => row.reference || row.expected || row.reference_event || (Number.isInteger(row.reference_index) ? { name: `事件 ${row.reference_index + 1}` } : null);
    const rowActual = row => row.actual || row.candidate || row.actual_event || (Number.isInteger(row.actual_index) ? { name: `事件 ${row.actual_index + 1}` } : null);
    const differenceLabel = kind => ({ missing: '漏放', extra: '多放', changed: '时间 / 状态', same: '一致', reordered: '顺序变化' }[kind] || kind || '差异');

    function renderAlignment(candidate) {
      const alignment = candidate.alignment || {}, summary = alignment.summary || alignment;
      const rows = Array.isArray(alignment.rows) ? alignment.rows : [];
      const referenceActions = new Map((candidate.reference_actions || []).map(action => [action.index, action]));
      const actualActions = new Map((candidate.actual_actions || []).map(action => [action.index, action]));
      $('diff_summary').replaceChildren(...[['漏放', summary.missing], ['多放', summary.extra], ['时间 / 状态', summary.changed]].map(([label, count]) => node('span', `${label} ${Number.isFinite(count) ? count : '—'}`)));
      const firstIndex = summary.first_difference ?? summary.firstDifference;
      const first = candidate.first_difference || (alignment.first_difference && typeof alignment.first_difference === 'object' ? alignment.first_difference : Number.isInteger(firstIndex) ? rows[firstIndex] : rows.find(row => row.kind && row.kind !== 'same'));
      const firstEl = $('first_difference');
      firstEl.replaceChildren();
      if (first) {
        firstEl.append(node('strong', '首处分歧'), node('div', `原技能轴：${eventText(rowReference(first))}`), node('div', `候选宏：${eventText(rowActual(first))}`), node('div', `差异：${differenceLabel(first.kind)}`));
        const delta = first.time_delta ?? first.timeDelta ?? (Number.isFinite(first.reference?.cast_time) && Number.isFinite(first.actual?.cast_time) ? first.actual.cast_time - first.reference.cast_time : null);
        if (Number.isFinite(delta)) firstEl.append(node('div', `释放偏移 ${delta >= 0 ? '+' : ''}${delta.toFixed(3)} 秒`));
        const resourceNames = { rage: '怒气', berserk_value: '暴怒', max_berserk_value: '暴怒上限', block_value: '格挡值' };
        const resourceDiffs = first.resource_diffs || (Number.isInteger(firstIndex) ? rows[firstIndex]?.resource_diffs : []) || [];
        resourceDiffs.forEach(diff => firstEl.append(node('div', `${resourceNames[diff.field] || diff.field}：原轴 ${diff.reference} → 宏 ${diff.actual}`)));
        if (candidate.diagnosis?.message) firstEl.append(node('div', candidate.diagnosis.message));
      } else firstEl.append(node('strong', candidate.reproduced ? '观测窗口内未发现动作分歧' : '等待完整动作对照'), node('div', candidate.reproduced ? '动作、顺序、时机与已采集资源通过验证；不代表全部增益与冷却状态等价。' : '仅以返回的动作对照和宏页校验判定复现。'));
      const changed = rows.filter(row => row.kind !== 'same').slice(0, 50);
      const table = node('table'), head = node('thead'), body = node('tbody'), header = node('tr');
      ['差异', '原技能轴', '候选宏', '时间偏移'].forEach(text => header.append(node('th', text))); head.append(header);
      changed.forEach(row => { const tr = node('tr'), delta = row.time_delta ?? row.timeDelta; tr.append(node('td', differenceLabel(row.kind)), node('td', eventText(referenceActions.get(row.reference_index) || rowReference(row))), node('td', eventText(actualActions.get(row.actual_index) || rowActual(row))), node('td', Number.isFinite(delta) ? `${delta >= 0 ? '+' : ''}${delta.toFixed(3)}s` : '—')); body.append(tr); });
      table.append(head, body); $('differences').replaceChildren(...(changed.length ? [table] : []));
      if (rows.filter(row => row.kind !== 'same').length > 50) $('differences').append(node('p', '此处显示前 50 条，完整差异见下载结果包。', 'harness-note'));
    }

    function renderResult(result, live) {
      const candidate = result.best;
      if (!candidate?.macro_text || candidate.verified !== true) return;
      const key = JSON.stringify([candidate.fingerprint, candidate.macro_text, candidate.reproduced, candidate.page_constraints_passed, candidate.alignment, result.baseline, result.stop_reason, live]);
      if (key === state.renderKey) return;
      state.renderKey = key; $('result').hidden = false;
      $('verdict').textContent = candidate.reproduced ? '窗口内动作与资源已还原' : '已验证 · 仍有差异';
      $('verdict').dataset.tone = candidate.reproduced ? 'good' : 'warn';
      $('result_note').textContent = `${live ? '搜索仍在继续，下方是已经实际运行过的最佳候选。' : (stops[result.stop_reason] || '本次验证已结束。请检查动作差异后再使用。')} ${Number.isFinite(result.window_seconds) ? `对照窗口 ${seconds(result.window_seconds)}。` : ''}DPS 为各自完整模拟的观测值，尾段时长可能不同，不作为提升结论。`;
      metrics(result); renderPages(candidate); renderAlignment(candidate);
      $('before_macro').textContent = state.initial == null ? '正在读取起始宏…' : state.initial || '未提供起始宏，由技能轴自动生成。';
      $('after_macro').textContent = candidate.macro_text;
      $('history').replaceChildren(...(result.history || []).map(entry => node('div', `第 ${entry.round} 轮 · ${entry.accepted ? '保留候选' : '未采纳'} · ${typeof entry.summary === 'string' ? entry.summary : entry.summary ? `漏放 ${entry.summary.missing} / 多放 ${entry.summary.extra} / 变化 ${entry.summary.changed}` : '已运行验证'}`)));
      $('limitations').replaceChildren(...(result.limitations || ['仅验证本次技能轴与冻结场景，尚未验证其他配装、时长、延迟和随机种子。']).map(text => node('li', text)));
    }

    async function loadArtifact(download) {
      const id = state.job, serial = state.serial;
      if (!id) return;
      try {
        const artifact = state.artifact || await request(path(id, '/artifacts'));
        if (serial !== state.serial) return;
        // A running result package changes as candidates finish; cache only terminal snapshots.
        if (terminal.has(artifact?.status)) state.artifact = artifact;
        artifactSource(artifact);
        state.initial = artifact?.request?.initial_macro || '';
        $('before_macro').textContent = state.initial || '未提供起始宏，由技能轴自动生成。';
        if (download) {
          const url = URL.createObjectURL(new Blob([JSON.stringify(artifact, null, 2)], { type: 'application/json' }));
          const link = node('a'); link.href = url; link.download = `苍云武学助手-${id}.json`; doc.body.append(link); link.click(); link.remove(); setTimeout(() => URL.revokeObjectURL(url), 1000);
          feedback('结果包已下载，包含本次场景、任务参数和验证结果。');
        }
      } catch (error) { if (serial === state.serial) { if (download) feedback(error.message, true); else $('before_macro').textContent = '暂未读取到起始宏，可重试下载结果包。'; } }
    }

    async function refreshRecent(restore = false) {
      if (state.refreshing) return;
      state.refreshing = true; $('refresh').disabled = true;
      try {
        const value = await request('/api/harness/jobs');
        state.jobs = Array.isArray(value) ? value : value?.jobs || [];
        const select = $('recent');
        select.replaceChildren(node('option', state.jobs.length ? '选择最近任务' : '暂无任务'));
        select.firstElementChild.value = '';
        state.jobs.forEach(job => { const option = node('option', `${statuses[job.status] || job.status} · ${String(job.job_id).slice(-10)} · ${job.simulations || 0} 次模拟`); option.value = job.job_id; select.append(option); });
        select.value = state.job || '';
        if (restore && !state.job && !state.preparing && state.jobs.length) {
          const job = state.jobs.find(running) || state.jobs[0];
          select.value = job.job_id;
          const restored = await follow(job.job_id, job);
          if (restored && state.job === job.job_id) feedback('已恢复当前服务中的最近任务。');
        }
      } catch (error) { if (restore || page.classList.contains('active')) feedback(error.message, true); }
      finally { state.refreshing = false; $('refresh').disabled = false; }
    }

    $('start').addEventListener('click', start);
    $('go_sim').addEventListener('click', goSim);
    $('open_editor').addEventListener('click', () => { goSim(); if (!root.Jx3MacroAssist?.isActive()) doc.getElementById('macro_assist_toggle')?.click(); root.Jx3MacroLayout?.tab('draft'); });
    $('use_draft').addEventListener('click', () => { const value = doc.getElementById('macro_draft_text')?.value || ''; $('initial_macro').value = value; feedback(value ? '已读取当前写宏草稿。' : '当前写宏草稿为空，可留空自动生成。'); });
    $('copy_all').addEventListener('click', () => { if (state.result?.best?.macro_text) void copy(state.result.best.macro_text, $('copy_all')); });
    $('download').addEventListener('click', () => { void loadArtifact(true); });
    $('refresh').addEventListener('click', () => { void refreshRecent(!state.job); });
    $('recent').addEventListener('change', () => { const id = $('recent').value; if (id) void follow(id, state.jobs.find(job => job.job_id === id)); });
    $('cancel').addEventListener('click', async () => {
      if (!state.job || !running(state.status)) return;
      const id = state.job;
      $('cancel').disabled = true;
      try { await request(path(id, '/cancel'), { method: 'POST' }); if (state.job === id) { feedback('已请求停止，当前模拟结束后保留已验证候选。'); schedulePoll(100); } }
      catch (error) { feedback(error.message, true); controls(); }
    });
    doc.querySelectorAll('[data-harness-open]').forEach(button => button.addEventListener('click', () => root.Jx3Nav?.switchPage('page-harness')));
    new MutationObserver(() => { if (page.classList.contains('active')) { if (!state.seen) { state.seen = true; void refreshRecent(true); } else if (!active()) void refreshRecent(false); } }).observe(page, { attributes: true, attributeFilter: ['class'] });
    root.addEventListener('beforeunload', stopWatching);
    if (page.classList.contains('active')) { state.seen = true; void refreshRecent(true); }
  }

  return { captureScenario, macroPages, shouldAcceptStatus, mount };
});
