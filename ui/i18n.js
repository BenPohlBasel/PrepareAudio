'use strict';

/* Interface language: one dictionary, de/en/fr/it, switched without reload.
   Dictionaries live in ui/i18n/*.js and register with I18N.add({...}).
   Fallback: fr/it -> en -> de; a missing key shows the key itself (loud, not silent).

   Static HTML:  data-i18n="key"             -> textContent
                 data-i18n-html="key"        -> innerHTML (only for trusted dictionary text)
                 data-i18n-title / -aria / -placeholder -> attributes
   JavaScript:   t('key', { n: 3 })          -> "{n}" placeholders replaced
                 tn('key', n, params)        -> picks key.one (n === 1) or key.other
   Change hook:  I18N.onChange(fn)           -> re-render dynamic parts */
(() => {
  const LANGS = ['de', 'en', 'fr', 'it'];
  const NAMES = { de: 'Deutsch', en: 'English', fr: 'Français', it: 'Italiano' };
  const LOCALES = { de: 'de-DE', en: 'en-GB', fr: 'fr-FR', it: 'it-IT' };
  const STORE = 'prepareaudio.lang';
  const W = {};
  const listeners = new Set();
  const warned = new Set();

  let stored = null;
  try { stored = localStorage.getItem(STORE); } catch (e) { /* storage unavailable */ }
  const system = String(navigator.language || '').slice(0, 2).toLowerCase();
  let lang = LANGS.includes(stored) ? stored : LANGS.includes(system) ? system : 'en';

  function add(dict) {
    for (const [k, v] of Object.entries(dict)) {
      if (W[k] && window.console) console.warn(`i18n: key defined twice: ${k}`);
      W[k] = v;
    }
  }

  function raw(key) {
    const e = W[key];
    if (!e) {
      if (!warned.has(key)) { warned.add(key); if (window.console) console.warn(`i18n: missing key ${key}`); }
      return key;
    }
    return e[lang] ?? (lang === 'fr' || lang === 'it' ? e.en : undefined) ?? e.en ?? e.de ?? key;
  }

  function t(key, params) {
    let s = raw(key);
    if (params) s = s.replace(/\{(\w+)\}/g, (m, p) => (p in params ? String(params[p]) : m));
    return s;
  }

  function tn(key, n, params) {
    return t(`${key}.${n === 1 ? 'one' : 'other'}`, { n, ...(params || {}) });
  }

  function apply(root) {
    const r = root || document;
    r.querySelectorAll('[data-i18n]').forEach((el) => { el.textContent = t(el.dataset.i18n); });
    r.querySelectorAll('[data-i18n-html]').forEach((el) => { el.innerHTML = t(el.dataset.i18nHtml); });
    r.querySelectorAll('[data-i18n-title]').forEach((el) => { el.title = t(el.dataset.i18nTitle); });
    r.querySelectorAll('[data-i18n-aria]').forEach((el) => { el.setAttribute('aria-label', t(el.dataset.i18nAria)); });
    r.querySelectorAll('[data-i18n-placeholder]').forEach((el) => { el.placeholder = t(el.dataset.i18nPlaceholder); });
  }

  function tellBackend() {
    const core = window.__TAURI__ && window.__TAURI__.core;
    if (core) core.invoke('set_language', { lang }).catch(() => {});
  }

  function setLang(l) {
    if (!LANGS.includes(l)) return;
    lang = l;
    try { localStorage.setItem(STORE, l); } catch (e) { /* storage unavailable */ }
    document.documentElement.lang = l;
    apply();
    tellBackend();
    listeners.forEach((fn) => { try { fn(l); } catch (e) { if (window.console) console.error(e); } });
  }

  window.I18N = {
    LANGS, NAMES, add, t, tn, apply, setLang,
    lang: () => lang,
    locale: () => LOCALES[lang],
    onChange: (fn) => listeners.add(fn),
    has: (key) => key in W,
    keys: () => Object.keys(W),
    entry: (key) => W[key],
  };
  window.t = t;
  window.tn = tn;

  document.documentElement.lang = lang;
  tellBackend();
  document.addEventListener('DOMContentLoaded', () => apply());
})();
