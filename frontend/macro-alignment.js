/* 宏实际循环与模板的主动技能对齐。只描述差异，不推断释放原因。 */
(function (root, factory) {
  'use strict';
  const api = factory();
  if (typeof module === 'object' && module.exports) module.exports = api;
  if (root) root.Jx3MacroAlignment = api;
})(typeof window !== 'undefined' ? window : null, function () {
  'use strict';

  const MAX_ACTIVE_EVENTS = 2048;
  const DEFAULT_TIME_TOLERANCE = 1 / 16;
  const RESOURCE_FIELDS = ['rage', 'berserk_value', 'max_berserk_value', 'block_value'];
  const EPSILON = 1e-9;
  const MISSING = 1, EXTRA = 2, MATCH = 3;

  function activeEvents(timeline, label) {
    if (!Array.isArray(timeline)) throw new TypeError(`${label}时间轴必须是数组。`);
    const active = [];
    for (let index = 0; index < timeline.length; index++) {
      const event = timeline[index];
      if (!event || event.triggered || event.success === false || event.cast_success === false || event.failed === true) continue;
      const name = typeof event.name === 'string' ? event.name.trim() : '';
      if (!name || name.startsWith('__') || name.startsWith('移除气劲') || name === '清除冷却' || event.skill_id === 90001) continue;
      active.push({ event, index, key: skillKey(name, event.skill_id) });
      if (active.length > MAX_ACTIVE_EVENTS) {
        const error = new RangeError(`${label}超过 ${MAX_ACTIVE_EVENTS} 个主动技能，请缩短对照时长后重试。`);
        error.code = 'MACRO_ALIGNMENT_LIMIT';
        throw error;
      }
    }
    return active;
  }

  function skillKey(name, id) {
    const base = name.split('·')[0];
    // 雾海是独立技能，不是等级/怒气档位；连招各段也保持各自身份。
    if ((id >= 90010 && id <= 90012) || /^(阵云结晦|月照连营|雁门迢递)·雾海(?:·|$)/.test(name)) {
      return `${base}·雾海`;
    }
    return base;
  }

  function timeDelta(reference, actual) {
    return Number.isFinite(reference.cast_time) && Number.isFinite(actual.cast_time)
      ? actual.cast_time - reference.cast_time : null;
  }

  function matchRow(reference, actual, tolerance) {
    const a = reference.event, b = actual.event;
    const delta = timeDelta(a, b);
    const resourceDiffs = [];
    for (const field of RESOURCE_FIELDS) {
      const before = a.state_before?.[field], after = b.state_before?.[field];
      // 未采集/不适用的状态不能解释成 0，也不能据此判定资源变化。
      if (Number.isFinite(before) && Number.isFinite(after) && before !== after) {
        resourceDiffs.push({ field, reference: before, actual: after });
      }
    }
    const variantChanged = a.name !== b.name
      || (Number.isFinite(a.skill_id) && Number.isFinite(b.skill_id) && a.skill_id !== b.skill_id)
      || (Number.isFinite(a.channel_ticks) && Number.isFinite(b.channel_ticks) && a.channel_ticks !== b.channel_ticks);
    return {
      referenceIndex: reference.index,
      actualIndex: actual.index,
      kind: variantChanged || resourceDiffs.length || (delta !== null && Math.abs(delta) > tolerance + EPSILON) ? 'changed' : 'same',
      timeDelta: delta,
      resourceDiffs,
    };
  }

  /**
   * 对齐完整 timeline 中成功释放的主动技能，返回索引仍指向原数组。
   * options.timeTolerance 单位为秒；默认一帧。summary.firstDifference 是 rows 索引或 null。
   * changed 包括释放时间、资源、等级/怒气档位和引导跳数差异，不表示宏有错误。
   * 超过任意一侧 2048 个主动技能会抛出 code=MACRO_ALIGNMENT_LIMIT 的 RangeError。
   *
   * 最长公共子序列优先最大化同技能配对数；仅在同分路径间比较总时间距离。
   * 不同技能用 missing/extra 表示，避免一次插入/删除让后续循环整段错位。
   * O(n*m) 时间，O(n*m) 字节回溯矩阵与 O(m) 分数空间（上限约 4.3 MiB）。
   */
  function align(referenceTimeline, actualTimeline, options = {}) {
    const tolerance = options.timeTolerance ?? DEFAULT_TIME_TOLERANCE;
    if (!Number.isFinite(tolerance) || tolerance < 0) throw new RangeError('时间容差必须是非负有限秒数。');
    const reference = activeEvents(referenceTimeline, '模板');
    const actual = activeEvents(actualTimeline, '实际循环');
    const n = reference.length, m = actual.length, stride = m + 1;
    const directions = new Uint8Array((n + 1) * stride);
    let nextCounts = new Uint16Array(stride), counts = new Uint16Array(stride);
    let nextCosts = new Float64Array(stride), costs = new Float64Array(stride);

    for (let j = 0; j < m; j++) directions[n * stride + j] = EXTRA;
    for (let i = n - 1; i >= 0; i--) {
      directions[i * stride + m] = MISSING;
      counts[m] = 0; costs[m] = 0;
      for (let j = m - 1; j >= 0; j--) {
        let count = nextCounts[j], cost = nextCosts[j], direction = MISSING;
        if (counts[j + 1] > count || (counts[j + 1] === count && costs[j + 1] < cost - EPSILON)) {
          count = counts[j + 1]; cost = costs[j + 1]; direction = EXTRA;
        }
        if (reference[i].key === actual[j].key) {
          const matchCount = nextCounts[j + 1] + 1;
          const delta = timeDelta(reference[i].event, actual[j].event);
          // 有限封顶只用于同分择优，避免异常时刻使代价溢出；返回的时间差不截断。
          const matchCost = nextCosts[j + 1] + (delta === null ? 0 : Math.min(Math.abs(delta), 1e6));
          // 完全同分时优先当前同技能，重复循环在无时间数据时也能稳定对齐。
          if (matchCount > count || (matchCount === count && matchCost <= cost + EPSILON)) {
            count = matchCount; cost = matchCost; direction = MATCH;
          }
        }
        counts[j] = count; costs[j] = cost;
        directions[i * stride + j] = direction;
      }
      [counts, nextCounts] = [nextCounts, counts];
      [costs, nextCosts] = [nextCosts, costs];
    }

    const rows = [], summary = { missing: 0, extra: 0, changed: 0, firstDifference: null };
    let i = 0, j = 0;
    while (i < n || j < m) {
      const direction = directions[i * stride + j];
      let row;
      if (direction === MATCH) {
        row = matchRow(reference[i++], actual[j++], tolerance);
      } else if (direction === MISSING) {
        row = { referenceIndex: reference[i++].index, actualIndex: null, kind: 'missing', timeDelta: null, resourceDiffs: [] };
      } else {
        row = { referenceIndex: null, actualIndex: actual[j++].index, kind: 'extra', timeDelta: null, resourceDiffs: [] };
      }
      if (row.kind !== 'same') {
        summary[row.kind]++;
        if (summary.firstDifference === null) summary.firstDifference = rows.length;
      }
      rows.push(row);
    }
    return { rows, summary };
  }

  return Object.freeze({ align, MAX_ACTIVE_EVENTS, DEFAULT_TIME_TOLERANCE });
});
