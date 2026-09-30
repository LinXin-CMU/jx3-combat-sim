/* 分歧附近的真实两阶段判定；仅重放冻结请求，不修改模板或宏。 */
(function () {
  'use strict';
  const node = (tag, text, cls) => { const el = document.createElement(tag); if (text != null) el.textContent = text; if (cls) el.className = cls; return el; };
  const seconds = value => Number.isFinite(value) ? `${value.toFixed(3)}s` : '时间未记录';
  function mount(host, options) {
    const { saved, reference, actual, current, locate, focus } = options;
    const block = node('section', null, 'ma-diagnostic');

    const content = node('div'); block.append(content); host.append(block);
    if (!current() || !saved.request) { content.append(node('p', '草稿或环境已变化，请重新运行对照后查看判定。', 'ma-explanation')); return; }
    const time = Jx3MacroRepairSearch.anchor(focus);
    if (time < 0) { content.append(node('p', '预释放不通过宏的两步判定。', 'ma-explanation')); return; }
    const radius = Math.min(5, Math.max(3, (saved.request.network_delay || 0) / 1000 + 1));
    const start = Math.max(0, time - radius), end = Math.min(saved.window, time + 3);
    saved.diagnostics ||= new Map();
    const targetSkill = reference ? Jx3MacroRepair.skill(reference) : null;
    const key = `${start}:${end}:${targetSkill || ''}`;
    async function load() {
      content.replaceChildren(node('p', '正在重放分歧附近的宏判定…', 'ma-explanation'));
      if (!saved.diagnostics.has(key)) {
        const promise = (async () => {
          const response = await fetch('/api/macro/diagnose', {
            method: 'POST', headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ simulation: saved.request, version: saved.version, mount: saved.mount, start, end, target_skill: targetSkill }),
            signal: AbortSignal.timeout(60000),
          });
          const data = await response.json().catch(() => null);
          if (!response.ok || !data?.trace) throw new Error(data?.error || '诊断接口未就绪，请稍后重试。');
          if (data.fingerprint !== saved.actual.fingerprint) throw new Error('重放结果与本次对照不同，请重新运行对照。');
          return data.trace;
        })();
        saved.diagnostics.set(key, promise);
      }
      try {
        const trace = await saved.diagnostics.get(key);
        if (!block.isConnected || !current()) return;
        render(trace);
      } catch (error) {
        saved.diagnostics.delete(key);
        if (!block.isConnected || !current()) return;
        content.replaceChildren(node('p', error.name === 'TimeoutError' ? '诊断超时，请重试。' : error.message, 'ma-explanation'));
        const retry = node('button', '重试诊断', 'ma-button'); retry.type = 'button'; retry.onclick = load; content.append(retry);
      }
    }
    function render(trace) {
      content.replaceChildren();
      const goal = node('div', null, 'ma-diagnostic-goal');
      goal.append(node('div', `模板期待：${reference?.name || '后续无对应技能'}${reference ? ` · ${seconds(reference.cast_time)}` : ''}`, 'ma-diagnostic-fail'),
        node('div', `实际${focus.kind==='missing' ? '后续' : focus.approximate ? '附近' : '对应'}：${actual?.name || '没有释放'}${actual ? ` · ${seconds(actual.cast_time)}` : ''}`, 'ma-diagnostic-pass'));
      content.append(goal);
      if (focus.approximate) content.append(node('p', focus.kind==='missing'?'按模板期待时刻诊断；后续实际技能不代表同一轮。':'未精确配对；按实际轮次诊断，结论仅适用于该轮状态。', 'ma-explanation'));
      if (!focus.approximate) content.append(node('p', '以下为实际判断时刻；施放时间还包含后续延迟。', 'ma-explanation'));
      if (trace.truncated) content.append(node('p', '附近判定过密，达到记录上限；只显示已记录部分。', 'ma-diagnostic-warning'));
      const steps = trace.decisions || [];
      if (!steps.length) { content.append(node('p', '这个时间窗口没有宏判定，可能处于引导、停手或运行结束阶段。', 'ma-explanation')); return; }
      let index=Jx3MacroRepairSearch.decisionIndex(steps,focus);
      const controls = node('div', null, 'ma-diagnostic-controls');
      const select = node('select'); select.setAttribute('aria-label', '选择宏判定轮次');
      steps.forEach((step, i) => { const option = node('option', `${seconds(step.time)} · 第 ${step.page} 页 · ${step.selected || '本轮未选中技能'}`); option.value = i; select.append(option); });
      select.value = String(index);
      const previous = node('button', '上一轮', 'ma-button'), next = node('button', '下一轮', 'ma-button');
      previous.type = next.type = 'button'; controls.append(previous, select, next); content.append(controls);
      const summary=node('div',null,'ma-diagnostic-compact');summary.append(node('strong','分歧诊断'));
      while(content.firstChild)summary.append(content.firstChild);
      const body = node('div',null,'ma-diagnostic-body'); content.append(body);
      const show = () => {
        index = Number(select.value); previous.disabled = index === 0; next.disabled = index === steps.length - 1;
        body.replaceChildren(); const step = steps[index];
        const repair = node('div'); body.append(repair);
        Jx3MacroRepairPanel.render(repair, { ...options, step });
        repair.querySelector('.ma-diagnostic-flow').prepend(summary);
        const logs = node('details'); logs.append(node('summary', '展开完整宏判定日志'));
        const flow = node('div'); logs.append(flow); repair.querySelector('.ma-diagnostic-flow').append(logs);
        flow.append(node('p', `判断时刻 ${seconds(step.time)} · ${({ Shield:'擎盾', Blade:'擎刀', Wall:'盾墙' })[step.stance] || step.stance} · 上次成功技能：${step.last_skill || '无'}`));
        const resource = saved.mount === 'TieGuYi' ? `格挡 ${step.block_value}` : saved.version === 'CangShengZhuShiTest' ? `暴怒 ${step.berserk_value}` : '';
        flow.append(node('p', `怒气 ${step.rage}${resource ? ` / ${resource}` : ''}`));
        function row(line, text, cls) {
          const item = node('div', null, `ma-diagnostic-row ${cls || ''}`);
          const link = node('button', `第 ${line.line} 行 · ${line.skill}`, 'ma-button'); link.type = 'button';
          link.onclick = () => { if (current()) locate(step.page, line.line); };
          item.append(link, node('span', text)); return item;
        }
        flow.append(node('h4', 'Step 1 · 扫描当前宏页，筛选条件成立的技能'));
        for (const line of step.phase1) flow.append(row(line, `${line.condition} → ${line.passed ? '成立，进入技能池' : '不成立，不进入技能池'}`, line.passed ? 'ma-diagnostic-pass' : 'ma-diagnostic-fail'));
        flow.append(node('h4', 'Step 2 · 按技能池顺序，选择第一个可释放技能'));
        const pool = step.phase1.filter(line => line.passed);
        if (!pool.length) flow.append(node('p', '技能池为空，本轮不进入 Step 2。', 'ma-explanation'));
        for (const line of pool) {
          const checked = step.phase2.find(entry => entry.line === line.line);
          const text = !checked ? '前面已选中技能，本行未检查可释放性' : checked.castable ? '可释放 → 本轮选中，停止向后检查' : `${checked.reason} → 继续检查下一行`;
          flow.append(row(line, text, !checked ? '' : checked.castable ? 'ma-diagnostic-pass' : 'ma-diagnostic-fail'));
        }
        const outcome = !step.selected ? '本轮未选中技能，等待状态变化后重新判定。' : step.cast_success === true ? `最终释放：${step.selected} · ${seconds(step.cast_time)}` : step.cast_success === false ? `Step 2 选中 ${step.selected}，后续施放未成功。` : `Step 2 选中 ${step.selected}，没有后续施放记录。`;
        flow.append(node('p', outcome, 'ma-diagnostic-outcome'));
      };
      previous.onclick = () => { select.value = String(index - 1); show(); };
      next.onclick = () => { select.value = String(index + 1); show(); };
      select.onchange = show; show();
    }
    load();
  }
  window.Jx3MacroDiagnostic = Object.freeze({ mount });
})();
