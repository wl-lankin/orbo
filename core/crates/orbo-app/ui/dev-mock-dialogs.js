/* Browser-only stand-in for the Tauri bridge. Does nothing inside the app. */
(function () {
  if (window.__TAURI__) return;
  const tokens = (location.hash || '#').slice(1).split(',').filter(Boolean);
  const has = t => tokens.includes(t);

  const img = { path: 'dev-assets/shot.png', name: 'Bildschirmfoto 2026-10-18 um 21.04.png', format: 'png', kind: 'image', size: 7515000, meta: '1600 × 1000', width: 1600, height: 1000, duration_s: null };
  const vid = { path: 'dev-assets/clip.mp4', name: 'Bildschirmaufnahme.mp4', format: 'mp4', kind: 'video', size: 2379198, meta: '1280 × 720 · 0:06', width: 1280, height: 720, duration_s: 6 };
  const aud = { path: 'dev-assets/song.mp3', name: 'Sprachmemo.mp3', format: 'mp3', kind: 'audio', size: 64592, meta: '0:08 · 128 kbit/s', width: null, height: null, duration_s: 8 };

  const T = (id, title, sub, controls, presets = []) => ({ id, title, sub, controls, presets });
  const ctxs = {
    crop: { file: img, target: T('crop', 'Zuschneiden', 'Bildausschnitt wählen', ['crop', 'aspect']) },
    beautify: { file: img, target: T('beautify', 'Präsentation', 'Hintergrund, Rand, Schatten', ['background', 'padding', 'corners', 'shadow', 'aspect']) },
    trim: { file: vid, target: T('trim_video', 'Kürzen', 'Anfang und Ende festlegen', ['trim']) },
    audio: { file: aud, target: T('mp3', 'MP3', 'universell abspielbar', ['audio_bitrate', 'trim', 'sample_rate', 'channels'], ['128k', '192k', '320k']) },
    resize: { file: img, target: T('resize', 'Größe ändern', 'Breite und Höhe festlegen', ['resize', 'quality']) },
    quality: { file: img, target: T('compress_image', 'Komprimieren', 'Dateigröße reduzieren', ['quality', 'target_size']) },
    gif: { file: vid, target: T('gif_video', 'GIF', 'Ausschnitt, 6 Sekunden', ['trim', 'fps', 'resize'], ['Klein', 'Mittel', 'Groß']) },
  };
  const key = Object.keys(ctxs).find(has) || 'crop';
  const c = ctxs[key];
  const dialogContext = {
    paths: [c.file.path], target: c.target, file: c.file,
    preview: c.file.path, preset: has('preset') ? c.target.presets[0] || null : null,
  };

  let settings = {
    style: has('glass') ? 'glass' : 'classic', trigger_key: 'option', shake: true, notch: false, sound: false, reduce_motion: false,
    output: { mode: 'beside', folder: null, naming: 'suffix' },
    ollama: { enabled: has('ai') || has('ollama'), url: 'http://localhost:11434', model: null, actions: ['rename', 'alt_text', 'summary'] },
  };

  const listeners = {};
  const emit = (name, payload) => (listeners[name] || []).forEach(f => f({ payload }));

  const commands = {
    dialog_context: () => dialogContext,
    support: () => console.log('[mock] support'),
    dialog_close: () => console.log('[mock] dialog_close'),
    convert: a => { console.log('[mock] convert', JSON.stringify(a)); return [1]; },
    settings_get: () => JSON.parse(JSON.stringify(settings)),
    settings_set: a => { settings = JSON.parse(JSON.stringify(a.settings)); console.log('[mock] settings_set', JSON.stringify(settings)); emit('settings', settings); },
  };

  window.__TAURI__ = {
    core: {
      invoke: async (cmd, args) => { if (!(cmd in commands)) throw new Error('mock: unknown command ' + cmd); return commands[cmd](args || {}); },
      convertFileSrc: p => p,
    },
    event: {
      listen: async (name, cb) => { (listeners[name] = listeners[name] || []).push(cb); return () => { listeners[name] = listeners[name].filter(f => f !== cb); }; },
      emit: async (name, payload) => emit(name, payload),
    },
  };
  window.__ORBO_MOCK__ = true;

  if (has('ollama')) {
    const real = window.fetch.bind(window);
    window.fetch = (u, o) => String(u).endsWith('/api/tags')
      ? Promise.resolve(new Response(JSON.stringify({ models: [{ name: 'qwen3:8b' }, { name: 'llama3.2:3b' }, { name: 'gemma3:12b' }] }), { status: 200 }))
      : real(u, o);
  }
})();
