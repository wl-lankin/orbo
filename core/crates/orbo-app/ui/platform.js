// Platform wording for the panels: Windows gets its own key names and loses macOS-only rows.
(() => {
  'use strict';
  const IS_WIN = /Windows/i.test(navigator.userAgent);
  const KEYS = IS_WIN
    ? { option: ['Alt', 'Alt'], control: ['Strg', 'Strg'], shift: ['⇧', 'Umschalt'] }
    : { option: ['⌥', 'Option'], control: ['⌃', 'Control'], shift: ['⇧', 'Shift'], command: ['⌘', 'Command'], fn: ['fn', 'Funktion'] };
  const glyph = v => (KEYS[v] || KEYS.option)[0];

  function apply(root = document) {
    document.documentElement.classList.toggle('os-win', IS_WIN);
    document.documentElement.classList.toggle('os-mac', !IS_WIN);
    root.querySelectorAll('.key[data-v]').forEach(k => {
      const info = KEYS[k.dataset.v];
      if (!info) { k.style.display = 'none'; return; }
      const b = k.querySelector('b'), s = k.querySelector('span');
      if (b) b.textContent = info[0];
      if (s) s.textContent = info[1];
      k.classList.toggle('word', info[0].length > 1);
    });
    if (IS_WIN) root.querySelectorAll('[data-win]').forEach(el => { el.textContent = el.dataset.win; });
    root.querySelectorAll('[data-key-glyph]').forEach(el => { el.textContent = glyph('option'); });
  }

  window.OrboPlatform = { IS_WIN, KEYS, glyph, apply, fileManager: IS_WIN ? 'Explorer' : 'Finder' };
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', () => apply()); else apply();
})();
