/* 写宏四个区域的显示比例；不改变技能输入、宏草稿或模拟环境。 */
(function () {
  'use strict';
  const panel = document.getElementById('panel_manual');
  const workspace = document.getElementById('macro_workspace');
  const heading = document.getElementById('macro_compare_heading');
  if (!panel || !workspace || !heading) return;
  const STORAGE_KEY = 'jx3_macro_layout_v2';
  const defaults = { topWidth: .5, bottomWidth: .46, topHeight: .46, editorWidth: .5 };
  const values = { ...defaults };
  try {
    const saved = JSON.parse(localStorage.getItem(STORAGE_KEY) || '{}');
    for (const key of Object.keys(defaults)) {
      if (Number.isFinite(saved?.[key]) && saved[key] >= .1 && saved[key] <= .9) values[key] = saved[key];
    }
  } catch { /* 不可用的本机设置使用默认比例。 */ }
  const specs = [
    { id: 'macro_split_top_width', key: 'topWidth', axis: 'x', host: workspace.querySelector('.ma-top-row'), before: '--ma-top-left', after: '--ma-top-right' },
    { id: 'macro_split_bottom_width', key: 'bottomWidth', axis: 'x', host: workspace.querySelector('.ma-bottom-row'), before: '--ma-bottom-left', after: '--ma-bottom-right' },
    { id: 'macro_split_height', key: 'topHeight', axis: 'y', host: workspace, before: '--ma-top-height', after: '--ma-bottom-height' },
    { id: 'macro_split_editor_width', key: 'editorWidth', axis: 'x', host: workspace.querySelector('.ma-draft-columns'), before: '--ma-editor-left', after: '--ma-editor-right', minBefore: 110, minAfter: 110 },
  ].map(spec => ({ ...spec, handle: document.getElementById(spec.id) }));
  let active = false, drag = null, resizeFrame = null;
  const clamp = (value, min, max) => Math.max(min, Math.min(max, value));
  const notify = () => window.dispatchEvent(new Event('jx3-macro-layout-change'));
  function limits(spec) {
    const bounds = spec.host.getBoundingClientRect();
    const size = Math.max(1, (spec.axis === 'x' ? bounds.width : bounds.height) - 8);
    const minimum = Math.min(spec.minBefore ?? (spec.axis === 'x' ? 200 : 110), size * .4);
    const remaining = Math.min(spec.minAfter ?? (spec.axis === 'x' ? 200 : 140), size * .4);
    return { size, min: Math.max(.1, minimum / size), max: Math.min(.9, 1 - remaining / size) };
  }
  function apply() {
    if (!active) return;
    for (const spec of specs) {
      const bounds = limits(spec), ratio = clamp(values[spec.key], bounds.min, bounds.max);
      panel.style.setProperty(spec.before, `${ratio}fr`);
      panel.style.setProperty(spec.after, `${1 - ratio}fr`);
      spec.handle.setAttribute('aria-valuemin', String(Math.round(bounds.min * 100)));
      spec.handle.setAttribute('aria-valuemax', String(Math.round(bounds.max * 100)));
      spec.handle.setAttribute('aria-valuenow', String(Math.round(ratio * 100)));
      spec.handle.setAttribute('aria-valuetext', `${spec.axis === 'x' ? '左侧宽度' : '上方高度'} ${Math.round(ratio * 100)}%`);
    }
  }
  function save() {
    try { localStorage.setItem(STORAGE_KEY, JSON.stringify(values)); } catch { /* 保留当前页面比例。 */ }
  }
  function finish(event) {
    if (!drag || (event?.pointerId != null && event.pointerId !== drag.pointerId)) return;
    const previous = drag; drag = null;
    previous.spec.handle.classList.remove('is-dragging');
    document.body.classList.remove('ma-resizing-x', 'ma-resizing-y');
    try { previous.spec.handle.releasePointerCapture(previous.pointerId); } catch { /* 已释放。 */ }
    save(); notify();
  }
  for (const spec of specs) {
    spec.handle.addEventListener('pointerdown', event => {
      if (!active || event.button !== 0 || drag) return;
      event.preventDefault();
      const bounds = limits(spec);
      drag = { spec, pointerId: event.pointerId, start: spec.axis === 'x' ? event.clientX : event.clientY,
        ratio: clamp(values[spec.key], bounds.min, bounds.max), ...bounds };
      spec.handle.classList.add('is-dragging');
      document.body.classList.add(`ma-resizing-${spec.axis}`);
      try { spec.handle.setPointerCapture(event.pointerId); } catch { /* window 监听仍可结束拖动。 */ }
    });
    spec.handle.addEventListener('lostpointercapture', finish);
    spec.handle.addEventListener('dblclick', () => {
      if (!active) return;
      values[spec.key] = defaults[spec.key]; apply(); save(); notify();
    });
    spec.handle.addEventListener('keydown', event => {
      if (!active) return;
      const decrement = spec.axis === 'x' ? 'ArrowLeft' : 'ArrowUp';
      const increment = spec.axis === 'x' ? 'ArrowRight' : 'ArrowDown';
      if (![decrement, increment, 'Home', 'End'].includes(event.key)) return;
      event.preventDefault();
      const bounds = limits(spec), ratio = clamp(values[spec.key], bounds.min, bounds.max);
      values[spec.key] = event.key === 'Home' ? bounds.min : event.key === 'End' ? bounds.max
        : clamp(ratio + (event.key === increment ? 1 : -1) * (event.shiftKey ? 48 : 16) / bounds.size, bounds.min, bounds.max);
      apply(); save(); notify();
    });
  }
  window.addEventListener('pointermove', event => {
    if (!drag || event.pointerId !== drag.pointerId) return;
    const point = drag.spec.axis === 'x' ? event.clientX : event.clientY;
    values[drag.spec.key] = clamp(drag.ratio + (point - drag.start) / drag.size, drag.min, drag.max);
    apply();
  });
  window.addEventListener('pointerup', finish);
  window.addEventListener('pointercancel', finish);
  window.addEventListener('blur', () => finish());
  window.addEventListener('jx3-macro-assist-mode', event => {
    finish(); active = !!event.detail?.active; heading.hidden = !active;
    if (active) { apply(); notify(); }
  });
  function resized() {
    if (!active || resizeFrame != null) return;
    resizeFrame = requestAnimationFrame(() => { resizeFrame = null; apply(); if (!drag) notify(); });
  }
  if (window.ResizeObserver) new ResizeObserver(resized).observe(workspace);
  else window.addEventListener('resize', resized);
  function tab(name) {
    if (name === 'diagnosis') return; // 右侧诊断常驻，不切换左侧正在查看的条件。
    document.getElementById('macro_editor_panel').dataset.tab = name;
    for (const key of ['draft','conditions']) document.getElementById(`macro_review_${key}`).setAttribute('aria-selected',String(key===name));
    apply();
  }
  for (const name of ['draft','conditions']) {
    const button = document.getElementById(`macro_review_${name}`);
    button.onclick = () => tab(name);
    button.onkeydown = event => {
      if (!['ArrowLeft','ArrowRight'].includes(event.key)) return;
      event.preventDefault(); const other = name === 'draft' ? 'conditions' : 'draft';
      tab(other); document.getElementById(`macro_review_${other}`).focus();
    };
  }
  window.Jx3MacroLayout = Object.freeze({ tab, reset() {
    finish(); Object.assign(values, defaults); apply(); save(); notify();
  } });
})();
