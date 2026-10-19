(() => {
  'use strict';
  const T = window.__TAURI__;
  const { invoke } = T.core;
  const { listen } = T.event;
  const SVGNS = 'http://www.w3.org/2000/svg';

  const R = { lens: 62, inner: 70, out: 180, pin: 190, pout: 244 };
  const CENTER = 260;
  const IS_MAC = /Mac/.test(navigator.platform || navigator.userAgent);
  const KINDS = new Set(['image', 'video', 'audio', 'document', 'data', 'archive', 'folder', 'model', 'cad', 'typeface']);
  const MB = 1048576, KB = 1024;
  const $ = id => document.getElementById(id);
  const secsG = $('secs'), presG = $('pres'), lens = $('lens'), lensText = $('lensText'),
    needle = $('needle'), needlePath = $('needlePath'), stage = $('stage');

  const S = {
    active: false, token: 0, sound: false,
    paths: [], files: [], targets: [], more: [], def: 0, step: 0,
    srcKind: 'document', inBytes: 0,
    hot: null, key: '', presetsFor: -1, timer: 0, qv: {}, wheelAcc: 0, needleAngle: 0,
    waiting: false, pendingOpen: null, closeTimer: 0,
    mode: 'drag', openedAt: 0, committed: false,
  };

  // ---------- helpers ----------
  const esc = s => String(s).replace(/[&<>"]/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));
  const pol = (r, a) => [r * Math.sin(a), -r * Math.cos(a)];
  const norm = a => ((a % (2 * Math.PI)) + 2 * Math.PI) % (2 * Math.PI);
  const f1 = n => +n.toFixed(2);
  const pt = (r, a) => pol(r, a).map(f1).join(' ');

  function fmtB(b) {
    if (b < MB) return Math.max(1, Math.round(b / KB)).toLocaleString('de-DE') + ' KB';
    if (b < 1024 * MB) return (b / MB).toLocaleString('de-DE', { maximumFractionDigits: b < 10 * MB ? 1 : 0 }) + ' MB';
    return (b / 1024 / MB).toLocaleString('de-DE', { maximumFractionDigits: 1 }) + ' GB';
  }
  function delta(inB, out) {
    if (!inB || out == null) return null;
    const p = Math.max(-99, Math.round((out / inB - 1) * 100));
    return p < 0 ? { t: `−${-p} %`, up: false } : { t: `+${p} %`, up: true };
  }
  const qf = q => .35 + .65 * Math.pow(q / 80, 2);

  const cv = document.createElement('canvas').getContext('2d');
  const FONT = 'ui-rounded, "SF Pro Rounded", -apple-system, system-ui, sans-serif';
  function fit(txt, size, weight, maxW) {
    cv.font = `${weight} ${size}px ${FONT}`;
    if (cv.measureText(txt).width <= maxW) return txt;
    let s = txt;
    while (s.length > 1 && cv.measureText(s + '…').width > maxW) s = s.slice(0, -1);
    return s.trimEnd() + '…';
  }
  // Shrinks the font before resorting to an ellipsis.
  function shrink(txt, size, weight, maxW, minSize) {
    let sz = size;
    cv.font = `${weight} ${sz}px ${FONT}`;
    while (sz > minSize && cv.measureText(txt).width > maxW) { sz -= .5; cv.font = `${weight} ${sz}px ${FONT}`; }
    return { v: fit(txt, sz, weight, maxW), sz };
  }
  // Shrinks first, then breaks at the most central space; cuts only a single word that still does not fit.
  function wrap(txt, size, weight, maxW, minSize) {
    const one = shrink(txt, size, weight, maxW, minSize);
    if (!txt || one.v === txt || !txt.includes(' ')) return [one];
    const mid = txt.length / 2;
    const cut = [...txt].reduce((best, ch, i) => ch === ' ' && Math.abs(i - mid) < Math.abs(best - mid) ? i : best, txt.indexOf(' '));
    const ls = [txt.slice(0, cut), txt.slice(cut + 1)].map(l => shrink(l, size, weight, maxW, minSize));
    const sz = Math.min(...ls.map(l => l.sz));
    return ls.map(l => ({ v: l.v, sz }));
  }
  function fitName(name, size, weight, maxW) {
    cv.font = `${weight} ${size}px ${FONT}`;
    if (cv.measureText(name).width <= maxW) return name;
    const dot = name.lastIndexOf('.');
    const ext = dot > 0 ? name.slice(dot) : '';
    let base = dot > 0 ? name.slice(0, dot) : name;
    while (base.length > 1 && cv.measureText(base + '…' + ext).width > maxW) base = base.slice(0, -1);
    return base + '…' + ext;
  }

  // Centers of stacked lines so that the whole block is centered on cy.
  function stack(heights, cy, gap) {
    const total = heights.reduce((a, b) => a + b, 0) + gap * (heights.length - 1);
    let y = cy - total / 2;
    return heights.map(h => { const c = y + h / 2; y += h + gap; return f1(c); });
  }

  // Annular sector with radial sides and rounded corners.
  function sector(r0, r1, a0, a1, rc) {
    const eo = Math.asin(rc / (r1 - rc)), ei = Math.asin(rc / (r0 + rc));
    const o0 = a0 + eo, o1 = a1 - eo, i0 = a0 + ei, i1 = a1 - ei;
    const e1 = Math.sqrt((r1 - rc) ** 2 - rc ** 2), e0 = Math.sqrt((r0 + rc) ** 2 - rc ** 2);
    const L = o1 - o0 > Math.PI ? 1 : 0, Li = i1 - i0 > Math.PI ? 1 : 0;
    return `M${pt(e0, a0)}L${pt(e1, a0)}A${rc} ${rc} 0 0 1 ${pt(r1, o0)}A${r1} ${r1} 0 ${L} 1 ${pt(r1, o1)}` +
      `A${rc} ${rc} 0 0 1 ${pt(e1, a1)}L${pt(e0, a1)}A${rc} ${rc} 0 0 1 ${pt(r0, i1)}` +
      `A${r0} ${r0} 0 ${Li} 0 ${pt(r0, i0)}A${rc} ${rc} 0 0 1 ${pt(e0, a0)}Z`;
  }
  function rimArc(r, a0, a1, rc) {
    const e = Math.asin(rc / (R.out - rc)) + .03;
    const s = a0 + e, t = a1 - e;
    return `M${pt(r, s)}A${r} ${r} 0 ${t - s > Math.PI ? 1 : 0} 1 ${pt(r, t)}`;
  }

  const kindOf = t => (t.intent ? S.srcKind : (KINDS.has(t.kind) ? t.kind : 'document'));
  const colorOf = t => `var(--${kindOf(t)})`;
  const iconOf = t => (t.intent ? 'intent' : t.chain ? 'chain' : t.ai ? 'ai' : kindOf(t));

  // ---------- sound ----------
  let actx = null;
  function tick(freq = 2300) {
    if (!S.sound) return;
    try {
      actx = actx || new (window.AudioContext || window.webkitAudioContext)();
      const o = actx.createOscillator(), g = actx.createGain(), t = actx.currentTime;
      o.type = 'sine'; o.frequency.value = freq;
      g.gain.setValueAtTime(.045, t); g.gain.exponentialRampToValueAtTime(.0001, t + .035);
      o.connect(g).connect(actx.destination); o.start(t); o.stop(t + .045);
    } catch (_) { /* audio unavailable */ }
  }

  // ---------- estimate ----------
  function estimate(t, idx) {
    if (t.size_estimate == null) return null;
    const q = S.qv[idx];
    return q == null ? t.size_estimate : t.size_estimate * qf(q) / qf(80);
  }
  // Presets plus a trailing "…" field that opens the options dialog.
  const hasMore = t => !t.dialog && (t.controls || []).length > 0;
  const plist = t => [...(t.presets || []), ...(hasMore(t) ? ['…'] : [])];
  const isMore = (t, k) => hasMore(t) && k === (t.presets || []).length;
  const hasQuality = t => (t.controls || []).includes('quality');
  function sizeLine(t, idx) {
    const e = estimate(t, idx);
    if (e == null) return t.dialog ? 'Optionen' : '';
    const d = delta(S.inBytes, e);
    return d ? `${fmtB(e)} · ${d.t}` : fmtB(e);
  }

  // ---------- build ----------
  function reset() {
    clearTimeout(S.timer);
    secsG.innerHTML = ''; presG.innerHTML = ''; lensText.innerHTML = '';
    S.hot = null; S.key = ''; S.presetsFor = -1; S.qv = {}; S.wheelAcc = 0;
    needle.classList.remove('on'); lens.classList.remove('hot');
    stage.classList.remove('closing', 'dismiss', 'committed');
  }

  function build(sug) {
    S.files = sug.files || [];
    S.srcKind = (S.files[0] && KINDS.has(S.files[0].kind)) ? S.files[0].kind : 'document';
    S.inBytes = S.files.reduce((a, f) => a + (f.size || 0), 0);
    S.targets = (sug.ring || []).slice(0, 6);
    S.more = sug.more || [];
    const n = S.targets.length;
    S.step = 2 * Math.PI / Math.max(n, 1);
    const di = S.targets.findIndex(t => t.id === sug.default);
    S.def = di >= 0 ? di : 0;
    secsG.innerHTML = '';
    presG.innerHTML = '';

    const ga = 5 / 125;
    const rm = (R.inner + R.out) / 2;
    const maxW = Math.max(70, Math.min(150, 2 * rm * Math.sin(Math.min(S.step, Math.PI) / 2) - 12));
    S.targets.forEach((t, i) => {
      const a = i * S.step, a0 = a - S.step / 2 + ga / 2, a1 = a + S.step / 2 - ga / 2;
      const [lx, ly] = pol(rm, a).map(f1);
      const d = sector(R.inner, R.out, a0, a1, 8);
      const subs = t.sub ? wrap(t.sub, 10.5, 400, maxW, 9.5) : [], z = sizeLine(t, i);
      const rows = [{ h: 15, k: 'ic' }, { h: 18, k: 'n', ...shrink(t.title, 15, 700, maxW, 11) }];
      subs.forEach(l => rows.push({ h: subs.length > 1 ? 11.5 : 13, k: 's', v: l.v, sz: l.sz, base: 10.5 }));
      if (z) rows.push({ h: 13, k: 'z', v: z });
      const ys = stack(rows.map(r => r.h), ly, 2);
      let lbl = '';
      rows.forEach((r, j) => {
        if (r.k === 'ic') {
          lbl += `<use class="ic" href="#i-${iconOf(t)}" x="${f1(lx - 7.5)}" y="${f1(ys[j] - 7.5)}" width="15" height="15"/>`;
        }
        else lbl += `<text class="${r.k}" x="${lx}" y="${ys[j]}"${r.sz && r.sz < (r.base || 15) ? ` style="font-size:${r.sz}px"` : ''}>${esc(r.v)}</text>`;
      });
      const [dx, dy] = pol(R.out - 10, a).map(f1);
      const g = document.createElementNS(SVGNS, 'g');
      g.setAttribute('class', 'sec' + (t.intent ? ' intent' : '') + (t.ai ? ' ai' : '') + (t.chain ? ' chain' : ''));
      g.style.cssText = `--k:${colorOf(t)};--ux:${Math.sin(a).toFixed(3)};--uy:${(-Math.cos(a)).toFixed(3)};--d:${i * 30}ms`;
      g.innerHTML = `<g class="lift">
        <path class="gl" d="${d}" filter="url(#glowF)"/>
        <path class="body" d="${d}"/>
        <path class="tint" d="${d}"/>
        <path class="sheen" d="${d}" fill="url(#sheen)"/>
        <path class="spec" d="${d}" stroke="url(#gSpec)"/>
        <path class="edge" d="${d}"/>
        <path class="rim" d="${rimArc(R.out - 3, a0, a1, 8)}"/>
        ${i === S.def ? `<circle class="dflt" cx="${dx}" cy="${dy}" r="2.6"/>` : ''}
        ${lbl}</g>`;
      secsG.appendChild(g);
      t._g = g; t._z = g.querySelector('.z');
    });

    needlePath.setAttribute('d', `M${pt(66, -.2)}A66 66 0 0 1 ${pt(66, .2)}`);
    lens.style.setProperty('--k', S.targets[S.def] ? colorOf(S.targets[S.def]) : 'var(--accent)');
    renderLens(true);
  }

  // ---------- lens ----------
  function lensName() {
    if (S.files.length > 1) return `${S.files.length} Dateien`;
    return S.files[0] ? fitName(S.files[0].name, 13, 700, 104) : '';
  }
  function renderLens(animate) {
    const t = S.hot, tg = S.targets;
    let rows = [];
    const tx = (cls, v, h, sh) => ({ cls, v: sh ? sh.v : v, h, sz: sh && sh.sz < 21 ? sh.sz : 0 });
    const subRows = sub => (sub ? wrap(sub, 10, 400, 100, 8.5) : []).map(l => tx('l-m', l.v, 11.5, l));
    if (!tg.length) {
      rows = [tx('l-m', S.waiting ? 'Wird gelesen …' : 'Keine Umwandlung', 14)];
    } else if (!t || t.type === 'none') {
      const f = S.files[0] || {}, parts = (f.meta || '').split(' · ').filter(Boolean);
      const total = fmtB(S.inBytes);
      const l1 = parts[0] || total;
      const l2 = parts.length ? [...parts.slice(1), total].join(' · ') : '';
      rows = [tx('l-name', lensName(), 16), tx('l-m', l1, 12, shrink(l1, 10, 400, 112, 8.5))];
      if (l2) rows.push(tx('l-m', l2, 12, shrink(l2, 10, 400, 112, 8.5)));
      const mid = `Mitte: ${tg[S.def].title}`;
      rows.push(tx('l-g', mid, 14, shrink(mid, 10.5, 650, 100, 8.5)));
      if (S.files.some(x => x.readable === false)) rows.push(tx('l-warn', 'Ordnerfreigabe fehlt', 12));
    } else if (t.type === 'center') {
      const o = tg[S.def];
      rows = [tx('l-m', 'Vorschlag', 12), tx('l-big', o.title, 25, shrink(o.title, 21, 800, 108, 12)), ...subRows(o.sub)];
      const z = sizeLine(o, S.def);
      if (z) rows.push(tx('l-g', z, 14));
    } else {
      const o = tg[t.idx], e = estimate(o, t.idx), d = delta(S.inBytes, e);
      rows = [tx('l-big', o.title, 25, shrink(o.title, 21, 800, 108, 12))];
      if (t.type === 'pre') rows.push(tx('l-name', isMore(o, t.k) ? 'Optionen' : o.presets[t.k], 15));
      if (e != null) {
        rows.push(tx('l-m', `${fmtB(S.inBytes)} → ${fmtB(e)}`, 12));
        if (d) rows.push(tx(d.up ? 'l-up' : 'l-ok', d.t, 15));
      } else {
        if (o.dialog) rows.push(tx('l-m', 'Mit Optionen', 12)); else rows.push(...subRows(o.sub));
      }
      if (t.type === 'sec' && hasQuality(o)) rows.push(tx('l-m', `Qualität ${S.qv[t.idx] ?? 80} % (Mausrad)`, 12, { v: `Qualität ${S.qv[t.idx] ?? 80} % (Mausrad)`, sz: 9 }));
    }
    const ys = stack(rows.map(r => r.h), 0, 1.5);
    lensText.innerHTML = `<g class="ll${animate ? ' swap' : ''}">` +
      rows.map((r, i) => `<text class="${r.cls}" x="0" y="${ys[i]}"${r.sz ? ` style="font-size:${r.sz}px"` : ''}>${esc(r.v)}</text>`).join('') + '</g>';
  }

  // ---------- presets ----------
  function showPresets(idx) {
    if (!S.active) return;
    const t = S.targets[idx];
    const labels = plist(t);
    if (!t || !labels.length) return;
    S.presetsFor = idx;
    presG.innerHTML = '';
    const a = idx * S.step, np = labels.length, span = presetSpan(np), sub = span / np, ga = 5 / 217;
    labels.forEach((label, k) => {
      const a0 = a - span / 2 + k * sub + ga / 2, a1 = a - span / 2 + (k + 1) * sub - ga / 2;
      const d = sector(R.pin, R.pout, a0, a1, 6);
      const am = (a0 + a1) / 2;
      const [lx, ly] = pol((R.pin + R.pout) / 2, am).map(f1);
      // Horizontal text runs along the arc at the top/bottom but across the band at the sides.
      const along = (R.pin + R.pout) / 2 * (a1 - a0) - 6, across = R.pout - R.pin - 10;
      const avail = Math.max(28, Math.min(72, Math.abs(Math.cos(am)) * along + Math.abs(Math.sin(am)) * across));
      let pf = shrink(label, 11, 650, avail, label.includes(' ') ? 9 : 7.5);
      let lines = null;
      if (pf.sz < 11 && label.includes(' ')) {
        const mid = label.length / 2;
        const cut = [...label].reduce((best, ch, i) => ch === ' ' && Math.abs(i - mid) < Math.abs(best - mid) ? i : best, label.indexOf(' '));
        lines = [label.slice(0, cut), label.slice(cut + 1)].map(l => shrink(l, 10.5, 650, avail, 8.5));
      }
      const g = document.createElementNS(SVGNS, 'g');
      g.setAttribute('class', 'pre' + (isMore(t, k) ? ' more' : ''));
      g.style.cssText = `--k:${colorOf(t)};--d:${k * 40}ms`;
      g.innerHTML = `<path class="pgl" d="${d}" filter="url(#glowS)"/><path class="pb" d="${d}"/><path class="pt" d="${d}"/>` +
        (lines
          ? `<text x="${lx}" y="${ly}" style="font-size:${Math.min(lines[0].sz, lines[1].sz)}px"><tspan x="${lx}" dy="-0.58em">${esc(lines[0].v)}</tspan><tspan x="${lx}" dy="1.16em">${esc(lines[1].v)}</tspan></text>`
          : `<text x="${lx}" y="${ly}"${pf.sz < 11 ? ` style="font-size:${pf.sz}px"` : ''}>${esc(pf.v)}</text>`);
      presG.appendChild(g);
    });
    tick(1900);
  }
  // The outer band has more room than the sector, so presets may reach over the neighbours.
  const presetSpan = np => Math.min(Math.max(S.step, np * .34), Math.PI * 1.2);
  function hidePresets() { S.presetsFor = -1; presG.innerHTML = ''; }

  // ---------- hit testing ----------
  function hit(x, y) {
    const n = S.targets.length;
    if (!n) return { type: 'none' };
    const dx = x - CENTER, dy = y - CENTER, d = Math.hypot(dx, dy);
    if (d < R.lens + 4) return { type: 'center' };
    const a = norm(Math.atan2(dx, -dy));
    const idx = Math.round(a / S.step) % n;
    if (S.presetsFor >= 0 && d >= R.pin - 6 && d <= R.pout + 16) {
      const pi = S.presetsFor, np = plist(S.targets[pi]).length, span = presetSpan(np);
      const rel = norm(a - pi * S.step + Math.PI) - Math.PI;
      if (Math.abs(rel) <= span / 2 + .02) {
        const k = Math.min(np - 1, Math.max(0, Math.floor((rel + span / 2) / span * np)));
        return { type: 'pre', idx: pi, k };
      }
    }
    if (d > R.out + 10) return { type: 'none' };
    return { type: 'sec', idx };
  }

  function setTarget(t) {
    const key = t.type === 'sec' ? `s${t.idx}` : t.type === 'pre' ? `p${t.idx}.${t.k}` : t.type;
    if (key === S.key) return;
    const prev = S.hot;
    S.key = key; S.hot = t;
    clearTimeout(S.timer);

    const idx = (t.type === 'sec' || t.type === 'pre') ? t.idx : -1;
    S.targets.forEach((o, i) => o._g.classList.toggle('hot', i === idx));
    lens.classList.toggle('hot', t.type === 'center');
    presG.querySelectorAll('.pre').forEach((p, k) => p.classList.toggle('hot', t.type === 'pre' && t.k === k));

    const colorT = t.type === 'center' ? S.targets[S.def] : (idx >= 0 ? S.targets[idx] : null);
    if (colorT) lens.style.setProperty('--k', colorOf(colorT));
    else if (S.targets[S.def]) lens.style.setProperty('--k', colorOf(S.targets[S.def]));

    // The bead sits on the lens edge pointing at the hovered sector.
    if (idx >= 0) {
      const target = idx * S.step * 180 / Math.PI;
      S.needleAngle += ((target - S.needleAngle) % 360 + 540) % 360 - 180;
      needle.style.transform = `rotate(${S.needleAngle}deg)`;
    }
    needle.classList.toggle('on', idx >= 0);

    if (t.type === 'sec') {
      const o = S.targets[t.idx];
      if (S.presetsFor !== -1 && S.presetsFor !== t.idx) hidePresets();
      if (plist(o).length && S.presetsFor !== t.idx) {
        const i = t.idx; S.timer = setTimeout(() => showPresets(i), 380);
      }
    } else if (t.type !== 'pre') {
      hidePresets();
    }
    if (!prev || prev.idx !== t.idx || prev.type !== t.type) tick(t.type === 'pre' ? 2800 : 2300);
    else if (t.type === 'pre') tick(2800);
    renderLens(true);
  }

  function hover(x, y) { if (S.active) setTarget(hit(x, y)); }

  // ---------- wheel ----------
  window.addEventListener('wheel', e => {
    const t = S.hot;
    if (!S.active || !t || t.type !== 'sec') return;
    const o = S.targets[t.idx];
    if (!hasQuality(o)) return;
    e.preventDefault();
    S.wheelAcc += e.deltaY;
    const steps = Math.trunc(Math.abs(S.wheelAcc) / 20);
    if (!steps) return;
    const dir = S.wheelAcc < 0 ? 1 : -1;
    S.wheelAcc = 0;
    const cur = S.qv[t.idx] ?? 80;
    const q = Math.min(100, Math.max(10, cur + dir * 5 * steps));
    if (q === cur) return;
    S.qv[t.idx] = q;
    if (o._z) o._z.textContent = sizeLine(o, t.idx);
    tick(1200 + q * 15);
    renderLens(false);
  }, { passive: false });

  // ---------- drop ----------
  async function drop(paths, pos) {
    // The watcher often sees the mouse release before WebKit delivers the drop.
    if (!S.active && !(S.graceUntil && performance.now() < S.graceUntil)) return;
    S.graceUntil = 0;
    clearTimeout(S.timer);
    const t = pos ? hit(pos.x, pos.y) : (S.hot || { type: 'none' });
    paths = (paths && paths.length) ? paths : S.paths;
    const idx = t.type === 'center' ? S.def : t.idx;
    const o = (t.type === 'none' || idx == null) ? null : S.targets[idx];
    invoke('debug_log', { msg: `drop hit=${JSON.stringify(t)} target=${o && o.id}` }).catch(() => {});
    if (!o) { try { await invoke('ring_hide'); } catch (_) { /* ignore */ } return; }
    const more = t.type === 'pre' && isMore(o, t.k);
    const preset = t.type === 'pre' && !more ? o.presets[t.k] : undefined;
    const options = {};
    if (S.qv[idx] != null && t.type !== 'center') options.quality = S.qv[idx];
    stage.classList.add('committed');
    secsG.children[idx] && secsG.children[idx].classList.add('picked');
    tick(3200);
    try {
      if (more) await invoke('open_dialog', { paths, target: o.id });
      else if (o.dialog) await invoke('open_dialog', { paths, target: o.id, preset });
      else await invoke('convert', { paths, target: o.id, preset, options });
    } catch (err) { console.error('convert failed', err); invoke('debug_log', { msg: `convert failed: ${err}` }).catch(() => {}); }
    try { await invoke('ring_hide'); } catch (_) { /* ignore */ }
  }

  // ---------- open / close ----------
  function applySettings(s) {
    if (!s) return;
    if (s.style) {
      document.body.classList.toggle('style-glass', s.style === 'glass');
      document.body.classList.toggle('style-classic', s.style !== 'glass');
    }
    if (typeof s.reduce_motion === 'boolean') document.body.classList.toggle('reduce-motion', s.reduce_motion);
    if (typeof s.sound === 'boolean') S.sound = s.sound;
  }

  function open(p) {
    clearTimeout(S.closeTimer);
    S.token++;
    reset(); S.loading = false;
    applySettings({ style: p.style, reduce_motion: p.reduce_motion, sound: p.sound });
    S.active = true; S.paths = p.files || [];
    S.mode = p.mode === 'click' ? 'click' : 'drag'; S.committed = false; S.openedAt = performance.now();
    // Restart the stage entrance animation.
    stage.style.animation = 'none'; void stage.offsetWidth; stage.style.animation = '';
    if (p.suggestions) { S.waiting = false; S.pendingOpen = null; build(p.suggestions); }
    else { S.waiting = true; S.pendingOpen = p; S.targets = []; S.files = []; renderLens(true); if (S.paths.length) loadSuggest(S.paths); }
    tick(1500);
  }

  async function loadSuggest(paths) {
    const tok = S.token;
    S.waiting = true; S.loading = true;
    try {
      const sug = await invoke('suggest', { paths });
      if (tok !== S.token || !S.active) return;
      S.paths = paths; S.waiting = false;
      build(sug);
      if (S.lastPos) hover(S.lastPos.x, S.lastPos.y);
    } catch (err) { console.error('suggest failed', err); S.waiting = false; renderLens(true); }
    finally { if (tok === S.token) S.loading = false; }
  }

  function close(reason) {
    if (!S.active) return;
    S.active = false; clearTimeout(S.timer);
    S.graceUntil = reason === 'released' ? performance.now() + 450 : 0;
    stage.classList.remove('closing', 'dismiss');
    stage.classList.add(reason === 'cancelled' ? 'dismiss' : 'closing');
    const tok = S.token;
    S.closeTimer = setTimeout(() => { if (tok === S.token) reset(); }, 500);
  }

  // ---------- click mode (context menu entry) ----------
  // Rust polls the pointer and the primary button while no drag is running and sends them as events.
  function clickMove(x, y) {
    S.lastPos = { x, y };
    hover(x, y);
  }
  function clickCommit(x, y) {
    if (!S.active || S.committed || hit(x, y).type === 'none') return;
    S.committed = true;
    drop(S.paths, { x, y });
  }
  function clickDismiss() {
    if (!S.active) return;
    close('cancelled');
    invoke('ring_hide').catch(() => {});
  }
  listen('ring:pointer', e => { if (S.mode === 'click' && e.payload) clickMove(e.payload.x, e.payload.y); });
  listen('ring:click', e => { if (S.mode === 'click' && e.payload) clickCommit(e.payload.x, e.payload.y); });
  window.addEventListener('pointermove', e => { if (S.mode === 'click') clickMove(e.clientX, e.clientY); });
  window.addEventListener('keydown', e => { if (S.mode === 'click' && e.key === 'Escape') clickDismiss(); });
  window.addEventListener('blur', () => { if (S.mode === 'click' && S.active && performance.now() - S.openedAt > 500) clickDismiss(); });

  // ---------- wiring ----------
  listen('ring:open', e => open(e.payload || {}));
  listen('ring:close', e => close(e.payload && e.payload.reason));
  listen('settings', e => applySettings(e.payload));

  T.webview.getCurrentWebview().onDragDropEvent(e => {
    const p = e.payload;
    if (!p) return;
    // wry reports points on macOS but real physical pixels on Windows.
    const scale = IS_MAC ? 1 : (window.devicePixelRatio || 1);
    const pos = p.position ? { x: p.position.x / scale, y: p.position.y / scale } : null;
    if (pos) S.lastPos = pos;
    if (p.type !== 'over') invoke('debug_log', { msg: `ring ${p.type} active=${S.active} targets=${S.targets.length} pos=${JSON.stringify(pos)}` }).catch(() => {});
    if (window.ORBO_DEBUG === true) invoke('debug_log', { msg: `${p.type} raw=${JSON.stringify(p.position)} pos=${JSON.stringify(pos)} dpr=${window.devicePixelRatio} inner=${innerWidth}x${innerHeight}` }).catch(() => {});
    if (p.type === 'enter') {
      if (p.paths && p.paths.length) S.paths = p.paths;
      if (S.active && !S.targets.length && !S.loading && p.paths && p.paths.length) loadSuggest(p.paths);
      if (pos) hover(pos.x, pos.y);
    } else if (p.type === 'over') {
      if (pos) hover(pos.x, pos.y);
    } else if (p.type === 'drop') {
      drop(p.paths, pos);
    } else if (p.type === 'leave') {
      if (S.active && S.targets.length) setTarget({ type: 'none' });
    }
  });

  invoke('settings_get').then(applySettings).catch(() => {});
})();
