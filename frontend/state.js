// ─────────────────────────────────────────────────────────────────────────────
// state.js — 全局共享状态 + localStorage 持久化骨架
//
// 这是 PR 1 引入的新模块，负责：
//   1. 共享输入（属性/目标/奇穴/秘籍/延迟）的 localStorage 持久化
//   2. 当前工作流（A/B）和步骤记录
//   3. attrsHash —— 用于 Hub 恢复时判断"上次保存时的属性和现在是否一致"
//
// 所有函数纯粹（不直接操作 DOM），DOM 绑定由 app.js 完成。
// ─────────────────────────────────────────────────────────────────────────────

(function () {
  'use strict';

  const LS_KEY_SHARED = 'jx3_shared_inputs';
  const LS_KEY_WORKFLOW = 'jx3_last_workflow';

  // ── 简易 sha1（同步）—— 用于 attrsHash ──
  // 用 Web Crypto subtle.digest 异步太麻烦，这里用快速的 djb2-hash 代替，
  // 够用于"判断两次属性快照是否一致"。
  function quickHash(s) {
    let h = 5381;
    for (let i = 0; i < s.length; i++) {
      h = ((h << 5) + h + s.charCodeAt(i)) | 0;
    }
    return (h >>> 0).toString(16);
  }

  function sortKeys(obj) {
    if (obj == null || typeof obj !== 'object') return obj;
    if (Array.isArray(obj)) return obj.map(sortKeys);
    const out = {};
    for (const k of Object.keys(obj).sort()) out[k] = sortKeys(obj[k]);
    return out;
  }

  // ── 共享输入持久化 ──
  // schema:
  //   { attrs: {...}, target: {...}, talents: [...], recipes: [...], delay: 0 }
  function saveSharedInputs(payload) {
    try {
      localStorage.setItem(LS_KEY_SHARED, JSON.stringify(payload));
    } catch (_) { /* quota */ }
  }

  function loadSharedInputs() {
    try {
      const raw = localStorage.getItem(LS_KEY_SHARED);
      return raw ? JSON.parse(raw) : null;
    } catch (_) { return null; }
  }

  // attrsHash 只算**影响 timeline 合法性**的字段：属性、目标、奇穴、秘籍。
  // 延迟、初始怒气等场景变量**不纳入**（它们是场景 knob）。
  function attrsHash(attrs, target, talents, recipes) {
    const canonical = JSON.stringify(sortKeys({
      attrs: attrs || {},
      target: target || {},
      talents: Array.isArray(talents) ? [...talents].sort((a, b) => a - b) : [],
      recipes: Array.isArray(recipes) ? [...recipes].sort((a, b) => a - b) : [],
    }));
    return quickHash(canonical);
  }

  // ── 工作流/步骤状态 ──
  // schema:
  //   { workflow: "A" | "B", step: 3, completed: [1,2,3],
  //     attrs_hash: "...", payload: {...}, updated: 1712345678 }
  function saveLastStep({ workflow, step, completed, attrsHash: hash, payload }) {
    try {
      const data = {
        workflow,
        step,
        completed: completed || [],
        attrs_hash: hash || '',
        payload: payload || {},
        updated: Date.now(),
      };
      localStorage.setItem(LS_KEY_WORKFLOW, JSON.stringify(data));
      // 同步到 userdata/resume.json（fire-and-forget，失败不影响 UI）
      fetch('/api/resume/save', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(data),
      }).catch(() => {});
    } catch (_) { /* quota */ }
  }

  // 从后端拉取 userdata/resume.json 并写回 localStorage（启动时调用一次）
  async function pullResumeFromServer() {
    try {
      const r = await fetch('/api/resume/load');
      const txt = await r.text();
      if (!txt || txt === 'null') return null;
      const data = JSON.parse(txt);
      if (!data || !data.workflow) return null;
      // 用后端版本覆盖 localStorage（后端是权威）
      localStorage.setItem(LS_KEY_WORKFLOW, JSON.stringify(data));
      return data;
    } catch (_) { return null; }
  }

  function loadLastStep() {
    try {
      const raw = localStorage.getItem(LS_KEY_WORKFLOW);
      return raw ? JSON.parse(raw) : null;
    } catch (_) { return null; }
  }

  function clearLastStep() {
    try { localStorage.removeItem(LS_KEY_WORKFLOW); } catch (_) {}
  }

  // 导出到全局
  window.Jx3State = {
    saveSharedInputs,
    loadSharedInputs,
    attrsHash,
    pullResumeFromServer,
    saveLastStep,
    loadLastStep,
    clearLastStep,
  };
})();
