'use strict';

/* Info panel: what the app does, its MIT license and every bundled third-party license. */
(() => {
  const q = (sel) => document.querySelector(sel);
  const modal = q('#info');
  let data = null;
  let loading = null;

  async function load() {
    if (data) return data;
    if (!loading) loading = fetch('licenses.json').then((r) => r.json()).then((j) => (data = j));
    return loading;
  }

  const link = (url, label) => `<button data-link="${esc(url)}" title="${esc(url)}">${esc(label)}</button>`;

  function renderCrates(filter) {
    const f = (filter || '').toLowerCase();
    const rows = data.crates
      .filter((c) => !f || c.name.toLowerCase().includes(f) || String(c.license).toLowerCase().includes(f))
      .map((c) => `<div class="crate-row"><span class="cname">${link(c.repository, c.name)}</span><span class="cver">${esc(c.version)}</span><span class="clic" title="${esc(c.license)}">${esc(c.license)}</span></div>`)
      .join('');
    q('#info-crates').innerHTML = rows || '<div class="crate-row"><span class="clic">Keine Treffer.</span></div>';
  }

  async function open() {
    modal.hidden = false;
    try {
      await load();
    } catch (e) {
      q('#info-third-intro').textContent = `Lizenzangaben konnten nicht geladen werden: ${e}`;
      return;
    }
    const app = data.app;
    q('#info-version').textContent = `Version ${app.version} · ${app.authors.join(', ')} · Lizenz ${app.license}`;
    q('#info-license').textContent = `PrepareAudio steht unter der ${app.license}-Lizenz. Copyright © 2026 ${app.authors.join(', ')}.`;
    q('#info-license-text').textContent = app.license_text;
    const counts = {};
    for (const c of data.crates) counts[c.license] = (counts[c.license] || 0) + 1;
    q('#info-third-intro').textContent =
      `Die App enthält ${data.crates.length} Softwarepakete Dritter. Ihre Lizenzen erlauben die Verwendung in einer MIT-lizenzierten App; ` +
      'die Lizenztexte stehen unten und liegen als THIRD_PARTY_LICENSES.md im App-Paket. Die macOS-WebView stellt das System.';
    const notes = data.crates.filter((c) => c.note && (c.name === 'mp3lame-sys' || c.name === 'symphonia'));
    q('#info-notes').innerHTML = notes.length
      ? `<ul class="notes">${notes.map((c) => `<li><b>${esc(c.name)} ${esc(c.version)} (${esc(c.license)}):</b> ${esc(c.note)}</li>`).join('')}</ul>`
      : '';
    renderCrates(q('#info-filter').value);
    q('#info-texts-summary').textContent = `Alle Lizenztexte (${data.texts.length} verschiedene)`;
  }

  function renderTexts() {
    if (q('#info-texts').dataset.done) return;
    q('#info-texts').innerHTML = data.texts
      .map((t) => `<div class="lic-text"><h4>Lizenztext ${esc(t.id)}</h4><div class="for">gilt für ${esc(t.crates.join(', '))}</div><pre>${esc(t.text)}</pre></div>`)
      .join('');
    q('#info-texts').dataset.done = '1';
  }

  q('#info-open').addEventListener('click', open);
  q('#info-close').addEventListener('click', () => { modal.hidden = true; });
  modal.addEventListener('click', (e) => {
    if (e.target === modal) modal.hidden = true;
    const b = e.target.closest('[data-link]');
    if (b && window.openLink) window.openLink(b.dataset.link);
  });
  document.addEventListener('keydown', (e) => { if (e.key === 'Escape' && !modal.hidden) modal.hidden = true; });
  q('#info-filter').addEventListener('input', (e) => { if (data) renderCrates(e.target.value); });
  q('#info-texts-wrap').addEventListener('toggle', (e) => { if (e.target.open && data) renderTexts(); });
})();
