(async function () {
  const { invoke } = window.__TAURI__.core;
  const { listen } = window.__TAURI__.event;
  const $ = id => document.getElementById(id);
  const h = (tag, props = {}, ...kids) => {
    const e = document.createElement(tag);
    for (const [k, v] of Object.entries(props)) {
      if (k === 'class') e.className = v;
      else if (k === 'html') e.innerHTML = v;
      else if (k.startsWith('on')) e.addEventListener(k.slice(2), v);
      else if (v !== false && v != null) e.setAttribute(k, v === true ? '' : v);
    }
    for (const c of kids.flat()) if (c != null) e.append(c);
    return e;
  };
  const clamp = (v, a, b) => Math.min(b, Math.max(a, v));
  const fmtTime = t => { t = Math.max(0, t); const m = Math.floor(t / 60); return m + ':' + (t - m * 60).toFixed(1).padStart(4, '0'); };
  const nf = new Intl.NumberFormat('de-DE');

  function applyStyle(s) {
    document.body.classList.toggle('style-glass', s.style === 'glass');
    document.body.classList.toggle('style-classic', s.style !== 'glass');
    document.body.classList.toggle('reduce-motion', !!s.reduce_motion);
  }
  try { applyStyle(await invoke('settings_get')); } catch (_) {}
  listen('settings', e => applyStyle(e.payload));

  const ctx = await invoke('dialog_context');
  // Rust returns a file path; the web view needs an asset-protocol URL.
  if (ctx.preview && !/^[a-z]+:/i.test(ctx.preview) && window.__TAURI__) ctx.preview = window.__TAURI__.core.convertFileSrc(ctx.preview);
  const f = ctx.file, target = ctx.target;
  const ctl = new Set(target.controls);
  const stage = $('stage'), panel = $('panel');
  const parts = [];
  let preset = ctx.preset || null;

  document.body.style.setProperty('--k', `var(--${f.kind === 'document' ? 'document' : f.kind})`);
  $('stripTitle').append(h('b', {}, target.title), h('span', {}, target.sub));
  $('fname').textContent = f.name;
  $('fmeta').append(h('i', {}, (f.format || '').toUpperCase()), (f.meta || '') + (ctx.paths.length > 1 ? `  +${ctx.paths.length - 1} weitere` : ''));
  $('fileline').hidden = false;

  const id = target.id;
  $('goLabel').textContent = /^trim/.test(id) || (ctl.has('trim') && target.controls.length === 1) ? 'Kürzen'
    : ctl.has('crop') ? 'Zuschneiden' : ctl.has('background') ? 'Speichern' : id === 'resize' ? 'Anwenden' : 'Umwandeln';

  /* ---------- building blocks ---------- */
  function chips(items, value, onPick) {
    const el = h('div', { class: 'chips' });
    const btns = items.map(it => {
      const b = h('button', { class: 'chip', type: 'button', disabled: !!it.disabled, onclick: () => { api.set(it.v); onPick(it.v); } }, it.label);
      b._v = it.v; el.append(b); return b;
    });
    const api = { el, set(v) { btns.forEach(b => b.classList.toggle('on', b._v === v)); } };
    api.set(value);
    return api;
  }
  function seg(items, value, onPick) {
    const el = h('div', { class: 'seg' });
    const btns = items.map(it => { const b = h('button', { type: 'button', onclick: () => { api.set(it.v); onPick(it.v); } }, it.label); b._v = it.v; el.append(b); return b; });
    const api = { el, set(v) { btns.forEach(b => b.classList.toggle('on', b._v === v)); } };
    api.set(value);
    return api;
  }
  function slider(min, max, step, value, onInput) {
    const el = h('input', { type: 'range', min, max, step, value });
    const paint = () => el.style.setProperty('--p', ((el.value - min) / (max - min)) * 100 + '%');
    el.addEventListener('input', () => { paint(); onInput(+el.value); });
    paint();
    return { el, set(v) { el.value = v; paint(); } };
  }
  const ctlBox = (label, body, span = 's3', readout) => {
    const out = readout ? h('output') : null;
    const el = h('div', { class: 'ctl ' + span }, h('div', { class: 'lab' }, h('span', {}, label), out), body);
    return { el, out };
  };
  const toPanel = el => panel.append(el);

  /* ---------- fitted stage area ---------- */
  function fitArea(cls) {
    const area = h('div', { class: 'stage-fit ' + (cls || '') });
    stage.append(area);
    const fit = (w, hh) => {
      const aw = area.clientWidth - 32, ah = area.clientHeight - 32;
      const s = Math.min(aw / w, ah / hh);
      return { s, w: Math.max(1, Math.floor(w * s)), h: Math.max(1, Math.floor(hh * s)) };
    };
    return { area, fit };
  }
  const loadImg = src => new Promise(res => { const i = new Image(); i.onload = () => res(i); i.onerror = () => res(i); i.src = src; });

  /* ---------- presets ---------- */
  if (target.presets && target.presets.length) {
    const c = chips([{ v: null, label: 'Standard' }, ...target.presets.map(p => ({ v: p, label: p }))], preset, v => { preset = v; });
    const b = ctlBox('Vorlage', c.el, 's6');
    toPanel(b.el);
    parts.push({ reset() { preset = ctx.preset || null; c.set(preset); } });
  }

  /* ---------- Stage: crop ---------- */
  if (ctl.has('crop')) {
    const img = await loadImg(ctx.preview);
    const W = f.width || img.naturalWidth, H = f.height || img.naturalHeight;
    const { area, fit } = fitArea();
    const box = h('div', { class: 'imgbox checker' });
    img.style.cssText = '';
    const rectEl = h('div', { class: 'crop' }, ['nw', 'n', 'ne', 'w', 'e', 'sw', 's', 'se'].map(d => h('div', { class: 'h', 'data-h': d })));
    box.append(img, rectEl);
    const badge = h('div', { class: 'badge' });
    area.append(box);
    stage.append(badge);
    const MIN = 16;
    let ratio = null, scale = 1, rect, inW, inH;
    const initRect = () => { const w = Math.round(W * .8), hh = Math.round(H * .8); rect = { x: Math.round((W - w) / 2), y: Math.round((H - hh) / 2), w, h: hh }; };
    initRect();

    function paint() {
      rectEl.style.cssText = `left:${rect.x * scale}px;top:${rect.y * scale}px;width:${rect.w * scale}px;height:${rect.h * scale}px`;
      const r = { w: Math.round(rect.w), h: Math.round(rect.h) };
      badge.innerHTML = `Auswahl <b>${nf.format(r.w)} × ${nf.format(r.h)}</b> px`;
      if (inW && document.activeElement !== inW) inW.value = r.w;
      if (inH && document.activeElement !== inH) inH.value = r.h;
    }
    function layout() { const d = fit(W, H); scale = d.s; box.style.width = d.w + 'px'; box.style.height = d.h + 'px'; paint(); }
    new ResizeObserver(layout).observe(area);

    function applyHandle(hd, px, py, R) {
      let x0 = R.x, x1 = R.x + R.w, y0 = R.y, y1 = R.y + R.h;
      px = clamp(px, 0, W); py = clamp(py, 0, H);
      if (hd.includes('w')) x0 = Math.min(px, x1 - MIN);
      if (hd.includes('e')) x1 = Math.max(px, x0 + MIN);
      if (hd.includes('n')) y0 = Math.min(py, y1 - MIN);
      if (hd.includes('s')) y1 = Math.max(py, y0 + MIN);
      if (ratio) {
        const hasX = /[we]/.test(hd), hasY = /[ns]/.test(hd);
        let w = x1 - x0, hh = y1 - y0;
        if (hasX && hasY) {
          const ax = hd.includes('w') ? R.x + R.w : R.x, ay = hd.includes('n') ? R.y + R.h : R.y;
          const maxW = hd.includes('w') ? ax : W - ax, maxH = hd.includes('n') ? ay : H - ay;
          w = Math.min(Math.max(w, hh * ratio), maxW, maxH * ratio); hh = w / ratio;
          x0 = hd.includes('w') ? ax - w : ax; x1 = x0 + w;
          y0 = hd.includes('n') ? ay - hh : ay; y1 = y0 + hh;
        } else if (hasX) {
          const cy = R.y + R.h / 2;
          w = Math.min(w, (hd.includes('w') ? R.x + R.w : W - R.x), 2 * Math.min(cy, H - cy) * ratio); hh = w / ratio;
          if (hd.includes('w')) x0 = x1 - w; else x1 = x0 + w;
          y0 = cy - hh / 2; y1 = y0 + hh;
        } else {
          const cx = R.x + R.w / 2;
          hh = Math.min(hh, (hd.includes('n') ? R.y + R.h : H - R.y), 2 * Math.min(cx, W - cx) / ratio); w = hh * ratio;
          if (hd.includes('n')) y0 = y1 - hh; else y1 = y0 + hh;
          x0 = cx - w / 2; x1 = x0 + w;
        }
      }
      rect = { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
    }

    let drag = null;
    rectEl.addEventListener('pointerdown', e => {
      const hd = e.target.dataset.h || 'move';
      drag = { hd, sx: e.clientX, sy: e.clientY, R: { ...rect } };
      rectEl.setPointerCapture(e.pointerId); rectEl.classList.add('drag'); e.preventDefault();
    });
    rectEl.addEventListener('pointermove', e => {
      if (!drag) return;
      const bb = box.getBoundingClientRect();
      if (drag.hd === 'move') {
        const dx = (e.clientX - drag.sx) / scale, dy = (e.clientY - drag.sy) / scale;
        rect = { ...drag.R, x: clamp(drag.R.x + dx, 0, W - drag.R.w), y: clamp(drag.R.y + dy, 0, H - drag.R.h) };
      } else applyHandle(drag.hd, (e.clientX - bb.left) / scale, (e.clientY - bb.top) / scale, drag.R);
      paint();
    });
    const end = () => { drag = null; rectEl.classList.remove('drag'); };
    rectEl.addEventListener('pointerup', end); rectEl.addEventListener('pointercancel', end);

    function setRatio(r) {
      ratio = r;
      if (!r) return paint();
      const cx = rect.x + rect.w / 2, cy = rect.y + rect.h / 2;
      let w = rect.w, hh = w / r;
      if (hh > rect.h) { hh = rect.h; w = hh * r; }
      w = Math.min(w, 2 * Math.min(cx, W - cx), 2 * Math.min(cy, H - cy) * r); hh = w / r;
      rect = { x: cx - w / 2, y: cy - hh / 2, w, h: hh };
      paint();
    }
    const AR = [['Frei', null], ['1:1', 1], ['16:9', 16 / 9], ['9:16', 9 / 16], ['4:3', 4 / 3], ['3:4', 3 / 4], ['4:5', 4 / 5]];
    let arKey = 'Frei';
    const ac = chips(AR.map(([l]) => ({ v: l, label: l })), arKey, v => { arKey = v; setRatio(AR.find(a => a[0] === v)[1]); });
    toPanel(ctlBox('Seitenverhältnis', ac.el, 's4').el);

    inW = h('input', { class: 'num', type: 'number', min: MIN, max: W }); inH = h('input', { class: 'num', type: 'number', min: MIN, max: H });
    const commit = which => {
      let w = clamp(+inW.value || rect.w, MIN, W), hh = clamp(+inH.value || rect.h, MIN, H);
      if (ratio) { if (which === 'w') hh = w / ratio; else w = hh * ratio; if (hh > H) { hh = H; w = hh * ratio; } if (w > W) { w = W; hh = w / ratio; } }
      rect = { x: clamp(rect.x, 0, W - w), y: clamp(rect.y, 0, H - hh), w, h: hh };
      paint();
    };
    inW.addEventListener('input', () => commit('w')); inH.addEventListener('input', () => commit('h'));
    inW.addEventListener('blur', paint); inH.addEventListener('blur', paint);
    toPanel(ctlBox('Größe in Pixeln', h('div', { class: 'row' }, inW, h('span', { class: 'unit' }, '×'), inH), 's2').el);

    parts.push({
      collect(o) { o.crop = { x: Math.round(rect.x), y: Math.round(rect.y), width: Math.round(rect.w), height: Math.round(rect.h) }; if (ratio) o.aspect = arKey; },
      reset() { ratio = null; arKey = 'Frei'; ac.set(arKey); initRect(); paint(); },
    });
    ctl.delete('aspect'); ctl.delete('crop');
  }

  /* ---------- Stage: beautify ---------- */
  if (ctl.has('background')) {
    const img = await loadImg(ctx.preview);
    const iw = f.width || img.naturalWidth, ih = f.height || img.naturalHeight;
    const GRAD = { sunset: ['#ff7e5f', '#feb47b'], ocean: ['#2193b0', '#6dd5ed'], lavender: ['#a18cd1', '#fbc2eb'], mint: ['#43e97b', '#38f9d7'], night: ['#0f2027', '#2c5364'] };
    const SOLID = ['#ffffff', '#f2efe9', '#1b1d2a', '#000000', '#ff6b7a', '#ffd84d', '#5be08f', '#74b0ff'];
    const DEF = { bg: 'sunset', pad: 96, cor: 24, sh: 24, asp: 'Auto' };
    const S = { ...DEF };
    const { area, fit } = fitArea();
    const cv = h('canvas', { class: 'out' });
    area.append(cv);
    const badge = h('div', { class: 'badge' });
    stage.append(badge);
    const ASP = { Auto: null, '1:1': 1, '16:9': 16 / 9, '4:5': 4 / 5 };

    function canvasSize() {
      let cw = iw + 2 * S.pad, ch = ih + 2 * S.pad;
      const r = ASP[S.asp];
      if (r) { if (cw / ch < r) cw = Math.round(ch * r); else ch = Math.round(cw / r); }
      return { cw, ch };
    }
    function draw() {
      const { cw, ch } = canvasSize();
      const d = fit(cw, ch), dpr = window.devicePixelRatio || 1, s = d.s;
      cv.style.width = d.w + 'px'; cv.style.height = d.h + 'px';
      cv.width = Math.round(d.w * dpr); cv.height = Math.round(d.h * dpr);
      const g = cv.getContext('2d'); g.setTransform(dpr * d.w / cw, 0, 0, dpr * d.h / ch, 0, 0);
      if (GRAD[S.bg]) { const gr = g.createLinearGradient(0, 0, cw, ch); gr.addColorStop(0, GRAD[S.bg][0]); gr.addColorStop(1, GRAD[S.bg][1]); g.fillStyle = gr; }
      else g.fillStyle = S.bg;
      g.fillRect(0, 0, cw, ch);
      const x = (cw - iw) / 2, y = (ch - ih) / 2, rad = Math.min(S.cor, iw / 2, ih / 2);
      const path = () => { g.beginPath(); g.roundRect ? g.roundRect(x, y, iw, ih, rad) : g.rect(x, y, iw, ih); };
      if (S.sh > 0) {
        g.save(); const u = d.w / cw * dpr;
        g.shadowColor = 'rgba(0,0,0,.42)'; g.shadowBlur = S.sh * u; g.shadowOffsetY = S.sh * .35 * u; g.fillStyle = '#000'; path(); g.fill(); g.restore();
      }
      g.save(); path(); g.clip(); g.drawImage(img, x, y, iw, ih); g.restore();
      badge.innerHTML = `Ergebnis <b>${nf.format(cw)} × ${nf.format(ch)}</b> px`;
    }
    new ResizeObserver(draw).observe(area);

    // background swatches
    const GNAME = { sunset: 'Sonnenuntergang', ocean: 'Ozean', lavender: 'Lavendel', mint: 'Minze', night: 'Nacht' };
    const sw = h('div', { class: 'sw' });
    const swBtns = [];
    const mk = (val, css, title) => { const b = h('button', { type: 'button', title, 'aria-label': title, style: `background:${css}`, onclick: () => { S.bg = val; sync(); draw(); } }); b._v = val; swBtns.push(b); sw.append(b); };
    Object.entries(GRAD).forEach(([k, [a, b]]) => mk(k, `linear-gradient(135deg,${a},${b})`, GNAME[k] || k));
    sw.append(h('span', { class: 'sep' }));
    SOLID.forEach(c => mk(c, c, c));
    const cin = h('input', { type: 'color', value: '#3d7bff', 'aria-label': 'Eigene Farbe' });
    const clab = h('label', { class: 'custom', title: 'Eigene Farbe' }, cin);
    cin.addEventListener('input', () => { S.bg = cin.value; sync(); draw(); });
    sw.append(clab);
    const sync = () => {
      swBtns.forEach(b => b.classList.toggle('on', b._v === S.bg));
      const custom = !GRAD[S.bg] && !SOLID.includes(S.bg);
      clab.classList.toggle('on', custom); clab.style.setProperty('--cc', S.bg);
    };
    toPanel(ctlBox('Hintergrund', sw, 's6').el);

    const sl = (key, label, max, unit = ' px') => {
      const b = ctlBox(label, null, 's2', true);
      const s = slider(0, max, 1, S[key], v => { S[key] = v; b.out.textContent = v + unit; draw(); });
      b.el.append(s.el); b.out.textContent = S[key] + unit; toPanel(b.el);
      return { reset() { s.set(S[key]); b.out.textContent = S[key] + unit; } };
    };
    const sPad = sl('pad', 'Rand', 400), sCor = sl('cor', 'Ecken', 200), sSh = sl('sh', 'Schatten', 80);
    const ac = chips(Object.keys(ASP).map(k => ({ v: k, label: k })), S.asp, v => { S.asp = v; draw(); });
    toPanel(ctlBox('Seitenverhältnis', ac.el, 's6').el);

    parts.push({
      collect(o) { o.background = S.bg; o.padding_px = S.pad; o.corner_px = S.cor; o.shadow_px = S.sh; if (S.asp !== 'Auto') o.aspect = S.asp; },
      reset() { Object.assign(S, DEF); sync(); sPad.reset(); sCor.reset(); sSh.reset(); ac.set(S.asp); draw(); },
    });
    sync();
    ['background', 'padding', 'corners', 'shadow', 'aspect'].forEach(c => ctl.delete(c));
  }

  /* ---------- Stage: trim ---------- */
  if (ctl.has('trim') && (f.kind === 'video' || f.kind === 'audio')) {
    const isVideo = f.kind === 'video';
    stage.classList.toggle('audio', !isVideo);
    const media = isVideo ? h('video', { src: ctx.preview, playsinline: true, preload: 'auto' }) : new Audio(ctx.preview);
    if (isVideo) stage.append(h('div', { class: 'mediabox' }, media));
    let dur = f.duration_s || 0, s = 0, e = dur;
    const tS = h('div', { class: 't' }, 'Start', h('b')), tE = h('div', { class: 't r' }, 'Ende', h('b'));
    const durEl = h('span', { class: 'dur' });
    const playBtn = h('button', { class: 'play', type: 'button' });
    const icoPlay = '<svg viewBox="0 0 12 12"><path d="M2 1l9 5-9 5z"/></svg>Auswahl abspielen', icoPause = '<svg viewBox="0 0 12 12"><path d="M2 1h3v10H2zM7 1h3v10H7z"/></svg>Pause';
    playBtn.innerHTML = icoPlay;
    stage.append(h('div', { class: 'tbar' }, tS, h('div', { class: 'mid' }, playBtn, durEl), tE));
    const canvas = h('canvas');
    const dimA = h('div', { class: 'dim a' }), dimB = h('div', { class: 'dim b' });
    const sel = h('div', { class: 'sel' }), hS = h('div', { class: 'hd s' }), hE = h('div', { class: 'hd e' }), ph = h('div', { class: 'ph' });
    const track = h('div', { class: 'tl' }, h('div', { class: 'strip-bg' }, canvas, dimA, dimB), sel, hS, hE, ph);
    stage.append(track);

    const pct = t => (dur ? t / dur : 0) * 100 + '%';
    function paint() {
      if (!dur) return;
      dimA.style.width = pct(s); dimB.style.width = (100 - (e / dur) * 100) + '%';
      sel.style.left = pct(s); sel.style.width = ((e - s) / dur) * 100 + '%';
      hS.style.left = pct(s); hE.style.left = pct(e);
      tS.lastChild.textContent = fmtTime(s); tE.lastChild.textContent = fmtTime(e); durEl.textContent = fmtTime(e - s);
      ph.style.left = pct(clamp(media.currentTime || 0, 0, dur));
    }
    const init = () => { dur = dur || media.duration || 0; if (!e || e > dur) e = dur; paint(); drawBack(); };
    media.addEventListener('loadedmetadata', init);
    if (dur) init();

    const timeAt = ev => { const r = track.getBoundingClientRect(); return clamp((ev.clientX - r.left) / r.width, 0, 1) * dur; };
    let dragH = null;
    track.addEventListener('pointerdown', ev => {
      if (!dur) return;
      const t = timeAt(ev);
      dragH = ev.target === hS ? 's' : ev.target === hE ? 'e' : 'seek';
      track.setPointerCapture(ev.pointerId);
      if (dragH === 'seek') media.currentTime = t;
    });
    track.addEventListener('pointermove', ev => {
      if (!dragH) return;
      const t = timeAt(ev);
      if (dragH === 's') { s = Math.min(t, e - .1); media.currentTime = s; }
      else if (dragH === 'e') { e = Math.max(t, s + .1); media.currentTime = Math.max(s, e - .05); }
      else media.currentTime = t;
      paint();
    });
    const up = () => { dragH = null; };
    track.addEventListener('pointerup', up); track.addEventListener('pointercancel', up);

    const setPlayLabel = () => { playBtn.innerHTML = media.paused ? icoPlay : icoPause; };
    playBtn.addEventListener('click', () => {
      if (!media.paused) { media.pause(); return; }
      if (media.currentTime < s || media.currentTime >= e - .05) media.currentTime = s;
      media.play();
    });
    media.addEventListener('play', setPlayLabel); media.addEventListener('pause', setPlayLabel);
    media.addEventListener('timeupdate', () => { if (!media.paused && media.currentTime >= e) { media.pause(); media.currentTime = s; } paint(); });
    document.addEventListener('keydown', ev => { if (ev.code === 'Space' && !/INPUT|BUTTON|SELECT/.test(ev.target.tagName)) { ev.preventDefault(); playBtn.click(); } });

    async function drawBack() {
      const w = track.clientWidth, hh = track.clientHeight, dpr = window.devicePixelRatio || 1;
      if (!w) return;
      canvas.width = w * dpr; canvas.height = hh * dpr;
      const g = canvas.getContext('2d'); g.scale(dpr, dpr);
      const bars = Math.floor(w / 4);
      let pk = null;
      try {
        const buf = await (await fetch(ctx.preview)).arrayBuffer();
        const ac = new (window.AudioContext || window.webkitAudioContext)();
        const d = await ac.decodeAudioData(buf); ac.close();
        const ch = d.getChannelData(0), step = Math.max(1, Math.floor(ch.length / bars));
        pk = Array.from({ length: bars }, (_, i) => { let m = 0; for (let j = 0; j < step; j += Math.max(1, step >> 5)) m = Math.max(m, Math.abs(ch[i * step + j] || 0)); return m; });
        const mx = Math.max(...pk, .001); pk = pk.map(v => v / mx);
      } catch (_) {}
      if (!isVideo) {
        if (!pk) pk = Array.from({ length: bars }, (_, i) => .25 + .6 * Math.abs(Math.sin(i * .21) * Math.cos(i * .067)));
        pk.forEach((v, i) => { const bh = Math.max(3, v * (hh - 16)); g.globalAlpha = .9; g.fillStyle = '#fff'; g.beginPath(); g.roundRect(i * 4 + .5, (hh - bh) / 2, 2.4, bh, 1.2); g.fill(); });
        g.globalAlpha = 1;
        return;
      }
      // Video: filmstrip of frames, best effort
      g.fillStyle = 'rgba(255,255,255,.08)'; g.fillRect(0, 0, w, hh);
      try {
        const v = h('video', { src: ctx.preview, muted: true, preload: 'auto' });
        const once = (el, ev) => Promise.race([new Promise(r => el.addEventListener(ev, r, { once: true })), new Promise((_, rj) => setTimeout(rj, 2500))]);
        await once(v, 'loadeddata');
        const tw = Math.round(hh * (v.videoWidth / v.videoHeight || 1.78)), n = Math.ceil(w / tw);
        for (let i = 0; i < n; i++) { v.currentTime = (dur * (i + .5)) / n; await once(v, 'seeked'); g.drawImage(v, i * tw, 0, tw, hh); }
      } catch (_) {}
    }
    new ResizeObserver(() => { paint(); drawBack(); }).observe(track);

    parts.push({
      collect(o) { if (dur && (s > .05 || e < dur - .05)) { o.start_s = +s.toFixed(2); o.end_s = +e.toFixed(2); } },
      reset() { s = 0; e = dur; media.pause(); media.currentTime = 0; paint(); },
    });
    ctl.delete('trim');
  }

  /* ---------- Stage: plain preview ---------- */
  let sizeBadge = null;
  if (!stage.children.length && ctx.preview && (f.kind === 'image' || f.kind === 'video')) {
    const { area, fit } = fitArea();
    const el = f.kind === 'image' ? h('img', { src: ctx.preview }) : h('video', { src: ctx.preview, muted: true, preload: 'auto' });
    const box = h('div', { class: 'imgbox checker' }, el);
    area.append(box);
    sizeBadge = h('div', { class: 'badge' });
    stage.append(sizeBadge);
    const lay = () => { if (!f.width) return; const d = fit(f.width, f.height); box.style.width = d.w + 'px'; box.style.height = d.h + 'px'; };
    new ResizeObserver(lay).observe(area);
    if (f.kind === 'video') el.addEventListener('loadeddata', () => { el.currentTime = .1; });
  }
  if (!stage.children.length) stage.hidden = true;

  /* ---------- Panel controls ---------- */
  if (ctl.has('resize') && f.width && f.height) {
    const W0 = f.width, H0 = f.height, ar = W0 / H0;
    const R = { w: W0, h: H0, lock: true };
    const iw = h('input', { class: 'num', type: 'number', min: 1, max: 16384 }), ih = h('input', { class: 'num', type: 'number', min: 1, max: 16384 });
    const lockB = h('button', { class: 'lock on', type: 'button', title: 'Seitenverhältnis sperren', 'aria-pressed': 'true',
      html: '<svg viewBox="0 0 16 16"><rect x="3.5" y="7.5" width="9" height="6" rx="1.6"/><path d="M5.5 7.5V5.5a2.5 2.5 0 015 0v2"/></svg>' });
    const pcts = [25, 50, 75, 100], longs = [3840, 1920, 1280, 800];
    const pc = chips(pcts.map(p => ({ v: p, label: p + ' %' })), 100, v => setW(Math.round(W0 * v / 100)));
    const lc = chips(longs.map(p => ({ v: p, label: String(p), disabled: p > Math.max(W0, H0) })), null, v => { const k = v / Math.max(W0, H0); R.lock = true; setW(Math.round(W0 * k), true); });
    function show() {
      if (document.activeElement !== iw) iw.value = R.w;
      if (document.activeElement !== ih) ih.value = R.h;
      pc.set(R.w === Math.round(W0 * R.w / W0) && pcts.find(p => Math.round(W0 * p / 100) === R.w && Math.round(H0 * p / 100) === R.h) || null);
      lc.set(longs.find(p => Math.max(R.w, R.h) === p) || null);
      lockB.classList.toggle('on', R.lock); lockB.setAttribute('aria-pressed', R.lock);
      if (sizeBadge) sizeBadge.innerHTML = `Ergebnis <b>${nf.format(R.w)} × ${nf.format(R.h)}</b> px`;
    }
    function setW(w, fromLong) { R.w = clamp(w, 1, 16384); if (R.lock) R.h = Math.max(1, Math.round(R.w / ar)); show(); }
    function setH(v) { R.h = clamp(v, 1, 16384); if (R.lock) R.w = Math.max(1, Math.round(R.h * ar)); show(); }
    iw.addEventListener('input', () => +iw.value && setW(+iw.value)); ih.addEventListener('input', () => +ih.value && setH(+ih.value));
    iw.addEventListener('blur', show); ih.addEventListener('blur', show);
    lockB.addEventListener('click', () => { R.lock = !R.lock; if (R.lock) setW(R.w); show(); });
    const row = h('div', { class: 'row', style: 'gap:14px' },
      h('div', { class: 'row' }, iw, lockB, ih, h('span', { class: 'unit' }, 'px')), pc.el, lc.el);
    toPanel(ctlBox('Größe', row, 's6').el);
    parts.push({
      collect(o) { if (R.w !== W0 || R.h !== H0) { o.width = R.w; o.height = R.h; } },
      reset() { R.w = W0; R.h = H0; R.lock = true; show(); },
    });
    show();
  }

  if (ctl.has('quality') || ctl.has('target_size')) {
    const Q = { q: 80, mb: null };
    let qb, tb;
    const qlab = v => v < 40 ? 'Klein' : v < 70 ? 'Ausgewogen' : v < 90 ? 'Hoch' : 'Höchste';
    if (ctl.has('quality')) {
      qb = ctlBox('Qualität', null, ctl.has('target_size') ? 's3' : 's6', true);
      const sl = slider(10, 100, 5, Q.q, v => { Q.q = v; qb.out.textContent = `${v} %  ·  ${qlab(v)}`; });
      qb.out.textContent = `${Q.q} %  ·  ${qlab(Q.q)}`;
      qb.el.append(sl.el); toPanel(qb.el);
      parts.push({ collect(o) { if (Q.mb == null) o.quality = Q.q; }, reset() { Q.q = 80; sl.set(80); qb.out.textContent = `80 %  ·  ${qlab(80)}`; } });
    }
    if (ctl.has('target_size')) {
      const items = [{ v: 25, label: 'E-Mail 25 MB' }, { v: 16, label: 'WhatsApp 16 MB' }, { v: 10, label: 'Discord 10 MB' }, { v: null, label: 'Frei' }];
      const c = chips(items, null, v => { Q.mb = v; qb && qb.el.classList.toggle('dimmed', v != null); });
      tb = ctlBox('Zielgröße', c.el, ctl.has('quality') ? 's3' : 's6'); toPanel(tb.el);
      parts.push({ collect(o) { if (Q.mb != null) o.target_mb = Q.mb; }, reset() { Q.mb = null; c.set(null); qb && qb.el.classList.remove('dimmed'); } });
    }
  }

  const simple = (key, label, span, items, optKey, def = null) => {
    if (!ctl.has(key)) return;
    let val = def;
    const c = (items.length > 5 ? chips : seg)(items, val, v => { val = v; });
    toPanel(ctlBox(label, c.el, span).el);
    parts.push({ collect(o) { if (val != null) o[optKey] = val; }, reset() { val = def; c.set(def); } });
  };
  simple('audio_bitrate', 'Bitrate', 's3', [96, 128, 192, 256, 320].map(v => ({ v, label: v + ' k' })), 'audio_bitrate_k', ctx.target.id === 'mp3' || ctx.target.id === 'm4a' ? 192 : null);
  simple('sample_rate', 'Abtastrate', 's3', [{ v: null, label: 'Original' }, { v: 44100, label: '44,1 kHz' }, { v: 48000, label: '48 kHz' }], 'sample_rate');
  simple('channels', 'Kanäle', 's2', [{ v: null, label: 'Original' }, { v: 1, label: 'Mono' }, { v: 2, label: 'Stereo' }], 'channels');
  simple('fps', 'Bildrate', 's4', [{ v: null, label: 'Standard' }, ...[8, 12, 15, 24, 30].map(v => ({ v, label: v + ' fps' }))], 'fps');

  if (ctl.has('rotate')) {
    let rot = null, flip = false;
    const sg = seg([{ v: null, label: 'Original' }, { v: 90, label: '90° rechts' }, { v: 180, label: '180°' }, { v: 270, label: '90° links' }], null, v => { rot = v; });
    const fl = h('button', { class: 'chip', type: 'button', onclick: () => { flip = !flip; fl.classList.toggle('on', flip); } }, 'Horizontal spiegeln');
    toPanel(ctlBox('Drehen', h('div', { class: 'row' }, sg.el, fl), 's6').el);
    parts.push({ collect(o) { if (rot) o.rotate = rot; if (flip) o.flip_horizontal = true; }, reset() { rot = null; flip = false; sg.set(null); fl.classList.remove('on'); } });
  }

  /* ---------- Footer ---------- */
  const err = $('err'), go = $('go');
  async function submit() {
    if (go.disabled) return;
    const options = {};
    parts.forEach(p => p.collect && p.collect(options));
    go.disabled = true; err.textContent = '';
    try {
      await invoke('convert', { paths: ctx.paths, target: target.id, preset, options });
      await invoke('dialog_close');
    } catch (ex) { err.textContent = String(ex && ex.message || ex); go.disabled = false; }
  }
  const close = () => invoke('dialog_close');
  go.addEventListener('click', submit);
  $('cancel').addEventListener('click', close);
  $('reset').addEventListener('click', () => { parts.forEach(p => p.reset && p.reset()); err.textContent = ''; });
  document.addEventListener('keydown', ev => {
    if (ev.key === 'Escape') { ev.preventDefault(); close(); }
    else if (ev.key === 'Enter' && !/BUTTON|SELECT/.test(ev.target.tagName)) { ev.preventDefault(); submit(); }
  });
})();
