/* Preview-only Tauri mock. Does nothing inside the real app. */
(() => {
  if (window.__TAURI__) return;
  window.__ORBO_MOCK__ = true;
  const listeners = {};
  let dragCb = null;
  const dpr = () => window.devicePixelRatio || 1;
  const MB = 1048576;

  const T = (id, title, sub, kind, o = {}) => Object.assign({
    id, title, sub, ext: id, intent: false, group: kind, kind, dialog: false,
    controls: [], presets: [], size_estimate: null,
  }, o);
  const Q = ['quality', 'resize'];
  const IMG = ['Web', 'Ausgewogen', 'Verlustfrei'];

  const SAMPLES = {
    png: {
      files: [{ path: '/Users/demo/Bilder/logo.png', name: 'logo.png', format: 'png', kind: 'image', size: 1.8 * MB,
        meta: '1024 × 1024 · Transparenz', width: 1024, height: 1024, duration_s: null }],
      default: 'webp',
      ring: [
        T('webp', 'WEBP', 'kompakt für das Web', 'image', { controls: Q, presets: IMG, size_estimate: 217000 }),
        T('avif', 'AVIF', 'maximale Kompression', 'image', { controls: Q, presets: IMG, size_estimate: 151000 }),
        T('web', 'Web', '1920 px · WEBP', 'image', { intent: true, size_estimate: 38000 }),
        T('svg', 'SVG', 'vektorisieren', 'image', { dialog: true, size_estimate: 36000 }),
        T('ico', 'ICO', 'Symbol 16 bis 256 px', 'image', { dialog: true, size_estimate: 94000 }),
        T('jpg', 'JPG', 'universell kompatibel', 'image', { controls: Q, presets: IMG, size_estimate: 321000 }),
      ],
      more: [],
    },
    mov: {
      files: [{ path: '/Users/demo/Filme/Urlaub.mov', name: 'Urlaub.mov', format: 'mov', kind: 'video', size: 612 * MB,
        meta: '4K · 1:12 min · H.264', width: 3840, height: 2160, duration_s: 72 }],
      default: 'mp4',
      ring: [
        T('mp4', 'MP4', 'universell abspielbar', 'video', { size_estimate: 600 * MB }),
        T('hevc', 'HEVC', 'halbe Dateigröße', 'video', { presets: ['4K', '1080p', '720p'], size_estimate: 288 * MB }),
        T('mail', 'E-Mail', 'unter 25 MB', 'video', { intent: true, size_estimate: 24 * MB }),
        T('srt', 'SRT', 'Untertitel, universell', 'document', { size_estimate: 4400 }),
        T('m4a', 'M4A', 'AAC, kompakt', 'audio', { size_estimate: 1.1 * MB }),
        T('gif', 'GIF', 'Ausschnitt, 6 Sekunden', 'image', { dialog: true, size_estimate: 8 * MB }),
      ],
      more: [],
    },
    wav: {
      files: [{ path: '/Users/demo/Audio/Interview.wav', name: 'Interview.wav', format: 'wav', kind: 'audio', size: 480 * MB,
        meta: '42 min · 48 kHz', width: null, height: null, duration_s: 2520 }],
      default: 'mp3',
      ring: [
        T('mp3', 'MP3', 'universell abspielbar', 'audio', { controls: ['quality'], presets: ['128k', '192k', '320k'], size_estimate: 58 * MB }),
        T('m4a', 'M4A', 'AAC, kompakt', 'audio', { controls: ['quality'], size_estimate: 40 * MB }),
        T('transcript', 'Transkript', 'Text, lokale KI', 'document', { intent: true, size_estimate: 38000 }),
        T('loud', 'Lautheit', 'normalisieren (EBU R128)', 'audio', { intent: true, dialog: true }),
        T('opus', 'OPUS', 'optimal für Sprache', 'audio', { size_estimate: 12 * MB }),
        T('flac', 'FLAC', 'verlustfrei, kompakt', 'audio', { size_estimate: 264 * MB }),
      ],
      more: [],
    },
  };

  const settings = {
    style: 'classic', trigger_key: 'option', shake: true, notch: false, sound: false, reduce_motion: false,
    output: { mode: 'beside', folder: null, naming: 'suffix' },
    ollama: { enabled: false, url: 'http://localhost:11434', model: null, actions: ['rename'] },
  };

  const emit = (name, payload) => (listeners[name] || []).forEach(cb => cb({ event: name, payload }));
  const drag = payload => dragCb && dragCb({ payload });
  // Matches wry: points on macOS, physical pixels elsewhere.
  const IS_MAC = /Mac/.test(navigator.platform || navigator.userAgent);
  const pos = (x, y) => (IS_MAC ? { x, y } : { x: x * dpr(), y: y * dpr() });
  const sample = k => SAMPLES[k] || SAMPLES.png;

  window.__TAURI__ = {
    core: {
      invoke: async (cmd, args) => {
        console.log('[mock invoke]', cmd, args);
        if (cmd === 'suggest' && /noaccess/.test(args.paths[0])) {
          const r = JSON.parse(JSON.stringify(SAMPLES.png)); r.files[0].readable = false; return r;
        }
        if (cmd === 'suggest') return sample(/\.mov$/i.test(args.paths[0]) ? 'mov' : /\.wav$/i.test(args.paths[0]) ? 'wav' : 'png');
        if (cmd === 'settings_get') return settings;
        if (cmd === 'convert') return [1];
        if (cmd === 'support') return null;
        return null;
      },
      convertFileSrc: p => p,
    },
    event: {
      listen: async (name, cb) => {
        (listeners[name] = listeners[name] || []).push(cb);
        return () => { listeners[name] = listeners[name].filter(f => f !== cb); };
      },
      emit: async (name, payload) => emit(name, payload),
    },
    webview: { getCurrentWebview: () => ({ onDragDropEvent: async cb => { dragCb = cb; return () => { dragCb = null; }; } }) },
  };

  // Window coordinates in CSS px, ring center at 260/260.
  const polar = (deg, r) => [260 + r * Math.sin(deg * Math.PI / 180), 260 - r * Math.cos(deg * Math.PI / 180)];
  window.mock = {
    samples: SAMPLES,
    open(kind = 'png', extra = {}) {
      const s = sample(kind);
      emit('ring:open', Object.assign({ files: s.files.map(f => f.path), suggestions: s, style: settings.style,
        sound: false, reduce_motion: false }, extra));
    },
    openEmpty(extra = {}) {
      emit('ring:open', Object.assign({ files: [], suggestions: null, style: settings.style, sound: false, reduce_motion: false }, extra));
    },
    enter(kind = 'png', x = 260, y = 260) { drag({ type: 'enter', paths: sample(kind).files.map(f => f.path), position: pos(x, y) }); },
    over(x, y) { drag({ type: 'over', position: pos(x, y) }); },
    overPolar(deg, r) { const [x, y] = polar(deg, r); this.over(x, y); },
    drop(x, y, kind = 'png') { drag({ type: 'drop', paths: sample(kind).files.map(f => f.path), position: pos(x, y) }); },
    leave() { drag({ type: 'leave' }); },
    notchOpen(kind = 'png', extra = {}) {
      const s = sample(kind);
      emit('notch:open', Object.assign({ files: s.files.map(f => f.path), suggestions: s, style: settings.style,
        sound: false, reduce_motion: false }, extra));
    },
    notchClose(reason = 'cancelled') { emit('notch:close', { reason }); },
    openNoAccess() {
      const s = JSON.parse(JSON.stringify(SAMPLES.png)); s.files[0].readable = false;
      emit('ring:open', { files: s.files.map(f => f.path), suggestions: s, style: settings.style, sound: false, reduce_motion: false });
    },
    close(reason = 'released') { emit('ring:close', { reason }); },
    settings(patch) { Object.assign(settings, patch); emit('settings', settings); },
    job(payload) { emit('job', payload); },
    jobDemo() {
      const j = (o) => emit('job', o);
      j({ event: 'started', id: 1, input: '/Users/demo/Filme/Urlaub.mov', target: 'mp4', title: 'MP4' });
      j({ event: 'progress', id: 1, fraction: 0.42 });
      j({ event: 'started', id: 2, input: '/Users/demo/Bilder/logo.png', target: 'webp', title: 'WEBP' });
      j({ event: 'finished', id: 2, output: '/Users/demo/Bilder/logo.webp', bytes: 217000, millis: 400, input_bytes: 1887436 });
      j({ event: 'started', id: 3, input: '/Users/demo/Audio/Interview.wav', target: 'mp3', title: 'MP3' });
      j({ event: 'failed', id: 3, error: 'Der Audiokodierer konnte die Datei nicht lesen: unerwartetes Dateiende bei 00:12:41.' });
    },
    // Fake the native blurred disc behind the ring, only for previews.
    backdrop(on = true) {
      let d = document.getElementById('mock-disc');
      if (!on) return d && d.remove();
      if (!d) {
        d = document.createElement('div');
        d.id = 'mock-disc';
        document.body.prepend(d);
      }
      const glass = document.body.classList.contains('style-glass');
      d.style.cssText = 'position:fixed;left:80px;top:80px;width:360px;height:360px;border-radius:50%;z-index:-1;' +
        (glass ? 'background:radial-gradient(circle at 50% 22%,rgba(255,255,255,.28),rgba(255,255,255,.1) 70%);box-shadow:inset 0 1px 0 rgba(255,255,255,.55);backdrop-filter:blur(24px) saturate(1.6);'
               : 'background:rgba(16,18,30,.62);backdrop-filter:blur(22px) saturate(1.5);box-shadow:0 20px 60px rgba(0,0,0,.35);');
    },
  };
})();
