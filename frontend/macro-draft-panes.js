/* 可见两栏保留原文顺序；标准姿态页头存为隐藏区间，完整宏始终由 canonical 保存。 */
(function () {
  'use strict';
  const isBlade = stance => ['blade', '擎刀'].includes(stance);
  const isShield = stance => ['shield', '擎盾'].includes(stance);
  const meaningful = text => text.split('\n').map(line => line.trim()).find(line => line && !line.startsWith('//'));
  const hasCommands = text => text.split('\n').some(line => {
    const value = line.trim(), prefix = value.startsWith('/fcast') ? '/fcast' : value.startsWith('/cast') ? '/cast' : '';
    return prefix && value.slice(prefix.length).trim();
  });
  function headers(text) {
    let offset = 0, first = true;
    const found = [];
    for (const line of text.split('\n')) {
      const value = line.trim(), end = Math.min(text.length, offset + line.length + 1);
      if (value.startsWith('#page')) found.push({
        start: offset, end, raw: text.slice(offset, end), stance: value.slice(5).trim(), leading: first,
      });
      if (value && !value.startsWith('//')) first = false;
      offset += line.length + 1;
    }
    return found;
  }
  function split(text) {
    // 与后端一致，#pageblade / #page擎刀 也合法；大小写保持敏感。
    const boundary = headers(text).find(header => isBlade(header.stance));
    return boundary ? { left: text.slice(0, boundary.start), right: text.slice(boundary.start) } : { left: text, right: '' };
  }
  function compose(left, right) {
    const first = meaningful(right);
    const header = first && !first.startsWith('#page') ? '#page blade\n' : '';
    const separator = left && right && !left.endsWith('\n') ? '\n' : '';
    return { text: left + separator + header + right, rightOffset: left.length + separator.length + header.length,
      autoBlade: !!header };
  }
  function create(canonical, left, right) {
    if (!canonical || !left || !right) return null;
    const inputs = [left, right], states = inputs.map(input => ({ input, header: null, previous: '' }));
    let lastPane = left, emitting = false, hadRightCommands = false, addedShield = false, complex = false;
    const clamp = (value, limit) => Math.max(0, Math.min(limit, value));
    const currentPane = () => inputs.includes(document.activeElement) ? document.activeElement : lastPane;
    const stateFor = pane => states[inputs.indexOf(pane)];
    function hiddenText(state) {
      if (!state.header) return '';
      const header = state.header.raw;
      return header + (!header.endsWith('\n') && state.input.value.slice(state.header.start) ? '\n' : '');
    }
    function raw(state) {
      const text = state.input.value;
      return state.header ? text.slice(0, state.header.start) + hiddenText(state) + text.slice(state.header.start) : text;
    }
    function toRaw(state, offset) {
      return offset + (state.header && offset >= state.header.start ? hiddenText(state).length : 0);
    }
    function toVisible(state, offset) {
      if (!state.header || offset <= state.header.start) return clamp(offset, state.input.value.length);
      return clamp(Math.max(state.header.start, offset - hiddenText(state).length), state.input.value.length);
    }
    const joined = () => compose(raw(states[0]), raw(states[1]));
    function updateSelection(pane = currentPane()) {
      lastPane = pane;
      const state = stateFor(pane), base = pane === right ? joined().rightOffset : 0;
      canonical.setSelectionRange(base + toRaw(state, pane.selectionStart), base + toRaw(state, pane.selectionEnd), pane.selectionDirection);
    }
    function project(rawLeft, rawRight) {
      const parts = [rawLeft, rawRight], lists = parts.map(headers);
      // 多页、通用页或头前已有命令：保留全部原文，避免隐藏后混淆分页语义。
      complex = lists.some((list, index) => list.length > 1 || list.some(header =>
        !header.leading || !(index ? isBlade(header.stance) : isShield(header.stance))));
      states.forEach((state, index) => {
        const text = parts[index], header = !complex && lists[index][0];
        state.header = header ? { start: header.start, raw: header.raw } : null;
        const visible = header ? text.slice(0, header.start) + text.slice(header.end) : text;
        if (state.input.value !== visible) state.input.value = visible;
        state.previous = state.input.value;
      });
    }
    function setText(text) {
      canonical.value = text;
      // textarea 统一 CRLF 后再计算所有 UI offset，不能用传入 raw text 的坐标。
      const parts = split(canonical.value);
      project(parts.left, parts.right);
      hadRightCommands = hasCommands(parts.right); addedShield = false;
      updateSelection();
    }
    function moveHeaderAnchor(state) {
      if (!state.header || state.previous === state.input.value) return;
      const before = state.previous, after = state.input.value;
      let start = 0, oldEnd = before.length, newEnd = after.length;
      while (start < oldEnd && start < newEnd && before[start] === after[start]) start++;
      while (oldEnd > start && newEnd > start && before[oldEnd - 1] === after[newEnd - 1]) { oldEnd--; newEnd--; }
      const anchor = state.header.start;
      if (oldEnd <= anchor && start < anchor) state.header.start += newEnd - oldEnd;
      else if (start < anchor) state.header.start = start;
      state.header.start = clamp(state.header.start, after.length);
      // 删除头前换行可能把锚点带入注释/命令中；隐藏页头必须始终从物理行首插入。
      if (state.header.start > 0 && after[state.header.start - 1] !== '\n') {
        state.header.start = after.lastIndexOf('\n', state.header.start - 1) + 1;
      }
      // 在头前注释中新增命令时，命令仍属于这个可见姿态栏。
      let offset = 0;
      for (const line of after.split('\n')) {
        const value = line.trim();
        if (value && !value.startsWith('//')) { state.header.start = Math.min(state.header.start, offset); break; }
        offset += line.length + 1;
      }
    }
    function sync(source) {
      const state = stateFor(source);
      moveHeaderAnchor(state);
      const selection = [toRaw(state, source.selectionStart), toRaw(state, source.selectionEnd)], direction = source.selectionDirection;
      let rawLeft = raw(states[0]), rawRight = raw(states[1]);
      const rightCommands = hasCommands(rawRight), first = meaningful(rawRight);
      const bladePage = first && (!first.startsWith('#page') || isBlade(first.slice(5).trim()));
      if (source === right && !hadRightCommands && rightCommands && bladePage && !headers(rawLeft).length) {
        // 新写刀宏进入双姿态编辑；空左栏也保留盾姿态，避免以后写入通用页遮蔽刀页。
        rawLeft = '#page shield\n' + rawLeft; addedShield = true;
      }
      hadRightCommands = rightCommands;
      canonical.value = compose(rawLeft, rawRight).text;
      project(rawLeft, rawRight);
      const start = toVisible(state, selection[0]), end = toVisible(state, selection[1]);
      if (source.selectionStart !== start || source.selectionEnd !== end || source.selectionDirection !== direction) {
        source.setSelectionRange(start, end, direction);
      }
      updateSelection(source);
      emitting = true;
      try { canonical.dispatchEvent(new Event('input', { bubbles: true })); }
      finally { emitting = false; }
    }
    function location(start, end = start) {
      const value = joined(), rawRight = raw(states[1]);
      const pane = rawRight && start >= raw(states[0]).length ? right : left;
      const state = stateFor(pane), base = pane === right ? value.rightOffset : 0;
      const localStart = toVisible(state, start - base), localEnd = toVisible(state, end - base);
      return { pane, start: localStart, end: localEnd, line: pane.value.slice(0, localStart).split('\n').length,
        label: pane === right ? '刀宏' : '盾宏/通用宏' };
    }
    function selectRange(start, end = start, focus = true) {
      const target = location(start, end);
      lastPane = target.pane;
      target.pane.setSelectionRange(target.start, target.end);
      updateSelection(target.pane);
      if (focus) target.pane.focus();
    }
    function insert(text) {
      const pane = currentPane(), start = pane.selectionStart, end = pane.selectionEnd;
      const before = pane.value.slice(0, start), after = pane.value.slice(end);
      const prefix = before && !before.endsWith('\n') ? '\n' : '';
      const suffix = after && !after.startsWith('\n') ? '\n' : '';
      pane.setRangeText(prefix + text.trim() + suffix, start, end, 'end');
      lastPane = pane; sync(pane); pane.focus(); return true;
    }
    function description() {
      const value = joined(), pages = [];
      let commands = false, stance = null;
      for (const line of value.text.split('\n')) {
        const trimmed = line.trim();
        if (!trimmed || trimmed.startsWith('//')) continue;
        if (trimmed.startsWith('#page')) {
          if (commands) pages.push(stance);
          commands = false; stance = trimmed.slice(5).trim() || null;
        } else commands = true;
      }
      if (commands) pages.push(stance);
      if (pages.some((page, index) => page == null && index < pages.length - 1)) {
        return '通用页优先匹配，后续宏页不会执行；请在左栏添加 #page shield 或检查原文分页';
      }
      if (complex || pages.length > 2) return '复杂分页按原文顺序显示，保留所有页头；请按原文检查姿态与优先级';
      if (addedShield) return '已按盾、刀姿态分页，页头自动保留；本次双栏修改可一起撤销';
      if (states.some(state => state.header) || value.autoBlade) return '姿态分页已保留，编辑区仅显示宏正文';
      return '';
    }
    for (const pane of inputs) {
      pane.addEventListener('input', () => sync(pane));
      for (const type of ['focus', 'click', 'select', 'keyup']) pane.addEventListener(type, () => updateSelection(pane));
    }
    canonical.addEventListener('input', () => { if (!emitting) setText(canonical.value); });
    setText(canonical.value);
    return { inputs, setText, selectRange, location, insert, description,
      focus: () => { const pane = currentPane(); pane.focus(); updateSelection(pane); } };
  }
  window.Jx3MacroDraftPanes = Object.freeze({ create, split, compose });
})();
