'use strict';

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (sel) => document.querySelector(sel);
const el = {
  empty: $('#empty'), results: $('#results'), summary: $('#summary'), list: $('#list'),
  extras: $('#extras'), extrasSummary: $('#extras-summary'), extrasBody: $('#extras-body'),
  singles: $('#singles'), bottom: $('#bottom'), outDir: $('#out-dir'), progress: $('#progress'),
  bar: $('#bar'), ptext: $('#ptext'), done: $('#done'), sel: $('#sel'), go: $('#go'), cancel: $('#cancel'),
  overlay: $('#overlay'), scanning: $('#scanning'), toast: $('#toast'), topActions: $('#top-actions'),
};

const st = {
  inputs: [],
  scan: null,
  outDir: '',
  selected: new Set(),
  outcomes: new Map(),
  busy: false,
  activeId: null,
};

window.mergeState = st;

function setMode(mode) {
  document.body.dataset.mode = mode;
  document.querySelectorAll('#tabs button').forEach((b) => b.classList.toggle('on', b.dataset.mode === mode));
}
document.querySelectorAll('#tabs button').forEach((b) => b.addEventListener('click', () => setMode(b.dataset.mode)));
window.openLink = (url) => invoke('open_link', { url }).catch((e) => toast(String(e), 'bad'));
function anyBusy() { return st.busy || !!(window.syncState && window.syncState.busy) || !!(window.masterState && window.masterState.busy); }

/* ---------- formatting ---------- */

function esc(s) {
  return String(s).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
}
function fmtDur(sec) {
  const s = Math.round(sec);
  const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60), r = s % 60;
  if (h) return `${h} h ${String(m).padStart(2, '0')} min`;
  if (m) return `${m} min ${String(r).padStart(2, '0')} s`;
  return `${r} s`;
}
function fmtBytes(b) {
  const u = ['B', 'KB', 'MB', 'GB', 'TB'];
  let i = 0;
  while (b >= 1024 && i < u.length - 1) { b /= 1024; i++; }
  return `${b.toLocaleString('de-DE', { maximumFractionDigits: i >= 2 ? 1 : 0 })} ${u[i]}`;
}
function plural(n, one, many) { return `${n} ${n === 1 ? one : many}`; }
function isSplit(r) { return r.parts.length > 1; }

let toastTimer = null;
function toast(msg, kind = '') {
  el.toast.textContent = msg;
  el.toast.className = `toast ${kind}`;
  el.toast.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => { el.toast.hidden = true; }, kind === 'bad' ? 9000 : 4000);
}

/* ---------- actions ---------- */

async function chooseSource() {
  if (anyBusy()) return;
  try {
    const p = await invoke('pick_folder', { title: 'Ordner mit DJI-Aufnahmen wählen' });
    if (p) runScan([p]);
  } catch (e) { toast(String(e), 'bad'); }
}

async function runScan(paths) {
  if (anyBusy() || !paths.length) return;
  st.inputs = paths;
  el.scanning.hidden = false;
  try {
    const scan = await invoke('scan_paths', { paths });
    st.scan = scan;
    st.outDir = scan.default_out_dir;
    st.outcomes.clear();
    st.selected = new Set(scan.recordings.filter((r) => isSplit(r) || el.singles.checked).map((r) => r.id));
    el.done.hidden = true;
    render();
    if (!scan.recordings.length) toast('Keine DJI-Aufnahmen gefunden (erwartet: DJI_NN_JJJJMMTT_HHMMSS.WAV).');
  } catch (e) {
    toast(String(e), 'bad');
  } finally {
    el.scanning.hidden = true;
  }
}

async function chooseOutDir() {
  if (st.busy) return;
  try {
    const p = await invoke('pick_folder', { title: 'Wo soll der Ordner „tracks“ liegen?' });
    if (!p) return;
    st.outDir = /[\\/]tracks[\\/]?$/.test(p) ? p.replace(/[\\/]$/, '') : `${p.replace(/[\\/]$/, '')}/tracks`;
    st.outcomes.clear();
    el.done.hidden = true;
    render();
  } catch (e) { toast(String(e), 'bad'); }
}

async function runMerge() {
  const ids = st.scan.recordings.filter((r) => st.selected.has(r.id)).map((r) => r.id);
  if (!ids.length || st.busy) return;
  setBusy(true);
  st.outcomes.clear();
  el.done.hidden = true;
  el.bar.style.width = '0%';
  el.ptext.textContent = 'Vorbereiten…';
  render();
  try {
    const sum = await invoke('merge_recordings', { ids, outDir: st.outDir });
    for (const o of sum.outcomes) st.outcomes.set(o.id, o);
    showDone(sum);
  } catch (e) {
    toast(String(e), 'bad');
  } finally {
    st.activeId = null;
    setBusy(false);
    render();
  }
}

function showDone(sum) {
  const c = { written: 0, existing: 0, failed: 0, cancelled: 0 };
  for (const o of sum.outcomes) c[o.status]++;
  const bits = [];
  if (c.written) bits.push(`${plural(c.written, 'Datei', 'Dateien')} geschrieben`);
  if (c.existing) bits.push(`${c.existing} schon vorhanden`);
  if (c.failed) bits.push(`${c.failed} fehlgeschlagen`);
  if (c.cancelled) bits.push(`${c.cancelled} abgebrochen`);
  const ok = !c.failed && !sum.cancelled;
  el.done.className = `done${ok ? '' : ' partial'}`;
  el.done.innerHTML = `<span>${ok ? 'Fertig' : sum.cancelled ? 'Abgebrochen' : 'Mit Fehlern beendet'}: ${esc(bits.join(', ') || 'nichts zu tun')}.</span>
    <button class="link" id="open-out">tracks öffnen</button>
    <button class="link" id="to-sync">Weiter: synchronisieren</button>`;
  el.done.hidden = false;
  $('#open-out').onclick = () => invoke('reveal', { path: sum.out_dir, select: false }).catch((e) => toast(String(e), 'bad'));
  $('#to-sync').onclick = () => { setMode('sync'); if (window.syncAnalyze) window.syncAnalyze([sum.out_dir]); };
}

function setBusy(b) {
  st.busy = b;
  el.progress.hidden = !b;
  el.cancel.hidden = !b;
  el.go.hidden = b;
  document.body.classList.toggle('busy', b);
}

/* ---------- rendering ---------- */

function render() {
  const s = st.scan;
  el.empty.hidden = !!s;
  el.results.hidden = !s;
  el.bottom.hidden = !s;
  el.topActions.hidden = !s;
  if (!s) return;

  const split = s.recordings.filter(isSplit);
  const singles = s.recordings.length - split.length;
  const partCount = split.reduce((a, r) => a + r.parts.length, 0);
  const skipped = s.duplicates.length + s.ignored.length;
  el.summary.innerHTML = `
    <div class="stat lead"><div class="n">${split.length}</div><div class="l">gestückelte Aufnahmen aus ${plural(partCount, 'Teil', 'Teilen')}</div></div>
    <div class="stat"><div class="n">${singles}</div><div class="l">Einzelaufnahmen</div></div>
    <div class="stat"><div class="n">${s.files_seen}</div><div class="l">WAV-Dateien gefunden</div></div>
    <div class="stat"><div class="n">${skipped}</div><div class="l">übersprungen (${plural(s.duplicates.length, 'Kopie', 'Kopien')})</div></div>
    <div class="roots" title="${esc(s.roots.join('\n'))}">Durchsucht: ${esc(s.roots.join(' · '))}</div>`;

  let html = '';
  let day = null;
  for (const r of s.recordings) {
    if (r.date !== day) { day = r.date; html += `<div class="day">${esc(day)}</div>`; }
    html += recHtml(r);
  }
  if (!s.recordings.length) html = '<p class="day">Keine Aufnahmen gefunden.</p>';
  el.list.innerHTML = html;

  if (skipped) {
    el.extras.hidden = false;
    el.extrasSummary.textContent = `${plural(skipped, 'Datei', 'Dateien')} übersprungen`;
    const li = (x) => `<li>${esc(x.path)} <span>– ${esc(x.reason)}</span></li>`;
    el.extrasBody.innerHTML =
      (s.duplicates.length ? `<h4>Identische Kopien</h4><ul>${s.duplicates.map(li).join('')}</ul>` : '') +
      (s.ignored.length ? `<h4>Nicht verwendet</h4><ul>${s.ignored.map(li).join('')}</ul>` : '');
  } else {
    el.extras.hidden = true;
  }

  el.outDir.innerHTML = `<bdi>${esc(st.outDir)}</bdi>`;
  el.outDir.title = `${st.outDir}\nKlicken zum Ändern`;
  updateSelection();
}

function recHtml(r) {
  const on = st.selected.has(r.id);
  const o = st.outcomes.get(r.id);
  const split = isSplit(r);
  const statusLabel = { written: 'Fertig', existing: 'Schon vorhanden', failed: 'Fehler', cancelled: 'Abgebrochen' };
  let status = '';
  if (o) {
    status = o.path
      ? `<div class="status ${o.status}"><button data-reveal="${esc(o.path)}" title="Im Finder zeigen">${statusLabel[o.status]}</button></div>`
      : `<div class="status ${o.status}">${statusLabel[o.status]}</div>`;
  } else if (st.activeId === r.id) {
    status = '<div class="status existing">läuft…</div>';
  }
  const parts = r.parts.map((p) => {
    const gap = p.gap_to_prev == null ? '' : ` · Anschluss ${p.gap_to_prev > 0 ? '+' : ''}${p.gap_to_prev.toLocaleString('de-DE')} s`;
    return `<li><button data-reveal="${esc(p.path)}" title="Im Finder zeigen">${esc(p.path)}</button><span class="pmeta">${p.start.slice(11)} · ${fmtDur(p.duration)}${gap}</span></li>`;
  }).join('');
  return `
  <div class="rec${on ? '' : ' off'}${st.activeId === r.id ? ' active' : ''}" data-id="${r.id}">
    <input type="checkbox" data-id="${r.id}" ${on ? 'checked' : ''} ${st.busy ? 'disabled' : ''} aria-label="Aufnahme auswählen">
    <div class="when">
      <span class="time">${esc(r.start)} – ${esc(r.end)}</span>
      <span class="dur">${fmtDur(r.duration)}</span>
      <span class="parts-badge${split ? '' : ' single'}">${split ? `${r.parts.length} Teile` : 'Einzeldatei'}</span>
      <span class="tag" title="Ordner des ersten Teils">${esc(r.label)}</span>
    </div>
    ${status}
    <div class="meta"><span>${esc(r.format)} · ${fmtBytes(r.output_bytes)}</span><span class="out">${esc(r.out_name)}</span></div>
    ${r.warnings.length ? `<ul class="warnings">${r.warnings.map((w) => `<li>${esc(w)}</li>`).join('')}</ul>` : ''}
    ${o && o.message ? `<div class="errmsg">${esc(o.message)}</div>` : ''}
    <details class="partlist"><summary>${split ? 'Teile' : 'Datei'} anzeigen</summary><ol>${parts}</ol></details>
  </div>`;
}

function updateSelection() {
  if (!st.scan) return;
  const chosen = st.scan.recordings.filter((r) => st.selected.has(r.id));
  const bytes = chosen.reduce((a, r) => a + r.output_bytes, 0);
  el.sel.textContent = chosen.length ? `${plural(chosen.length, 'Aufnahme', 'Aufnahmen')} · ${fmtBytes(bytes)}` : 'nichts ausgewählt';
  el.go.disabled = !chosen.length || st.busy;
  const singles = st.scan.recordings.filter((r) => !isSplit(r));
  el.singles.disabled = st.busy || !singles.length;
  el.singles.checked = singles.length > 0 && singles.every((r) => st.selected.has(r.id));
}

/* ---------- events ---------- */

$('#pick').addEventListener('click', chooseSource);
$('#pick-again').addEventListener('click', chooseSource);
$('#rescan').addEventListener('click', () => runScan(st.inputs));
el.outDir.addEventListener('click', chooseOutDir);
el.go.addEventListener('click', runMerge);
el.cancel.addEventListener('click', () => {
  el.ptext.textContent = 'Breche ab…';
  invoke('cancel_merge');
});

el.singles.addEventListener('change', () => {
  for (const r of st.scan.recordings) {
    if (isSplit(r)) continue;
    if (el.singles.checked) st.selected.add(r.id); else st.selected.delete(r.id);
  }
  render();
});
$('#select-all').addEventListener('click', () => { if (!st.busy) { st.scan.recordings.forEach((r) => st.selected.add(r.id)); render(); } });
$('#select-none').addEventListener('click', () => { if (!st.busy) { st.selected.clear(); render(); } });

el.list.addEventListener('change', (e) => {
  const id = e.target.dataset && e.target.dataset.id;
  if (id === undefined || st.busy) return;
  if (e.target.checked) st.selected.add(Number(id)); else st.selected.delete(Number(id));
  const row = e.target.closest('.rec');
  if (row) row.classList.toggle('off', !e.target.checked);
  updateSelection();
});
document.addEventListener('click', (e) => {
  const b = e.target.closest('[data-reveal]');
  if (b) invoke('reveal', { path: b.dataset.reveal, select: true }).catch((err) => toast(String(err), 'bad'));
});

listen('merge-progress', ({ payload: p }) => {
  const pct = p.total ? p.done / p.total : 1;
  el.bar.style.width = `${(pct * 100).toFixed(1)}%`;
  el.ptext.textContent = `${p.index + 1} von ${p.count} · ${p.name} · ${Math.floor(pct * 100)} %`;
  if (st.activeId !== p.id) {
    st.activeId = p.id;
    document.querySelectorAll('.rec.active').forEach((n) => n.classList.remove('active'));
    const row = el.list.querySelector(`.rec[data-id="${p.id}"]`);
    if (row) { row.classList.add('active'); row.scrollIntoView({ block: 'nearest', behavior: 'smooth' }); }
  }
});

listen('tauri://drag-enter', () => { if (!anyBusy()) el.overlay.hidden = false; });
listen('tauri://drag-leave', () => { el.overlay.hidden = true; });
listen('tauri://drag-drop', ({ payload }) => {
  el.overlay.hidden = true;
  const paths = (payload && payload.paths) || [];
  if (anyBusy()) { toast('Bitte warten, bis der laufende Vorgang fertig ist.'); return; }
  if (!paths.length) return;
  if (document.body.dataset.mode === 'sync' && window.syncAnalyze) window.syncAnalyze(paths);
  else if (document.body.dataset.mode === 'master' && window.masterAnalyze) window.masterAnalyze(paths);
  else runScan(paths);
});

render();
