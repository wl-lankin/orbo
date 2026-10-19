/* Browser-only stand-in for the Tauri bridge. Does nothing inside the app. */
(function () {
  if (window.__TAURI__) return;
  window.__ORBO_MOCK__ = true;
  const tokens = (location.hash || '#').slice(1).split(',').filter(Boolean);
  const has = t => tokens.includes(t);
  let settings = { style: has('glass') ? 'glass' : 'classic', trigger_key: 'option', shake: true, notch: false, sound: false,
    reduce_motion: false, autostart: false, onboarded: false, output: { mode: 'beside', folder: null, naming: 'suffix' } };
  let calls = 0;
  const listeners = {};
  const commands = {
    settings_get: () => JSON.parse(JSON.stringify(settings)),
    settings_set: a => { settings = JSON.parse(JSON.stringify(a.settings)); console.log('[mock] settings_set', JSON.stringify(settings)); },
    // First call grants two of three folders, later calls the rest.
    request_folder_access: () => { calls++; return has('denied') ? { desktop: false, documents: false, downloads: false }
      : { desktop: true, documents: calls > 1 || has('all'), downloads: calls > 1 || has('all') }; },
    support: () => null,
    onboarding_sample: () => '/Users/demo/Orbo Beispiel.png',
  };
  window.__TAURI__ = {
    core: { invoke: async (cmd, args) => { console.log('[mock invoke]', cmd, args); return cmd in commands ? commands[cmd](args || {}) : null; }, convertFileSrc: p => p },
    event: { listen: async (n, cb) => { (listeners[n] = listeners[n] || []).push(cb); return () => {}; }, emit: async () => {} },
  };
})();
