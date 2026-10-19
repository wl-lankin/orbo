(async function () {
  const { invoke } = window.__TAURI__.core;
  const { listen } = window.__TAURI__.event;
  const $ = id => document.getElementById(id);
  const $$ = (s, r = document) => [...r.querySelectorAll(s)];

  let S = await invoke('settings_get');
  S.output = S.output || { mode: 'beside', folder: null, naming: 'suffix' };
  S.ollama = S.ollama || { enabled: false, url: 'http://localhost:11434', model: null, actions: [] };

  /* ---------- persistence ---------- */
  let timer = null;
  const save = () => { clearTimeout(timer); timer = setTimeout(() => invoke('settings_set', { settings: S }).catch(() => {}), 200); };
  const change = fn => { fn(); render(); save(); };

  /* ---------- tabs ---------- */
  const tabs = $$('.tab'), panes = $$('.pane'), pill = $('pill');
  function movePill() {
    const on = tabs.find(t => t.classList.contains('on'));
    pill.style.transform = `translateY(${on.offsetTop}px)`;
  }
  function openTab(id) {
    tabs.forEach(t => t.classList.toggle('on', t.dataset.tab === id));
    panes.forEach(p => p.classList.toggle('on', p.dataset.pane === id));
    $$('.content')[0].scrollTop = 0;
    movePill();
    if (id === 'ai' && S.ollama.enabled && status === 'idle') testOllama();
  }
  tabs.forEach(t => t.addEventListener('click', () => openTab(t.dataset.tab)));

  /* ---------- helpers ---------- */
  const bindSeg = (root, get, set) => $$('[data-v]', root).forEach(b => b.addEventListener('click', () => change(() => set(b.dataset.v))));
  const bindToggle = (el, get, set) => el.addEventListener('change', () => change(() => set(el.checked)));
  const mark = (root, v) => $$('[data-v]', root).forEach(b => b.classList.toggle('on', b.dataset.v === v));

  bindSeg($('themes'), null, v => { S.style = v; });
  bindToggle($('sound'), null, v => { S.sound = v; });
  bindToggle($('reduce'), null, v => { S.reduce_motion = v; });
  bindSeg($('keys'), null, v => { S.trigger_key = v; });
  bindToggle($('autostart'), null, v => { S.autostart = v; });
  bindToggle($('autoupdate'), null, v => { S.auto_update = v; });
  $('intro').addEventListener('click', () => invoke('open_onboarding').catch(() => {}));
  bindToggle($('shake'), null, v => { S.shake = v; });
  bindToggle($('notch'), null, v => { S.notch = v; });
  bindSeg($('mode'), null, v => { S.output.mode = v; });
  bindSeg($('naming'), null, v => { S.output.naming = v; });
  $('folder').addEventListener('input', e => { S.output.folder = e.target.value.trim() || null; save(); });
  $('pick').addEventListener('click', async () => {
    const dlg = window.__TAURI__.dialog;
    if (dlg && dlg.open) {
      const p = await dlg.open({ directory: true, multiple: false });
      if (p) change(() => { S.output.folder = p; S.output.mode = 'folder'; });
    } else $('folder').focus();
  });

  bindToggle($('ollama'), null, v => { S.ollama.enabled = v; if (v) status = 'idle'; });
  $('url').addEventListener('input', e => { S.ollama.url = e.target.value.trim(); status = 'idle'; models = []; render(true); save(); });
  $('remote').addEventListener('change', e => { S.ollama.allow_remote = e.target.checked; save(); });
  $('model').addEventListener('change', e => { S.ollama.model = e.target.value || null; save(); });
  $$('#actions [data-a]').forEach(c => c.addEventListener('change', () => {
    const set = new Set(S.ollama.actions || []); c.checked ? set.add(c.dataset.a) : set.delete(c.dataset.a);
    S.ollama.actions = [...set]; save();
  }));

  /* ---------- Ollama ---------- */
  let status = 'idle', models = [];
  // Mirrors orbo_core::ai::server_scope.
  function scopeOf(u) {
    let h;
    try { h = new URL(u).hostname.toLowerCase().replace(/^\[|\]$/g, ''); } catch (_) { return 'local'; }
    if (h === 'localhost' || h === '::1' || /^127(\.\d{1,3}){3}$/.test(h)) return 'local';
    const v4 = h.match(/^(\d{1,3})\.(\d{1,3})\.\d{1,3}\.\d{1,3}$/);
    if (v4) {
      const a = +v4[1], b = +v4[2];
      return a === 10 || (a === 172 && b >= 16 && b <= 31) || (a === 192 && b === 168) || (a === 169 && b === 254) || (a === 100 && b >= 64 && b <= 127) ? 'private' : 'public';
    }
    if (h.includes(':')) return /^f[cd]|^fe[89ab]/.test(h) ? 'private' : 'public';
    return !h.includes('.') || ['.local', '.lan', '.home', '.home.arpa', '.internal', '.fritz.box', '.netbird.cloud', '.ts.net'].some(s => h.endsWith(s)) ? 'private' : 'public';
  }
  async function testOllama() {
    status = 'busy'; models = []; render();
    const ctrl = new AbortController(), to = setTimeout(() => ctrl.abort(), 2000);
    try {
      const r = await fetch(S.ollama.url.replace(/\/+$/, '') + '/api/tags', { signal: ctrl.signal });
      if (!r.ok) throw new Error(r.status);
      const j = await r.json();
      models = (j.models || []).map(m => m.name);
      status = 'ok';
      if (models.length && !models.includes(S.ollama.model)) { S.ollama.model = models[0]; save(); }
    } catch (_) { status = 'bad'; models = []; }
    clearTimeout(to);
    render();
  }
  $('test').addEventListener('click', testOllama);

  /* ---------- Watched folders ---------- */
  S.watch = S.watch || [];
  let wTargets = null, wStatus = {}, pick = null;
  const isMac = /Mac/i.test(navigator.platform);
  const baseName = p => p.replace(/[\\/]+$/, '').split(/[\\/]/).pop() || p;
  const loadTargets = async () => { if (!wTargets) { try { wTargets = await invoke('watch_targets'); } catch (_) { wTargets = []; } } return wTargets; };
  const targetOf = id => (wTargets || []).find(t => t.id === id);
  async function refreshStatus() {
    if (!S.watch.length) { wStatus = {}; return; }
    try { wStatus = Object.fromEntries((await invoke('watch_status')).map(x => [x.id, x])); } catch (_) {}
    renderWatch();
  }
  function renderWatch() {
    const list = $('wList');
    $('wEmpty').hidden = S.watch.length > 0 || !!pick;
    list.hidden = !S.watch.length;
    $('wMacNote').hidden = !isMac;
    list.textContent = '';
    S.watch.forEach(r => {
      const row = document.createElement('div');
      row.className = 'row wrow' + (r.enabled ? '' : ' off');
      const lbl = document.createElement('span'); lbl.className = 'lbl';
      const b = document.createElement('b'); b.textContent = baseName(r.folder);
      const path = document.createElement('small'); path.textContent = r.folder;
      lbl.append(b, path);
      const st = wStatus[r.id];
      if (st && (!st.exists || !st.readable)) {
        const w = document.createElement('small'); w.className = 'warnt';
        if (!st.exists) w.textContent = 'Ordner nicht gefunden';
        else {
          w.append(st.protected ? 'Kein Zugriff auf diesen Ordner. ' : 'Kein Zugriff auf diesen Ordner.');
          if (st.protected) {
            const a = document.createElement('a'); a.href = '#'; a.textContent = 'Ordnerfreigabe prüfen';
            a.addEventListener('click', async e => { e.preventDefault(); try { await invoke('request_folder_access'); } catch (_) {} try { await invoke('open_privacy_settings'); } catch (_) {} refreshStatus(); });
            w.append(a);
          }
        }
        lbl.append(w);
      }
      const tgt = document.createElement('span'); tgt.className = 'wtarget';
      const t = targetOf(r.target);
      tgt.textContent = (t ? t.title : r.target) + (r.preset ? ' · ' + r.preset : '');
      const tg = document.createElement('input'); tg.type = 'checkbox'; tg.className = 'tg'; tg.checked = !!r.enabled;
      tg.setAttribute('aria-label', 'Ordner überwachen');
      tg.addEventListener('change', () => change(() => { r.enabled = tg.checked; }));
      const del = document.createElement('button'); del.type = 'button'; del.className = 'wdel'; del.title = 'Entfernen'; del.setAttribute('aria-label', 'Ordner entfernen');
      del.innerHTML = '<svg viewBox="0 0 20 20"><path d="M5 5l10 10M15 5L5 15"/></svg>';
      del.addEventListener('click', () => change(() => { S.watch = S.watch.filter(x => x.id !== r.id); }));
      row.append(lbl, tgt, tg, del);
      list.append(row);
    });
  }
  function renderTargets() {
    const box = $('wTargets'), q = $('wSearch').value.trim().toLowerCase();
    box.textContent = '';
    const groups = new Map();
    (wTargets || []).filter(t => !q || (t.title + ' ' + t.sub + ' ' + t.group + ' ' + t.id).toLowerCase().includes(q))
      .forEach(t => { if (!groups.has(t.group)) groups.set(t.group, []); groups.get(t.group).push(t); });
    if (!groups.size) { const n = document.createElement('div'); n.className = 'wnone'; n.textContent = 'Kein passendes Ziel gefunden.'; box.append(n); }
    groups.forEach((items, g) => {
      const h = document.createElement('h4'); h.textContent = g; box.append(h);
      items.forEach(t => {
        const b = document.createElement('button'); b.type = 'button'; b.className = 'wt' + (pick.target === t.id ? ' on' : '');
        const n = document.createElement('span'); n.textContent = t.title;
        const sm = document.createElement('small'); sm.textContent = t.sub || '';
        b.append(n, sm);
        b.addEventListener('click', () => {
          pick.target = t.id; pick.preset = null;
          $$('.wt', box).forEach(x => x.classList.toggle('on', x === b));
          const sel = $('wPreset'); sel.textContent = '';
          if (t.presets && t.presets.length) { sel.add(new Option('Standard', '')); t.presets.forEach(p => sel.add(new Option(p, p))); }
          $('wPresetRow').hidden = !(t.presets && t.presets.length);
          $('wConfirm').disabled = false;
        });
        box.append(b);
      });
    });
  }
  function closePick() { pick = null; $('wPick').hidden = true; $('wBar').hidden = false; renderWatch(); }
  $('wAdd').addEventListener('click', async () => {
    const dlg = window.__TAURI__.dialog;
    if (!dlg || !dlg.open) return;
    const folder = await dlg.open({ directory: true, multiple: false });
    if (!folder) return;
    wTargets = null;
    await loadTargets();
    pick = { folder, target: null, preset: null };
    $('wPickName').textContent = baseName(folder); $('wPickPath').textContent = folder;
    $('wSearch').value = ''; $('wSub').checked = false; $('wPresetRow').hidden = true; $('wConfirm').disabled = true;
    $('wPick').hidden = false; $('wBar').hidden = true; $('wEmpty').hidden = true;
    renderTargets();
    $('wSearch').focus();
  });
  $('wSearch').addEventListener('input', () => pick && renderTargets());
  $('wPreset').addEventListener('change', e => { if (pick) pick.preset = e.target.value || null; });
  $('wCancel').addEventListener('click', closePick);
  $('wConfirm').addEventListener('click', () => {
    if (!pick || !pick.target) return;
    const rule = { id: Date.now().toString(36) + Math.random().toString(36).slice(2, 7), folder: pick.folder, target: pick.target,
      preset: pick.preset, include_subfolders: $('wSub').checked, enabled: true };
    pick = null; $('wPick').hidden = true; $('wBar').hidden = false;
    change(() => {
      const old = S.watch.find(x => x.folder === rule.folder);
      if (old) Object.assign(old, rule, { id: old.id }); else S.watch.push(rule);
    });
    refreshStatus();
  });
  $('wPriv').addEventListener('click', e => { e.preventDefault(); invoke('open_privacy_settings').catch(() => {}); });


  /* ---------- Abläufe ---------- */
  S.chains = S.chains || [];
  let stepInfo = [], edit = null, val = null, valSeq = 0;
  const infoOf = id => stepInfo.find(t => t.id === id);
  const stepLabel = st => { const t = infoOf(st.target); return (t ? t.title : st.target) + (st.preset ? ' ' + st.preset : ''); };
  const newId = () => Date.now().toString(36) + Math.random().toString(36).slice(2, 6);
  const el = (tag, cls, text) => { const e = document.createElement(tag); if (cls) e.className = cls; if (text != null) e.textContent = text; return e; };
  const cleanSteps = steps => steps.map(s => ({ target: s.target, preset: s.preset || null }));
  const TEMPLATES = [
    { name: 'Bild fürs Web', note: 'Web-Format, dann Metadaten entfernen', steps: [{ target: 'web_image' }, { target: 'strip_meta' }] },
    { name: 'Podcast fertig', note: 'Lautheit normalisieren, dann MP3 192k', steps: [{ target: 'loudnorm' }, { target: 'mp3', preset: '192k' }] },
  ];

  function pills(steps) {
    const box = el('span', 'cpills');
    steps.forEach((st, i) => {
      if (i) box.append(el('i', 'carrow', '→'));
      box.append(el('span', 'cpill', stepLabel(st)));
    });
    return box;
  }

  function renderChainList() {
    const list = $('cList');
    $('cEmpty').hidden = S.chains.length > 0 || !!edit;
    list.hidden = !S.chains.length;
    $('cBar').hidden = !!edit;
    list.textContent = '';
    S.chains.forEach(c => {
      const row = el('div', 'row crow');
      const lbl = el('span', 'lbl'); lbl.append(el('b', null, c.name), pills(c.steps));
      const ringLbl = el('label', 'cring'); const tg = el('input', 'tg'); tg.type = 'checkbox'; tg.checked = !!c.in_ring;
      tg.setAttribute('aria-label', 'Im Ring');
      tg.addEventListener('change', () => change(() => { c.in_ring = tg.checked; }));
      ringLbl.append(el('span', null, 'Im Ring'), tg);
      const ed = el('button', 'btn', 'Bearbeiten'); ed.type = 'button';
      ed.addEventListener('click', () => openEdit(c));
      const del = el('button', 'wdel'); del.type = 'button'; del.title = 'Löschen'; del.setAttribute('aria-label', 'Ablauf löschen');
      del.innerHTML = '<svg viewBox="0 0 20 20"><path d="M5 5l10 10M15 5L5 15"/></svg>';
      del.addEventListener('click', () => change(() => {
        S.chains = S.chains.filter(x => x.id !== c.id);
        S.watch = (S.watch || []).filter(w => w.target !== 'chain_' + c.id);
      }));
      row.append(lbl, ringLbl, ed, del);
      list.append(row);
    });
  }

  async function revalidate() {
    const seq = ++valSeq;
    let v = null;
    if (edit && edit.steps.length) { try { v = await invoke('chain_validate', { steps: cleanSteps(edit.steps) }); } catch (_) {} }
    if (seq !== valSeq) return;
    val = v; renderEditState();
  }

  function renderEditState() {
    if (!edit) return;
    const bad = val && !val.ok;
    const err = $('cErr');
    err.hidden = !bad; err.textContent = bad ? val.error : '';
    $$('#cSteps .cstep').forEach((r, i) => r.classList.toggle('bad', !!bad && val.error_step === i));
    $('cSave').disabled = !(edit.name.trim() && edit.steps.length && val && val.ok);
  }

  function renderSteps() {
    const box = $('cSteps'); box.textContent = '';
    edit.steps.forEach((st, i) => {
      const t = infoOf(st.target);
      const row = el('div', 'row cstep');
      const num = el('span', 'cnum', String(i + 1));
      const lbl = el('span', 'lbl'); lbl.append(el('b', null, t ? t.title : st.target), el('small', null, t ? t.sub : 'Ziel nicht gefunden'));
      row.append(num, lbl);
      if (t && t.presets && t.presets.length) {
        const sel = el('select', 'sel');
        sel.add(new Option('Standard', ''));
        t.presets.forEach(p => sel.add(new Option(p, p, false, p === st.preset)));
        sel.addEventListener('change', () => { st.preset = sel.value || null; revalidate(); });
        row.append(sel);
      }
      const mk = (cls, label, path, fn, disabled) => {
        const b = el('button', 'wdel ' + cls); b.type = 'button'; b.title = label; b.setAttribute('aria-label', label); b.disabled = disabled;
        b.innerHTML = '<svg viewBox="0 0 20 20"><path d="' + path + '"/></svg>'; b.addEventListener('click', fn); return b;
      };
      row.append(
        mk('up', 'Nach oben', 'M5 12l5-5 5 5', () => moveStep(i, -1), i === 0),
        mk('down', 'Nach unten', 'M5 8l5 5 5-5', () => moveStep(i, 1), i === edit.steps.length - 1),
        mk('rm', 'Schritt entfernen', 'M5 5l10 10M15 5L5 15', () => { edit.steps.splice(i, 1); stepsChanged(); }, false));
      box.append(row);
    });
    renderEditState();
  }
  function moveStep(i, d) { const j = i + d; if (j < 0 || j >= edit.steps.length) return; [edit.steps[i], edit.steps[j]] = [edit.steps[j], edit.steps[i]]; stepsChanged(); }
  function stepsChanged() { closePicker(); renderSteps(); val = null; renderEditState(); revalidate(); }

  let candidates = [];
  function renderCandidates() {
    const box = $('cTargets'), q = $('cSearch').value.trim().toLowerCase();
    box.textContent = '';
    const groups = new Map();
    candidates.filter(t => !q || (t.title + ' ' + t.sub + ' ' + t.group + ' ' + t.id).toLowerCase().includes(q))
      .forEach(t => { if (!groups.has(t.group)) groups.set(t.group, []); groups.get(t.group).push(t); });
    if (!groups.size) box.append(el('div', 'wnone', edit.steps.length ? 'Kein weiterer Schritt passt zum Ergebnis des letzten Schritts.' : 'Kein passendes Ziel gefunden.'));
    groups.forEach((items, g) => {
      box.append(el('h4', null, g));
      items.forEach(t => {
        const b = el('button', 'wt'); b.type = 'button';
        b.append(el('span', null, t.title), el('small', null, t.sub || ''));
        b.addEventListener('click', () => { edit.steps.push({ target: t.id, preset: null }); stepsChanged(); });
        box.append(b);
      });
    });
  }
  function closePicker() { $('cPicker').hidden = true; }
  $('cAdd').addEventListener('click', async () => {
    if (!$('cPicker').hidden) { closePicker(); return; }
    if (edit.steps.length && val && !val.ok) return;
    try { candidates = await invoke('chain_candidates', { steps: cleanSteps(edit.steps) }); } catch (_) { candidates = []; }
    $('cSearch').value = ''; $('cPicker').hidden = false; renderCandidates(); $('cSearch').focus();
  });
  $('cSearch').addEventListener('input', renderCandidates);
  $('cName').addEventListener('input', e => { edit.name = e.target.value; renderEditState(); });
  $('cRing').addEventListener('change', e => { edit.in_ring = e.target.checked; });

  function openEdit(c) {
    edit = c ? { id: c.id, name: c.name, in_ring: !!c.in_ring, steps: c.steps.map(s => ({ ...s })) } : { id: null, name: '', in_ring: false, steps: [] };
    $('cName').value = edit.name; $('cRing').checked = edit.in_ring;
    $('cEdit').hidden = false; closePicker(); val = null;
    renderChainList(); renderSteps(); revalidate();
    $('cName').focus();
  }
  function closeEdit() { edit = null; valSeq++; $('cEdit').hidden = true; renderChainList(); }
  $('cNew').addEventListener('click', () => openEdit(null));
  $('cCancel').addEventListener('click', closeEdit);
  $('cSave').addEventListener('click', () => {
    if (!edit || $('cSave').disabled) return;
    const c = { id: edit.id || newId(), name: edit.name.trim(), steps: cleanSteps(edit.steps), in_ring: edit.in_ring };
    edit = null; valSeq++; $('cEdit').hidden = true;
    change(() => {
      const i = S.chains.findIndex(x => x.id === c.id);
      if (i >= 0) S.chains[i] = c; else S.chains.push(c);
    });
  });

  async function loadChainInfo() {
    try { stepInfo = await invoke('chain_step_info'); } catch (_) { stepInfo = []; }
    const box = $('cTempl'); box.textContent = '';
    for (const tpl of TEMPLATES) {
      if (!tpl.steps.every(s => infoOf(s.target))) continue;
      try { const v = await invoke('chain_validate', { steps: cleanSteps(tpl.steps) }); if (!v.ok) continue; } catch (_) { continue; }
      const b = el('button', 'btn ctpl'); b.type = 'button';
      b.append(el('b', null, tpl.name), el('small', null, tpl.note));
      b.addEventListener('click', () => change(() => { S.chains.push({ id: newId(), name: tpl.name, steps: cleanSteps(tpl.steps), in_ring: false }); }));
      box.append(b);
    }
    $('cTempl').hidden = !box.children.length;
    renderChainList();
  }

  /* ---------- render ---------- */
  const EX = { plain: 'Urlaub.jpg', suffix: 'Urlaub (JPG).jpg' };
  function render(skipText) {
    document.body.classList.toggle('style-glass', S.style === 'glass');
    document.body.classList.toggle('style-classic', S.style !== 'glass');
    document.body.classList.toggle('reduce-motion', !!S.reduce_motion);
    $$('.hero svg').forEach(s => { try { S.reduce_motion ? s.pauseAnimations() : s.unpauseAnimations(); } catch (_) {} });
    mark($('themes'), S.style || 'classic');
    $('autoupdate').checked = S.auto_update !== false;
    $('autostart').checked = !!S.autostart; $('sound').checked = !!S.sound; $('reduce').checked = !!S.reduce_motion;
    mark($('keys'), S.trigger_key || 'option');
    $('shake').checked = !!S.shake; $('notch').checked = !!S.notch;
    mark($('mode'), S.output.mode); mark($('naming'), S.output.naming === 'plain' ? 'plain' : 'suffix');
    $('folderRow').hidden = S.output.mode !== 'folder';
    if (document.activeElement !== $('folder')) $('folder').value = S.output.folder || '';
    $('nameEx').textContent = 'Beispiel: ' + EX[S.output.naming === 'plain' ? 'plain' : 'suffix'];

    renderWatch();
    renderChainList();
    const o = S.ollama;
    $('ollama').checked = !!o.enabled;
    $('aiBody').classList.toggle('off', !o.enabled);
    if (!skipText && document.activeElement !== $('url')) $('url').value = o.url || '';
    const scope = scopeOf(o.url);
    $('lanNote').hidden = scope !== 'private';
    $('warn').hidden = scope !== 'public';
    $('remoteRow').hidden = scope !== 'public';
    $('remote').checked = !!o.allow_remote;
    const st = $('status'); st.className = 'status ' + status;
    $('statusTxt').textContent = status === 'ok' ? `Verbunden (${models.length} ${models.length === 1 ? 'Modell' : 'Modelle'})`
      : status === 'bad' ? 'Nicht erreichbar. Ollama muss laufen (ollama serve).' : status === 'busy' ? 'Wird geprüft …' : 'Nicht geprüft';
    $('test').disabled = status === 'busy';
    const sel = $('model');
    const list = status === 'ok' ? models : [];
    sel.innerHTML = '';
    if (list.length) list.forEach(m => sel.add(new Option(m, m, false, m === o.model)));
    else sel.add(new Option(o.model || 'Nicht verbunden', o.model || ''));
    sel.disabled = !list.length;
    $$('#actions [data-a]').forEach(c => { c.checked = (o.actions || []).includes(c.dataset.a); });
  }

  /* ---------- Rechtsklick-Eintrag (Finder / Explorer) ---------- */
  const ctxWords = { finder: 'Finder', explorer: 'Explorer' };
  function ctxRender(st, err) {
    if (!st || !st.supported) { $('ctxRow').hidden = true; return; }
    const app = ctxWords[st.kind] || 'Dateimanager';
    $('ctxRow').hidden = false;
    $('ctxTitle').textContent = app + '-Rechtsklick';
    $('ctxState').textContent = err || (st.installed
      ? `Eingerichtet. Rechtsklick auf Dateien im ${app}, dann „Mit Orbo umwandeln“.`
      : `Noch nicht eingerichtet. Fügt „Mit Orbo umwandeln“ zum Kontextmenü im ${app} hinzu.`);
    $('ctxInstall').textContent = st.installed ? 'Neu einrichten' : app + '-Rechtsklick einrichten';
    $('ctxRemove').hidden = !st.installed;
  }
  async function ctxRun(cmd) {
    $('ctxInstall').disabled = $('ctxRemove').disabled = true;
    try { ctxRender(await invoke(cmd)); }
    catch (e) { ctxRender(await invoke('context_menu_status').catch(() => null), String(e)); }
    $('ctxInstall').disabled = $('ctxRemove').disabled = false;
  }
  $('ctxInstall').addEventListener('click', () => ctxRun('context_menu_install'));
  $('ctxRemove').addEventListener('click', () => ctxRun('context_menu_remove'));
  invoke('context_menu_status').then(ctxRender).catch(() => {});

  /* ---------- Kommandozeile und KI-Programme ---------- */
  const INTEG = [
    { id: 'cli', title: 'Kommandozeilen-Tool', idle: 'Macht den Befehl „orbo“ im Terminal verfügbar.', button: 'Kommandozeilen-Tool einrichten' },
    { id: 'claude_code', title: 'Claude Code', idle: 'Orbo-Werkzeuge für Claude Code (MCP-Server).', button: 'In Claude Code einrichten' },
    { id: 'claude_desktop', title: 'Claude Desktop', idle: 'Orbo-Werkzeuge für Claude Desktop (MCP-Server).', button: 'In Claude Desktop einrichten' },
    { id: 'codex', title: 'Codex', idle: 'Orbo-Werkzeuge für Codex (MCP-Server).', button: 'In Codex einrichten' },
  ];
  const integEls = {};
  INTEG.forEach(d => {
    const wrap = document.createElement('div');
    wrap.hidden = true;
    wrap.innerHTML = '<div class="row"><span class="lbl"><span class="t"></span><small class="s"></small></span><span class="ctxbtns"><button class="btn do" type="button"></button><button class="btn rm" type="button" hidden>Entfernen</button></span></div>' +
      '<div class="hintrow" hidden><code></code><button class="btn cp" type="button">Kopieren</button></div>';
    $('integ').appendChild(wrap);
    const e = integEls[d.id] = { wrap, t: wrap.querySelector('.t'), s: wrap.querySelector('.s'), doB: wrap.querySelector('.do'), rm: wrap.querySelector('.rm'), hint: wrap.querySelector('.hintrow'), code: wrap.querySelector('code'), cp: wrap.querySelector('.cp') };
    e.t.textContent = d.title;
    e.doB.addEventListener('click', () => integRun(d, 'integration_install'));
    e.rm.addEventListener('click', () => integRun(d, 'integration_remove'));
    e.cp.addEventListener('click', async () => {
      try { await navigator.clipboard.writeText(e.code.textContent); e.cp.textContent = 'Kopiert'; setTimeout(() => { e.cp.textContent = 'Kopieren'; }, 1500); }
      catch { const r = document.createRange(); r.selectNodeContents(e.code); const sel = getSelection(); sel.removeAllRanges(); sel.addRange(r); }
    });
  });
  function integRender(d, st, msg) {
    const e = integEls[d.id];
    if (!st || !st.supported) { e.wrap.hidden = true; return; }
    e.wrap.hidden = false;
    let text = d.idle;
    if (!st.available) text = 'Das Kommandozeilen-Tool fehlt in dieser Orbo-Installation.';
    else if (st.installed && st.stale) text = 'Eingerichtet, zeigt aber auf einen anderen Ort. „Neu einrichten“ korrigiert das.';
    else if (st.installed) text = 'Eingerichtet.';
    e.s.textContent = msg || (st.note && !st.manual ? st.note : text);
    e.doB.textContent = st.installed ? 'Neu einrichten' : d.button;
    e.doB.disabled = !st.available;
    e.rm.hidden = !st.installed;
    e.hint.hidden = !st.manual;
    if (st.manual) {
      e.code.textContent = st.manual;
      if (st.note) e.s.textContent = msg || st.note;
    }
  }
  async function integRun(d, cmd) {
    const e = integEls[d.id];
    e.doB.disabled = e.rm.disabled = true;
    try { integRender(d, await invoke(cmd, { id: d.id })); }
    catch (err) { integRender(d, await invoke('integration_status', { id: d.id }).catch(() => null), String(err)); }
    e.rm.disabled = false;
  }
  INTEG.forEach(d => invoke('integration_status', { id: d.id }).then(st => integRender(d, st)).catch(() => {}));

  $('support').addEventListener('click', () => invoke('support').catch(() => {}));

  listen('settings', e => {
    if (!e.payload) return;
    S = e.payload;
    S.watch = S.watch || [];
    S.chains = S.chains || [];
    S.output = S.output || { mode: 'beside', folder: null, naming: 'suffix' };
    S.ollama = S.ollama || { enabled: false, url: 'http://localhost:11434', model: null, actions: [] };
    render();
    wTargets = null;
    loadTargets().then(renderWatch);
  });

  try {
    const v = await window.__TAURI__.app?.getVersion?.();
    if (v) $('version').textContent = 'Version ' + v;
  } catch (_) {}

  /* ---------- updates ---------- */
  const UPD_TEXT = {
    idle: () => 'Noch nicht geprüft',
    checking: () => 'Suche nach Updates …',
    current: () => 'Orbo ist aktuell',
    available: u => `Version ${u.version} ist verfügbar`,
    downloading: u => `Version ${u.version} wird geladen …`,
    ready: u => `Version ${u.version} ist geladen`,
    installing: u => `Version ${u.version} wird installiert …`,
    error: u => 'Fehler: ' + (u.message || 'Unbekannter Fehler'),
  };
  function showUpdate(u) {
    u = u || { state: 'idle' };
    const text = (UPD_TEXT[u.state] || UPD_TEXT.idle)(u);
    $('updText').firstChild.nodeValue = text;
    $('updSub').textContent = u.state === 'ready' ? 'Orbo startet neu, sobald gerade nichts umgewandelt wird.' : '';
    $('updCheck').disabled = ['checking', 'downloading', 'ready', 'installing'].includes(u.state);
    $('updInstall').hidden = !(u.state === 'available' || (u.state === 'error' && u.version));
    $('updInstall').disabled = false;
  }
  listen('update-state', e => showUpdate(e.payload));
  $('updCheck').addEventListener('click', () => {
    showUpdate({ state: 'checking' });
    invoke('update_check').catch(() => {});
  });
  $('updInstall').addEventListener('click', () => {
    $('updInstall').disabled = true;
    invoke('update_install').catch(() => {});
  });
  invoke('update_status').then(showUpdate).catch(() => showUpdate(null));

  await loadChainInfo();
  await loadTargets();
  render();
  refreshStatus();
  const hash = (location.hash || '').slice(1).split(',');
  const first = ['gen', 'trg', 'out', 'wat', 'chn', 'ai', 'about'].find(t => hash.includes(t)) || 'gen';
  openTab(first);
  if (S.ollama.enabled && first !== 'ai') testOllama();
  document.addEventListener('keydown', e => { if (e.key === 'Escape') window.close(); });
})();

// External links open in the default browser instead of inside the settings window.
document.addEventListener('click', e => {
  const a = e.target.closest('[data-url]');
  if (!a) return;
  e.preventDefault();
  const url = a.dataset.url;
  if (window.__TAURI__) window.__TAURI__.core.invoke('open_url', { url }).catch(() => {});
  else window.open(url, '_blank');
});
