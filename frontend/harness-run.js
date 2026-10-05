/* Autonomous experiment UI. Simulation facts and tool execution stay on the server. */
(function (root, factory) {
  'use strict';
  const api = factory();
  if (typeof module === 'object' && module.exports) module.exports = api;
  if (root) api.mount(root);
})(typeof window !== 'undefined' ? window : null, function () {
  'use strict';
  const goals = {
    macro: '根据当前技能轴写出可运行的宏，逐步验证动作顺序、资源与时机，尽量复现原轴。',
    rotation: '在当前装备和战斗环境下优化输出循环，寻找更高 DPS 的可运行宏，并验证网络延迟变化下的表现。',
    equipment: '保持当前战斗环境和循环，优化当前配装。保留强化、镶嵌、附魔和锁定部位，用模拟验证收益。',
    joint: '联合优化当前循环和配装：先建立基线，再根据实验结果选择下一步，在预算内寻找可复现的更高 DPS 方案。'
  };
  const labels = { running: '实验进行中', completed: '实验已交付', cancelled: '已停止', failed: '运行失败', interrupted: '已中断', budget_exhausted: '预算已用尽', paused: '已暂停', cancelling: '正在停止', unavailable: '记录不可用' };
  const versions = { ShanHaiYuanLiu:'山海源流', AnYingQianJi:'暗影千机', CangShengZhuShiTest:'苍生铸世测试服' };
  const steps = { inspect:'观察场景', experiment:'开展实验', evaluate:'测试假设', compile_macro:'推导宏', search_rotation:'探索循环', optimize_equipment:'配装实验', validate:'独立验证', record_learning:'记录观察', finish:'交付方案', model_started:'模型推演', model_completed:'推演完成', learning:'记录观察', experiment_started:'开始实验', experiment_completed:'实验完成', tool_error:'调整策略', duplicate_suppressed:'复用已有证据', workspace_prepared:'准备应用', workspace_undo_prepared:'准备撤销' };
  const slots = { HAT:'帽子', JACKET:'衣服', BELT:'腰带', WRIST:'护腕', BOTTOMS:'裤子', SHOES:'鞋子', NECKLACE:'项链', PENDANT:'腰坠', RING_1:'戒指一', RING_2:'戒指二', PRIMARY_WEAPON:'主武器', SECONDARY_WEAPON:'副武器' };
  const clone = value => JSON.parse(JSON.stringify(value));
  const isRunning = value => !!value && (value.running === true || value.status === 'running' || value.status === 'cancelling');
  function acceptSnapshot(current, incoming, runId) {
    return !!incoming && incoming.run_id === runId && typeof incoming.status === 'string' && Number.isFinite(incoming.sequence)
      && !(incoming.sequence < Number(current?.sequence ?? -1));
  }
  function chooseProvider(profiles, selected) {
    const available = (profiles || []).filter(profile => profile.available);
    return available.find(profile => profile.id === selected)?.id
      || available.find(profile => /deepseek/i.test(profile.id + profile.model) && /flash/i.test(profile.id + profile.model))?.id
      || available.find(profile => /deepseek/i.test(profile.id + profile.model))?.id
      || available.find(profile => !/offline/i.test(profile.id))?.id || available[0]?.id || '';
  }
  function candidateSummary(artifact) {
    const result = artifact?.result || {}, best = result.best || (result.metrics ? result : null), baseline = result.baseline;
    const dps = value => Number.isFinite(value?.metrics?.dps) ? value.metrics.dps : Number.isFinite(value?.dps) ? value.dps : null;
    return { best, baseline, bestDps: dps(best), baselineDps: dps(baseline),
      macro: best?.macro_text || artifact?.simulation?.macro_text || '',
      verified: best?.verified === true, alignment: best?.alignment?.summary, limitations: result.limitations || [] };
  }
  function dpsComparable(artifact) {
    const result = artifact?.result || {};
    if (typeof result.dps_comparable === 'boolean') return result.dps_comparable;
    if (artifact?.kind === 'search_rotation' || result.task === 'search_rotation') return Number.isFinite(result.duration_seconds) && !!result.baseline && !!result.best;
    return !!result.baseline_policy?.hash && result.baseline_policy.hash === result.candidate_policy?.hash;
  }
  function mount(root) {
    const doc = root.document, panel = doc.getElementById('assistant_harness_panel');
    if (!panel || doc.getElementById('hr_goal')) return;
    panel.innerHTML = `<div class="hr-workbench">
      <div class="hr-intro"><span class="hr-eyebrow">目标 → 实验 → 方案</span><h2>一起推演下一种可能</h2><p>描述目标，器灵会读取当前循环与配装，自主调用模拟器试验、比较并继续改进。</p></div>
      <label class="hr-goal-label" for="hr_goal">这次想解决什么？</label>
      <textarea id="hr_goal" class="hr-goal" rows="3" maxlength="2500" placeholder="例如：把这条技能轴写成双体态宏，再找出高延迟下最影响输出的一步…"></textarea>
      <div class="hr-intents" aria-label="目标灵感"><button type="button" data-hr-goal="macro">写宏</button><button type="button" data-hr-goal="rotation">优化循环</button><button type="button" data-hr-goal="equipment">自动配装</button><button type="button" data-hr-goal="joint">联合优化 ↗</button></div>
      <div class="hr-provider"><label for="hr_provider">推演模型</label><select id="hr_provider" aria-label="推演模型"><option value="">正在读取…</option></select><button type="button" id="hr_settings" title="复用 AI 模型与自定义接口设置">模型设置</button></div>
      <details class="hr-constraints"><summary>约束与预算 <span>控制实验范围</span></summary>
        <div class="hr-fields"><label>宏模拟窗口（秒）<input id="hr_duration" type="number" min="10" max="600" value="120"></label><label>宏页数上限<select id="hr_pages"><option>1</option><option selected>2</option><option>3</option><option>4</option><option>5</option><option>6</option></select></label>
        <label>模型调用上限<input id="hr_calls" type="number" min="1" max="32" value="16"></label><label>模拟次数上限<input id="hr_sims" type="number" min="4" max="1024" value="192"></label><label>总时限（秒）<input id="hr_seconds" type="number" min="10" max="900" value="240"></label><label>总 Token 上限<input id="hr_tokens" type="number" min="4096" max="256000" value="192000"></label><label>单轮输出 Token 上限<input id="hr_output_tokens" type="number" min="1024" max="8192" value="8192"></label>
        <label>装备最低品级<input id="hr_item_min" type="number" min="0" placeholder="不限"></label><label>装备最高品级<input id="hr_item_max" type="number" min="0" placeholder="不限"></label><label>加速下限<input id="hr_haste_min" type="number" min="0" placeholder="不限"></label><label>加速上限<input id="hr_haste_max" type="number" min="0" placeholder="不限"></label></div>
        <label class="hr-wide-label">允许搜索的技能（可选，逗号分隔）<input id="hr_skills" placeholder="留空：从当前轴和宏推导"></label>
        <fieldset class="hr-locks"><legend>锁定配装部位</legend><div id="hr_locks"></div></fieldset><p class="hr-hint">手动轴保留完整动作；宏按窗口模拟。配装候选来自当前版本目录，保留现有强化、镶嵌与附魔。应用前会展示差异。</p>
      </details>
      <div class="hr-start-row"><button type="button" id="hr_start" class="hr-primary">开始自主实验 <span>↗</span></button><button type="button" id="hr_editor">查看工作区</button></div>
      <p id="hr_feedback" class="hr-feedback" role="status" aria-live="polite">读取当前编辑器的完整环境；后台运行不会占住助手。</p>
      <section id="hr_run" class="hr-run" hidden aria-label="自主实验进度"><div class="hr-section-head"><div><span id="hr_status" class="hr-status"></span><span id="hr_model" class="hr-model"></span></div><button type="button" id="hr_cancel">停止</button></div><p id="hr_run_goal" class="hr-frozen-goal"></p><p id="hr_message" class="hr-message" aria-live="polite"></p><div id="hr_usage" class="hr-usage"></div><div class="hr-budget-bar" aria-hidden="true"><i id="hr_budget_bar"></i></div><p id="hr_scene" class="hr-scene"></p>
      <details class="hr-events" open><summary>实验过程 <span id="hr_event_count"></span></summary><ol id="hr_events"></ol></details>
      <section id="hr_delivery" class="hr-delivery" hidden><div class="hr-section-head"><h3>候选与证据</h3><span id="hr_completion" class="hr-badge"></span></div><p id="hr_conclusion"></p><label class="hr-wide-label">查看方案<select id="hr_artifacts"></select></label><div id="hr_evidence"></div><div class="hr-result-actions"><button type="button" class="hr-primary" id="hr_preview">预览应用方案</button><button type="button" id="hr_undo" disabled>撤销本次应用</button></div></section>
      <div class="hr-run-actions"><button type="button" id="hr_resume" hidden>继续剩余预算</button><button type="button" id="hr_export">下载实验包</button><button type="button" id="hr_details">完整记录</button><button type="button" id="hr_new_goal">调整目标</button></div></section>
      <div class="hr-recent-head"><label for="hr_recent">最近实验</label><button type="button" id="hr_refresh" aria-label="刷新最近实验">↻</button></div><select id="hr_recent"><option value="">选择最近实验…</option></select>
      <div class="hr-footer"><span>数值由模拟器验证 · 方案可追溯</span><button type="button" id="hr_advanced">技能轴写宏工具 ↗</button></div>
    </div>`;
    const $ = id => doc.getElementById('hr_' + id);
    const create = (tag, text, className) => { const el = doc.createElement(tag); if (text != null) el.textContent = text; if (className) el.className = className; return el; };
    for (const [position, name] of Object.entries(slots)) { const label = create('label'), check = create('input'); check.type = 'checkbox'; check.value = position; label.append(check, create('span', name)); $('locks').append(label); }
    const state = { id: null, snapshot: null, source: null, bundle: null, stream: null, timer: null, serial: 0, preparing: false, applying: false, selected: '', selectedByUser: false, transaction: null, cancelRequested: false, eventSignature: '', artifactSignature: '' };
    const pathFor = (id, suffix = '') => `/api/harness/runs/${encodeURIComponent(id)}${suffix}`;
    const path = (suffix = '') => pathFor(state.id, suffix);
    const feedback = (message, error = false) => { $('feedback').textContent = message; $('feedback').dataset.error = String(error); };
    const number = value => Number.isFinite(value) ? Math.round(value).toLocaleString('zh-CN') : '—';
    async function request(url, body) {
      const response = await root.fetch(url, { cache: 'no-store', ...(body !== undefined ? { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) } : {}), signal: root.AbortSignal.timeout(20000) });
      let data = null; try { data = await response.json(); } catch (_) {}
      if (!response.ok) {
        const error = new Error(data?.error?.message || (typeof data?.error === 'string' ? data.error : '') || (response.status === 404 ? '当前后端还没有自主实验接口，请更新服务。' : `请求失败（${response.status}）`));
        error.status = response.status; throw error;
      }
      return data;
    }
    function controls() {
      const running = isRunning(state.snapshot), busy = state.preparing || state.applying;
      $('start').disabled = busy || running || !$('provider').value;
      $('start').firstChild.textContent = state.preparing ? '正在读取工作区… ' : running ? '器灵正在推演… ' : '开始自主实验 ';
      $('cancel').disabled = !running || state.cancelRequested;
      $('cancel').textContent = state.cancelRequested ? '正在停止…' : '停止';
      $('resume').hidden = !state.snapshot?.resumable;
      $('resume').disabled = busy || running;
      $('preview').disabled = busy || running || !selectedArtifact();
      $('undo').disabled = busy || running || !state.transaction;
      $('recent').disabled = busy; $('goal').disabled = state.preparing;
      root.Jx3Assistant?.setActivity(running || state.preparing, 'harness');
    }
    async function providers(selected) {
      try {
        const data = await request('/api/agent/providers'), profiles = data?.profiles || [];
        const preferred = chooseProvider(profiles, selected || $('provider').value);
        $('provider').replaceChildren();
        for (const profile of profiles) { const option = create('option', `${profile.label || profile.id}${profile.available ? '' : ' · 未配置'}`); option.value = profile.id; option.disabled = !profile.available; $('provider').append(option); }
        if (!preferred) { const option = create('option', '先在模型设置中配置接口'); option.value = ''; $('provider').prepend(option); }
        $('provider').value = preferred; controls();
      } catch (error) { feedback(error.message, true); }
    }
    function value(id, min, max) {
      const n = Number($(id).value);
      if (!Number.isFinite(n) || n < min || n > max || !Number.isInteger(n)) throw new Error(`请检查“${$(id).parentElement.firstChild.textContent.trim()}”的范围。`);
      return n;
    }
    function settings() {
      const constraints = { duration_seconds: value('duration',10,600), max_pages: value('pages',1,6),
        allowed_skills: $('skills').value.split(/[,，\n]/).map(s => s.trim()).filter(Boolean), locked_slots: [...$('locks').querySelectorAll('input:checked')].map(el => el.value),
        candidate_source: 'catalog', candidate_ids: {}, allowed_sources: [], max_candidates_per_slot: 12 };
      for (const [id, name] of [['item_min','min_item_level'],['item_max','max_item_level'],['haste_min','haste_min'],['haste_max','haste_max']]) if ($(id).value !== '') constraints[name] = value(id,0,10000000);
      if (constraints.min_item_level > constraints.max_item_level || constraints.haste_min > constraints.haste_max) throw new Error('约束下限不能高于上限。');
      return { constraints, budget: { max_model_calls: value('calls',1,32), max_simulations: value('sims',4,1024), wall_time_ms: value('seconds',10,900)*1000, max_output_tokens: value('output_tokens',1024,8192), max_total_tokens: value('tokens',4096,256000) } };
    }
    async function start() {
      if (state.preparing || state.applying || isRunning(state.snapshot)) return;
      const goal = $('goal').value.trim(); if (!goal) { feedback('写下一个目标，或点上方灵感开始。'); $('goal').focus(); return; }
      state.preparing = true; controls();
      try {
        const options = settings(); feedback('正在读取当前循环、配装和完整战斗环境…');
        const source = await root.Jx3HarnessWorkspace.capture();
        const body = { goal, provider_profile: $('provider').value, simulation: source.simulation, version: source.version, mount: source.mount, ...options };
        if (source.equipment?.slots?.PRIMARY_WEAPON?.equip_id) body.equipment = source.equipment;
        const created = await request('/api/harness/runs', body);
        if (!created?.run_id) throw new Error('服务未返回实验标识，请刷新最近实验。');
        state.source = source; state.transaction = null; state.selected = ''; state.selectedByUser = false; state.bundle = null;
        await follow(created.run_id, null, true);
        feedback('实验已开始。器灵会根据每次结果自行选择下一步，完成后可预览并应用。');
        $('run').scrollIntoView({ block:'start', behavior:'smooth' });
        void recent();
      } catch (error) { feedback(error.message, true); }
      finally { state.preparing = false; controls(); }
    }
    function stopWatch() { state.stream?.close(); state.stream = null; clearTimeout(state.timer); state.timer = null; }
    async function follow(id, snapshot, preserveSource = false) {
      if (state.applying) { feedback('工作区正在更新，请等待本次操作完成。'); return; }
      stopWatch(); const serial = ++state.serial; state.id = id; state.snapshot = null; state.bundle = null; state.cancelRequested = false; state.eventSignature = ''; state.artifactSignature = '';
      if (!preserveSource) { state.source = null; state.transaction = null; state.selected = ''; state.selectedByUser = false; }
      if (snapshot) update(snapshot);
      try { const latest = await request(path()); if (serial !== state.serial) return; update(latest); }
      catch (error) { if (serial === state.serial) feedback(error.message, true); }
      if (serial !== state.serial) return;
      if (isRunning(state.snapshot) && root.EventSource) {
        const stream = new root.EventSource(path('/events')); state.stream = stream;
        const handle = event => { if (serial !== state.serial) return; try { update(JSON.parse(event.data)); } catch (_) {} };
        stream.addEventListener('progress',handle); stream.addEventListener('completed',handle);
        stream.onerror = () => { stream.close(); if (state.stream === stream) state.stream = null; };
      }
      poll(serial);
    }
    function poll(serial) {
      clearTimeout(state.timer);
      if (serial !== state.serial || !isRunning(state.snapshot)) return;
      state.timer = setTimeout(async () => {
        try { const incoming = await request(path()); if (serial === state.serial) update(incoming); }
        catch (error) {
          if (serial === state.serial && error.status === 404) update({ ...state.snapshot, run_id:state.id, sequence:(state.snapshot?.sequence || 0)+1, status:'unavailable', running:false, resumable:false, message:'此 worker 中已找不到该实验，可刷新记录或重新开始。' });
          else if (serial === state.serial) feedback('连接暂时中断，将继续读取实验进度。' + error.message, true);
        }
        poll(serial);
      }, state.stream ? 8000 : 2000);
    }
    function update(snapshot) {
      if (!acceptSnapshot(state.snapshot, snapshot, state.id)) return;
      const wasRunning = isRunning(state.snapshot);
      if (snapshot.sequence !== state.snapshot?.sequence) state.bundle = null;
      state.snapshot = snapshot;
      $('run').hidden = false; $('status').textContent = labels[snapshot.status] || snapshot.status;
      $('status').dataset.running = String(isRunning(snapshot)); $('model').textContent = snapshot.model || snapshot.provider_profile || '';
      $('message').textContent = snapshot.message || snapshot.phase || '';
      $('run_goal').textContent = snapshot.goal ? `本次目标 · ${snapshot.goal}` : '';
      const usage = snapshot.usage || {}, budget = snapshot.budget || {};
      $('usage').replaceChildren();
      for (const [label, used, limit] of [['模型',usage.model_calls,budget.max_model_calls],['模拟',usage.simulations,budget.max_simulations],['Token',usage.total_tokens,budget.max_total_tokens],['秒',Number(usage.elapsed_ms || 0)/1000,Number(budget.wall_time_ms || 0)/1000]]) {
        const cell = create('div'); cell.append(create('strong', `${number(used)} / ${number(limit)}`), create('span',label)); $('usage').append(cell);
      }
      const fraction = Math.max(...[['model_calls','max_model_calls'],['simulations','max_simulations'],['total_tokens','max_total_tokens'],['elapsed_ms','wall_time_ms']].map(([used,limit]) => (usage[used] || 0) / (budget[limit] || Infinity)));
      $('budget_bar').style.width = `${Math.min(100, fraction*100)}%`;
      $('scene').textContent = `${snapshot.mount === 'TieGuYi' ? '铁骨衣' : '分山劲'} · ${versions[snapshot.version] || snapshot.version} · 场景 ${String(snapshot.scenario_hash || '').slice(0,12)}`;
      $('scene').title = `scene: ${snapshot.scenario_hash || ''}\nexperiment: ${snapshot.experiment_hash || ''}`;
      const events = snapshot.events || [], signature = JSON.stringify(events);
      if (signature !== state.eventSignature) {
        const list = $('events'), nearBottom = list.scrollHeight - list.scrollTop - list.clientHeight < 55;
        list.replaceChildren();
        for (const event of events) {
          const li = create('li'); li.dataset.kind = event.kind || '';
          const head = create('div', null, 'hr-event-head'); head.append(create('span', steps[event.tool] || steps[event.kind] || '实验记录'), create('small', `#${event.sequence}`));
          li.append(head, create('p', event.message || ''));
          if (event.artifact_id) { const button = create('button','查看证据 →','hr-event-link'); button.type = 'button'; button.addEventListener('click', () => { state.selected = event.artifact_id; state.selectedByUser = true; renderArtifacts(); $('delivery').scrollIntoView({block:'nearest',behavior:'smooth'}); }); li.append(button); }
          if (event.data) { const detail = create('details'), summary = create('summary','参数与结果'); detail.append(summary,create('pre',JSON.stringify(event.data,null,2))); li.append(detail); }
          list.append(li);
        }
        if (nearBottom) list.scrollTop = list.scrollHeight;
        state.eventSignature = signature;
      }
      $('event_count').textContent = `${events.length} 条记录`;
      renderArtifacts(); controls();
      if (!isRunning(snapshot)) {
        stopWatch();
        if (wasRunning) { feedback(snapshot.result?.summary || snapshot.message || '实验已结束，已保留取得的证据。'); void recent(); }
      }
    }
    function selectedArtifact() { return (state.snapshot?.artifacts || []).find(a => a.id === state.selected) || null; }
    function renderArtifacts() {
      const artifacts = state.snapshot?.artifacts || [], result = state.snapshot?.result;
      $('delivery').hidden = !artifacts.length && !result;
      if (!state.selectedByUser || !artifacts.some(a => a.id === state.selected)) state.selected = result?.selected_artifact_id || artifacts.at(-1)?.id || '';
      const signature = JSON.stringify({ artifacts, result, selected: state.selected });
      if (signature === state.artifactSignature) return; state.artifactSignature = signature;
      $('conclusion').textContent = result?.summary || '已取得的中间方案与证据会保留在这里。';
      $('completion').textContent = ({ verified:'已验证交付',partial:'阶段结果',no_solution:'尚无可用方案' })[result?.completion] || (isRunning(state.snapshot) ? '持续更新' : '已保留');
      $('artifacts').replaceChildren();
      for (const artifact of artifacts) { const option = create('option',`${artifact.id} · ${artifact.summary || artifact.kind}`); option.value = artifact.id; $('artifacts').append(option); }
      $('artifacts').value = state.selected; renderEvidence(selectedArtifact());
    }
    function renderEvidence(artifact) {
      const container = $('evidence'); container.replaceChildren(); if (!artifact) return;
      const summary = candidateSummary(artifact), result = artifact.result || {};
      container.append(create('p',artifact.summary || artifact.kind,'hr-artifact-summary'));
      if (summary.bestDps != null || summary.baselineDps != null) {
        const metrics = create('div',null,'hr-comparison');
        const scope = artifact.kind === 'validate' ? '验证场景 ' : '';
        for (const [label, value] of [[scope+'基线 DPS',summary.baselineDps],[scope+'候选 DPS',summary.bestDps]]) { const cell = create('div'); cell.append(create('span',label),create('strong',number(value))); metrics.append(cell); }
        if (dpsComparable(artifact) && summary.baselineDps > 0 && summary.bestDps != null) { const change = (summary.bestDps/summary.baselineDps-1)*100; metrics.append(create('b',`${change >= 0 ? '+' : ''}${change.toFixed(2)}%`)); }
        container.append(metrics);
        if (!dpsComparable(artifact) && summary.baselineDps != null && summary.bestDps != null) container.append(create('p','两侧尚未建立同口径比较；这些 DPS 数值不表示输出提升。','hr-hint'));
      }
      if (summary.alignment) {
        const a = summary.alignment; container.append(create('p',`动作对齐：缺失 ${a.missing || 0} · 多余 ${a.extra || 0} · 状态变化 ${a.changed || 0} · 时间偏差 ${a.time_error || 0}`,'hr-hint'));
      }
      if (summary.macro) {
        const label = create('div',null,'hr-section-head'); label.append(create('h4','候选宏'));
        const copy = create('button','复制文本'); copy.type = 'button'; copy.addEventListener('click', async () => { try { await root.navigator.clipboard.writeText(summary.macro); feedback('宏文本已复制；也可用“预览应用方案”写入循环编辑器。'); } catch (_) { feedback('浏览器未允许剪贴板，请从下方文本选择复制。',true); } }); label.append(copy); container.append(label,create('pre',summary.macro,'hr-code'));
      }
      if (result.slot_diff?.length) {
        const table = create('table',null,'hr-diff-table');
        for (const item of result.slot_diff) { const tr = create('tr'); tr.append(create('th',slots[item.position || item.slot] || item.position || item.slot || '装备'),create('td', item.before_name || item.before?.name || String(item.before_id ?? item.before?.equip_id ?? '—')),create('td','→'),create('td',item.after_name || item.after?.name || String(item.after_id ?? item.after?.equip_id ?? '—'))); table.append(tr); } container.append(table);
      }
      if (result.validation) container.append(create('p', `延迟留出验证：${result.validation.status || '已记录'}${result.validation.network_delay != null ? ' · '+result.validation.network_delay+' ms' : ''}。只代表这一组额外场景。`,'hr-hint'));
      const limitations = [...summary.limitations, ...(state.snapshot?.result?.limitations || [])];
      if (limitations.length) { const details = create('details',null,'hr-limits'); details.append(create('summary','验证范围与局限')); const list = create('ul'); [...new Set(limitations)].forEach(text => list.append(create('li',text))); details.append(list); container.append(details); }
      const metadata = create('details',null,'hr-evidence-meta'); metadata.append(create('summary','复现信息与完整结果'),create('pre',JSON.stringify({artifact_id:artifact.id,parent_id:artifact.parent_id,scenario_hash:artifact.scenario_hash,evidence_ids:state.snapshot?.result?.evidence_ids,result},null,2))); container.append(metadata);
    }
    async function bundle() { if (!state.id) return null; if (!state.bundle) { const id = state.id; const data = await request(path('/artifacts')); if (id !== state.id) throw new Error('查看的实验已切换，请重试。'); state.bundle = data; } return state.bundle; }
    async function recent() {
      try { const data = await request('/api/harness/runs'); $('recent').replaceChildren(create('option','选择最近实验…')); $('recent').firstChild.value = '';
        for (const item of data.runs || []) { const option = create('option',`${labels[item.status] || item.status} · ${item.goal || item.run_id}`); option.value = item.run_id; $('recent').append(option); } $('recent').value = state.id || '';
      } catch (error) { if (root.Jx3Assistant?.isOpen()) feedback(error.message,true); }
    }
    function dialog(title) {
      const el = create('dialog',null,'hr-dialog'), head = create('div',null,'hr-section-head'), close = create('button','×'); close.type = 'button'; close.setAttribute('aria-label','关闭预览'); close.addEventListener('click',()=>el.close()); head.append(create('h3',title),close); el.append(head); doc.body.append(el); el.addEventListener('close',()=>el.remove()); return el;
    }
    async function preview() {
      if (state.applying) return;
      state.applying = true; controls();
      try {
        const runId = state.id, serial = state.serial, scenarioHash = state.snapshot.scenario_hash, id = state.selected;
        const sameRun = () => state.id === runId && state.serial === serial;
        const exported = await bundle();
        if (!sameRun()) throw new Error('查看的实验已切换，请重新预览。');
        const artifact = exported?.artifacts?.find(a => a.id === id);
        if (!artifact?.simulation) throw new Error('这条证据不是可应用的完整方案，请选择有完整模拟结果的候选。');
        const current = await root.Jx3HarnessWorkspace.capture();
        if (!state.source) {
          const original = exported.request;
          if (!original || current.version !== original.version || current.mount !== original.mount || !root.Jx3HarnessWorkspace.equivalent(current.simulation, original.simulation)
            || (original.equipment && (!root.Jx3HarnessWorkspace.equivalent(current.equipment?.slots,original.equipment.slots) || current.equipment?.stone_id !== original.equipment.stone_id))) throw new Error('当前工作区与这次实验的初始场景不同。请重新开始实验后应用。');
          state.source = current;
        }
        if (current.sourceKey !== state.source.sourceKey) throw new Error('开始实验后工作区已改变。请保留当前编辑并重新开始实验。');
        const after = { simulation: artifact.simulation, equipment: artifact.equipment || current.equipment };
        const changes = root.Jx3HarnessWorkspace.preview(current, after), modal = dialog('应用前预览');
        modal.append(create('p','将修改当前循环编辑器与配装器。保留本页撤销点；其他战斗环境保持本次冻结场景。','hr-hint'));
        if (!changes.length) modal.append(create('p','方案与当前工作区一致。'));
        for (const change of changes) { const section = create('details'); section.open = ['macro_text','sequence'].includes(change.field); section.append(create('summary', change.field)); const columns = create('div',null,'hr-preview-diff'); columns.append(create('pre',JSON.stringify(change.before,null,2)),create('pre',JSON.stringify(change.after,null,2))); section.append(columns); modal.append(section); }
        const actions = create('div',null,'hr-result-actions'), apply = create('button','确认应用方案','hr-primary'); apply.type = 'button';
        let committing = false;
        modal.addEventListener('cancel', event => { if (committing) event.preventDefault(); });
        apply.addEventListener('click',async () => {
          if (committing) return;
          committing = true; state.applying = true; controls(); apply.disabled = true;
          const closeButton = modal.querySelector('[aria-label="关闭预览"]'); if (closeButton) closeButton.disabled = true;
          let transaction;
          try {
            if (!sameRun()) throw new Error('查看的实验已切换，请重新预览。');
            if (root.Jx3HarnessWorkspace.currentKey() !== current.sourceKey) throw new Error('预览期间工作区发生变化，请重新预览。');
            transaction = await request(pathFor(runId,'/apply'),{artifact_id:id,expected_scenario_hash:scenarioHash});
            if (!transaction?.transaction_id || !transaction.after) throw new Error('服务没有返回可应用的事务。');
            if (!sameRun()) throw new Error('查看的实验已切换，未写入工作区。');
            await root.Jx3HarnessWorkspace.apply({...transaction.after,expected_fingerprint:artifact.result?.validation?.application_fingerprint || artifact.result?.best?.fingerprint},current.sourceKey,transaction.transaction_id);
            state.transaction = {id:transaction.transaction_id,runId}; modal.close(); feedback('方案已应用到循环编辑器和配装器；可点击“撤销本次应用”恢复。');
          } catch (error) {
            if (transaction?.transaction_id) { try { await request(pathFor(runId,'/undo'),{transaction_id:transaction.transaction_id}); } catch (_) { feedback('工作区应用失败；服务端事务回退未确认，请保留实验包。',true); } }
            const errorNode = create('p',error.message,'hr-feedback'); errorNode.dataset.error = 'true'; modal.append(errorNode); apply.disabled = false;
          } finally { committing = false; state.applying = false; if (closeButton) closeButton.disabled = false; controls(); }
        }); actions.append(apply); modal.append(actions); modal.showModal();
      } catch (error) { feedback(error.message,true); }
      finally { state.applying = false; controls(); }
    }
    $('start').addEventListener('click',start);
    $('goal').addEventListener('keydown',event => { if ((event.ctrlKey || event.metaKey) && event.key === 'Enter') { event.preventDefault(); void start(); } });
    panel.querySelectorAll('[data-hr-goal]').forEach(button => button.addEventListener('click',()=>{ $('goal').value=goals[button.dataset.hrGoal]; $('goal').focus(); }));
    $('provider').addEventListener('change',controls);
    $('settings').addEventListener('click',()=>doc.querySelector('#sim_ai_dock [data-agent-provider-settings]')?.click());
    $('editor').addEventListener('click',()=>root.Jx3Nav?.switchPage('page-sim'));
    $('new_goal').addEventListener('click',()=>{ $('goal').scrollIntoView({block:'center',behavior:'smooth'}); $('goal').focus({preventScroll:true}); });
    $('advanced').addEventListener('click',()=>{ root.Jx3Nav?.switchPage('page-harness'); root.Jx3Assistant?.close(); });
    $('artifacts').addEventListener('change',()=>{state.selected=$('artifacts').value;state.selectedByUser=true;renderArtifacts();controls();});
    $('recent').addEventListener('change',()=>{ if ($('recent').value) void follow($('recent').value); });
    $('refresh').addEventListener('click',()=>void recent());
    $('cancel').addEventListener('click',async()=>{state.cancelRequested=true;controls();try{const data=await request(path('/cancel'),{});if(data?.run_id)update(data);feedback('已请求停止；当前计算结束后保留已取得的候选与证据。');}catch(error){state.cancelRequested=false;feedback(error.message,true);controls();}});
    $('resume').addEventListener('click',async()=>{try{await request(path('/resume'),{});await follow(state.id,null,true);feedback('已从检查点继续，使用原场景与剩余预算。');}catch(error){feedback(error.message,true);}});
    $('preview').addEventListener('click',preview);
    $('undo').addEventListener('click',async()=>{if(state.applying||!state.transaction)return;const transaction={...state.transaction};state.applying=true;controls();try{const point=root.Jx3HarnessWorkspace.undoPoint();if(!point||state.id!==transaction.runId||root.Jx3HarnessWorkspace.currentKey()!==point.installedKey)throw new Error('应用后有其他编辑，不能覆盖这些修改。');await request(pathFor(transaction.runId,'/undo'),{transaction_id:transaction.id});const restored=await root.Jx3HarnessWorkspace.undo(transaction.id);state.transaction=null;state.source=restored;feedback('已恢复应用前的配装、循环与属性。');}catch(error){feedback(error.message,true);}finally{state.applying=false;controls();}});
    $('export').addEventListener('click',async()=>{try{state.bundle=null;const data=await bundle(),blob=new root.Blob([JSON.stringify(data,null,2)],{type:'application/json'}),url=root.URL.createObjectURL(blob),a=create('a');a.href=url;a.download=`${state.id}.json`;a.click();setTimeout(()=>root.URL.revokeObjectURL(url),1000);}catch(error){feedback(error.message,true);}});
    $('details').addEventListener('click',async()=>{try{state.bundle=null;const data=await bundle(),modal=dialog('完整实验记录');modal.append(create('pre',JSON.stringify(data,null,2),'hr-code'));modal.showModal();}catch(error){feedback(error.message,true);}});
    root.addEventListener('jx3-agent-providers-changed',event=>void providers(event.detail?.selectedId));
    root.addEventListener('jx3-assistant-mode',event=>{if(event.detail?.mode==='harness'&&event.detail.open){void recent();void providers();}});
    root.addEventListener('beforeunload',stopWatch);
    void providers(); void recent(); controls();
  }
  return { acceptSnapshot, chooseProvider, candidateSummary, dpsComparable, isRunning, goals, mount };
});
