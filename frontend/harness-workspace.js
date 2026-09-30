/* The only Harness browser write boundary. No model text is executable here. */
(function (root, factory) {
  'use strict';
  const api = factory();
  if (typeof module === 'object' && module.exports) module.exports = api;
  if (root) root.Jx3HarnessWorkspace = api.mount(root);
})(typeof window !== 'undefined' ? window : null, function () {
  'use strict';
  const POSITIONS = ['HAT','JACKET','BELT','WRIST','BOTTOMS','SHOES','NECKLACE','PENDANT','RING_1','RING_2','PRIMARY_WEAPON','SECONDARY_WEAPON'];
  const clone = value => JSON.parse(JSON.stringify(value));
  function stable(value) {
    if (Array.isArray(value)) return '[' + value.map(stable).join(',') + ']';
    if (value && typeof value === 'object') return '{' + Object.keys(value).filter(k => value[k] !== undefined).sort().map(k => JSON.stringify(k) + ':' + stable(value[k])).join(',') + '}';
    return JSON.stringify(value);
  }
  function equipmentSnapshot(config) {
    if (!config) return null;
    const slots = {};
    for (const position of POSITIONS) {
      const source = config.slots?.[position] || {};
      // Hydrated names/quality arrive asynchronously; they are display metadata, not loadout identity.
      slots[position] = { equip_id: source.equip_id || 0, strength: source.strength || 0, embedding: clone(source.embedding || []), enhance_id: source.enhance_id || 0, enchant_id: source.enchant_id || 0 };
    }
    return { slots, stone_id: config.stone_id ?? config.stoneId ?? 0, source_label: '当前配装器' };
  }
  function key(scene) {
    return stable({ version: scene.version, mount: scene.mount, simulation: scene.simulation,
      equipment: scene.equipment && { slots: scene.equipment.slots, stone_id: scene.equipment.stone_id } });
  }
  function macroConfig(text) {
    const pages = { general: '', shield: '', blade: '' }, seen = []; let current = 'general';
    for (const line of String(text || '').replace(/\r\n?/g, '\n').split('\n')) {
      if (line.trim().startsWith('#page')) {
        const name = line.trim().slice(5).trim();
        current = ({shield:'shield',blade:'blade','擎盾':'shield','擎刀':'blade'})[name];
        if (!current || seen.includes(current) || pages.general.trim() || (seen[0] === 'blade' && current === 'shield')) throw new Error('当前编辑器只能应用单通用宏，或盾页→刀页且每体态最多一页；该方案含其他分页结构，请下载完整宏使用。');
        seen.push(current);
      } else pages[current] += (pages[current] ? '\n' : '') + line;
    }
    for (const name of Object.keys(pages)) pages[name] = pages[name].trim();
    return { mode: pages.shield || pages.blade ? 'stance' : 'general', ...pages };
  }
  function toLoop(base, simulation, preserveMacro = false) {
    const sequence = (simulation.sequence || []).map((skill, index) => {
      const entry = skill === '__macro__' ? { type: 'macro' } : skill === '__切体态延迟中__' ? { type: 'wait_stance' }
        : skill === '__战绝回怒__' ? { type: 'wait_zhan_jue' } : { type: 'skill', skill };
      if (simulation.channel_ticks?.[index] != null) entry.channel_ticks = simulation.channel_ticks[index];
      const offset = simulation.timing_offsets?.[index];
      if (offset < 0) entry.offset_max = true; else if (offset > 0) entry.offset = offset;
      if (simulation.qijin_buffs?.[index] != null) entry.qijin_buff = simulation.qijin_buffs[index];
      if (simulation.solidified_casts?.[index]) entry.solidified_cast = clone(simulation.solidified_casts[index]);
      return entry;
    });
    // Import helper counts pre-release DOM items in channel indices; put these last.
    // Its final sort restores their display position while preserving backend indices.
    for (const pre of simulation.pre_releases || []) sequence.push({ type: 'pre_release', skill: pre.skill, pre_time: pre.time_before });
    return { ...clone(base), sequence, macro: preserveMacro ? clone(base.macro) : macroConfig(simulation.macro_text), macro_duration: simulation.macro_duration || 0 };
  }
  function diff(before, after) {
    const rows = [];
    for (const field of new Set([...Object.keys(before.simulation || {}), ...Object.keys(after.simulation || {})])) {
      if (stable(before.simulation?.[field]) !== stable(after.simulation?.[field])) rows.push({ field, before: before.simulation?.[field] ?? null, after: after.simulation?.[field] ?? null });
    }
    for (const pos of POSITIONS) if (stable(before.equipment?.slots?.[pos]) !== stable(after.equipment?.slots?.[pos])) rows.push({ field: 'equipment.' + pos, before: before.equipment?.slots?.[pos] ?? null, after: after.equipment?.slots?.[pos] ?? null });
    if (before.equipment?.stone_id !== after.equipment?.stone_id) rows.push({ field: 'equipment.stone_id', before: before.equipment?.stone_id ?? 0, after: after.equipment?.stone_id ?? 0 });
    return rows;
  }
  function createAttributeStore() {
    const fields = ['gen_gu', 'yuan_qi', 'base_magical_attack']; let saved = null;
    const visible = attrs => Object.fromEntries(Object.entries(attrs).filter(([name]) => !fields.includes(name)));
    return {
      set(values, base, identity) {
        const extra = Object.fromEntries(fields.filter(name => Number.isFinite(values?.[name])).map(name => [name, values[name]]));
        saved = { extra, signature: stable({ base: visible(base), identity }) };
      },
      read(base, identity) {
        if (saved && saved.signature !== stable({ base: visible(base), identity })) saved = null;
        return saved ? { ...base, ...saved.extra } : base;
      }
    };
  }
  function createBridge(deps) {
    let busy = false, undoPoint = null;
    async function capture(options = {}) {
      await deps.ready();
      const snapshot = clone(deps.read());
      if (!options.allowEmpty && !snapshot.simulation?.sequence?.length) throw new Error('请先在循环编辑器放入技能轴或宏。');
      if (!snapshot.simulation.attributes || !snapshot.simulation.target) throw new Error('当前角色属性或目标尚未加载。');
      snapshot.sourceKey = key(snapshot); return snapshot;
    }
    function preview(before, after) {
      if (!after?.simulation?.sequence?.length) throw new Error('方案缺少可应用的完整循环。');
      if (after.simulation.macro_text) macroConfig(after.simulation.macro_text);
      return diff(before, after);
    }
    async function apply(after, expectedKey, transactionId) {
      if (busy) throw new Error('工作区正在更新，请等待本次操作完成。');
      busy = true;
      let before, changed = false;
      try {
        before = await capture();
        if (!expectedKey || before.sourceKey !== expectedKey) throw new Error('当前循环、配装或环境已改变。请用最新工作区重新实验，避免覆盖后续编辑。');
        preview(before, after);
        changed = true;
        await deps.write(clone(after), before);
        const installed = await capture();
        if (installed.version !== before.version || installed.mount !== before.mount) throw new Error('应用期间版本或心法已改变');
        await deps.verify(clone(after), installed);
        undoPoint = { before, installedKey: installed.sourceKey, transactionId };
        return { sourceKey: installed.sourceKey, transactionId };
      } catch (error) {
        if (changed && before) {
          try { await deps.restore(before); }
          catch (rollback) { throw new Error(`${error.message}；恢复原工作区失败：${rollback.message}`); }
        }
        throw error;
      } finally { busy = false; }
    }
    async function undo(transactionId) {
      if (!undoPoint || undoPoint.transactionId !== transactionId) throw new Error('本页没有这次应用的撤销点。');
      if (busy) throw new Error('工作区正在更新。');
      busy = true;
      try {
        const current = await capture();
        if (current.sourceKey !== undoPoint.installedKey) throw new Error('应用后工作区又有编辑，撤销会覆盖这些修改；请先保存后续编辑。');
        await deps.restore(undoPoint.before);
        const restored = await capture();
        if (restored.sourceKey !== undoPoint.before.sourceKey) throw new Error('撤销后的工作区与原快照不一致，已保留撤销记录');
        undoPoint = null;
        return restored;
      } finally { busy = false; }
    }
    return { capture, preview, apply, undo, currentKey: () => key(deps.read()), undoPoint: () => undoPoint && { transactionId: undoPoint.transactionId, installedKey: undoPoint.installedKey } };
  }
  function mount(root) {
    const doc = root.document, supplemental = createAttributeStore();
    const identity = () => ({ version: currentMount.version, mount: currentMount.mount });
    function read() {
      const sequence = [], offsets = {}, qijin = {};
      doc.querySelectorAll('#sim_sequence .sim-seq-item:not(.seq-auto):not(.seq-pre-release)').forEach(el => {
        const index = sequence.length, skill = el.dataset.skill || el.querySelector('.seq-label')?.textContent.trim();
        sequence.push(skill === '__clearCD__' && el.dataset.clearcdTarget ? '__clearCD__:' + el.dataset.clearcdTarget : skill);
        const value = Number(el.dataset.timingOffset);
        if (value < 0) offsets[index] = -1; else if (value > 0.001) offsets[index] = value;
        if (el.dataset.qijinBuff) qijin[index] = Number(el.dataset.qijinBuff);
      });
      const solidified = root.Jx3MacroSolidify?.read(doc.querySelectorAll('#sim_sequence .sim-seq-item:not(.seq-auto)')) || {};
      const pureMacro = sequence.length > 0 && sequence.every((skill, index) => skill === '__macro__' || solidified[index]);
      const simulation = { haste_level: getSimHasteLevel(), sequence, talents: getSelectedTalents(),
        channel_ticks: getSequenceChannelTicks(), timing_offsets: offsets, solidified_casts: solidified, qijin_buffs: qijin,
        network_delay: Number(doc.getElementById('sim_delay').value) || 0, recipes: getSelectedRecipes(),
        ...(sequence.includes('__macro__') ? { macro_text: buildMacroText() } : {}),
        ...(pureMacro && macroLastDuration > 0 ? { macro_duration: macroLastDuration } : {}),
        attributes: getSimAttrs(), target: getTarget(), initial_rage: adminInitialRage,
        boss_attack_interval: getBossAttackInterval(), hanjia_expectation: isHanjiaExpectationEnabled(), ...(typeof getDunyaResetOptions === "function" ? getDunyaResetOptions() : {}),
        tiegu_mode: getTieguMode(), experimental: isExperimental(), equipment: getEquipmentMap(),
        team_buffs: getTeamBuffs(), formation: getCurrentFormation(), pre_releases: getPreReleases() };
      const equipmentConfig = root.Jx3Equip?.getCurrentConfig?.();
      return { simulation, equipment: equipmentSnapshot(equipmentConfig), version: currentMount.version, mount: currentMount.mount,
        workspace: { loop: toLoop(buildLoopConfig(), simulation, true), equipmentConfig, attrs: getAttrs(), hasteOverride: _hasteOverride } };
    }
    function attrs(values, override) {
      _hasteOverride = override;
      for (const [name, value] of Object.entries(values || {})) {
        const input = doc.getElementById(name);
        if (!input || !Number.isFinite(value)) continue;
        input.value = String(value); input.dispatchEvent(new root.Event('input', { bubbles: true }));
      }
      supplemental.set(values, getSimAttrs(), identity());
    }
    async function restore(before) {
      if (before.version !== currentMount.version || before.mount !== currentMount.mount) throw new Error('版本或心法已切换，未把原场景写入新环境');
      applyLoopConfig(before.workspace.loop, { skipSimulate: true });
      if (before.workspace.equipmentConfig) {
        const result = await root.Jx3Equip.applyConfig(before.workspace.equipmentConfig);
        if (!result?.raw) throw new Error('原配装重新计算失败');
      }
      const hidden = Object.fromEntries(['gen_gu','yuan_qi','base_magical_attack'].filter(name => Number.isFinite(before.simulation.attributes?.[name])).map(name => [name,before.simulation.attributes[name]]));
      attrs({ ...before.workspace.attrs, ...hidden }, before.workspace.hasteOverride);
      root.dispatchEvent(new root.CustomEvent('jx3-harness-workspace-changed', { detail: { action: 'restore' } }));
      void runSimulate();
    }
    const bridge = createBridge({
      ready: async () => {
        for (let attempt = 0; attempt < 4; attempt++) {
          const mountReady = currentMountReady, attrsReady = attributesReady;
          await Promise.all([mountReady, attrsReady]);
          if (mountReady === currentMountReady && attrsReady === attributesReady) return;
        }
        throw new Error('版本或属性仍在切换，请稍后重新读取工作区。');
      }, read, restore,
      async write(after, before) {
        // Operators may change rotation or equipment, never silently change the combat environment.
        const allowed = new Set(['sequence','macro_text','macro_duration','channel_ticks','timing_offsets','solidified_casts','qijin_buffs','attributes','haste_level','equipment','lite','lite_keep_timeline']);
        for (const row of diff(before, after)) {
          if (!row.field.startsWith('equipment.') && !allowed.has(row.field) && !equivalent(row.before, row.after)) throw new Error(`该方案修改了工作区暂不支持应用的环境字段：${row.field}`);
        }
        applyLoopConfig(toLoop(before.workspace.loop, after.simulation), { skipSimulate: true });
        if (after.equipment && (stable(after.equipment.slots) !== stable(before.equipment?.slots) || after.equipment.stone_id !== before.equipment?.stone_id)) {
          const result = await root.Jx3Equip.applyConfig({ slots: after.equipment.slots, stoneId: after.equipment.stone_id, stoneName: after.equipment.stone_id === before.equipment?.stone_id ? before.workspace.equipmentConfig?.stoneName || '' : '' });
          if (!result?.raw) throw new Error('配装应用后的属性计算失败');
        }
        if (before.version !== currentMount.version || before.mount !== currentMount.mount) throw new Error('应用期间版本或心法已改变，未继续写入属性');
        if (equivalent(after.simulation.attributes, before.simulation.attributes)) {
          const hidden = Object.fromEntries(['gen_gu','yuan_qi','base_magical_attack'].filter(name => Number.isFinite(after.simulation.attributes?.[name])).map(name => [name,after.simulation.attributes[name]]));
          attrs({ ...before.workspace.attrs, ...hidden }, before.workspace.hasteOverride);
        } else attrs(after.simulation.attributes, null);
      },
      async verify(after, installed) {
        if (!equivalent(installed.simulation, after.simulation)) throw new Error('工作区复核未能完整还原候选，已撤回修改');
        root.dispatchEvent(new root.CustomEvent('jx3-harness-workspace-changed', { detail: { action: 'apply' } }));
        // Use the frozen request to keep the operator's exact fixed simulation window.
        const response = await root.fetch('/api/simulate', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(after.simulation) });
        if (!response.ok) throw new Error('应用后的回放验证失败');
        const serialized = await response.text(), result = JSON.parse(serialized);
        if (!Number.isFinite(result.dps)) throw new Error('应用回放未返回有效 DPS');
        const fingerprint = serialized.match(/"fingerprint"\s*:\s*(?:"(\d+)"|(\d+))/);
        if (after.expected_fingerprint && (fingerprint?.[1] || fingerprint?.[2]) !== String(after.expected_fingerprint)) throw new Error('应用后回放指纹与所选候选不一致');
        root._lastSimBody = clone(after.simulation);
        result._macroAssistBody = clone(after.simulation);
        result._macroAssistKey = root.Jx3MacroAssist?.contextKey();
        lastSimResult = result;
        if (typeof renderTimeline === 'function') renderTimeline(result.timeline || []);
        if (typeof renderBuffTimeline === 'function') renderBuffTimeline(result.buff_timeline || [], parseInt(doc.getElementById('timeline_track')?.style.width) || 100);
        if (typeof renderBuffList === 'function') renderBuffList(result.buffs || []);
        if (typeof renderExpectation === 'function') renderExpectation(result.expectation);
        if (typeof updateSkillButtons === 'function') updateSkillButtons(result.available_skills, result.stance, result.rage, result.skill_cds, result.skill_charges, result.buffs, result.remaining_gcd, result.total_gcd, result.combo_states, result.skill_effective);
        if (typeof updateBlockValue === 'function') updateBlockValue(result.block_value, result.max_block_value);
        if (typeof updateBerserkValue === 'function') updateBerserkValue(result.berserk_value, result.max_berserk_value);
        if (typeof updateRealtimePanel === 'function') updateRealtimePanel(result.initial_stats);
        if (typeof decorateSeqItems === 'function') decorateSeqItems(result);
        if (typeof onSimulateComplete === 'function') onSimulateComplete(result);
        for (const [id, value] of [['sim_dps_value', Math.round(result.dps).toLocaleString('zh-CN')], ['sim_fight_time', `${Number(result.fight_time || 0).toFixed(2)}s`], ['sim_total_damage', Math.round(result.total_damage || 0).toLocaleString('zh-CN')]]) { const el = doc.getElementById(id); if (el) el.textContent = value; }
      }
    });
    return { ...bridge, equivalent, equipmentSnapshot, POSITIONS, completeAttributes: base => supplemental.read(base, identity()) };
  }
  function equivalent(left, right) {
    function normalize(value) {
      if (value == null || value === false || (typeof value === 'object' && Object.keys(value).length === 0)) return null;
      if (Array.isArray(value)) return value.map(normalize);
      if (value && typeof value === 'object') {
        const output = {};
        for (const [name, entry] of Object.entries(value)) {
          if (['lite', 'lite_keep_timeline', 'base_magical_attack'].includes(name) && !entry) continue;
          if (['damage_cof','defense_bonus'].includes(name) && entry === 0) continue;
          if (['gen_gu','yuan_qi'].includes(name) && entry === 44) continue;
          if (entry == null || entry === false || (typeof entry === 'object' && Object.keys(entry).length === 0)) continue;
          if (name === 'macro_text' && typeof entry === 'string') { output[name] = entry.replace(/#page\s+擎盾/g, '#page shield').replace(/#page\s+擎刀/g, '#page blade').replace(/\r\n?/g, '\n').trim(); continue; }
          output[name] = normalize(entry);
        }
        return output;
      }
      return value;
    }
    return stable(normalize(left)) === stable(normalize(right));
  }
  return { stable, key, equivalent, equipmentSnapshot, macroConfig, toLoop, diff, createAttributeStore, createBridge, mount, POSITIONS };
});
