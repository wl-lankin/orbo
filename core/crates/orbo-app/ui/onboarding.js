(() => {
  'use strict';
  const T = window.__TAURI__;
  const { invoke } = T.core;
  const $ = id => document.getElementById(id);
  const $$ = (s, r = document) => [...r.querySelectorAll(s)];
  const SVGNS = 'http://www.w3.org/2000/svg';
  const KIND_COLORS = ['image', 'video', 'audio', 'data', 'archive', 'document'];
  const KEYS = { option: '⌥', control: '⌃', shift: '⇧', command: '⌘', fn: 'fn' };

  let S = { style: 'classic', trigger_key: 'option', shake: true, autostart: false };
  let step = 0;
  const steps = $$('.step'), prog = $$('#prog button');
  const LAST = steps.length - 1;

  const pol = (cx, cy, r, deg) => { const a = deg * Math.PI / 180; return [cx + r * Math.sin(a), cy - r * Math.cos(a)]; };
  function arcPath(cx, cy, r, a0, a1) {
    const [x0, y0] = pol(cx, cy, r, a0), [x1, y1] = pol(cx, cy, r, a1);
    return `M${x0.toFixed(2)} ${y0.toFixed(2)}A${r} ${r} 0 ${a1 - a0 > 180 ? 1 : 0} 1 ${x1.toFixed(2)} ${y1.toFixed(2)}`;
  }
  function el(name, attrs, parent) {
    const n = document.createElementNS(SVGNS, name);
    for (const k in attrs) n.setAttribute(k, attrs[k]);
    if (parent) parent.appendChild(n);
    return n;
  }

  // Hero: ring segments assemble from outside, files orbit.
  (function buildHero() {
    const g = $('heroArcs');
    KIND_COLORS.forEach((k, i) => {
      const mid = i * 60 + 30, a = mid * Math.PI / 180;
      const p = el('path', { class: 'arc', d: arcPath(210, 120, 50, i * 60 + 6, i * 60 + 54), stroke: `var(--${k})` }, g);
      p.style.cssText = `--i:${i};--ux:${Math.sin(a).toFixed(3)};--uy:${(-Math.cos(a)).toFixed(3)}`;
    });
    const o = $('heroOrbo');
    [['image', 0, 26, 33], ['video', 90, 22, 28], ['audio', 180, 24, 30], ['data', 270, 20, 25]].forEach(([k, deg, w, h], i) => {
      const [x, y] = pol(210, 120, 106, deg + 40);
      const wrap = el('g', { class: 'fo-in' }, o);
      wrap.style.setProperty('--i', i);
      const fo = el('g', { class: 'fo' }, wrap);
      const u = el('use', { href: '#file', x: (x - w / 2).toFixed(1), y: (y - h / 2).toFixed(1), width: w, height: h, class: `c-${k}` }, fo);
      u.style.opacity = '.92';
    });
  })();

  (function buildScene() {
    const g = $('sceneArcs');
    KIND_COLORS.forEach((k, i) => {
      el('path', { class: 'sarc' + (i === 1 ? ' hit' : ''), d: arcPath(270, 108, 58, i * 60 + 2.5, i * 60 + 57.5), stroke: `var(--${k})`, style: `color:var(--${k})` }, g);
    });
  })();

  // ---------- navigation ----------
  function show(n, instant) {
    step = Math.max(0, Math.min(LAST, n));
    steps.forEach((s, i) => {
      s.classList.toggle('on', i === step);
      s.classList.toggle('before', i < step);
      s.setAttribute('aria-hidden', i === step ? 'false' : 'true');
    });
    prog.forEach((b, i) => {
      b.classList.toggle('on', i === step);
      b.classList.toggle('done', i < step);
      b.setAttribute('aria-current', i === step ? 'step' : 'false');
    });
    $('back').classList.toggle('hide', step === 0);
    $('next').textContent = step === LAST ? 'Einrichtung abschließen' : 'Weiter';
    const first = steps[step].querySelector('[data-first]');
    if (first && !instant) first.focus({ preventScroll: true });
  }
  async function next() {
    if (step < LAST) return show(step + 1);
    const btn = $('next');
    btn.disabled = true;
    try {
      const cur = await invoke('settings_get');
      await invoke('settings_set', { settings: Object.assign({}, cur, { style: S.style, trigger_key: S.trigger_key, shake: S.shake, autostart: S.autostart, onboarded: true }) });
    } catch (err) { console.error('save failed', err); }
    try { await invoke('onboarding_done'); } catch (err) { console.error(err); btn.disabled = false; }
  }
  $('next').addEventListener('click', next);
  $('back').addEventListener('click', () => show(step - 1));
  prog.forEach((b, i) => b.addEventListener('click', () => { if (i < step) show(i); }));
  document.addEventListener('keydown', e => {
    if (e.key !== 'Enter' || e.isComposing) return;
    const t = e.target;
    if (t && t.closest && t.closest('button, a, [role=radio]') && !t.matches('[data-next]')) return;
    e.preventDefault(); next();
  });

  // ---------- step 2: sample ----------
  const hint = $('sampleHint');
  $('sampleBtn').addEventListener('click', async () => {
    hint.hidden = false; hint.classList.remove('bad');
    try {
      await invoke('onboarding_sample');
      hint.innerHTML = `Die Beispieldatei liegt jetzt im Finder. Ziehe sie, halte <span class="kbd">${KEYS[S.trigger_key] || '⌥'}</span> gedrückt und lege sie auf einem Feld ab.`;
    } catch (_) {
      hint.classList.add('bad');
      hint.textContent = 'Die Beispieldatei konnte nicht angelegt werden.';
    }
  });

  // ---------- step 3: folder access (macOS) or Explorer entry (Windows) ----------
  const IS_WIN = /Windows/i.test(navigator.userAgent);
  document.documentElement.classList.toggle('is-win', IS_WIN);
  if (IS_WIN) {
    $('step3Label').textContent = 'Rechtsklick';
    const btn = $('winCtxBtn'), note = $('winCtxNote');
    const render = st => {
      if (st && st.installed) {
        btn.textContent = 'Eingerichtet';
        btn.disabled = true;
        note.textContent = 'Fertig. Rechtsklick auf eine Datei im Explorer, dann „Mit Orbo umwandeln“. Unter Windows 11 steht der Eintrag unter „Weitere Optionen anzeigen“.';
      }
    };
    invoke('context_menu_status').then(render).catch(() => {});
    btn.addEventListener('click', async () => {
      btn.disabled = true;
      try { render(await invoke('context_menu_install')); }
      catch (e) { btn.disabled = false; note.textContent = 'Das hat nicht geklappt: ' + e; }
    });
  }
  const NAMES = { desktop: 'Schreibtisch', documents: 'Dokumente', downloads: 'Downloads' };
  let granted = {};
  function renderFolders(res, animate) {
    let all = true;
    $$('.frow').forEach(r => {
      const k = r.dataset.f, ok = !!(res && res[k]), s = r.querySelector('[data-s]');
      if (!ok) all = false;
      const was = s.classList.contains('ok');
      s.classList.toggle('ok', ok);
      s.textContent = ok ? 'Freigegeben' : 'Nicht freigegeben';
      if (animate && ok && !was) { s.classList.remove('pop'); void s.offsetWidth; s.classList.add('pop'); }
    });
    granted = res || {};
    const gb = $('grantBtn');
    gb.textContent = all ? 'Alles freigegeben' : 'Zugriff erlauben';
    gb.disabled = all;
    $('grantNote').textContent = all ? 'Orbo hat Zugriff auf alle drei Ordner.'
      : 'Die Freigabe kannst du jederzeit nachholen. Ohne Freigabe fragt macOS erst beim ersten Zugriff.';
  }
  async function check() {
    try { renderFolders(await invoke('request_folder_access'), true); }
    catch (err) { console.error('folder access failed', err); }
  }
  $('grantBtn').addEventListener('click', async () => { $('grantBtn').disabled = true; await check(); if ($('grantBtn').textContent !== 'Alles freigegeben') $('grantBtn').disabled = false; });
  $('recheckBtn').addEventListener('click', check);
  $('privacyBtn').addEventListener('click', () => invoke('open_privacy_settings').catch(() => {}));
  window.addEventListener('focus', () => { if (step === 2 && Object.keys(granted).length) check(); });

  // ---------- step 4: choices ----------
  const mark = (root, v) => $$('[data-v]', root).forEach(b => { const on = b.dataset.v === v; b.classList.toggle('on', on); b.setAttribute('aria-checked', on); });
  function applyStyle() {
    document.body.classList.toggle('style-glass', S.style === 'glass');
    document.body.classList.toggle('style-classic', S.style !== 'glass');
    document.body.classList.toggle('reduce-motion', !!S.reduce_motion);
  }
  function renderChoices() {
    applyStyle();
    mark($('keys'), S.trigger_key);
    mark($('themes'), S.style);
    $('autostart').checked = !!S.autostart;
    $('shake').checked = !!S.shake;
    const glyph = KEYS[S.trigger_key] || '⌥';
    $('keyInline').textContent = glyph; $('sceneKey').textContent = glyph;
    $('sceneKey').setAttribute('font-size', glyph.length > 1 ? 18 : 22);
  }
  $$('#keys [data-v]').forEach(b => b.addEventListener('click', () => { S.trigger_key = b.dataset.v; renderChoices(); }));
  $$('#themes [data-v]').forEach(b => b.addEventListener('click', () => { S.style = b.dataset.v; renderChoices(); }));
  $('autostart').addEventListener('change', e => { S.autostart = e.target.checked; });
  $('shake').addEventListener('change', e => { S.shake = e.target.checked; });

  T.event.listen('settings', e => { if (e.payload) { S = Object.assign({}, S, e.payload); renderChoices(); } });

  (async () => {
    try { S = Object.assign(S, await invoke('settings_get')); } catch (_) { /* defaults */ }
    renderChoices();
    renderFolders(null);
    const h = (location.hash || '').slice(1).split(',');
    const n = h.find(x => /^s[0-3]$/.test(x));
    show(n ? +n[1] : 0, true);
  })();
})();
