/* Skill descriptions are formatted from the same parameters used by the backend. */
(function (root) {
  'use strict';
  const escape = value => String(value).replace(/[&<>"']/g, ch =>
    ({'&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;'}[ch]));
  const number = value => Number(value || 0).toFixed(3).replace(/\.?0+$/, '') || '0';
  const baseName = spec => spec.skill_id >= 90010 && spec.skill_id <= 90012
    ? spec.name : spec.name.split('·')[0];

  function formula(spec) {
    if (spec.damage_kind === 'surplus_only') {
      return spec.surplus_coeff > 0 ? `造成（${number(spec.surplus_coeff)}×破招）点破招伤害` : '';
    }
    const range = spec.base_damage_range;
    const base = range ? (range[0] === range[1] ? number(range[0]) : `${number(range[0])}–${number(range[1])}`)
      : number(spec.base_damage);
    if (!(spec.base_damage > 0 || range?.[1] > 0 || spec.attack_coeff > 0 || spec.weapon_coeff > 0)) return '';
    if (spec.true_damage) return `造成 ${base} 点真实伤害`;
    const terms = [];
    if (spec.attack_coeff) terms.push(`${number(spec.attack_coeff)}×最终${spec.damage_kind === 'magical' ? '内功' : '外功'}攻击`);
    if (spec.weapon_coeff) terms.push(`${number(spec.weapon_coeff)}×武器伤害`);
    const average = !range && spec.base_damage > 0 ? '（基础伤害按均值显示）' : '';
    return `造成 ${base}${terms.length ? `（+${terms.join(' + ')}）` : ''} 点伤害${average}`;
  }

  function group(specs) {
    const groups = Object.create(null);
    for (const spec of specs) {
      const name = baseName(spec);
      (groups[name] ||= []).push(spec);
      if (spec.skill_id === 8249) (groups['斩刀'] ||= []).push({...spec, name: '触发流血时·每跳'});
    }
    return groups;
  }

  function descriptionReferences(text = '') {
    return new Set([...text.matchAll(/\{\{damage:([^}]+)\}\}/g)].map(match => match[1]));
  }

  function descriptionText(text = '', specs = new Map()) {
    return text.replace(/\{\{damage:([^}]+)\}\}/g, (_, name) => {
      const spec = specs.get(name);
      if (!spec) return '';
      const range = spec.base_damage_range;
      const base = range ? (range[0] === range[1] ? number(range[0]) : `${number(range[0])}–${number(range[1])}`)
        : number(spec.base_damage);
      const attack = spec.attack_coeff ? `（+${number(spec.attack_coeff)}×最终${spec.damage_kind === 'magical' ? '内功' : '外功'}攻击）` : '';
      return `${base}${attack}点`;
    });
  }

  function renderDescription(text = '', specs = new Map()) {
    return escape(descriptionText(text, specs)).replace(/\r?\n/g, '<br>');
  }

  function activeTalents(info, talents = [], selected = []) {
    const ids = new Set(selected.map(Number));
    return talents.filter(talent => ids.has(Number(talent.id)) && Number(talent.id) !== Number(info.id)
      && (Array.isArray(talent.description_skills) ? talent.description_skills.includes(Number(info.id))
        : (talent.desc || '').includes(`“${info.name}”`)));
  }

  function renderTalents(info, talents, selected, specs) {
    const active = activeTalents(info, talents, selected);
    if (!active.length) return '';
    return '<div class="skill-active-talents">' + active.map(talent =>
      `<div class="skill-active-talent"><b>【${escape(talent.name)}】</b><div>${renderDescription(talent.desc, specs)}</div></div>`
    ).join('') + '</div>';
  }

  function render(specs = [], described = new Set()) {
    const rows = [];
    for (const spec of specs) {
      const text = formula(spec);
      if (!text) continue;
      const high = spec.high_berserk_damage;
      if (!described.has(spec.name)) rows.push([spec.name + (high ? '·首段消耗50暴怒' : ''), text]);
      if (high) rows.push([spec.name + '·首段消耗100暴怒', formula({...spec, ...high})]);
    }
    if (!rows.length) return '';
    return '<div class="skill-damage-description">' + rows.map(([name, text]) =>
      `<div class="skill-damage-row"><span class="skill-damage-label">${escape(name)}</span><span>${escape(text)}</span></div>`
    ).join('') + '</div>';
  }

  const api = {formula, group, render, descriptionReferences, descriptionText, renderDescription, activeTalents, renderTalents};
  if (typeof module === 'object' && module.exports) module.exports = api;
  else root.Jx3SkillDamageDescription = api;
})(typeof window === 'object' ? window : globalThis);
