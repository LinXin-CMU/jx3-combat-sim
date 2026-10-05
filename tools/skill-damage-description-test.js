'use strict';
const assert = require('node:assert/strict');
const ui = require('../frontend/skill-damage-description.js');
const shield = {skill_id: 13044, name: '盾刀·一段', damage_kind: 'physical', base_damage: 13.5,
  base_damage_range: [13, 14], attack_coeff: 0.156379725670206, weapon_coeff: 1};
assert.equal(ui.formula(shield), '造成 13–14（+0.156×最终外功攻击 + 1×武器伤害） 点伤害');
assert.equal(ui.formula({base_damage: 0, attack_coeff: 0, weapon_coeff: 0}), '');
assert.equal(ui.formula({damage_kind: 'surplus_only', surplus_coeff: 6.3}), '造成（6.3×破招）点破招伤害');
assert.equal(ui.formula({true_damage: true, base_damage: 12500}), '造成 12500 点真实伤害');
assert.match(ui.formula({base_damage: 47, attack_coeff: 0.01388817967}), /基础伤害按均值显示/);
assert.match(ui.formula({...shield, damage_kind: 'magical'}), /最终内功攻击/);
const groups = ui.group([shield, {...shield, name: '盾刀·二段'}, {skill_id: 8249, name: '流血·每跳',
  passive: true, base_damage: 47, attack_coeff: 0.01388817967}]);
assert.equal(groups['盾刀'].length, 2);
assert.equal(groups['斩刀'][0].name, '触发流血时·每跳');
const high = ui.render([{...shield, name: '雁门迢递', high_berserk_damage: {
  base_damage_range: [13.8, 14.7], attack_coeff: 1.5749175927497188}}]);
assert.match(high, /首段消耗50暴怒/);
assert.match(high, /首段消耗100暴怒/);
assert.match(high, /13.8–14.7/);
assert.match(high, /1.575×最终外功攻击/);
assert(!ui.render([{...shield, name: '<img src=x onerror=alert(1)>'}]).includes('<img'));
const catalog = new Map([[shield.name, shield]]);
const description = '回复10点怒气，造成{{damage:盾刀·一段}}外功伤害。';
assert.equal(ui.descriptionText(description, catalog), '回复10点怒气，造成13–14（+0.156×最终外功攻击）点外功伤害。');
assert.equal(ui.render([shield], ui.descriptionReferences(description)), '');
assert(!ui.renderDescription('<script>alert(1)</script>', catalog).includes('<script>'));
const talents = [{id: 91002, name: '神威', desc: '盾刀伤害提高112%。', description_skills: [13044, 13045]},
  {id: 91003, name: '威压', desc: '每层威压强化盾猛。', description_skills: [13044, 13045, 13046]}];
assert.deepEqual(ui.activeTalents({id: 13044, name: '盾刀'}, talents, [91002]).map(t => t.id), [91002]);
assert.deepEqual(ui.activeTalents({id: 30769, name: '阵云结晦'}, talents, [91002, 91003]), []);
assert.equal(ui.renderTalents({id: 13044, name: '盾刀'}, talents, [], catalog), '');
assert.match(ui.renderTalents({id: 13044, name: '盾刀'}, talents, [91003], catalog), /【威压】/);
assert.deepEqual(ui.activeTalents({id: 91002, name: '神威'}, talents, [91002]), []);
console.log('PASS: damage ranges, coefficients, weapon terms, high resource tier, all combo ranks, DOT, true damage, surplus, escaping');
