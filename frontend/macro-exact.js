/* Deterministic exact synthesis. This panel does not use an LLM or API key. */
(function (root, factory) {
  'use strict';
  const api = factory();
  if (typeof module === 'object' && module.exports) module.exports = api;
  if (root) api.mount(root);
})(typeof window !== 'undefined' ? window : null, function () {
  'use strict';
  const labels = { running: '计算中', exact: '已匹配', partial: '已保留最佳宏', cancelled: '已停止', unknown: '求解结果未知', bounded_unsat: '当前规则范围内无解', finite_and_conflict: '当前 AND 条件语言无法表达', finite_language_conflict: '当前条件无法区分目标状态', runtime_search_exhausted: '当前候选搜索已耗尽', invalid_target: '目标循环存在未成功施放的技能', semantic_mismatch: '手动轴与宏执行语义不一致', probe_budget: '轨迹观测达到上限', budget_exhausted: '计算未完成', error: '执行失败', dependency_missing: '缺少求解依赖' };
  const phases = { expanding: '补充首次分歧附近的合法条件', branching: '更换到达路径，重新组合整份宏', relaxing: '时间容差调整至 125.0ms', preparing: '回放目标循环', prepared: '目标已建立', target: '目标已建立', solving: '自动组合条件与规则顺序', constructing: '构造规则，不限制行数或每行条件数', candidate: '新候选已生成，等待真实回放', replaying: '正在真实回放当前候选', replayed: '当前候选已完成回放', best: '最佳候选已更新', compressing: '已完整复现，正在精简', pausing: '正在暂停并保留求解进度', paused: '已暂停，进度已保留', resuming: '正在继续', resumed: '继续求解', cancelling: '正在停止', finished: '计算结束' };
  const number = value => Number.isFinite(Number(value)) ? Number(value).toFixed(1) : '—';
  function phaseText(job) {
    const report = job.result?.report;
    const summary = job.compression || report?.compression;
    if (!job.done) {
      if (['paused', 'pausing', 'cancelling', 'resuming', 'resumed'].includes(job.phase)) return phases[job.phase];
      const compressing = job.stage === 'compression' || !!summary || job.best?.comparison?.reproduced;
      return `${compressing ? '第二阶段 · 压缩' : '第一阶段 · 提取宏'} · ${phases[job.phase] || '计算中'}`;
    }
    if (summary) {
      const reason = report?.compression_stop || summary.status;
      if (reason === 'cancelled') return '压缩已停止，保留已验证宏。';
      if (reason === 'error' || reason === 'budget_exhausted') return '压缩未完成，保留已验证宏。';
      return '本轮压缩完成，保留最短已验证宏。';
    }
    return job.reason || report?.reason || labels[job.status] || '计算结束';
  }
  function compressionText(job) {
    const c = job.compression || job.result?.report?.compression;
    return c ? `字符 ${c.initial_chars} → ${c.best_chars} · 减少 ${Math.max(0, c.initial_chars - c.best_chars)} · 试算 ${c.trial_count || 0} 次` : '';
  }
  function comparisonText(c) {
    if (!c) return '等待第一个经过真实回放的候选。';
    const timed = c.acceptance === 'skills_and_time';
    return `技能 ${c.order_prefix}/${c.target_count} · ${timed ? '时序' : '时序与状态'} ${timed ? c.exact_prefix : Math.min(c.exact_prefix, c.state_prefix)}/${c.target_count} · ${c.completed_full_replay === false ? '已回放' : '实际'} ${c.actual_count} 次\n最大偏差 ${number((c.max_time_error_on_order_prefix || 0) * 1000)}ms · 容差 ${number((c.time_tolerance_seconds ?? 1e-7) * 1000)}ms${timed && c.state_reproduced === false ? `\n状态快照诊断 ${c.state_prefix}/${c.target_count}（不作为施放验收）` : ''}`;
  }
  function differenceText(c) {
    if (!c) return '';
    if (c.reproduced) return '';
    const d = c.first_difference;
    if (!d) return '尚未完整复现。';
    const cast = e => e ? `${e.name} @ ${number(e.time)} 秒` : '无施放';
    return `首次分歧：第 ${d.index + 1} 次\n目标：${cast(d.expected)}\n候选：${cast(d.actual)}`;
  }
  function preparationText(report) {
    if (!report) return '';
    const failure = report.preparation_details?.probe_failure;
    if (report.status === 'semantic_mismatch') {
      const index = failure?.index ?? report.comparison?.first_difference?.index;
      const action = failure?.expected || report.comparison?.first_difference?.expected;
      const where = index != null ? `第 ${index + 1} 次${action ? `「${action.name}」@ ${number(action.time)} 秒` : '施放'}` : '目标回放';
      const reason = failure?.kind === 'state_mismatch' ? '技能和施放时间一致，但 Buff、资源或冷却状态不同。' : '手动路径与宏路径的动作或时机未对齐。';
      return `${where}：${reason}\n任务已结束，尚未进入求解。需要先解决执行语义差异，增加等待时间不会继续计算。`;
    }
    if (report.status === 'invalid_target') return '手动循环中有技能未成功施放，请修正循环后重新开始。';
    return '';
  }
  function currentCandidate(job) {
    if (job?.candidate?.macro) return job.candidate;
    if (job?.best?.macro) return job.best;
    return { macro: job?.result?.macro || '', comparison: job?.result?.report?.comparison };
  }
  function displayCandidate(job, mode = 'best') {
    return mode === 'best' && job?.best?.macro ? job.best : currentCandidate(job);
  }
  function mergeSnapshot(previous, next) {
    if (!previous || previous.id !== next.id) return next;
    const merged = { ...previous, ...next };
    for (const key of ['best', 'candidate', 'result']) {
      if (next[key] && typeof next[key] === 'object') merged[key] = { ...previous[key], ...next[key] };
    }
    return merged;
  }
  function snapshotQuery(job, includeRevision = true) {
    const params = new URLSearchParams({ compact: 'true' });
    if (job) {
      params.set('job_id', job.id);
      if (includeRevision) params.set('revision', job.revision ?? 0);
      for (const key of ['best', 'candidate', 'result']) {
        if (typeof job[key]?.macro === 'string' && job[key].macro_revision) params.set(key + '_macro', job[key].macro_revision);
      }
    }
    return params.toString();
  }
  function sceneParameters(simulation) {
    const scene = { ...simulation };
    for (const key of ['sequence', 'macro_text', 'macro_duration', 'channel_ticks', 'timing_offsets', 'solidified_casts', 'qijin_buffs', 'pre_releases', 'lite', 'lite_keep_timeline']) delete scene[key];
    return scene;
  }
  function previewRequest(source, macro) {
    return { ...source.simulation, sequence: Array(6000).fill('__macro__'), macro_text: macro,
      macro_duration: source.horizon, channel_ticks: {}, timing_offsets: {}, solidified_casts: {}, qijin_buffs: {}, lite: false, lite_keep_timeline: false };
  }
  function macroDiff(before, after) {
    const old = before.split('\n'), next = after.split('\n'), pairs = [];
    // LCS for ordinary macros; very large drafts use ordered equal-line anchors.
    // Both paths retain all text and avoid a quadratic allocation for huge drafts.
    if (old.length * next.length <= 1000000) {
      const width = next.length + 1, dp = new Uint32Array((old.length + 1) * width);
      for (let i = old.length - 1; i >= 0; i--) for (let j = next.length - 1; j >= 0; j--)
        dp[i * width + j] = old[i] === next[j] ? 1 + dp[(i + 1) * width + j + 1] : Math.max(dp[(i + 1) * width + j], dp[i * width + j + 1]);
      let i = 0, j = 0;
      while (i < old.length && j < next.length) {
        if (old[i] === next[j]) { pairs.push([i++, j++]); }
        else if (dp[(i + 1) * width + j] >= dp[i * width + j + 1]) i++;
        else j++;
      }
    } else {
      const positions = new Map();
      old.forEach((line, i) => { if (!positions.has(line)) positions.set(line, { indices: [], cursor: 0 }); positions.get(line).indices.push(i); });
      let last = -1;
      next.forEach((line, j) => {
        const entry = positions.get(line); if (!entry) return;
        while (entry.cursor < entry.indices.length && entry.indices[entry.cursor] <= last) entry.cursor++;
        if (entry.cursor < entry.indices.length) { last = entry.indices[entry.cursor++]; pairs.push([last, j]); }
      });
    }
    const lines = next.map(text => ({ text, start: 0, end: 0, changed: false, removed: 0 }));
    let oi = 0, ni = 0, added = 0, removed = 0, changed = 0;
    for (const [oe, ne] of [...pairs, [old.length, next.length]]) {
      const paired = Math.min(oe - oi, ne - ni);
      for (let k = 0; k < paired; k++) {
        const a = old[oi + k], b = next[ni + k], line = lines[ni + k];
        let start = 0, end = b.length, tail = a.length;
        while (start < a.length && start < b.length && a[start] === b[start]) start++;
        while (end > start && tail > start && a[tail - 1] === b[end - 1]) { tail--; end--; }
        Object.assign(line, { start, end, changed: true }); changed++;
      }
      for (let k = ni + paired; k < ne; k++) { Object.assign(lines[k], { end: next[k].length, changed: true }); added++; }
      const deleted = oe - oi - paired;
      if (deleted) { removed += deleted; lines[Math.min(ne, lines.length - 1)].removed += deleted; }
      oi = oe + 1; ni = ne + 1;
    }
    return { lines, added, removed, changed };
  }
  function runClock(now) {
    let id, base = 0, at = now(), running = false, lastWall = 0;
    const value = () => base + (running ? Math.max(0, now() - at) : 0);
    return {
      value,
      get running() { return running; },
      freeze() { base = value(); at = now(); running = false; },
      update(job) {
        const fresh = id !== job.id, server = Number(job.elapsed_ms) || 0;
        if (fresh || job.elapsed_excludes_pauses) base = fresh ? server : Math.max(value(), server);
        else if (running) base = Math.max(value(), base + Math.max(0, server - lastWall));
        else base = value();
        lastWall = server; id = job.id; at = now();
        running = !job.done && !job.pause_requested && !job.cancel_requested && !['paused', 'pausing', 'cancelling'].includes(job.phase);
      },
    };
  }
  function mount(root) {
    const doc = root.document, panel = doc.getElementById('assistant_exact_panel');
    if (!panel || panel.dataset.mounted) return;
    panel.dataset.mounted = 'true'; panel.classList.add('macro-exact');
    panel.innerHTML = `<div class="em-controls"><button type="button" id="em_start" class="harness-primary">读取循环并开始</button><button type="button" id="em_pause" class="sim-btn" disabled>暂停</button><button type="button" id="em_stop" class="sim-btn" disabled>停止</button></div>
      <p id="em_feedback" role="status" aria-live="polite"></p><div class="em-heading"><span id="em_status">等待开始</span><span id="em_runtime" class="em-muted"></span></div><p id="em_phase" class="em-muted">先复现，再自动压缩。</p>
      <section class="em-preview"><div class="em-heading"><h3 id="em_title">最佳宏</h3><button type="button" id="em_view" class="sim-btn">查看当前试算</button></div><p id="em_verdict" class="em-muted">等待生成</p><p id="em_metrics"></p><p id="em_difference" class="em-muted"></p><span id="em_delta" class="em-delta" role="status"></span><pre id="em_macro" tabindex="0" role="textbox" aria-readonly="true" aria-multiline="true" aria-label="宏预览" data-empty="生成后在这里显示；暂停或结束后保留已有结果。"></pre><div class="em-controls"><button type="button" id="em_copy" class="sim-btn" disabled>复制宏</button></div></section>`;
    const el = id => doc.getElementById('em_' + id);
    const text = (id, value) => {
      const node = el(id), next = String(value ?? '');
      if (node.textContent === next) return;
      if (node.childNodes.length === 1 && node.firstChild.nodeType === 3) node.firstChild.data = next;
      else node.textContent = next;
    };
    let job = null, timer = null, ticker = null, starting = false, pendingControl = '', preview = '', epoch = 0, mode = 'best';
    let pollFlight = null, restoring = false, renderedPreview = null, renderedMode = '', previewJob = '', disposed = false;
    let revisions = {}, runningPreview = false;
    const output = doc.createElement('select'); output.id = 'em_output'; output.setAttribute('aria-label', '宏回放目标循环页');
    const run = doc.createElement('button'); run.id = 'em_run'; run.type = 'button'; run.className = 'sim-btn'; run.textContent = '运行当前宏'; run.disabled = true;
    el('copy').parentElement.append(output, run);
    function outputs() {
      const value = output.value, items = [{ id: '', name: '新建循环页' }, ...(root.Jx3LoopTabs?.list() || [])];
      const signature = JSON.stringify(items);
      if (output.dataset.signature === signature) return;
      output.dataset.signature = signature;
      output.replaceChildren(...items.map(item => { const option = doc.createElement('option'); option.value = item.id; option.textContent = item.name + (item.hidden ? '（隐藏）' : ''); return option; }));
      output.value = items.some(item => item.id === value) ? value : '';
    }
    outputs(); root.addEventListener('jx3-loop-tabs-change', outputs);
    const clock = runClock(() => root.performance.now());
    const visible = () => !doc.hidden && !panel.hidden && (root.Jx3Assistant?.isOpen() ?? true);
    async function request(url, body, signal, timeoutMs = 10000) {
      const timeout = root.AbortSignal.timeout(timeoutMs);
      const response = await root.fetch(url, { ...(body === undefined ? {} : { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) }),
        cache: 'no-store', signal: signal ? root.AbortSignal.any([signal, timeout]) : timeout });
      const raw = await response.text(); let data;
      try { data = JSON.parse(raw); } catch (_) { throw new Error(response.status === 404 ? '当前后端尚未包含精确合成接口，请切换到新构建并刷新页面。' : '服务未返回有效结果。'); }
      if (!response.ok) throw new Error(typeof data.error === 'string' ? data.error : data.error?.message || '请求失败，请检查当前循环。');
      return data;
    }
    function runtime() {
      if (!job) return;
      text('runtime', `运行 ${number(clock.value() / 1000)} 秒 · 第 ${Number(job.iteration) || (job.progress || []).reduce((n, p) => Math.max(n, Number(p.iteration) || 0), 0)} 轮`);
    }
    function tick() {
      root.clearTimeout(ticker); ticker = null;
      if (visible()) runtime();
      if (clock.running && visible() && !disposed) ticker = root.setTimeout(tick, 100);
    }
    function renderMacro(value) {
      if (previewJob !== job.id) { previewJob = job.id; revisions = {}; renderedPreview = null; }
      if (value === renderedPreview && mode === renderedMode) return;
      const previous = revisions[mode], diff = previous != null && previous !== value ? macroDiff(previous, value) : null;
      const fragment = doc.createDocumentFragment();
      const lines = diff?.lines || value.split('\n').map(line => ({ text: line }));
      if (value) lines.forEach((line, index) => {
        const row = doc.createElement('span'); row.className = 'em-code-line'; row.dataset.line = String(index + 1);
        if (line.changed || line.removed) row.classList.add('em-line-changed');
        if (line.removed) { row.dataset.removed = String(line.removed); row.title = `此处移除了 ${line.removed} 行`; }
        if (line.changed && line.end > line.start) {
          row.append(doc.createTextNode(line.text.slice(0, line.start)));
          const mark = doc.createElement('mark'); mark.className = 'em-change'; mark.textContent = line.text.slice(line.start, line.end); row.append(mark);
          row.append(doc.createTextNode(line.text.slice(line.end)));
        } else row.textContent = line.text;
        if (index < lines.length - 1) row.append(doc.createTextNode('\n'));
        fragment.append(row);
      });
      const top = el('macro').scrollTop, left = el('macro').scrollLeft;
      el('macro').replaceChildren(fragment); el('macro').scrollTop = top; el('macro').scrollLeft = left;
      text('delta', diff ? `本次更新${diff.changed ? ` · 改 ${diff.changed} 行` : ''}${diff.added ? ` · +${diff.added} 行` : ''}${diff.removed ? ` · −${diff.removed} 行` : ''}` : '');
      revisions[mode] = value; renderedPreview = value; renderedMode = mode;
    }
    function render(value) {
      if (!value) return;
      job = mergeSnapshot(job, value);
      clock.update(job);
      const active = !job.done, report = job.result?.report, candidate = displayCandidate(job, mode), c = candidate.comparison;
      preview = candidate.macro || '';
      const certified = !!preview && !!c?.reproduced, stopping = job.cancel_requested || pendingControl === 'cancel';
      el('start').disabled = active || starting;
      el('pause').disabled = !active || !!pendingControl || stopping;
      el('stop').disabled = !active || stopping;
      text('pause', pendingControl === 'pause' || job.phase === 'pausing' ? '暂停中…' : job.pause_requested ? '继续' : '暂停');
      text('stop', active && stopping ? '停止中…' : '停止');
      text('status', active && stopping ? '正在停止' : job.phase === 'paused' && active ? '已暂停' : labels[job.status] || job.status);
      el('status').dataset.exact = String(job.status === 'exact');
      text('phase', phaseText(job));
      runtime(); tick();
      text('verdict', certified ? (c.strict_reproduced ? '完全一致' : '容差内匹配') : preview ? (c ? '部分匹配 · 未通过' : job.phase === 'replaying' ? '回放中 · 未验证' : '待回放 · 未验证') : active ? '等待生成' : '未生成候选');
      el('verdict').dataset.exact = String(certified);
      text('metrics', preview && !c ? `第 ${candidate.iteration || '—'} 轮 · ${candidate.rule_count || preview.split('\n').length} 行 · 尚未取得这份宏的回放结果。` : !preview && !active ? preparationText(report) || '任务已结束，未取得候选宏；请查看运行状态。' : comparisonText(preview ? c : null));
      const compact = compressionText(job);
      if (compact) text('metrics', el('metrics').textContent + '\n' + compact);
      text('difference', preview ? differenceText(c) : report?.status === 'semantic_mismatch' ? differenceText(c) : '');
      renderMacro(preview);
      el('copy').disabled = !preview; text('copy', certified ? '复制已验证宏' : c ? '复制当前候选（未通过）' : '复制当前候选（未验证）');
      run.disabled = !preview || runningPreview;
      text('title', mode === 'best' ? '最佳宏' : '当前试算');
      text('view', mode === 'best' ? '查看当前试算' : '返回最佳宏');
      root.Jx3Assistant?.setActivity(active && !job.pause_requested && !stopping, 'exact');
    }
    function schedule(delay) {
      root.clearTimeout(timer);
      if (job && !job.done && !pendingControl && !disposed) timer = root.setTimeout(poll, delay ?? (doc.hidden ? 10000 : !visible() || job.phase === 'paused' ? 3000 : 750));
    }
    async function poll() {
      if (!job || job.done || pendingControl || pollFlight || disposed) return;
      const version = epoch, id = job.id, controller = new root.AbortController(); pollFlight = controller;
      try {
        const value = await request(`/api/macro/exact/${encodeURIComponent(id)}?${snapshotQuery(job)}`, undefined, controller.signal);
        if (version === epoch && id === job?.id) { render(value); text('feedback', ''); }
      } catch (error) { if (!controller.signal.aborted && version === epoch) text('feedback', error.message + '；将继续读取进度。'); }
      finally { if (pollFlight === controller) pollFlight = null; if (version === epoch) schedule(); }
    }
    function interruptPoll() { root.clearTimeout(timer); pollFlight?.abort(); pollFlight = null; }
    el('view').addEventListener('click', () => { mode = mode === 'best' ? 'current' : 'best'; if (job) render(job); });
    el('start').addEventListener('click', async () => {
      if (starting || (job && !job.done)) return;
      starting = true; epoch++; interruptPoll(); el('start').disabled = true;
      try {
        text('feedback', '正在读取完整场景…');
        const source = await root.Jx3HarnessWorkspace.capture();
        if (source.simulation.sequence.some(s => s.startsWith('__'))) throw new Error('请先使用手动技能循环，暂不支持宏占位或调试操作。');
        const result = await request('/api/simulate', source.simulation);
        const last = Math.max(0, ...(result.timeline || []).filter(e => !e.triggered).map(e => Number(e.cast_time)));
        const horizon = Math.max(0.1, last + 0.125 + 1e-7);
        if (horizon > 1200) throw new Error('循环验证窗口超过 1200 秒');
        mode = 'best';
        render(await request('/api/macro/exact', { version: source.version, mount: source.mount, simulation: source.simulation, horizon, compress: true }));
        text('feedback', ''); schedule();
      } catch (error) { text('feedback', error.message); }
      finally { starting = false; el('start').disabled = !!job && !job.done; }
    });
    async function control(action) {
      if (!job || job.done || job.cancel_requested || (pendingControl && action !== 'cancel')) return;
      const version = ++epoch, before = job; pendingControl = action; interruptPoll();
      clock.freeze();
      render({ ...job, pause_requested: action !== 'resume', cancel_requested: action === 'cancel', phase: action === 'cancel' ? 'cancelling' : action === 'pause' ? 'pausing' : 'resuming', elapsed_ms: clock.value() });
      text('feedback', action === 'cancel' ? '正在停止，保留已有宏…' : action === 'pause' ? '正在暂停…' : '正在继续…');
      try {
        const value = await request(`/api/macro/exact/${encodeURIComponent(job.id)}/${action}?${snapshotQuery(job, false)}`, {});
        if (version === epoch) { render(value); text('feedback', ''); }
      } catch (error) { if (version === epoch) { render(before); text('feedback', error.message); } }
      finally { if (version === epoch) { pendingControl = ''; render(job); schedule(0); } }
    }
    el('pause').addEventListener('click', () => control(job?.pause_requested ? 'resume' : 'pause'));
    el('stop').addEventListener('click', () => control('cancel'));
    run.addEventListener('click', async () => {
      if (!preview || !job || runningPreview) return;
      const macro = preview, id = job.id, target = output.value, tabs = root.Jx3LoopTabs;
      runningPreview = true; run.disabled = true; output.disabled = true; run.textContent = '运行中…';
      const feedback = message => { el('run_feedback').textContent = message; };
      try {
        if (!tabs?.ownsAutosave()) throw new Error('循环页仍在载入，请稍后重试。');
        const stamp = tabs.outputStamp();
        const source = await request(`/api/macro/exact/${encodeURIComponent(id)}/source`);
        const current = await root.Jx3HarnessWorkspace.capture({ allowEmpty: true });
        if (source.version !== current.version || source.mount !== current.mount
            || !root.Jx3HarnessWorkspace.equivalent(sceneParameters(source.simulation), sceneParameters(current.simulation)))
          throw new Error('当前公共场景参数与模板不同，请恢复模板参数或重新读取模板后运行。');
        const body = previewRequest(source, macro);
        feedback('按模板参数运行…');
        const result = await request('/api/simulate', body, undefined, 60000);
        if (!(result.timeline || []).some(event => !event.triggered)) throw new Error(result.skipped?.[0]?.[1] || '当前宏没有成功施放技能，目标页保持原样。');
        const name = await tabs.importMacro(target, body, result, stamp);
        feedback(`已运行至「${name}」${target ? '，原内容已保留在隐藏备份页' : ''}。`);
      } catch (error) { feedback(error.message); }
      finally { runningPreview = false; run.disabled = !preview; output.disabled = false; run.textContent = '运行当前宏'; }
    });
    const runFeedback = doc.createElement('p'); runFeedback.id = 'em_run_feedback'; runFeedback.className = 'em-muted'; runFeedback.setAttribute('role', 'status'); run.parentElement.after(runFeedback);
    el('copy').addEventListener('click', async () => {
      try {
        if (root.navigator.clipboard?.writeText && root.isSecureContext) await root.navigator.clipboard.writeText(preview);
        else { const range = doc.createRange(); range.selectNodeContents(el('macro')); const selection = root.getSelection(); selection.removeAllRanges(); selection.addRange(range); if (!doc.execCommand('copy')) throw new Error('请选中宏文本后手动复制。'); }
        el('feedback').textContent = '已复制当前宏。';
      } catch (error) { el('feedback').textContent = error.message; }
    });
    async function restore() {
      if (restoring || starting || disposed) return;
      restoring = true; const version = epoch;
      try {
        const data = await request('/api/macro/exact?' + snapshotQuery(job));
        if (version !== epoch || disposed) return;
        if (data.job) { render(data.job); schedule(); }
        else if (!data.available) text('feedback', '服务缺少精确合成运行文件，请按文档安装。');
      } catch (error) { if (version === epoch) text('feedback', error.message); }
      finally { restoring = false; }
    }
    root.addEventListener('jx3-assistant-mode', event => {
      if (event.detail.mode !== 'exact') return;
      if (!starting && !job) void restore();
      tick(); if (!pollFlight) schedule(0);
    });
    doc.addEventListener('visibilitychange', () => { tick(); if (!pollFlight) schedule(doc.hidden ? 10000 : 0); });
    root.addEventListener('pagehide', () => { disposed = true; epoch++; interruptPoll(); root.clearTimeout(ticker); });
    root.addEventListener('pageshow', () => { if (disposed) { disposed = false; void restore(); } });
    void restore();
  }
  return { mount, comparisonText, differenceText, preparationText, currentCandidate, displayCandidate, macroDiff, runClock, mergeSnapshot, snapshotQuery, sceneParameters, previewRequest, phaseText, compressionText };
});
