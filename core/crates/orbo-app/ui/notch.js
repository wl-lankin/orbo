(() => {
  'use strict';
  const T = window.__TAURI__;
  const { invoke } = T.core;
  const { listen } = T.event;
  const IS_MAC = /Mac/.test(navigator.platform || navigator.userAgent);
  const KINDS = new Set(['image', 'video', 'audio', 'document', 'data', 'archive', 'folder', 'model', 'cad', 'typeface']);
  const MB = 1048576, KB = 1024;
  const $ = id => document.getElementById(id);
  const island = $('island'), tilesEl = $('tiles');
  const esc = s => String(s).replace(/[&<>"]/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));

  const S = { active: false, token: 0, paths: [], targets: [], srcKind: 'document', hot: -1, closeTimer: 0, lastPos: null, loading: false };

  function fmtB(b) {
    if (b < MB) return Math.max(1, Math.round(b / KB)).toLocaleString('de-DE') + ' KB';
    if (b < 1024 * MB) return (b / MB).toLocaleString('de-DE', { maximumFractionDigits: b < 10 * MB ? 1 : 0 }) + ' MB';
    return (b / 1024 / MB).toLocaleString('de-DE', { maximumFractionDigits: 1 }) + ' GB';
  }
  const kindOf = t => (t.intent ? S.srcKind : (KINDS.has(t.kind) ? t.kind : 'document'));

  function build(sug) {
    const files = sug.files || [];
    S.srcKind = (files[0] && KINDS.has(files[0].kind)) ? files[0].kind : 'document';
    S.targets = (sug.ring || []).slice(0, 6);
    $('fname').textContent = files.length > 1 ? `${files.length} Dateien` : (files[0] ? files[0].name : '');
    $('fhint').textContent = 'Auf einem Feld ablegen';
    tilesEl.classList.remove('committed');
    if (!S.targets.length) { tilesEl.innerHTML = '<div class="empty">Keine Umwandlung verfügbar</div>'; return; }
    tilesEl.innerHTML = S.targets.map((t, i) => {
      const k = kindOf(t), z = t.size_estimate != null ? fmtB(t.size_estimate) : (t.dialog ? 'Optionen' : '');
      return `<div class="tile${t.intent ? ' intent' : ''}${t.ai ? ' ai' : ''}${t.chain ? ' chain' : ''}" style="--k:var(--${k});--i:${i}">
        <svg class="ic"><use href="#i-${t.intent ? 'intent' : t.chain ? 'chain' : t.ai ? 'ai' : k}"/></svg>
        <div class="t">${esc(t.title)}</div><div class="s">${esc(t.sub || '')}</div><div class="z">${esc(z)}</div>
        <div class="done"><svg><use href="#i-check"/></svg></div></div>`;
    }).join('');
  }

  function hitTile(x, y) {
    const tiles = [...tilesEl.children];
    for (let i = 0; i < tiles.length; i++) {
      const r = tiles[i].getBoundingClientRect();
      // Slightly generous so the gaps between tiles still count.
      if (x >= r.left - 4 && x <= r.right + 4 && y >= r.top - 8 && y <= r.bottom + 12) return i;
    }
    return -1;
  }
  function hover(x, y) {
    if (!S.active || !S.targets.length) return;
    const i = hitTile(x, y);
    if (i === S.hot) return;
    S.hot = i;
    [...tilesEl.children].forEach((el, j) => el.classList.toggle('hot', j === i));
  }

  async function drop(paths, pos) {
    if (!S.active && !(S.graceUntil && performance.now() < S.graceUntil)) return;
    S.graceUntil = 0;
    const i = pos ? hitTile(pos.x, pos.y) : S.hot;
    paths = (paths && paths.length) ? paths : S.paths;
    const o = i >= 0 ? S.targets[i] : null;
    if (!o) { try { await invoke('notch_hide'); } catch (_) { /* ignore */ } return; }
    tilesEl.classList.add('committed');
    tilesEl.children[i].classList.add('picked');
    try {
      if (o.dialog) await invoke('open_dialog', { paths, target: o.id });
      else await invoke('convert', { paths, target: o.id, options: {} });
    } catch (err) { console.error('convert failed', err); }
    setTimeout(() => invoke('notch_hide').catch(() => {}), 380);
  }

  function applySettings(s) {
    if (!s) return;
    if (s.style) {
      document.body.classList.toggle('style-glass', s.style === 'glass');
      document.body.classList.toggle('style-classic', s.style !== 'glass');
    }
    if (typeof s.reduce_motion === 'boolean') document.body.classList.toggle('reduce-motion', s.reduce_motion);
  }

  function reset() { tilesEl.innerHTML = ''; S.targets = []; S.hot = -1; }

  async function loadSuggest(paths) {
    const tok = S.token;
    S.loading = true;
    try {
      const sug = await invoke('suggest', { paths });
      if (tok !== S.token || !S.active) return;
      S.paths = paths; build(sug);
      if (S.lastPos) hover(S.lastPos.x, S.lastPos.y);
    } catch (err) { console.error('suggest failed', err); }
    finally { if (tok === S.token) S.loading = false; }
  }

  function open(p) {
    clearTimeout(S.closeTimer);
    S.token++;
    reset();
    applySettings({ style: p.style, reduce_motion: p.reduce_motion });
    S.active = true; S.paths = p.files || [];
    if (p.suggestions) build(p.suggestions);
    else { tilesEl.innerHTML = '<div class="empty">Datei wird gelesen …</div>'; if (S.paths.length) loadSuggest(S.paths); }
    void island.offsetWidth;
    island.classList.add('open');
  }
  function close(reason) {
    if (!S.active) return;
    S.active = false;
    S.graceUntil = reason === 'released' ? performance.now() + 450 : 0;
    island.classList.remove('open');
    const tok = S.token;
    S.closeTimer = setTimeout(() => { if (tok === S.token) reset(); }, 500);
  }

  listen('notch:open', e => open(e.payload || {}));
  listen('notch:close', e => close(e.payload && e.payload.reason));
  listen('settings', e => applySettings(e.payload));

  T.webview.getCurrentWebview().onDragDropEvent(e => {
    const p = e.payload;
    if (!p) return;
    const scale = IS_MAC ? 1 : (window.devicePixelRatio || 1);
    const pos = p.position ? { x: p.position.x / scale, y: p.position.y / scale } : null;
    if (pos) S.lastPos = pos;
    if (p.type === 'enter') {
      if (p.paths && p.paths.length) S.paths = p.paths;
      if (S.active && !S.targets.length && !S.loading && p.paths && p.paths.length) loadSuggest(p.paths);
      if (pos) hover(pos.x, pos.y);
    } else if (p.type === 'over') { if (pos) hover(pos.x, pos.y); }
    else if (p.type === 'drop') drop(p.paths, pos);
    else if (p.type === 'leave') { S.hot = -1; [...tilesEl.children].forEach(el => el.classList.remove('hot')); }
  });

  invoke('settings_get').then(applySettings).catch(() => {});
})();
