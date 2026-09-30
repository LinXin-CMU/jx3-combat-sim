(function (root) {
  'use strict';
  const active = result => (result?.timeline || []).filter(e => !e.triggered);
  function canonical(value, key) {
    if (Array.isArray(value)) {
      const list = value.map(v => canonical(v));
      return key === 'skill_cds' ? list.sort((a, b) => a.name.localeCompare(b.name)) : list;
    }
    if (value && typeof value === 'object') return Object.fromEntries(
      Object.keys(value).sort().map(k => [k, canonical(value[k], k)]));
    return value;
  }
  function read(items) {
    const result = {}; let index = 0;
    for (const item of items) {
      if (item.classList.contains('seq-pre-release') || item.classList.contains('seq-auto')) continue;
      if (item.dataset.solidifiedCast && !(Number(item.dataset.timingOffset) || 0)) {
        try { result[index] = JSON.parse(item.dataset.solidifiedCast); } catch {}
      }
      index++;
    }
    return result;
  }
  function prepare(source, original) {
    const body = JSON.parse(JSON.stringify(original));
    body.solidified_casts ||= {};
    const changes = [];
    for (const event of active(source)) {
      const index = event.sequence_index;
      if (body.sequence[index] !== '__macro__') continue;
      if (!event.solidify) throw new Error('模拟结果缺少固化等待记录，请更新后端');
      const name = event.name.split('·')[0];
      body.sequence[index] = name;
      body.solidified_casts[index] = event.solidify;
      delete body.timing_offsets?.[index];
      delete body.channel_ticks?.[index];
      changes.push({ index, name, frozen: event.solidify });
    }
    if (!changes.length) throw new Error('没有可固化的成功施放');
    return { body, changes, unready: body.sequence.filter(s => s === '__macro__').length };
  }
  function compare(source, candidate) {
    const a = active(source), b = active(candidate);
    if (a.length !== b.length) return `施放数量不同：${a.length} → ${b.length}`;
    for (let i = 0; i < a.length; i++) {
      const x = a[i], y = b[i];
      if (x.skill_id !== y.skill_id || x.name !== y.name) return `第 ${i + 1} 次技能不同`;
      if (Math.abs(x.cast_time - y.cast_time) > 1e-6) return `第 ${i + 1} 次时间不同`;
      for (const key of ['channel_ticks', 'rage_after', 'damage_total']) {
        if (x[key] !== y[key]) return `第 ${i + 1} 次${key}不同`;
      }
      for (const key of ['state_before', 'state_after']) {
        if (JSON.stringify(canonical(x[key])) !== JSON.stringify(canonical(y[key]))) return `第 ${i + 1} 次状态不同`;
      }
    }
    for (const key of ['fight_time', 'total_damage', 'rage', 'stance']) {
      if (source[key] !== candidate[key]) return `结束时${key}不同`;
    }
    return null;
  }
  const api = { read, prepare, compare };
  root.Jx3MacroSolidify = api;
  if (typeof module !== 'undefined') module.exports = api;
})(typeof window === 'undefined' ? globalThis : window);
