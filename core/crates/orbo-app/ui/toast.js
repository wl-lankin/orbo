(() => {
  'use strict';
  const T = window.__TAURI__;
  const { invoke } = T.core;
  const { listen } = T.event;
  const stack = document.getElementById('stack');
  const cards = new Map();
  const MAX_CARDS = 3;
  const MB = 1048576, KB = 1024;

  const EXT_KIND = {
    image: 'png jpg jpeg webp avif heic gif svg ico bmp tiff', video: 'mp4 mov mkv webm avi hevc m4v',
    audio: 'mp3 m4a wav flac opus ogg aac aiff', document: 'pdf docx odt txt md html srt rtf',
    data: 'csv xlsx json sqlite xml', archive: 'zip 7z tar gz rar',
  };
  const KIND_BY_EXT = {};
  Object.entries(EXT_KIND).forEach(([k, v]) => v.split(' ').forEach(e => { KIND_BY_EXT[e] = k; }));
  const extOf = p => ((p || '').split('.').pop() || '').toLowerCase();
  const baseName = p => (p || '').split(/[\\/]/).pop();
  const kindFor = (target, input, output) => KIND_BY_EXT[extOf(output)] || KIND_BY_EXT[String(target || '').toLowerCase()] || KIND_BY_EXT[extOf(input)] || 'document';
  const esc = s => String(s).replace(/[&<>"]/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));

  function fmtB(b) {
    if (b < MB) return Math.max(1, Math.round(b / KB)).toLocaleString('de-DE') + ' KB';
    if (b < 1024 * MB) return (b / MB).toLocaleString('de-DE', { maximumFractionDigits: b < 10 * MB ? 1 : 0 }) + ' MB';
    return (b / 1024 / MB).toLocaleString('de-DE', { maximumFractionDigits: 1 }) + ' GB';
  }

  // Animate existing cards to their new position after the stack changed.
  function flip(change) {
    const before = new Map([...cards].map(([id, c]) => [id, c.wrap.getBoundingClientRect().top]));
    change();
    cards.forEach((c, id) => {
      if (!before.has(id)) return;
      const dy = before.get(id) - c.wrap.getBoundingClientRect().top;
      if (Math.abs(dy) < 1) return;
      c.wrap.style.transition = 'none';
      c.wrap.style.transform = `translateY(${dy}px)`;
      void c.wrap.offsetWidth;
      c.wrap.style.transition = '';
      c.wrap.style.transform = '';
    });
  }

  function create(id, input, target, title) {
    const kind = kindFor(target, input);
    const wrap = document.createElement('div');
    wrap.className = 'wrap';
    wrap.style.transition = 'height .3s var(--ease-out), margin-top .3s var(--ease-out), opacity .24s, transform .4s var(--ease-spring)';
    const el = document.createElement('div');
    el.className = 'card running indet';
    el.style.setProperty('--k', `var(--${kind})`);
    el.innerHTML = `
      <button class="ico" title="Abbrechen" aria-label="Abbrechen">
        <svg class="pr" viewBox="0 0 40 40"><circle class="trk" cx="20" cy="20" r="17"/><circle class="arc" cx="20" cy="20" r="17" pathLength="100"/></svg>
        <span class="ring-pulse"></span>
        <svg class="g main"><use href="#i-${kind}"/></svg><svg class="g alt"><use href="#i-x"/></svg>
      </button>
      <div class="mid"><div class="t"></div><div class="sub"></div></div>
      <div class="act"></div>
      <div class="more" hidden></div>`;
    wrap.appendChild(el);
    const c = { id, wrap, el, kind, input, target, title, output: null, state: 'running', expanded: false };
    el.querySelector('.t').textContent = `${baseName(input)} → ${title || String(target).toUpperCase()}`;
    el.querySelector('.sub').innerHTML = '<span class="nm">Wird vorbereitet …</span>';
    el.querySelector('.ico').addEventListener('click', () => { if (c.state === 'running') invoke('cancel_job', { id }).catch(() => {}); });
    flip(() => { stack.appendChild(wrap); cards.set(id, c); });
    trim(c);
    return c;
  }

  function trim(keep) {
    while (cards.size > MAX_CARDS) {
      const old = [...cards.values()].find(c => c !== keep && c.state !== 'running');
      if (!old) break;
      dismiss(old);
    }
  }

  // An expanded error needs space: drop older finished cards until it fits.
  function makeRoom(keep) {
    const room = () => [...cards.values()].reduce((a, x) => a + x.wrap.offsetHeight + 8, 0) + keep.el.querySelector('.more').offsetHeight;
    while (room() > stack.clientHeight - 20) {
      const old = [...cards.values()].find(x => x !== keep && x.state !== 'running');
      if (!old) break;
      cards.delete(old.id); old.wrap.remove();
    }
  }
  function setSub(c, html) { c.el.querySelector('.sub').innerHTML = html; }
  function setActions(c, defs) {
    const act = c.el.querySelector('.act');
    act.innerHTML = '';
    defs.forEach(([label, fn, cls, svg]) => {
      const b = document.createElement('button');
      if (cls) b.className = cls;
      if (svg) b.innerHTML = svg; else b.textContent = label;
      if (svg) b.setAttribute('aria-label', label);
      b.addEventListener('click', e => { e.stopPropagation(); fn(b); });
      act.appendChild(b);
    });
  }
  function startDrain(c) {
    c.el.querySelector('.drain')?.remove();
    const d = document.createElement('div');
    d.className = 'drain';
    d.addEventListener('animationend', e => { if (e.animationName === 'drain') dismiss(c); });
    c.el.appendChild(d);
  }
  function setIcon(c, name) { c.el.querySelector('.g.main use').setAttribute('href', `#i-${name}`); }
  function setProgress(c, f) {
    c.el.classList.remove('indet');
    const v = Math.min(1, Math.max(0, f));
    c.el.querySelector('.arc').style.strokeDashoffset = String(100 * (1 - v));
    const pct = c.el.querySelector('.pct');
    if (pct) pct.textContent = `${Math.round(v * 100)} %`;
    else setSub(c, `<span class="nm">Wird umgewandelt</span><span class="pct">${Math.round(v * 100)} %</span>`);
  }

  function dismiss(c) {
    if (!cards.has(c.id)) return;
    cards.delete(c.id);
    c.wrap.style.height = c.wrap.offsetHeight + 'px';
    void c.wrap.offsetWidth;
    c.wrap.classList.add('out');
    c.wrap.style.height = '0px';
    c.wrap.style.marginTop = '0px';
    c.el.style.transition = 'transform .3s var(--ease-in)';
    c.el.style.transform = 'translateY(10px) scale(.96)';
    setTimeout(() => {
      c.wrap.remove();
      if (cards.size === 0) invoke('toast_hide').catch(() => {});
    }, 340);
  }

  function finish(c, p) {
    c.state = 'done'; c.output = p.output;
    c.kind = kindFor(c.target, c.input, p.output);
    c.el.style.setProperty('--k', `var(--${c.kind})`);
    c.el.classList.remove('running', 'indet');
    c.el.classList.add('done');
    c.el.querySelector('.arc').style.strokeDashoffset = '0';
    setIcon(c, 'check');
    c.el.querySelector('.ico').title = ''; c.el.querySelector('.ico').disabled = true;
    c.el.querySelector('.ring-pulse').style.animation = 'none';
    void c.el.offsetWidth; c.el.querySelector('.ring-pulse').style.animation = '';
    let dl = '';
    if (p.input_bytes) {
      const pc = Math.max(-99, Math.round((p.bytes / p.input_bytes - 1) * 100));
      dl = pc < 0 ? `<span class="dl ok">−${-pc} %</span>` : `<span class="dl up">+${pc} %</span>`;
    }
    setSub(c, `<span class="nm" title="${esc(baseName(p.output))}">${esc(baseName(p.output))}</span><span class="sz">${fmtB(p.bytes)}</span>${dl}`);
    setActions(c, [
      ['Anzeigen', () => invoke('reveal', { path: p.output }).catch(() => {})],
      ['Rückgängig', () => undo(c)],
    ]);
    startDrain(c);
  }

  async function undo(c) {
    try { await invoke('undo', { path: c.output }); } catch (_) { /* file may already be gone */ }
    c.state = 'undone';
    c.el.classList.remove('done'); c.el.classList.add('undone');
    setIcon(c, 'undo');
    setSub(c, '<span class="nm">Rückgängig gemacht</span>');
    setActions(c, []);
    startDrain(c);
  }

  function fail(c, error) {
    c.state = 'failed';
    c.el.classList.remove('running', 'indet'); c.el.classList.add('failed');
    c.el.querySelector('.ico').disabled = true;
    setIcon(c, 'alert');
    const full = String(error || 'Unbekannter Fehler');
    const first = full.split('\n')[0].split(/(?<=[.!?])\s/)[0].replace(/\.$/, '');
    setSub(c, `<span class="err">${esc(first.length > 70 ? first.slice(0, 68) + '…' : first)}</span>`);
    const more = c.el.querySelector('.more');
    more.textContent = full;
    const toggle = () => {
      c.expanded = !c.expanded; more.hidden = !c.expanded;
      c.el.classList.toggle('hold', c.expanded);
      btn.textContent = c.expanded ? 'Weniger' : 'Details';
      if (c.expanded) makeRoom(c);
    };
    let btn;
    setActions(c, [['Details', toggle], ['Schließen', () => dismiss(c), 'sq', '<svg viewBox="0 0 16 16"><path d="M4 4l8 8M12 4l-8 8"/></svg>']]);
    btn = c.el.querySelector('.act button');
    startDrain(c);
  }

  // A plain notice without a job, e.g. "Orbo wurde auf Version X aktualisiert".
  listen('info', e => {
    const p = e.payload;
    if (!p || !p.title) return;
    const id = 'info-' + Date.now();
    const wrap = document.createElement('div');
    wrap.className = 'wrap';
    wrap.style.transition = 'height .3s var(--ease-out), margin-top .3s var(--ease-out), opacity .24s, transform .4s var(--ease-spring)';
    const el = document.createElement('div');
    el.className = 'card done';
    el.style.setProperty('--k', 'var(--ok)');
    el.innerHTML = `
      <div class="ico">
        <svg class="pr" viewBox="0 0 40 40"><circle class="trk" cx="20" cy="20" r="17"/><circle class="arc" cx="20" cy="20" r="17" pathLength="100" style="stroke-dashoffset:0"/></svg>
        <svg class="g main"><use href="#i-check"/></svg>
      </div>
      <div class="mid"><div class="t"></div><div class="sub"></div></div>
      <div class="act"></div>
      <div class="more" hidden></div>`;
    el.querySelector('.t').textContent = p.title;
    if (p.sub) el.querySelector('.sub').textContent = p.sub;
    wrap.appendChild(el);
    const c = { id, wrap, el, kind: 'document', state: 'done', expanded: false };
    flip(() => { stack.appendChild(wrap); cards.set(id, c); });
    trim(c);
    setActions(c, [['Schließen', () => dismiss(c), 'sq', '<svg viewBox="0 0 16 16"><path d="M4 4l8 8M12 4l-8 8"/></svg>']]);
    startDrain(c);
  });

  listen('job', e => {
    const p = e.payload;
    if (!p) return;
    let c = cards.get(p.id);
    if (p.event === 'started') {
      if (c) return;
      create(p.id, p.input, p.target, p.title);
      return;
    }
    if (!c) {
      if (p.event === 'cancelled') return;
      c = create(p.id, p.input || '', p.target || '', p.title || '');
    }
    if (p.event === 'progress') { if (c.state === 'running') setProgress(c, p.fraction || 0); }
    else if (p.event === 'finished') finish(c, p);
    else if (p.event === 'failed') fail(c, p.error);
    else if (p.event === 'cancelled') dismiss(c);
  });

  function applySettings(s) {
    if (!s) return;
    if (s.style) {
      document.body.classList.toggle('style-glass', s.style === 'glass');
      document.body.classList.toggle('style-classic', s.style !== 'glass');
    }
    if (typeof s.reduce_motion === 'boolean') document.body.classList.toggle('reduce-motion', s.reduce_motion);
  }
  listen('settings', e => applySettings(e.payload));
  invoke('settings_get').then(applySettings).catch(() => {});
})();
