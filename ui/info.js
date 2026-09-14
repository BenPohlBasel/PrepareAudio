'use strict';

/* Info panel: publisher, links, language switch, the app's AGPL licence and every bundled
   third-party licence. Texts: ui/i18n/info.js. Licence texts themselves stay in English. */
(() => {
  const tr = (k, p) => I18N.t(k, p);
  const trn = (k, n, p) => I18N.tn(k, n, p);
  const q = (sel) => document.querySelector(sel);
  const modal = q('#info');
  const PUBLISHER_SITE = 'https://bias.city/prepareaudio';
  const SOURCE_URL = 'https://github.com/BenPohlBasel/PrepareAudio';
  const DEFAULT_LICENSE = 'AGPL-3.0-or-later';
  let data = null;
  let loading = null;
  let loadError = null;

  async function load() {
    if (data) return data;
    if (!loading) loading = fetch('licenses.json').then((r) => r.json()).then((j) => (data = j));
    return loading;
  }

  const link = (url, label) => `<button type="button" data-link="${esc(url)}" title="${esc(url)}">${esc(label)}</button>`;

  /* Parts that do not need licenses.json: shown at once and on every language change. */
  function renderStatic() {
    const app = data && data.app;
    const license = (app && app.license) || DEFAULT_LICENSE;
    q('#info-version').textContent = tr('info.version', { version: (app && app.version) || '–', license });
    q('#info-about').innerHTML = `
      <p>${esc(tr('info.about'))}</p>
      <div class="info-links">${link(PUBLISHER_SITE, tr('info.link.site'))}${link(SOURCE_URL, tr('info.link.source'))}</div>`;
    const cur = I18N.lang();
    q('#info-lang-section').innerHTML = `
      <h3 id="info-lang-title">${esc(tr('info.lang.heading'))}</h3>
      <div class="lang-switch" role="group" aria-labelledby="info-lang-title">${I18N.LANGS.map((l) =>
    `<button type="button" data-lang="${l}" lang="${l}" aria-pressed="${l === cur}">${esc(I18N.NAMES[l])}</button>`).join('')}</div>`;
    q('#info-license').textContent = tr('info.license', { license });
  }

  function renderCrates(filter) {
    const f = (filter || '').toLowerCase();
    const rows = data.crates
      .filter((c) => !f || c.name.toLowerCase().includes(f) || String(c.license).toLowerCase().includes(f))
      .map((c) => `<div class="crate-row"><span class="cname">${link(c.repository, c.name)}</span><span class="cver">${esc(c.version)}</span><span class="clic" title="${esc(c.license)}">${esc(c.license)}</span></div>`)
      .join('');
    q('#info-crates').innerHTML = rows || `<div class="crate-row"><span class="clic">${esc(tr('info.noHits'))}</span></div>`;
  }

  // licenses.json carries the notes in German; the two shown here have dictionary translations.
  const noteText = (c) => {
    const key = `info.note.${c.name}`;
    return I18N.has(key) ? tr(key) : c.note;
  };

  function renderData() {
    if (loadError) {
      q('#info-third-intro').textContent = tr('info.loadError', { error: loadError });
      return;
    }
    if (!data) return;
    q('#info-license-text').textContent = data.app.license_text;
    q('#info-third-intro').textContent = trn('info.thirdIntro', data.crates.length);
    const notes = data.crates.filter((c) => c.note && (c.name === 'mp3lame-sys' || c.name === 'symphonia'));
    q('#info-notes').innerHTML = notes.length
      ? `<ul class="notes">${notes.map((c) => `<li><b>${esc(c.name)} ${esc(c.version)} (${esc(c.license)}):</b> ${esc(noteText(c))}</li>`).join('')}</ul>`
      : '';
    renderCrates(q('#info-filter').value);
    q('#info-texts-summary').textContent = trn('info.textsSummary', data.texts.length);
  }

  async function open() {
    modal.hidden = false;
    renderStatic();
    try {
      await load();
      loadError = null;
    } catch (e) {
      loadError = String(e);
      loading = null;
    }
    renderStatic();
    renderData();
  }

  function renderTexts() {
    if (q('#info-texts').dataset.done) return;
    q('#info-texts').innerHTML = data.texts
      .map((x) => `<div class="lic-text"><h4>${esc(tr('info.textHeading', { id: x.id }))}</h4><div class="for">${esc(tr('info.appliesTo', { crates: x.crates.join(', ') }))}</div><pre>${esc(x.text)}</pre></div>`)
      .join('');
    q('#info-texts').dataset.done = '1';
  }

  q('#info-open').addEventListener('click', open);
  q('#info-close').addEventListener('click', () => { modal.hidden = true; });
  modal.addEventListener('click', (e) => {
    if (e.target === modal) modal.hidden = true;
    const b = e.target.closest('[data-link]');
    if (b && window.openLink) window.openLink(b.dataset.link);
    const l = e.target.closest('[data-lang]');
    if (l && l.dataset.lang !== I18N.lang()) I18N.setLang(l.dataset.lang);
  });
  document.addEventListener('keydown', (e) => { if (e.key === 'Escape' && !modal.hidden) modal.hidden = true; });
  q('#info-filter').addEventListener('input', (e) => { if (data) renderCrates(e.target.value); });
  q('#info-texts-wrap').addEventListener('toggle', (e) => { if (e.target.open && data) renderTexts(); });

  I18N.onChange(() => {
    renderStatic();
    renderData();
    const texts = q('#info-texts');
    if (texts.dataset.done) {
      delete texts.dataset.done;
      if (q('#info-texts-wrap').open && data) renderTexts(); else texts.innerHTML = '';
    }
    const pressed = modal.querySelector(`[data-lang="${I18N.lang()}"]`);
    if (!modal.hidden && pressed) pressed.focus();
  });

  renderStatic();
})();
