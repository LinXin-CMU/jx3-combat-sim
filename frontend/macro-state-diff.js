(function () {
  'use strict';

  const unknown = '未记录';
  const finite = value => typeof value === 'number' && Number.isFinite(value);
  const number = value => finite(value) ? String(value) : unknown;
  const seconds = value => finite(value) ? `${value.toFixed(1)}s` : unknown;
  const name = value => typeof value === 'string' && value.trim() ? value : unknown;
  const object = value => value && typeof value === 'object' && !Array.isArray(value);

  function node(tag, text, className) {
    const element = document.createElement(tag);
    if (text != null) element.textContent = text;
    if (className) element.className = className;
    return element;
  }

  function value(text, side, changed) {
    return node('span', text, `ma-detail-value${changed ? ` ma-detail-${side ? 'after' : 'before'}` : ''}`);
  }

  // Match IDs first, falling back to names only when one side has no ID.
  // One-to-one matching also preserves duplicate entries and one-sided items.
  function align(left, right, idField) {
    const used = new Set();
    const id = item => item?.[idField] != null ? String(item[idField]) : null;
    const pairs = left.map(item => {
      let index = id(item) == null ? -1 : right.findIndex((other, i) => !used.has(i) && id(other) === id(item));
      if (index < 0 && name(item.name) !== unknown) {
        index = right.findIndex((other, i) => !used.has(i)
          && (id(item) == null || id(other) == null) && other.name === item.name);
      }
      if (index >= 0) used.add(index);
      return [item, index >= 0 ? right[index] : null];
    });
    right.forEach((item, index) => { if (!used.has(index)) pairs.push([null, item]); });
    return pairs;
  }

  function render(referenceEvent, actualEvent) {
    const events = [referenceEvent, actualEvent].map(event => object(event) ? event : null);
    const states = events.map(event => object(event?.state_before) ? event.state_before : null);
    const grid = node('div', null, 'ma-detail-grid');

    function row(contents, extraClass = '') {
      const element = node('div', null, `ma-detail-row${extraClass ? ` ${extraClass}` : ''}`);
      contents.forEach(content => {
        const cell = node('div', null, 'ma-detail-cell');
        if (typeof content === 'string') cell.textContent = content;
        else if (content) cell.append(content);
        element.append(cell);
      });
      grid.append(element);
    }

    // Labels and punctuation are plain text. Only each changed value is tinted.
    function fields(label, tokens, texts, present = events.map(Boolean)) {
      row(texts.map((values, side) => {
        if (!present[side]) return '—';
        const fragment = document.createDocumentFragment();
        if (label) fragment.append(node('span', `${label}：`));
        tokens.forEach((token, index) => {
          if (token.before) fragment.append(node('span', token.before));
          fragment.append(value(values[index], side,
            !present[1 - side] || values[index] !== texts[1 - side][index]));
          if (token.after) fragment.append(node('span', token.after));
        });
        return fragment;
      }));
    }

    row(['模板', '实际'], 'ma-detail-header');
    if (events.some(event => !event)) {
      row(events.map(event => event ? '对应释放' : '此侧没有对应释放。'));
    }
    if (events.every(event => !event)) return grid;

    fields('技能', [{}], events.map(event => [name(event?.name)]));
    fields('释放时间', [{}], events.map(event => [seconds(event?.cast_time)]));
    row(events.map(event => event ? '释放前状态' : '—'), 'ma-detail-section');
    if (states.some(state => !state)) {
      fields('', [{}], states.map(state => [state ? '已记录' : '释放前状态未记录']));
    }
    if (states.some(Boolean)) {
      const resources = [['怒气', 'rage'], ['暴怒', 'berserk_value'], ['格挡', 'block_value']]
        .filter(([, key]) => key === 'rage' || states.some(state => finite(state?.[key])));
      fields('', resources.map(([label], index) => ({ before: `${index ? ' / ' : ''}${label} ` })),
        states.map(state => resources.map(([, key]) => number(state?.[key]))));
      if (states.some(state => finite(state?.time))) {
        fields('采样时间', [{}], states.map(state => [seconds(state?.time)]));
      }
      list('自身气劲', 'buffs', 'buff_id', [
        {}, { before: ' ×' }, { before: '（', after: '）' },
      ], buff => [name(buff.name), number(buff.stacks),
        buff.permanent === true || buff.remaining === 0 ? '永久' : seconds(buff.remaining)]);
      list('目标气劲', 'target_buffs', 'buff_id', [
        {}, { before: ' ×' }, { before: '（', after: '）' },
      ], buff => [name(buff.name), number(buff.stacks),
        buff.permanent === true || buff.remaining === 0 ? '永久' : seconds(buff.remaining)]);
      list('技能冷却', 'skill_cds', 'skill_id', [{}, { before: ' ' }],
        skill => [name(skill.name), seconds(skill.remaining)]);
      list('技能充能', 'skill_states', 'skill_id', [{}, { before: ' ' }, { before: '/' }],
        skill => [name(skill.name), skill.charges === null ? '—' : number(skill.charges),
          skill.max_charges === null ? '—' : number(skill.max_charges)],
        pair => pair.some(skill => skill && !(skill.charges === null && skill.max_charges === null)));
    }
    if (events.some(event => finite(event?.macro_page) || finite(event?.macro_line))) {
      fields('宏来源', [{ before: '第 ', after: ' 页' }, { before: '，第 ', after: ' 条语句' }],
        events.map(event => [number(event?.macro_page), number(event?.macro_line)]));
    }
    return grid;

    function list(label, key, idField, tokens, format, include = () => true) {
      const lists = states.map(state => Array.isArray(state?.[key]) ? state[key].filter(object) : null);
      const pairs = align(lists[0] || [], lists[1] || [], idField).filter(include);
      row(events.map(event => event ? label : '—'), 'ma-detail-section');
      if (!pairs.length) {
        fields('', [{}], lists.map(items => [items ? (key === 'skill_states' ? '无充能技能' : '无') : unknown]));
        return;
      }
      for (const pair of pairs) {
        if (pair.every(Boolean)) {
          fields('', tokens, pair.map(format));
          continue;
        }
        // An absent entry in a recorded list means “none”; a missing list or
        // event carries no such evidence. Never invent a zero-duration item.
        row(pair.map((item, side) => {
          if (!events[side]) return '—';
          let text;
          if (item) {
            const parts = format(item);
            text = tokens.map((token, i) => `${token.before || ''}${parts[i]}${token.after || ''}`).join('');
          } else text = lists[side] ? '无' : unknown;
          return value(text, side, true);
        }));
      }
    }
  }

  window.Jx3MacroStateDiff = Object.freeze({ render });
})();
