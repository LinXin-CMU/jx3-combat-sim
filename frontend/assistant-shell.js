/* One movable assistant surface for experiments and the existing AI analysis. */
(function (root, factory) {
  'use strict';
  const api = factory();
  if (typeof module === 'object' && module.exports) module.exports = api;
  if (root) api.mount(root);
})(typeof window !== 'undefined' ? window : null, function () {
  'use strict';
  const STORAGE = 'jx3_assistant_geometry_v1';
  // Keep autonomous experiments available in source while hiding their UI entry.
  const VISIBLE_MODES = ['analysis', 'exact'];
  const SIGIL = `<svg viewBox="0 0 32 32" fill="none" aria-hidden="true" focusable="false"><path d="M16 3 27 7v8c0 7-6.5 12-11 15C11.5 27 5 22 5 15V7L16 3Z" fill="currentColor" fill-opacity=".08" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round"/><path d="m16 7 2 4v9l-2 4-2-4v-9l2-4Z" fill="currentColor"/><path d="M8 12h3l2 3m11-3h-3l-2 3M8.5 19l3 3m12-3-3 3" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg>`;
  const clamp = (value, low, high) => Math.max(low, Math.min(high, value));
  function fit(rect, viewport) {
    const margin = 8, width = Math.max(0, viewport.width), height = Math.max(0, viewport.height);
    const availableWidth = Math.max(1, width - margin * 2), availableHeight = Math.max(1, height - margin * 2);
    const w = clamp(Number.isFinite(rect.width) ? rect.width : 530, Math.min(360, availableWidth), availableWidth);
    const h = clamp(Number.isFinite(rect.height) ? rect.height : height - 92, Math.min(420, availableHeight), availableHeight);
    return { x: clamp(Number.isFinite(rect.x) ? rect.x : width - w - 24, margin, Math.max(margin, width - w - margin)),
      y: clamp(Number.isFinite(rect.y) ? rect.y : 76, margin, Math.max(margin, height - h - margin)), width: w, height: h };
  }
  function fitBall(position, viewport) {
    return { x: clamp(Number.isFinite(position.x) ? position.x : viewport.width - 80, 8, Math.max(8, viewport.width - 64)),
      y: clamp(Number.isFinite(position.y) ? position.y : viewport.height - 88, 8, Math.max(8, viewport.height - 64)) };
  }
  function resize(rect, dx, dy, edge, viewport) {
    const min = Math.min(360, Math.max(1, viewport.width - 16));
    if (edge === 'left') {
      const x = clamp(rect.x + dx, 8, rect.x + rect.width - min);
      return fit({ ...rect, x, width: rect.x + rect.width - x }, viewport);
    }
    return fit({ ...rect, width: rect.width + dx, height: rect.height + dy }, viewport);
  }
  function mount(root) {
    const doc = root.document;
    if (doc.getElementById('assistant_shell')) return;
    const shell = doc.createElement('aside');
    shell.id = 'assistant_shell'; shell.className = 'assistant-shell'; shell.hidden = true;
    shell.setAttribute('aria-label', '苍云器灵助手');
    shell.innerHTML = `<div class="assistant-resize-left" role="separator" aria-label="调整助手宽度" aria-orientation="vertical" tabindex="0"></div>
      <header class="assistant-handle"><div class="assistant-brand"><span class="assistant-sigil" aria-hidden="true">${SIGIL}</span><div><strong>苍云器灵</strong><span>把想法变成可验证的方案</span></div></div><div class="assistant-window-actions"><button type="button" data-assistant-reset title="恢复侧栏位置" aria-label="恢复助手位置">↗</button><button type="button" data-assistant-close title="收起，后台任务继续" aria-label="收起助手">−</button></div></header>
      <div class="assistant-tabs" role="tablist" aria-label="助手模式"><button type="button" id="assistant_tab_harness" role="tab" aria-selected="false" hidden aria-controls="assistant_harness_panel" data-assistant-mode="harness"><span>武学助手</span><small>自主实验</small></button><button type="button" id="assistant_tab_analysis" role="tab" aria-selected="true" aria-controls="assistant_analysis_panel" data-assistant-mode="analysis"><span>AI 分析</span><small>对话与诊断</small></button></div>
      <section id="assistant_harness_panel" class="assistant-panel" role="tabpanel" aria-labelledby="assistant_tab_harness" hidden></section>
      <section id="assistant_exact_panel" class="assistant-panel" role="tabpanel" aria-labelledby="assistant_tab_exact" hidden></section>
      <section id="assistant_analysis_panel" class="assistant-panel assistant-analysis" role="tabpanel" aria-labelledby="assistant_tab_analysis"></section>
      <div class="assistant-resize-corner" aria-hidden="true"></div>`;
    const ball = doc.createElement('button');
    const exactTab = doc.createElement('button');
    exactTab.type = 'button'; exactTab.id = 'assistant_tab_exact'; exactTab.setAttribute('data-assistant-mode', 'exact');
    exactTab.setAttribute('role', 'tab'); exactTab.setAttribute('aria-selected', 'false'); exactTab.setAttribute('aria-controls', 'assistant_exact_panel');
    exactTab.innerHTML = '<span>循环合成</span><small>条件宏</small>';
    shell.querySelector('.assistant-tabs').append(exactTab);
    shell.querySelector('.assistant-tabs').append(shell.querySelector('#assistant_tab_harness'));
    ball.id = 'assistant_ball'; ball.className = 'assistant-ball'; ball.type = 'button';
    ball.setAttribute('aria-controls', shell.id); ball.setAttribute('aria-expanded', 'false');
    ball.setAttribute('aria-label', '打开苍云器灵助手；可拖动，方向键移动');
    ball.title = '苍云器灵 · 拖动调整位置';
    ball.innerHTML = `<span class="assistant-ball-ring" aria-hidden="true"></span><span class="assistant-ball-shine" aria-hidden="true"></span><span class="assistant-ball-mark" aria-hidden="true">${SIGIL}</span><span class="assistant-ball-label">器灵</span><i class="assistant-ball-activity" hidden></i>`;
    doc.body.append(shell, ball); doc.body.classList.add('assistant-unified');
    const legacy = doc.getElementById('sim_ai_dock');
    if (legacy) { shell.querySelector('#assistant_analysis_panel').append(legacy); legacy.classList.add('assistant-embedded'); }
    const oldBall = doc.getElementById('sim_ai_fab'); if (oldBall) oldBall.hidden = true;
    let saved = {}; try { saved = JSON.parse(root.localStorage.getItem(STORAGE) || '{}') || {}; } catch (_) {}
    let rect, position, mode = 'analysis', opened = false, suppressClick = false;
    const viewport = () => ({ width: root.innerWidth, height: root.innerHeight });
    const place = () => {
      rect = fit(rect || saved.panel || {}, viewport()); position = fitBall(position || saved.ball || {}, viewport());
      Object.assign(shell.style, { left: `${rect.x}px`, top: `${rect.y}px`, width: `${rect.width}px`, height: `${rect.height}px` });
      Object.assign(ball.style, { left: `${position.x}px`, top: `${position.y}px` });
      shell.querySelector('.assistant-resize-left').setAttribute('aria-valuenow', String(Math.round(rect.width)));
    };
    const save = () => { try { root.localStorage.setItem(STORAGE, JSON.stringify({ panel: rect, ball: position })); } catch (_) {} };
    function select(next) {
      mode = VISIBLE_MODES.includes(next) ? next : 'analysis'; shell.dataset.mode = mode;
      shell.querySelectorAll('[data-assistant-mode]').forEach(tab => {
        const selected = tab.dataset.assistantMode === mode;
        tab.setAttribute('aria-selected', String(selected)); tab.tabIndex = selected ? 0 : -1;
      });
      doc.getElementById('assistant_harness_panel').hidden = mode !== 'harness';
      doc.getElementById('assistant_analysis_panel').hidden = mode !== 'analysis';
      doc.getElementById('assistant_exact_panel').hidden = mode !== 'exact';
      root.Jx3AgentDock?.setEmbeddedVisible(opened && mode === 'analysis');
      root.dispatchEvent(new root.CustomEvent('jx3-assistant-mode', { detail: { mode, open: opened } }));
    }
    function open(next = mode, focus = false) {
      opened = true; shell.hidden = false; ball.setAttribute('aria-expanded', 'true'); select(next); place();
      if (focus) doc.getElementById(({ analysis: 'sim_ai_question', exact: 'em_start', harness: 'hr_goal' })[mode])?.focus({ preventScroll: true });
    }
    function close() {
      opened = false; shell.hidden = true; ball.setAttribute('aria-expanded', 'false');
      root.Jx3AgentDock?.setEmbeddedVisible(false); ball.focus({ preventScroll: true });
    }
    function drag(element, kind) {
      element.addEventListener('pointerdown', event => {
        if (event.button !== 0 || (kind === 'panel' && event.target.closest('button,input,select,textarea,a'))) return;
        const start = { x: event.clientX, y: event.clientY, rect: { ...rect }, ball: { ...position } };
        let moved = false;
        element.setPointerCapture?.(event.pointerId);
        const move = current => {
          const dx = current.clientX - start.x, dy = current.clientY - start.y;
          if (!moved && Math.hypot(dx, dy) < 5) return;
          moved = true; doc.body.classList.add('assistant-dragging');
          if (kind === 'ball') position = fitBall({ x: start.ball.x + dx, y: start.ball.y + dy }, viewport());
          else if (kind === 'panel') rect = fit({ ...start.rect, x: start.rect.x + dx, y: start.rect.y + dy }, viewport());
          else rect = resize(start.rect, dx, dy, kind, viewport());
          place(); current.preventDefault();
        };
        const end = () => {
          element.removeEventListener('pointermove', move); element.removeEventListener('pointerup', end); element.removeEventListener('pointercancel', end);
          doc.body.classList.remove('assistant-dragging');
          if (moved) { save(); if (kind === 'ball') { suppressClick = true; setTimeout(() => { suppressClick = false; }, 0); } }
        };
        element.addEventListener('pointermove', move); element.addEventListener('pointerup', end); element.addEventListener('pointercancel', end);
      });
    }
    place(); select(mode);
    drag(ball, 'ball'); drag(shell.querySelector('.assistant-handle'), 'panel');
    drag(shell.querySelector('.assistant-resize-left'), 'left'); drag(shell.querySelector('.assistant-resize-corner'), 'corner');
    ball.addEventListener('click', () => { if (!suppressClick) opened ? close() : open(mode, true); });
    ball.addEventListener('keydown', event => {
      const delta = { ArrowLeft: [-16, 0], ArrowRight: [16, 0], ArrowUp: [0, -16], ArrowDown: [0, 16] }[event.key];
      if (delta) { event.preventDefault(); position = fitBall({ x: position.x + delta[0], y: position.y + delta[1] }, viewport()); place(); save(); }
    });
    shell.querySelector('.assistant-resize-left').addEventListener('keydown', event => {
      if (!['ArrowLeft', 'ArrowRight', 'Home'].includes(event.key)) return;
      event.preventDefault(); rect = event.key === 'Home' ? fit({}, viewport()) : resize(rect, event.key === 'ArrowLeft' ? -24 : 24, 0, 'left', viewport()); place(); save();
    });
    shell.querySelectorAll('[data-assistant-mode]').forEach(tab => {
      tab.addEventListener('click', () => select(tab.dataset.assistantMode));
      tab.addEventListener('keydown', event => { if (['ArrowLeft', 'ArrowRight'].includes(event.key)) { event.preventDefault(); const modes = VISIBLE_MODES; select(modes[(modes.indexOf(mode) + (event.key === 'ArrowRight' ? 1 : modes.length - 1)) % modes.length]); doc.getElementById(`assistant_tab_${mode}`).focus(); } });
    });
    shell.querySelector('[data-assistant-close]').addEventListener('click', close);
    shell.querySelector('[data-assistant-reset]').addEventListener('click', () => { rect = fit({}, viewport()); place(); save(); });
    doc.addEventListener('keydown', event => { if (event.key === 'Escape' && opened && !doc.querySelector('dialog[open],.modal-overlay[style*="display: flex"]')) close(); });
    const openFromButton = (event, button) => {
      if (event._assistantOpened || !button) return;
      event._assistantOpened = true; open(button.dataset.assistantOpen || 'analysis', true);
    };
    // Existing navigation menus stop bubbling after choosing a page.
    doc.querySelectorAll('[data-assistant-open]').forEach(button => button.addEventListener('click', event => openFromButton(event, button)));
    doc.addEventListener('click', event => openFromButton(event, event.target.closest('[data-assistant-open]')));
    root.addEventListener('resize', place);
    const activities = new Map();
    function activity(value, source = 'analysis') {
      activities.set(source, !!value); const busy = [...activities.values()].some(Boolean);
      ball.querySelector('.assistant-ball-activity').hidden = !busy; ball.dataset.busy = String(busy);
    }
    root.Jx3Assistant = { open, close, select, isOpen: () => opened, mode: () => mode, setActivity: activity };
    if (legacy && root.MutationObserver) {
      const observer = new root.MutationObserver(() => activity(!!legacy.querySelector('.ai-thinking,.sim-ai-progress[data-running="true"]'), 'analysis'));
      observer.observe(legacy, { childList: true, subtree: true });
    }
    root.dispatchEvent(new root.CustomEvent('jx3-assistant-ready'));
    if (root.location?.hash === '#page-harness') open('analysis');
  }
  return { fit, fitBall, resize, mount };
});
