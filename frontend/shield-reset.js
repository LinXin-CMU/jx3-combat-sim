// Reuse the expectation toggle; keep one seed across a template/macro comparison.
function getDunyaResetOptions(context) {
  if (context && Object.hasOwn(context, 'dunya_reset_seed')) {
    return { dunya_reset_seed: Number(context.dunya_reset_seed) >>> 0 };
  }
  if (typeof isCangShengFenShan !== 'function' || !isCangShengFenShan()) return {};
  let seed = 0;
  try { seed = Number(localStorage.getItem('dunya_reset_seed')) >>> 0; } catch {}
  return { dunya_reset_seed: seed };
}

function initDunyaResetSettings(container) {
  const enabled = typeof isCangShengFenShan === 'function' && isCangShengFenShan();
  const seed = getDunyaResetOptions().dunya_reset_seed || 0;
  container.innerHTML = `
    <label class="settings-label" for="dunya_reset_seed">盾压随机样本</label>
    <span class="settings-hint">相同种子可复现；模板与宏共用种子。换一组可观察新的随机循环。</span>
    <div style="display:flex;align-items:center;gap:8px;flex-wrap:wrap">
      <input id="dunya_reset_seed" class="sim-input" aria-label="盾压随机种子" type="number" min="0" max="4294967295" step="1" value="${seed}" style="width:120px">
      <button id="dunya_reset_reroll" class="sim-btn" type="button">换一组随机结果</button>
    </div>`;
  const seedInput = container.querySelector('#dunya_reset_seed');
  const sync = () => {
    container.style.display = enabled && !isHanjiaExpectationEnabled() ? '' : 'none';
  };
  const save = () => {
    const value = Math.min(4294967295, Math.max(0, Math.trunc(Number(seedInput.value) || 0)));
    seedInput.value = String(value);
    try { localStorage.setItem('dunya_reset_seed', String(value)); } catch {}
    runSimulate();
  };
  seedInput.addEventListener('change', save);
  container.querySelector('#dunya_reset_reroll').addEventListener('click', () => {
    seedInput.value = String(crypto.getRandomValues(new Uint32Array(1))[0]);
    save();
  });
  sync();
  return sync;
}
