'use strict';

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

/* i18n aliases for this file (never the bare global t: see docs/I18N.md). */
const tr = (k, p) => I18N.t(k, p);
const trn = (k, n, p) => I18N.tn(k, n, p);

const $ = (sel) => document.querySelector(sel);
const el = {
  empty: $('#empty'), results: $('#results'), summary: $('#summary'), list: $('#list'),
  extras: $('#extras'), extrasSummary: $('#extras-summary'), extrasBody: $('#extras-body'),
  singles: $('#singles'), bottom: $('#bottom'), retry: $('#retry'), finished: $('#merge-finished'), progress: $('#progress'),
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
  lastSum: null,      // summary of a fully finished run (finished card is showing)
  partial: null,      // { done, total, failed } of a run with failures (done message is showing)
};

window.mergeState = st;

function setMode(mode) {
  document.body.dataset.mode = mode;
  document.querySelectorAll('#tabs button').forEach((b) => b.classList.toggle('on', b.dataset.mode === mode));
}
document.querySelectorAll('#tabs button').forEach((b) => b.addEventListener('click', () => setMode(b.dataset.mode)));
window.openLink = (url) => invoke('open_link', { url }).catch((e) => toast(String(e), 'bad'));

/* Heartbeat: the backend sends 'work-heartbeat' every 0.5 s while it computes.
   Every activity wheel stops and turns amber when the beat stays away. */
window.beat = { last: 0 };
listen('work-heartbeat', () => { window.beat.last = Date.now(); });
setInterval(() => {
  const quiet = Date.now() - window.beat.last;
  document.querySelectorAll('.wheel').forEach((w) => {
    const stalled = window.beat.last > 0 && quiet > 2500;
    w.classList.toggle('stalled', stalled);
    w.title = stalled ? tr('common.noResponse', { s: Math.round(quiet / 1000) }) : tr('common.working');
  });
}, 500);

/* A1: every step asks where to save; the app writes into the step's subfolder. */
function parentDir(p) { return String(p || '').replace(/[\\/][^\\/]*$/, ''); }
window.parentDir = parentDir;
window.pickOutput = (title, start, sub) => invoke('pick_output_dir', { title, start, sub });

/* A3: a finished run replaces the list with a short summary.
   opts: noun [one, many] (already translated), openLabel, nextLabel, newLabel, onNext, onNew.
   Optional opts.written(n) returns the full "n … written" phrase (lets a caller get gender/plural right). */
window.finishedCard = (container, sum, opts) => {
  const c = { written: 0, existing: 0 };
  for (const o of sum.outcomes) if (o.status in c) c[o.status]++;
  const n = sum.outcomes.length;
  const written = !c.written ? ''
    : opts.written ? opts.written(c.written)
      : tr('common.finished.written', { items: plural(c.written, opts.noun[0], opts.noun[1]) });
  const detail = [written, c.existing ? trn('common.finished.existing', c.existing) : '']
    .filter(Boolean).join(', ');
  container.innerHTML = `
    <div class="finished-card">
      <div class="finished-icon">✓</div>
      <h2>${esc(tr('common.doneOf', { done: n, total: n }))}</h2>
      <p>${esc(detail || tr('common.finished.nothing'))}</p>
      <p class="path"><bdi>${esc(sum.out_dir)}</bdi></p>
      <div class="finished-actions">
        <button class="btn" data-act="open">${esc(opts.openLabel)}</button>
        ${opts.nextLabel ? `<button class="btn primary" data-act="next">${esc(opts.nextLabel)}</button>` : ''}
        <button class="btn ghost" data-act="new">${esc(opts.newLabel)}</button>
      </div>
    </div>`;
  container.hidden = false;
  container.onclick = (e) => {
    const act = e.target.closest('[data-act]');
    if (!act) return;
    if (act.dataset.act === 'open') invoke('reveal', { path: sum.out_dir, select: false }).catch((err) => toast(String(err), 'bad'));
    if (act.dataset.act === 'next' && opts.onNext) opts.onNext();
    if (act.dataset.act === 'new') { container.hidden = true; if (opts.onNew) opts.onNew(); }
  };
};
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
  const u = tr('common.bytesUnits').split('|');
  let i = 0;
  while (b >= 1024 && i < u.length - 1) { b /= 1024; i++; }
  return `${b.toLocaleString(I18N.locale(), { maximumFractionDigits: i >= 2 ? 1 : 0 })} ${u[i]}`;
}
/* Legacy helper for callers that pass already-translated words; new code uses trn(). */
function plural(n, one, many) { return `${n} ${n === 1 ? one : many}`; }
function isSplit(r) { return r.parts.length > 1; }
/* Where a chunk's start time came from (scan.rs TimeSource). */
const TIME_SOURCE_KEY = { bext: 'merge.timeSource.bext', name: 'merge.timeSource.name', file: 'merge.timeSource.file' };
/* How sure a chain of chunks is (scan.rs Confidence). */
const CONFIDENCE_KEY = { high: 'merge.confidence.high', medium: 'merge.confidence.medium', low: 'merge.confidence.low' };
function confLabel(level) { return tr('merge.confidence', { level: tr(CONFIDENCE_KEY[level]) }); }

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
    const p = await invoke('pick_folder', { title: tr('merge.pickTitle') });
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
    // Chains with low confidence are listed but not preselected: the user checks them first.
    st.selected = new Set(scan.recordings.filter((r) => (isSplit(r) ? r.confidence !== 'low' : el.singles.checked)).map((r) => r.id));
    st.lastSum = null;
    st.partial = null;
    el.done.hidden = true;
    el.retry.hidden = true;
    el.finished.hidden = true;
    render();
    if (!scan.recordings.length) toast(tr('merge.noneFound'));
  } catch (e) {
    toast(String(e), 'bad');
  } finally {
    el.scanning.hidden = true;
  }
}

async function runMerge(retryIds) {
  if (!st.scan || anyBusy()) return;
  const ids = retryIds || st.scan.recordings.filter((r) => st.selected.has(r.id)).map((r) => r.id);
  if (!ids.length) return;
  if (!retryIds) {
    let out = null;
    try {
      out = await window.pickOutput(tr('merge.whereToSave'), parentDir(st.scan.default_out_dir), 'tracks');
    } catch (e) { toast(String(e), 'bad'); return; }
    if (!out) return;
    st.outDir = out;
  }
  setBusy(true);
  st.outcomes.clear();
  st.partial = null;
  el.done.hidden = true;
  el.retry.hidden = true;
  el.bar.style.width = '0%';
  el.ptext.textContent = tr('common.preparing');
  render();
  try {
    const sum = await invoke('merge_recordings', { ids, outDir: st.outDir });
    for (const o of sum.outcomes) st.outcomes.set(o.id, o);
    finishRun(sum);
  } catch (e) {
    toast(String(e), 'bad');
  } finally {
    st.activeId = null;
    setBusy(false);
    render();
  }
}

function showFinished(sum) {
  window.finishedCard(el.finished, sum, {
    noun: [tr('merge.track.one'), tr('merge.track.other')],
    written: (n) => trn('merge.finished.written', n),
    openLabel: tr('merge.finished.open'), nextLabel: tr('merge.finished.next'), newLabel: tr('merge.finished.new'),
    onNext: () => { setMode('sync'); if (window.syncAnalyze) window.syncAnalyze([sum.out_dir]); },
    onNew: () => { st.lastSum = null; render(); },
  });
}

function showPartial() {
  const p = st.partial;
  el.done.innerHTML = `<span>${esc(tr('common.doneOf', { done: p.done, total: p.total }))}. ${esc(trn('merge.partialFailed', p.failed))}</span>`;
}

function finishRun(sum) {
  const bad = sum.outcomes.filter((o) => o.status === 'failed' || o.status === 'cancelled');
  if (!bad.length) {
    st.scan = null;
    st.selected.clear();
    st.outcomes.clear();
    st.lastSum = sum;
    showFinished(sum);
    return;
  }
  const left = new Set(bad.map((o) => o.id));
  st.scan.recordings = st.scan.recordings.filter((r) => left.has(r.id));
  st.selected = new Set(left);
  st.partial = { done: sum.outcomes.length - bad.length, total: sum.outcomes.length, failed: bad.length };
  el.done.className = 'done partial';
  showPartial();
  el.done.hidden = false;
  el.retry.hidden = false;
  el.retry.onclick = () => runMerge([...left]);
}

function setBusy(b) {
  st.busy = b;
  if (b) window.beat.last = Date.now();
  el.progress.hidden = !b;
  el.cancel.hidden = !b;
  el.go.hidden = b;
  document.body.classList.toggle('busy', b);
}

/* ---------- rendering ---------- */

function render() {
  const s = st.scan;
  el.empty.hidden = !!s || !el.finished.hidden;
  el.results.hidden = !s;
  el.bottom.hidden = !s;
  el.topActions.hidden = !s;
  if (!s) return;

  const split = s.recordings.filter(isSplit);
  const singles = s.recordings.length - split.length;
  const partCount = split.reduce((a, r) => a + r.parts.length, 0);
  const skipped = s.duplicates.length + s.ignored.length;
  el.summary.innerHTML = `
    <div class="stat lead"><div class="n">${split.length}</div><div class="l">${esc(trn('merge.stat.split', partCount))}</div></div>
    <div class="stat"><div class="n">${singles}</div><div class="l">${esc(tr('merge.stat.singles'))}</div></div>
    <div class="stat"><div class="n">${s.files_seen}</div><div class="l">${esc(tr('merge.stat.files'))}</div></div>
    <div class="stat"><div class="n">${skipped}</div><div class="l">${esc(trn('merge.stat.skipped', s.duplicates.length))}</div></div>
    <div class="roots" title="${esc(s.roots.join('\n'))}">${esc(tr('merge.stat.scanned', { roots: s.roots.join(' · ') }))}</div>`;

  let html = '';
  let day = null;
  for (const r of s.recordings) {
    if (r.date !== day) { day = r.date; html += `<div class="day">${esc(day)}</div>`; }
    html += recHtml(r);
  }
  if (!s.recordings.length) html = `<p class="day">${esc(tr('merge.noRecordings'))}</p>`;
  el.list.innerHTML = html;

  if (skipped) {
    el.extras.hidden = false;
    el.extrasSummary.textContent = trn('merge.skippedFiles', skipped);
    const li = (x) => `<li>${esc(x.path)} <span>– ${esc(x.reason)}</span></li>`;
    el.extrasBody.innerHTML =
      (s.duplicates.length ? `<h4>${esc(tr('merge.duplicates'))}</h4><ul>${s.duplicates.map(li).join('')}</ul>` : '') +
      (s.ignored.length ? `<h4>${esc(tr('merge.unused'))}</h4><ul>${s.ignored.map(li).join('')}</ul>` : '');
  } else {
    el.extras.hidden = true;
  }

  updateSelection();
}

function recHtml(r) {
  const on = st.selected.has(r.id);
  const o = st.outcomes.get(r.id);
  const split = isSplit(r);
  const statusLabel = {
    written: tr('common.status.written'), existing: tr('common.status.existing'),
    failed: tr('common.status.failed'), cancelled: tr('common.status.cancelled'),
  };
  const reveal = esc(tr('common.reveal'));
  let status = '';
  if (o) {
    status = o.path
      ? `<div class="status ${o.status}"><button data-reveal="${esc(o.path)}" title="${reveal}">${esc(statusLabel[o.status])}</button></div>`
      : `<div class="status ${o.status}">${esc(statusLabel[o.status])}</div>`;
  } else if (st.activeId === r.id) {
    status = `<div class="status existing">${esc(tr('common.running'))}</div>`;
  }
  const parts = r.parts.map((p) => {
    const gap = p.gap_to_prev == null ? ''
      : ` · ${esc(tr('merge.gap', { gap: `${p.gap_to_prev > 0 ? '+' : ''}${p.gap_to_prev.toLocaleString(I18N.locale())}` }))}`;
    const src = TIME_SOURCE_KEY[p.time_source];
    const when = src ? ` · <span class="tsrc">${esc(tr(src))}</span>` : '';
    const link = CONFIDENCE_KEY[p.link_confidence] ? ` · <span class="pconf ${p.link_confidence}">${esc(confLabel(p.link_confidence))}</span>` : '';
    return `<li><button data-reveal="${esc(p.path)}" title="${reveal}">${esc(p.path)}</button><span class="pmeta">${p.start.slice(11)} · ${fmtDur(p.duration)}${gap}${when}${link}</span></li>`;
  }).join('');
  return `
  <div class="rec${on ? '' : ' off'}${st.activeId === r.id ? ' active' : ''}" data-id="${r.id}">
    <input type="checkbox" data-id="${r.id}" ${on ? 'checked' : ''} ${st.busy ? 'disabled' : ''} aria-label="${esc(tr('merge.selectRecording'))}">
    <div class="when">
      <span class="time">${esc(r.start)} – ${esc(r.end)}</span>
      <span class="dur">${fmtDur(r.duration)}</span>
      <span class="parts-badge${split ? '' : ' single'}">${esc(split ? trn('merge.partsBadge', r.parts.length) : tr('merge.singleFile'))}</span>
      ${split && CONFIDENCE_KEY[r.confidence] ? `<span class="conf ${r.confidence}">${esc(confLabel(r.confidence))}</span>` : ''}
      <span class="tag" title="${esc(tr('merge.firstChunkFolder'))}">${esc(r.label)}</span>
    </div>
    ${status}
    <div class="meta"><span>${esc(r.format)} · ${fmtBytes(r.output_bytes)}</span><span class="out">${esc(r.out_name)}</span></div>
    ${r.warnings.length ? `<ul class="warnings">${r.warnings.map((w) => `<li>${esc(w)}</li>`).join('')}</ul>` : ''}
    ${o && o.message ? `<div class="errmsg">${esc(o.message)}</div>` : ''}
    <details class="partlist"><summary>${esc(tr(split ? 'merge.showParts' : 'merge.showFile'))}</summary><ol>${parts}</ol></details>
  </div>`;
}

function updateSelection() {
  if (!st.scan) return;
  const chosen = st.scan.recordings.filter((r) => st.selected.has(r.id));
  const bytes = chosen.reduce((a, r) => a + r.output_bytes, 0);
  el.sel.textContent = chosen.length ? trn('merge.selection', chosen.length, { size: fmtBytes(bytes) }) : tr('common.nothingSelected');
  el.go.disabled = !chosen.length || st.busy;
  const singles = st.scan.recordings.filter((r) => !isSplit(r));
  el.singles.disabled = st.busy || !singles.length;
  el.singles.checked = singles.length > 0 && singles.every((r) => st.selected.has(r.id));
}

/* ---------- events ---------- */

$('#pick').addEventListener('click', chooseSource);
$('#pick-again').addEventListener('click', chooseSource);
$('#rescan').addEventListener('click', () => runScan(st.inputs));
el.go.addEventListener('click', () => runMerge());
el.cancel.addEventListener('click', () => {
  el.ptext.textContent = tr('common.cancelling');
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
  el.ptext.textContent = tr('common.progress', { i: p.index + 1, count: p.count, name: p.name, pct: Math.floor(pct * 100) });
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
  if (anyBusy()) { toast(tr('common.waitBusy')); return; }
  if (!paths.length) return;
  if (document.body.dataset.mode === 'sync' && window.syncAnalyze) window.syncAnalyze(paths);
  else if (document.body.dataset.mode === 'master' && window.masterAnalyze) window.masterAnalyze(paths);
  else runScan(paths);
});

/* Language switch: redraw everything this tab shows, without reload. */
I18N.onChange(() => {
  render();
  if (st.lastSum && !el.finished.hidden) showFinished(st.lastSum);
  if (st.partial && !el.done.hidden) showPartial();
});

render();
